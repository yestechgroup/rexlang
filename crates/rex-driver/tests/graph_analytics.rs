//! Cross-policy analytics tests (issue rexlang#246): the four analytics run
//! on the committed conformance artifact alone.
//!
//! The fixture is `tests/conformance/graph/` — a self-contained, cleanly
//! compiling application (`app.*` across every surface) plus one unrelated
//! application (`other.*`, an opaque `cedar` policy), so the per-application
//! rule and the `unknown` outcome are exercised for real. The two expected
//! self-narrowing warnings the compiler emits for the fixture's
//! permit-then-forbid pairs are deliberately tolerated: the fixture does not
//! deny warnings (the gap is the point of the analytic).
//!
//! The byte goldens are `app.graph.json` (the artifact) and
//! `queries/*.json` (one answer per analytic), regenerated with:
//!
//! ```sh
//! REX_UPDATE_FIXTURES=1 cargo test -p rex-driver --test graph_analytics
//! ```
//!
//! Every analytic runs on the **deserialized artifact** — the product
//! property — and never touches a source file at query time.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use rex_driver::graph::analytics::{
    dead_capabilities, monotonicity, open, reachability, separation_of_duty, AnalyticAnswer,
    CapabilityStatus, Finding, MonotonicityOptions, ReachabilityOptions, SeparationOfDutyOptions,
};
use rex_driver::graph::query::{affected_by, ClosureDirection};
use rex_driver::graph::{build, GraphFile, GraphIndex, GraphInputs};
use rex_driver::{Diagnostic, Severity};
use rex_ir::graph::GraphModel;
use rex_ir::SessionKind;

// ---------------------------------------------------------------------------
// Fixture loading
// ---------------------------------------------------------------------------

fn workspace_root() -> &'static Path {
    static ROOT: OnceLock<PathBuf> = OnceLock::new();
    ROOT.get_or_init(|| {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../")
            .canonicalize()
            .expect("workspace root")
    })
}

/// The canonical path of a workspace-relative display path.
fn disk(display: &str) -> PathBuf {
    std::fs::canonicalize(workspace_root().join(display))
        .unwrap_or_else(|error| panic!("missing fixture file {display}: {error}"))
}

fn read(display: &str) -> String {
    std::fs::read_to_string(disk(display))
        .unwrap_or_else(|error| panic!("missing fixture file {display}: {error}"))
}

fn update() -> bool {
    std::env::var("REX_UPDATE_FIXTURES").is_ok()
}

fn fixture_dir() -> PathBuf {
    workspace_root().join("tests/conformance/graph")
}

fn artifact_path() -> PathBuf {
    fixture_dir().join("app.graph.json")
}

fn queries_dir() -> PathBuf {
    fixture_dir().join("queries")
}

/// The `import "..."` strings of a source, in order.
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

/// The host-normalized import targets of one fixture file.
fn imports_of(display: &str) -> Vec<String> {
    let source = read(display);
    let directory = disk(display)
        .parent()
        .expect("fixture file has a directory")
        .to_path_buf();
    import_strings(&source)
        .into_iter()
        .filter_map(|import| std::fs::canonicalize(directory.join(import)).ok())
        .map(|path| path.display().to_string())
        .collect()
}

/// One fixture file as the builder sees it: the **base-relative** display
/// path (the fixture directory is the graph's base, exactly as the CLI's
/// `rexlang graph tests/conformance/graph` emits it) and the normalized
/// absolute matching key.
fn graph_file<M>(name: &str, model: M) -> GraphFile<M> {
    let display = format!("tests/conformance/graph/{name}");
    GraphFile::new(
        name,
        disk(&display).display().to_string(),
        imports_of(&display),
        model,
    )
}

/// The fixture's two self-narrowing warnings are expected (they are the
/// analytic's evidence); anything else — and every error — fails.
fn assert_no_errors(where_: &str, diagnostics: &[Diagnostic]) {
    let errors: Vec<&Diagnostic> = diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.severity == Severity::Error)
        .collect();
    assert!(
        errors.is_empty(),
        "{where_} must compile without errors: {errors:#?}"
    );
}

