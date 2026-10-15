//! Golden-artifact conformance test: compiling the canonical `.uc` use case
//! file against its imported domain, design, actor, and event artifacts must
//! produce an [`rex_ir::usecases::UseCaseModel`] whose pretty JSON
//! byte-matches the committed golden artifact, and the artifact must
//! round-trip through `from_json`. Negative cases pin the driver's
//! validation diagnostics.
//!
//! Regenerate with `REX_UPDATE_FIXTURES=1 cargo test -p rex-driver --test
//! usecase_golden`.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use rex_driver::{compile_actors_str, compile_ddd_str, compile_evt_str, compile_usecase_str};
use rex_ir::usecases::{Anchor, NodeRef, PayloadRef};

fn workspace_root() -> &'static Path {
    static ROOT: OnceLock<PathBuf> = OnceLock::new();
    ROOT.get_or_init(|| {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../")
            .canonicalize()
            .expect("workspace root")
    })
}

fn fixture_path(relative: &str) -> PathBuf {
    workspace_root().join(relative)
}

const UC_PATH: &str = "tests/conformance/usecases/delivery.uc";
const ARTIFACT_PATH: &str = "tests/conformance/usecases/delivery.uc.json";

/// Compiles the conformance use case file, resolving its four imports
/// exactly the way the CLI does (relative to the `.uc` file; the resolved
/// absolute paths are the keys the driver matches the buckets against).
fn compile_conformance() -> rex_driver::UseCaseCompilation {
    let uc_absolute = fixture_path(UC_PATH);
    let source = std::fs::read_to_string(&uc_absolute).expect("read conformance use case file");
    let dir = uc_absolute.parent().expect("use case directory");

    let domain_path = dir.join("delivery.mox");
    let domain_source =
        std::fs::read_to_string(&domain_path).expect("read conformance domain delivery.mox");
    let domain = [(domain_path.display().to_string(), domain_source.clone())];

    let design_source =
        std::fs::read_to_string(dir.join("handover.ddd")).expect("read imported design");
    let design_path = dir.join("handover.ddd").display().to_string();
    let design = compile_ddd_str(
        &design_path,
        &design_source,
        &domain,
        &rex_driver::DomainImports::default(),
    );
    assert!(
        design.diagnostics.is_empty(),
        "imported design must compile clean: {:?}",
        design.diagnostics
    );

    let actors_source =
        std::fs::read_to_string(dir.join("handover.actor")).expect("read imported actors");
    let actors_path = dir.join("handover.actor").display().to_string();
    let actors = compile_actors_str(
        &actors_path,
        &actors_source,
        &domain,
        &rex_driver::DomainImports::default(),
    );
    assert!(
        actors.diagnostics.is_empty(),
        "imported actors must compile clean: {:?}",
        actors.diagnostics
    );

    let events_source =
        std::fs::read_to_string(dir.join("handover.evt")).expect("read imported events");
    let events_path = dir.join("handover.evt").display().to_string();
    let events = compile_evt_str(
        &events_path,
        &events_source,
        &domain,
        &rex_driver::DomainImports::default(),
    );
    assert!(
        events.diagnostics.is_empty(),
        "imported events must compile clean: {:?}",
        events.diagnostics
    );

    compile_usecase_str(
        uc_absolute.to_str().expect("utf-8 path"),
        &source,
        &domain,
        &rex_driver::DomainImports::default(),
        &[(design_path, design.model.expect("design artifact"))],
        &[(actors_path, actors.model.expect("actor artifact"))],
        &[(events_path, events.model.expect("event artifact"))],
    )
}

