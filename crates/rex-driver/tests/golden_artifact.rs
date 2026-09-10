//! Golden-artifact conformance test: the driver output for the flagship
//! model must byte-match the committed Core IR artifact.
//!
//! Regenerate with `REX_UPDATE_FIXTURES=1 cargo test -p rex-driver`.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use rex_driver::compile_str;

const MODEL_RELATIVE_PATH: &str = "tests/conformance/models/library.mox";

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

#[test]
fn library_model_matches_golden_artifact() {
    let source = std::fs::read_to_string(fixture_path(MODEL_RELATIVE_PATH))
        .expect("read conformance model");
    let compilation = compile_str(MODEL_RELATIVE_PATH, &source);
    assert!(
        compilation.diagnostics.is_empty(),
        "conformance model must compile cleanly: {:?}",
        compilation.diagnostics
    );
    let model = compilation.model.expect("model lowered");
    let json = model.to_json_pretty().expect("serialize IR");

    let artifact = fixture_path("tests/conformance/artifacts/library.rex.json");
    if std::env::var("REX_UPDATE_FIXTURES").is_ok() {
        std::fs::create_dir_all(artifact.parent().unwrap()).unwrap();
        std::fs::write(&artifact, &json).unwrap();
        eprintln!("updated {}", artifact.display());
        return;
    }
    let expected = std::fs::read_to_string(&artifact)
        .unwrap_or_else(|_| panic!("missing golden artifact {}; regenerate with REX_UPDATE_FIXTURES=1", artifact.display()));
    assert_eq!(json, expected, "IR artifact drifted from the golden file");
}
