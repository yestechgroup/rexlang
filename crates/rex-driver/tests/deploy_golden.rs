//! Golden-artifact conformance test: compiling the canonical `.deploy`
//! deployment file against its imported `.ddd` design and `.ifml` flow must
//! produce an [`rex_ir::deploy::DeployModel`] whose pretty JSON byte-matches
//! the committed golden artifact, and the artifact must round-trip through
//! `from_json`. Negative cases pin the driver's validation diagnostics.
//!
//! Regenerate with `REX_UPDATE_FIXTURES=1 cargo test -p rex-driver --test
//! deploy_golden`.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use rex_driver::compile_deploy_str;

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

const DEPLOY_PATH: &str = "tests/conformance/deploy/svc.deploy";
const ARTIFACT_PATH: &str = "tests/conformance/deploy/svc.deploy.json";

/// Compiles the conformance deployment file, resolving its imports exactly
/// the way the CLI does (relative to the deploy file; the resolved absolute
/// paths are the keys the driver matches bindings against).
fn compile_conformance() -> rex_driver::DeployCompilation {
    let deploy_absolute = fixture_path(DEPLOY_PATH);
    let source =
        std::fs::read_to_string(&deploy_absolute).expect("read conformance deployment file");
    let dir = deploy_absolute.parent().expect("deploy directory");
    let designs: Vec<(String, rex_ir::ddd::DddModel)> =
        [("../ddd/library.ddd", "../ddd/library.mox")]
            .iter()
            .map(|(import, domain)| {
                let design_source =
                    std::fs::read_to_string(dir.join(import)).expect("read imported design");
                let domain_source =
                    std::fs::read_to_string(dir.join(domain)).expect("read design domain");
                let design_path = dir.join(import).display().to_string();
                let compilation = rex_driver::compile_ddd_str(
                    &design_path,
                    &design_source,
                    &[(dir.join(domain).display().to_string(), domain_source)],
                    &rex_driver::DomainImports::default(),
                );
                assert!(
                    compilation.diagnostics.is_empty(),
                    "imported design must compile clean: {:?}",
                    compilation.diagnostics
                );
                (design_path, compilation.model.expect("design artifact"))
            })
            .collect();
    let flows: Vec<(String, rex_ir::ifml::IfmlModel)> = ["../ifml/app.ifml"]
        .iter()
        .map(|import| {
            let flow_path = dir.join(import).display().to_string();
            let flow_source =
                std::fs::read_to_string(dir.join(import)).expect("read imported flow");
            let compilation = rex_ifml::compile_ifml_str(
                &flow_path,
                &flow_source,
                &rex_ifml::IfmlImports::default(),
            )
            .expect("imported flow must compile clean");
            (flow_path, compilation.model)
        })
        .collect();

    compile_deploy_str(
        deploy_absolute.to_str().expect("utf-8 path"),
        &source,
        &designs,
        &flows,
    )
}

#[test]
fn conformance_deploy_file_matches_golden_artifact() {
    let compilation = compile_conformance();
    assert!(
        compilation.diagnostics.is_empty(),
        "conformance deployment must compile clean: {:?}",
        compilation.diagnostics
    );
    let model = compilation.model.expect("deployment artifact lowered");
    assert_eq!(
        model.format_version,
        rex_ir::deploy::DEPLOY_MODEL_FORMAT_VERSION
    );
    assert_eq!(model.applications.len(), 1);
    assert_eq!(model.profiles.len(), 2);
    assert_eq!(model.deployments.len(), 2);
    let application = &model.applications[0];
    assert_eq!(application.name, "Svc");
    assert_eq!(application.components.len(), 5);
    assert_eq!(application.connections.len(), 2);
    // The strong design/flow bindings survived lowering.
    assert_eq!(
        application.components[1].designs[0].module.as_deref(),
        Some("media")
    );
    assert_eq!(
        application.components[2].flows[0].module.as_deref(),
        Some("Pagination")
    );
    // Policies carry their verbatim expression text.
    assert_eq!(
        model.profiles[0].policies[0].expression,
        "api.replicas >= 3"
    );
    assert!(model.profiles[0].policies[1].prohibit);

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
        "deployment artifact for {DEPLOY_PATH} drifted from the golden file"
    );
}

