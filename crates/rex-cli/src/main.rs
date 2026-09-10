//! `rexlang` — command-line front end for the rexlang compiler driver.
//!
//! Subcommands:
//!
//! * `rexlang check <file>` — compile and render diagnostics; exits `1` on
//!   errors.
//! * `rexlang ir <file> [-o <out>]` — compile and emit the Core IR JSON to
//!   stdout or to a file; exits `1` on errors.

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use rex_driver::{compile_str, render, Compilation};
/// rexlang compiler command-line interface.
#[derive(Debug, Parser)]
#[command(name = "rexlang", version, about)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Parse, resolve, and validate a `.mox` file, rendering diagnostics.
    Check {
        /// Path to the `.mox` source file.
        file: PathBuf,
    },
    /// Compile a `.mox` file to the Core IR and emit it as JSON.
    Ir {
        /// Path to the `.mox` source file.
        file: PathBuf,
        /// Write the JSON to this path instead of stdout.
        #[arg(short, long, value_name = "FILE")]
        out: Option<PathBuf>,
    },
    /// Code generation from a compiled `.mox` file.
    Gen {
        #[command(subcommand)]
        target: GenTarget,
    },
}

#[derive(Debug, Subcommand)]
enum GenTarget {
    /// Generate arena-based Rust model code (`models.rs`).
    Rust {
        /// Path to the `.mox` source file.
        file: PathBuf,
        /// Directory to write generated files into.
        #[arg(short, long, value_name = "DIR")]
        out: PathBuf,
    },
    /// Generate JSON Schema (draft 2020-12) for the model (`schema.json`).
    JsonSchema {
        /// Path to the `.mox` source file.
        file: PathBuf,
        /// Which schema flavor to emit.
        #[arg(long, value_enum, default_value_t = SchemaProfile::Wire)]
        profile: SchemaProfile,
        /// Directory to write generated files into.
        #[arg(short, long, value_name = "DIR")]
        out: PathBuf,
    },
}

/// The JSON Schema flavor to generate.
#[derive(Debug, Clone, Copy, clap::ValueEnum)]
enum SchemaProfile {
    /// Validates the canonical instance document (envelope, ids, `$ref`s).
    Wire,
    /// Flattened DTO projection for OpenAPI-style consumers.
    Api,
}

fn main() -> ExitCode {
    match run(Cli::parse()) {
        Ok(code) => code,
        Err(error) => {
            eprintln!("error: {error:#}");
            ExitCode::FAILURE
        }
    }
}

fn run(cli: Cli) -> anyhow::Result<ExitCode> {
    match cli.command {
        Command::Check { file } => {
            let (path, source) = read_source(&file)?;
            let compilation = compile_str(&path, &source);
            report_diagnostics(&path, &source, &compilation);
            if compilation.model.is_some() {
                println!("OK {path}");
                Ok(ExitCode::SUCCESS)
            } else {
                Ok(ExitCode::FAILURE)
            }
        }
        Command::Ir { file, out } => {
            let (path, source) = read_source(&file)?;
            let compilation = compile_str(&path, &source);
            report_diagnostics(&path, &source, &compilation);
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
        Command::Gen {
            target: GenTarget::Rust { file, out },
        } => {
            let (path, source) = read_source(&file)?;
            let compilation = compile_str(&path, &source);
            report_diagnostics(&path, &source, &compilation);
            let Some(model) = compilation.model else {
                return Ok(ExitCode::FAILURE);
            };
            rex_backend_rust::generate_to_dir(&model, &out)?;
            println!("generated Rust model code into {}", out.display());
            Ok(ExitCode::SUCCESS)
        }
        Command::Gen {
            target:
                GenTarget::JsonSchema {
                    file,
                    profile,
                    out,
                },
        } => {
            let (path, source) = read_source(&file)?;
            let compilation = compile_str(&path, &source);
            report_diagnostics(&path, &source, &compilation);
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
    }
}

fn read_source(file: &PathBuf) -> anyhow::Result<(String, String)> {
    let source = std::fs::read_to_string(file)
        .map_err(|error| anyhow::anyhow!("cannot read {}: {error}", file.display()))?;
    Ok((file.display().to_string(), source))
}

fn report_diagnostics(path: &str, source: &str, compilation: &Compilation) {
    if !compilation.diagnostics.is_empty() {
        eprint!("{}", render(path, source, &compilation.diagnostics));
    }
}