#[test]
fn conformance_usecase_file_matches_golden_artifact() {
    let compilation = compile_conformance();
    assert!(
        compilation.diagnostics.is_empty(),
        "conformance use case file must compile clean: {:?}",
        compilation.diagnostics
    );
    let model = compilation.model.expect("use case artifact lowered");
    assert_eq!(
        model.format_version,
        rex_ir::usecases::USE_CASE_MODEL_FORMAT_VERSION
    );
    assert_eq!(
        model
            .use_cases
            .iter()
            .map(|use_case| use_case.name.as_str())
            .collect::<Vec<_>>(),
        [
            "HandOverParcel1",
            "HandOverParcel2a",
            "HandOverParcel2b",
            "HandOverParcel3",
            "MonitorHandover"
        ]
    );

    // Iteration 1: the subject keeps its resolved type; the bare actor is a
    // policy principal.
    let first = &model.use_cases[0];
    let subject = first.subject.as_ref().expect("subject");
    assert_eq!(
        subject.type_name.as_deref(),
        Some("nz.example.delivery.DeliveryDrone")
    );
    assert_eq!(first.actors.len(), 1);
    assert_eq!(first.actors[0].name, "Courier");
    assert_eq!(first.actors[0].type_name, None);

    // Iteration 2a: the message anchors at occurrences; its payload is the
    // resolved domain class.
    let second = &model.use_cases[1];
    let message = &second.messages[0];
    assert_eq!(
        message.payload,
        Some(PayloadRef::Domain(rex_ir::TypeRef::Class {
            package: "nz.example.delivery".to_string(),
            name: "HandoverPrompt".to_string(),
        }))
    );
    assert_eq!(
        message.source.anchor,
        Anchor::Occurrence("recipientVerified".to_string())
    );
    assert_eq!(
        message.target.anchor,
        Anchor::Occurrence("promptAcknowledged".to_string())
    );
    assert_eq!(
        second.successions[0].first,
        NodeRef::Start,
        "successions keep source order"
    );

    // Iteration 2b: both ends are message-owned.
    let third = &model.use_cases[2];
    let message = &third.messages[0];
    assert_eq!(message.source.participant, "Drone");
    assert_eq!(
        message.source.anchor,
        Anchor::MessageEnd(rex_ir::usecases::MessageEndSide::Source)
    );
    assert_eq!(
        message.target.anchor,
        Anchor::MessageEnd(rex_ir::usecases::MessageEndSide::Target)
    );

    // Iteration 3: steps carry operations and requirements, the event
    // payload binds, and the realizes binding survives as authored.
    let fourth = &model.use_cases[3];
    let subject = fourth.subject.as_ref().expect("subject");
    assert_eq!(subject.steps[0].operation, "verifyRecipient");
    assert_eq!(subject.steps[1].requires, ["ReleaseParcel"]);
    let messages: Vec<_> = fourth
        .messages
        .iter()
        .map(|message| message.payload.clone())
        .collect();
    assert_eq!(
        messages[1],
        Some(PayloadRef::Event("PromptIssued".to_string())),
        "the event contract payload binds by event name"
    );
    let request = &fourth.messages[0];
    assert_eq!(request.source.participant, "Courier");
    assert_eq!(request.target.anchor, Anchor::Step("verify".to_string()));
    assert_eq!(fourth.successions.len(), 9);
    assert_eq!(
        fourth.realizes,
        ["handover.HandoverService.releaseParcel".to_string()]
    );

    let json = model.to_json_pretty().expect("serialize IR");

    let artifact = fixture_path(ARTIFACT_PATH);
    if std::env::var("REX_UPDATE_FIXTURES").is_ok() {
        std::fs::create_dir_all(artifact.parent().unwrap()).unwrap();
        std::fs::write(&artifact, &json).unwrap();
        eprintln!("updated {}", artifact.display());
        return;
    }
    let expected = std::fs::read_to_string(&artifact).unwrap_or_else(|_| {
        panic!(
            "missing golden artifact {}; regenerate with REX_UPDATE_FIXTURES=1",
            artifact.display()
        )
    });
    assert_eq!(
        json, expected,
        "use case artifact for {UC_PATH} drifted from the golden file"
    );
}

#[test]
fn golden_artifact_round_trips_through_from_json() {
    let expected = std::fs::read_to_string(fixture_path(ARTIFACT_PATH))
        .unwrap_or_else(|error| panic!("read golden artifact {ARTIFACT_PATH}: {error}"));
    let model = rex_ir::usecases::UseCaseModel::from_json(&expected)
        .expect("golden artifact must deserialize");
    assert_eq!(
        model.format_version,
        rex_ir::usecases::USE_CASE_MODEL_FORMAT_VERSION
    );
    let reserialized = model.to_json_pretty().expect("re-serialize IR");
    assert_eq!(reserialized, expected, "artifact round trip must be stable");
}

