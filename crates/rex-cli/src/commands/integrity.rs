//! The union integrity pass (rexlang#114), run from directory-mode `check`
//! as a second phase after the per-file project pipeline.
//!
//! Phase 1 (the project pipeline in `check`) checks every file
//! independently with its imports resolving map-first from the snapshot.
//! Since rexlang#128 it also owns the capability joins (existence and
//! coverage) as spanned compile-time errors. Phase 2 re-compiles each file
//! standalone (no project context — the semantics the integrity contract
//! is specified at) and gathers the compiled artifacts of every successful
//! check into import-connected applications;
//! [`rex_driver::check_integrity`] then diagnoses the dangling cross-layer
//! wiring each: errors for the joins the compiler does not yet own, and
//! the capability findings only as the lint channel — suppressed for files
//! phase 1 already failed (their spanned compiler error rendered there).
//! Per-file diagnostics are re-reported here only when the standalone
//! compile disagrees with phase 1 (or a file was skipped there), so the
//! union pass stays quiet in the common case.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use rex_driver::{
    check_integrity, compile_actors_str, compile_cmmn_str, compile_ddd_str, compile_deploy_str,
    compile_dmn_str, compile_evt_str, compile_usecase_str, Diagnostic,
};
use rex_ir::ddd::DddModel;
use rex_ir::deploy::DeployModel;
use rex_ir::events::EventModel;
use rex_ir::ifml::IfmlModel;
use rex_ir::ActorModel;

use crate::inputs::{
    is_actor_file, is_cmmn_file, is_ddd_file, is_deploy_file, is_dmn_file, is_evt_file,
    is_ifml_file, is_uc_file, read_actor_pair, read_cmmn_pair, read_ddd_design, read_deploy_pair,
    read_dmn_pair, read_evt_pair, read_ifml_inputs, read_uc_pair,
};
use crate::report::{
    report_actor_diagnostics, report_cmmn_diagnostics, report_ddd_diagnostics,
    report_deploy_diagnostics, report_dmn_diagnostics, report_evt_diagnostics,
    report_ifml_diagnostics, report_usecase_diagnostics,
};

use super::pipeline::compile_reported_sources;

/// Runs the pass over the expanded directory inputs; returns the number of
/// failing integrity diagnostics — error-severity always, plus the
/// warnings when `deny_warnings` is set (file failures already reported).
/// `checked_clean` carries the driver paths phase 1 compiled: a file that
/// failed there already owns its spanned compiler diagnostics, so the
/// pass's lint-channel warnings for it stay unprinted (rexlang#128).
pub(crate) fn run_union_pass(
    expanded: &[PathBuf],
    checked_clean: &BTreeSet<String>,
    deny_warnings: bool,
) -> anyhow::Result<usize> {
    let mut contributions: Vec<Contribution> = Vec::new();
    for file in expanded {
        contributions.extend(check_contribution(std::slice::from_ref(file))?);
    }
    Ok(report_union_integrity(
        &contributions,
        checked_clean,
        deny_warnings,
    ))
}

/// The compiled artifact a successful standalone check contributes to the
/// union integrity pass, with the raw material for clustering: the file's
/// display path (how diagnostics and the source map name it), its source
/// text, and the imports to cluster through.
struct Contribution {
    path: String,
    source: String,
    imports: Vec<ImportPath>,
    artifact: Option<Artifact>,
}

/// One import of a contribution, in the form its reader produced: the
/// `.ddd`/`.actor`/`.evt`/`.deploy` readers resolve imports against the
/// importing file's directory (the driver keys), while an `.ifml` artifact
/// carries the statements as written.
enum ImportPath {
    Resolved(String),
    AsWritten(String),
}

impl ImportPath {
    /// The file this import names, relative to the process working
    /// directory.
    fn target(&self, dir: &Path) -> PathBuf {
        match self {
            ImportPath::Resolved(path) => PathBuf::from(path),
            ImportPath::AsWritten(path) => dir.join(path),
        }
    }
}

/// One artifact bucket of the union pass. `.mox` domains and `.uc`/`.cmmn`
/// files contribute no artifact — domains are not a check input, and the
/// use case surfaces are consumers of the other buckets — but `.mox` files
/// still join the clustering as bridges between the surfaces importing
/// them.
enum Artifact {
    Design(DddModel),
    Actors(ActorModel),
    Events(EventModel),
    Deploys(DeployModel),
    Flows(IfmlModel),
}

