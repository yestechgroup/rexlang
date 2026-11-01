//! The shared scaffolding of the per-surface compile engines: the
//! diagnostic prologue and the artifact tail. Engines differ in their
//! import-matching rule (exact path vs the `.deploy` lexical rule, two
//! vs four buckets) — that rule stays in the engine; this module owns
//! only the plumbing around it, including the pinned diagnostic order:
//! the file's parse diagnostics first, then its import diagnostics,
//! then the imported domains' diagnostics under their own paths, and —
//! last — the file's own lowering diagnostics.

use crate::diagnostic::Diagnostic;
use crate::lower::DomainUnit;

/// Opens an engine's diagnostic list, in the pinned order: the surface
/// file's parse diagnostics tagged with its path, the engine's
/// import-provision diagnostics (also tagged with the path), then every
/// imported domain's diagnostics under their own paths, in import order.
pub(crate) fn engine_diags(
    path: &str,
    parse_diagnostics: &[Diagnostic],
    import_diagnostics: Vec<Diagnostic>,
    domains: &[DomainUnit],
) -> Vec<(String, Diagnostic)> {
    let mut diags: Vec<(String, Diagnostic)> = parse_diagnostics
        .iter()
        .map(|diagnostic| (path.to_string(), diagnostic.clone()))
        .collect();
    diags.extend(
        import_diagnostics
            .into_iter()
            .map(|diagnostic| (path.to_string(), diagnostic)),
    );
    for unit in domains {
        for diagnostic in &unit.diagnostics {
            diags.push((unit.path.clone(), diagnostic.clone()));
        }
    }
    diags
}

/// Closes an engine: the file's own lowering diagnostics append last,
/// tagged with its path, and the artifact is withheld when any diagnostic
/// anywhere is an error.
pub(crate) fn engine_finish<Model>(
    path: &str,
    local: Vec<Diagnostic>,
    mut diags: Vec<(String, Diagnostic)>,
    model: Model,
) -> (Option<Model>, Vec<(String, Diagnostic)>) {
    diags.extend(
        local
            .into_iter()
            .map(|diagnostic| (path.to_string(), diagnostic)),
    );
    let blocked = diags.iter().any(|(_, diagnostic)| diagnostic.is_error());
    ((!blocked).then_some(model), diags)
}
