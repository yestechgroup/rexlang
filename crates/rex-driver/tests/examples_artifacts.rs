//! Examples conformance harness (artifact surfaces): every canonical
//! `.evt`, `.ddd`, `.uc`, `.deploy`, and `.cmmn` example under `examples/` must
//! compile with zero diagnostics against its imports — resolved from disk
//! exactly the way the CLI resolves them — and byte-match its committed
//! golden artifact under `examples/artifacts/`, which must round-trip
//! through the artifact's `from_json`.
//!
//! The lists are hardcoded (the house convention: a missing file fails
//! loudly instead of being skipped) and a sync test pins each list against
//! the directory, so a new example can never be added unlisted.
//! `support.cmmn` compiles against all four buckets (the `.uc` rule), and
//! `support.ddd` is additionally compiled through
//! `compile_ddd_str_with_actors`, proving its service-operation
//! capabilities validate against the real `support.actor` policy.
//!
//! Regenerate the goldens with `REX_UPDATE_FIXTURES=1 cargo test -p
//! rex-driver --test examples_artifacts`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// The canonical `.evt` event-contract examples.
const EVT_EXAMPLES: [&str; 2] = ["ecommerce", "support"];

/// The canonical `.ddd` design examples.
const DDD_EXAMPLES: [&str; 2] = ["orders", "support"];

/// The canonical `.uc` use-case examples.
const UC_EXAMPLES: [&str; 1] = ["support"];

/// The canonical `.deploy` deployment examples.
const DEPLOY_EXAMPLES: [&str; 1] = ["support"];

/// The canonical `.cmmn` case-management examples.
const CMMN_EXAMPLES: [&str; 1] = ["support"];

fn workspace_root() -> &'static Path {
    static ROOT: OnceLock<PathBuf> = OnceLock::new();
    ROOT.get_or_init(|| {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../")
            .canonicalize()
            .expect("workspace root")
    })
}

fn examples_dir() -> PathBuf {
    workspace_root().join("examples")
}

fn read(path: &Path) -> String {
    std::fs::read_to_string(path)
        .unwrap_or_else(|error| panic!("missing canonical example {}: {error}", path.display()))
}

/// The example's path and source: `examples/<name>.<extension>`.
fn example(name: &str, extension: &str) -> (PathBuf, String) {
    let path = examples_dir().join(format!("{name}.{extension}"));
    let source = read(&path);
    (path, source)
}

/// An `.evt`/`.ddd` example's imported domain stem (the `.mox` each file
/// declares), resolved next to the example the way the CLI resolves
/// import paths (the driver matches the resolved path, so the key is the
/// joined absolute path).
fn domain_of(example_name: &str) -> (String, String) {
    let stem = match example_name {
        // orders.ddd designs the shop domain.
        "orders" => "ecommerce",
        other => other,
    };
    let path = examples_dir().join(format!("{stem}.mox"));
    (path.display().to_string(), read(&path))
}

fn assert_clean<D: std::fmt::Debug>(where_: &str, diagnostics: &[D]) {
    assert!(
        diagnostics.is_empty(),
        "{where_} must compile with zero diagnostics: {:?}",
        diagnostics
    );
}

/// Byte-compares `json` against the committed golden artifact for
/// `name.extension`, regenerating it under `REX_UPDATE_FIXTURES`.
fn assert_matches_golden(name: &str, extension: &str, json: &str) {
    let golden = examples_dir()
        .join("artifacts")
        .join(format!("{name}.{extension}.json"));
    if std::env::var("REX_UPDATE_FIXTURES").is_ok() {
        std::fs::create_dir_all(golden.parent().unwrap()).expect("create artifacts dir");
        std::fs::write(&golden, json).expect("write golden artifact");
        eprintln!("updated {}", golden.display());
        return;
    }
    let expected = std::fs::read_to_string(&golden).unwrap_or_else(|error| {
        panic!(
            "missing golden artifact {}: {error} — regenerate with REX_UPDATE_FIXTURES=1",
            golden.display()
        )
    });
    assert_eq!(
        json, expected,
        "artifact for {name}.{extension} drifted from the golden file"
    );
}