/// [`assert_no_errors`] for the compile entry points that tag each
/// diagnostic with the file it belongs to.
fn assert_no_file_errors(where_: &str, diagnostics: &[(String, Diagnostic)]) {
    let errors: Vec<&(String, Diagnostic)> = diagnostics
        .iter()
        .filter(|(_, diagnostic)| diagnostic.severity == Severity::Error)
        .collect();
    assert!(
        errors.is_empty(),
        "{where_} must compile without errors: {errors:#?}"
    );
}

// ---------------------------------------------------------------------------
// Compiling the fixture (imports resolved from disk, the CLI's way)
// ---------------------------------------------------------------------------

fn domain_model(display: &str) -> rex_ir::Model {
    let compilation = rex_driver::compile_str(
        &disk(display).display().to_string(),
        &read(display),
        &rex_driver::DomainImports::default(),
    );
    assert_no_errors(display, &compilation.diagnostics);
    compilation.model.expect("domain must lower")
}

/// One `.actor` file; the import key is the string as written (the driver
/// receives texts only).
fn actor_model(display: &str) -> rex_ir::ActorModel {
    let source = read(display);
    let import = import_strings(&source)
        .into_iter()
        .next()
        .expect("the fixture policy imports its domain");
    let imported = format!(
        "{}/{}",
        Path::new(display)
            .parent()
            .expect("fixture file has a directory")
            .display(),
        import
    );
    let compilation = rex_driver::compile_actors_str(
        &disk(display).display().to_string(),
        &source,
        &[(import, read(&imported))],
        &rex_driver::DomainImports::default(),
    );
    assert_no_file_errors(display, &compilation.diagnostics);
    compilation.model.expect("actor policy must lower")
}

fn ddd_model() -> rex_ir::ddd::DddModel {
    let domains = [(
        disk("tests/conformance/graph/app.mox")
            .display()
            .to_string(),
        read("tests/conformance/graph/app.mox"),
    )];
    let compilation = rex_driver::compile_ddd_str(
        &disk("tests/conformance/graph/app.ddd")
            .display()
            .to_string(),
        &read("tests/conformance/graph/app.ddd"),
        &domains,
        &rex_driver::DomainImports::default(),
    );
    assert_no_file_errors("app.ddd", &compilation.diagnostics);
    compilation.model.expect("design must lower")
}

fn evt_model() -> rex_ir::events::EventModel {
    let domains = [(
        disk("tests/conformance/graph/app.mox")
            .display()
            .to_string(),
        read("tests/conformance/graph/app.mox"),
    )];
    let compilation = rex_driver::compile_evt_str(
        &disk("tests/conformance/graph/app.evt")
            .display()
            .to_string(),
        &read("tests/conformance/graph/app.evt"),
        &domains,
        &rex_driver::DomainImports::default(),
    );
    assert_no_file_errors("app.evt", &compilation.diagnostics);
    compilation.model.expect("event contract must lower")
}

fn flow_model() -> rex_ir::ifml::IfmlModel {
    let path = disk("tests/conformance/graph/app.ifml");
    let key = path.display().to_string();
    let source = read("tests/conformance/graph/app.ifml");
    let compilation = rex_ifml::compile_ifml_str(&key, &source, &rex_ifml::IfmlImports::default())
        .unwrap_or_else(|diagnostics| panic!("app.ifml must compile: {diagnostics:#?}"));
    let domain = domain_model("tests/conformance/graph/app.mox");
    let diagnostics = rex_ifml::check_ifml_with_actors(&compilation, Some(&domain), &[]);
    let errors: Vec<&rex_ifml::IfmlDiagnostic> = diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.severity == rex_ifml::Severity::Error)
        .collect();
    assert!(
        errors.is_empty(),
        "app.ifml must check without errors: {errors:#?}"
    );
    compilation.model
}

fn deploy_model() -> rex_ir::deploy::DeployModel {
    let designs = [(
        disk("tests/conformance/graph/app.ddd")
            .display()
            .to_string(),
        ddd_model(),
    )];
    let flows = [(
        disk("tests/conformance/graph/app.ifml")
            .display()
            .to_string(),
        flow_model(),
    )];
    let compilation = rex_driver::compile_deploy_str(
        &disk("tests/conformance/graph/app.deploy")
            .display()
            .to_string(),
        &read("tests/conformance/graph/app.deploy"),
        &designs,
        &flows,
    );
    assert_no_file_errors("app.deploy", &compilation.diagnostics);
    compilation.model.expect("deployment must lower")
}

