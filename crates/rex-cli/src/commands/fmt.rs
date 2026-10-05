//! `rexlang fmt`: format `.mox`, `.actor`, `.ddd`, or `.evt` files in
//! place, preserving comments.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use crate::inputs::{is_actor_file, is_ddd_file, is_evt_file};

pub(crate) fn run(check: bool, files: Vec<PathBuf>) -> anyhow::Result<ExitCode> {
    if files.is_empty() || files.iter().any(|file| file == Path::new("-")) {
        let mut source = String::new();
        std::io::stdin()
            .read_to_string(&mut source)
            .map_err(|error| anyhow::anyhow!("cannot read stdin: {error}"))?;
        let formatted = rex_syntax::fmt::format(&source)?;
        std::io::stdout()
            .write_all(formatted.as_bytes())
            .map_err(|error| anyhow::anyhow!("cannot write stdout: {error}"))?;
        return Ok(ExitCode::SUCCESS);
    }
    let mut unformatted = false;
    for file in &files {
        let source = std::fs::read_to_string(file)
            .map_err(|error| anyhow::anyhow!("cannot read {}: {error}", file.display()))?;
        let formatted = if is_ddd_file(file) {
            rex_syntax::fmt::format_ddd(&source)?
        } else if is_actor_file(file) {
            rex_syntax::fmt::format_actors(&source)?
        } else if is_evt_file(file) {
            rex_syntax::fmt::format_evt(&source)?
        } else {
            rex_syntax::fmt::format(&source)?
        };
        if formatted != source {
            unformatted = true;
            if check {
                println!("would reformat: {}", file.display());
            } else {
                std::fs::write(file, formatted)
                    .map_err(|error| anyhow::anyhow!("cannot write {}: {error}", file.display()))?;
            }
        }
    }
    if check && unformatted {
        Ok(ExitCode::FAILURE)
    } else {
        Ok(ExitCode::SUCCESS)
    }
}
