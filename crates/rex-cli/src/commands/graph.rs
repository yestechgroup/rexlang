//! `rexlang graph`: compile every input and emit **one** semantic graph
//! artifact (`rex_ir::graph::GraphModel`) spanning the whole tree.
//!
//! Inputs expand exactly like `check`/`build`: directories enter project mode
//! (one snapshot, per-file pipeline, sibling artifacts reused compile-once,
//! formatter goldens skipped), explicit files compile in their own context.
//! Each compiling file contributes its typed artifact and its resolved
//! imports; [`rex_driver::graph::build`] clusters the import-connected
//! applications and derives the nodes and edges.
//!
//! Two host facts the wire artifacts cannot carry are supplied here: the
//! **root-relative POSIX display path** every emitted path uses, and the
//! **host-normalized absolute keys** (`path`/`imports`, canonicalized where
//! the file exists) the builder clusters and matches on. Display paths are
//! relative to the common ancestor of the input roots — the directory
//! arguments when directories were given, else the explicit files' common
//! ancestor — so goldens never carry a machine path ([`display_path`]).
//!
//! Emission is best-effort over the compiling subset (the `build`
//! precedent): a failing file renders its diagnostics, contributes nothing,
//! and the command still writes the artifact for everything that compiled,
//! exiting `1`.

use std::io::Read;
use std::path::{Component, Path, PathBuf};
use std::process::ExitCode;

use anyhow::Context;
use rex_driver::graph::analytics::{
    dead_capabilities, monotonicity, reachability, separation_of_duty, AnalyticAnswer,
    MonotonicityOptions, ReachabilityOptions, SeparationOfDutyOptions,
};
use rex_driver::graph::query::{affected_by, dependencies, list, node, ClosureDirection};
use rex_driver::graph::{build, GraphFile, GraphIndex, GraphInputs};
use rex_ir::graph::GraphModel;
use rex_ir::SessionKind;

use crate::inputs::expand_check_inputs;
use crate::project::{Artifact, Project};

use super::pipeline::{compile_surface, emit_json};

/// The `rexlang graph` subcommand tree — today only the artifact-only query
/// surface. Clap matches a subcommand before the `PATHS` positional, so a
/// directory literally named `query` is passed as `./query`.
#[derive(Debug, clap::Subcommand)]
pub(crate) enum GraphCommand {
    /// Query a committed semantic graph artifact (`-` reads stdin): no
    /// sources, no recompile. Every answer is one versioned JSON envelope
    /// (see docs/GRAPH.md). An unknown or ambiguous reference exits `1` with
    /// the sorted candidates on stderr.
    Query {
        /// The graph artifact (`-` reads stdin).
        #[arg(value_name = "GRAPH")]
        graph: String,
        /// Pretty-print the envelope (2-space) instead of the compact form.
        #[arg(long, global = true)]
        pretty: bool,
        #[command(subcommand)]
        query: QueryCommand,
    },
}

