//! Shared fixture helpers for the navigation unit tests (issue rexlang#245).
//!
//! The builders under test are pure: this module is the only place the tests
//! touch the disk (the `examples/` fixtures) and the only place they compile
//! a graph to compare against — exactly the invocation style of
//! `crates/rex-driver/tests/graph.rs`.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use rex_ir::graph::{kinds, GraphModel};

use super::SurfaceIndex;
use crate::graph::{build, GraphFile, GraphInputs};

/// The repository root (this crate's manifest is two levels down).
pub(crate) fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("workspace root")
}

/// The canonical path of a root-relative display path.
pub(crate) fn disk(display: &str) -> PathBuf {
    std::fs::canonicalize(workspace_root().join(display))
        .unwrap_or_else(|error| panic!("missing fixture file {display}: {error}"))
}

/// Reads a root-relative fixture file.
pub(crate) fn fixture(display: &str) -> String {
    std::fs::read_to_string(disk(display))
        .unwrap_or_else(|error| panic!("missing fixture file {display}: {error}"))
}

/// The `import "..."` strings of a source, in order (the fixture's own
/// scanner: graph inputs need the *resolved* keys, and resolving them from
/// disk is the host's job).
fn import_strings(source: &str) -> Vec<String> {
    let mut imports = Vec::new();
    for line in source.lines() {
        let line = line.trim_start();
        let Some(rest) = line.strip_prefix("import ") else {
            continue;
        };
        let Some(rest) = rest.trim_start().strip_prefix('"') else {
            continue;
        };
        if let Some(end) = rest.find('"') {
            imports.push(rest[..end].to_string());
        }
    }
    imports
}

/// One graph input file with its display path, host-normalized key, and
/// disk-resolved import targets.
fn graph_file<M>(display: &str, model: M) -> GraphFile<M> {
    let path = disk(display);
    let imports = import_strings(&fixture(display))
        .into_iter()
        .filter_map(|import| std::fs::canonicalize(path.parent()?.join(import)).ok())
        .map(|path| path.display().to_string())
        .collect();
    GraphFile::new(display, path.display().to_string(), imports, model)
}

/// The graph of one `.actor` example (its `import` texts keyed as written —
/// the `.actor` compile contract).
pub(crate) fn actor_graph(display: &str) -> GraphModel {
    let source = fixture(display);
    let directory = disk(display).parent().expect("a directory").to_path_buf();
    let domains: Vec<(String, String)> = import_strings(&source)
        .into_iter()
        .filter_map(|import| {
            std::fs::read_to_string(directory.join(&import))
                .ok()
                .map(|text| (import, text))
        })
        .collect();
    let compilation = crate::compile_actors_str(
        &disk(display).display().to_string(),
        &source,
        &domains,
        &crate::DomainImports::default(),
    );
    let model = compilation
        .model
        .unwrap_or_else(|| panic!("{display} must compile: {:?}", compilation.diagnostics));
    let mut inputs = GraphInputs::default();
    inputs.actors.push(graph_file(display, model));
    build(&inputs)
}

/// The graph of one `.ddd` example, resolved against `domain` (imports keyed
/// by canonical path — the `.ddd` compile contract).
pub(crate) fn ddd_graph(display: &str, domain: &str) -> GraphModel {
    let domains = [(disk(domain).display().to_string(), fixture(domain))];
    let compilation = crate::compile_ddd_str(
        &disk(display).display().to_string(),
        &fixture(display),
        &domains,
        &crate::DomainImports::default(),
    );
    let model = compilation
        .model
        .unwrap_or_else(|| panic!("{display} must compile: {:?}", compilation.diagnostics));
    let mut inputs = GraphInputs::default();
    inputs.designs.push(graph_file(display, model));
    build(&inputs)
}

/// The graph of one `.evt` example, resolved against `domain`.
pub(crate) fn evt_graph(display: &str, domain: &str) -> GraphModel {
    let domains = [(disk(domain).display().to_string(), fixture(domain))];
    let compilation = crate::compile_evt_str(
        &disk(display).display().to_string(),
        &fixture(display),
        &domains,
        &crate::DomainImports::default(),
    );
    let model = compilation
        .model
        .unwrap_or_else(|| panic!("{display} must compile: {:?}", compilation.diagnostics));
    let mut inputs = GraphInputs::default();
    inputs.events.push(graph_file(display, model));
    build(&inputs)
}