#[test]
fn golden_artifact_round_trips_through_from_json() {
    let expected = std::fs::read_to_string(fixture_path(ARTIFACT_PATH))
        .unwrap_or_else(|error| panic!("read golden artifact {ARTIFACT_PATH}: {error}"));
    let model = rex_ir::deploy::DeployModel::from_json(&expected)
        .expect("golden artifact must deserialize");
    assert_eq!(
        model.format_version,
        rex_ir::deploy::DEPLOY_MODEL_FORMAT_VERSION
    );
    let reserialized = model.to_json_pretty().expect("re-serialize IR");
    assert_eq!(reserialized, expected, "artifact round trip must be stable");
}

// --- negative cases ------------------------------------------------------------

/// The imported design's IR, for the binding tests.
fn library_design() -> (String, rex_ir::ddd::DddModel) {
    let dir = fixture_path("tests/conformance/ddd");
    let source = std::fs::read_to_string(dir.join("library.ddd")).expect("read library.ddd");
    let domain = std::fs::read_to_string(dir.join("library.mox")).expect("read library.mox");
    // Both paths are the plain import strings: the design's own import
    // resolves lexically against them, and the deploy sources bind
    // `"library.ddd"` exactly.
    let compilation = rex_driver::compile_ddd_str(
        "library.ddd",
        &source,
        &[("library.mox".to_string(), domain)],
        &rex_driver::DomainImports::default(),
    );
    (
        "library.ddd".to_string(),
        compilation.model.expect("design artifact"),
    )
}

fn compile_with(
    designs: &[(String, rex_ir::ddd::DddModel)],
    source: &str,
) -> rex_driver::DeployCompilation {
    compile_deploy_str("bad.deploy", source, designs, &[])
}

#[test]
fn missing_import_is_an_error() {
    let source = concat!(
        "import \"nowhere.ifml\"\n",
        "\n",
        "application Svc {\n",
        "    component web: frontend {\n",
        "        flow \"nowhere.ifml\"\n",
        "    }\n",
        "}\n",
    );
    let compilation = compile_with(&[], source);
    assert!(compilation.model.is_none());
    let (_, diagnostic) = compilation
        .diagnostics
        .iter()
        .find(|(_, diagnostic)| diagnostic.message.contains("nowhere.ifml"))
        .expect("an unprovided-import diagnostic");
    assert_eq!(
        diagnostic.message,
        "imported file \"nowhere.ifml\" was not provided"
    );
}

#[test]
fn binding_to_unknown_module_is_an_error() {
    let (path, design) = library_design();
    let source = concat!(
        "import \"library.ddd\"\n",
        "\n",
        "application Svc {\n",
        "    component lending: api {\n",
        "        design \"library.ddd#Nope\"\n",
        "    }\n",
        "}\n",
    );
    let compilation = compile_with(&[(path, design)], source);
    assert!(compilation.model.is_none());
    let (_, diagnostic) = compilation
        .diagnostics
        .iter()
        .find(|(_, diagnostic)| diagnostic.message.contains("Nope"))
        .expect("an unknown-module diagnostic");
    assert_eq!(
        diagnostic.message,
        "design \"library.ddd\" declares no module 'Nope' (component 'lending')"
    );
}

#[test]
fn worker_on_the_edge_is_a_capability_error() {
    let source = concat!(
        "application Svc {\n",
        "    component search: worker {}\n",
        "}\n",
        "\n",
        "profile edge {\n",
        "    target: cloudflareWorkers\n",
        "}\n",
        "\n",
        "deployment prod for Svc {\n",
        "    use edge\n",
        "}\n",
    );
    let compilation = compile_with(&[], source);
    assert!(compilation.model.is_none());
    let (_, diagnostic) = compilation
        .diagnostics
        .iter()
        .find(|(_, diagnostic)| diagnostic.message.contains("longRunningProcess"))
        .expect("a capability diagnostic");
    assert_eq!(
        diagnostic.message,
        "component 'search' requires capability 'longRunningProcess', which target \
         'cloudflareWorkers' does not provide"
    );
}

#[test]
fn postgres_on_the_edge_is_an_engine_error() {
    let source = concat!(
        "application Svc {\n",
        "    component db: database {\n",
        "        engine: postgres\n",
        "    }\n",
        "}\n",
        "\n",
        "profile edge {\n",
        "    target: cloudflareWorkers\n",
        "}\n",
        "\n",
        "deployment prod for Svc {\n",
        "    use edge\n",
        "}\n",
    );
    let compilation = compile_with(&[], source);
    assert!(compilation.model.is_none());
    let (_, diagnostic) = compilation
        .diagnostics
        .iter()
        .find(|(_, diagnostic)| diagnostic.message.contains("postgres"))
        .expect("an engine diagnostic");
    assert_eq!(
        diagnostic.message,
        "database 'db' engine 'postgres' is not available on target 'cloudflareWorkers'"
    );
}