fn uc_model() -> rex_ir::usecases::UseCaseModel {
    let domains = [(
        disk("tests/conformance/graph/app.mox")
            .display()
            .to_string(),
        read("tests/conformance/graph/app.mox"),
    )];
    let designs = [(
        disk("tests/conformance/graph/app.ddd")
            .display()
            .to_string(),
        ddd_model(),
    )];
    let actors = [(
        disk("tests/conformance/graph/app.actor")
            .display()
            .to_string(),
        actor_model("tests/conformance/graph/app.actor"),
    )];
    let events = [(
        disk("tests/conformance/graph/app.evt")
            .display()
            .to_string(),
        evt_model(),
    )];
    let compilation = rex_driver::compile_usecase_str(
        &disk("tests/conformance/graph/app.uc").display().to_string(),
        &read("tests/conformance/graph/app.uc"),
        &domains,
        &rex_driver::DomainImports::default(),
        &designs,
        &actors,
        &events,
    );
    assert_no_file_errors("app.uc", &compilation.diagnostics);
    compilation.model.expect("use cases must lower")
}

fn dmn_model() -> rex_ir::dmn::DmnModel {
    let domains = [(
        disk("tests/conformance/graph/app.mox")
            .display()
            .to_string(),
        read("tests/conformance/graph/app.mox"),
    )];
    let events = [("app.evt".to_string(), evt_model())];
    let compilation = rex_driver::compile_dmn_str(
        &disk("tests/conformance/graph/app.dmn")
            .display()
            .to_string(),
        &read("tests/conformance/graph/app.dmn"),
        &domains,
        &rex_driver::DomainImports::default(),
        &[],
        &events,
    );
    assert_no_file_errors("app.dmn", &compilation.diagnostics);
    compilation.model.expect("decision model must lower")
}

fn cmmn_model() -> rex_ir::cmmn::CmmnModel {
    let domains = [(
        disk("tests/conformance/graph/app.mox")
            .display()
            .to_string(),
        read("tests/conformance/graph/app.mox"),
    )];
    let designs = [(
        disk("tests/conformance/graph/app.ddd")
            .display()
            .to_string(),
        ddd_model(),
    )];
    let actors = [(
        disk("tests/conformance/graph/app.actor")
            .display()
            .to_string(),
        actor_model("tests/conformance/graph/app.actor"),
    )];
    let events = [(
        disk("tests/conformance/graph/app.evt")
            .display()
            .to_string(),
        evt_model(),
    )];
    let decisions = [(
        disk("tests/conformance/graph/app.dmn")
            .display()
            .to_string(),
        dmn_model(),
    )];
    let compilation = rex_driver::compile_cmmn_str(
        &disk("tests/conformance/graph/app.cmmn")
            .display()
            .to_string(),
        &read("tests/conformance/graph/app.cmmn"),
        &domains,
        &rex_driver::DomainImports::default(),
        &designs,
        &actors,
        &events,
        &decisions,
    );
    assert_no_file_errors("app.cmmn", &compilation.diagnostics);
    compilation.model.expect("case must lower")
}

/// The full fixture: every surface of `app.*` plus the unrelated `other.*`
/// application (no import path between them).
fn fixture_inputs() -> GraphInputs {
    let mut inputs = GraphInputs::default();
    inputs.domains.push(graph_file(
        "app.mox",
        domain_model("tests/conformance/graph/app.mox"),
    ));
    inputs.domains.push(graph_file(
        "other.mox",
        domain_model("tests/conformance/graph/other.mox"),
    ));
    inputs.actors.push(graph_file(
        "app.actor",
        actor_model("tests/conformance/graph/app.actor"),
    ));
    inputs.actors.push(graph_file(
        "other.actor",
        actor_model("tests/conformance/graph/other.actor"),
    ));
    inputs.designs.push(graph_file("app.ddd", ddd_model()));
    inputs.events.push(graph_file("app.evt", evt_model()));
    inputs
        .deploys
        .push(graph_file("app.deploy", deploy_model()));
    inputs.flows.push(graph_file("app.ifml", flow_model()));
    inputs.use_cases.push(graph_file("app.uc", uc_model()));
    inputs.cases.push(graph_file("app.cmmn", cmmn_model()));
    inputs.decisions.push(graph_file("app.dmn", dmn_model()));
    inputs
}

