//! `rexlang` — command-line front end for the rexlang compiler driver.
//!
//! Subcommands:
//!
//! * `rexlang check <file>...` — compile and render diagnostics; exits `1`
//!   on errors. Accepts `.mox` models, `.actor` policy files, `.ddd`
//!   design files, `.evt` event-contract files, `.dmn` decision-model
//!   files, `.deploy` deployment files, `.uc` use case files, `.cmmn`
//!   case files, and `.ifml` interaction flows (the latter compile against
//!   the domain models, `.ddd` designs, `.actor` policies, and `.evt`
//!   contracts they import; `.ifml` expressions are also type-checked
//!   against the imported domains). Each input is a file or a directory —
//!   a directory enters **project mode**: every supported surface under it
//!   is gathered into one snapshot and checked independently, each file's
//!   imports resolving map-first from the snapshot and sibling artifacts
//!   reused compile-once; several `.mox` files passed explicitly compile as
//!   one multi-package model.
//! * `rexlang ir <file>... [-o <out>]` — compile and emit the Core IR JSON
//!   to stdout or to a file; exits `1` on errors. On `.actor` files the
//!   standalone ActorModel artifact is emitted; on `.ddd` files the
//!   standalone DDD design artifact; on `.evt` files the standalone
//!   EventModel artifact; on `.dmn` files the standalone DmnModel
//!   artifact; on `.deploy` files the standalone DeployModel
//!   artifact; on `.uc` files the standalone UseCaseModel artifact, on `.cmmn` files the standalone CmmnModel artifact; on
//!   `.ifml` files the standalone IfmlModel artifact.
//! * `rexlang build <paths>... [-o <dir>]` — compile every input and write
//!   each file's `ir` artifact JSON to `<dir>/<input path>.json` (the
//!   output directory defaults to `target/rex`). Directories enter project
//!   mode like `check`; exits `1` when any file fails, while compiling
//!   files still emit their artifacts.
//! * `rexlang graph <paths>... [-o <file>]` — compile every input and emit
//!   **one** semantic graph artifact spanning the whole tree (the
//!   import-connected applications, their nodes, and the semantic edges
//!   between them) as JSON to stdout or a file. Directories enter project
//!   mode like `check`/`build`; display paths are relative to the input
//!   roots' common ancestor, so the artifact never carries a machine path.
//!   Exits `1` when any file fails, while the compiling subset still emits
//!   (the `build` precedent). `rexlang graph query <graph.json> <subcommand>`
//!   runs the structural queries (`node`, `list`, `affected-by`,
//!   `dependencies`) on a committed artifact alone (see docs/GRAPH.md).
//! * `rexlang manifest <file.ifml> [-o <out>]` — compile one `.ifml`
//!   interaction flow, check it, and export its control/route manifest
//!   (routes, controls, edges, actions, commands) as JSON to stdout or a
//!   file; exits `1` on errors — a dangling reference never reaches a
//!   manifest.
//! * `rexlang artifact check <artifact.json>...` — validate wire-format
//!   artifacts (Core IR, standalone actor policy, DDD design, canonical
//!   instance) without the originating model; exits `1` on any violation.
//!   The portable test kit for out-of-tree backends (see docs/BACKENDS.md).
//! * `rexlang verify <files>... -o <receipt.json>` — compile the surfaces
//!   and emit a **verification receipt**: one canonical, versioned JSON
//!   document recording the hash of every source file the compile read
//!   (inputs, imports, pattern modules, decision tables), the hash and
//!   `formatVersion` of every artifact it emitted (pinned byte-identical
//!   to the committed golden where one exists), the generator version, and
//!   the proof-obligation register's rows with their statuses as the
//!   register states them. `--check <receipt.json>` validates a receipt
//!   standalone (no register, models, toolchain, or network — artifacts are
//!   re-hashed from the files next to it); `--check-fresh <receipt.json>`
//!   regenerates from the receipt's own inputs and byte-compares, exiting
//!   `1` with the command to run when the receipt is stale. See
//!   docs/RECEIPTS.md.
//! * `rexlang vocab fetch <file> [--provider file:<DIR>|http]` — fetch and
//!   vendor vocabulary snapshots, updating `model.lock`.
//! * `rexlang explain <code>` — print one diagnostic code's registry entry
//!   (band, meaning, fix); `--list` prints the allocation ledger and every
//!   registered code.
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
mod project;
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
    /// `.evt`, `.dmn`, `.deploy`, `.uc`, or `.ifml` files, rendering diagnostics
    /// grouped per file.
    Check {
        /// Files to compile, or directories (batch mode: every supported
        /// surface under them is checked independently).
        files: Vec<PathBuf>,
        /// Also exit `1` when only warning-severity diagnostics were
        /// found: the `.ifml` lints (`W0150`–`W0154`) and the union
        /// integrity pass's lint channel.
        #[arg(long)]
        deny_warnings: bool,
    },
    /// Compile `.mox`, `.actor`, `.ddd`, `.evt`, `.dmn`, `.deploy`, `.uc`, or `.cmmn` files
    /// and emit the artifact as JSON (`.actor` files emit the standalone
    /// ActorModel artifact, `.ddd` files the standalone DDD design
    /// artifact, `.evt` files the standalone EventModel artifact, `.dmn`
    /// files the standalone DmnModel artifact, `.deploy`
    /// files the standalone DeployModel artifact, `.uc` files the
    /// standalone UseCaseModel artifact; several `.mox` files emit one
    /// multi-package model).
    Ir {
        /// Paths to compile: `.mox`/`.actor`/`.ddd`/`.evt`/`.dmn`/`.deploy`/`.uc`
        /// files, or directories scanned recursively for `*.mox`.
        files: Vec<PathBuf>,
        /// Write the JSON to this path instead of stdout.
        #[arg(short, long, value_name = "FILE")]
        out: Option<PathBuf>,
        /// `.ifml` only: inline every `use` into the resolved module's
        /// contents (overrides applied) before emitting.
        #[arg(long)]
        expand: bool,
    },
    /// Compile every input file and write each artifact JSON into the
    /// output directory (default `target/rex`): `.mox` files emit the Core
    /// IR, `.actor` files the ActorModel artifact, `.ddd` files the DDD
    /// design artifact, `.evt` files the EventModel artifact, `.dmn`
    /// files the DmnModel artifact, `.deploy`
    /// files the DeployModel artifact, `.uc` files the UseCaseModel
    /// artifact, `.cmmn` files the CmmnModel artifact, and `.ifml` files
    /// the IfmlModel artifact — one artifact per file at
    /// `<out>/<input path>.json`. Directories enter project mode like
    /// `check`: every supported surface under them emits its own artifact.
    /// Exits `1` when any file fails; compiling files still emit.
    Build {
        /// Files or directories to compile.
        files: Vec<PathBuf>,
        /// Directory to write the artifacts into (`<input path>.json` per
        /// file).
        #[arg(short, long, value_name = "DIR", default_value = "target/rex")]
        out: PathBuf,
    },
    /// Compile every input and emit one semantic graph artifact spanning the
    /// whole tree: the import-connected applications, every surface's nodes,
    /// and the semantic edges between them (see docs/GRAPH.md). Directories
    /// enter project mode like `check`/`build`, and display paths are
    /// relative to the input roots' common ancestor. Exits `1` when any file
    /// fails; the compiling subset still emits.
    ///
    /// `rexlang graph query <graph.json> <subcommand>` runs the structural
    /// queries (`node`, `list`, `affected-by`, `dependencies`) and the
    /// cross-policy analytics (`separation-of-duty`, `monotonicity`,
    /// `dead-capabilities`, `reachability`) on a committed artifact instead —
    /// no sources, no recompile. Clap matches the subcommand first, so a
    /// directory literally named `query` must be given as `./query`.
    Graph {
        /// Files or directories to compile.
        #[arg(value_name = "PATHS")]
        paths: Vec<PathBuf>,
        /// Write the graph JSON to this path instead of stdout.
        #[arg(short, long, value_name = "FILE")]
        out: Option<PathBuf>,
        /// Query a committed graph artifact instead of compiling sources.
        #[command(subcommand)]
        action: Option<commands::graph::GraphCommand>,
    },
    /// Export the declaration-backed control/route manifest of one `.ifml`
    /// interaction flow as JSON: its routes/views, controls (expanded, with
    /// resolved `data:` entities), interaction edges (navigate/refresh/
    /// action invocations with triggers, guards, and bindings), actions
    /// with their target entities, and CLI commands flattened to their
    /// invocation paths. Compilation and checking gate emission — a
    /// dangling reference is an error before any manifest exists.
    Manifest {
        /// The `.ifml` interaction-flow file.
        file: PathBuf,
        /// Write the JSON to this path instead of stdout.
        #[arg(short, long, value_name = "FILE")]
        out: Option<PathBuf>,
    },
    /// Emit a verification receipt for the given model surfaces: one
    /// canonical, versioned JSON document recording every source file the
    /// compile read (hashed, repo-relative), every artifact it emitted
    /// (hashed, pinned against its committed golden where one exists), the
    /// generator version, and the proof-obligation register's rows with
    /// their statuses exactly as the register states them. `--check`
    /// validates a receipt standalone — no register, models, or toolchain —
    /// and `--check-fresh` regenerates from the receipt's own inputs and
    /// byte-compares (the CI freshness gate).
    Verify {
        /// Model surfaces to compile into the receipt (files or directories).
        files: Vec<PathBuf>,
        /// Write the receipt to this path. Artifact paths are recorded
        /// relative to it, so it belongs next to the artifacts it names.
        #[arg(short, long, value_name = "RECEIPT")]
        out: Option<PathBuf>,
        /// The receipt's subject name; defaults to the inputs' common file
        /// stem, else the receipt file's stem.
        #[arg(long, value_name = "NAME")]
        subject: Option<String>,
        /// Validate the given receipt instead of generating one: the
        /// schema, the closed vocabularies, the canonical byte form, and
        /// every artifact hash (against the files next to it). Needs no
        /// register, models, or network.
        #[arg(long, value_name = "RECEIPT")]
        check: Option<PathBuf>,
        /// Regenerate the receipt from the inputs it names and compare
        /// byte-for-byte with the committed file; exits `1` when stale.
        #[arg(long, value_name = "RECEIPT")]
        check_fresh: Option<PathBuf>,
    },
    /// Code generation from compiled `.mox` files.
    Gen {
        #[command(subcommand)]
        target: GenTarget,
    },
    /// Format `.mox`, `.actor`, `.ddd`, `.evt`, `.dmn`, `.uc`, `.deploy`,
    /// or `.ifml` files in place, preserving comments.
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
    /// Explain a diagnostic code from the central registry (`rex-diagnostics`):
    /// its band, meaning, and fix. Numeric surface codes (`E0104`) and the
    /// driver's camelCase LSP codes (`attributeWithClassType`) both resolve.
    Explain {
        /// The diagnostic code to explain.
        code: Option<String>,
        /// List the allocation ledger and every registered code instead of
        /// explaining one code.
        #[arg(long)]
        list: bool,
    },
    /// Emit shell completion script for the `rexlang` binary (bash, zsh,
    /// fish, powershell, or elvish) to stdout.
    Completions {
        /// The shell to generate completions for.
        #[arg(value_enum)]
        shell: clap_complete::Shell,
    },
    /// Emit the `rexlang` man page (roff) to stdout, or write it to
    /// `--out`.
    Man {
        /// Write the man page to this path instead of stdout.
        #[arg(short, long, value_name = "FILE")]
        out: Option<PathBuf>,
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
        Command::Check {
            files,
            deny_warnings,
        } => commands::check::run(files, deny_warnings),
        Command::Ir { files, out, expand } => commands::ir::run(files, out, expand),
        Command::Build { files, out } => commands::build::run(files, out),
        Command::Graph { paths, out, action } => match action {
            Some(action) => commands::graph::query(action),
            None => commands::graph::run(paths, out),
        },
        Command::Manifest { file, out } => commands::manifest::run(vec![file], out),
        Command::Verify {
            files,
            out,
            subject,
            check,
            check_fresh,
        } => commands::verify::run(files, out, subject, check, check_fresh),
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
        Command::Explain { code, list } => commands::explain::run(code.as_deref(), list),
        Command::Completions { shell } => commands::completions::run_completions(shell),
        Command::Man { out } => commands::completions::run_man(out),
    }
}
