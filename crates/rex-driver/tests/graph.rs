//! Semantic-graph builder tests (issue rexlang#241): the `support` example
//! stack is the full-stack fixture — every surface of the repo's reference
//! application — plus the conformance machine fixtures and small inline
//! sources for the joins no example exercises (ambiguity, dangling wiring).
//!
//! The fixture loads and compiles the examples exactly the way the CLI and
//! the `examples_artifacts` harness do (imports resolved from disk relative
//! to the importing file), then hands the artifacts to
//! [`rex_driver::graph::build`] with the normalized import keys the builder
//! clusters on. Assertions are curated id/edge sets, not counts: an id is a
//! contract (`docs/GRAPH.md`), a count is a moving target.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use rex_driver::graph::query::node as query_node;
use rex_driver::graph::query::{self, affected_by, dependencies, ClosureDirection};
use rex_driver::graph::{build, GraphFile, GraphIndex, GraphInputs};
use rex_ir::graph::{kinds, GraphEdge, GraphModel, GraphNode};

// ---------------------------------------------------------------------------
// Fixture: paths, displays, and disk imports
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

/// The canonical path of a root-relative display path.
fn disk(display: &str) -> PathBuf {
    std::fs::canonicalize(workspace_root().join(display))
        .unwrap_or_else(|error| panic!("missing fixture file {display}: {error}"))
}

fn read(display: &str) -> String {
    std::fs::read_to_string(disk(display))
        .unwrap_or_else(|error| panic!("missing fixture file {display}: {error}"))
}

/// The `import "..."` strings of a source, in order (the fixture's own
/// scanner: the graph input needs the *resolved* keys, and the host's job is
/// to resolve them from disk exactly as the CLI does).
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

/// The host-normalized import targets of one input file.
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

fn graph_file<M>(display: &str, model: M) -> GraphFile<M> {
    GraphFile::new(
        display,
        disk(display).display().to_string(),
        imports_of(display),
        model,
    )
}

fn assert_clean<D: std::fmt::Debug>(where_: &str, diagnostics: &[D]) {
    assert!(
        diagnostics.is_empty(),
        "{where_} must compile with zero diagnostics: {diagnostics:?}"
    );
}

// ---------------------------------------------------------------------------
// The support stack
// ---------------------------------------------------------------------------

fn support_mox() -> rex_ir::Model {
    let display = "examples/support.mox";
    let compilation = rex_driver::compile_str(
        &disk(display).display().to_string(),
        &read(display),
        &rex_driver::DomainImports::default(),
    );
    assert_clean(display, &compilation.diagnostics);
    compilation.model.expect("support.mox must lower")
}

/// The compiled artifact of one `.ddd` example, resolved the CLI's way.
fn ddd_model(display: &str, domain_display: &str) -> rex_ir::ddd::DddModel {
    let domains = [(
        disk(domain_display).display().to_string(),
        read(domain_display),
    )];
    let compilation = rex_driver::compile_ddd_str(
        &disk(display).display().to_string(),
        &read(display),
        &domains,
        &rex_driver::DomainImports::default(),
    );
    assert_clean(display, &compilation.diagnostics);
    compilation.model.expect("design must lower")
}

fn evt_model(display: &str, domain_display: &str) -> rex_ir::events::EventModel {
    let domains = [(
        disk(domain_display).display().to_string(),
        read(domain_display),
    )];
    let compilation = rex_driver::compile_evt_str(
        &disk(display).display().to_string(),
        &read(display),
        &domains,
        &rex_driver::DomainImports::default(),
    );
    assert_clean(display, &compilation.diagnostics);
    compilation.model.expect("event contract must lower")
}

/// The `support.actor` policy — the `.actor` import key is the string as
/// written (the driver receives texts only).
fn actor_model() -> rex_ir::ActorModel {
    let display = "examples/support.actor";
    let compilation = rex_driver::compile_actors_str(
        &disk(display).display().to_string(),
        &read(display),
        &[("support.mox".to_string(), read("examples/support.mox"))],
        &rex_driver::DomainImports::default(),
    );
    assert_clean(display, &compilation.diagnostics);
    compilation.model.expect("actor policy must lower")
}

fn dmn_model() -> rex_ir::dmn::DmnModel {
    let display = "examples/support.dmn";
    let domains = [(
        disk("examples/support.mox").display().to_string(),
        read("examples/support.mox"),
    )];
    let tables = [(
        "./tables/refund-authority.md".to_string(),
        read("examples/tables/refund-authority.md"),
    )];
    let events = [(
        "support.evt".to_string(),
        evt_model("examples/support.evt", "examples/support.mox"),
    )];
    let compilation = rex_driver::compile_dmn_str(
        &disk(display).display().to_string(),
        &read(display),
        &domains,
        &rex_driver::DomainImports::default(),
        &tables,
        &events,
    );
    assert_clean(display, &compilation.diagnostics);
    compilation.model.expect("decision model must lower")
}

/// The pattern files `support.ifml` instantiates (its `walk_ifml_imports`
/// closure), hardcoded per the house convention: a missing file fails loudly.
const PATTERN_FLOWS: [&str; 6] = [
    "patterns/ifml/collections.ifml",
    "patterns/ifml/communication.ifml",
    "patterns/ifml/data.ifml",
    "patterns/ifml/input.ifml",
    "patterns/ifml/navigation.ifml",
    "patterns/ifml/workflow.ifml",
];

/// One `.ifml` file compiled with its `.ifml` import closure gathered from
/// disk through the shared traversal.
fn flow_model(display: &str) -> rex_ir::ifml::IfmlModel {
    let path = disk(display);
    let key = path.display().to_string();
    let source = read(display);
    let mut imports = rex_ifml::IfmlImports::default();
    let mut disk_paths = BTreeMap::new();
    disk_paths.insert(key.clone(), path.clone());
    rex_ifml::walk_ifml_imports(&key, &source, &mut |importer, import| {
        if !import.ends_with(".ifml") {
            return None;
        }
        let importer_path = disk_paths.get(importer)?;
        let resolved = importer_path.parent()?.join(import);
        let imported = std::fs::read_to_string(&resolved).ok()?;
        let resolved_key = resolved.display().to_string();
        disk_paths.insert(import.to_string(), resolved);
        imports.provide(importer, import, imported.clone());
        Some((resolved_key, imported))
    });
    let compilation = rex_ifml::compile_ifml_str(&key, &source, &imports)
        .unwrap_or_else(|diagnostics| panic!("{display} must compile: {diagnostics:?}"));
    compilation.model
}

fn uc_model() -> rex_ir::usecases::UseCaseModel {
    let display = "examples/support.uc";
    let domains = [(
        disk("examples/support.mox").display().to_string(),
        read("examples/support.mox"),
    )];
    let designs = [(
        disk("examples/support.ddd").display().to_string(),
        ddd_model("examples/support.ddd", "examples/support.mox"),
    )];
    let actors = [(
        disk("examples/support.actor").display().to_string(),
        actor_model(),
    )];
    let events = [(
        disk("examples/support.evt").display().to_string(),
        evt_model("examples/support.evt", "examples/support.mox"),
    )];
    let compilation = rex_driver::compile_usecase_str(
        &disk(display).display().to_string(),
        &read(display),
        &domains,
        &rex_driver::DomainImports::default(),
        &designs,
        &actors,
        &events,
    );
    assert_clean(display, &compilation.diagnostics);
    compilation.model.expect("use cases must lower")
}

fn cmmn_model() -> rex_ir::cmmn::CmmnModel {
    let display = "examples/support.cmmn";
    let domains = [(
        disk("examples/support.mox").display().to_string(),
        read("examples/support.mox"),
    )];
    let designs = [(
        disk("examples/support.ddd").display().to_string(),
        ddd_model("examples/support.ddd", "examples/support.mox"),
    )];
    let actors = [(
        disk("examples/support.actor").display().to_string(),
        actor_model(),
    )];
    let events = [(
        disk("examples/support.evt").display().to_string(),
        evt_model("examples/support.evt", "examples/support.mox"),
    )];
    let decisions = [(
        disk("examples/support.dmn").display().to_string(),
        dmn_model(),
    )];
    let compilation = rex_driver::compile_cmmn_str(
        &disk(display).display().to_string(),
        &read(display),
        &domains,
        &rex_driver::DomainImports::default(),
        &designs,
        &actors,
        &events,
        &decisions,
    );
    assert_clean(display, &compilation.diagnostics);
    compilation.model.expect("case must lower")
}

