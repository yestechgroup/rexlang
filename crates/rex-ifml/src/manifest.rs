//! The control/route manifest emitter: derives the declaration-backed
//! [`IfmlManifest`] from a compiled `.ifml` interaction model (issue
//! #118).
//!
//! [`crate::compile_ifml_str`] resolves modules; [`crate::check_ifml`]
//! proves every interaction reference terminates at a declaration. This
//! module is the *export* stage on top: it walks the **expanded** view
//! tree ([`crate::expand_ifml`] — module `use` sites inlined, overrides
//! applied), the entry file's action declarations, and the raw command
//! tree, and lowers them into the versioned
//! [`rex_ir::ifml_manifest::IfmlManifest`].
//!
//! # The derivation contract
//!
//! - **Declaration-backed by construction.** The host gates emission on
//!   the checker (`rexlang manifest` runs the same compile → check path
//!   `ir` does), so every emitted edge's target resolved — a dangling
//!   reference is a checker error *before* the manifest can exist.
//!   Consumers cannot re-derive resolution and cannot synthesize
//!   endpoints; the #112 defect class is structurally impossible at this
//!   artifact boundary.
//! - **Expanded controls and edges.** Everything a `use` splices in is
//!   indistinguishable from app-authored UI: pattern-contributed
//!   components appear as controls of the consuming view, and their
//!   handlers appear as edges.
//! - **Entry-file scope.** Routes, actions, and commands are the entry
//!   file's own declarations (the pattern library declares modules, not
//!   views or actions, so example manifests are complete); edge targets
//!   are names the checker proved resolve union-wide.
//! - **Resolution is best-effort, never failing.** `data:`/`target:`
//!   entity names resolve to `(package, class)` when the host provides
//!   the domain union and the name is unique; otherwise the bare
//!   spelling rides alone. The emitter is total — it never errors.
//! - **Deterministic order.** Views in declaration order; per view:
//!   view-level edges, then level components (control + its edges), then
//!   containers (edges, then depth-first recursion); commands depth-first
//!   pre-order, subcommands flattened to invocation paths.

use std::collections::BTreeMap;

use rex_ir::ifml::{
    ActionDeclaration, CommandDeclaration, ComponentDeclaration, ComponentSpec,
    ContainerDeclaration, EventAction, EventHandler, ViewDeclaration,
};
use rex_ir::ifml_manifest::{
    IfmlManifest, ManifestAction, ManifestCommand, ManifestControl, ManifestEdge, ManifestEdgeKind,
    ManifestEdgeSource, ManifestRoute, ResolvedName,
};
use rex_ir::Model;

use crate::resolve::{IfmlCompilation, IfmlImports};

/// Derives the control/route manifest of a compiled interaction model.
/// `imports` supplies the imported files' sources for the expansion (the
/// artifact carries only the entry file's declarations); `domains` is the
/// optional domain union the `data:`/`target:` entity names resolve
/// against.
///
/// The host must run [`crate::check_ifml`] first: the manifest's
/// declaration-backed guarantee rests on the checker's verdict.
///
/// ```
/// use rex_ifml::{compile_ifml_str, check_ifml, manifest_ifml, IfmlImports};
///
/// let source = r#"
/// view "List" {
///     component "grid" {
///         type: list;
///         data: Customer;
///
///         on select(row) -> navigate("Detail", { id: row.id });
///         on open -> action("Open");
///     }
/// }
///
/// view "Detail" {
///     component "form" {
///         type: form;
///         data: Customer;
///     }
/// }
///
/// action "Open" {
///     target: Customer;
/// }
/// "#;
/// let compilation = compile_ifml_str("app.ifml", source, &IfmlImports::default())
///     .expect("resolves");
/// assert!(check_ifml(&compilation, None).is_empty());
/// let manifest = manifest_ifml(&compilation, &IfmlImports::default(), None);
/// assert_eq!(manifest.views.len(), 2);
/// assert_eq!(manifest.edges.len(), 2);
/// assert_eq!(manifest.edges[0].target, "Detail");
/// assert_eq!(manifest.actions.len(), 1);
/// ```
pub fn manifest_ifml(
    compilation: &IfmlCompilation,
    imports: &IfmlImports,
    domains: Option<&Model>,
) -> IfmlManifest {
    let entities = Entities::new(domains);
    let expanded = crate::expand_ifml(compilation, imports);

    let mut manifest = IfmlManifest::new();
    for view in &expanded.views {
        manifest.views.push(route_of(view));
        let mut controls = Vec::new();
        let mut edges = Vec::new();
        edges.extend(handlers_of(&view.events, view_source(view, None)));
        for component in &view.components {
            controls.push(control_of(component, view, &[], entities.as_ref()));
            edges.extend(handlers_of(
                &component.events,
                view_source(view, Some(&component.name)),
            ));
        }
        for container in &view.containers {
            walk_container(
                container,
                view,
                &[],
                entities.as_ref(),
                &mut controls,
                &mut edges,
            );
        }
        manifest.controls.extend(controls);
        manifest.edges.extend(edges);
    }
    for action in &compilation.model.actions {
        manifest.actions.push(action_of(action, entities.as_ref()));
    }
    for command in &compilation.model.commands {
        walk_command(command, &mut Vec::new(), &mut manifest);
    }
    manifest
}

