//! `rexlang completions` and `rexlang man`: developer-experience output
//! generated from the CLI's own clap definition (WI-9 of v0.2, #173
//! Phase 1) — the completion script and the man page can never drift from
//! the real argument surface, because both are rendered from
//! [`crate::Cli`] itself.

use std::io::Write;
use std::path::PathBuf;
use std::process::ExitCode;

use clap::CommandFactory;
use clap_complete::Shell;

/// The clap command graph the generators render from.
fn rexlang_command() -> clap::Command {
    crate::Cli::command()
}

/// Writes the completion script for `shell` to stdout.
pub(crate) fn run_completions(shell: Shell) -> anyhow::Result<ExitCode> {
    let mut command = rexlang_command();
    let name = command.get_name().to_string();
    clap_complete::generate(shell, &mut command, name, &mut std::io::stdout().lock());
    Ok(ExitCode::SUCCESS)
}

/// Renders the `rexlang` man page (roff) to stdout, or into `out`.
pub(crate) fn run_man(out: Option<PathBuf>) -> anyhow::Result<ExitCode> {
    let mut rendered = Vec::new();
    clap_mangen::Man::new(rexlang_command()).render(&mut rendered)?;
    let rendered = String::from_utf8(rendered)
        .map_err(|error| anyhow::anyhow!("man page is not UTF-8: {error}"))?;
    match out {
        Some(path) => {
            std::fs::write(&path, rendered.as_bytes())
                .map_err(|error| anyhow::anyhow!("cannot write {}: {error}", path.display()))?;
        }
        None => {
            std::io::stdout()
                .write_all(rendered.as_bytes())
                .map_err(|error| anyhow::anyhow!("cannot write stdout: {error}"))?;
        }
    }
    Ok(ExitCode::SUCCESS)
}