fn deploy_model() -> rex_ir::deploy::DeployModel {
    let display = "examples/support.deploy";
    let designs = [(
        disk("examples/support.ddd").display().to_string(),
        ddd_model("examples/support.ddd", "examples/support.mox"),
    )];
    let flows = [(
        disk("examples/support.ifml").display().to_string(),
        flow_model("examples/support.ifml"),
    )];
    let compilation = rex_driver::compile_deploy_str(
        &disk(display).display().to_string(),
        &read(display),
        &designs,
        &flows,
    );
    assert_clean(display, &compilation.diagnostics);
    compilation.model.expect("deployment must lower")
}

/// The full support stack: every surface of the reference application.
fn support_inputs() -> GraphInputs {
    let mut inputs = GraphInputs::default();
    inputs
        .domains
        .push(graph_file("examples/support.mox", support_mox()));
    inputs
        .actors
        .push(graph_file("examples/support.actor", actor_model()));
    inputs.designs.push(graph_file(
        "examples/support.ddd",
        ddd_model("examples/support.ddd", "examples/support.mox"),
    ));
    inputs.events.push(graph_file(
        "examples/support.evt",
        evt_model("examples/support.evt", "examples/support.mox"),
    ));
    inputs
        .deploys
        .push(graph_file("examples/support.deploy", deploy_model()));
    for display in [
        "examples/support.ifml",
        PATTERN_FLOWS[0],
        PATTERN_FLOWS[1],
        PATTERN_FLOWS[2],
        PATTERN_FLOWS[3],
        PATTERN_FLOWS[4],
        PATTERN_FLOWS[5],
    ] {
        inputs.flows.push(graph_file(display, flow_model(display)));
    }
    inputs
        .use_cases
        .push(graph_file("examples/support.uc", uc_model()));
    inputs
        .cases
        .push(graph_file("examples/support.cmmn", cmmn_model()));
    inputs
        .decisions
        .push(graph_file("examples/support.dmn", dmn_model()));
    inputs
}

/// The support stack plus the unrelated `orders` application (its own
/// `ecommerce.mox` bridge, no import path to the support files).
fn two_application_inputs() -> GraphInputs {
    let mut inputs = support_inputs();
    inputs.domains.push(graph_file("examples/ecommerce.mox", {
        let display = "examples/ecommerce.mox";
        let compilation = rex_driver::compile_str(
            &disk(display).display().to_string(),
            &read(display),
            &rex_driver::DomainImports::default(),
        );
        assert_clean(display, &compilation.diagnostics);
        compilation.model.expect("ecommerce.mox must lower")
    }));
    inputs.designs.push(graph_file(
        "examples/orders.ddd",
        ddd_model("examples/orders.ddd", "examples/ecommerce.mox"),
    ));
    inputs
}

// ---------------------------------------------------------------------------
// Assertion helpers
// ---------------------------------------------------------------------------

fn node_ids(graph: &GraphModel) -> BTreeSet<&str> {
    graph.nodes.iter().map(|node| node.id.as_str()).collect()
}

fn node<'a>(graph: &'a GraphModel, id: &str) -> &'a GraphNode {
    graph
        .nodes
        .iter()
        .find(|node| node.id == id)
        .unwrap_or_else(|| {
            let kind = id.split(':').next().unwrap_or_default();
            let available: Vec<&str> = graph
                .nodes
                .iter()
                .filter(|node| node.kind == kind)
                .map(|node| node.id.as_str())
                .collect();
            panic!("missing node {id}; {kind} nodes present: {available:#?}")
        })
}

/// The single edge matching `(from, kind, to)`, with every attribute
/// asserted by the caller.
fn edge<'a>(graph: &'a GraphModel, from: &str, kind: &str, to: &str) -> &'a GraphEdge {
    let matches: Vec<&GraphEdge> = graph
        .edges
        .iter()
        .filter(|edge| edge.from == from && edge.kind == kind && edge.to == to)
        .collect();
    assert_eq!(
        matches.len(),
        1,
        "expected exactly one {kind} edge {from} → {to}, found {}",
        matches.len()
    );
    matches[0]
}

fn assert_no_edge(graph: &GraphModel, from: &str, kind: &str) {
    let found: Vec<&GraphEdge> = graph
        .edges
        .iter()
        .filter(|edge| edge.from == from && edge.kind == kind)
        .collect();
    assert!(
        found.is_empty(),
        "expected no {kind} edge from {from}, found {found:#?}"
    );
}

fn attributes<'a>(edge: &'a GraphEdge, key: &str) -> &'a str {
    edge.attributes
        .get(key)
        .unwrap_or_else(|| panic!("edge {:?} carries no '{key}' attribute", edge))
        .as_str()
}

// ---------------------------------------------------------------------------
// Curated ids (the fixture's contract)
// ---------------------------------------------------------------------------

const MOX: &str = "examples/support.mox";
const ACTOR: &str = "examples/support.actor";
const DDD: &str = "examples/support.ddd";
const EVT: &str = "examples/support.evt";
const DMN: &str = "examples/support.dmn";
const IFML: &str = "examples/support.ifml";
const UC: &str = "examples/support.uc";
const CMMN: &str = "examples/support.cmmn";
const DEPLOY: &str = "examples/support.deploy";

fn class(file: &str, name: &str) -> String {
    format!("{}:{file}#{name}", kinds::CLASS)
}
fn feature(file: &str, name: &str) -> String {
    format!("{}:{file}#{name}", kinds::FEATURE)
}
fn operation(file: &str, name: &str) -> String {
    format!("{}:{file}#{name}", kinds::OPERATION)
}
fn actor(file: &str, name: &str) -> String {
    format!("{}:{file}#{name}", kinds::ACTOR)
}
fn capability(file: &str, name: &str) -> String {
    format!("{}:{file}#{name}", kinds::CAPABILITY)
}
fn service_operation(file: &str, name: &str) -> String {
    format!("{}:{file}#{name}", kinds::SERVICE_OPERATION)
}
fn repository(file: &str, name: &str) -> String {
    format!("{}:{file}#{name}", kinds::REPOSITORY)
}
fn search(file: &str, name: &str) -> String {
    format!("{}:{file}#{name}", kinds::SEARCH)
}
fn event(file: &str, name: &str) -> String {
    format!("{}:{file}#{name}", kinds::EVENT)
}
fn decision(file: &str, name: &str) -> String {
    format!("{}:{file}#{name}", kinds::DECISION)
}
fn view(file: &str, name: &str) -> String {
    format!("{}:{file}#{name}", kinds::VIEW)
}
fn component(file: &str, name: &str) -> String {
    format!("{}:{file}#{name}", kinds::COMPONENT)
}
fn action(file: &str, name: &str) -> String {
    format!("{}:{file}#{name}", kinds::ACTION)
}
fn use_case(file: &str, name: &str) -> String {
    format!("{}:{file}#{name}", kinds::USE_CASE)
}
fn plan_item(file: &str, name: &str) -> String {
    format!("{}:{file}#{name}", kinds::PLAN_ITEM)
}
fn case_role(file: &str, name: &str) -> String {
    format!("{}:{file}#{name}", kinds::CASE_ROLE)
}
fn module(file: &str, name: &str) -> String {
    format!("{}:{file}#{name}", kinds::MODULE)
}
fn deploy_component(file: &str, name: &str) -> String {
    format!("{}:{file}#{name}", kinds::DEPLOY_COMPONENT)
}

// ---------------------------------------------------------------------------
// The tests
// ---------------------------------------------------------------------------