/// One graph query subcommand.
#[derive(Debug, clap::Subcommand)]
pub(crate) enum QueryCommand {
    /// One node: its id, kind, name, file, and application, and with
    /// `--edges` its incident edges grouped by direction.
    Node {
        /// Node reference: exact id, exact name, or a unique name suffix.
        reference: String,
        /// Group the node's incident edges by direction (`out`/`in`).
        #[arg(long)]
        edges: bool,
    },
    /// Every node matching the filters, sorted by id.
    List {
        /// Node kind (the open vocabulary in docs/GRAPH.md).
        #[arg(long, value_name = "KIND")]
        kind: Option<String>,
        /// Declaring file (a root-relative display path).
        #[arg(long, value_name = "FILE")]
        file: Option<String>,
        /// Application id.
        #[arg(long, value_name = "APPLICATION")]
        application: Option<String>,
    },
    /// The closure of nodes that may be affected when the reference changes
    /// (the reverse walk; ownership edges walk forward). A file reference
    /// walks the `imports` graph instead.
    AffectedBy {
        /// Node reference: exact id, exact name, or a unique name suffix.
        reference: String,
        /// Restrict the reported nodes to these kinds (comma-separated). The
        /// walk still traverses intermediate nodes.
        #[arg(long, value_name = "KINDS", value_delimiter = ',')]
        kinds: Vec<String>,
        /// `dependents` (default) or `dependencies`.
        #[arg(long, value_name = "DIRECTION")]
        direction: Option<String>,
    },
    /// The nodes the reference depends on — the forward mirror of
    /// `affected-by`.
    Dependencies {
        /// Node reference: exact id, exact name, or a unique name suffix.
        reference: String,
    },
    /// Separation of duty: for every declared `never_both` pair (or the pair
    /// given by `--pair`, declared or not), which actors hold both
    /// capabilities. Conservative may-analysis: `when` guards count as
    /// satisfiable, and opaque `cedar` entries are reported as unknown, never
    /// as a definite answer.
    SeparationOfDuty {
        /// Ask the question for this capability pair (declared or not).
        #[arg(long, num_args = 2, value_names = ["A", "B"])]
        pair: Option<Vec<String>>,
        /// Restrict the actors examined and reported to this one.
        #[arg(long, value_name = "ACTOR")]
        actor: Option<String>,
        /// Refilter the grants by session kind.
        #[arg(long, value_name = "SESSION")]
        session: Option<SessionArg>,
        /// Exit `1` when a definite finding exists (the envelope still
        /// prints).
        #[arg(long)]
        fail_on_findings: bool,
    },
    /// Monotonicity: for every actor `extends` edge, the capabilities the
    /// parent holds and the child does not (`missing`).
    Monotonicity {
        /// Restrict the answer to the `extends` edge whose child is this
        /// actor.
        #[arg(long, value_name = "ACTOR")]
        actor: Option<String>,
        /// Exit `1` when a definite finding exists (the envelope still
        /// prints).
        #[arg(long)]
        fail_on_findings: bool,
    },
    /// Dead capabilities: every declared capability's outcome — `alive`,
    /// `dead` (permitted somewhere, open nowhere), `latent` (never granted),
    /// or `unknown` (an opaque grant leaves it undecidable).
    DeadCapabilities {
        /// Refilter the grants by session kind.
        #[arg(long, value_name = "SESSION")]
        session: Option<SessionArg>,
        /// Exit `1` when a definite finding exists (the envelope still
        /// prints).
        #[arg(long)]
        fail_on_findings: bool,
    },
    /// Reachability: can the authority of an actor (or one capability) reach
    /// a class — or a feature's class — through the domain object graph
    /// (`contains`/`refers`/`opposite`)?
    Reachability {
        /// Start from the classes of the capabilities this actor holds.
        #[arg(long, value_name = "ACTOR")]
        from_actor: Option<String>,
        /// Start from this capability's class.
        #[arg(long, value_name = "CAPABILITY")]
        from_capability: Option<String>,
        /// The target: a class or a feature (`Class.feature`).
        #[arg(long, value_name = "TARGET")]
        to: String,
        /// Exit `1` when the target is unreachable (the envelope still
        /// prints).
        #[arg(long)]
        fail_on_findings: bool,
    },
}

/// The `--session` value: `human` or `agent` (the unmodified default, `any`,
/// is never spelled — omitting the flag asks over every session kind).
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub(crate) enum SessionArg {
    /// Human sessions only.
    Human,
    /// Automation (LLM-agent) sessions only.
    Agent,
}

impl SessionArg {
    /// The IR session kind this argument names.
    fn kind(self) -> SessionKind {
        match self {
            SessionArg::Human => SessionKind::Human,
            SessionArg::Agent => SessionKind::Agent,
        }
    }
}

/// Runs `rexlang graph query <graph> <subcommand>`: loads the artifact,
/// resolves references against it, and prints one query envelope.
pub(crate) fn query(command: GraphCommand) -> anyhow::Result<ExitCode> {
    let GraphCommand::Query {
        graph,
        pretty,
        query,
    } = command;
    let text = read_artifact(&graph)?;
    let model = GraphModel::from_json(&text).with_context(|| {
        format!("{graph}: not a semantic graph artifact (see `rexlang graph <paths>`)")
    })?;
    let index = GraphIndex::new(&model);

    // The cross-policy analytics (issue rexlang#246) answer in the same
    // envelope but carry their own result records.
    if let Some(code) = analytics(&index, &query, pretty)? {
        return Ok(code);
    }

    let envelope = match query {
        QueryCommand::Node { reference, edges } => node(&index, &reference, edges),
        QueryCommand::List {
            kind,
            file,
            application,
        } => Ok(list(
            &index,
            kind.as_deref(),
            file.as_deref(),
            application.as_deref(),
        )),
        QueryCommand::AffectedBy {
            reference,
            kinds,
            direction,
        } => {
            let direction = match direction {
                Some(direction) => direction
                    .parse::<ClosureDirection>()
                    .map_err(|message: String| anyhow::anyhow!(message))?,
                None => ClosureDirection::Dependents,
            };
            affected_by(&index, &reference, &kinds, direction)
        }
        QueryCommand::Dependencies { reference } => dependencies(&index, &reference),
        QueryCommand::SeparationOfDuty { .. }
        | QueryCommand::Monotonicity { .. }
        | QueryCommand::DeadCapabilities { .. }
        | QueryCommand::Reachability { .. } => unreachable!("handled by `analytics`"),
    };
    let envelope = match envelope {
        Ok(envelope) => envelope,
        Err(error) => {
            eprintln!("error: {error}");
            for candidate in error.candidates() {
                eprintln!("  candidate: {candidate}");
            }
            return Ok(ExitCode::FAILURE);
        }
    };
    println!("{}", envelope.to_json(pretty)?);
    Ok(ExitCode::SUCCESS)
}

