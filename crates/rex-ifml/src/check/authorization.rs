//! The authorization pass: `requires:`/`roles:` names resolve against the
//! actor models the host provides — capability names for `requires:`
//! (`E0107`), actor names for `roles:` (`E0108`), bare-name union-wide
//! across all blocks. Sites and spans come from the index's `auth_sites`.

use std::collections::BTreeSet;

use rex_ir::ActorModel;

use crate::index::AuthKind;
use crate::resolve::{IfmlCompilation, IfmlDiagnostic};

use super::reachability::{instantiated_modules, site_is_instantiated};
use super::{E_UNKNOWN_CAPABILITY, E_UNKNOWN_ROLE};

/// The authorization pass: every `requires:`/`roles:` name on an
/// **instantiated** site resolves union-wide across all provided actor
/// models' blocks — `requires:` against declared capability names
/// (`E0107`), `roles:` against declared actor names (`E0108`). Spans
/// come from the index's [`AuthSite`](crate::index::AuthSite)s (the name
/// token itself — editor squiggles land on the name). A module an app
/// instantiates via `use` contributes its sites to the app's surface
/// (the reachability rule of [`instantiated_modules`]); an uninstantiated
/// module keeps its names to its own compilation's report.
pub(super) fn check_authorization(
    compilation: &IfmlCompilation,
    actors: &[ActorModel],
) -> Vec<IfmlDiagnostic> {
    if actors.is_empty() {
        return Vec::new();
    }
    let capabilities: BTreeSet<&str> = actors
        .iter()
        .flat_map(|model| model.blocks.iter())
        .flat_map(|block| block.capabilities.iter())
        .map(|capability| capability.name.as_str())
        .collect();
    let roles: BTreeSet<&str> = actors
        .iter()
        .flat_map(|model| model.blocks.iter())
        .flat_map(|block| block.actors.iter())
        .map(|actor| actor.name.as_str())
        .collect();
    let index = &compilation.index;
    let reached = instantiated_modules(index);
    let mut diags = Vec::new();
    for site in &index.auth_sites {
        if !site_is_instantiated(&reached, site.file, site.name_span) {
            continue;
        }
        let (code, message) = match site.kind {
            AuthKind::Capability => {
                if capabilities.contains(site.name.as_str()) {
                    continue;
                }
                (
                    E_UNKNOWN_CAPABILITY,
                    format!(
                        "capability '{}' is not declared — add \"capability {} on <Class>;\" to an actors block",
                        site.name, site.name
                    ),
                )
            }
            AuthKind::Role => {
                if roles.contains(site.name.as_str()) {
                    continue;
                }
                (
                    E_UNKNOWN_ROLE,
                    format!(
                        "role '{}' is not declared — add \"actor {};\" to an actors block",
                        site.name, site.name
                    ),
                )
            }
        };
        diags.push(IfmlDiagnostic::error(
            code,
            message,
            site.name_span,
            index.files[site.file].path.clone(),
        ));
    }
    diags
}