/// The bare-name class resolver over the optional domain union: a name
/// resolves when exactly one package declares the class (the checker's
/// `data:` discipline; ambiguity is already its `E0100`).
struct Entities {
    classes: BTreeMap<String, Vec<(String, String)>>,
}

impl Entities {
    fn new(model: Option<&Model>) -> Option<Self> {
        let model = model?;
        let mut classes: BTreeMap<String, Vec<(String, String)>> = BTreeMap::new();
        for package in &model.packages {
            for class in &package.classes {
                classes
                    .entry(class.name.clone())
                    .or_default()
                    .push((package.name.clone(), class.name.clone()));
            }
        }
        Some(Self { classes })
    }

    /// The `(package, class)` a bare name denotes, when unique.
    fn resolve(&self, bare: &str) -> Option<ResolvedName> {
        let matches = self.classes.get(bare)?;
        let [(package, name)] = matches.as_slice() else {
            return None;
        };
        Some(ResolvedName {
            package: package.clone(),
            name: name.clone(),
        })
    }
}

fn route_of(view: &ViewDeclaration) -> ManifestRoute {
    ManifestRoute {
        name: view.name.clone(),
        label: view.label.clone(),
        is_landmark: view.is_landmark,
        is_xor: view.is_xor,
        is_modal: view.is_modal,
        params: view.params.clone(),
        requires: view.requires.clone(),
        roles: view.roles.clone(),
        condition: view.condition.clone(),
    }
}

fn view_source(view: &ViewDeclaration, component: Option<&str>) -> ManifestEdgeSource {
    ManifestEdgeSource::View {
        view: view.name.clone(),
        component: component.map(str::to_string),
    }
}

fn control_of(
    component: &ComponentDeclaration,
    view: &ViewDeclaration,
    containers: &[String],
    entities: Option<&Entities>,
) -> ManifestControl {
    let mut control = ManifestControl {
        view: view.name.clone(),
        containers: containers.to_vec(),
        name: component.name.clone(),
        component_type: component.component_type.clone(),
        data: None,
        resolved_data: None,
        fields: Vec::new(),
        columns: Vec::new(),
        condition: component.condition.clone(),
    };
    for property in &component.properties {
        if property.key == "data" {
            if let Some(bare) = property_value_name(&property.value) {
                control.data = Some(bare);
            }
        }
    }
    if let (Some(entities), Some(bare)) = (entities, control.data.as_deref()) {
        control.resolved_data = entities.resolve(bare);
    }
    match component.spec.as_ref() {
        Some(ComponentSpec::Form(form)) => control.fields = form.fields.clone(),
        Some(ComponentSpec::Table(table)) => control.columns = table.columns.clone(),
        Some(ComponentSpec::Chart(_) | ComponentSpec::ChartRef(_)) | None => {}
    }
    control
}

