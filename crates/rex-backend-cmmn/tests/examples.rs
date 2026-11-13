//! The authoring → engine pipeline, exercised from `examples/`: the
//! canonical `support.cmmn` example compiles the way the CLI compiles it
//! (four-bucket imports resolved from disk), and the cmmn-rs backend
//! lowers it to the engine's `CaseDefinition` JSON — byte-pinned against
//! `examples/artifacts/support.case.json`.
//!
//! Regenerate the golden with
//! `REX_UPDATE_FIXTURES=1 cargo test -p rex-backend-cmmn --test examples`.

use rex_driver::{
    compile_actors_str, compile_cmmn_str, compile_ddd_str, compile_dmn_str, compile_evt_str,
};

#[test]
fn support_cmmn_lowers_to_the_engine_definition() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let dir = root.join("examples");
    let read = |name: &str| std::fs::read_to_string(dir.join(name)).expect(name);

    let domain = [("support.mox".to_string(), read("support.mox"))];

    let events_source = read("support.evt");
    let events_path = dir.join("support.evt").display().to_string();
    let events = compile_evt_str(
        &events_path,
        &events_source,
        &domain,
        &rex_driver::DomainImports::default(),
    );
    let actors_source = read("support.actor");
    let actors_path = dir.join("support.actor").display().to_string();
    let actors = compile_actors_str(
        &actors_path,
        &actors_source,
        &domain,
        &rex_driver::DomainImports::default(),
    );
    let design_source = read("support.ddd");
    let design_path = dir.join("support.ddd").display().to_string();
    let design = compile_ddd_str(
        &design_path,
        &design_source,
        &domain,
        &rex_driver::DomainImports::default(),
    );
    for (what, diagnostics) in [
        ("support.evt", &events.diagnostics),
        ("support.actor", &actors.diagnostics),
        ("support.ddd", &design.diagnostics),
    ] {
        assert!(
            diagnostics.is_empty(),
            "{what} must compile clean: {:?}",
            diagnostics
        );
    }

    // The sibling `.dmn` decision model, compiled the same way — the
    // decision dimension a `decision` task's `invokes` clause resolves
    // against.
    let decisions_source = read("support.dmn");
    let decisions_path = dir.join("support.dmn").display().to_string();
    // The example's `.evt` import resolves next to the file and joins
    // the events bucket (the emits rule joins it the same way).
    let events_path = dir.join("support.evt").display().to_string();
    let events = compile_evt_str(
        &events_path,
        &read("support.evt"),
        &domain,
        &rex_driver::DomainImports::default(),
    );
    assert!(
        events.diagnostics.is_empty(),
        "support.evt must compile clean"
    );
    // The DMN rule-1 import check matches the bucket keys exactly as
    // written — the authored import string, not the resolved path.
    let event_bucket = vec![(
        "support.evt".to_string(),
        events.model.clone().expect("event artifact"),
    )];
    let decisions = compile_dmn_str(
        &decisions_path,
        &decisions_source,
        &domain,
        &rex_driver::DomainImports::default(),
        &[(
            "./tables/refund-authority.md".to_string(),
            read("tables/refund-authority.md"),
        )],
        &event_bucket,
    );
    assert!(
        decisions.diagnostics.is_empty(),
        "support.dmn must compile clean: {:?}",
        decisions
            .diagnostics
            .iter()
            .map(|(_, d)| &d.message)
            .collect::<Vec<_>>()
    );

    let case_path = dir.join("support.cmmn").display().to_string();
    let compilation = compile_cmmn_str(
        &case_path,
        &read("support.cmmn"),
        &domain,
        &rex_driver::DomainImports::default(),
        &[(design_path, design.model.expect("design artifact"))],
        &[(actors_path, actors.model.expect("actor artifact"))],
        &[(events_path, events.model.expect("event artifact"))],
        &[(decisions_path, decisions.model.expect("decision artifact"))],
    );
    assert!(
        compilation.diagnostics.is_empty(),
        "support.cmmn must compile clean: {:?}",
        compilation
            .diagnostics
            .iter()
            .map(|(_, diagnostic)| &diagnostic.message)
            .collect::<Vec<_>>()
    );
    let model = compilation.model.expect("case artifact");

    let definitions =
        rex_backend_cmmn::generate(&model, compilation.domains_model.as_ref().expect("domains"))
            .expect("the engine definition generates");
    assert_eq!(definitions.len(), 1);
    assert_eq!(definitions[0].case, "RefundDesk");

    let golden_path = root.join("examples/artifacts/support.case.json");
    if std::env::var("REX_UPDATE_FIXTURES").is_ok() {
        std::fs::write(&golden_path, &definitions[0].json).unwrap();
        eprintln!("updated {}", golden_path.display());
        return;
    }
    let expected = std::fs::read_to_string(&golden_path).expect("golden case definition");
    assert_eq!(definitions[0].json, expected);
}
