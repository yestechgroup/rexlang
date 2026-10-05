//! Golden-artifact conformance test: compiling the canonical `.ddd` design
//! against its imported domain model must produce a [`DddModel`] whose
//! pretty JSON byte-matches the committed golden artifact, and the artifact
//! must round-trip through `from_json`.
//!
//! Regenerate with `REX_UPDATE_FIXTURES=1 cargo test -p rex-driver
//! --test ddd_golden`.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use rex_driver::compile_ddd_str;

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

const DESIGN_PATH: &str = "tests/conformance/ddd/library.ddd";
const DOMAIN_PATH: &str = "tests/conformance/ddd/library.mox";
const ARTIFACT_PATH: &str = "tests/conformance/ddd/library.ddd.json";

#[test]
fn conformance_design_matches_golden_artifact() {
    let design_absolute = fixture_path(DESIGN_PATH);
    let source = std::fs::read_to_string(&design_absolute)
        .unwrap_or_else(|error| panic!("read conformance design {DESIGN_PATH}: {error}"));
    let domain_absolute = fixture_path(DOMAIN_PATH);
    let domain = std::fs::read_to_string(&domain_absolute)
        .unwrap_or_else(|error| panic!("read conformance domain {DOMAIN_PATH}: {error}"));

    // The driver is filesystem-free: the harness resolves the import the
    // way the CLI does (relative to the design file) and provides the
    // domain text under its resolved absolute path.
    let compilation = compile_ddd_str(
        design_absolute.to_str().expect("utf-8 path"),
        &source,
        &[(
            domain_absolute.to_str().expect("utf-8 path").to_string(),
            domain,
        )],
        &rex_driver::DomainImports::empty(),
    );
    assert!(
        compilation.diagnostics.is_empty(),
        "conformance design must compile clean: {:?}",
        compilation.diagnostics
    );
    let model = compilation.model.expect("design artifact lowered");
    assert_eq!(model.format_version, rex_ir::ddd::DDD_MODEL_FORMAT_VERSION);
    assert_eq!(
        model
            .application
            .as_ref()
            .map(|application| application.name.as_str()),
        Some("Library")
    );
    assert_eq!(
        model
            .modules
            .iter()
            .map(|module| module.name.as_str())
            .collect::<Vec<_>>(),
        ["media", "person"]
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
        "DDD artifact for {DESIGN_PATH} drifted from the golden file"
    );
}

#[test]
fn golden_artifact_round_trips_through_from_json() {
    let expected = std::fs::read_to_string(fixture_path(ARTIFACT_PATH))
        .unwrap_or_else(|error| panic!("read golden artifact {ARTIFACT_PATH}: {error}"));
    let model =
        rex_ir::ddd::DddModel::from_json(&expected).expect("golden artifact must deserialize");
    assert_eq!(model.format_version, rex_ir::ddd::DDD_MODEL_FORMAT_VERSION);
    let reserialized = model.to_json_pretty().expect("re-serialize IR");
    assert_eq!(reserialized, expected, "artifact round trip must be stable");
}
