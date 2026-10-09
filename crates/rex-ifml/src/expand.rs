//! Module expansion: inlining `use` statements into ready-to-consume views.
//!
//! The resolver keeps `use` sites as references (the artifact stays
//! byte-identical); expansion is the opt-in consumer convenience on top:
//! [`expand_ifml`] replaces every use site with the referenced module's
//! containers and components, so downstream tools that do not want to
//! resolve imports themselves can walk plain views.
//!
//! # Semantics (v1)
//!
//! - A `use` in a view/container body is replaced by the referenced
//!   module's containers and components, expanded recursively
//!   (module-internal uses inline too; module recursion through itself is
//!   skipped rather than looped).
//! - The module's events are appended to the use point's events.
//! - Overrides apply: any property inside the spliced subtree whose key
//!   matches a use override is replaced by the override (the module's own
//!   property defaults lose); unmatched overrides are dropped — inputs are
//!   consumed by the spliced tree, everything else is a slot with no
//!   default to fill.
//!
//! The expansion is structural only: nothing is renamed or qualified, and
//! the wire format is untouched (`formatVersion: 1`).

use std::collections::{BTreeMap, BTreeSet};

use rex_ir::ifml::{
    ComponentDeclaration, ContainerDeclaration, EventHandler, IfmlModel, ModuleDeclaration,
    ModuleUse, PropertyAssignment, ViewDeclaration,
};

use crate::IfmlCompilation;

/// Expands every `use` in the compilation's main model into the referenced
/// module contents. `imports` supplies the imported files' sources (the
/// artifact carries only the main file's own declarations); their modules
/// join the main file's in the module table, keyed by declared name.
pub fn expand_ifml(compilation: &IfmlCompilation, imports: &crate::IfmlImports) -> IfmlModel {
    let modules = module_table(compilation, imports);
    let mut expander = Expander {
        compilation,
        modules: &modules,
        consumed: BTreeSet::new(),
        active: BTreeSet::new(),
    };
    let mut model = compilation.model.clone();
    for view in &mut model.views {
        expander.expand_view(view);
    }
    model
}

/// The module table: the main file's declarations plus every import's,
/// keyed by declared module name (the resolver guarantees uniqueness —
/// local shadows imported, cross-import duplicates are ambiguous).
fn module_table(
    compilation: &IfmlCompilation,
    imports: &crate::IfmlImports,
) -> BTreeMap<String, ModuleDeclaration> {
    let mut table: BTreeMap<String, ModuleDeclaration> = BTreeMap::new();
    for module in &compilation.model.modules {
        table.insert(module.name.clone(), module.clone());
    }
    for ((_, import_path), text) in imports.iter() {
        if !import_path.ends_with(".ifml") {
            continue;
        }
        if let Ok(model) = crate::parse_ifml(text) {
            for module in model.modules {
                table
                    .entry(module.name.clone())
                    .or_insert_with(|| module.clone());
            }
        }
    }
    table
}

struct Expander<'a> {
    compilation: &'a IfmlCompilation,
    modules: &'a BTreeMap<String, ModuleDeclaration>,
    /// Index positions of already-spliced use sites: index sites are
    /// recorded in the same order the model walk encounters them, but
    /// same-target siblings exist, so consumption is tracked explicitly.
    consumed: BTreeSet<usize>,
    /// Module names currently being inlined (recursion guard).
    active: BTreeSet<String>,
}

impl<'a> Expander<'a> {
    fn expand_view(&mut self, view: &mut ViewDeclaration) {
        self.expand_body(
            &mut view.containers,
            &mut view.components,
            &mut view.events,
            &mut view.module_uses,
        );
    }

    fn expand_body(
        &mut self,
        containers: &mut Vec<ContainerDeclaration>,
        components: &mut Vec<ComponentDeclaration>,
        events: &mut Vec<EventHandler>,
        uses: &mut Vec<ModuleUse>,
    ) {
        // Splice this level's uses first: each resolves to a module whose
        // children join the use point, recursively expanded.
        let level_uses = std::mem::take(uses);
        for use_site in level_uses {
            if let Some(module) = self.module_for(&use_site) {
                self.active.insert(module.name.clone());
                let mut spliced_containers = module.containers.clone();
                let mut spliced_components = module.components.clone();
                let mut spliced_events = module.events.clone();
                let mut spliced_uses = module.module_uses.clone();
                // Recursion inlines the module's own uses (composition).
                self.expand_body(
                    &mut spliced_containers,
                    &mut spliced_components,
                    &mut spliced_events,
                    &mut spliced_uses,
                );
                for container in &mut spliced_containers {
                    apply_overrides_to_container(container, &use_site.properties);
                }
                for component in &mut spliced_components {
                    apply_overrides_recursively(
                        component.properties.as_mut_slice(),
                        &use_site.properties,
                    );
                }
                containers.append(&mut spliced_containers);
                components.append(&mut spliced_components);
                events.append(&mut spliced_events);
                self.active.remove(&module.name);
            }
        }
        for container in containers.iter_mut() {
            self.expand_body(
                &mut container.containers,
                &mut container.components,
                &mut container.events,
                &mut container.module_uses,
            );
        }
    }

