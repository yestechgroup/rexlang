//! In-memory conformance tests for [`rex_driver::project`] — the shared
//! host-side project resolver (Phase B of #117). Every test drives the
//! module through pure in-memory fetch closures: no disk, no files. The
//! fetch contract is the unit under test as much as the gathering itself —
//! `(importer, import) -> Option<(resolved key, text)>` for the `.ifml`
//! walk, `path -> Option<String>` for the path-keyed gatherers.

use std::collections::BTreeMap;

use rex_driver::project::{
    compile_actor_imports, compile_decision_imports, compile_domain_union, gather_deploy_imports,
    gather_domain_sources, gather_ifml_project, gather_usecase_imports, ActorImport,
    DecisionImport,
};

const WIDGET_MOX: &str = "package test.widgets\n\nclass Widget {\n    id readonly String widgetId\n    String label\n}\n";

const GADGET_MOX: &str = "package test.gadgets\n\nclass Gadget {\n    id readonly String gadgetId\n    String label\n}\n";

const OPS_ACTOR: &str = "import \"widgets.mox\"\n\nactors Ops {\n    actor Admin\n\n    capability Touch on Widget\n\n    grant Admin {\n        permit Touch\n    }\n}\n";

const BROKEN_ACTOR: &str = "import \"widgets.mox\"\n\nactors Ops {\n    actor Admin\n\n    capability Touch on Nope\n\n    grant Admin {\n        permit Touch\n    }\n}\n";

const DESIGN_DDD: &str = "import \"widgets.mox\";\n\napplication App {\n    base test.widgets\n    module core {\n        service WidgetService {\n            inject WidgetRepository;\n            Widget find(String widgetId);\n            findOne => WidgetRepository.findById;\n        }\n        entity Widget scaffold repository WidgetRepository {\n            findById;\n        }\n    }\n}\n";

const OPS_EVT: &str = "import \"widgets.mox\"\n\nevent WidgetMade version \"1.0.0\" {\n    widgetId: String;\n}\n\nchannel ops {\n    publishes WidgetMade;\n}\n";

const GUI_IFML: &str = "import \"pager.ifml\";\n\nmodule \"Pager\" {\n    input { page: Int }\n    output { total: Int }\n}\n";

const PAGER_IFML: &str = "view \"PagerPage\" {}\n";

const WIDGET_DMN: &str = "import \"widgets.mox\"\n\ndecision_model WidgetDecisions {\n    namespace test.decisions\n    inputs {\n        widget: Widget;\n    }\n    decisions {\n        decision WidgetVerdict {\n            outputs {\n                verdict: string;\n            }\n            logic table \"./tables/verdict.md\"\n        }\n    }\n    services {\n        service AssessWidget {\n            inputs { widget }\n            outputs { WidgetVerdict }\n        }\n    }\n}\n";

const VERDICT_TABLE: &str =
    "> # Widget verdict\n\n| U | verdict |\n|:-:|:-------:|\n|   |   `o`   |\n| 1 | \"ok\"   |\n";

/// The in-memory two-key fetch: `(importer, import) -> (resolved key,
/// text)`, exactly the contract [`rex_ifml::walk_ifml_imports`] and
/// [`gather_ifml_project`] impose on hosts.
fn pair_fetch(
    pairs: BTreeMap<(&'static str, &'static str), (&'static str, &'static str)>,
) -> impl FnMut(&str, &str) -> Option<(String, String)> {
    move |importer: &str, import: &str| {
        pairs
            .get(&(importer, import))
            .map(|(key, text)| ((*key).to_string(), (*text).to_string()))
    }
}

/// The in-memory path fetch: `resolved path -> text`, the contract the
/// path-keyed gatherers impose.
fn path_fetch(paths: BTreeMap<&'static str, &'static str>) -> impl FnMut(&str) -> Option<String> {
    move |key: &str| paths.get(key).cloned().map(str::to_string)
}