fn walk_container(
    container: &ContainerDeclaration,
    view: &ViewDeclaration,
    path: &[String],
    entities: Option<&Entities>,
    controls: &mut Vec<ManifestControl>,
    edges: &mut Vec<ManifestEdge>,
) {
    edges.extend(handlers_of(&container.events, view_source(view, None)));
    let mut inner = path.to_vec();
    inner.push(container.name.clone());
    for component in &container.components {
        controls.push(control_of(component, view, &inner, entities));
        edges.extend(handlers_of(
            &component.events,
            view_source(view, Some(&component.name)),
        ));
    }
    for nested in &container.containers {
        walk_container(nested, view, &inner, entities, controls, edges);
    }
}

fn action_of(action: &ActionDeclaration, entities: Option<&Entities>) -> ManifestAction {
    let mut manifest_action = ManifestAction {
        name: action.name.clone(),
        target: None,
        resolved_target: None,
    };
    for property in &action.properties {
        if property.key == "target" {
            if let Some(bare) = property_value_name(&property.value) {
                manifest_action.target = Some(bare);
            }
        }
    }
    if let (Some(entities), Some(bare)) = (entities, manifest_action.target.as_deref()) {
        manifest_action.resolved_target = entities.resolve(bare);
    }
    manifest_action
}

fn walk_command(command: &CommandDeclaration, path: &mut Vec<String>, manifest: &mut IfmlManifest) {
    path.push(command.name.clone());
    let manifest_command = ManifestCommand {
        path: path.clone(),
        description: command.description.clone(),
        arguments: command.arguments.clone(),
        options: command.options.clone(),
        prompts: command.prompts.clone(),
        requires: command.requires.clone(),
        condition: command.condition.clone(),
    };
    manifest.commands.push(manifest_command);
    for handler in &command.events {
        manifest.edges.extend(handlers_of(
            std::slice::from_ref(handler),
            ManifestEdgeSource::Command { path: path.clone() },
        ));
    }
    for subcommand in &command.subcommands {
        walk_command(subcommand, path, manifest);
    }
    path.pop();
}

/// Lowers every handler in source order; nested action-body handlers
/// (`on success`/`on error`) recurse onto the same source.
fn handlers_of(handlers: &[EventHandler], source: ManifestEdgeSource) -> Vec<ManifestEdge> {
    let mut edges = Vec::new();
    for handler in handlers {
        edge_of(handler, &source, &mut edges);
    }
    edges
}

fn edge_of(handler: &EventHandler, source: &ManifestEdgeSource, edges: &mut Vec<ManifestEdge>) {
    let (kind, target, bindings) = match &handler.action {
        EventAction::Navigate { target, binding } => (
            ManifestEdgeKind::Navigate,
            target.clone(),
            binding
                .as_ref()
                .map(|binding| binding.pairs.clone())
                .unwrap_or_default(),
        ),
        EventAction::Refresh { target, binding } => (
            ManifestEdgeKind::Refresh,
            target.clone(),
            binding
                .as_ref()
                .map(|binding| binding.pairs.clone())
                .unwrap_or_default(),
        ),
        EventAction::ActionInvocation { name, .. } => {
            (ManifestEdgeKind::Action, name.clone(), Vec::new())
        }
        EventAction::Stay => return,
    };
    edges.push(ManifestEdge {
        source: source.clone(),
        trigger: handler.event_type.clone(),
        requires: handler.requires.clone(),
        condition: handler.condition.clone(),
        kind,
        target,
        bindings,
    });
    // Nested action-body handlers (`on success`/`on error`) become their
    // own edges, after the invocation that carries them, on the same
    // source.
    if let EventAction::ActionInvocation {
        body: Some(body), ..
    } = &handler.action
    {
        for nested in &body.handlers {
            edge_of(nested, source, edges);
        }
    }
}

/// The bare entity name a property value spells: `data: Customer` /
/// `target: Customer` are identifiers, quoted spellings are strings;
/// anything else (expressions, arrays) carries no name.
fn property_value_name(value: &rex_ir::ifml::ValueExpression) -> Option<String> {
    match value {
        rex_ir::ifml::ValueExpression::Identifier(name) => Some(name.clone()),
        rex_ir::ifml::ValueExpression::String(text) => Some(text.clone()),
        _ => None,
    }
}
