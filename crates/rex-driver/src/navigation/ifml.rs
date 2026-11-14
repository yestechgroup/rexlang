//! The `.ifml` surface's spanned index: the graph-node projection of the
//! interaction flow's own declarations, plus its cross-surface references.
//!
//! `.ifml` already had a span table ([`rex_ifml::IfmlIndex`]) before the
//! shared contract existed; this module adapts it to
//! [`SurfaceIndex`] so the LSP can resolve through the semantic graph and
//! land on a span exactly like every other surface.
//!
//! # Definitions
//!
//! One per graph node: `module` (a `<Module>` declaration in the entry
//! file), `view` (`<View>`), `container` and `component` (their declaration
//! paths — `Inbox.escalations`), `action` (`<action>`), and `command`
//! (joined subcommand paths). Only the **entry file's** declarations are
//! nodes, matching the emitter: a `use "Module"` splice is an
//! instantiation, not a flattening, and an imported pattern's own file
//! contributes its own flow.
//!
//! The `flow` node is excluded: its name is the file stem, a host fact with
//! no written declaration site — the same exclusion as `file` nodes.
//!
//! # References
//!
//! `use "…"` → `module`; `navigate` / `refresh` / `action` string targets →
//! the interaction tree or an action; a view's `requires:` names →
//! `capability`; its `roles:` names → `actor`; a component's `data:` name →
//! `class`. The names are hints — the graph index decides the binding.

use rex_ifml::{AuthKind, IfmlIndex, IfmlModel, NamedSite};
use rex_ir::graph::kinds;

use super::surface::{ReferenceContext, SurfaceIndex};

/// The `.ifml` surface's spanned index, projected from a compilation's
/// index (`index.files[0]` is the entry file).
///
/// Both halves come from the same parse the compiler walked, so the
/// declaration keys and their spans cannot drift from the graph builder's.
pub fn ifml_surface(model: &IfmlModel, index: &IfmlIndex) -> SurfaceIndex {
    let mut surface = SurfaceIndex::default();
    // The flow node is deliberately not recorded: its name is the file stem,
    // a host fact with no written declaration site — the `file` node's
    // exclusion. It still exists in the graph (reached by `bindsFlow`).

    for module in &model.modules {
        if let Some(site) = index
            .module_decls
            .iter()
            .find(|site| site.file == 0 && site.name == module.name)
        {
            surface.push_definition(
                kinds::MODULE,
                module.name.clone(),
                span(site.name_span),
                span(site.span),
            );
        }
    }
    for view in &model.views {
        let Some(site) = file_site(&index.views, view.name.as_str()) else {
            continue;
        };
        surface.push_definition(
            kinds::VIEW,
            view.name.clone(),
            span(site.name_span),
            span(site.span),
        );
        let mut used = Vec::new();
        walk_members(
            &mut surface,
            index,
            &view.name,
            site.span,
            &view.containers,
            &view.components,
            &mut used,
        );
    }
    for action in &model.actions {
        if let Some(site) = file_site(&index.actions, action.name.as_str()) {
            surface.push_definition(
                kinds::ACTION,
                action.name.clone(),
                span(site.name_span),
                span(site.span),
            );
        }
    }
    let mut commands = Vec::new();
    for command in &model.commands {
        walk_commands(&mut surface, index, "", command, None, &mut commands);
    }

    for use_site in index.module_uses.iter().filter(|site| site.file == 0) {
        surface.push_reference(
            &[kinds::MODULE],
            use_site.target.clone(),
            span(use_site.name_span),
            ReferenceContext::IfmlUse,
        );
    }
    for target in index.action_targets.iter().filter(|site| site.file == 0) {
        let (kinds, context) = match target.action.as_str() {
            "navigate" => (
                [kinds::VIEW, kinds::CONTAINER, kinds::COMPONENT].as_slice(),
                ReferenceContext::IfmlNavigate,
            ),
            "refresh" => (
                [kinds::CONTAINER, kinds::COMPONENT].as_slice(),
                ReferenceContext::IfmlRefresh,
            ),
            _ => ([kinds::ACTION].as_slice(), ReferenceContext::IfmlAction),
        };
        surface.push_reference(
            kinds,
            target.target.clone(),
            span(target.name_span),
            context,
        );
    }
    for site in index.auth_sites.iter().filter(|site| site.file == 0) {
        let (target_kinds, context): (&'static [&'static str], ReferenceContext) = match site.kind {
            AuthKind::Capability => (&[kinds::CAPABILITY], ReferenceContext::IfmlRequires),
            AuthKind::Role => (&[kinds::ACTOR], ReferenceContext::IfmlRoles),
        };
        surface.push_reference(
            target_kinds,
            site.name.clone(),
            span(site.name_span),
            context,
        );
    }
    for site in index.data_sites.iter().filter(|site| site.file == 0) {
        surface.push_reference(
            &[kinds::CLASS],
            site.name.clone(),
            span(site.name_span),
            ReferenceContext::IfmlData,
        );
    }
    surface.sort();
    surface
}

