//! `rexlang gen`: code generation from compiled models — Rust model code,
//! JSON Schema, Cedar policies, and per-agent tool manifests.

use std::process::ExitCode;

use rex_driver::compile_actors_str;

use crate::inputs::{compile_sources, expand_inputs, is_actor_file, read_actor_pair, read_sources};
use crate::report::{report_actor_diagnostics, report_multi_diagnostics};
use crate::GenTarget;
use crate::SchemaProfile;

pub(crate) fn run(target: GenTarget) -> anyhow::Result<ExitCode> {
    match target {
        GenTarget::Rust { files, out } => {
            let files = expand_inputs(&files)?;
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
            rex_backend_rust::generate_to_dir(&model, &out)?;
            println!("generated Rust model code into {}", out.display());
            Ok(ExitCode::SUCCESS)
        }
        GenTarget::JsonSchema {
            files,
            profile,
            out,
        } => {
            let files = expand_inputs(&files)?;
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
            let profile = match profile {
                SchemaProfile::Wire => rex_backend_jsonschema::Profile::Wire,
                SchemaProfile::Api => rex_backend_jsonschema::Profile::Api,
            };
            rex_backend_jsonschema::generate_to_dir(&model, profile, &out)?;
            println!("generated JSON Schema into {}", out.display());
            Ok(ExitCode::SUCCESS)
        }
        GenTarget::Cedar { files, out } => {
            let files = expand_inputs(&files)?;
            if files.len() == 1 && is_actor_file(&files[0]) {
                let pair = read_actor_pair(&files[0])?;
                let compilation =
                    compile_actors_str(&pair.path, &pair.source, &pair.domains, &pair.imports());
                report_actor_diagnostics(&pair, &compilation);
                let Some(actor_model) = compilation.model else {
                    return Ok(ExitCode::FAILURE);
                };
                let Some(domain_model) = compilation.domains_model else {
                    return Ok(ExitCode::FAILURE);
                };
                rex_backend_cedar::generate_to_dir(&actor_model, &domain_model, &out)?;
                println!("generated Cedar policies and schema into {}", out.display());
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
                let files = rex_backend_cedar::generate_for_model(&model)?;
                std::fs::create_dir_all(&out)?;
                for (relative, contents) in &files {
                    std::fs::write(out.join(relative), contents)?;
                }
                println!("generated Cedar policies and schema into {}", out.display());
                Ok(ExitCode::SUCCESS)
            }
        }
        GenTarget::Tools { files } => {
            let files = expand_inputs(&files)?;
            if files.len() == 1 && is_actor_file(&files[0]) {
                let pair = read_actor_pair(&files[0])?;
                let compilation =
                    compile_actors_str(&pair.path, &pair.source, &pair.domains, &pair.imports());
                report_actor_diagnostics(&pair, &compilation);
                let Some(actor_model) = compilation.model else {
                    return Ok(ExitCode::FAILURE);
                };
                print_tool_manifests(&actor_model)?;
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
                let mut actor_model = rex_ir::ActorModel::new();
                for package in &model.packages {
                    for actors in &package.actors {
                        actor_model.blocks.push(actors.clone());
                    }
                }
                print_tool_manifests(&actor_model)?;
            }
            Ok(ExitCode::SUCCESS)
        }
    }
}

/// Prints the per-agent tool manifests of a compiled actor model as a
/// single pretty JSON document to stdout.
fn print_tool_manifests(actor_model: &rex_ir::ActorModel) -> anyhow::Result<()> {
    let document = rex_driver::manifest::ToolManifestDocument {
        agents: rex_driver::manifest::agent_tool_manifests(actor_model),
    };
    println!("{}", document.to_json_pretty()?);
    Ok(())
}