/// Runs one cross-policy analytic, prints its envelope, and returns the exit
/// code (`1` under `--fail-on-findings` when a definite finding exists).
/// `None` means the subcommand is not an analytic.
fn analytics(
    index: &GraphIndex<'_>,
    query: &QueryCommand,
    pretty: bool,
) -> anyhow::Result<Option<ExitCode>> {
    let code = match query {
        QueryCommand::SeparationOfDuty {
            pair,
            actor,
            session,
            fail_on_findings,
        } => {
            let pair = pair
                .as_ref()
                .map(|pair| (pair[0].as_str(), pair[1].as_str()));
            let options = SeparationOfDutyOptions {
                pair,
                actor: actor.as_deref(),
                session: session.map(SessionArg::kind),
            };
            match separation_of_duty(index, &options) {
                Ok(answer) => Ok(print_analytics(&answer, pretty, *fail_on_findings)),
                Err(error) => Err(error),
            }
        }
        QueryCommand::Monotonicity {
            actor,
            fail_on_findings,
        } => {
            let options = MonotonicityOptions {
                actor: actor.as_deref(),
            };
            match monotonicity(index, &options) {
                Ok(answer) => Ok(print_analytics(&answer, pretty, *fail_on_findings)),
                Err(error) => Err(error),
            }
        }
        QueryCommand::DeadCapabilities {
            session,
            fail_on_findings,
        } => {
            let answer = dead_capabilities(index, session.map(SessionArg::kind));
            Ok(print_analytics(&answer, pretty, *fail_on_findings))
        }
        QueryCommand::Reachability {
            from_actor,
            from_capability,
            to,
            fail_on_findings,
        } => {
            let options = ReachabilityOptions {
                from_actor: from_actor.as_deref(),
                from_capability: from_capability.as_deref(),
                to,
            };
            match reachability(index, &options) {
                Ok(answer) => Ok(print_analytics(&answer, pretty, *fail_on_findings)),
                Err(error) => Err(error),
            }
        }
        _ => return Ok(None),
    };
    let code = match code {
        Ok(code) => code,
        Err(error) => {
            eprintln!("error: {error}");
            for candidate in error.candidates() {
                eprintln!("  candidate: {candidate}");
            }
            return Ok(Some(ExitCode::FAILURE));
        }
    };
    Ok(Some(code))
}

