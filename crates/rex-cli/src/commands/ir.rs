//! `rexlang ir`: compile inputs and emit the artifact as JSON.

use std::path::PathBuf;
use std::process::ExitCode;

use crate::inputs::expand_inputs;

use super::pipeline::{compile_surface, emit_json};

pub(crate) fn run(
    files: Vec<PathBuf>,
    out: Option<PathBuf>,
    expand: bool,
) -> anyhow::Result<ExitCode> {
    let files = expand_inputs(&files)?;
    let Some(outcome) = compile_surface(&files, None, expand)? else {
        return Ok(ExitCode::FAILURE);
    };
    emit_json(outcome.json, out.as_deref())?;
    Ok(ExitCode::SUCCESS)
}