/// The whole node vocabulary the support stack declares, one curated id per
/// kind (the node table in `docs/GRAPH.md`), and the edges that make the
/// stack one application.
#[test]
fn support_stack_emits_the_expected_nodes_and_edges() {
    let library = support_inputs();
    let graph = build(&library);

    let curated = [
        // `.mox` domain.
        class(MOX, "nz.example.support.Ticket"),
        class(MOX, "nz.example.support.User"),
        format!("{}:{MOX}#nz.example.support.TicketState", kinds::ENUM),
        feature(MOX, "nz.example.support.Ticket.needsRefund"),
        feature(MOX, "nz.example.support.User.assignedTickets"),
        operation(MOX, "nz.example.support.Ticket.raiseRefund"),
        // `.actor` policy.
        actor(ACTOR, "Support.Agent"),
        capability(ACTOR, "Support.RaiseRefund"),
        capability(ACTOR, "Support.ApproveRefund"),
        // `.ddd` design.
        format!("{}:{DDD}#SupportDesk", kinds::DESIGN_APPLICATION),
        format!("{}:{DDD}#SupportDesk.desk", kinds::MODULE),
        format!("{}:{DDD}#SupportDesk.desk.TicketService", kinds::SERVICE),
        service_operation(DDD, "SupportDesk.desk.TicketService.raiseRefund"),
        repository(DDD, "SupportDesk.desk.TicketRepository"),
        search(DDD, "SupportDesk.desk.TicketSearch"),
        // `.evt` contract.
        event(EVT, "support.TicketRaised"),
        event(EVT, "support.RefundApproved"),
        format!("{}:{EVT}#support.tickets", kinds::CHANNEL),
        format!("{}:{EVT}#support.refundDesk", kinds::SUBSCRIPTION),
        // `.dmn` decision model.
        format!("{}:{DMN}#RefundDecisions", kinds::DECISION_MODEL),
        decision(DMN, "RefundDecisions.RefundEligibility"),
        format!(
            "{}:{DMN}#RefundDecisions.AssessRefund",
            kinds::DECISION_SERVICE
        ),
        format!("{}:{DMN}#RefundDecisions.assess_refund", kinds::BKM),
        format!("{}:{DMN}#RefundDecisions.ticket", kinds::INPUT_DATA),
        format!(
            "{}:{DMN}#RefundDecisions.ConsumerGuaranteesAct",
            kinds::KNOWLEDGE_SOURCE
        ),
        // `.ifml` flow.
        format!("{}:{IFML}#support", kinds::FLOW),
        view(IFML, "Queue"),
        component(IFML, "Queue.refunds"),
        action(IFML, "ApproveRefund"),
        // `.uc` use case.
        use_case(UC, "RaiseRefund"),
        // `.cmmn` case.
        format!("{}:{CMMN}#RefundDesk", kinds::CASE),
        plan_item(CMMN, "RefundDesk.AssessRefund"),
        format!("{}:{CMMN}#RefundDesk.ticket", kinds::CASE_FILE_ITEM),
        case_role(CMMN, "RefundDesk.finance"),
        // `.deploy` deployment.
        format!("{}:{DEPLOY}#SupportDesk", kinds::DEPLOY_APPLICATION),
        deploy_component(DEPLOY, "SupportDesk.api"),
        format!("{}:{DEPLOY}#production_k8s", kinds::DEPLOY_PROFILE),
        format!("{}:{DEPLOY}#SupportDesk.production", kinds::DEPLOYMENT),
        // The file node of every compiled surface.
        format!("{}:{MOX}#{MOX}", kinds::FILE),
        format!("{}:{DDD}#{DDD}", kinds::FILE),
    ];
    for id in &curated {
        node(&graph, id);
    }

    // Object graph and domain typing.
    let ticket = class(MOX, "nz.example.support.Ticket");
    let user = class(MOX, "nz.example.support.User");
    assert_eq!(
        attributes(edge(&graph, &ticket, kinds::REFERS, &user), "feature"),
        "assignee"
    );
    assert_eq!(
        attributes(
            edge(
                &graph,
                &feature(MOX, "nz.example.support.Ticket.state"),
                kinds::TYPE_OF,
                &format!("{}:{MOX}#nz.example.support.TicketState", kinds::ENUM)
            ),
            "field"
        ),
        "state"
    );
    edge(
        &graph,
        &feature(MOX, "nz.example.support.User.assignedTickets"),
        kinds::OPPOSITE,
        &feature(MOX, "nz.example.support.Ticket.assignee"),
    );

    // Authorization: the grant carries its effect, guard, and obligations.
    let grant = edge(
        &graph,
        &actor(ACTOR, "Support.Agent"),
        kinds::PERMITS,
        &capability(ACTOR, "Support.RaiseRefund"),
    );
    assert_eq!(
        grant.when.as_deref(),
        Some("refundCents > 0 && refundCents <= 5000")
    );
    assert_eq!(grant.obligations, ["audit", "notifyCustomer"]);
    edge(
        &graph,
        &actor(ACTOR, "Support.Agent"),
        kinds::EXTENDS,
        &actor(ACTOR, "Support.Customer"),
    );
    // `never_both`: one edge per direction.
    edge(
        &graph,
        &capability(ACTOR, "Support.RaiseRefund"),
        kinds::EXCLUDES,
        &capability(ACTOR, "Support.ApproveRefund"),
    );
    edge(
        &graph,
        &capability(ACTOR, "Support.ApproveRefund"),
        kinds::EXCLUDES,
        &capability(ACTOR, "Support.RaiseRefund"),
    );

    // Membership: domain members, the design tree, the interaction tree.
    edge(
        &graph,
        &feature(MOX, "nz.example.support.Ticket.state"),
        kinds::MEMBER_OF,
        &ticket,
    );
    edge(
        &graph,
        &operation(MOX, "nz.example.support.Ticket.raiseRefund"),
        kinds::MEMBER_OF,
        &ticket,
    );
    edge(
        &graph,
        &decision(DMN, "RefundDecisions.RefundEligibility"),
        kinds::MEMBER_OF,
        &format!("{}:{DMN}#RefundDecisions", kinds::DECISION_MODEL),
    );
    edge(
        &graph,
        &view(IFML, "Queue"),
        kinds::MEMBER_OF,
        &format!("{}:{IFML}#support", kinds::FLOW),
    );
    edge(
        &graph,
        &plan_item(CMMN, "RefundDesk.AssessRefund"),
        kinds::MEMBER_OF,
        &format!("{}:{CMMN}#RefundDesk", kinds::CASE),
    );

    // Design joins and the signature types.
    edge(
        &graph,
        &service_operation(DDD, "SupportDesk.desk.TicketService.raiseRefund"),
        kinds::MEMBER_OF,
        &format!("{}:{DDD}#SupportDesk.desk.TicketService", kinds::SERVICE),
    );
    edge(
        &graph,
        &repository(DDD, "SupportDesk.desk.TicketRepository"),
        kinds::DESIGNS,
        &ticket,
    );
    edge(
        &graph,
        &service_operation(DDD, "SupportDesk.desk.TicketService.getTicket"),
        kinds::TYPE_OF,
        &ticket,
    );
    edge(
        &graph,
        &format!("{}:{DDD}#SupportDesk.desk.TicketService", kinds::SERVICE),
        kinds::INJECTS,
        &repository(DDD, "SupportDesk.desk.TicketRepository"),
    );
    edge(
        &graph,
        &service_operation(DDD, "SupportDesk.desk.TicketService.close"),
        kinds::FORWARDS_TO,
        &format!(
            "{}:{DDD}#SupportDesk.desk.TicketRepository.delete",
            kinds::REPOSITORY_OPERATION
        ),
    );

    // Event graph.
    edge(
        &graph,
        &format!("{}:{EVT}#support.tickets", kinds::CHANNEL),
        kinds::PUBLISHES,
        &event(EVT, "support.TicketRaised"),
    );
    edge(
        &graph,
        &format!("{}:{EVT}#support.analytics", kinds::SUBSCRIPTION),
        kinds::SUBSCRIBES,
        &event(EVT, "support.TicketRaised"),
    );

    // Interaction graph.
    edge(
        &graph,
        &component(IFML, "Queue.refunds"),
        kinds::DATA,
        &ticket,
    );
    assert_eq!(
        attributes(
            edge(
                &graph,
                &component(IFML, "Queue.refunds"),
                kinds::READS_FEATURE,
                &feature(MOX, "nz.example.support.Ticket.title")
            ),
            "via"
        ),
        "column"
    );
    // `on select(row) -> navigate("Queue")` on Inbox's escalations list —
    // declared views are the navigate targets, and the firing control rides
    // along.
    let navigation = edge(
        &graph,
        &component(IFML, "Inbox.escalations"),
        kinds::NAVIGATES,
        &view(IFML, "Queue"),
    );
    assert_eq!(attributes(navigation, "trigger"), "select");
    assert_eq!(attributes(navigation, "source"), "Inbox.escalations");
    edge(
        &graph,
        &component(IFML, "Queue.refunds"),
        kinds::INVOKES_ACTION,
        &action(IFML, "ApproveRefund"),
    );

    // Deployment graph.
    edge(
        &graph,
        &deploy_component(DEPLOY, "SupportDesk.api"),
        kinds::CONNECTS,
        &deploy_component(DEPLOY, "SupportDesk.db"),
    );
    edge(
        &graph,
        &format!("{}:{DEPLOY}#SupportDesk.production", kinds::DEPLOYMENT),
        kinds::USES_PROFILE,
        &format!("{}:{DEPLOY}#production_k8s", kinds::DEPLOY_PROFILE),
    );
    edge(
        &graph,
        &format!("{}:{DEPLOY}#SupportDesk.production", kinds::DEPLOYMENT),
        kinds::DEPLOYS,
        &format!("{}:{DEPLOY}#SupportDesk", kinds::DEPLOY_APPLICATION),
    );

    // One application: every surface imports support.mox (or a sibling), so
    // the stack clusters as exactly one component carrying every file.
    assert_eq!(graph.applications.len(), 1, "{:#?}", graph.applications);
    // The application id is the component's smallest member path.
    assert_eq!(graph.applications[0].id, "examples/support.actor");
    assert_eq!(graph.applications[0].packages, ["nz.example.support"]);
    assert!(
        graph.applications[0].files.contains(&DDD.to_string()),
        "{:?}",
        graph.applications[0].files
    );
    // Import edges are file → file.
    edge(
        &graph,
        &format!("{}:{IFML}#{IFML}", kinds::FILE),
        kinds::IMPORTS,
        &format!("{}:{MOX}#{MOX}", kinds::FILE),
    );
}

