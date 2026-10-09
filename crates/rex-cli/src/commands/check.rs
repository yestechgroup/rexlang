//! `rexlang check`: compile inputs and render diagnostics grouped per file.

use std::path::PathBuf;
use std::process::ExitCode;

use rex_driver::{compile_actors_str, compile_ddd_str, compile_evt_str};

use crate::inputs::{
    compile_sources, expand_check_inputs, expand_inputs, is_actor_file, is_ddd_file, is_evt_file,
    is_ifml_file, read_actor_pair, read_ddd_design, read_evt_pair, read_ifml_inputs, read_sources,
};
use crate::report::{
    report_actor_diagnostics, report_ddd_diagnostics, report_evt_diagnostics,
    report_ifml_diagnostics, report_multi_diagnostics,
};

pub(crate) fn run(files: Vec<PathBuf>) -> anyhow::Result<ExitCode> {
    // Directory inputs enter batch mode: every supported surface under the
    // directory is checked **independently** (per-file pipeline). Pass
    // several `.mox` files explicitly to compile them as one multi-package
    // model instead.
    if files.iter().any(|file| file.is_dir()) {
        let expanded = expand_check_inputs(&files)?;
        let mut failed = 0usize;
        for file in &expanded {
            match check_one(std::slice::from_ref(file)) {
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
    check_one(&expand_inputs(&files)?)
}

fn check_one(files: &[PathBuf]) -> anyhow::Result<ExitCode> {
    let files = files.to_vec();
    if files.len() == 1 && is_ifml_file(&files[0]) {
        let inputs = read_ifml_inputs(&files[0])?;
        let sources = |compilation: &rex_ifml::IfmlCompilation| -> Vec<(String, String)> {
            compilation
                .index
                .files
                .iter()
                .map(|file| (file.path.clone(), file.text.clone()))
                .collect()
        };
        let compilation =
            match rex_ifml::compile_ifml_str(&inputs.path, &inputs.source, &inputs.ifml_imports) {
                Ok(compilation) => compilation,
                Err(diagnostics) => {
                    report_ifml_diagnostics(&inputs.sources, &diagnostics);
                    return Ok(ExitCode::FAILURE);
                }
            };
        let binding = rex_ifml::check_ifml(&compilation, inputs.domains.as_ref());
        report_ifml_diagnostics(&sources(&compilation), &binding);
        if binding.is_empty() {
            println!("OK {}", inputs.path);
            return Ok(ExitCode::SUCCESS);
        }
        Ok(ExitCode::FAILURE)
    } else if files.len() == 1 && is_ddd_file(&files[0]) {
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
    } else if files.len() == 1 && is_evt_file(&files[0]) {
        let pair = read_evt_pair(&files[0])?;
        let compilation = compile_evt_str(&pair.path, &pair.source, &pair.domains, &pair.imports());
        report_evt_diagnostics(&pair, &compilation);
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
