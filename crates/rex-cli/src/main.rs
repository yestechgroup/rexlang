//! `rexlang` — command-line front end for the rexlang compiler driver.
//!
//! Subcommands:
//!
//! * `rexlang check <file>...` — compile and render diagnostics; exits `1`
//!   on errors. Accepts `.mox` models, `.actor` policy files, `.ddd`
//!   design files, `.evt` event-contract files, and `.ifml` interaction
//!   flows (the latter four compile against the domain models and `.ifml`
//!   pattern libraries they import; `.ifml` expressions are also type-checked
//!   against the imported domains). Each input is a file or a directory —
//!   a directory enters **batch mode**, checking every supported surface
//!   under it independently (`.mox`, `.actor`, `.ddd`, `.evt`, `.ifml`);
//!   several `.mox` files passed explicitly compile as one multi-package
//!   model.
//! * `rexlang ir <file>... [-o <out>]` — compile and emit the Core IR JSON
//!   to stdout or to a file; exits `1` on errors. On `.actor` files the
//!   standalone ActorModel artifact is emitted; on `.ddd` files the
//!   standalone DDD design artifact; on `.evt` files the standalone
//!   EventModel artifact; on `.ifml` files the standalone IfmlModel
//!   artifact.
//! * `rexlang artifact check <artifact.json>...` — validate wire-format
//!   artifacts (Core IR, standalone actor policy, DDD design, canonical
//!   instance) without the originating model; exits `1` on any violation.
//!   The portable test kit for out-of-tree backends (see docs/BACKENDS.md).
//! * `rexlang vocab fetch <file> [--provider file:<DIR>|http]` — fetch and
//!   vendor vocabulary snapshots, updating `model.lock`.
//!
//! This file is clap wiring and dispatch only: input expansion and reading
//! live in `inputs` (plus the shared compile step), disk-facing import
//! collection in `imports`, diagnostics rendering in `report`, artifact
//! validation in `artifact_check`, and one handler per subcommand in
//! `commands`.

mod artifact_check;
mod commands;
mod imports;
mod inputs;
mod report;

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};

/// rexlang compiler command-line interface.
#[derive(Debug, Parser)]
#[command(name = "rexlang", version, about)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Parse, resolve, validate, and type-check `.mox`, `.actor`, `.ddd`,
    /// `.evt`, or `.ifml` files, rendering diagnostics grouped per file.
    Check {
        /// Files to compile, or directories (batch mode: every supported
        /// surface under them is checked independently).
        files: Vec<PathBuf>,
    },
    /// Compile `.mox`, `.actor`, `.ddd`, or `.evt` files and emit the
    /// artifact as JSON (`.actor` files emit the standalone ActorModel
    /// artifact, `.ddd` files the standalone DDD design artifact, `.evt`
    /// files the standalone EventModel artifact; several `.mox` files emit
    /// one multi-package model).
    Ir {
        /// Paths to compile: `.mox`/`.actor`/`.ddd`/`.evt` files, or
        /// directories scanned recursively for `*.mox`.
        files: Vec<PathBuf>,
        /// Write the JSON to this path instead of stdout.
        #[arg(short, long, value_name = "FILE")]
        out: Option<PathBuf>,
    },
    /// Code generation from compiled `.mox` files.
    Gen {
        #[command(subcommand)]
        target: GenTarget,
    },
    /// Format `.mox`, `.actor`, `.ddd`, `.evt`, or `.ifml` files in place,
    /// preserving comments.
    Fmt {
        /// List files whose formatting would change instead of rewriting
        /// them; exits `1` when any file is unformatted.
        #[arg(long)]
        check: bool,
        /// Files to format, or `-` to read stdin and write stdout. With no
        /// files, stdin is formatted.
        files: Vec<PathBuf>,
    },
    /// Serve the rexlang language server over stdin/stdout.
    Lsp,
    /// Validate serialized rexlang artifacts against the wire format.
    Artifact {
        #[command(subcommand)]
        action: ArtifactAction,
    },
    /// Vocabulary snapshot tooling.
    Vocab {
        #[command(subcommand)]
        action: VocabAction,
    },
}