/// The self-contained conformance fixture's directory: one file per surface
/// (`app.mox`, `app.actor`, `app.ddd`, `app.evt`, `app.dmn`, `app.uc`,
/// `app.cmmn`, `app.deploy`, `app.ifml`), all one import-connected
/// application. It is the per-surface equality test's fixture: compiling it
/// needs no sibling examples, tables, or pattern library.
const FIXTURE_DIR: &str = "tests/conformance/graph";

/// A fixture file's root-relative display path.
fn fixture_display(name: &str) -> String {
    format!("{FIXTURE_DIR}/{name}")
}

/// One file's compiled artifact, panicking with its diagnostics when it does
/// not lower (the fixture must stay clean).
fn lowered<T, D: std::fmt::Debug>(display: &str, model: Option<T>, diagnostics: &D) -> T {
    model.unwrap_or_else(|| panic!("{display} must compile: {diagnostics:?}"))
}

/// The compiled artifacts of the `app.*` fixture, one per surface, wired into
/// the graph's input buckets exactly the way `tests/graph.rs` and the CLI
/// compile them (imports resolved from disk next to the importing file).
fn fixture_inputs() -> GraphInputs {
    let domain_display = fixture_display("app.mox");
    let domain_key = disk(&domain_display).display().to_string();
    let domain_source = fixture(&domain_display);
    let compilation = crate::compile_str(
        &domain_key,
        &domain_source,
        &crate::DomainImports::default(),
    );
    let domain = lowered(&domain_display, compilation.model, &compilation.diagnostics);
    let domains = [(domain_key, domain_source.clone())];

    let actor_display = fixture_display("app.actor");
    let compilation = crate::compile_actors_str(
        &disk(&actor_display).display().to_string(),
        &fixture(&actor_display),
        // The `.actor` compile keys its imports by the string as written.
        &[("app.mox".to_string(), domain_source.clone())],
        &crate::DomainImports::default(),
    );
    let actor = lowered(&actor_display, compilation.model, &compilation.diagnostics);

    let ddd_display = fixture_display("app.ddd");
    let compilation = crate::compile_ddd_str(
        &disk(&ddd_display).display().to_string(),
        &fixture(&ddd_display),
        &domains,
        &crate::DomainImports::default(),
    );
    let design = lowered(&ddd_display, compilation.model, &compilation.diagnostics);

    let evt_display = fixture_display("app.evt");
    let compilation = crate::compile_evt_str(
        &disk(&evt_display).display().to_string(),
        &fixture(&evt_display),
        &domains,
        &crate::DomainImports::default(),
    );
    let events = lowered(&evt_display, compilation.model, &compilation.diagnostics);

    let dmn_display = fixture_display("app.dmn");
    let compilation = crate::compile_dmn_str(
        &disk(&dmn_display).display().to_string(),
        &fixture(&dmn_display),
        &domains,
        &crate::DomainImports::default(),
        &[],
        // The decision model's import keys are the strings as written.
        &[("app.evt".to_string(), events.clone())],
    );
    let decision = lowered(&dmn_display, compilation.model, &compilation.diagnostics);

    let ifml_display = fixture_display("app.ifml");
    let ifml_key = disk(&ifml_display).display().to_string();
    let flow = rex_ifml::compile_ifml_str(
        &ifml_key,
        &fixture(&ifml_display),
        &rex_ifml::IfmlImports::default(),
    )
    .unwrap_or_else(|diagnostics| panic!("{ifml_display} must compile: {diagnostics:#?}"))
    .model;

    let uc_display = fixture_display("app.uc");
    let compilation = crate::compile_usecase_str(
        &disk(&uc_display).display().to_string(),
        &fixture(&uc_display),
        &domains,
        &crate::DomainImports::default(),
        &[(disk(&ddd_display).display().to_string(), design.clone())],
        &[(disk(&actor_display).display().to_string(), actor.clone())],
        &[(disk(&evt_display).display().to_string(), events.clone())],
    );
    let use_cases = lowered(&uc_display, compilation.model, &compilation.diagnostics);

    let cmmn_display = fixture_display("app.cmmn");
    let compilation = crate::compile_cmmn_str(
        &disk(&cmmn_display).display().to_string(),
        &fixture(&cmmn_display),
        &domains,
        &crate::DomainImports::default(),
        &[(disk(&ddd_display).display().to_string(), design.clone())],
        &[(disk(&actor_display).display().to_string(), actor.clone())],
        &[(disk(&evt_display).display().to_string(), events.clone())],
        &[(disk(&dmn_display).display().to_string(), decision.clone())],
    );
    let case = lowered(&cmmn_display, compilation.model, &compilation.diagnostics);

    let deploy_display = fixture_display("app.deploy");
    let compilation = crate::compile_deploy_str(
        &disk(&deploy_display).display().to_string(),
        &fixture(&deploy_display),
        &[(disk(&ddd_display).display().to_string(), design.clone())],
        &[(ifml_key, flow.clone())],
    );
    let deployment = lowered(&deploy_display, compilation.model, &compilation.diagnostics);

    let mut inputs = GraphInputs::default();
    inputs.domains.push(graph_file(&domain_display, domain));
    inputs.actors.push(graph_file(&actor_display, actor));
    inputs.designs.push(graph_file(&ddd_display, design));
    inputs.events.push(graph_file(&evt_display, events));
    inputs.decisions.push(graph_file(&dmn_display, decision));
    inputs.flows.push(graph_file(&ifml_display, flow));
    inputs.use_cases.push(graph_file(&uc_display, use_cases));
    inputs.cases.push(graph_file(&cmmn_display, case));
    inputs.deploys.push(graph_file(&deploy_display, deployment));
    inputs
}

