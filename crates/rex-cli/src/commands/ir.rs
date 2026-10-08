//! `rexlang ir`: compile inputs and emit the artifact as JSON.

use std::path::PathBuf;
use std::process::ExitCode;

use rex_driver::{compile_actors_str, compile_ddd_str, compile_evt_str};

use crate::inputs::{
    compile_sources, expand_inputs, is_actor_file, is_ddd_file, is_evt_file, is_ifml_file,
    read_actor_pair, read_ddd_design, read_evt_pair, read_ifml_inputs, read_sources,
};
use crate::report::{
    report_actor_diagnostics, report_ddd_diagnostics, report_evt_diagnostics,
    report_ifml_diagnostics, report_multi_diagnostics,
};

pub(crate) fn run(files: Vec<PathBuf>, out: Option<PathBuf>) -> anyhow::Result<ExitCode> {
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
        let Some(ddd_model) = compilation.model else {
            return Ok(ExitCode::FAILURE);
        };
        let json = ddd_model.to_json_pretty()?;
        match out {
            Some(out_path) => std::fs::write(out_path, json)?,
            None => println!("{json}"),
        }
        Ok(ExitCode::SUCCESS)
    } else if files.len() == 1 && is_actor_file(&files[0]) {
        let pair = read_actor_pair(&files[0])?;
        let compilation =
            compile_actors_str(&pair.path, &pair.source, &pair.domains, &pair.imports());
        report_actor_diagnostics(&pair, &compilation);
        let Some(actor_model) = compilation.model else {
            return Ok(ExitCode::FAILURE);
        };
        let json = actor_model.to_json_pretty()?;
        match out {
            Some(out_path) => std::fs::write(out_path, json)?,
            None => println!("{json}"),
        }
        Ok(ExitCode::SUCCESS)
    } else if files.len() == 1 && is_ifml_file(&files[0]) {
        let inputs = read_ifml_inputs(&files[0])?;
        let compilation =
            match rex_ifml::compile_ifml_str(&inputs.path, &inputs.source, &inputs.ifml_imports) {
                Ok(compilation) => compilation,
                Err(diagnostics) => {
                    report_ifml_diagnostics(&inputs.sources, &diagnostics);
                    return Ok(ExitCode::FAILURE);
                }
            };
        let binding = rex_ifml::check_ifml(&compilation, inputs.domains.as_ref());
        if !binding.is_empty() {
            let sources: Vec<(String, String)> = compilation
                .index
                .files
                .iter()
                .map(|file| (file.path.clone(), file.text.clone()))
                .collect();
            report_ifml_diagnostics(&sources, &binding);
            return Ok(ExitCode::FAILURE);
        }
        let json = compilation.model.to_json_pretty()?;
        match out {
            Some(out_path) => std::fs::write(out_path, json)?,
            None => println!("{json}"),
        }
        Ok(ExitCode::SUCCESS)
    } else if files.len() == 1 && is_evt_file(&files[0]) {
        let pair = read_evt_pair(&files[0])?;
        let compilation = compile_evt_str(&pair.path, &pair.source, &pair.domains, &pair.imports());
        report_evt_diagnostics(&pair, &compilation);
        let Some(event_model) = compilation.model else {
            return Ok(ExitCode::FAILURE);
        };
        let json = event_model.to_json_pretty()?;
        match out {
            Some(out_path) => std::fs::write(out_path, json)?,
            None => println!("{json}"),
        }
        Ok(ExitCode::SUCCESS)
    } else {
        let sources = read_sources(&files)?;
        let (compilation, sigil_sources) = compile_sources(&sources)?;
        report_multi_diagnostics(
            &sources,
            &sigil_sources.sources,
            &compilation.diagnostics,
            &compilation.sigil_diagnostics,
        );
        let Some(model) = compilation.model else {
            return Ok(ExitCode::FAILURE);
        };
        let json = model.to_json_pretty()?;
        match out {
            Some(out_path) => std::fs::write(out_path, json)?,
            None => println!("{json}"),
        }
        Ok(ExitCode::SUCCESS)
    }
}