#[derive(Debug, Subcommand)]
enum ArtifactAction {
    /// Check each artifact file against the wire-format invariants: known
    /// root shape, supported `formatVersion`, camelCase structural keys,
    /// and (for instances) well-formed unique `$id`s with resolvable
    /// `$ref`s. Exits `1` when any file fails.
    Check {
        /// Artifact JSON files to check.
        files: Vec<PathBuf>,
    },
}

#[derive(Debug, Subcommand)]
enum VocabAction {
    /// Fetch each declared vocabulary snapshot, vendor it next to the model,
    /// and pin its digest in `model.lock`.
    Fetch {
        /// Path to the `.mox` source file.
        file: PathBuf,
        /// `file:<DIR>` to fetch from a directory of snapshots, or `http` to
        /// fetch from `--url-template`. Defaults to `file:<model-dir>/vocab`.
        #[arg(long, value_name = "PROVIDER")]
        provider: Option<String>,
        /// URL template for `--provider http`, with `{source}` and
        /// `{version}` placeholders.
        #[arg(long, value_name = "TEMPLATE")]
        url_template: Option<String>,
    },
}

#[derive(Debug, Subcommand)]
pub(crate) enum GenTarget {
    /// Generate arena-based Rust model code (`models.rs`).
    Rust {
        /// Paths to compile: `.mox` files, or directories scanned
        /// recursively for `*.mox`.
        files: Vec<PathBuf>,
        /// Directory to write generated files into.
        #[arg(short, long, value_name = "DIR")]
        out: PathBuf,
    },
    /// Generate JSON Schema (draft 2020-12) for the model (`schema.json`).
    JsonSchema {
        /// Paths to compile: `.mox` files, or directories scanned
        /// recursively for `*.mox`.
        files: Vec<PathBuf>,
        /// Which schema flavor to emit.
        #[arg(long, value_enum, default_value_t = SchemaProfile::Wire)]
        profile: SchemaProfile,
        /// Directory to write generated files into.
        #[arg(short, long, value_name = "DIR")]
        out: PathBuf,
    },
    /// Generate Cedar policies + schema for the `actors` blocks of `.mox`
    /// files, or of a `.actor` file plus its imported domain models
    /// (`<Block>.cedar` + `<Block>.cedarschema.json`).
    Cedar {
        /// Paths to compile: `.mox`/`.actor` files, or directories scanned
        /// recursively for `*.mox`.
        files: Vec<PathBuf>,
        /// Directory to write generated files into.
        #[arg(short, long, value_name = "DIR")]
        out: PathBuf,
    },
    /// Generate the per-agent tool manifests of the `actors` blocks as JSON
    /// on stdout (`.mox`: inline blocks; `.actor`: file plus its imported
    /// domain models).
    Tools {
        /// Paths to compile: `.mox`/`.actor` files, or directories scanned
        /// recursively for `*.mox`.
        files: Vec<PathBuf>,
    },
}

/// The JSON Schema flavor to generate.
#[derive(Debug, Clone, Copy, clap::ValueEnum)]
pub(crate) enum SchemaProfile {
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
        Command::Check { files } => commands::check::run(files),
        Command::Ir { files, out } => commands::ir::run(files, out),
        Command::Gen { target } => commands::gen::run(target),
        Command::Fmt { check, files } => commands::fmt::run(check, files),
        Command::Artifact {
            action: ArtifactAction::Check { files },
        } => commands::artifact::check(files),
        Command::Vocab {
            action:
                VocabAction::Fetch {
                    file,
                    provider,
                    url_template,
                },
        } => commands::vocab::run(&file, provider.as_deref(), url_template.as_deref()),
        Command::Lsp => commands::lsp::run(),
    }
}