#[test]
fn fixture_is_fmt_stable() {
    // The CLI `fmt` gate over this fixture is a later task; pin the
    // formatter fixpoint here so that task only has to wire it up.
    let source = std::fs::read_to_string(fixture_path(UC_PATH)).expect("read conformance file");
    let once = rex_syntax::format_usecases(&source).expect("format succeeds");
    assert_eq!(once, source, "the conformance file must be canonical");
    let twice = rex_syntax::format_usecases(&once).expect("format succeeds");
    assert_eq!(twice, once, "formatting must be a fixpoint");
    let parsed = rex_syntax::parse_usecases(&once);
    assert!(
        parsed.errors.is_empty(),
        "formatted output must parse cleanly: {:?}",
        parsed.errors
    );
}

// --- negative cases ------------------------------------------------------------

/// The inline domain: one class with one operation, enough for typed
/// subjects and steps.
const DOMAIN_TEXT: &str =
    "package nz.example.delivery\n\nclass Drone {\n    op boolean signal()\n}\n";

fn domain_pair() -> (String, String) {
    ("domain.mox".to_string(), DOMAIN_TEXT.to_string())
}

/// An `.actor` model over [`DOMAIN_TEXT`] with the given block body.
fn actors_model(body: &str) -> (String, rex_ir::ActorModel) {
    let source = format!("import \"domain.mox\"\n\nactors Policy {{\n{body}\n}}\n");
    let compilation = compile_actors_str(
        "policy.actor",
        &source,
        &[domain_pair()],
        &rex_driver::DomainImports::default(),
    );
    assert!(
        compilation.diagnostics.is_empty(),
        "inline actors must compile clean: {:?}",
        compilation.diagnostics
    );
    (
        "policy.actor".to_string(),
        compilation.model.expect("actor artifact"),
    )
}

/// A `.ddd` design over [`DOMAIN_TEXT`] with one service, for the `realizes`
/// cases.
fn design_pair() -> (String, rex_ir::ddd::DddModel) {
    let source = concat!(
        "import \"domain.mox\";\n",
        "\n",
        "application Delivery {\n",
        "    base nz.example.delivery\n",
        "\n",
        "    module m {\n",
        "        service Svc {\n",
        "            boolean go();\n",
        "        }\n",
        "    }\n",
        "}\n",
    );
    let compilation = compile_ddd_str(
        "design.ddd",
        source,
        &[domain_pair()],
        &rex_driver::DomainImports::default(),
    );
    assert!(
        compilation.diagnostics.is_empty(),
        "inline design must compile clean: {:?}",
        compilation.diagnostics
    );
    (
        "design.ddd".to_string(),
        compilation.model.expect("design artifact"),
    )
}

/// An `.evt` contract over [`DOMAIN_TEXT`] declaring one event, for the
/// payload cases.
fn events_pair(event: &str) -> (String, rex_ir::events::EventModel) {
    let source = format!("import \"domain.mox\"\n\nevent {event} {{\n    code: String;\n}}\n");
    let compilation = compile_evt_str(
        "events.evt",
        &source,
        &[domain_pair()],
        &rex_driver::DomainImports::default(),
    );
    assert!(
        compilation.diagnostics.is_empty(),
        "inline events must compile clean: {:?}",
        compilation.diagnostics
    );
    (
        "events.evt".to_string(),
        compilation.model.expect("event artifact"),
    )
}

/// Compiles an inline `.uc` source against the given buckets.
#[allow(clippy::too_many_arguments)]
fn compile_uc(
    source: &str,
    designs: &[(String, rex_ir::ddd::DddModel)],
    actors: &[(String, rex_ir::ActorModel)],
    events: &[(String, rex_ir::events::EventModel)],
    domains: &[(String, String)],
) -> rex_driver::UseCaseCompilation {
    compile_usecase_str(
        "bad.uc",
        source,
        domains,
        &rex_driver::DomainImports::default(),
        designs,
        actors,
        events,
    )
}

/// Asserts the compilation is blocked and carries exactly one diagnostic on
/// the `.uc` file whose message equals `expected`.
fn expect_diagnostic(compilation: &rex_driver::UseCaseCompilation, expected: &str) {
    assert!(
        compilation.model.is_none(),
        "the negative case must block the artifact"
    );
    let matches: Vec<_> = compilation
        .diagnostics
        .iter()
        .filter(|(path, diagnostic)| path == "bad.uc" && diagnostic.message == expected)
        .collect();
    assert_eq!(
        matches.len(),
        1,
        "expected exactly one {expected:?} diagnostic, got {:?}",
        compilation.diagnostics
    );
}

