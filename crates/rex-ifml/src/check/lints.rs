//! The lint pass: warning-severity findings over the resolution index and
//! the main file's AST (`W0150`–`W0154`). Every lint considers **only the
//! main file's** declarations — imported pattern libraries declare unused
//! modules and actions by design — while references count anywhere in the
//! compilation (an uninstantiated module's `action("X")` is still live
//! intent). Warnings never fail a compilation; hosts render them and
//! decide (`rexlang check --deny-warnings`).

use std::collections::{BTreeMap, BTreeSet};

use rex_ir::ifml::{
    ColumnDef, ComponentDeclaration, ComponentSpec, ComponentType, ContainerDeclaration,
    EventHandler, ViewDeclaration,
};

use crate::index::{IfmlIndex, NamedSite};
use crate::resolve::{IfmlCompilation, IfmlDiagnostic};

use super::{
    W_COMPONENT_WITHOUT_DATA, W_DEAD_ACTION, W_SHADOWED_MODULE, W_UNREACHABLE_VIEW,
    W_VIEW_WITHOUT_DOMAIN,
};

/// The lint pass: five structural warnings over the main file's
/// declarations, run after the interaction and authorization passes (their
/// errors already name dangling references; the lints judge what remains).
pub(super) fn check_lints(compilation: &IfmlCompilation) -> Vec<IfmlDiagnostic> {
    let mut diags = Vec::new();
    diags.extend(dead_actions(compilation));
    diags.extend(unreachable_views(compilation));
    diags.extend(shadowed_modules(compilation));
    diags.extend(components_without_data(compilation));
    diags.extend(views_without_domain_binding(compilation));
    diags
}

/// The compilation's main file path — every lint reports there.
fn main_path(index: &IfmlIndex) -> String {
    index.files[0].path.clone()
}

/// Pairs the AST views (source order) with their index sites: both derive
/// from one parse, so position `i` in `model.views` is position `i` among
/// the index's main-file view sites.
fn main_view_sites(
    compilation: &IfmlCompilation,
) -> impl Iterator<Item = (&ViewDeclaration, &NamedSite)> {
    compilation
        .model
        .views
        .iter()
        .zip(compilation.index.views.iter().filter(|site| site.file == 0))
}

/// W0150 — a main-file `action "X"` declaration no `action("X")` invocation
/// names, anywhere in the compilation.
fn dead_actions(compilation: &IfmlCompilation) -> Vec<IfmlDiagnostic> {
    let index = &compilation.index;
    let invoked: BTreeSet<&str> = index
        .action_targets
        .iter()
        .filter(|site| site.action == "action")
        .map(|site| site.target.as_str())
        .collect();
    index
        .actions
        .iter()
        .filter(|site| site.file == 0)
        .filter(|site| !invoked.contains(site.name.as_str()))
        .map(|site| {
            IfmlDiagnostic::warning(
                W_DEAD_ACTION,
                format!("action '{}' is declared but never invoked", site.name),
                site.name_span,
                main_path(index),
            )
        })
        .collect()
}

/// W0151 — a main-file view that is not the entry view (first by
/// position), not a landmark, reached by no `navigate` target, and carries
/// no body content. The manifest exports every view as a route, so a
/// content view is intentionally routable; the empty shell nothing
/// navigates to is the dead-UI class.
fn unreachable_views(compilation: &IfmlCompilation) -> Vec<IfmlDiagnostic> {
    let index = &compilation.index;
    let navigated: BTreeSet<&str> = index
        .action_targets
        .iter()
        .filter(|site| site.action == "navigate")
        .map(|site| site.target.as_str())
        .collect();
    let mut diags = Vec::new();
    for (position, (view, site)) in main_view_sites(compilation).enumerate() {
        if position == 0 || view.is_landmark || navigated.contains(view.name.as_str()) {
            continue;
        }
        let has_content = !view.components.is_empty()
            || !view.containers.is_empty()
            || !view.events.is_empty()
            || !view.module_uses.is_empty();
        if has_content {
            continue;
        }
        diags.push(IfmlDiagnostic::warning(
            W_UNREACHABLE_VIEW,
            format!(
                "view '{}' is never navigated to — mark it landmark or navigate to it",
                view.name
            ),
            site.name_span,
            main_path(index),
        ));
    }
    diags
}

/// W0152 — a main-file module declaration whose name equals an imported
/// file's declaration. Resolution deliberately lets the main file win
/// silently; the warning documents the shadowing at the winning
/// declaration's name.
fn shadowed_modules(compilation: &IfmlCompilation) -> Vec<IfmlDiagnostic> {
    let index = &compilation.index;
    let mut imported_by: BTreeMap<&str, &str> = BTreeMap::new();
    for decl in &index.module_decls {
        if decl.file != 0 {
            imported_by
                .entry(decl.name.as_str())
                .or_insert(index.files[decl.file].path.as_str());
        }
    }
    let mut diags = Vec::new();
    for decl in index.module_decls.iter().filter(|decl| decl.file == 0) {
        let Some(imported_from) = imported_by.get(decl.name.as_str()) else {
            continue;
        };
        diags.push(IfmlDiagnostic::warning(
            W_SHADOWED_MODULE,
            format!(
                "module '{}' shadows an imported module of the same name (from '{}')",
                decl.name, imported_from
            ),
            decl.name_span,
            main_path(index),
        ));
    }
    diags
}