#[test]
fn capability_on_class_edges_resolve_across_surfaces() {
    // `.actor` `capability RaiseRefund on Ticket` → the `.mox` class node.
    let graph = build(&support_inputs());
    edge(
        &graph,
        &capability(ACTOR, "Support.RaiseRefund"),
        kinds::ON,
        &class(MOX, "nz.example.support.Ticket"),
    );
    edge(
        &graph,
        &capability(ACTOR, "Support.ReadTicket"),
        kinds::ON,
        &class(MOX, "nz.example.support.Ticket"),
    );
}

#[test]
fn ddd_capability_clauses_resolve_to_actor_capabilities() {
    let graph = build(&support_inputs());
    edge(
        &graph,
        &service_operation(DDD, "SupportDesk.desk.TicketService.raiseRefund"),
        kinds::CARRIES,
        &capability(ACTOR, "Support.RaiseRefund"),
    );
    edge(
        &graph,
        &service_operation(DDD, "SupportDesk.desk.TicketService.approveRefund"),
        kinds::CARRIES,
        &capability(ACTOR, "Support.ApproveRefund"),
    );
    // Searches carry capabilities through the same pooled universe.
    edge(
        &graph,
        &search(DDD, "SupportDesk.desk.TicketSearch"),
        kinds::CARRIES,
        &capability(ACTOR, "Support.ReadTicket"),
    );
    // A search's text field reads the designed class's feature (the same
    // feature also backs the sort key, so the vias are distinct edges).
    let reads: Vec<&GraphEdge> = graph
        .edges
        .iter()
        .filter(|edge| {
            edge.from == search(DDD, "SupportDesk.desk.TicketSearch")
                && edge.kind == kinds::READS_FEATURE
                && edge.to == feature(MOX, "nz.example.support.Ticket.title")
        })
        .collect();
    let mut vias: Vec<&str> = reads.iter().map(|edge| attributes(edge, "via")).collect();
    vias.sort_unstable();
    assert_eq!(vias, ["sort", "text"]);
}

/// An inline artifact surface joined to the support application through the
/// domain import (the display path is a pseudo path: the file need not exist,
/// the graph only needs its identity and imports).
fn inline_file<M>(display: &str, imports: &[&str], model: M) -> GraphFile<M> {
    GraphFile::new(
        display,
        display,
        imports
            .iter()
            .map(|display| disk(display).display().to_string())
            .collect(),
        model,
    )
}

fn inline_ddd(display: &str, source: &str) -> rex_ir::ddd::DddModel {
    let domains = [(
        disk("examples/support.mox").display().to_string(),
        read("examples/support.mox"),
    )];
    // The compile path resolves imports lexically, so it lives next to the
    // real examples even though the file itself is source text.
    let compile_path = disk("examples")
        .join(display.rsplit('/').next().unwrap_or(display))
        .display()
        .to_string();
    let compilation = rex_driver::compile_ddd_str(
        &compile_path,
        source,
        &domains,
        &rex_driver::DomainImports::default(),
    );
    assert_clean(display, &compilation.diagnostics);
    compilation.model.expect("inline design must lower")
}

fn inline_ifml(display: &str, source: &str) -> rex_ir::ifml::IfmlModel {
    let compilation =
        rex_ifml::compile_ifml_str(display, source, &rex_ifml::IfmlImports::default())
            .unwrap_or_else(|diagnostics| panic!("{display} must compile: {diagnostics:?}"));
    compilation.model
}

#[test]
fn ddd_invokes_resolve_to_decision_services() {
    let source = concat!(
        "import \"support.mox\";\n",
        "\n",
        "application Inline {\n",
        "    base nz.example.support\n",
        "    module desk {\n",
        "        service TicketService {\n",
        "            inject decision AssessRefund;\n",
        "            boolean assess(String ticketNo) invokes AssessRefund;\n",
        "        }\n",
        "    }\n",
        "}\n",
    );
    let mut inputs = support_inputs();
    inputs.designs.push(inline_file(
        "examples/inline-decisions.ddd",
        &["examples/support.mox"],
        inline_ddd("examples/inline-decisions.ddd", source),
    ));
    let graph = build(&inputs);
    let assess = service_operation(
        "examples/inline-decisions.ddd",
        "Inline.desk.TicketService.assess",
    );
    let decision_service = format!(
        "{}:{DMN}#RefundDecisions.AssessRefund",
        kinds::DECISION_SERVICE
    );
    assert_eq!(
        attributes(
            edge(&graph, &assess, kinds::INVOKES_DECISION, &decision_service),
            "via"
        ),
        "invokes"
    );
    // The service-level `inject decision` link is the same edge, `via =
    // "inject"`.
    edge(
        &graph,
        &format!(
            "{}:examples/inline-decisions.ddd#Inline.desk.TicketService",
            kinds::SERVICE
        ),
        kinds::INVOKES_DECISION,
        &decision_service,
    );
    // The decision model's own decision → event edge (support.dmn) carries the
    // verbatim FEEL condition.
    let emits = edge(
        &graph,
        &decision(DMN, "RefundDecisions.RefundAssessment"),
        kinds::EMITS,
        &event(EVT, "support.RefundApproved"),
    );
    assert_eq!(emits.when.as_deref(), Some("eligible"));
}

#[test]
fn ddd_emits_resolve_to_events() {
    let source = concat!(
        "import \"support.mox\";\n",
        "\n",
        "application Inline {\n",
        "    base nz.example.support\n",
        "    module desk {\n",
        "        service TicketService {\n",
        "            boolean approve(String ticketNo) emits RefundApproved;\n",
        "        }\n",
        "    }\n",
        "}\n",
    );
    let mut inputs = support_inputs();
    inputs.designs.push(inline_file(
        "examples/inline-emits.ddd",
        &["examples/support.mox"],
        inline_ddd("examples/inline-emits.ddd", source),
    ));
    let graph = build(&inputs);
    edge(
        &graph,
        &service_operation(
            "examples/inline-emits.ddd",
            "Inline.desk.TicketService.approve",
        ),
        kinds::EMITS,
        &event(EVT, "support.RefundApproved"),
    );
}

#[test]
fn action_binds_to_operation_by_case_insensitive_name() {
    let graph = build(&support_inputs());
    // `ApproveRefund` ↔ `approveRefund`: equality after ASCII lowercasing.
    edge(
        &graph,
        &action(IFML, "ApproveRefund"),
        kinds::BINDS,
        &service_operation(DDD, "SupportDesk.desk.TicketService.approveRefund"),
    );
    edge(
        &graph,
        &action(IFML, "RaiseRefund"),
        kinds::BINDS,
        &service_operation(DDD, "SupportDesk.desk.TicketService.raiseRefund"),
    );
}

#[test]
fn ambiguous_bindings_are_flagged() {
    // Two services declare `approveRefund`; the action's case-insensitive
    // mirror names both, so both edges carry `ambiguous = "true"`.
    let design = concat!(
        "import \"support.mox\";\n",
        "\n",
        "application Inline {\n",
        "    base nz.example.support\n",
        "    module desk {\n",
        "        service FirstService {\n",
        "            boolean approveRefund(String ticketNo);\n",
        "        }\n",
        "        service SecondService {\n",
        "            boolean approveRefund(String ticketNo);\n",
        "        }\n",
        "    }\n",
        "}\n",
    );
    let flow = "action \"ApproveRefund\" {\n    target: Ticket;\n}\n";
    // A minimal application: the two carriers must be the only candidates.
    let mut inputs = GraphInputs::default();
    inputs
        .domains
        .push(graph_file("examples/support.mox", support_mox()));
    inputs.designs.push(inline_file(
        "examples/inline-ambiguous.ddd",
        &["examples/support.mox"],
        inline_ddd("examples/inline-ambiguous.ddd", design),
    ));
    inputs.flows.push(inline_file(
        "examples/inline-ambiguous.ifml",
        &["examples/support.mox"],
        inline_ifml("examples/inline-ambiguous.ifml", flow),
    ));
    let graph = build(&inputs);
    let action_id = action("examples/inline-ambiguous.ifml", "ApproveRefund");
    let mut binds: Vec<&GraphEdge> = graph
        .edges
        .iter()
        .filter(|edge| edge.from == action_id && edge.kind == kinds::BINDS)
        .collect();
    binds.sort_by(|left, right| left.to.cmp(&right.to));
    assert_eq!(binds.len(), 2, "{binds:#?}");
    for binding in &binds {
        assert_eq!(attributes(binding, "ambiguous"), "true");
    }
    assert_eq!(
        binds[0].to,
        service_operation(
            "examples/inline-ambiguous.ddd",
            "Inline.desk.FirstService.approveRefund"
        )
    );
    assert_eq!(
        binds[1].to,
        service_operation(
            "examples/inline-ambiguous.ddd",
            "Inline.desk.SecondService.approveRefund"
        )
    );
}