/// The deserialized fixture artifact — the one object every analytic runs
/// on. Under `REX_UPDATE_FIXTURES` the freshly built artifact is written
/// first; otherwise the committed golden must byte-match the build.
fn artifact() -> &'static GraphModel {
    static ARTIFACT: OnceLock<GraphModel> = OnceLock::new();
    ARTIFACT.get_or_init(|| {
        let built = build(&fixture_inputs());
        let json = built.to_json_pretty().expect("serialize the artifact");
        if update() {
            std::fs::write(artifact_path(), &json).expect("write the artifact golden");
            eprintln!("updated {}", artifact_path().display());
        } else {
            let golden = std::fs::read_to_string(artifact_path())
                .expect("read app.graph.json — regenerate with REX_UPDATE_FIXTURES=1");
            assert_eq!(
                json, golden,
                "the fixture artifact golden is stale — regenerate with REX_UPDATE_FIXTURES=1"
            );
        }
        GraphModel::from_json(&json).expect("the artifact round-trips")
    })
}

/// Compares (or writes) one query golden. The CLI writes the same bytes plus
/// the trailing newline `println!` adds, so the golden carries that newline.
fn golden(name: &str, actual: &str) {
    let path = queries_dir().join(name);
    if update() {
        std::fs::write(&path, actual).expect("write the query golden");
        eprintln!("updated {}", path.display());
        return;
    }
    let expected = std::fs::read_to_string(&path)
        .expect("read the query golden — regenerate with REX_UPDATE_FIXTURES=1");
    assert_eq!(actual, expected, "stale golden {}", path.display());
}

fn pretty(answer: &dyn AnalyticAnswer) -> String {
    format!(
        "{}\n",
        answer.render_json(true).expect("serialize the answer")
    )
}

// ---------------------------------------------------------------------------
// Fixture id constants
// ---------------------------------------------------------------------------

const CUSTOMER: &str = "actor:app.actor#App.Customer";
const AGENT: &str = "actor:app.actor#App.Agent";
const MANAGER: &str = "actor:app.actor#App.Manager";
const FINANCE: &str = "actor:app.actor#App.Finance";
const AUDITOR: &str = "actor:other.actor#Audit.Auditor";
const AUDITOR_AGENT: &str = "actor:other.actor#Audit.AuditorAgent";

const READ: &str = "capability:app.actor#App.ReadTicket";
const UPDATE: &str = "capability:app.actor#App.UpdateTicket";
const RESOLVE: &str = "capability:app.actor#App.ResolveTicket";
const RAISE: &str = "capability:app.actor#App.RaiseRefund";
const APPROVE: &str = "capability:app.actor#App.ApproveRefund";
const ESCALATE: &str = "capability:app.actor#App.EscalateRefund";
const PURGE: &str = "capability:other.actor#Audit.PurgeAudit";

const TICKET: &str = "class:app.mox#nz.example.graph.Ticket";
const NOTE: &str = "class:app.mox#nz.example.graph.Note";

// ---------------------------------------------------------------------------
// The artifact
// ---------------------------------------------------------------------------

/// Two import-disconnected applications, every surface present, no absolute
/// path anywhere.
#[test]
fn artifact_carries_the_two_applications() {
    let artifact = artifact();
    assert_eq!(
        artifact.applications.len(),
        2,
        "{:#?}",
        artifact.applications
    );
    let json = artifact.to_json_pretty().expect("serialize");
    assert!(!json.contains(&workspace_root().display().to_string()));
    assert!(!json.contains('\\'));
    for file in [
        "app.mox",
        "app.actor",
        "app.ddd",
        "app.evt",
        "app.ifml",
        "app.deploy",
        "app.uc",
        "app.cmmn",
        "app.dmn",
        "other.mox",
        "other.actor",
    ] {
        assert!(
            json.contains(&format!("\"file\": \"{file}\"")),
            "{file} contributes nodes: {json}"
        );
    }
}

// ---------------------------------------------------------------------------
// separation-of-duty
// ---------------------------------------------------------------------------