/// The graph of the whole `app.*` fixture application, built once and shared
/// by every per-surface equality test (the compile chain is the expensive
/// part; the node table per file is fixed).
pub(crate) fn fixture_graph() -> &'static GraphModel {
    static GRAPH: OnceLock<GraphModel> = OnceLock::new();
    GRAPH.get_or_init(|| build(&fixture_inputs()))
}

/// The `(kind, name)` keys of the graph nodes attributed to `display`.
///
/// The one `file` node per compiled file is excluded: its name is the
/// document's display path, which a source-only span index cannot know (the
/// index keys declarations, not files).
pub(crate) fn graph_definition_keys(graph: &GraphModel, display: &str) -> Vec<(String, String)> {
    graph
        .nodes
        .iter()
        .filter(|node| node.file == display && node.kind != kinds::FILE)
        .map(|node| (node.kind.clone(), node.name.clone()))
        .collect()
}

/// Asserts the index's definitions are exactly the graph's nodes for
/// `display`, one-to-one (`file` nodes excluded — see
/// [`graph_definition_keys`]).
pub(crate) fn assert_definitions_match_graph(
    index: &SurfaceIndex,
    graph: &GraphModel,
    display: &str,
) {
    let mut expected: Vec<(String, String)> = graph_definition_keys(graph, display);
    expected.sort();
    let mut actual: Vec<(String, String)> = index
        .definitions
        .iter()
        .map(|definition| (definition.kind.clone(), definition.name.clone()))
        .collect();
    actual.sort();

    let unique: BTreeSet<(String, String)> = actual.iter().cloned().collect();
    assert_eq!(
        unique.len(),
        actual.len(),
        "{display}: duplicate (kind, name) definitions: {actual:#?}"
    );
    assert_eq!(
        actual, expected,
        "{display}: the span index's definitions must be the graph's nodes, one-to-one"
    );
    for definition in &index.definitions {
        assert_eq!(
            index
                .definitions_of(&definition.kind, &definition.name)
                .len(),
            1,
            "{display}: {} '{}' must have exactly one definition site",
            definition.kind,
            definition.name
        );
    }
}
