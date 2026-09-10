//! Golden-artifact conformance test: the driver output for each flagship
//! model must byte-match the committed Core IR artifact.
//!
//! Regenerate with `REX_UPDATE_FIXTURES=1 cargo test -p rex-driver`.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use rex_driver::compile_str;

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

/// Every conformance model and its committed golden artifact.
const MODELS: &[(&str, &str)] = &[
    ("tests/conformance/models/library.mox", "tests/conformance/artifacts/library.rex.json"),
    ("tests/conformance/models/currency.mox", "tests/conformance/artifacts/currency.rex.json"),
];

fn compile_conformance_model(relative_path: &str) -> rex_ir::Model {
    // Absolute path: the driver reads `vocab/` and `model.lock` relative to
    // the model path, and tests run with the crate dir as CWD.
    let absolute = fixture_path(relative_path);
    let source = std::fs::read_to_string(&absolute)
        .unwrap_or_else(|error| panic!("read conformance model {relative_path}: {error}"));
    let compilation = compile_str(absolute.to_str().expect("utf-8 path"), &source);
    assert!(
        compilation.diagnostics.is_empty(),
        "conformance model {relative_path} must compile cleanly: {:?}",
        compilation.diagnostics
    );
    compilation.model.expect("model lowered")
}

#[test]
fn conformance_models_match_golden_artifacts() {
    for (model_path, artifact_path) in MODELS {
        let model = compile_conformance_model(model_path);
        let json = model.to_json_pretty().expect("serialize IR");

        let artifact = fixture_path(artifact_path);
        if std::env::var("REX_UPDATE_FIXTURES").is_ok() {
            std::fs::create_dir_all(artifact.parent().unwrap()).unwrap();
            std::fs::write(&artifact, &json).unwrap();
            eprintln!("updated {}", artifact.display());
            continue;
        }
        let expected = std::fs::read_to_string(&artifact).unwrap_or_else(|_| {
            panic!(
                "missing golden artifact {}; regenerate with REX_UPDATE_FIXTURES=1",
                artifact.display()
            )
        });
        assert_eq!(
            json, expected,
            "IR artifact for {model_path} drifted from the golden file"
        );
    }
}

/// The `model.lock` committed next to the conformance models must pin the
/// exact bytes of the vendored snapshot (hermetic builds key on this).
#[test]
fn currency_lockfile_digest_matches_vendored_snapshot() {
    let snapshot = std::fs::read(fixture_path("tests/conformance/models/vocab/iso-4217@2024-01-01.json"))
        .expect("read vendored snapshot");
    let lockfile =
        rex_vocab::Lockfile::read(&fixture_path("tests/conformance/models/model.lock"))
            .expect("read model.lock");
    let entry = lockfile
        .entry_for("iso:4217")
        .expect("model.lock pins iso:4217");
    assert_eq!(entry.version, "2024-01-01");
    assert_eq!(
        entry.digest,
        rex_vocab::digest(&snapshot),
        "lockfile digest must match the committed snapshot bytes"
    );
}

/// The currency conformance model lowers its vocabulary with the vendored
/// entries embedded (entries are EMBEDDED so artifacts stay self-contained).
#[test]
fn currency_model_lowers_embedded_vocabulary_entries() {
    let model = compile_conformance_model("tests/conformance/models/currency.mox");
    let package = &model.packages[0];
    assert_eq!(package.name, "nz.example.payments");

    let vocabulary = &package.vocabularies[0];
    assert_eq!(vocabulary.name, "Currency");
    assert_eq!(vocabulary.source, "iso:4217");
    assert_eq!(vocabulary.version.as_deref(), Some("2024-01-01"));
    assert_eq!(vocabulary.key, "alpha3");
    assert_eq!(vocabulary.entries.len(), 5);
    assert_eq!(vocabulary.entries[0].key, "USD");
    assert_eq!(vocabulary.entries[0].facets["symbol"], rex_ir::DefaultValue::String("$".to_string()));
    assert_eq!(vocabulary.entries[2].facets["minorUnits"], rex_ir::DefaultValue::Int(0));
    let account = &package.classes[0];
    assert_eq!(account.name, "Account");
    assert_eq!(
        account.features[1].type_,
        rex_ir::TypeRef::Vocabulary {
            package: "nz.example.payments".to_string(),
            name: "Currency".to_string(),
        }
    );
}