#[test]
fn deploy_component_binds_design_module_and_flow() {
    let graph = build(&support_inputs());
    let binding = edge(
        &graph,
        &deploy_component(DEPLOY, "SupportDesk.api"),
        kinds::BINDS_DESIGN,
        &format!("{}:{DDD}#SupportDesk.desk", kinds::MODULE),
    );
    assert_eq!(attributes(binding, "module"), "desk");
    edge(
        &graph,
        &deploy_component(DEPLOY, "SupportDesk.web"),
        kinds::BINDS_FLOW,
        &format!("{}:{IFML}#support", kinds::FLOW),
    );
}

#[test]
fn subscription_consumer_resolves_to_deploy_component() {
    let graph = build(&support_inputs());
    edge(
        &graph,
        &format!("{}:{EVT}#support.refundDesk", kinds::SUBSCRIPTION),
        kinds::DELIVERS,
        &deploy_component(DEPLOY, "SupportDesk.refund_approval_service"),
    );
    edge(
        &graph,
        &format!("{}:{EVT}#support.analytics", kinds::SUBSCRIPTION),
        kinds::DELIVERS,
        &deploy_component(DEPLOY, "SupportDesk.analytics_service"),
    );
}

#[test]
fn cmmn_execute_and_decision_invocation_resolve() {
    let graph = build(&support_inputs());
    edge(
        &graph,
        &plan_item(CMMN, "RefundDesk.AssessRefund"),
        kinds::EXECUTES,
        &service_operation(DDD, "SupportDesk.desk.TicketService.raiseRefund"),
    );
    let invocation = edge(
        &graph,
        &plan_item(CMMN, "RefundDesk.AssessAuthority"),
        kinds::INVOKES_DECISION,
        &format!(
            "{}:{DMN}#RefundDecisions.AssessRefund",
            kinds::DECISION_SERVICE
        ),
    );
    assert_eq!(attributes(invocation, "via"), "decision");
    edge(
        &graph,
        &plan_item(CMMN, "RefundDesk.ReviewTicket"),
        kinds::ASSIGNED_TO,
        &case_role(CMMN, "RefundDesk.triage"),
    );
    let role = edge(
        &graph,
        &case_role(CMMN, "RefundDesk.finance"),
        kinds::REQUIRES,
        &actor(ACTOR, "Support.Finance"),
    );
    assert_eq!(attributes(role, "via"), "roles");
    assert_eq!(
        attributes(
            edge(
                &graph,
                &format!("{}:{CMMN}#RefundDesk.ticket", kinds::CASE_FILE_ITEM),
                kinds::TYPE_OF,
                &class(MOX, "nz.example.support.Ticket"),
            ),
            "field"
        ),
        "ticket"
    );
}

#[test]
fn uc_realize_and_steps_resolve() {
    let graph = build(&support_inputs());
    let raise_refund = service_operation(DDD, "SupportDesk.desk.TicketService.raiseRefund");
    // The declared `realizes support.TicketService.raiseRefund` and the step's
    // `raise: raiseRefund` are one edge (exact duplicates collapse).
    edge(
        &graph,
        &use_case(UC, "RaiseRefund"),
        kinds::REALIZES,
        &raise_refund,
    );
    // The step's `requires [ RaiseRefund ]` pools to the actor capability.
    let requirement = edge(
        &graph,
        &use_case(UC, "RaiseRefund"),
        kinds::REQUIRES,
        &capability(ACTOR, "Support.RaiseRefund"),
    );
    assert_eq!(attributes(requirement, "via"), "ucStep");
    // The subject's domain type.
    assert_eq!(
        attributes(
            edge(
                &graph,
                &use_case(UC, "RaiseRefund"),
                kinds::TYPE_OF,
                &class(MOX, "nz.example.support.Ticket"),
            ),
            "field"
        ),
        "Ticket"
    );
    edge(
        &graph,
        &use_case(UC, "ApproveRefund"),
        kinds::REALIZES,
        &service_operation(DDD, "SupportDesk.desk.TicketService.approveRefund"),
    );
}

#[test]
fn machine_transitions_link_operations_and_state_enums() {
    // The conformance machine fixture: payrun lifecycle with explicit
    // `transition <Enum>.<Trigger>` bindings on the workflow operations.
    const DISPLAY: &str = "tests/conformance/ddd/timesheet.ddd";
    const DOMAIN: &str = "tests/conformance/ddd/timesheet.mox";
    let mut inputs = GraphInputs::default();
    inputs.domains.push(graph_file(DOMAIN, {
        let compilation = rex_driver::compile_str(
            &disk(DOMAIN).display().to_string(),
            &read(DOMAIN),
            &rex_driver::DomainImports::default(),
        );
        assert_clean(DOMAIN, &compilation.diagnostics);
        compilation.model.expect("timesheet.mox must lower")
    }));
    inputs
        .designs
        .push(graph_file(DISPLAY, ddd_model(DISPLAY, DOMAIN)));
    let graph = build(&inputs);

    let machine = format!(
        "{}:{DISPLAY}#Timesheet.payroll.PayRun",
        kinds::STATE_MACHINE
    );
    assert_eq!(
        node(&graph, &machine)
            .attributes
            .get("field")
            .map(String::as_str),
        Some("status")
    );
    node(
        &graph,
        &format!(
            "{}:{DISPLAY}#Timesheet.payroll.Timecard",
            kinds::STATE_MACHINE
        ),
    );

    let transition = format!(
        "{}:{DISPLAY}#Timesheet.payroll.PayRun.Calculating->AwaitingApproval",
        kinds::TRANSITION
    );
    node(&graph, &transition);
    edge(&graph, &transition, kinds::MEMBER_OF, &machine);
    // The transition's state set is the machine's enum.
    assert_eq!(
        attributes(
            edge(
                &graph,
                &transition,
                kinds::TYPE_OF,
                &format!("{}:{DOMAIN}#nz.example.timesheet.PayRunStatus", kinds::ENUM)
            ),
            "field"
        ),
        "status"
    );
    // `submit => … transition PayRunStatus.Submit` names the transition whose
    // `via` trigger is Submit.
    edge(
        &graph,
        &service_operation(DISPLAY, "Timesheet.payroll.PayRunService.submit"),
        kinds::TRIGGERS,
        &transition,
    );
    edge(
        &graph,
        &service_operation(DISPLAY, "Timesheet.payroll.PayRunService.approve"),
        kinds::TRIGGERS,
        &format!(
            "{}:{DISPLAY}#Timesheet.payroll.PayRun.AwaitingApproval->Posted",
            kinds::TRANSITION
        ),
    );
    // A transition between the same two states again would be suffixed
    // `~<n>`; this machine has one per pair, so no suffix node exists.
    assert!(
        !node_ids(&graph)
            .iter()
            .any(|id| id.contains("Calculating->AwaitingApproval~")),
        "single transitions carry no suffix"
    );
}

#[test]
fn unresolved_references_produce_no_edges() {
    let design = concat!(
        "import \"support.mox\";\n",
        "\n",
        "application Inline {\n",
        "    base nz.example.support\n",
        "    module desk {\n",
        "        service TicketService {\n",
        "            boolean approve() capability NoSuchCapability emits NoSuchEvent;\n",
        "        }\n",
        "    }\n",
        "}\n",
    );
    let flow = concat!(
        "view \"Desk\" {\n",
        "    requires: [ NoSuchCapability ];\n",
        "    on load -> navigate(\"Nowhere\");\n",
        "}\n",
        "action \"NoSuchAction\" {\n",
        "    target: Ticket;\n",
        "}\n",
    );
    let mut inputs = support_inputs();
    inputs.designs.push(inline_file(
        "examples/inline-unresolved.ddd",
        &["examples/support.mox"],
        inline_ddd("examples/inline-unresolved.ddd", design),
    ));
    inputs.flows.push(inline_file(
        "examples/inline-unresolved.ifml",
        &["examples/support.mox"],
        inline_ifml("examples/inline-unresolved.ifml", flow),
    ));
    let graph = build(&inputs);

    let operation = service_operation(
        "examples/inline-unresolved.ddd",
        "Inline.desk.TicketService.approve",
    );
    assert_no_edge(&graph, &operation, kinds::CARRIES);
    assert_no_edge(&graph, &operation, kinds::EMITS);
    let view_id = view("examples/inline-unresolved.ifml", "Desk");
    assert_no_edge(&graph, &view_id, kinds::REQUIRES);
    assert_no_edge(&graph, &view_id, kinds::NAVIGATES);
    assert_no_edge(
        &graph,
        &action("examples/inline-unresolved.ifml", "NoSuchAction"),
        kinds::BINDS,
    );
    // The dangling names never become nodes.
    assert!(!node_ids(&graph)
        .iter()
        .any(|id| id.contains("NoSuchCapability") || id.contains("Nowhere")));
}