/// Standalone check of one input: diagnostics render on failure (the
/// standalone resolution can disagree with phase 1's project reuse), a
/// successful compile contributes its artifact. No `OK` printing — phase 1
/// owns the per-file output.
fn check_contribution(files: &[PathBuf]) -> anyhow::Result<Vec<Contribution>> {
    let mut sink = Vec::new();
    let files = files.to_vec();
    if files.len() == 1 && is_ifml_file(&files[0]) {
        let inputs = read_ifml_inputs(&files[0], None)?;
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
                    return Ok(sink);
                }
            };
        let binding = rex_ifml::check_ifml(&compilation, inputs.domains.as_ref());
        // Only an error-severity finding disagrees with phase 1 (which
        // owns the warnings' rendering, and the `--deny-warnings` call on
        // them): a warning-only standalone re-check still contributes.
        if binding
            .iter()
            .any(|diagnostic| diagnostic.severity.is_error())
        {
            report_ifml_diagnostics(&sources(&compilation), &binding);
            return Ok(sink);
        }
        sink.push(Contribution {
            path: inputs.path.clone(),
            source: inputs.source,
            imports: compilation
                .model
                .imports
                .iter()
                .map(|import| ImportPath::AsWritten(import.clone()))
                .collect(),
            artifact: Some(Artifact::Flows(compilation.model)),
        });
    } else if files.len() == 1 && is_ddd_file(&files[0]) {
        let design = read_ddd_design(&files[0], None)?;
        let compilation = compile_ddd_str(
            &design.path,
            &design.source,
            &design.domains,
            &design.imports(),
        );
        if compilation.model.is_none() {
            report_ddd_diagnostics(&design, &compilation);
            return Ok(sink);
        }
        let model = compilation.model.expect("checked above");
        sink.push(Contribution {
            path: design.path.clone(),
            source: design.source.clone(),
            imports: design
                .domains
                .iter()
                .map(|(path, _)| ImportPath::Resolved(path.clone()))
                .collect(),
            artifact: Some(Artifact::Design(model)),
        });
    } else if files.len() == 1 && is_actor_file(&files[0]) {
        let pair = read_actor_pair(&files[0])?;
        let compilation =
            compile_actors_str(&pair.path, &pair.source, &pair.domains, &pair.imports());
        if compilation.model.is_none() {
            report_actor_diagnostics(&pair, &compilation);
            return Ok(sink);
        }
        let model = compilation.model.expect("checked above");
        sink.push(Contribution {
            path: pair.path.clone(),
            source: pair.source.clone(),
            imports: pair
                .domains
                .iter()
                .map(|(path, _)| ImportPath::Resolved(path.clone()))
                .collect(),
            artifact: Some(Artifact::Actors(model)),
        });
    } else if files.len() == 1 && is_evt_file(&files[0]) {
        let pair = read_evt_pair(&files[0], None)?;
        let compilation = compile_evt_str(&pair.path, &pair.source, &pair.domains, &pair.imports());
        if compilation.model.is_none() {
            report_evt_diagnostics(&pair, &compilation);
            return Ok(sink);
        }
        let model = compilation.model.expect("checked above");
        sink.push(Contribution {
            path: pair.path.clone(),
            source: pair.source.clone(),
            imports: pair
                .domains
                .iter()
                .map(|(path, _)| ImportPath::Resolved(path.clone()))
                .collect(),
            artifact: Some(Artifact::Events(model)),
        });
    } else if files.len() == 1 && is_deploy_file(&files[0]) {
        let pair = read_deploy_pair(&files[0], None)?;
        let compilation = compile_deploy_str(&pair.path, &pair.source, &pair.designs, &pair.flows);
        if compilation.model.is_none() {
            report_deploy_diagnostics(&pair, &compilation);
            return Ok(sink);
        }
        let model = compilation.model.expect("checked above");
        let mut imports: Vec<ImportPath> = pair
            .designs
            .iter()
            .map(|(path, _)| ImportPath::Resolved(path.clone()))
            .collect();
        imports.extend(
            pair.flows
                .iter()
                .map(|(path, _)| ImportPath::Resolved(path.clone())),
        );
        sink.push(Contribution {
            path: pair.path.clone(),
            source: pair.source.clone(),
            imports,
            artifact: Some(Artifact::Deploys(model)),
        });
    } else if files.len() == 1 && is_uc_file(&files[0]) {
        let pair = read_uc_pair(&files[0], None)?;
        let compilation = compile_usecase_str(
            &pair.pair.path,
            &pair.pair.source,
            &pair.pair.domains,
            &pair.pair.imports(),
            &pair.designs,
            &pair.actors,
            &pair.events,
        );
        if compilation.model.is_none() {
            report_usecase_diagnostics(&pair.pair, &compilation);
        }
    } else if files.len() == 1 && is_cmmn_file(&files[0]) {
        let pair = read_cmmn_pair(&files[0], None)?;
        let compilation = compile_cmmn_str(
            &pair.pair.path,
            &pair.pair.source,
            &pair.pair.domains,
            &pair.pair.imports(),
            &pair.designs,
            &pair.actors,
            &pair.events,
            &pair.decisions,
        );
        if compilation.model.is_none() {
            report_cmmn_diagnostics(&pair.pair, &compilation);
        }
    } else if files.len() == 1 && is_dmn_file(&files[0]) {
        // The decision-model surface contributes no bucket: the union pass
        // diagnoses the design/actor/event/deploy/flow wiring, and a
        // `.dmn` file is a consumer of domain types like the use case
        // surfaces (the `.uc`/`.cmmn` precedent).
        let pair = read_dmn_pair(&files[0], None)?;
        let compilation = compile_dmn_str(
            &pair.pair.path,
            &pair.pair.source,
            &pair.pair.domains,
            &pair.pair.imports(),
            &pair.tables,
        );
        if compilation.model.is_none() {
            report_dmn_diagnostics(&pair.pair, &compilation);
        }
    } else {
        let (sources, compilation) = compile_reported_sources(&files)?;
        if compilation.model.is_some() {
            // A single `.mox` file joins the clustering as a bridge node
            // (no artifact bucket); the multi-file explicit form is not a
            // batch-mode shape and contributes nothing.
            if files.len() == 1 && files[0].extension().and_then(|e| e.to_str()) == Some("mox") {
                sink.push(Contribution {
                    path: files[0].display().to_string(),
                    source: sources
                        .first()
                        .map(|(_, source)| source.clone())
                        .unwrap_or_default(),
                    imports: Vec::new(),
                    artifact: None,
                });
            }
        }
    }
    Ok(sink)
}

