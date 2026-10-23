//! Golden-artifact conformance test: compiling the canonical `.mox` with its
//! `import sigil` declaration must produce a [`rex_ir::Model`] whose pretty
//! JSON byte-matches the committed golden artifact, and the artifact must
//! round-trip through `from_json`.
//!
//! Regenerate with `REX_UPDATE_FIXTURES=1 cargo test -p rex-driver --test
//! sigil_golden`.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use rex_driver::{compile_files, DomainImports, SchemaImports, SigilImports};

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

const MODEL_PATH: &str = "tests/conformance/sigil/basic.mox";
const SIGIL_PATH: &str = "basic.rosetta";
const ARTIFACT_PATH: &str = "tests/conformance/sigil/basic.mox.json";

const FUNCS_MODEL_PATH: &str = "tests/conformance/sigil/funcs.mox";
const FUNCS_SIGIL_PATH: &str = "funcs.rosetta";
const FUNCS_ARTIFACT_PATH: &str = "tests/conformance/sigil/funcs.mox.json";

#[test]
fn conformance_sigil_model_matches_golden_artifact() {
    golden_artifact_matches(
        MODEL_PATH,
        SIGIL_PATH,
        ARTIFACT_PATH,
        &["rex.conformance.sigil", "rex.sigil.fixture"],
    );
}

#[test]
fn conformance_sigil_functions_match_golden_artifact() {
    golden_artifact_matches(
        FUNCS_MODEL_PATH,
        FUNCS_SIGIL_PATH,
        FUNCS_ARTIFACT_PATH,
        &["rex.conformance.funcs", "rex.sigil.funcs"],
    );
}

/// Compiles the canonical `.mox` with its `import sigil` declaration and
/// byte-compares the resulting IR artifact against the committed golden.
fn golden_artifact_matches(
    model_path: &str,
    sigil_path: &str,
    artifact_path: &str,
    expected_packages: &[&str],
) {
    let model_absolute = fixture_path(model_path);
    let source = std::fs::read_to_string(&model_absolute)
        .unwrap_or_else(|error| panic!("read conformance model {model_path}: {error}"));
    let sigil_absolute = model_absolute
        .parent()
        .expect("model directory")
        .join(sigil_path);
    let rosetta = std::fs::read_to_string(&sigil_absolute)
        .unwrap_or_else(|error| panic!("read conformance sigil {sigil_path}: {error}"));

    // The driver is filesystem-free: the harness resolves the sigil import
    // the way the CLI does (relative to the model file) and provides the
    // rosetta text under the model's absolute path.
    let sigil = SigilImports::new().provide(
        model_absolute.to_str().expect("utf-8 path"),
        sigil_path,
        rosetta,
    );
    let compilation = compile_files(
        &[(
            model_absolute.to_str().expect("utf-8 path").to_string(),
            source,
        )],
        &DomainImports {
            schemas: SchemaImports::new(),
            sigil,
        },
    );
    let mut diagnostics = compilation.diagnostics.clone();
    diagnostics.extend(compilation.sigil_diagnostics.clone());
    assert!(
        diagnostics.is_empty(),
        "conformance model must compile clean: {:?}",
        diagnostics
    );
    let model = compilation.model.expect("model lowered");
    assert_eq!(model.format_version, rex_ir::FORMAT_VERSION);
    // The declared package first, then the synthetic namespace package; the
    // builtin `com.rosetta.model` is absent (only primitives referenced).
    assert_eq!(
        model
            .packages
            .iter()
            .map(|package| package.name.as_str())
            .collect::<Vec<_>>(),
        expected_packages,
        "package emission for {model_path}"
    );

    let json = model.to_json_pretty().expect("serialize IR");

    let artifact = fixture_path(artifact_path);
    if std::env::var("REX_UPDATE_FIXTURES").is_ok() {
        std::fs::create_dir_all(artifact.parent().unwrap()).unwrap();
        std::fs::write(&artifact, &json).unwrap();
        eprintln!("updated {}", artifact.display());
        return;
    }
    let expected = std::fs::read_to_string(&artifact).unwrap_or_else(|_| {
        panic!("missing golden artifact {artifact_path}; regenerate with REX_UPDATE_FIXTURES=1")
    });
    assert_eq!(
        json, expected,
        "IR artifact for {model_path} drifted from the golden file"
    );
}

#[test]
fn golden_artifact_round_trips_through_from_json() {
    for artifact_path in [ARTIFACT_PATH, FUNCS_ARTIFACT_PATH] {
        let expected = std::fs::read_to_string(fixture_path(artifact_path))
            .unwrap_or_else(|error| panic!("read golden artifact {artifact_path}: {error}"));
        let model = rex_ir::Model::from_json(&expected).expect("golden artifact must deserialize");
        assert_eq!(model.format_version, rex_ir::FORMAT_VERSION);
        let reserialized = model.to_json_pretty().expect("re-serialize IR");
        assert_eq!(reserialized, expected, "artifact round trip must be stable");
    }
}
