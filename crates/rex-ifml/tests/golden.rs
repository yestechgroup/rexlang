//! Golden-artifact conformance test: parsing the canonical `.ifml` model
//! must produce an [`IfmlModel`] whose pretty JSON byte-matches the
//! committed golden artifact, and the artifact must round-trip through
//! `from_json`. Compiling the same models and exporting the control/route
//! manifest must byte-match the committed manifest goldens the same way
//! (`app.manifest.json`, `cli.manifest.json`).
//!
//! Regenerate with `REX_UPDATE_FIXTURES=1 cargo test -p rex-ifml`.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use rex_ifml::{
    compile_ifml_str, manifest_ifml, parse_ifml, IfmlImports, IfmlManifest, IfmlModel,
    IFML_MODEL_FORMAT_VERSION,
};

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

const MODEL_PATH: &str = "tests/conformance/ifml/app.ifml";
const ARTIFACT_PATH: &str = "tests/conformance/ifml/app.ifml.json";
const CLI_MODEL_PATH: &str = "tests/conformance/ifml/cli.ifml";
const CLI_ARTIFACT_PATH: &str = "tests/conformance/ifml/cli.ifml.json";
const MANIFEST_PATH: &str = "tests/conformance/ifml/app.manifest.json";
const CLI_MANIFEST_PATH: &str = "tests/conformance/ifml/cli.manifest.json";

#[test]
fn conformance_model_matches_golden_artifact() {
    golden_matches(MODEL_PATH, ARTIFACT_PATH);
}

#[test]
fn cli_conformance_model_matches_golden_artifact() {
    golden_matches(CLI_MODEL_PATH, CLI_ARTIFACT_PATH);
}

#[test]
fn conformance_model_matches_golden_manifest() {
    manifest_matches(MODEL_PATH, MANIFEST_PATH);
}

#[test]
fn cli_conformance_model_matches_golden_manifest() {
    manifest_matches(CLI_MODEL_PATH, CLI_MANIFEST_PATH);
}

fn golden_matches(model_relative: &str, artifact_relative: &str) {
    let absolute = fixture_path(model_relative);
    let source = std::fs::read_to_string(&absolute)
        .unwrap_or_else(|error| panic!("read conformance model {model_relative}: {error}"));

    let model = parse_ifml(&source).expect("conformance model must parse");
    if model_relative == MODEL_PATH {
        assert!(
            !model.views.is_empty(),
            "conformance model must carry views"
        );
    } else {
        assert!(
            !model.commands.is_empty(),
            "the CLI conformance model must carry commands"
        );
    }
    assert_eq!(model.format_version, IFML_MODEL_FORMAT_VERSION);

    let json = model.to_json_pretty().expect("serialize IR");

    let artifact = fixture_path(artifact_relative);
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
        "IFML artifact for {model_relative} drifted from the golden file"
    );
}

#[test]
fn golden_artifact_round_trips_through_from_json() {
    let expected = std::fs::read_to_string(fixture_path(ARTIFACT_PATH))
        .unwrap_or_else(|error| panic!("read golden artifact {ARTIFACT_PATH}: {error}"));
    let model = IfmlModel::from_json(&expected).expect("golden artifact must deserialize");
    assert_eq!(model.format_version, IFML_MODEL_FORMAT_VERSION);
    let reserialized = model.to_json_pretty().expect("re-serialize IR");
    assert_eq!(reserialized, expected, "artifact round trip must be stable");
}

/// Compiles `model_relative` (no imports — the conformance fixtures
/// declare everything locally or import only non-`.ifml` surfaces) and
/// byte-compares its control/route manifest against the golden file.
fn manifest_matches(model_relative: &str, artifact_relative: &str) {
    let absolute = fixture_path(model_relative);
    let source = std::fs::read_to_string(&absolute)
        .unwrap_or_else(|error| panic!("read conformance model {model_relative}: {error}"));

    let compilation = compile_ifml_str(model_relative, &source, &IfmlImports::default())
        .unwrap_or_else(|diagnostics| panic!("{model_relative} must compile: {diagnostics:?}"));
    let diags = rex_ifml::check_ifml(&compilation, None);
    assert!(
        diags.is_empty(),
        "{model_relative} must pass the checker: {diags:?}"
    );
    let manifest = manifest_ifml(&compilation, &IfmlImports::default(), None);
    if model_relative == MODEL_PATH {
        assert!(
            !manifest.views.is_empty() && !manifest.edges.is_empty(),
            "the app conformance manifest must carry routes and edges"
        );
    } else {
        assert!(
            !manifest.commands.is_empty(),
            "the CLI conformance manifest must carry commands"
        );
    }

    let json = manifest.to_json_pretty().expect("serialize manifest");
    let artifact = fixture_path(artifact_relative);
    if std::env::var("REX_UPDATE_FIXTURES").is_ok() {
        std::fs::create_dir_all(artifact.parent().unwrap()).unwrap();
        std::fs::write(&artifact, &json).unwrap();
        eprintln!("updated {}", artifact.display());
        return;
    }
    let expected = std::fs::read_to_string(&artifact).unwrap_or_else(|_| {
        panic!(
            "missing golden manifest {}; regenerate with REX_UPDATE_FIXTURES=1",
            artifact.display()
        )
    });
    assert_eq!(
        json, expected,
        "IFML manifest for {model_relative} drifted from the golden file"
    );
}

#[test]
fn golden_manifest_round_trips_through_from_json() {
    for artifact_relative in [MANIFEST_PATH, CLI_MANIFEST_PATH] {
        let expected = std::fs::read_to_string(fixture_path(artifact_relative))
            .unwrap_or_else(|error| panic!("read golden manifest {artifact_relative}: {error}"));
        let manifest = IfmlManifest::from_json(&expected)
            .unwrap_or_else(|error| panic!("golden manifest {artifact_relative}: {error}"));
        assert_eq!(
            manifest.format_version,
            rex_ifml::IFML_MANIFEST_FORMAT_VERSION
        );
        let reserialized = manifest.to_json_pretty().expect("re-serialize manifest");
        assert_eq!(
            reserialized, expected,
            "manifest round trip must be stable ({artifact_relative})"
        );
    }
}