/// Clusters the contributions into import-connected applications and runs
/// [`check_integrity`] per application, rendering its diagnostics against
/// the owning files. Returns the number of failing diagnostics —
/// error-severity always, plus warnings when `deny_warnings` is set.
/// Warnings (the rexlang#128 lint channel for the capability joins) render
/// only for files phase 1 checked clean — a failed file's compiler error
/// is the report of record — and that suppression rule holds under
/// `deny_warnings` too.
///
/// Two files share an application when an import of one resolves to the
/// other (transitively, through the `.mox` domains every surface names) —
/// the house import graph, the same relation the per-surface compilers
/// already resolve. Unrelated applications in one checked tree never
/// cross-check each other, and an import that leaves the checked set (or
/// misses the disk) contributes no edge.
fn report_union_integrity(
    contributions: &[Contribution],
    checked_clean: &BTreeSet<String>,
    deny_warnings: bool,
) -> usize {
    let mut nodes = Nodes::default();
    // Contribution index → node index.
    let mut owner: Vec<Option<usize>> = Vec::with_capacity(contributions.len());
    for contribution in contributions {
        owner.push(nodes.get(Path::new(&contribution.path)));
    }
    // Node → the contribution that owns it (first wins).
    let mut contributor_of: BTreeMap<usize, usize> = BTreeMap::new();
    for (index, node) in owner.iter().enumerate() {
        if let Some(node) = node {
            contributor_of.entry(*node).or_insert(index);
        }
    }
    // Import edges: a contribution's resolved import targets join its node
    // to whatever file lives there — a bucket file or a bridging `.mox`.
    let mut edges: Vec<Vec<usize>> = vec![Vec::new(); nodes.len()];
    for (index, contribution) in contributions.iter().enumerate() {
        let Some(from) = owner[index] else { continue };
        let dir = Path::new(&contribution.path)
            .parent()
            .unwrap_or(Path::new("."));
        for import in &contribution.imports {
            if let Some(to) = nodes.get(&import.target(dir)) {
                edges[from].push(to);
            }
        }
    }
    // Union-find over the nodes; components then sort by their smallest
    // canonical path so diagnostic order is stable across runs.
    let mut parent: Vec<usize> = (0..nodes.len()).collect();
    for (from, targets) in edges.iter().enumerate() {
        for &to in targets {
            let a = find_root(&mut parent, from);
            let b = find_root(&mut parent, to);
            if a != b {
                parent[a] = b;
            }
        }
    }
    let mut components: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for node in 0..nodes.len() {
        let root = find_root(&mut parent, node);
        components.entry(root).or_default().push(node);
    }
    let mut ordered: Vec<Vec<usize>> = components.into_values().collect();
    ordered.sort_by_key(|members| {
        members
            .iter()
            .map(|&node| nodes.path(node).to_path_buf())
            .min()
            .expect("a component is never empty")
    });

    let sources: BTreeMap<&str, &str> = contributions
        .iter()
        .map(|contribution| (contribution.path.as_str(), contribution.source.as_str()))
        .collect();
    let mut total_errors = 0usize;
    for members in &ordered {
        let mut designs = Vec::new();
        let mut actors = Vec::new();
        let mut events = Vec::new();
        let mut deploys = Vec::new();
        let mut flows = Vec::new();
        for &node in members {
            let Some(&contributor) = contributor_of.get(&node) else {
                continue;
            };
            let contribution = &contributions[contributor];
            match &contribution.artifact {
                Some(Artifact::Design(model)) => {
                    designs.push((contribution.path.clone(), model.clone()))
                }
                Some(Artifact::Actors(model)) => {
                    actors.push((contribution.path.clone(), model.clone()))
                }
                Some(Artifact::Events(model)) => {
                    events.push((contribution.path.clone(), model.clone()))
                }
                Some(Artifact::Deploys(model)) => {
                    deploys.push((contribution.path.clone(), model.clone()))
                }
                Some(Artifact::Flows(model)) => {
                    flows.push((contribution.path.clone(), model.clone()))
                }
                None => {}
            }
        }
        if designs.is_empty()
            && actors.is_empty()
            && events.is_empty()
            && deploys.is_empty()
            && flows.is_empty()
        {
            continue;
        }
        let diagnostics = check_integrity(&designs, &actors, &events, &deploys, &flows);
        for (path, group) in group_by_path(&diagnostics) {
            let Some(source) = sources.get(path.as_str()) else {
                continue;
            };
            let errors = group
                .iter()
                .filter(|diagnostic| diagnostic.is_error())
                .count();
            let warnings = if deny_warnings {
                group
                    .iter()
                    .filter(|diagnostic| !diagnostic.is_error())
                    .count()
            } else {
                0
            };
            // rexlang#128: a lint-channel-only group for a file whose
            // phase-1 check already failed stays unprinted — the
            // compiler's spanned error rendered there.
            if errors == 0 && !checked_clean.contains(path.as_str()) {
                continue;
            }
            total_errors += errors + warnings;
            eprint!("{}", rex_driver::render(&path, source, &group));
        }
    }
    total_errors
}