/// W0153 — a main-file component with no `data:` binding that also carries
/// no events, no chart, and no explicit `Entity.field` table columns: a
/// fully static placeholder. Walks views and modules recursively through
/// containers.
fn components_without_data(compilation: &IfmlCompilation) -> Vec<IfmlDiagnostic> {
    let index = &compilation.index;
    let mut diags = Vec::new();
    let mut warn = |context: &str, component: &ComponentDeclaration| {
        if component_binds_data(component) {
            return;
        }
        let span = component
            .properties
            .iter()
            .find_map(|property| property.span)
            .unwrap_or((0, 0));
        diags.push(IfmlDiagnostic::warning(
            W_COMPONENT_WITHOUT_DATA,
            format!(
                "{context} component '{}' binds no entity data",
                component.name
            ),
            span,
            main_path(index),
        ));
    };
    for view in &compilation.model.views {
        walk_body(
            &format!("view '{}'", view.name),
            &view.containers,
            &view.components,
            &mut warn,
        );
    }
    for module in &compilation.model.modules {
        walk_body(
            &format!("module '{}'", module.name),
            &module.containers,
            &module.components,
            &mut warn,
        );
    }
    diags
}

/// W0154 — a main-file view with no domain tie: no `data:` binding or
/// explicit entity columns in its body, no `params`, no module composition,
/// no event handlers anywhere in its body, and no `roles:`/`requires:`
/// policy gate.
fn views_without_domain_binding(compilation: &IfmlCompilation) -> Vec<IfmlDiagnostic> {
    let index = &compilation.index;
    let mut diags = Vec::new();
    for (view, site) in main_view_sites(compilation) {
        let tied = !view.params.is_empty()
            || !view.module_uses.is_empty()
            || !view.roles.is_empty()
            || !view.requires.is_empty()
            || body_has_domain_tie(&view.containers, &view.components, &view.events);
        if tied {
            continue;
        }
        diags.push(IfmlDiagnostic::warning(
            W_VIEW_WITHOUT_DOMAIN,
            format!(
                "view '{}' has no domain binding — bind data, declare params, or compose a module",
                view.name
            ),
            site.name_span,
            main_path(index),
        ));
    }
    diags
}

/// Recurses one body's components and nested containers.
fn walk_body(
    context: &str,
    containers: &[ContainerDeclaration],
    components: &[ComponentDeclaration],
    warn: &mut dyn FnMut(&str, &ComponentDeclaration),
) {
    for component in components {
        warn(context, component);
    }
    for container in containers {
        walk_body(context, &container.containers, &container.components, warn);
    }
}

/// Whether the component carries an entity binding of any exempting kind:
/// a `data:` property, an event handler, a chart (inline spec, chart
/// reference, or bare `type: chart` — charts aggregate by construction), or
/// an explicit `Entity.field` table column (the generation-time spelling).
fn component_binds_data(component: &ComponentDeclaration) -> bool {
    if component
        .properties
        .iter()
        .any(|property| property.key == "data")
    {
        return true;
    }
    if !component.events.is_empty() {
        return true;
    }
    if component.component_type == Some(ComponentType::Chart) {
        return true;
    }
    if matches!(
        &component.spec,
        Some(ComponentSpec::Chart(_) | ComponentSpec::ChartRef(_))
    ) {
        return true;
    }
    columns_spell_entities(component)
}

/// Whether any table column spells an explicit entity (`Entity.field` —
/// the generation-time shape that carries its own row source).
fn columns_spell_entities(component: &ComponentDeclaration) -> bool {
    let Some(ComponentSpec::Table(spec)) = &component.spec else {
        return false;
    };
    spec.columns.iter().any(|column| match column {
        ColumnDef::Field { field, .. } | ColumnDef::Lookup { field, .. } => {
            !field.entity.is_empty()
        }
        ColumnDef::Expression { .. } => false,
    })
}

/// Whether events or data bindings appear anywhere in the body, recursion
/// through nested containers included.
fn body_has_domain_tie(
    containers: &[ContainerDeclaration],
    components: &[ComponentDeclaration],
    events: &[EventHandler],
) -> bool {
    !events.is_empty()
        || components
            .iter()
            .any(|component| component_carries_entity(component) || !component.events.is_empty())
        || containers.iter().any(|container| {
            body_has_domain_tie(
                &container.containers,
                &container.components,
                &container.events,
            )
        })
}

/// The W0154 tie a single component contributes: a `data:` property or an
/// explicit `Entity.field` column. Charts carry neither — a chart-only
/// view is the W0154 class, not the W0153 one.
fn component_carries_entity(component: &ComponentDeclaration) -> bool {
    component
        .properties
        .iter()
        .any(|property| property.key == "data")
        || columns_spell_entities(component)
}
