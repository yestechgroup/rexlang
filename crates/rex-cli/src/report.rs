//! Diagnostics rendering for the command handlers: the ariadne render
//! helper and the per-file grouped reporters for multi-model, `.actor`,
//! `.ddd`, `.evt`, `.dmn`, `.deploy`, `.uc`, `.cmmn`, and `.ifml` compiles.

use rex_driver::{render, ActorCompilation};

use crate::inputs::{ActorPair, DddDesignPair, DeployPair, DomainPair, EvtPair};

/// Renders a compilation's diagnostics grouped per file: each file's
/// diagnostics are ariadne-rendered against that file's own source, the way
/// `check` renders single-model diagnostics. The `.mox` diagnostics render
/// first, then the `import sigil` diagnostics against their rosetta sources.
pub(crate) fn report_multi_diagnostics(
    sources: &[(String, String)],
    rosetta_sources: &[(String, String)],
    diagnostics: &[(String, rex_driver::Diagnostic)],
    sigil_diagnostics: &[(String, rex_driver::Diagnostic)],
) {
    let lookup = |path: &str| {
        sources
            .iter()
            .chain(rosetta_sources)
            .find(|(name, _)| name == path)
            .map(|(_, source)| source.as_str())
    };
    render_grouped(diagnostics, lookup);
    render_grouped(sigil_diagnostics, lookup);
}

/// Renders an actor compilation's diagnostics grouped per file: each file's
/// diagnostics are ariadne-rendered against that file's own source, the way
/// `check` renders single-model diagnostics.
pub(crate) fn report_actor_diagnostics(pair: &ActorPair, compilation: &ActorCompilation) {
    render_grouped(&compilation.diagnostics, |path| pair.source_of(path));
}

/// Renders a `.ddd` compilation's diagnostics grouped per file: each file's
/// diagnostics are ariadne-rendered against that file's own source, the way
/// `check` renders single-model diagnostics.
pub(crate) fn report_ddd_diagnostics(
    pair: &DddDesignPair,
    compilation: &rex_driver::DddCompilation,
) {
    render_grouped(&compilation.diagnostics, |path| pair.source_of(path));
}

/// Renders an `.evt` compilation's diagnostics grouped per file: each file's
/// diagnostics are ariadne-rendered against that file's own source, the way
/// `check` renders single-model diagnostics.
pub(crate) fn report_evt_diagnostics(pair: &EvtPair, compilation: &rex_driver::EventCompilation) {
    render_grouped(&compilation.diagnostics, |path| pair.source_of(path));
}

/// Renders a `.dmn` compilation's diagnostics grouped per file: each file's
/// diagnostics are ariadne-rendered against that file's own source, the way
/// `check` renders single-model diagnostics.
pub(crate) fn report_dmn_diagnostics(pair: &DomainPair, compilation: &rex_driver::DmnCompilation) {
    render_grouped(&compilation.diagnostics, |path| pair.source_of(path));
}

/// Renders a `.deploy` compilation's diagnostics (all tagged with the deploy
/// file's path; its imports arrive pre-compiled and their diagnostics were
/// rendered when gathered).
pub(crate) fn report_deploy_diagnostics(
    pair: &DeployPair,
    compilation: &rex_driver::DeployCompilation,
) {
    render_grouped(&compilation.diagnostics, |path| pair.source_of(path));
}

/// Renders a `.uc` compilation's diagnostics grouped per file: each file's
/// diagnostics are ariadne-rendered against that file's own source, the way
/// `check` renders single-model diagnostics (design, actor, and event
/// imports arrive pre-compiled and their diagnostics were rendered when
/// gathered).
pub(crate) fn report_usecase_diagnostics(
    pair: &DomainPair,
    compilation: &rex_driver::UseCaseCompilation,
) {
    render_grouped(&compilation.diagnostics, |path| pair.source_of(path));
}

/// Renders a `.cmmn` compilation's diagnostics grouped per file,
/// ariadne-rendered against each file's own source — the case file and
/// every imported domain.
pub(crate) fn report_cmmn_diagnostics(
    pair: &DomainPair,
    compilation: &rex_driver::CmmnCompilation,
) {
    render_grouped(&compilation.diagnostics, |path| pair.source_of(path));
}

/// Renders an `.ifml` compilation's diagnostics (resolver, typed-binding
/// checker, and lints) grouped per file, ariadne-rendered against each
/// file's own source — the main interaction file and every imported
/// `.ifml` file. Severity is carried through: lints render as warnings.
pub(crate) fn report_ifml_diagnostics(
    sources: &[(String, String)],
    diagnostics: &[rex_ifml::IfmlDiagnostic],
) {
    let converted: Vec<(String, rex_driver::Diagnostic)> = diagnostics
        .iter()
        .map(|diagnostic| {
            let constructor = if diagnostic.severity.is_error() {
                rex_driver::Diagnostic::error
            } else {
                rex_driver::Diagnostic::warning
            };
            (
                diagnostic.file.clone(),
                constructor(
                    format!("{} {}", diagnostic.code, diagnostic.message),
                    Some(rex_driver::Span {
                        start: diagnostic.span.0,
                        end: diagnostic.span.1,
                        context: (),
                    }),
                ),
            )
        })
        .collect();
    render_grouped(&converted, |path| {
        sources
            .iter()
            .find(|(name, _)| name == path)
            .map(|(_, source)| source.as_str())
    });
}

/// Groups diagnostics by file path and renders each group against its
/// source; paths with no known source are skipped. Groups keep the
/// diagnostics' own order and file contiguity.
fn render_grouped<'d, 's>(
    diagnostics: &'d [(String, rex_driver::Diagnostic)],
    source_of: impl Fn(&str) -> Option<&'s str>,
) {
    if diagnostics.is_empty() {
        return;
    }
    let mut groups: Vec<(&str, Vec<rex_driver::Diagnostic>)> = Vec::new();
    for (path, diagnostic) in diagnostics {
        match groups.last_mut() {
            Some((group_path, group)) if *group_path == path.as_str() => {
                group.push(diagnostic.clone())
            }
            _ => groups.push((path.as_str(), vec![diagnostic.clone()])),
        }
    }
    for (path, group) in groups {
        let Some(source) = source_of(path) else {
            continue;
        };
        eprint!("{}", render(path, source, &group));
    }
}