/// The `file == 0` site named `name`, if the entry file declares one.
fn file_site<'a>(sites: &'a [NamedSite], name: &str) -> Option<&'a NamedSite> {
    sites
        .iter()
        .find(|site| site.file == 0 && site.name == name)
}

/// Walks one level of the interaction tree, recording container/component
/// definitions with their declaration paths.
///
/// The model gives the shape, the index gives the spans; a site is matched
/// by name inside its parent's source span, and each site is consumed once
/// (two same-named siblings under one parent are still distinct spans).
fn walk_members(
    surface: &mut SurfaceIndex,
    index: &IfmlIndex,
    prefix: &str,
    parent: (usize, usize),
    containers: &[rex_ir::ifml::ContainerDeclaration],
    components: &[rex_ir::ifml::ComponentDeclaration],
    used: &mut Vec<(usize, usize)>,
) {
    for container in containers {
        let Some(site) = child_site(&index.containers, &container.name, parent, used) else {
            continue;
        };
        let name = crate::graph::ids::qualify(&[prefix, &container.name]);
        surface.push_definition(
            kinds::CONTAINER,
            &name,
            span(site.name_span),
            span(site.span),
        );
        let child = site.span;
        used.push(child);
        walk_members(
            surface,
            index,
            &name,
            child,
            &container.containers,
            &container.components,
            used,
        );
    }
    for component in components {
        let Some(site) = child_site(&index.components, &component.name, parent, used) else {
            continue;
        };
        let name = crate::graph::ids::qualify(&[prefix, &component.name]);
        surface.push_definition(
            kinds::COMPONENT,
            &name,
            span(site.name_span),
            span(site.span),
        );
        used.push(site.span);
    }
}

/// One component/container site named `name` inside `parent`, not yet used.
fn child_site<'a>(
    sites: &'a [NamedSite],
    name: &str,
    parent: (usize, usize),
    used: &[(usize, usize)],
) -> Option<&'a NamedSite> {
    sites.iter().find(|site| {
        site.file == 0
            && site.name == name
            && site.span.0 >= parent.0
            && site.span.1 <= parent.1
            && !used.contains(&site.span)
    })
}

/// Records one command and its nested subcommands under their joined paths.
fn walk_commands(
    surface: &mut SurfaceIndex,
    index: &IfmlIndex,
    prefix: &str,
    command: &rex_ir::ifml::CommandDeclaration,
    parent: Option<(usize, usize)>,
    used: &mut Vec<(usize, usize)>,
) {
    let name = if prefix.is_empty() {
        command.name.clone()
    } else {
        crate::graph::ids::qualify(&[prefix, &command.name])
    };
    let site = index.commands.iter().find(|site| {
        site.file == 0
            && site.name == command.name
            && !used.contains(&site.span)
            && parent.is_none_or(|parent| site.span.0 >= parent.0 && site.span.1 <= parent.1)
    });
    let Some(site) = site else {
        return;
    };
    surface.push_definition(kinds::COMMAND, &name, span(site.name_span), span(site.span));
    let span_here = site.span;
    used.push(span_here);
    for subcommand in &command.subcommands {
        walk_commands(surface, index, &name, subcommand, Some(span_here), used);
    }
}

/// The shared `SurfaceIndex` span type is `rex_syntax::Span`; the `.ifml`
/// index records plain `(start, end)` pairs.
fn span(span: (usize, usize)) -> rex_syntax::Span {
    (span.0..span.1).into()
}