/// The canonical node table of the import graph: checked files keyed by
/// their canonical on-disk path, in first-seen index order.
#[derive(Default)]
struct Nodes {
    index: BTreeMap<PathBuf, usize>,
    list: Vec<PathBuf>,
}

impl Nodes {
    /// The node for `path`, creating it when absent. Paths that cannot be
    /// canonicalized (missing on disk) form no node.
    fn get(&mut self, path: &Path) -> Option<usize> {
        let canonical = std::fs::canonicalize(path).ok()?;
        let next = self.list.len();
        Some(*self.index.entry(canonical.clone()).or_insert_with(|| {
            self.list.push(canonical);
            next
        }))
    }

    #[allow(dead_code)]
    fn len(&self) -> usize {
        self.list.len()
    }

    fn path(&self, index: usize) -> &Path {
        &self.list[index]
    }
}

/// Follows union-find parent links with path halving.
fn find_root(parent: &mut [usize], mut node: usize) -> usize {
    while parent[node] != node {
        parent[node] = parent[parent[node]];
        node = parent[node];
    }
    node
}

/// Groups path-tagged diagnostics into consecutive same-path runs.
fn group_by_path(diagnostics: &[(String, Diagnostic)]) -> Vec<(String, Vec<Diagnostic>)> {
    let mut groups: Vec<(String, Vec<Diagnostic>)> = Vec::new();
    for (path, diagnostic) in diagnostics {
        match groups.last_mut() {
            Some((group_path, group)) if *group_path == *path => group.push(diagnostic.clone()),
            _ => groups.push((path.clone(), vec![diagnostic.clone()])),
        }
    }
    groups
}