#[test]
fn gather_ifml_project_classifies_by_extension_and_dedups_by_resolved_key() {
    let entry = "import \"gui.ifml\";\nimport \"widgets.mox\";\nimport \"./widgets.mox\";\nimport \"ops.actor\";\nimport \"missing.mox\";\nimport \"missing.actor\";\nimport \"notes.txt\";\n";
    let mut host = pair_fetch(BTreeMap::from([
        (("app.ifml", "gui.ifml"), ("lib/gui.ifml", GUI_IFML)),
        (("gui.ifml", "pager.ifml"), ("lib/pager.ifml", PAGER_IFML)),
        (("app.ifml", "widgets.mox"), ("lib/widgets.mox", WIDGET_MOX)),
        (
            ("app.ifml", "./widgets.mox"),
            ("lib/widgets.mox", WIDGET_MOX),
        ),
        (("app.ifml", "ops.actor"), ("lib/ops.actor", OPS_ACTOR)),
        (
            ("lib/ops.actor", "widgets.mox"),
            ("lib/widgets.mox", WIDGET_MOX),
        ),
    ]));
    let project = gather_ifml_project("app.ifml", entry, &mut host);

    // Bundle keys are the as-written identities, nested walk included.
    assert_eq!(
        project.imports.get("app.ifml", "gui.ifml"),
        Some(GUI_IFML),
        "the .ifml import is provided under the walker's own key"
    );
    assert_eq!(
        project.imports.get("gui.ifml", "pager.ifml"),
        Some(PAGER_IFML),
        "nested .ifml imports provide under their as-written importer"
    );
    let identities: Vec<&str> = project.sources.iter().map(|(id, _)| id.as_str()).collect();
    assert_eq!(identities, vec!["app.ifml", "gui.ifml", "pager.ifml"]);

    // Domains dedup by resolved key across two spellings of one import,
    // in walk order; the walker is not sent into them.
    assert_eq!(
        project.domains,
        vec![("lib/widgets.mox".to_string(), WIDGET_MOX.to_string())]
    );

    // The actor policy carries its own gathered `.mox` imports.
    assert_eq!(project.actors.len(), 1);
    assert_eq!(project.actors[0].key, "lib/ops.actor");
    assert_eq!(project.actors[0].source, OPS_ACTOR);
    assert_eq!(
        project.actors[0].domains,
        vec![("lib/widgets.mox".to_string(), WIDGET_MOX.to_string())]
    );

    // Missing `.mox`/`.actor` imports land in `unresolved` under the
    // importer's identity; unknown extensions are ignored entirely.
    assert_eq!(
        project.unresolved,
        vec![
            ("app.ifml".to_string(), "missing.mox".to_string()),
            ("app.ifml".to_string(), "missing.actor".to_string()),
        ]
    );
}

#[test]
fn gather_ifml_project_classifies_decision_imports_with_their_domains_and_tables() {
    let entry = "import \"verdicts.dmn\";\nimport \"missing.dmn\";\nimport \"notes.txt\";\n";
    let mut host = pair_fetch(BTreeMap::from([
        (
            ("app.ifml", "verdicts.dmn"),
            ("lib/verdicts.dmn", WIDGET_DMN),
        ),
        (
            ("lib/verdicts.dmn", "widgets.mox"),
            ("lib/widgets.mox", WIDGET_MOX),
        ),
        (
            ("lib/verdicts.dmn", "./tables/verdict.md"),
            ("lib/tables/verdict.md", VERDICT_TABLE),
        ),
    ]));
    let project = gather_ifml_project("app.ifml", entry, &mut host);

    // The decision model carries its own gathered `.mox` imports and
    // decision-table contents, keyed as written; the walker is not sent
    // into the `.dmn` (sources stay `.ifml`-only).
    let identities: Vec<&str> = project.sources.iter().map(|(id, _)| id.as_str()).collect();
    assert_eq!(identities, vec!["app.ifml"]);
    assert_eq!(project.decisions.len(), 1);
    assert_eq!(project.decisions[0].key, "lib/verdicts.dmn");
    assert_eq!(project.decisions[0].source, WIDGET_DMN);
    assert_eq!(
        project.decisions[0].domains,
        vec![("lib/widgets.mox".to_string(), WIDGET_MOX.to_string())]
    );
    assert_eq!(
        project.decisions[0].tables,
        vec![("./tables/verdict.md".to_string(), VERDICT_TABLE.to_string())]
    );

    // A missing `.dmn` import is unresolved under the importer's
    // identity; unknown extensions stay ignored entirely.
    assert_eq!(
        project.unresolved,
        vec![("app.ifml".to_string(), "missing.dmn".to_string())]
    );
}