#[test]
fn two_unrelated_applications_do_not_merge() {
    let graph = build(&two_application_inputs());
    assert_eq!(graph.applications.len(), 2, "{:#?}", graph.applications);
    assert_eq!(
        graph.applications[0].id, "examples/ecommerce.mox",
        "applications sort by id"
    );
    assert_eq!(graph.applications[0].packages, ["nz.example.shop"]);
    assert_eq!(graph.applications[1].id, "examples/support.actor");
    assert_eq!(graph.applications[1].packages, ["nz.example.support"]);

    // No edge crosses an application boundary: the pooled resolution never
    // leaves the component the referencing node belongs to.
    let application_of: BTreeMap<&str, &str> = graph
        .nodes
        .iter()
        .map(|node| (node.id.as_str(), node.application.as_str()))
        .collect();
    for edge in &graph.edges {
        let from = application_of
            .get(edge.from.as_str())
            .unwrap_or_else(|| panic!("edge from undeclared node {}", edge.from));
        let to = application_of
            .get(edge.to.as_str())
            .unwrap_or_else(|| panic!("edge to undeclared node {}", edge.to));
        assert_eq!(
            from, to,
            "edge {} -{}-> {} crosses applications",
            edge.from, edge.kind, edge.to
        );
    }
    // The shop application's design resolved inside itself.
    edge(
        &graph,
        &repository("examples/orders.ddd", "Orders.ordering.OrderRepository"),
        kinds::DESIGNS,
        &class("examples/ecommerce.mox", "nz.example.shop.Order"),
    );
    // `extend Order { OrderLine addLine(OrderLine line) => lines.add; }` — a
    // module-level child operation is a serviceOperation node owned by the
    // module, typed by the same `typeOf` rules as any operation.
    let add_line = service_operation("examples/orders.ddd", "Orders.ordering.Order.addLine");
    edge(
        &graph,
        &add_line,
        kinds::MEMBER_OF,
        &module("examples/orders.ddd", "Orders.ordering"),
    );
    // The return type and the parameter share the type but not the role.
    let mut typed: Vec<(&str, &str)> = graph
        .edges
        .iter()
        .filter(|edge| edge.from == add_line && edge.kind == kinds::TYPE_OF)
        .map(|edge| (attributes(edge, "role"), edge.to.as_str()))
        .collect();
    typed.sort_unstable();
    let line = class("examples/ecommerce.mox", "nz.example.shop.OrderLine");
    assert_eq!(typed, [("param", line.as_str()), ("return", line.as_str())]);

    // The shop application's object graph: `contains` and its back-pointer's
    // `opposite` are the same relation from each end.
    let order = class("examples/ecommerce.mox", "nz.example.shop.Order");
    let line = class("examples/ecommerce.mox", "nz.example.shop.OrderLine");
    assert_eq!(
        attributes(edge(&graph, &order, kinds::CONTAINS, &line), "feature"),
        "lines"
    );
    edge(
        &graph,
        &feature("examples/ecommerce.mox", "nz.example.shop.Order.lines"),
        kinds::OPPOSITE,
        &feature("examples/ecommerce.mox", "nz.example.shop.OrderLine.order"),
    );

    // The two applications' file sets stay disjoint.
    let shop_files = &graph.applications[0].files;
    let support_files = &graph.applications[1].files;
    assert!(shop_files.iter().any(|file| file == "examples/orders.ddd"));
    assert!(support_files
        .iter()
        .any(|file| file == "examples/support.ddd"));
    assert!(shop_files.iter().all(|file| !support_files.contains(file)));
}

#[test]
fn reversed_input_order_is_byte_identical() {
    let mut forward = support_inputs();
    let expected = build(&forward).to_json_pretty().expect("serialize graph");

    forward.domains.reverse();
    forward.actors.reverse();
    forward.designs.reverse();
    forward.events.reverse();
    forward.deploys.reverse();
    forward.flows.reverse();
    forward.use_cases.reverse();
    forward.cases.reverse();
    forward.decisions.reverse();
    // Shuffling every bucket must not move a byte: applications, nodes, and
    // edges are all ordered by content.
    let reversed = build(&forward).to_json_pretty().expect("serialize graph");
    assert_eq!(reversed, expected);

    let reloaded = GraphModel::from_json(&reversed).expect("graph must round-trip");
    assert_eq!(reloaded.to_json_pretty().expect("re-serialize"), reversed);
}

#[test]
fn artifact_contains_no_absolute_paths() {
    let graph = build(&two_application_inputs());
    let json = graph.to_json_pretty().expect("serialize graph");
    let root = workspace_root().display().to_string();
    assert!(
        !json.contains(&root),
        "the artifact must never carry the host's absolute root ({root})"
    );
    assert!(!json.contains("rexVersion"), "no provenance stamp");
    for node in &graph.nodes {
        assert!(
            !node.file.starts_with('/') && !node.file.contains('\\'),
            "node file must be a root-relative POSIX path: {}",
            node.file
        );
        assert!(
            !node.id.contains('\\'),
            "node id must be POSIX: {}",
            node.id
        );
        assert_eq!(
            node.id,
            format!("{}:{}#{}", node.kind, node.file, node.name),
            "every node id follows <kind>:<file>#<name>"
        );
    }
    for application in &graph.applications {
        for file in &application.files {
            assert!(
                !file.starts_with('/') && !file.contains('\\'),
                "application file must be a display path: {file}"
            );
        }
    }
    // No dedup or ordering violations survive into the artifact.
    let mut ids: Vec<&str> = graph.nodes.iter().map(|node| node.id.as_str()).collect();
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids.len(), graph.nodes.len(), "node ids are unique");
    for pair in graph.nodes.windows(2) {
        assert!(
            (&pair[0].application, &pair[0].file, &pair[0].id)
                <= (&pair[1].application, &pair[1].file, &pair[1].id),
            "nodes sort by (application, file, id)"
        );
    }
    for pair in graph.edges.windows(2) {
        assert!(
            (
                &pair[0].from,
                &pair[0].kind,
                &pair[0].to,
                &pair[0].attributes
            ) <= (
                &pair[1].from,
                &pair[1].kind,
                &pair[1].to,
                &pair[1].attributes
            ),
            "edges sort by (from, kind, to, attributes)"
        );
    }
}

