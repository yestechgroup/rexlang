//! The interaction cross-reference pass: every `action(...)` /
//! `navigate(...)` / `refresh(...)` string resolves union-wide against the
//! declarations of the whole compilation (`E0104`–`E0106`).

use std::collections::BTreeSet;

use crate::resolve::{IfmlCompilation, IfmlDiagnostic};

use super::reachability::{instantiated_modules, site_is_instantiated};
use super::{E_UNKNOWN_ACTION, E_UNKNOWN_COMPONENT, E_UNKNOWN_VIEW};

/// The interaction cross-reference pass: every navigation/action string
/// reference on an **instantiated** site resolves to a declaration,
/// union-wide across the whole compilation (the entry file plus its
/// imports) —
///
/// - `action("X")` → an `action "X" { target: ...; }` declaration
///   (`E0104`, with the add-it message: declarations carry the `target`
///   the lowering needs, so an implicit declaration from a reference
///   would have nothing to bind to);
/// - `navigate("X")` → a `view "X"` **or container** declaration
///   (`E0105` — wizard steps navigate between containers);
/// - `refresh("X")` → a component declaration (`E0106`).
///
/// Spans come from the index's
/// [`TargetSite`](crate::index::TargetSite)s (the referenced string
/// token itself — editor squiggles land on the name). A module an app
/// instantiates via `use` contributes its body's references to the
/// app's surface — the reachability rule of [`instantiated_modules`]
/// closes the silently-missing-control class for pattern compositions.
/// An uninstantiated module keeps its references to its own
/// compilation's report.
pub(super) fn check_references(compilation: &IfmlCompilation) -> Vec<IfmlDiagnostic> {
    let index = &compilation.index;
    let actions: BTreeSet<&str> = index
        .actions
        .iter()
        .map(|site| site.name.as_str())
        .collect();
    let views: BTreeSet<&str> = index.views.iter().map(|site| site.name.as_str()).collect();
    let containers: BTreeSet<&str> = index
        .containers
        .iter()
        .map(|site| site.name.as_str())
        .collect();
    let components: BTreeSet<&str> = index
        .components
        .iter()
        .map(|site| site.name.as_str())
        .collect();
    let reached = instantiated_modules(index);
    let mut diags = Vec::new();
    for site in &index.action_targets {
        if !site_is_instantiated(&reached, site.file, site.name_span) {
            continue;
        }
        let (code, message) = match site.action.as_str() {
            "navigate" => (
                E_UNKNOWN_VIEW,
                format!(
                    "navigate target '{}' is not a declared view or container",
                    site.target
                ),
            ),
            "refresh" => (
                E_UNKNOWN_COMPONENT,
                format!(
                    "refresh target '{}' is not a declared component",
                    site.target
                ),
            ),
            _ => (
                E_UNKNOWN_ACTION,
                format!(
                    "action '{}' is not declared — add action \"{}\" {{ target: <Entity>; }}",
                    site.target, site.target
                ),
            ),
        };
        let known = match site.action.as_str() {
            "navigate" => {
                views.contains(site.target.as_str()) || containers.contains(site.target.as_str())
            }
            "refresh" => components.contains(site.target.as_str()),
            _ => actions.contains(site.target.as_str()),
        };
        if known {
            continue;
        }
        diags.push(IfmlDiagnostic::error(
            code,
            message,
            site.name_span,
            index.files[site.file].path.clone(),
        ));
    }
    diags
}