#[test]
fn unprovided_import_is_an_error() {
    let source = "import \"nowhere.mox\"\n\nusecase U {}\n";
    let compilation = compile_uc(source, &[], &[], &[], &[]);
    expect_diagnostic(
        &compilation,
        "imported file \"nowhere.mox\" was not provided",
    );
}

#[test]
fn alias_on_a_non_design_import_is_an_error() {
    let source = "import \"domain.mox\" as Domain\n\nusecase U {}\n";
    let compilation = compile_uc(source, &[], &[], &[], &[domain_pair()]);
    expect_diagnostic(
        &compilation,
        "import \"domain.mox\" is not a design; only design imports take an alias",
    );
}

#[test]
fn unknown_participant_type_is_an_error() {
    let source = "usecase U {\n    subject D: Mystery\n}\n";
    let compilation = compile_uc(source, &[], &[], &[], &[domain_pair()]);
    expect_diagnostic(&compilation, "unknown type 'Mystery'");
}

#[test]
fn unknown_bare_actor_is_an_error() {
    let source = "usecase U {\n    actor Ghost\n}\n";
    let actors = actors_model("    actor Outsider\n");
    let compilation = compile_uc(source, &[], &[actors], &[], &[domain_pair()]);
    expect_diagnostic(&compilation, "unknown actor 'Ghost' in use case 'U'");

    // With no actor models provided the match is always an error.
    let compilation = compile_uc(source, &[], &[], &[], &[]);
    expect_diagnostic(&compilation, "unknown actor 'Ghost' in use case 'U'");
}

#[test]
fn unknown_step_operation_is_an_error() {
    let source = concat!(
        "import \"domain.mox\"\n",
        "\n",
        "usecase U {\n",
        "    subject D: Drone {\n",
        "        perform ping: nope;\n",
        "    }\n",
        "}\n",
    );
    let compilation = compile_uc(source, &[], &[], &[], &[domain_pair()]);
    expect_diagnostic(
        &compilation,
        "unknown operation 'nope' on class 'Drone' in use case 'U'",
    );
}

#[test]
fn a_step_on_an_actor_is_an_error() {
    let source = concat!(
        "usecase U {\n",
        "    actor A {\n",
        "        perform x;\n",
        "    }\n",
        "}\n",
    );
    let actors = actors_model("    actor A\n");
    let compilation = compile_uc(source, &[], &[actors], &[], &[domain_pair()]);
    expect_diagnostic(
        &compilation,
        "step 'x' on actor 'A' in use case 'U': only the subject performs steps",
    );
}

#[test]
fn an_unbound_message_end_is_an_error() {
    let source = concat!(
        "usecase U {\n",
        "    subject D: Drone\n",
        "    message m;\n",
        "}\n",
    );
    let compilation = compile_uc(source, &[], &[], &[], &[domain_pair()]);
    expect_diagnostic(
        &compilation,
        "unbound source end on message 'm' in use case 'U'",
    );
    expect_diagnostic(
        &compilation,
        "unbound target end on message 'm' in use case 'U'",
    );
}

#[test]
fn a_doubly_bound_message_end_is_an_error() {
    let source = concat!(
        "usecase U {\n",
        "    subject D: Drone {\n",
        "        occurrence x;\n",
        "        occurrence m.source;\n",
        "    }\n",
        "    message m from D.x to D.x;\n",
        "}\n",
    );
    let compilation = compile_uc(source, &[], &[], &[], &[domain_pair()]);
    expect_diagnostic(
        &compilation,
        "source end of message 'm' is bound more than once in use case 'U'",
    );
}

#[test]
fn an_unknown_anchor_is_an_error() {
    let source = concat!(
        "usecase U {\n",
        "    subject D: Drone {\n",
        "        occurrence x;\n",
        "    }\n",
        "    message m from D.ghost to D.x;\n",
        "}\n",
    );
    let compilation = compile_uc(source, &[], &[], &[], &[domain_pair()]);
    expect_diagnostic(
        &compilation,
        "unknown anchor 'D.ghost' for message 'm' in use case 'U'",
    );
}

