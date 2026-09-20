//! Golden-artifact conformance test: parsing the canonical `.ifml` model
//! must produce an [`IfmlModel`] whose pretty JSON byte-matches the
//! committed golden artifact, and the artifact must round-trip through
//! `from_json`.
//!
//! Regenerate with `REX_UPDATE_FIXTURES=1 cargo test -p rex-ifml`.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use rex_ifml::{parse_ifml, IfmlModel, IFML_MODEL_FORMAT_VERSION};

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

#[test]
fn conformance_model_matches_golden_artifact() {
    let absolute = fixture_path(MODEL_PATH);
    let source = std::fs::read_to_string(&absolute)
        .unwrap_or_else(|error| panic!("read conformance model {MODEL_PATH}: {error}"));

    let model = parse_ifml(&source).expect("conformance model must parse");
    assert!(
        !model.views.is_empty(),
        "conformance model must carry views"
    );
    assert_eq!(model.format_version, IFML_MODEL_FORMAT_VERSION);

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
        "IFML artifact for {MODEL_PATH} drifted from the golden file"
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