/// The declared `never_both` pair reports its holders and no violation.
#[test]
fn declared_pair_reports_holders_and_no_violation() {
    let index = GraphIndex::new(artifact());
    let answer = separation_of_duty(&index, &SeparationOfDutyOptions::default())
        .expect("the fixture resolves");
    golden(
        "app.separation-of-duty.declared-pair.json",
        &pretty(&answer),
    );

    assert_eq!(answer.results.len(), 1, "one declared pair, in app only");
    let result = &answer.results[0];
    assert_eq!(result.application, "app.actor");
    assert!(result.declared);
    assert_eq!(result.pair, [READ, RESOLVE]);
    assert!(result.violations.is_empty());
    let holders = |capability: &str| {
        result
            .holders
            .iter()
            .find(|holders| holders.capability == capability)
            .expect("each side reports its holders")
            .actors
            .clone()
    };
    assert_eq!(holders(READ), [AGENT, CUSTOMER, MANAGER]);
    assert_eq!(holders(RESOLVE), Vec::<String>::new());
    assert!(!answer.has_findings());
}

/// `--pair` answers an undeclared pair: `Manager` holds `RaiseRefund`
/// (inherited from `Agent`) and `ApproveRefund` (its own grant).
#[test]
fn undeclared_pair_reports_the_violation() {
    let index = GraphIndex::new(artifact());
    let options = SeparationOfDutyOptions {
        pair: Some(("App.RaiseRefund", "App.ApproveRefund")),
        actor: None,
        session: None,
    };
    let answer = separation_of_duty(&index, &options).expect("the fixture resolves");
    golden(
        "app.separation-of-duty.undeclared-pair.json",
        &pretty(&answer),
    );

    assert_eq!(
        answer.pair,
        Some(vec![APPROVE.to_string(), RAISE.to_string()]),
        "the echo is the sorted pair"
    );
    let result = &answer.results[0];
    assert!(!result.declared, "the pair is not a never_both constraint");
    assert_eq!(result.violations.len(), 1);
    assert_eq!(result.violations[0].actor, MANAGER);
    assert_eq!(
        result.violations[0].evidence,
        [format!("permits {APPROVE}"), format!("permits {RAISE}"),],
        "both sides name the granting edge"
    );
    assert!(answer.has_findings());

    // `--actor` narrows the question to one actor.
    let options = SeparationOfDutyOptions {
        pair: Some(("App.RaiseRefund", "App.ApproveRefund")),
        actor: Some("App.Agent"),
        session: None,
    };
    let answer = separation_of_duty(&index, &options).expect("the fixture resolves");
    assert!(
        answer.results[0].violations.is_empty(),
        "the agent only raises"
    );
    assert!(!answer.has_findings());

    // The human-only `ReadTicket` narrows away for an agent session while
    // `RaiseRefund` does not.
    let options = SeparationOfDutyOptions {
        pair: Some(("App.ReadTicket", "App.RaiseRefund")),
        actor: Some("App.Customer"),
        session: Some(SessionKind::Agent),
    };
    let answer = separation_of_duty(&index, &options).expect("the fixture resolves");
    let holder = |capability: &str| {
        answer.results[0]
            .holders
            .iter()
            .find(|holders| holders.capability == capability)
            .expect("each side")
            .actors
            .clone()
    };
    assert_eq!(holder(READ), Vec::<String>::new(), "human-only grant");
    assert_eq!(
        holder(RAISE),
        Vec::<String>::new(),
        "the customer never raises"
    );
}

// ---------------------------------------------------------------------------
// monotonicity
// ---------------------------------------------------------------------------

