//! `rexlang check`: compile inputs and render diagnostics grouped per file.

use std::path::PathBuf;
use std::process::ExitCode;

use crate::inputs::{expand_check_inputs, expand_inputs};
use crate::project::Project;

use super::pipeline::compile_surface;

pub(crate) fn run(files: Vec<PathBuf>) -> anyhow::Result<ExitCode> {
    // Directory inputs enter project mode: one snapshot of every supported
    // surface under the directory, each file checked **independently**
    // (per-file pipeline) with its imports resolving map-first from the
    // snapshot and sibling artifacts reused lookup-before-compute. Pass
    // several `.mox` files explicitly to compile them as one multi-package
    // model instead — that path keeps no project context.
    if files.iter().any(|file| file.is_dir()) {
        let expanded = expand_check_inputs(&files)?;
        let mut project = Project::gather(&expanded);
        let mut failed = 0usize;
        for file in &expanded {
            match check_files(std::slice::from_ref(file), Some(&mut project)) {
                Ok(ExitCode::SUCCESS) => {}
                Ok(_) => failed += 1,
                Err(error) => {
                    eprintln!("Error: {error:#}");
                    failed += 1;
                }
            }
        }
        if failed > 0 {
            eprintln!("{failed} of {} files failed", expanded.len());
            return Ok(ExitCode::FAILURE);
        }
        return Ok(ExitCode::SUCCESS);
    }
    check_files(&expand_inputs(&files)?, None)
}

/// The `check` tail over [`compile_surface`]: print `OK <path>` for every
/// compiling source; a `None` outcome means the diagnostics were rendered.
fn check_files(files: &[PathBuf], project: Option<&mut Project>) -> anyhow::Result<ExitCode> {
    let Some(outcome) = compile_surface(files, project, false)? else {
        return Ok(ExitCode::FAILURE);
    };
    for path in &outcome.ok {
        println!("OK {path}");
    }
    Ok(ExitCode::SUCCESS)
}