fn compile_evt(name: &str) -> rex_ir::events::EventModel {
    let (path, source) = example(name, "evt");
    let (domain_path, domain) = domain_of(name);
    let compilation = rex_driver::compile_evt_str(
        path.to_str().expect("utf-8 example path"),
        &source,
        &[(domain_path, domain)],
        &rex_driver::DomainImports::default(),
    );
    assert_clean(&format!("{name}.evt"), &compilation.diagnostics);
    compilation
        .model
        .unwrap_or_else(|| panic!("{name}.evt must lower to an event artifact"))
}

fn compile_ddd(name: &str) -> rex_ir::ddd::DddModel {
    let (path, source) = example(name, "ddd");
    let (domain_path, domain) = domain_of(name);
    let compilation = rex_driver::compile_ddd_str(
        path.to_str().expect("utf-8 example path"),
        &source,
        &[(domain_path, domain)],
        &rex_driver::DomainImports::default(),
    );
    assert_clean(&format!("{name}.ddd"), &compilation.diagnostics);
    compilation
        .model
        .unwrap_or_else(|| panic!("{name}.ddd must lower to a design artifact"))
}

/// The `support.actor` policy, compiled as a pair against the domain it
/// imports. The `.actor` import matching is by exact import string (the
/// driver receives texts only), so the key is the path as written.
fn support_actor_model() -> rex_ir::ActorModel {
    let (path, source) = example("support", "actor");
    let domain = read(&examples_dir().join("support.mox"));
    let compilation = rex_driver::compile_actors_str(
        path.to_str().expect("utf-8 example path"),
        &source,
        &[("support.mox".to_string(), domain)],
        &rex_driver::DomainImports::default(),
    );
    assert_clean("support.actor", &compilation.diagnostics);
    compilation
        .model
        .unwrap_or_else(|| panic!("support.actor must lower to an actor model"))
}

/// The `support.ifml` flow, compiled with its `.ifml` imports (the
/// pattern library) gathered from disk through the shared traversal.
fn support_ifml_model() -> rex_ir::ifml::IfmlModel {
    let (path, source) = example("support", "ifml");
    let path_string = path.display().to_string();
    let mut imports = rex_ifml::IfmlImports::default();
    let mut disk_paths = BTreeMap::new();
    disk_paths.insert(path_string.clone(), path.clone());
    rex_ifml::walk_ifml_imports(&path_string, &source, &mut |importer, import| {
        let importer_disk = disk_paths.get(importer)?;
        let resolved = importer_disk.parent().unwrap().join(import);
        if import.ends_with(".ifml") {
            let imported = read(&resolved);
            disk_paths.insert(import.to_string(), resolved);
            imports.provide(importer, import, imported.clone());
            Some(imported)
        } else {
            None
        }
    });
    let compilation = rex_ifml::compile_ifml_str(&path_string, &source, &imports)
        .expect("support.ifml must compile with zero diagnostics");
    compilation.model
}

#[test]
fn evt_examples_match_golden_artifacts_and_round_trip() {
    for name in EVT_EXAMPLES {
        let model = compile_evt(name);
        assert_eq!(
            model.format_version,
            rex_ir::events::EVENT_MODEL_FORMAT_VERSION
        );
        let json = model.to_json_pretty().expect("serialize event artifact");
        assert_matches_golden(name, "evt", &json);
        let reloaded = rex_ir::events::EventModel::from_json(&json)
            .unwrap_or_else(|error| panic!("{name}.evt artifact must round-trip: {error}"));
        assert_eq!(
            reloaded.to_json_pretty().expect("re-serialize"),
            json,
            "{name}.evt artifact round trip must be stable"
        );
    }
}

