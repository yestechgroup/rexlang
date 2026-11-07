//! `rexlang manifest <file.ifml>`: compile the interaction flow, gate on
//! the checker, and export the declaration-backed control/route manifest
//! (issue #118).

use std::path::PathBuf;
use std::process::ExitCode;

use crate::inputs::{is_ifml_file, read_ifml_inputs};
use crate::report::report_ifml_diagnostics;

use super::pipeline::emit_json;

pub(crate) fn run(files: Vec<PathBuf>, out: Option<PathBuf>) -> anyhow::Result<ExitCode> {
    let [file] = &files[..] else {
        anyhow::bail!("manifest takes exactly one .ifml interaction-flow file");
    };
    if !is_ifml_file(file) {
        anyhow::bail!(
            "manifest requires an .ifml interaction-flow file, got {}",
            file.display()
        );
    }
    let inputs = read_ifml_inputs(file, None)?;
    let compilation =
        match rex_ifml::compile_ifml_str(&inputs.path, &inputs.source, &inputs.ifml_imports) {
            Ok(compilation) => compilation,
            Err(diagnostics) => {
                report_ifml_diagnostics(&inputs.sources, &diagnostics);
                return Ok(ExitCode::FAILURE);
            }
        };
    let binding =
        rex_ifml::check_ifml_with_actors(&compilation, inputs.domains.as_ref(), &inputs.actors);
    if !binding.is_empty() {
        let sources: Vec<(String, String)> = compilation
            .index
            .files
            .iter()
            .map(|file| (file.path.clone(), file.text.clone()))
            .collect();
        report_ifml_diagnostics(&sources, &binding);
        return Ok(ExitCode::FAILURE);
    }
    let manifest =
        rex_ifml::manifest_ifml(&compilation, &inputs.ifml_imports, inputs.domains.as_ref());
    emit_json(manifest.to_json_pretty()?, out.as_deref())?;
    Ok(ExitCode::SUCCESS)
}