#[test]
fn an_ambiguous_event_payload_is_an_error() {
    let source = concat!(
        "usecase U {\n",
        "    subject D: Drone\n",
        "    message m of Ping;\n",
        "}\n",
    );
    let first = events_pair("Ping");
    let second_path = "events-2.evt".to_string();
    let second_source = "import \"domain.mox\"\n\nevent Ping {\n    code: String;\n}\n";
    let second = compile_evt_str(
        &second_path,
        second_source,
        &[domain_pair()],
        &rex_driver::DomainImports::default(),
    );
    let second = (second_path, second.model.expect("event artifact"));
    let compilation = compile_uc(source, &[], &[], &[first, second], &[domain_pair()]);
    expect_diagnostic(&compilation, "ambiguous payload 'Ping' in use case 'U'");
}

#[test]
fn an_unresolved_payload_is_an_error() {
    let source = concat!(
        "usecase U {\n",
        "    subject D: Drone\n",
        "    message m of Nope;\n",
        "}\n",
    );
    let compilation = compile_uc(source, &[], &[], &[], &[domain_pair()]);
    expect_diagnostic(&compilation, "unknown type 'Nope'");
}

#[test]
fn an_unresolved_succession_reference_is_an_error() {
    let source = concat!(
        "usecase U {\n",
        "    subject D: Drone\n",
        "    first start then ghost;\n",
        "}\n",
    );
    let compilation = compile_uc(source, &[], &[], &[], &[domain_pair()]);
    expect_diagnostic(&compilation, "unknown node 'ghost' in use case 'U'");
}

#[test]
fn an_ambiguous_bare_occurrence_is_an_error() {
    let source = concat!(
        "usecase U {\n",
        "    subject D: Drone {\n",
        "        occurrence x;\n",
        "    }\n",
        "    actor A: Drone {\n",
        "        occurrence x;\n",
        "    }\n",
        "    first start then x;\n",
        "}\n",
    );
    let compilation = compile_uc(source, &[], &[], &[], &[domain_pair()]);
    expect_diagnostic(
        &compilation,
        "ambiguous occurrence 'x' in use case 'U'; qualify as 'D.x'",
    );
}

#[test]
fn a_succession_cycle_is_an_error() {
    let source = concat!(
        "usecase U {\n",
        "    subject D: Drone\n",
        "    first start then done;\n",
        "    first done then start;\n",
        "}\n",
    );
    let compilation = compile_uc(source, &[], &[], &[], &[domain_pair()]);
    expect_diagnostic(
        &compilation,
        "use case 'U': the successions contain a cycle through 'start'",
    );
}

#[test]
fn an_unreachable_node_is_an_error() {
    let source = concat!(
        "usecase U {\n",
        "    subject D: Drone {\n",
        "        occurrence x;\n",
        "    }\n",
        "    message m from D.x to D.x;\n",
        "    first start then done;\n",
        "}\n",
    );
    let compilation = compile_uc(source, &[], &[], &[], &[domain_pair()]);
    expect_diagnostic(
        &compilation,
        "use case 'U': nodes unreachable from 'start': m.source, m.target",
    );
}

#[test]
fn an_unreachable_done_is_an_error() {
    let source = concat!(
        "usecase U {\n",
        "    subject D: Drone {\n",
        "        occurrence x;\n",
        "    }\n",
        "    first start then D.x;\n",
        "}\n",
    );
    let compilation = compile_uc(source, &[], &[], &[], &[domain_pair()]);
    expect_diagnostic(
        &compilation,
        "use case 'U': 'done' is not reachable from 'start'",
    );
}

#[test]
fn an_unknown_capability_is_an_error() {
    let source = concat!(
        "import \"domain.mox\"\n",
        "\n",
        "usecase U {\n",
        "    subject D: Drone {\n",
        "        perform x: signal requires [ Mystery ];\n",
        "    }\n",
        "}\n",
    );
    let actors = actors_model("    actor Courier\n");
    let compilation = compile_uc(source, &[], &[actors], &[], &[domain_pair()]);
    expect_diagnostic(&compilation, "unknown capability 'Mystery' in use case 'U'");

    // With no actor models provided the existence check always errors.
    let compilation = compile_uc(source, &[], &[], &[], &[domain_pair()]);
    expect_diagnostic(&compilation, "unknown capability 'Mystery' in use case 'U'");
}