#[test]
fn ddd_examples_match_golden_artifacts_and_round_trip() {
    for name in DDD_EXAMPLES {
        let model = compile_ddd(name);
        assert_eq!(model.format_version, rex_ir::ddd::DDD_MODEL_FORMAT_VERSION);
        let json = model.to_json_pretty().expect("serialize design artifact");
        assert_matches_golden(name, "ddd", &json);
        let reloaded = rex_ir::ddd::DddModel::from_json(&json)
            .unwrap_or_else(|error| panic!("{name}.ddd artifact must round-trip: {error}"));
        assert_eq!(
            reloaded.to_json_pretty().expect("re-serialize"),
            json,
            "{name}.ddd artifact round trip must be stable"
        );
    }
}

/// The loose design↛policy coupling, proven tight for the one example
/// that ships both dimensions: `support.ddd`'s capability clauses name
/// real `support.actor` capabilities.
#[test]
fn support_ddd_capabilities_validate_against_the_actor_model() {
    let (path, source) = example("support", "ddd");
    let (domain_path, domain) = domain_of("support");
    let actors = support_actor_model();
    let compilation = rex_driver::compile_ddd_str_with_actors(
        path.to_str().expect("utf-8 example path"),
        &source,
        &[(domain_path, domain)],
        &rex_driver::DomainImports::default(),
        &actors,
    );
    assert_clean("support.ddd (with actors)", &compilation.diagnostics);
    assert!(
        compilation.model.is_some(),
        "support.ddd must lower to a design artifact under actor validation"
    );
}

#[test]
fn uc_examples_match_golden_artifacts_and_round_trip() {
    for name in UC_EXAMPLES {
        let (path, source) = example(name, "uc");
        let dir = examples_dir();
        let domains = vec![domain_of(name)];
        let designs = vec![(
            dir.join("support.ddd").display().to_string(),
            compile_ddd("support"),
        )];
        let actors = vec![(
            dir.join("support.actor").display().to_string(),
            support_actor_model(),
        )];
        let events = vec![(
            dir.join("support.evt").display().to_string(),
            compile_evt("support"),
        )];
        let compilation = rex_driver::compile_usecase_str(
            path.to_str().expect("utf-8 example path"),
            &source,
            &domains,
            &rex_driver::DomainImports::default(),
            &designs,
            &actors,
            &events,
        );
        assert_clean(&format!("{name}.uc"), &compilation.diagnostics);
        let model = compilation
            .model
            .unwrap_or_else(|| panic!("{name}.uc must lower to a use-case artifact"));
        assert_eq!(
            model.format_version,
            rex_ir::usecases::USE_CASE_MODEL_FORMAT_VERSION
        );
        let json = model.to_json_pretty().expect("serialize use-case artifact");
        assert_matches_golden(name, "uc", &json);
        let reloaded = rex_ir::usecases::UseCaseModel::from_json(&json)
            .unwrap_or_else(|error| panic!("{name}.uc artifact must round-trip: {error}"));
        assert_eq!(
            reloaded.to_json_pretty().expect("re-serialize"),
            json,
            "{name}.uc artifact round trip must be stable"
        );
    }
}

#[test]
fn deploy_examples_match_golden_artifacts_and_round_trip() {
    for name in DEPLOY_EXAMPLES {
        let (path, source) = example(name, "deploy");
        let dir = examples_dir();
        let designs = vec![(
            dir.join("support.ddd").display().to_string(),
            compile_ddd("support"),
        )];
        let flows = vec![(
            dir.join("support.ifml").display().to_string(),
            support_ifml_model(),
        )];
        let compilation = rex_driver::compile_deploy_str(
            path.to_str().expect("utf-8 example path"),
            &source,
            &designs,
            &flows,
        );
        assert_clean(&format!("{name}.deploy"), &compilation.diagnostics);
        let model = compilation
            .model
            .unwrap_or_else(|| panic!("{name}.deploy must lower to a deployment artifact"));
        assert_eq!(
            model.format_version,
            rex_ir::deploy::DEPLOY_MODEL_FORMAT_VERSION
        );
        let json = model
            .to_json_pretty()
            .expect("serialize deployment artifact");
        assert_matches_golden(name, "deploy", &json);
        let reloaded = rex_ir::deploy::DeployModel::from_json(&json)
            .unwrap_or_else(|error| panic!("{name}.deploy artifact must round-trip: {error}"));
        assert_eq!(
            reloaded.to_json_pretty().expect("re-serialize"),
            json,
            "{name}.deploy artifact round trip must be stable"
        );
    }
}

