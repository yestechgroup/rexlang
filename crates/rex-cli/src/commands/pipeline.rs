//! The tail helpers shared by `check`, `ir`, and `gen`: the multi-file
//! `.mox` compile-and-report pipeline, the artifact-JSON output tail,
//! and the `check` exit tail.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use crate::inputs::{compile_sources, read_sources};
use crate::report::report_multi_diagnostics;

/// The multi-file `.mox` pipeline: read the sources, compile them as one
/// multi-package model, and render the diagnostics grouped per file (the
/// `.mox` diagnostics first, then the `import sigil` diagnostics against
/// their rosetta sources).
pub(crate) fn compile_reported_sources(
    files: &[PathBuf],
) -> anyhow::Result<(Vec<(String, String)>, rex_driver::MultiCompilation)> {
    let sources = read_sources(files)?;
    let (compilation, sigil_sources) = compile_sources(&sources)?;
    report_multi_diagnostics(
        &sources,
        &sigil_sources.sources,
        &compilation.diagnostics,
        &compilation.sigil_diagnostics,
    );
    Ok((sources, compilation))
}

/// The `ir` emission tail: write `json` to `out`, or print it when no
/// output path was given.
pub(crate) fn emit_json(json: String, out: Option<&Path>) -> anyhow::Result<()> {
    match out {
        Some(out_path) => std::fs::write(out_path, json)?,
        None => println!("{json}"),
    }
    Ok(())
}

/// The `check` exit tail: print `OK <path>` and succeed when the
/// compilation produced a model, fail otherwise.
pub(crate) fn check_exit(compiled: bool, path: &str) -> ExitCode {
    if compiled {
        println!("OK {path}");
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}