/// Every kind the builder emits is a v1 vocabulary constant (no typos, no
/// invented kinds), and every curated kind the support stack can declare
/// appears. This is the anti-drift gate between the emitters and
/// `rex_ir::graph::kinds`.
#[test]
fn emitted_kinds_are_the_published_vocabulary() {
    const NODE_KINDS: [&str; 41] = [
        kinds::FILE,
        kinds::CLASS,
        kinds::ENUM,
        kinds::DATATYPE,
        kinds::FEATURE,
        kinds::OPERATION,
        kinds::ACTOR,
        kinds::CAPABILITY,
        kinds::DESIGN_APPLICATION,
        kinds::MODULE,
        kinds::SERVICE,
        kinds::SERVICE_OPERATION,
        kinds::REPOSITORY,
        kinds::REPOSITORY_OPERATION,
        kinds::SEARCH,
        kinds::STATE_MACHINE,
        kinds::TRANSITION,
        kinds::DECISION_MODEL,
        kinds::DECISION,
        kinds::DECISION_SERVICE,
        kinds::BKM,
        kinds::INPUT_DATA,
        kinds::KNOWLEDGE_SOURCE,
        kinds::EVENT,
        kinds::CHANNEL,
        kinds::SUBSCRIPTION,
        kinds::FLOW,
        kinds::VIEW,
        kinds::CONTAINER,
        kinds::COMPONENT,
        kinds::ACTION,
        kinds::COMMAND,
        kinds::USE_CASE,
        kinds::CASE,
        kinds::PLAN_ITEM,
        kinds::CASE_FILE_ITEM,
        kinds::CASE_ROLE,
        kinds::DEPLOY_APPLICATION,
        kinds::DEPLOY_COMPONENT,
        kinds::DEPLOY_PROFILE,
        kinds::DEPLOYMENT,
    ];
    const EDGE_KINDS: [&str; 40] = [
        kinds::IMPORTS,
        kinds::MEMBER_OF,
        kinds::EXTENDS,
        kinds::CONTAINS,
        kinds::REFERS,
        kinds::OPPOSITE,
        kinds::TYPE_OF,
        kinds::ON,
        kinds::CARRIES,
        kinds::PERMITS,
        kinds::FORBIDS,
        kinds::OPAQUE_GRANT,
        kinds::DELEGATES,
        kinds::EXCLUDES,
        kinds::REQUIRES,
        kinds::INVOKES_DECISION,
        kinds::BINDS,
        kinds::REALIZES,
        kinds::EXECUTES,
        kinds::EMITS,
        kinds::PUBLISHES,
        kinds::SUBSCRIBES,
        kinds::DELIVERS,
        kinds::NAVIGATES,
        kinds::REFRESHES,
        kinds::INVOKES_ACTION,
        kinds::DATA,
        kinds::READS_FEATURE,
        kinds::BINDS_DESIGN,
        kinds::BINDS_FLOW,
        kinds::CONNECTS,
        kinds::USES_MODULE,
        kinds::DESIGNS,
        kinds::FORWARDS_TO,
        kinds::INJECTS,
        kinds::TRIGGERS,
        kinds::ASSIGNED_TO,
        kinds::PROVIDES,
        kinds::USES_PROFILE,
        kinds::DEPLOYS,
    ];
    // A merged tree: both example applications plus the CLI flow (commands)
    // and the conformance interaction model (containers, refreshes), so every
    // v1 kind the fixtures can reach is exercised.
    let mut inputs = two_application_inputs();
    inputs.flows.push(graph_file(
        "examples/cli.ifml",
        flow_model("examples/cli.ifml"),
    ));
    inputs.flows.push(graph_file(
        "tests/conformance/ifml/app.ifml",
        flow_model("tests/conformance/ifml/app.ifml"),
    ));
    let graph = build(&inputs);
    let mut seen_nodes: BTreeSet<&str> =
        graph.nodes.iter().map(|node| node.kind.as_str()).collect();
    let mut seen_edges: BTreeSet<&str> =
        graph.edges.iter().map(|edge| edge.kind.as_str()).collect();

    for kind in seen_nodes.iter() {
        assert!(NODE_KINDS.contains(kind), "undocumented node kind {kind}");
    }
    for kind in seen_edges.iter() {
        assert!(EDGE_KINDS.contains(kind), "undocumented edge kind {kind}");
    }
    // The vocabulary coverage the support stack + CLI flow reach today.
    for kind in [
        kinds::FILE,
        kinds::CLASS,
        kinds::ENUM,
        kinds::FEATURE,
        kinds::OPERATION,
        kinds::ACTOR,
        kinds::CAPABILITY,
        kinds::DESIGN_APPLICATION,
        kinds::MODULE,
        kinds::SERVICE,
        kinds::SERVICE_OPERATION,
        kinds::REPOSITORY,
        kinds::REPOSITORY_OPERATION,
        kinds::SEARCH,
        kinds::DECISION_MODEL,
        kinds::DECISION,
        kinds::DECISION_SERVICE,
        kinds::BKM,
        kinds::INPUT_DATA,
        kinds::KNOWLEDGE_SOURCE,
        kinds::EVENT,
        kinds::CHANNEL,
        kinds::SUBSCRIPTION,
        kinds::FLOW,
        kinds::VIEW,
        kinds::CONTAINER,
        kinds::COMPONENT,
        kinds::ACTION,
        kinds::COMMAND,
        kinds::USE_CASE,
        kinds::CASE,
        kinds::PLAN_ITEM,
        kinds::CASE_FILE_ITEM,
        kinds::CASE_ROLE,
        kinds::DEPLOY_APPLICATION,
        kinds::DEPLOY_COMPONENT,
        kinds::DEPLOY_PROFILE,
        kinds::DEPLOYMENT,
    ] {
        assert!(seen_nodes.contains(kind), "no {kind} node emitted");
        seen_nodes.remove(kind);
    }
    for kind in [
        kinds::CONTAINS,
        kinds::INJECTS,
        kinds::OPAQUE_GRANT,
        kinds::REFRESHES,
        kinds::IMPORTS,
        kinds::MEMBER_OF,
        kinds::EXTENDS,
        kinds::REFERS,
        kinds::OPPOSITE,
        kinds::TYPE_OF,
        kinds::ON,
        kinds::CARRIES,
        kinds::PERMITS,
        kinds::FORBIDS,
        kinds::EXCLUDES,
        kinds::REQUIRES,
        kinds::INVOKES_DECISION,
        kinds::BINDS,
        kinds::REALIZES,
        kinds::EXECUTES,
        kinds::EMITS,
        kinds::PUBLISHES,
        kinds::SUBSCRIBES,
        kinds::DELIVERS,
        kinds::NAVIGATES,
        kinds::INVOKES_ACTION,
        kinds::DATA,
        kinds::READS_FEATURE,
        kinds::BINDS_DESIGN,
        kinds::BINDS_FLOW,
        kinds::CONNECTS,
        kinds::USES_MODULE,
        kinds::DESIGNS,
        kinds::FORWARDS_TO,
        kinds::ASSIGNED_TO,
        kinds::PROVIDES,
        kinds::USES_PROFILE,
        kinds::DEPLOYS,
    ] {
        assert!(seen_edges.contains(kind), "no {kind} edge emitted");
        seen_edges.remove(kind);
    }
    // Kinds this fixture does not exercise yet (documented, not silent): a
    // datatype (`type … wraps`), a `.ddd` machine (the machine test covers
    // them), refreshes, a `requires` on a deploy component, a bare `cedar`
    // opaque grant, and a delegation (the `payrun.actor` fixture covers the
    // last two).
    for kind in seen_nodes {
        assert!(
            [kinds::DATATYPE, kinds::STATE_MACHINE, kinds::TRANSITION].contains(&kind),
            "unexpected uncovered node kind {kind}"
        );
    }
    for kind in seen_edges {
        assert!(
            [kinds::DELEGATES, kinds::TRIGGERS].contains(&kind),
            "unexpected uncovered edge kind {kind}"
        );
    }
}

/// A view's containers (nested naming), a `refresh` target, and a
/// same-file `use` — the conformance interaction model, which declares all
/// three and needs no domain.
#[test]
fn containers_refresh_and_module_uses_emit() {
    const FLOW: &str = "tests/conformance/ifml/app.ifml";
    let mut inputs = GraphInputs::default();
    inputs.flows.push(graph_file(FLOW, flow_model(FLOW)));
    let graph = build(&inputs);

    let summary = format!("{}:{FLOW}#Report.Summary", kinds::CONTAINER);
    let detail = format!("{}:{FLOW}#Report.Detail", kinds::CONTAINER);
    node(&graph, &summary);
    node(&graph, &detail);
    // A component inside a container is named by its declaration path.
    node(
        &graph,
        &format!("{}:{FLOW}#Report.Detail.rows", kinds::COMPONENT),
    );

    // `on load -> refresh("recentOrders")` from the Dashboard view.
    edge(
        &graph,
        &view(FLOW, "Dashboard"),
        kinds::REFRESHES,
        &component(FLOW, "Dashboard.recentOrders"),
    );
    // `on select(row) -> navigate("CustomerDetail")` from that list.
    edge(
        &graph,
        &component(FLOW, "Dashboard.recentOrders"),
        kinds::NAVIGATES,
        &view(FLOW, "CustomerDetail"),
    );
    // `use "Pagination" as pager` — the module is declared in this file.
    let usage = edge(
        &graph,
        &view(FLOW, "Dashboard"),
        kinds::USES_MODULE,
        &format!("{}:{FLOW}#Pagination", kinds::MODULE),
    );
    assert_eq!(attributes(usage, "alias"), "pager");

    // `data: Order` / `fields: [id, …]` with no domain in the inputs: no
    // edge, and no class node is invented.
    assert_no_edge(
        &graph,
        &component(FLOW, "Dashboard.recentOrders"),
        kinds::DATA,
    );
    assert_no_edge(
        &graph,
        &component(FLOW, "Dashboard.recentOrders"),
        kinds::READS_FEATURE,
    );
    assert!(
        !graph.nodes.iter().any(|node| node.kind == kinds::CLASS),
        "no domain, no class nodes"
    );
}