#[test]
fn compile_decision_imports_compiles_models_against_their_domains_and_tables() {
    let decisions = vec![DecisionImport {
        key: "lib/verdicts.dmn".to_string(),
        source: WIDGET_DMN.to_string(),
        domains: vec![("lib/widgets.mox".to_string(), WIDGET_MOX.to_string())],
        tables: vec![("./tables/verdict.md".to_string(), VERDICT_TABLE.to_string())],
        events: Vec::new(),
    }];
    let models = compile_decision_imports(&decisions).expect("model compiles against its inputs");
    assert_eq!(models.len(), 1);
    assert_eq!(models[0].services.len(), 1);
    assert_eq!(models[0].services[0].name, "AssessWidget");
}

#[test]
fn compile_decision_imports_reports_a_non_compiling_model() {
    let decisions = vec![DecisionImport {
        key: "lib/verdicts.dmn".to_string(),
        source: WIDGET_DMN.to_string(),
        // The model's types resolve against its own gathered domains: an
        // empty slice leaves `Widget` unknown — the hard error names the
        // import key.
        domains: Vec::new(),
        tables: vec![("./tables/verdict.md".to_string(), VERDICT_TABLE.to_string())],
        events: Vec::new(),
    }];
    let error = compile_decision_imports(&decisions).unwrap_err();
    assert!(
        error.contains("imported decision model lib/verdicts.dmn does not compile"),
        "unexpected error: {error}"
    );
}

#[test]
fn compile_actor_imports_compiles_policies_against_their_gathered_domains() {
    let actors = vec![ActorImport {
        key: "lib/ops.actor".to_string(),
        source: OPS_ACTOR.to_string(),
        domains: vec![("lib/widgets.mox".to_string(), WIDGET_MOX.to_string())],
    }];
    let models = compile_actor_imports(&actors).expect("policy compiles against its domains");
    assert_eq!(models.len(), 1);
}

#[test]
fn compile_actor_imports_reports_a_non_compiling_policy() {
    let actors = vec![ActorImport {
        key: "lib/ops.actor".to_string(),
        source: BROKEN_ACTOR.to_string(),
        domains: vec![("lib/widgets.mox".to_string(), WIDGET_MOX.to_string())],
    }];
    let error = compile_actor_imports(&actors).unwrap_err();
    assert!(
        error.contains("imported actor model lib/ops.actor does not compile"),
        "unexpected error: {error}"
    );
}

#[test]
fn compile_domain_union_unions_packages_and_reports_diagnostics() {
    let union = compile_domain_union(&[
        ("lib/widgets.mox".to_string(), WIDGET_MOX.to_string()),
        ("lib/gadgets.mox".to_string(), GADGET_MOX.to_string()),
    ])
    .expect("both domains compile");
    let names: Vec<&str> = union
        .packages
        .iter()
        .map(|package| package.name.as_str())
        .collect();
    assert_eq!(names, vec!["test.widgets", "test.gadgets"]);

    // The empty union is valid — hosts decide what empty means.
    assert!(compile_domain_union(&[]).unwrap().packages.is_empty());

    let broken = "package test.bad\n\nclass {";
    let error =
        compile_domain_union(&[("lib/bad.mox".to_string(), broken.to_string())]).unwrap_err();
    assert!(
        error.starts_with("imported domain lib/bad.mox does not compile"),
        "unexpected error: {error}"
    );
}

#[test]
fn gather_domain_sources_dedups_by_resolved_key_and_skips_missing() {
    let source = "import \"widgets.mox\"\nimport \"./widgets.mox\"\nimport \"missing.mox\"\n";
    let imports = |_: &str| {
        vec![
            "widgets.mox".to_string(),
            "./widgets.mox".to_string(),
            "missing.mox".to_string(),
        ]
    };
    let mut host = path_fetch(BTreeMap::from([("lib/widgets.mox", WIDGET_MOX)]));
    let domains = gather_domain_sources("lib/app.mox", source, imports, &mut host);
    assert_eq!(
        domains,
        vec![("lib/widgets.mox".to_string(), WIDGET_MOX.to_string())],
        "two spellings of one import dedup by resolved key; the missing one skips silently"
    );
}

