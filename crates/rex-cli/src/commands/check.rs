//! `rexlang check`: compile inputs and render diagnostics grouped per file.
//!
//! Directory batch mode runs two phases: the per-file project pipeline
//! (every file checked independently with its imports resolving map-first
//! from the snapshot), then the union integrity pass (rexlang#114) — the
//! standalone-compiled artifacts of every successful check are gathered
//! into import-connected applications and [`rex_driver::check_integrity`]
//! diagnoses the dangling cross-layer wiring each. Single-file mode has no
//! union context and stays per-surface.

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
        // Second phase — the union integrity pass (rexlang#114): the
        // standalone-compiled artifacts cluster into import-connected
        // applications and the cross-layer wiring is diagnosed per
        // application. Quiet in the common case; error-severity findings
        // fail the batch.
        let integrity_errors = super::integrity::run_union_pass(&expanded)?;
        if failed > 0 {
            eprintln!("{failed} of {} files failed", expanded.len());
        }
        if integrity_errors > 0 {
            eprintln!(
                "{integrity_errors} cross-surface integrity errors in the checked applications"
            );
            return Ok(ExitCode::FAILURE);
        }
        if failed > 0 {
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