    /// The module a use site resolves to, honoring consumption order and
    /// the recursion guard. Unresolved uses (unknown module) and cyclic
    /// self-splices drop out of the expansion.
    fn module_for(&mut self, use_site: &ModuleUse) -> Option<&'a ModuleDeclaration> {
        // Copy the long-lived borrows out of `&mut self` so the returned
        // reference keeps the Expander's lifetime, not the call's.
        let compilation: &'a IfmlCompilation = self.compilation;
        let modules: &'a BTreeMap<String, ModuleDeclaration> = self.modules;
        let position = compilation
            .index
            .module_uses
            .iter()
            .enumerate()
            .find(|(index, site)| !self.consumed.contains(index) && site.target == use_site.module)
            .map(|(index, _)| index)?;
        let site = &compilation.index.module_uses[position];
        let reference = site.resolved_module?;
        let decl = compilation.index.module_decl(reference)?;
        if self.active.contains(&decl.name) {
            return None;
        }
        self.consumed.insert(position);
        modules.get(&decl.name)
    }
}

fn apply_overrides_recursively(
    properties: &mut [PropertyAssignment],
    overrides: &[PropertyAssignment],
) {
    for property in properties.iter_mut() {
        if let Some(override_property) = overrides
            .iter()
            .find(|candidate| candidate.key == property.key)
        {
            property.value = override_property.value.clone();
        }
    }
}

/// Applies overrides through a container's whole subtree: the container's
/// own properties, then its containers and components recursively.
fn apply_overrides_to_container(
    container: &mut ContainerDeclaration,
    overrides: &[PropertyAssignment],
) {
    apply_overrides_recursively(container.properties.as_mut_slice(), overrides);
    for child in &mut container.containers {
        apply_overrides_to_container(child, overrides);
    }
    for component in &mut container.components {
        apply_overrides_recursively(component.properties.as_mut_slice(), overrides);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{compile_ifml_str, IfmlImports};

    #[test]
    fn expansion_inlines_modules_and_applies_overrides() {
        let module = r#"module "Pager" {
    input { page: Int = 1, pageSize: Int = 25 }
    output { total: Int }
    page_size: 50;

    container "PagerBody" {
        component "pager" {
            type: list;
            data: Item;
            page_size: 50;
        }
    }

    on load -> refresh("pager");
}"#;
        let consumer = r#"import "pager.ifml";

view "Catalogue" {
    use "Pager" as pager { page_size: 25; };
}
"#;
        let mut imports = IfmlImports::default();
        imports.provide("app.ifml", "pager.ifml", module);
        let compilation =
            compile_ifml_str("app.ifml", consumer, &imports).expect("consumer compiles");
        let expanded = expand_ifml(&compilation, &imports);
        let view = &expanded.views[0];

        // The use is gone; the module's container spliced in.
        assert!(view.module_uses.is_empty(), "uses are inlined");
        assert_eq!(
            view.containers.len(),
            1,
            "the module's container spliced: {view:?}"
        );
        assert_eq!(view.containers[0].name, "PagerBody");

        // The override replaced the module default inside the spliced tree.
        let pager = &view.containers[0].components[0];
        let page_size = pager
            .properties
            .iter()
            .find(|property| property.key == "page_size")
            .expect("page_size property");
        assert!(
            format!("{:?}", page_size.value).contains("25"),
            "the override wins over the module default: {page_size:?}"
        );

        // The module's events joined the view.
        assert_eq!(view.events.len(), 1, "module events attach: {view:?}");
    }

    #[test]
    fn expansion_recurses_through_module_composition() {
        let inner = r#"module "Inner" {
    input { size: Int = 5 }
    output { n: Int }

    component "inner" {
        type: list;
        size: 5;
    }
}"#;
        let outer = r#"module "Outer" {
    input { size: Int = 5 }
    output { n: Int }

    use "Inner" as inner { size: 5; };

    component "outer" {
        type: list;
    }
}"#;
        let consumer = r#"import "inner.ifml";
import "outer.ifml";

view "Page" {
    use "Outer" as outer { size: 9; };
}
"#;
        let mut imports = IfmlImports::default();
        imports.provide("app.ifml", "inner.ifml", inner);
        imports.provide("app.ifml", "outer.ifml", outer);
        let compilation =
            compile_ifml_str("app.ifml", consumer, &imports).expect("consumer compiles");
        let expanded = expand_ifml(&compilation, &imports);
        let view = &expanded.views[0];
        assert!(view.module_uses.is_empty());
        // Outer spliced: its component + its own use (Inner) expanded.
        let outer_component = view
            .components
            .iter()
            .find(|component| component.name == "outer")
            .expect("outer's own component splices");
        assert_eq!(outer_component.name, "outer");
        // Inner (a flat module) spliced through Outer's module body use:
        // both components land in the view.
        let names: Vec<&str> = view
            .components
            .iter()
            .map(|component| component.name.as_str())
            .collect();
        assert_eq!(names, vec!["outer", "inner"], "full recursion: {view:?}");
    }
}