#[test]
fn an_uncovered_capability_is_an_error() {
    let source = concat!(
        "import \"domain.mox\"\n",
        "\n",
        "usecase U {\n",
        "    actor Courier\n",
        "    subject D: Drone {\n",
        "        perform x: signal requires [ ReleaseParcel ];\n",
        "    }\n",
        "}\n",
    );
    let actors = actors_model(concat!(
        "    actor Courier\n",
        "    actor Outsider\n",
        "    capability ReleaseParcel on Drone\n",
        "\n",
        "    grant Outsider {\n",
        "        permit ReleaseParcel\n",
        "    }\n",
    ));
    let compilation = compile_uc(source, &[], &[actors], &[], &[domain_pair()]);
    expect_diagnostic(
        &compilation,
        "capability 'ReleaseParcel' is not held by any participating actor in use case 'U'",
    );
}

#[test]
fn extends_alone_does_not_cover_a_capability() {
    // Coverage is structural over grants and delegation hops — an `extends`
    // lineage that would satisfy the `.actor` surface's own rules is
    // deliberately not chased.
    let source = concat!(
        "import \"domain.mox\"\n",
        "\n",
        "usecase U {\n",
        "    actor Helper\n",
        "    subject D: Drone {\n",
        "        perform x: signal requires [ ReleaseParcel ];\n",
        "    }\n",
        "}\n",
    );
    let actors = actors_model(concat!(
        "    actor Courier\n",
        "    agent Helper extends Courier\n",
        "    capability ReleaseParcel on Drone\n",
        "\n",
        "    grant Courier {\n",
        "        permit ReleaseParcel\n",
        "    }\n",
    ));
    let compilation = compile_uc(source, &[], &[actors], &[], &[domain_pair()]);
    expect_diagnostic(
        &compilation,
        "capability 'ReleaseParcel' is not held by any participating actor in use case 'U'",
    );
}

#[test]
fn a_forbid_only_capability_is_an_error() {
    let source = concat!(
        "import \"domain.mox\"\n",
        "\n",
        "usecase U {\n",
        "    actor Courier\n",
        "    subject D: Drone {\n",
        "        perform x: signal requires [ ReleaseParcel ];\n",
        "    }\n",
        "}\n",
    );
    let actors = actors_model(concat!(
        "    actor Courier\n",
        "    capability ReleaseParcel on Drone\n",
        "\n",
        "    grant Courier {\n",
        "        forbid ReleaseParcel\n",
        "    }\n",
    ));
    let compilation = compile_uc(source, &[], &[actors], &[], &[domain_pair()]);
    expect_diagnostic(
        &compilation,
        "capability 'ReleaseParcel' only forbidden for participating actors in use case 'U'",
    );
}

#[test]
fn a_never_both_violation_is_an_error() {
    let source = concat!(
        "import \"domain.mox\"\n",
        "\n",
        "usecase U {\n",
        "    actor Courier\n",
        "    actor Auditor\n",
        "    subject D: Drone {\n",
        "        perform x: signal requires [ Alpha Beta ];\n",
        "    }\n",
        "}\n",
    );
    let actors = actors_model(concat!(
        "    actor Courier\n",
        "    actor Auditor\n",
        "    capability Alpha on Drone\n",
        "    capability Beta on Drone\n",
        "\n",
        "    grant Courier {\n",
        "        permit Alpha\n",
        "    }\n",
        "\n",
        "    grant Auditor {\n",
        "        permit Beta\n",
        "    }\n",
        "\n",
        "    never_both { Alpha, Beta }\n",
    ));
    let compilation = compile_uc(source, &[], &[actors], &[], &[domain_pair()]);
    expect_diagnostic(
        &compilation,
        "capabilities 'Alpha' and 'Beta' are never_both but use case 'U' requires both",
    );
}

