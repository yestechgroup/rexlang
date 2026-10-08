//! Diagnostics rendering for the command handlers: the ariadne render
//! helper and the per-file grouped reporters for multi-model, `.actor`,
//! `.ddd`, `.evt`, and `.ifml` compiles.

use rex_driver::{render, ActorCompilation};

use crate::inputs::{ActorPair, DddDesignPair, EvtPair};

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
    let mut lookup: Vec<(&str, &str)> = sources
        .iter()
        .map(|(path, source)| (path.as_str(), source.as_str()))
        .collect();
    lookup.extend(
        rosetta_sources
            .iter()
            .map(|(path, source)| (path.as_str(), source.as_str())),
    );
    render_grouped(&lookup, diagnostics);
    render_grouped(&lookup, sigil_diagnostics);
}

/// Groups diagnostics by file path and renders each group against its
/// source; paths with no known source are skipped.
fn render_grouped(lookup: &[(&str, &str)], diagnostics: &[(String, rex_driver::Diagnostic)]) {
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
        let Some((_, source)) = lookup.iter().find(|(name, _)| *name == path) else {
            continue;
        };
        eprint!("{}", render(path, source, &group));
    }
}

/// Renders an actor compilation's diagnostics grouped per file: each file's
/// diagnostics are ariadne-rendered against that file's own source, the way
/// `check` renders single-model diagnostics.
pub(crate) fn report_actor_diagnostics(pair: &ActorPair, compilation: &ActorCompilation) {
    if compilation.diagnostics.is_empty() {
        return;
    }
    let mut groups: Vec<(&str, Vec<rex_driver::Diagnostic>)> = Vec::new();
    for (path, diagnostic) in &compilation.diagnostics {
        match groups.last_mut() {
            Some((group_path, group)) if *group_path == path.as_str() => {
                group.push(diagnostic.clone())
            }
            _ => groups.push((path.as_str(), vec![diagnostic.clone()])),
        }
    }
    for (path, group) in groups {
        let Some(source) = pair.source_of(path) else {
            continue;
        };
        eprint!("{}", render(path, source, &group));
    }
}

/// Renders a `.ddd` compilation's diagnostics grouped per file: each file's
/// diagnostics are ariadne-rendered against that file's own source, the way
/// `check` renders single-model diagnostics.
pub(crate) fn report_ddd_diagnostics(
    pair: &DddDesignPair,
    compilation: &rex_driver::DddCompilation,
) {
    if compilation.diagnostics.is_empty() {
        return;
    }
    let mut groups: Vec<(&str, Vec<rex_driver::Diagnostic>)> = Vec::new();
    for (path, diagnostic) in &compilation.diagnostics {
        match groups.last_mut() {
            Some((group_path, group)) if *group_path == path.as_str() => {
                group.push(diagnostic.clone())
            }
            _ => groups.push((path.as_str(), vec![diagnostic.clone()])),
        }
    }
    for (path, group) in groups {
        let Some(source) = pair.source_of(path) else {
            continue;
        };
        eprint!("{}", render(path, source, &group));
    }
}

/// Renders an `.evt` compilation's diagnostics grouped per file: each file's
/// diagnostics are ariadne-rendered against that file's own source, the way
/// `check` renders single-model diagnostics.
pub(crate) fn report_evt_diagnostics(pair: &EvtPair, compilation: &rex_driver::EventCompilation) {
    if compilation.diagnostics.is_empty() {
        return;
    }
    let mut groups: Vec<(&str, Vec<rex_driver::Diagnostic>)> = Vec::new();
    for (path, diagnostic) in &compilation.diagnostics {
        match groups.last_mut() {
            Some((group_path, group)) if *group_path == path.as_str() => {
                group.push(diagnostic.clone())
            }
            _ => groups.push((path.as_str(), vec![diagnostic.clone()])),
        }
    }
    for (path, group) in groups {
        let Some(source) = pair.source_of(path) else {
            continue;
        };
        eprint!("{}", render(path, source, &group));
    }
}

/// Renders an `.ifml` compilation's diagnostics (resolver plus typed-binding
/// checker) grouped per file, ariadne-rendered against each file's own
/// source — the main interaction file and every imported `.ifml` file.
pub(crate) fn report_ifml_diagnostics(
    sources: &[(String, String)],
    diagnostics: &[rex_ifml::IfmlDiagnostic],
) {
    if diagnostics.is_empty() {
        return;
    }
    let mut groups: Vec<(&str, Vec<rex_driver::Diagnostic>)> = Vec::new();
    for diagnostic in diagnostics {
        let converted = rex_driver::Diagnostic::error(
            format!("{} {}", diagnostic.code, diagnostic.message),
            Some(rex_driver::Span {
                start: diagnostic.span.0,
                end: diagnostic.span.1,
                context: (),
            }),
        );
        match groups.last_mut() {
            Some((group_path, group)) if *group_path == diagnostic.file.as_str() => {
                group.push(converted)
            }
            _ => groups.push((diagnostic.file.as_str(), vec![converted])),
        }
    }
    for (path, group) in groups {
        let Some((_, source)) = sources.iter().find(|(name, _)| name == path) else {
            continue;
        };
        eprint!("{}", render(path, source, &group));
    }
}