/// Inline `.mox` `actors` blocks emit through the same policy emitter as a
/// standalone `.actor` file: agents, grants, `never_both`, and a delegation
/// with its purpose and obligations.
#[test]
fn inline_actors_blocks_emit_grants_and_delegations() {
    const DOMAIN: &str = "tests/conformance/models/actors.mox";
    let compilation = rex_driver::compile_str(
        &disk(DOMAIN).display().to_string(),
        &read(DOMAIN),
        &rex_driver::DomainImports::default(),
    );
    assert_clean(DOMAIN, &compilation.diagnostics);
    let mut inputs = GraphInputs::default();
    inputs.domains.push(graph_file(
        DOMAIN,
        compilation.model.expect("actors.mox must lower"),
    ));
    let graph = build(&inputs);

    edge(
        &graph,
        &capability(DOMAIN, "Support.RaiseRefund"),
        kinds::ON,
        &class(DOMAIN, "rex.conformance.actors.Ticket"),
    );
    // `agent Auditor extends Finance` — the actor-kind word does not change
    // the node's identity, the parent link does.
    edge(
        &graph,
        &actor(DOMAIN, "Support.Auditor"),
        kinds::EXTENDS,
        &actor(DOMAIN, "Support.Finance"),
    );
    // `delegation RefundIntake { from Customer to Agent purpose RefundTriage
    //   permit ReadTicket when (!internal) obligation ack }`.
    let delegation = edge(
        &graph,
        &actor(DOMAIN, "Support.Agent"),
        kinds::DELEGATES,
        &capability(DOMAIN, "Support.ReadTicket"),
    );
    assert_eq!(delegation.when.as_deref(), Some("!internal"));
    assert_eq!(delegation.obligations, ["ack"]);
    // `fromActor` is the delegating actor's surface-qualified name.
    assert_eq!(attributes(delegation, "fromActor"), "Support.Customer");
    assert_eq!(attributes(delegation, "delegation"), "RefundIntake");
    assert_eq!(attributes(delegation, "purpose"), "RefundTriage");
    // Both directions of the exclusivity constraint.
    edge(
        &graph,
        &capability(DOMAIN, "Support.RaiseRefund"),
        kinds::EXCLUDES,
        &capability(DOMAIN, "Support.ApproveRefund"),
    );
    edge(
        &graph,
        &capability(DOMAIN, "Support.ApproveRefund"),
        kinds::EXCLUDES,
        &capability(DOMAIN, "Support.RaiseRefund"),
    );
}

// ---------------------------------------------------------------------------
// Queries (issue rexlang#242): the artifact is the product
// ---------------------------------------------------------------------------

/// The product property: the queries answer identically from the in-memory
/// model and from the deserialized artifact. No sources, no recompile — the
/// two indexes see the same bytes and must produce the same envelopes.
#[test]
fn queries_run_on_the_deserialized_artifact() {
    let graph = build(&support_inputs());
    let bytes = graph.to_json().expect("serialize the graph");
    let restored = GraphModel::from_json(&bytes).expect("deserialize the graph");
    let live = GraphIndex::new(&graph);
    let artifact = GraphIndex::new(&restored);

    let reference = capability(ACTOR, "Support.RaiseRefund");
    let pairs = [
        (
            query_node(&live, &reference, true).expect("node on the live model"),
            query_node(&artifact, &reference, true).expect("node on the artifact"),
        ),
        (
            query::list(&live, None, None, None),
            query::list(&artifact, None, None, None),
        ),
        (
            affected_by(&live, &reference, &[], ClosureDirection::Dependents)
                .expect("affected-by on the live model"),
            affected_by(&artifact, &reference, &[], ClosureDirection::Dependents)
                .expect("affected-by on the artifact"),
        ),
        (
            dependencies(&live, &reference).expect("dependencies on the live model"),
            dependencies(&artifact, &reference).expect("dependencies on the artifact"),
        ),
    ];
    for (live_answer, artifact_answer) in pairs {
        assert_eq!(
            live_answer, artifact_answer,
            "the deserialized artifact must answer exactly like the in-memory model"
        );
        assert_eq!(
            live_answer.to_json(false).expect("serialize"),
            artifact_answer.to_json(false).expect("serialize"),
            "byte-identical envelopes across the two models"
        );
    }

    // `list` is the whole node table, sorted by id.
    let listed = query::list(&artifact, None, None, None);
    assert_eq!(listed.results.len(), graph.nodes.len());
    let ids: Vec<&str> = listed.results.iter().map(|node| node.id.as_str()).collect();
    let mut sorted = ids.clone();
    sorted.sort();
    assert_eq!(ids, sorted, "list sorts by id");
}

/// The exit gate: the impact answer for `Support.RaiseRefund` is pinned
/// exactly — the capability, the operation, the service, the `desk` module,
/// the `SupportDesk` design application, the `.ifml` `RaiseRefund` action,
/// the deploy `api` component and the deployments, and the permitting
/// actors.
///
/// The expected set is derived from the committed golden
/// (`examples/artifacts/examples.graph.json`) restricted to the support
/// fixture (the golden additionally carries `cli.ifml`, whose
/// `command:cli.ifml#tickets` hangs off `Support.Agent` through `requires:`;
/// the driver fixture has no `cli.ifml`), then sorted by `(depth, id)`. It is
/// written out literally on purpose: a snapshot of whatever the code prints
/// would pin a bug.
#[test]
fn affected_by_raise_refund_returns_the_support_desk_wiring() {
    let graph = build(&support_inputs());
    let index = GraphIndex::new(&graph);
    let envelope = affected_by(
        &index,
        &capability(ACTOR, "Support.RaiseRefund"),
        &[],
        ClosureDirection::Dependents,
    )
    .expect("the capability resolves");

    let expected = [
        // Depth 0 — the reference itself.
        capability(ACTOR, "Support.RaiseRefund"),
        // Depth 1 — the permitting actor, the excluded capability, the
        // operation carrying the capability, and the use case requiring it.
        actor(ACTOR, "Support.Agent"),
        capability(ACTOR, "Support.ApproveRefund"),
        service_operation(DDD, "SupportDesk.desk.TicketService.raiseRefund"),
        use_case(UC, "RaiseRefund"),
        // Depth 2 — the `.ifml` action bound to the operation, the actors
        // reached through `permits`/`extends`, the case wiring, and the
        // service owning the operation (`memberOf` walks forward).
        action(IFML, "RaiseRefund"),
        actor(ACTOR, "Support.Finance"),
        actor(ACTOR, "Support.Manager"),
        case_role(CMMN, "RefundDesk.triage"),
        plan_item(CMMN, "RefundDesk.AssessRefund"),
        format!("{}:{DDD}#SupportDesk.desk.TicketService", kinds::SERVICE),
        service_operation(DDD, "SupportDesk.desk.TicketService.approveRefund"),
        use_case(UC, "ApproveRefund"),
        // Depth 3 — the excluded operation's action, the case, the `desk`
        // module, the flow the action is declared in, and the refunds view.
        action(IFML, "ApproveRefund"),
        format!("{}:{CMMN}#RefundDesk", kinds::CASE),
        case_role(CMMN, "RefundDesk.escalation"),
        case_role(CMMN, "RefundDesk.finance"),
        component(IFML, "Queue.intake"),
        format!("{}:{IFML}#support", kinds::FLOW),
        module(DDD, "SupportDesk.desk"),
        plan_item(CMMN, "RefundDesk.CloseTicket"),
        plan_item(CMMN, "RefundDesk.ReviewTicket"),
        // Depth 4 — the design application owning the module, the deploy
        // components binding the module and the flow, and the remaining
        // case plan items.
        component(IFML, "Queue.refunds"),
        deploy_component(DEPLOY, "SupportDesk.api"),
        deploy_component(DEPLOY, "SupportDesk.web"),
        format!("{}:{DDD}#SupportDesk", kinds::DESIGN_APPLICATION),
        plan_item(CMMN, "RefundDesk.ApproveRefund"),
        plan_item(CMMN, "RefundDesk.Escalate"),
        // Depth 5 — the deploy application the `api` component belongs to
        // and its two deployments.
        format!("{}:{DEPLOY}#SupportDesk", kinds::DEPLOY_APPLICATION),
        format!("{}:{DEPLOY}#SupportDesk.edgeLive", kinds::DEPLOYMENT),
        format!("{}:{DEPLOY}#SupportDesk.production", kinds::DEPLOYMENT),
    ];
    let actual: Vec<&str> = envelope
        .results
        .iter()
        .map(|node| node.id.as_str())
        .collect();
    assert_eq!(
        actual, expected,
        "the impact answer for Support.RaiseRefund is pinned exactly"
    );
    // Every hit names its discovering edge and parent (except the root).
    for hit in &envelope.results[1..] {
        assert!(hit.via.is_some(), "{:?} discovers no edge", hit.id);
        assert!(hit.parent.is_some(), "{:?} names no parent", hit.id);
    }
    assert_eq!(envelope.results[0].depth, Some(0));
    assert_eq!(envelope.results[0].via, None);

    // The reverse query is not this one: `dependencies` looks forward.
    let forward = dependencies(&index, &capability(ACTOR, "Support.RaiseRefund"))
        .expect("the capability resolves");
    assert!(
        forward
            .results
            .iter()
            .any(|node| node.id == class(MOX, "nz.example.support.Ticket")),
        "the capability's `on` class is a dependency, not a dependent"
    );
}