/// The child's `forbid` is the realistic gap.
#[test]
fn monotonicity_reports_the_forbidden_capability() {
    let index = GraphIndex::new(artifact());
    let answer =
        monotonicity(&index, &MonotonicityOptions::default()).expect("the fixture resolves");
    golden("app.monotonicity.json", &pretty(&answer));

    let edge = |child: &str| {
        answer
            .results
            .iter()
            .find(|result| result.child == child)
            .unwrap_or_else(|| panic!("{child} extends an actor"))
    };
    let manager = edge(MANAGER);
    assert_eq!(manager.parent, AGENT);
    assert_eq!(manager.missing, [UPDATE]);
    assert!(!manager.dominates);
    assert!(!manager.strict);
    assert!(!manager.unknown);
    assert!(manager.is_finding());

    let agent = edge(AGENT);
    assert_eq!(agent.parent, CUSTOMER);
    assert!(agent.missing.is_empty());
    assert!(agent.dominates);
    assert!(agent.strict, "the agent gained authority over the customer");

    assert!(answer.has_findings());
    // Every extends edge in the artifact is an actor edge: class inheritance
    // never reaches the policy analytic.
    assert!(answer
        .results
        .iter()
        .all(|result| result.child.starts_with("actor:")));

    // `--actor` restricts to that child.
    let answer = monotonicity(
        &index,
        &MonotonicityOptions {
            actor: Some("App.Manager"),
        },
    )
    .expect("the fixture resolves");
    assert_eq!(answer.results.len(), 1);
    assert_eq!(answer.results[0].child, MANAGER);
}

// ---------------------------------------------------------------------------
// dead-capabilities
// ---------------------------------------------------------------------------

/// `alive`, `dead`, `latent`, and `unknown` are four distinct outcomes, and
/// the two applications never pool their actors.
#[test]
fn dead_latent_alive_unknown_are_distinguished() {
    let index = GraphIndex::new(artifact());
    let answer = dead_capabilities(&index, None);
    golden("app.dead-capabilities.json", &pretty(&answer));

    let status = |capability: &str| {
        answer
            .results
            .iter()
            .find(|result| result.capability == capability)
            .unwrap_or_else(|| panic!("{capability} is declared"))
    };
    let alive = status(UPDATE);
    assert_eq!(alive.status, CapabilityStatus::Alive);
    assert_eq!(alive.holders, [AGENT]);
    assert!(alive.blocked_by.is_empty() && alive.unknown_actors.is_empty());

    let dead = status(RESOLVE);
    assert_eq!(dead.status, CapabilityStatus::Dead);
    assert_eq!(dead.blocked_by, [FINANCE]);
    assert!(dead.holders.is_empty());
    assert!(dead.is_finding());

    let latent = status(ESCALATE);
    assert_eq!(
        latent.status,
        CapabilityStatus::Latent,
        "declared, never granted"
    );
    assert!(!latent.is_finding());

    let unknown = status(PURGE);
    assert_eq!(
        unknown.status,
        CapabilityStatus::Unknown,
        "the cedar entry is opaque"
    );
    assert_eq!(unknown.unknown_actors, [AUDITOR, AUDITOR_AGENT]);
    assert!(!unknown.is_finding(), "uncertainty is never a finding");

    assert_eq!(answer.finding_count(), 1, "only the dead capability");
    // Results are sorted by capability id.
    let ids: Vec<&str> = answer
        .results
        .iter()
        .map(|result| result.capability.as_str())
        .collect();
    let mut sorted = ids.clone();
    sorted.sort();
    assert_eq!(ids, sorted);
}

// ---------------------------------------------------------------------------
// reachability
// ---------------------------------------------------------------------------

/// The containment path from the ticket to its note is the witness.
#[test]
fn reachability_finds_the_containment_path() {
    let index = GraphIndex::new(artifact());
    let options = ReachabilityOptions {
        from_actor: Some("App.Agent"),
        from_capability: None,
        to: "Note.internalNotes",
    };
    let answer = reachability(&index, &options).expect("the fixture resolves");
    golden("app.reachability.reachable.json", &pretty(&answer));

    assert_eq!(answer.from.as_deref(), Some(AGENT));
    let result = &answer.results[0];
    assert!(result.reachable);
    assert_eq!(result.target, NOTE);
    assert_eq!(result.start_classes, [TICKET]);
    assert!(
        result.starts.contains(&READ.to_string()) && result.starts.contains(&RAISE.to_string())
    );
    assert_eq!(
        result
            .path
            .iter()
            .map(|step| step.node.as_str())
            .collect::<Vec<_>>(),
        [TICKET, NOTE]
    );
    assert_eq!(result.path[1].edge.as_deref(), Some("contains"));
    assert_eq!(result.path[1].feature.as_deref(), Some("notes"));
    assert!(!answer.has_findings());
}

