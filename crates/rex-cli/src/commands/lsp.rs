//! `rexlang lsp`: serve the language server over stdin/stdout.

use std::process::ExitCode;

pub(crate) fn run() -> anyhow::Result<ExitCode> {
    rex_lsp::run_stdio()?;
    Ok(ExitCode::SUCCESS)
}
