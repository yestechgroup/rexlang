//! `rexlang check`: compile inputs and render diagnostics grouped per file.

use std::path::PathBuf;
use std::process::ExitCode;

use rex_driver::{compile_actors_str, compile_ddd_str};

use crate::inputs::{
    compile_sources, expand_inputs, is_actor_file, is_ddd_file, read_actor_pair, read_ddd_design,
    read_sources,
};
use crate::report::{report_actor_diagnostics, report_ddd_diagnostics, report_multi_diagnostics};

pub(crate) fn run(files: Vec<PathBuf>) -> anyhow::Result<ExitCode> {
    let files = expand_inputs(&files)?;
    if files.len() == 1 && is_ddd_file(&files[0]) {
        let design = read_ddd_design(&files[0])?;
        let compilation = compile_ddd_str(
            &design.path,
            &design.source,
            &design.domains,
            &design.imports(),
        );
        report_ddd_diagnostics(&design, &compilation);
        if compilation.model.is_some() {
            println!("OK {}", design.path);
            Ok(ExitCode::SUCCESS)
        } else {
            Ok(ExitCode::FAILURE)
        }
    } else if files.len() == 1 && is_actor_file(&files[0]) {
        let pair = read_actor_pair(&files[0])?;
        let compilation =
            compile_actors_str(&pair.path, &pair.source, &pair.domains, &pair.imports());
        report_actor_diagnostics(&pair, &compilation);
        if compilation.model.is_some() {
            println!("OK {}", pair.path);
            Ok(ExitCode::SUCCESS)
        } else {
            Ok(ExitCode::FAILURE)
        }
    } else {
        let sources = read_sources(&files)?;
        let (compilation, sigil_sources) = compile_sources(&sources)?;
        report_multi_diagnostics(
            &sources,
            &sigil_sources.sources,
            &compilation.diagnostics,
            &compilation.sigil_diagnostics,
        );
        if compilation.model.is_some() {
            for (path, _) in &sources {
                println!("OK {path}");
            }
            Ok(ExitCode::SUCCESS)
        } else {
            Ok(ExitCode::FAILURE)
        }
    }
}
