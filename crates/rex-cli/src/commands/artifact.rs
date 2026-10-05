//! `rexlang artifact check`: validate wire-format artifacts without the
//! originating model.

use std::path::PathBuf;
use std::process::ExitCode;

use crate::artifact_check;

pub(crate) fn check(files: Vec<PathBuf>) -> anyhow::Result<ExitCode> {
    if files.is_empty() {
        anyhow::bail!("no input files");
    }
    let mut failed = false;
    for file in &files {
        let text = match std::fs::read_to_string(file) {
            Ok(text) => text,
            Err(error) => {
                eprintln!("error: cannot read {}: {error}", file.display());
                failed = true;
                continue;
            }
        };
        match artifact_check::check_str(&file.display().to_string(), &text) {
            Ok(_) => println!("OK {}", file.display()),
            Err(errors) => {
                for error in errors {
                    eprintln!("error: {error}");
                }
                failed = true;
            }
        }
    }
    if failed {
        Ok(ExitCode::FAILURE)
    } else {
        Ok(ExitCode::SUCCESS)
    }
}
