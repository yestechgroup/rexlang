//! `rexlang check`: compile inputs and render diagnostics grouped per file.
//!
//! Directory batch mode runs two phases: the per-file pipeline (every file
//! checked independently, exactly as single-file mode does), then the
//! union integrity pass — the per-file artifacts of every successful check
//! are gathered into import-connected applications (issue rexlang#114) and
//! [`rex_driver::check_integrity`] diagnoses the dangling cross-layer
//! wiring each. Single-file mode has no union context and stays
//! per-surface.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use rex_driver::{
    check_integrity, compile_actors_str, compile_cmmn_str, compile_ddd_str, compile_deploy_str,
    compile_evt_str, compile_usecase_str, Diagnostic,
};
use rex_ir::ddd::DddModel;
use rex_ir::deploy::DeployModel;
use rex_ir::events::EventModel;
use rex_ir::ifml::IfmlModel;
use rex_ir::ActorModel;

use crate::inputs::{
    expand_check_inputs, expand_inputs, is_actor_file, is_cmmn_file, is_ddd_file, is_deploy_file,
    is_evt_file, is_ifml_file, is_uc_file, read_actor_pair, read_cmmn_pair, read_ddd_design,
    read_deploy_pair, read_evt_pair, read_ifml_inputs, read_uc_pair,
};
use crate::report::{
    report_actor_diagnostics, report_cmmn_diagnostics, report_ddd_diagnostics,
    report_deploy_diagnostics, report_evt_diagnostics, report_ifml_diagnostics,
    report_usecase_diagnostics,
};

use super::pipeline::{check_exit, compile_reported_sources};

pub(crate) fn run(files: Vec<PathBuf>) -> anyhow::Result<ExitCode> {
    // Directory inputs enter batch mode: every supported surface under the
    // directory is checked per-file, then the compiled artifacts form
    // import-connected applications for the union integrity pass. Pass
    // several `.mox` files explicitly to compile them as one multi-package
    // model instead.
    if files.iter().any(|file| file.is_dir()) {
        let expanded = expand_check_inputs(&files)?;
        let mut failed = 0usize;
        let mut contributions: Vec<Contribution> = Vec::new();
        for file in &expanded {
            let mut sink = Vec::new();
            match check_one(std::slice::from_ref(file), &mut sink) {
                Ok(ExitCode::SUCCESS) => contributions.append(&mut sink),
                Ok(_) => failed += 1,
                Err(error) => {
                    eprintln!("Error: {error:#}");
                    failed += 1;
                }
            }
        }
        let integrity_errors = report_union_integrity(&contributions);
        if failed > 0 {
            eprintln!("{failed} of {} files failed", expanded.len());
        }
        if integrity_errors > 0 {
            eprintln!(
                "{integrity_errors} cross-surface integrity errors in the checked applications"
            );
            return Ok(ExitCode::FAILURE);
        }
        if failed > 0 {
            return Ok(ExitCode::FAILURE);
        }
        return Ok(ExitCode::SUCCESS);
    }
    let mut sink = Vec::new();
    check_one(&expand_inputs(&files)?, &mut sink)
}

/// The compiled artifact a successful per-file check contributes to the
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

fn check_one(files: &[PathBuf], sink: &mut Vec<Contribution>) -> anyhow::Result<ExitCode> {
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
        let code = check_exit(binding.is_empty(), &inputs.path);
        if code == ExitCode::SUCCESS {
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
        }
        Ok(code)
    } else if files.len() == 1 && is_ddd_file(&files[0]) {
        let design = read_ddd_design(&files[0])?;
        let compilation = compile_ddd_str(
            &design.path,
            &design.source,
            &design.domains,
            &design.imports(),
        );
        report_ddd_diagnostics(&design, &compilation);
        let code = check_exit(compilation.model.is_some(), &design.path);
        if let Some(model) = compilation.model {
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
        }
        Ok(code)
    } else if files.len() == 1 && is_actor_file(&files[0]) {
        let pair = read_actor_pair(&files[0])?;
        let compilation =
            compile_actors_str(&pair.path, &pair.source, &pair.domains, &pair.imports());
        report_actor_diagnostics(&pair, &compilation);
        let code = check_exit(compilation.model.is_some(), &pair.path);
        if let Some(model) = compilation.model {
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
        }
        Ok(code)
    } else if files.len() == 1 && is_evt_file(&files[0]) {
        let pair = read_evt_pair(&files[0])?;
        let compilation = compile_evt_str(&pair.path, &pair.source, &pair.domains, &pair.imports());
        report_evt_diagnostics(&pair, &compilation);
        let code = check_exit(compilation.model.is_some(), &pair.path);
        if let Some(model) = compilation.model {
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
        }
        Ok(code)
    } else if files.len() == 1 && is_deploy_file(&files[0]) {
        let pair = read_deploy_pair(&files[0])?;
        let compilation = compile_deploy_str(&pair.path, &pair.source, &pair.designs, &pair.flows);
        report_deploy_diagnostics(&pair, &compilation);
        let code = check_exit(compilation.model.is_some(), &pair.path);
        if let Some(model) = compilation.model {
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
        }
        Ok(code)
    } else if files.len() == 1 && is_uc_file(&files[0]) {
        let pair = read_uc_pair(&files[0])?;
        let compilation = compile_usecase_str(
            &pair.pair.path,
            &pair.pair.source,
            &pair.pair.domains,
            &pair.pair.imports(),
            &pair.designs,
            &pair.actors,
            &pair.events,
        );
        report_usecase_diagnostics(&pair.pair, &compilation);
        Ok(check_exit(compilation.model.is_some(), &pair.pair.path))
    } else if files.len() == 1 && is_cmmn_file(&files[0]) {
        let pair = read_cmmn_pair(&files[0])?;
        let compilation = compile_cmmn_str(
            &pair.pair.path,
            &pair.pair.source,
            &pair.pair.domains,
            &pair.pair.imports(),
            &pair.designs,
            &pair.actors,
            &pair.events,
        );
        report_cmmn_diagnostics(&pair.pair, &compilation);
        Ok(check_exit(compilation.model.is_some(), &pair.pair.path))
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
            for (path, _) in &sources {
                println!("OK {path}");
            }
            Ok(ExitCode::SUCCESS)
        } else {
            Ok(ExitCode::FAILURE)
        }
    }
}

/// Clusters the contributions into import-connected applications and runs
/// [`check_integrity`] per application, rendering its diagnostics against
/// the owning files. Returns the number of error-severity diagnostics.
///
/// Two files share an application when an import of one resolves to the
/// other (transitively, through the `.mox` domains every surface names) —
/// the house import graph, the same relation the per-surface compilers
/// already resolve. Unrelated applications in one checked tree never
/// cross-check each other, and an import that leaves the checked set (or
/// misses the disk) contributes no edge.
fn report_union_integrity(contributions: &[Contribution]) -> usize {
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
    let mut errors = 0usize;
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
            errors += group
                .iter()
                .filter(|diagnostic| diagnostic.is_error())
                .count();
            eprint!("{}", rex_driver::render(&path, source, &group));
        }
    }
    errors
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
