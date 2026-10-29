//! `rexlang ir`: compile inputs and emit the artifact as JSON.

use std::path::PathBuf;
use std::process::ExitCode;

use rex_driver::{
    compile_actors_str, compile_cmmn_str, compile_ddd_str, compile_deploy_str, compile_evt_str,
    compile_usecase_str,
};

use crate::inputs::{
    compile_sources, expand_inputs, is_actor_file, is_ddd_file, is_deploy_file, is_evt_file,
    is_cmmn_file, is_ifml_file, is_uc_file, read_actor_pair, read_cmmn_pair, read_ddd_design,
    read_deploy_pair, read_evt_pair, read_ifml_inputs, read_sources, read_uc_pair,
};
use crate::report::{
    report_actor_diagnostics, report_ddd_diagnostics, report_deploy_diagnostics,
    report_evt_diagnostics, report_ifml_diagnostics, report_multi_diagnostics,
    report_cmmn_diagnostics, report_usecase_diagnostics,
};

pub(crate) fn run(
    files: Vec<PathBuf>,
    out: Option<PathBuf>,
    expand: bool,
) -> anyhow::Result<ExitCode> {
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
        let model = if expand {
            rex_ifml::expand_ifml(&compilation, &inputs.ifml_imports)
        } else {
            compilation.model
        };
        let json = model.to_json_pretty()?;
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
    } else if files.len() == 1 && is_deploy_file(&files[0]) {
        let pair = read_deploy_pair(&files[0])?;
        let compilation = compile_deploy_str(&pair.path, &pair.source, &pair.designs, &pair.flows);
        report_deploy_diagnostics(&pair, &compilation);
        let Some(deploy_model) = compilation.model else {
            return Ok(ExitCode::FAILURE);
        };
        let json = deploy_model.to_json_pretty()?;
        match out {
            Some(out_path) => std::fs::write(out_path, json)?,
            None => println!("{json}"),
        }
        Ok(ExitCode::SUCCESS)
    } else if files.len() == 1 && is_uc_file(&files[0]) {
        let pair = read_uc_pair(&files[0])?;
        let compilation = compile_usecase_str(
            &pair.path,
            &pair.source,
            &pair.domains,
            &pair.imports(),
            &pair.designs,
            &pair.actors,
            &pair.events,
        );
        report_usecase_diagnostics(&pair, &compilation);
        let Some(use_case_model) = compilation.model else {
            return Ok(ExitCode::FAILURE);
        };
        let json = use_case_model.to_json_pretty()?;
        match out {
            Some(out_path) => std::fs::write(out_path, json)?,
            None => println!("{json}"),
        }
        Ok(ExitCode::SUCCESS)
    } else if files.len() == 1 && is_cmmn_file(&files[0]) {
        let pair = read_cmmn_pair(&files[0])?;
        let compilation = compile_cmmn_str(
            &pair.path,
            &pair.source,
            &pair.domains,
            &pair.imports(),
            &pair.designs,
            &pair.actors,
            &pair.events,
        );
        report_cmmn_diagnostics(&pair, &compilation);
        let Some(case_model) = compilation.model else {
            return Ok(ExitCode::FAILURE);
        };
        let json = case_model.to_json_pretty()?;
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
