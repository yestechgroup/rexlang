//! `rexlang build`: compile every input file and write each artifact JSON
//! into the output directory — the per-file `rexlang ir` artifact for the
//! file's surface, one artifact per file.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use crate::inputs::expand_check_inputs;
use crate::project::Project;

use super::pipeline::{compile_surface, emit_json};

pub(crate) fn run(files: Vec<PathBuf>, out: PathBuf) -> anyhow::Result<ExitCode> {
    // The same input-expansion semantics as `check`: directories enter
    // project mode (one snapshot, per-file pipeline); explicit files
    // compile in their own context — one artifact per file either way.
    let project_mode = files.iter().any(|file| file.is_dir());
    let expanded = if project_mode {
        expand_check_inputs(&files)?
    } else {
        if files.is_empty() {
            anyhow::bail!("no input files");
        }
        let mut expanded = files;
        expanded.sort();
        expanded
    };
    let mut project = project_mode.then(|| Project::gather(&expanded));
    let mut failed = 0usize;
    for file in &expanded {
        match compile_surface(std::slice::from_ref(file), project.as_mut(), false) {
            Ok(Some(outcome)) => {
                let artifact_path = artifact_path(&out, file);
                if let Some(parent) = artifact_path.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                emit_json(outcome.json, Some(&artifact_path))?;
            }
            Ok(None) => failed += 1,
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
    Ok(ExitCode::SUCCESS)
}

/// `<out>/<input path as given>.json`: the input path's components are
/// appended under `out` — an absolute input mirrors its own path *inside*
/// `out`, which `Path::join` would otherwise swallow as an absolute
/// replacement.
fn artifact_path(out: &Path, input: &Path) -> PathBuf {
    let suffixed = format!("{}.json", input.display());
    let mut path = out.to_path_buf();
    for component in Path::new(&suffixed).components() {
        match component {
            std::path::Component::RootDir | std::path::Component::Prefix(_) => {}
            component => path.push(component.as_os_str()),
        }
    }
    path
}