#[test]
fn gather_usecase_imports_buckets_all_four_kinds() {
    let mut host = path_fetch(BTreeMap::from([
        ("lib/widgets.mox", WIDGET_MOX),
        ("lib/app.ddd", DESIGN_DDD),
        ("lib/ops.actor", OPS_ACTOR),
        ("lib/ops.evt", OPS_EVT),
    ]));
    let imports = |_: &str| {
        vec![
            "widgets.mox".to_string(),
            "app.ddd".to_string(),
            "ops.actor".to_string(),
            "ops.evt".to_string(),
        ]
    };
    let buckets = gather_usecase_imports("lib/app.uc", "// uc", imports, &mut host)
        .expect("every import gathers and compiles");
    assert_eq!(
        buckets.domains,
        vec![("lib/widgets.mox".to_string(), WIDGET_MOX.to_string())]
    );
    assert_eq!(buckets.designs.len(), 1);
    assert_eq!(buckets.designs[0].0, "lib/app.ddd");
    assert_eq!(buckets.actors.len(), 1);
    assert_eq!(buckets.actors[0].0, "lib/ops.actor");
    assert_eq!(buckets.events.len(), 1);
    assert_eq!(buckets.events[0].0, "lib/ops.evt");
}

#[test]
fn gather_usecase_imports_errors_on_a_missing_import() {
    let imports = |_: &str| vec!["missing.mox".to_string()];
    let mut host = path_fetch(BTreeMap::new());
    let error = gather_usecase_imports("lib/app.uc", "// uc", imports, &mut host).unwrap_err();
    assert_eq!(
        error,
        "cannot read imported file lib/missing.mox (imported by lib/app.uc)"
    );
}

/// Depth-≥2 flow nesting: `a.ifml` imports `sub/b.ifml`, which imports
/// `c.ifml`, whose module `a.ifml` uses. The walker recurses under
/// as-written import strings, so the deploy gather must map identities to
/// resolved keys — a bridge that treats every importer as a disk path
/// resolves `c.ifml` against the process CWD and the flow fails to
/// compile.
#[test]
fn gather_deploy_imports_resolves_nested_flow_imports_relative_to_their_importer() {
    const NESTED_C_IFML: &str =
        "module \"Widget\" { input { count: Int } output { total: Int } }\n";
    const NESTED_B_IFML: &str = "import \"c.ifml\";\n";
    const NESTED_A_IFML: &str =
        "import \"sub/b.ifml\";\n\nview \"List\" { use \"Widget\" as widget { count: 1; } }\n";
    let mut host = path_fetch(BTreeMap::from([
        ("lib/a.ifml", NESTED_A_IFML),
        ("lib/sub/b.ifml", NESTED_B_IFML),
        ("lib/sub/c.ifml", NESTED_C_IFML),
    ]));
    let imports = |_: &str| vec!["a.ifml".to_string()];
    let pairs = gather_deploy_imports("lib/app.deploy", "// deploy", imports, &mut host)
        .expect("the nested flow closure gathers and compiles");
    assert_eq!(pairs.flows.len(), 1);
    assert_eq!(pairs.flows[0].0, "lib/a.ifml");
}

#[test]
fn gather_deploy_imports_compiles_design_and_flow_pairs() {
    let mut host = path_fetch(BTreeMap::from([
        ("lib/widgets.mox", WIDGET_MOX),
        ("lib/app.ddd", DESIGN_DDD),
        ("lib/gui.ifml", GUI_IFML),
        ("lib/pager.ifml", PAGER_IFML),
    ]));
    let imports = |_: &str| vec!["app.ddd".to_string(), "gui.ifml".to_string()];
    let pairs = gather_deploy_imports("lib/app.deploy", "// deploy", imports, &mut host)
        .expect("both pairs gather and compile");
    assert_eq!(pairs.designs.len(), 1);
    assert_eq!(pairs.designs[0].0, "lib/app.ddd");
    assert_eq!(pairs.flows.len(), 1);
    assert_eq!(pairs.flows[0].0, "lib/gui.ifml");
}

#[test]
fn gather_deploy_imports_errors_on_a_missing_flow() {
    let mut host = path_fetch(BTreeMap::from([
        ("lib/widgets.mox", WIDGET_MOX),
        ("lib/app.ddd", DESIGN_DDD),
    ]));
    let imports = |_: &str| vec!["app.ddd".to_string(), "gui.ifml".to_string()];
    let error =
        gather_deploy_imports("lib/app.deploy", "// deploy", imports, &mut host).unwrap_err();
    assert!(
        error.contains("cannot read imported file lib/gui.ifml"),
        "unexpected error: {error}"
    );
}