/// A class with no object-graph relation to the starts is unreachable — the
/// negative case is part of the contract.
#[test]
fn reachability_rejects_an_unrelated_field() {
    let index = GraphIndex::new(artifact());
    let options = ReachabilityOptions {
        from_actor: Some("App.Agent"),
        from_capability: None,
        to: "Invoice.amount",
    };
    let answer = reachability(&index, &options).expect("the fixture resolves");
    golden("app.reachability.unreachable.json", &pretty(&answer));

    let result = &answer.results[0];
    assert!(!result.reachable);
    assert!(result.path.is_empty());
    assert!(result.is_finding());

    // A capability start answers the same question for its own class.
    let options = ReachabilityOptions {
        from_actor: None,
        from_capability: Some(READ),
        to: "Note.body",
    };
    let answer = reachability(&index, &options).expect("the fixture resolves");
    assert!(answer.results[0].reachable);
    assert_eq!(answer.results[0].starts, [READ]);
}

// ---------------------------------------------------------------------------
// Per-application isolation and determinism
// ---------------------------------------------------------------------------

/// A pair split across the two applications has no shared policy scope, and
/// the capability pool never crosses the boundary.
#[test]
fn unrelated_applications_never_cross_check() {
    let index = GraphIndex::new(artifact());

    // The two capabilities of the pair live in different applications.
    let options = SeparationOfDutyOptions {
        pair: Some(("App.ReadTicket", "Audit.ReadAudit")),
        actor: None,
        session: None,
    };
    let answer = separation_of_duty(&index, &options).expect("the fixture resolves");
    assert!(
        answer.results.is_empty(),
        "no application holds both sides: {:#?}",
        answer.results
    );

    // `Audit.PurgeAudit` is not latent just because app's capabilities are
    // decidable: the other application's opacity decides it on its own.
    let answer = dead_capabilities(&index, None);
    let app_results = answer
        .results
        .iter()
        .filter(|result| result.application == "app.actor")
        .count();
    let other_results = answer
        .results
        .iter()
        .filter(|result| result.application == "other.actor")
        .count();
    assert_eq!(app_results, 6, "six capabilities in the support app");
    assert_eq!(other_results, 2, "two capabilities in the audit app");
    assert!(
        answer
            .results
            .iter()
            .filter(|result| result.application == "app.actor")
            .all(|result| result.status != CapabilityStatus::Unknown),
        "the support app carries no opaque grant"
    );

    // The `open` sets never mention a capability of the other application:
    // names pool per application only.
    for actor in [CUSTOMER, AGENT, MANAGER, FINANCE] {
        assert!(open(&index, actor, None)
            .iter()
            .all(|capability| capability.starts_with("capability:app.actor#")));
    }
    for actor in [AUDITOR, AUDITOR_AGENT] {
        assert!(open(&index, actor, None).is_empty());
    }
}

/// Two runs answer byte-identically, and so do the in-memory and the
/// deserialized artifact.
#[test]
fn analytics_are_byte_identical_across_runs() {
    let index = GraphIndex::new(artifact());
    let round_trip = artifact().to_json_pretty().expect("serialize");
    let reparsed = GraphModel::from_json(&round_trip).expect("deserialize");
    let rerun = GraphIndex::new(&reparsed);

    for (first, second) in [
        (
            pretty(
                &separation_of_duty(&index, &SeparationOfDutyOptions::default()).expect("resolves"),
            ),
            pretty(
                &separation_of_duty(&rerun, &SeparationOfDutyOptions::default()).expect("resolves"),
            ),
        ),
        (
            pretty(&monotonicity(&index, &MonotonicityOptions::default()).expect("resolves")),
            pretty(&monotonicity(&rerun, &MonotonicityOptions::default()).expect("resolves")),
        ),
        (
            pretty(&dead_capabilities(&index, None)),
            pretty(&dead_capabilities(&rerun, None)),
        ),
        (
            pretty(
                &reachability(
                    &index,
                    &ReachabilityOptions {
                        from_actor: Some("App.Agent"),
                        from_capability: None,
                        to: "Note.internalNotes",
                    },
                )
                .expect("resolves"),
            ),
            pretty(
                &reachability(
                    &rerun,
                    &ReachabilityOptions {
                        from_actor: Some("App.Agent"),
                        from_capability: None,
                        to: "Note.internalNotes",
                    },
                )
                .expect("resolves"),
            ),
        ),
    ] {
        assert_eq!(first, second);
    }
}