/// Prints one analytic envelope (the answer always goes out) and maps
/// `--fail-on-findings` to the exit code.
fn print_analytics(answer: &dyn AnalyticAnswer, pretty: bool, fail_on_findings: bool) -> ExitCode {
    match answer.render_json(pretty) {
        Ok(json) => println!("{json}"),
        Err(error) => {
            eprintln!("error: cannot serialize the answer: {error}");
            return ExitCode::FAILURE;
        }
    }
    if fail_on_findings && answer.has_findings() {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

/// Reads the artifact from a path, or from stdin when the argument is `-`.
fn read_artifact(input: &str) -> anyhow::Result<String> {
    if input == "-" {
        let mut text = String::new();
        std::io::stdin()
            .read_to_string(&mut text)
            .context("cannot read the graph artifact from stdin")?;
        return Ok(text);
    }
    std::fs::read_to_string(input).with_context(|| format!("cannot read {input}"))
}

/// Runs `rexlang graph`: compiles the inputs, builds the semantic graph, and
/// writes it as pretty JSON to `out` (or stdout), exiting `1` when any input
/// failed to compile.
pub(crate) fn run(files: Vec<PathBuf>, out: Option<PathBuf>) -> anyhow::Result<ExitCode> {
    if files.is_empty() {
        anyhow::bail!("no model files found in the given paths");
    }
    // The same input-expansion semantics as `check`/`build`: directories
    // enter project mode (one snapshot, per-file pipeline); explicit files
    // compile in their own context.
    let project_mode = files.iter().any(|file| file.is_dir());
    let expanded = if project_mode {
        expand_check_inputs(&files)?
    } else {
        let mut expanded = files.clone();
        expanded.sort();
        expanded
    };
    let base = base_path(&files);
    let mut project = project_mode.then(|| Project::gather(&expanded));
    let mut inputs = GraphInputs::default();
    let mut failed = 0usize;
    for file in &expanded {
        match compile_surface(std::slice::from_ref(file), project.as_mut(), false) {
            Ok(Some(outcome)) => {
                let Some(artifact) = outcome.artifact else {
                    continue;
                };
                let display = display_path(file, &base);
                let path = normalize(&file.display().to_string());
                let imports = outcome
                    .imports
                    .iter()
                    .map(|import| normalize(import))
                    .collect();
                push(&mut inputs, display, path, imports, artifact);
            }
            Ok(None) => failed += 1,
            Err(error) => {
                eprintln!("Error: {error:#}");
                failed += 1;
            }
        }
    }
    let graph = build(&inputs);
    emit_json(graph.to_json_pretty()?, out.as_deref())?;
    if failed > 0 {
        eprintln!("{failed} of {} files failed", expanded.len());
        return Ok(ExitCode::FAILURE);
    }
    Ok(ExitCode::SUCCESS)
}

/// Files the typed artifact into its surface bucket.
fn push(
    inputs: &mut GraphInputs,
    display: String,
    path: String,
    imports: Vec<String>,
    artifact: Artifact,
) {
    match artifact {
        Artifact::Domain(model) => inputs
            .domains
            .push(GraphFile::new(display, path, imports, model)),
        Artifact::Actor(model) => inputs
            .actors
            .push(GraphFile::new(display, path, imports, model)),
        Artifact::Design(model) => inputs
            .designs
            .push(GraphFile::new(display, path, imports, model)),
        Artifact::Event(model) => inputs
            .events
            .push(GraphFile::new(display, path, imports, model)),
        Artifact::Deploy(model) => inputs
            .deploys
            .push(GraphFile::new(display, path, imports, model)),
        Artifact::Flow(model) => inputs
            .flows
            .push(GraphFile::new(display, path, imports, model)),
        Artifact::UseCase(model) => inputs
            .use_cases
            .push(GraphFile::new(display, path, imports, model)),
        Artifact::Case(model) => inputs
            .cases
            .push(GraphFile::new(display, path, imports, model)),
        Artifact::Decision(model) => inputs
            .decisions
            .push(GraphFile::new(display, path, imports, model)),
    }
}

/// The common ancestor of the input roots: the directory arguments when
/// directories were given, else the explicit files' parent directories
/// (their common ancestor). Canonicalized where the filesystem has one, so
/// display paths are stable across machines.
fn base_path(files: &[PathBuf]) -> PathBuf {
    let roots: Vec<PathBuf> = if files.iter().any(|file| file.is_dir()) {
        files.iter().filter(|file| file.is_dir()).cloned().collect()
    } else {
        files
            .iter()
            .map(|file| {
                file.parent()
                    .filter(|parent| !parent.as_os_str().is_empty())
                    .unwrap_or_else(|| Path::new("."))
                    .to_path_buf()
            })
            .collect()
    };
    let mut base: Option<PathBuf> = None;
    for root in roots {
        let canonical = std::fs::canonicalize(&root).unwrap_or(root);
        base = Some(match base {
            None => canonical,
            Some(current) => common_ancestor(&current, &canonical),
        });
    }
    base.unwrap_or_else(|| PathBuf::from("."))
}

/// The deepest directory prefix two paths share (both already canonical).
fn common_ancestor(left: &Path, right: &Path) -> PathBuf {
    left.components()
        .zip(right.components())
        .take_while(|(left, right)| left == right)
        .map(|(component, _)| component)
        .collect()
}

/// The file's root-relative POSIX display path: relative to `base`, `/`
/// separated, with `.` components dropped. A file outside every root still
/// emits no absolute path (the base's own prefix is dropped instead).
fn display_path(file: &Path, base: &Path) -> String {
    let canonical = std::fs::canonicalize(file).unwrap_or_else(|_| file.to_path_buf());
    if let Ok(relative) = canonical.strip_prefix(base) {
        return posix(relative);
    }
    let relative: PathBuf = canonical
        .components()
        .skip(base.components().count())
        .collect();
    if relative.as_os_str().is_empty() {
        posix(Path::new(canonical.file_name().unwrap_or_default()))
    } else {
        posix(&relative)
    }
}

/// Joins a path's normal components with `/` (root, prefix, and `.` dropped).
fn posix(path: &Path) -> String {
    path.components()
        .filter_map(|component| match component {
            Component::Normal(part) => Some(part.to_string_lossy().into_owned()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("/")
}

/// The host-normalized matching key: the canonical path where the file
/// exists, else the path as resolved. Never emitted — used for clustering
/// and import matching only.
fn normalize(path: &str) -> String {
    std::fs::canonicalize(path)
        .map(|canonical| canonical.display().to_string())
        .unwrap_or_else(|_| path.to_string())
}