/// Compiles one `.cmmn` case example against its four buckets, resolved
/// the way the CLI resolves them: absolute paths for the raw domain (the
/// `.evt` rule) and for the pre-compiled event/actor/design artifacts (the
/// four-bucket rule, `.uc` matching).
fn compile_cmmn(name: &str) -> rex_ir::cmmn::CmmnModel {
    let (path, source) = example(name, "cmmn");
    let dir = examples_dir();
    let domains = vec![domain_of(name)];
    let designs = vec![(
        dir.join("support.ddd").display().to_string(),
        compile_ddd("support"),
    )];
    let actors = vec![(
        dir.join("support.actor").display().to_string(),
        support_actor_model(),
    )];
    let events = vec![(
        dir.join("support.evt").display().to_string(),
        compile_evt("support"),
    )];
    let compilation = rex_driver::compile_cmmn_str(
        path.to_str().expect("utf-8 example path"),
        &source,
        &domains,
        &rex_driver::DomainImports::default(),
        &designs,
        &actors,
        &events,
    );
    assert_clean(&format!("{name}.cmmn"), &compilation.diagnostics);
    compilation
        .model
        .unwrap_or_else(|| panic!("{name}.cmmn must lower to a case artifact"))
}

#[test]
fn cmmn_examples_match_golden_artifacts_and_round_trip() {
    for name in CMMN_EXAMPLES {
        let model = compile_cmmn(name);
        assert_eq!(
            model.format_version,
            rex_ir::cmmn::CMMN_MODEL_FORMAT_VERSION
        );
        let json = model.to_json_pretty().expect("serialize case artifact");
        assert_matches_golden(name, "cmmn", &json);
        let reloaded = rex_ir::cmmn::CmmnModel::from_json(&json)
            .unwrap_or_else(|error| panic!("{name}.cmmn artifact must round-trip: {error}"));
        assert_eq!(
            reloaded.to_json_pretty().expect("re-serialize"),
            json,
            "{name}.cmmn artifact round trip must be stable"
        );
    }
}

/// The house convention from `crates/rex-ifml/tests/examples.rs`: each
/// hardcoded list must cover every file of its extension on disk, so a
/// new example can never be added unlisted.
#[test]
fn the_example_lists_cover_every_artifact_file_on_disk() {
    let assert_sync = |list: &[&str], extension: &str| {
        let mut listed: Vec<String> = list
            .iter()
            .map(|name| format!("{name}.{extension}"))
            .collect();
        listed.sort();
        let mut on_disk: Vec<String> = std::fs::read_dir(examples_dir())
            .expect("examples directory exists")
            .map(|entry| entry.expect("readable entry").path())
            .filter(|path| {
                path.extension()
                    .is_some_and(|candidate| candidate == extension)
            })
            .map(|path| {
                path.file_name()
                    .expect("named file")
                    .to_string_lossy()
                    .into_owned()
            })
            .collect();
        on_disk.sort();
        assert_eq!(
            listed, on_disk,
            "examples/*.{extension} and its hardcoded list must stay in sync"
        );
    };
    assert_sync(&EVT_EXAMPLES, "evt");
    assert_sync(&DDD_EXAMPLES, "ddd");
    assert_sync(&UC_EXAMPLES, "uc");
    assert_sync(&DEPLOY_EXAMPLES, "deploy");
    assert_sync(&CMMN_EXAMPLES, "cmmn");
}