// ---------------------------------------------------------------------------
// The exit gate (slice 4/8's artifact)
// ---------------------------------------------------------------------------

/// On the support stack: the impact answer is exact (pinned in slice 4/8)
/// **and** the refund pair's holders are correct with no violation —
/// Agent/Manager raise, Finance approves, nobody both.
#[test]
fn exit_gate_support_example_answers_what_is_affected_by_changing_a_capability() {
    let path = workspace_root().join("examples/artifacts/examples.graph.json");
    let artifact = GraphModel::from_json(
        &std::fs::read_to_string(&path).expect("read the examples graph golden"),
    )
    .expect("the examples artifact is versioned");
    let index = GraphIndex::new(&artifact);

    let capability = "capability:support.actor#Support.RaiseRefund";
    let answer =
        affected_by(&index, capability, &[], ClosureDirection::Dependents).expect("resolves");
    let ids: Vec<&str> = answer.results.iter().map(|node| node.id.as_str()).collect();
    assert_eq!(
        ids,
        [
            capability,
            "actor:support.actor#Support.Agent",
            "capability:support.actor#Support.ApproveRefund",
            "serviceOperation:support.ddd#SupportDesk.desk.TicketService.raiseRefund",
            "useCase:support.uc#RaiseRefund",
            "action:support.ifml#RaiseRefund",
            "actor:support.actor#Support.Finance",
            "actor:support.actor#Support.Manager",
            "caseRole:support.cmmn#RefundDesk.triage",
            "command:cli.ifml#tickets",
            "planItem:support.cmmn#RefundDesk.AssessRefund",
            "service:support.ddd#SupportDesk.desk.TicketService",
            "serviceOperation:support.ddd#SupportDesk.desk.TicketService.approveRefund",
            "useCase:support.uc#ApproveRefund",
            "action:support.ifml#ApproveRefund",
            "case:support.cmmn#RefundDesk",
            "caseRole:support.cmmn#RefundDesk.escalation",
            "caseRole:support.cmmn#RefundDesk.finance",
            "component:support.ifml#Queue.intake",
            "flow:cli.ifml#cli",
            "flow:support.ifml#support",
            "module:support.ddd#SupportDesk.desk",
            "planItem:support.cmmn#RefundDesk.CloseTicket",
            "planItem:support.cmmn#RefundDesk.ReviewTicket",
            "component:support.ifml#Queue.refunds",
            "deployComponent:support.deploy#SupportDesk.api",
            "deployComponent:support.deploy#SupportDesk.web",
            "designApplication:support.ddd#SupportDesk",
            "planItem:support.cmmn#RefundDesk.ApproveRefund",
            "planItem:support.cmmn#RefundDesk.Escalate",
            "deployApplication:support.deploy#SupportDesk",
            "deployment:support.deploy#SupportDesk.edgeLive",
            "deployment:support.deploy#SupportDesk.production",
        ]
    );

    let options = SeparationOfDutyOptions {
        pair: Some((
            "capability:support.actor#Support.RaiseRefund",
            "capability:support.actor#Support.ApproveRefund",
        )),
        actor: None,
        session: None,
    };
    let answer = separation_of_duty(&index, &options).expect("the support pair resolves");
    assert_eq!(answer.results.len(), 1);
    let result = &answer.results[0];
    assert!(result.declared, "never_both over the refund pair");
    let holders = |capability: &str| {
        result
            .holders
            .iter()
            .find(|holders| holders.capability == capability)
            .expect("each side")
            .actors
            .clone()
    };
    assert_eq!(
        holders("capability:support.actor#Support.RaiseRefund"),
        [
            "actor:support.actor#Support.Agent",
            "actor:support.actor#Support.Manager",
        ]
    );
    assert_eq!(
        holders("capability:support.actor#Support.ApproveRefund"),
        ["actor:support.actor#Support.Finance"]
    );
    assert!(
        result.violations.is_empty(),
        "nobody raises and approves: {:#?}",
        result.violations
    );
    // `Manager` carries a bare `cedar` entry in `support.actor`, so the pair
    // is genuinely undecidable for it: the answer names it instead of
    // claiming a definite no.
    assert_eq!(
        result.unknown_actors,
        ["actor:support.actor#Support.Manager"]
    );
    assert!(!answer.has_findings());
}