#[test]
fn realizes_unknown_alias_is_an_error() {
    let source = concat!(
        "import \"design.ddd\" as d\n",
        "\n",
        "usecase U {\n",
        "    realizes nowhere.Svc.go;\n",
        "}\n",
    );
    let design = design_pair();
    let compilation = compile_uc(source, &[design], &[], &[], &[domain_pair()]);
    expect_diagnostic(
        &compilation,
        "unknown design alias 'nowhere' in use case 'U'",
    );
}

#[test]
fn realizes_unknown_service_is_an_error() {
    let source = concat!(
        "import \"design.ddd\" as d\n",
        "\n",
        "usecase U {\n",
        "    realizes d.Nope.go;\n",
        "}\n",
    );
    let design = design_pair();
    let compilation = compile_uc(source, &[design], &[], &[], &[domain_pair()]);
    expect_diagnostic(
        &compilation,
        "unknown service 'Nope' in design 'd' (use case 'U')",
    );
}

#[test]
fn realizes_unknown_operation_is_an_error() {
    let source = concat!(
        "import \"design.ddd\" as d\n",
        "\n",
        "usecase U {\n",
        "    realizes d.Svc.nope;\n",
        "}\n",
    );
    let design = design_pair();
    let compilation = compile_uc(source, &[design], &[], &[], &[domain_pair()]);
    expect_diagnostic(
        &compilation,
        "unknown operation 'nope' on service 'Svc' in design 'd' (use case 'U')",
    );
}

// --- positive cross-checks -------------------------------------------------------

#[test]
fn coverage_via_direct_grant_compiles_clean() {
    let source = concat!(
        "import \"domain.mox\"\n",
        "\n",
        "usecase U {\n",
        "    actor Courier\n",
        "    subject D: Drone {\n",
        "        perform x: signal requires [ ReleaseParcel ];\n",
        "    }\n",
        "}\n",
    );
    let actors = actors_model(concat!(
        "    actor Courier\n",
        "    capability ReleaseParcel on Drone\n",
        "\n",
        "    grant Courier {\n",
        "        permit ReleaseParcel\n",
        "    }\n",
    ));
    let compilation = compile_uc(source, &[], &[actors], &[], &[domain_pair()]);
    assert!(
        compilation.diagnostics.is_empty(),
        "the direct grant must cover the requirement: {:?}",
        compilation.diagnostics
    );
    assert!(compilation.model.is_some());
}

#[test]
fn coverage_via_delegation_compiles_clean() {
    let source = concat!(
        "import \"domain.mox\"\n",
        "\n",
        "usecase U {\n",
        "    actor RecipientAgent\n",
        "    subject D: Drone {\n",
        "        perform x: signal requires [ ReleaseParcel ];\n",
        "    }\n",
        "}\n",
    );
    let actors = actors_model(concat!(
        "    actor Courier\n",
        "    agent RecipientAgent extends Courier\n",
        "    capability ReleaseParcel on Drone\n",
        "\n",
        "    grant Courier {\n",
        "        permit ReleaseParcel\n",
        "    }\n",
        "\n",
        "    delegation HandoverAssist {\n",
        "        from Courier\n",
        "        to RecipientAgent\n",
        "        permit ReleaseParcel\n",
        "    }\n",
    ));
    let compilation = compile_uc(source, &[], &[actors], &[], &[domain_pair()]);
    assert!(
        compilation.diagnostics.is_empty(),
        "the delegation to the participating actor must cover the requirement: {:?}",
        compilation.diagnostics
    );
    assert!(compilation.model.is_some());
}

#[test]
fn an_event_payload_binds_by_event_name() {
    let source = concat!(
        "import \"domain.mox\"\n",
        "\n",
        "usecase U {\n",
        "    subject D: Drone {\n",
        "        occurrence x;\n",
        "    }\n",
        "    message m of Ping from D.x to D.x;\n",
        "    first start then m.source;\n",
        "    first m.source then m;\n",
        "    first m then done;\n",
        "}\n",
    );
    let events = events_pair("Ping");
    let compilation = compile_uc(source, &[], &[], &[events], &[domain_pair()]);
    assert!(
        compilation.diagnostics.is_empty(),
        "the event payload must bind: {:?}",
        compilation.diagnostics
    );
    let model = compilation.model.expect("artifact");
    assert_eq!(
        model.use_cases[0].messages[0].payload,
        Some(PayloadRef::Event("Ping".to_string()))
    );
}