#[test]
fn policy_must_type_as_boolean() {
    let source = concat!(
        "application Svc {\n",
        "    component api: api {}\n",
        "}\n",
        "\n",
        "profile prod {\n",
        "    target: standalone\n",
        "    defaults {\n",
        "        api.replicas: 2\n",
        "    }\n",
        "    require (api.replicas + 1)\n",
        "}\n",
        "\n",
        "deployment dev for Svc {\n",
        "    use prod\n",
        "}\n",
    );
    let compilation = compile_with(&[], source);
    assert!(compilation.model.is_none());
    let (_, diagnostic) = compilation
        .diagnostics
        .iter()
        .find(|(_, diagnostic)| diagnostic.message.contains("boolean"))
        .expect("a type diagnostic");
    assert_eq!(
        diagnostic.message,
        "policy condition must be boolean, found int"
    );
}

#[test]
fn unknown_component_in_policy_is_a_type_error() {
    let source = concat!(
        "application Svc {\n",
        "    component api: api {}\n",
        "}\n",
        "\n",
        "profile prod {\n",
        "    target: standalone\n",
        "    defaults {\n",
        "        api.replicas: 2\n",
        "    }\n",
        "    require (worker.replicas >= 1)\n",
        "}\n",
        "\n",
        "deployment dev for Svc {\n",
        "    use prod\n",
        "}\n",
    );
    let compilation = compile_with(&[], source);
    assert!(compilation.model.is_none());
    assert!(
        compilation
            .diagnostics
            .iter()
            .any(|(_, diagnostic)| diagnostic.message.contains("invalid policy condition")),
        "an unknown component must surface as a policy type error: {:?}",
        compilation.diagnostics
    );
}

#[test]
fn violated_require_is_an_error() {
    let source = concat!(
        "application Svc {\n",
        "    component api: api {}\n",
        "}\n",
        "\n",
        "profile prod {\n",
        "    target: standalone\n",
        "    defaults {\n",
        "        api.replicas: 2\n",
        "    }\n",
        "    require (api.replicas >= 3)\n",
        "}\n",
        "\n",
        "deployment dev for Svc {\n",
        "    use prod\n",
        "}\n",
    );
    let compilation = compile_with(&[], source);
    assert!(compilation.model.is_none());
    let (_, diagnostic) = compilation
        .diagnostics
        .iter()
        .find(|(_, diagnostic)| diagnostic.message.contains("policy violated"))
        .expect("a violation diagnostic");
    assert_eq!(
        diagnostic.message,
        "policy violated in deployment 'dev': require (api.replicas >= 3)"
    );
}

#[test]
fn override_violating_prohibit_is_an_error() {
    let source = concat!(
        "application Svc {\n",
        "    component db: database {\n",
        "        engine: sqlite\n",
        "    }\n",
        "}\n",
        "\n",
        "profile prod {\n",
        "    target: standalone\n",
        "    prohibit (db.storage == \"local\")\n",
        "}\n",
        "\n",
        "deployment dev for Svc {\n",
        "    use prod\n",
        "    configure {\n",
        "        db.storage: local\n",
        "    }\n",
        "}\n",
    );
    let compilation = compile_with(&[], source);
    assert!(compilation.model.is_none());
    let (_, diagnostic) = compilation
        .diagnostics
        .iter()
        .find(|(_, diagnostic)| diagnostic.message.contains("policy violated"))
        .expect("a violation diagnostic");
    assert_eq!(
        diagnostic.message,
        "policy violated in deployment 'dev': prohibit (db.storage == \"local\")"
    );
}

#[test]
fn default_naming_unknown_component_is_an_error() {
    let source = concat!(
        "application Svc {\n",
        "    component api: api {}\n",
        "}\n",
        "\n",
        "profile prod {\n",
        "    target: standalone\n",
        "    defaults {\n",
        "        worker.replicas: 2\n",
        "    }\n",
        "}\n",
        "\n",
        "deployment dev for Svc {\n",
        "    use prod\n",
        "}\n",
    );
    let compilation = compile_with(&[], source);
    assert!(compilation.model.is_none());
    let (_, diagnostic) = compilation
        .diagnostics
        .iter()
        .find(|(_, diagnostic)| diagnostic.message.contains("unknown component 'worker'"))
        .expect("an unknown-component diagnostic");
    assert!(
        diagnostic.message.contains("profile 'prod'"),
        "the diagnostic must name the profile: {:?}",
        diagnostic
    );
}
