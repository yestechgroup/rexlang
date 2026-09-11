//! Golden conformance test: the Cedar output for the conformance actors
//! model must byte-match the committed goldens. The golden files are the
//! spec for the emitted policy text and schema JSON.
//!
//! Regenerate with `REX_UPDATE_FIXTURES=1 cargo test -p rex-backend-cedar --test golden`.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use rex_backend_cedar::generate;
use rex_driver::compile_str;

/// The conformance model and its committed golden artifacts.
const MODEL_RELATIVE_PATH: &str = "tests/conformance/models/actors.mox";
const GOLDENS: &[&str] = &[
    "tests/conformance/cedar/Support.cedar",
    "tests/conformance/cedar/Support.cedarschema.json",
];

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

fn actors_model() -> rex_ir::Model {
    let path = fixture_path(MODEL_RELATIVE_PATH);
    let source = std::fs::read_to_string(&path).expect("read conformance model");
    let compilation = compile_str(path.to_str().expect("utf-8 path"), &source);
    assert!(
        compilation.diagnostics.is_empty(),
        "conformance model must compile cleanly: {:?}",
        compilation.diagnostics
    );
    compilation.model.expect("model lowered")
}

#[test]
fn conformance_cedar_output_matches_golden() {
    let files = generate(&actors_model()).expect("generate cedar output");
    for golden in GOLDENS {
        let file_name = Path::new(golden)
            .file_name()
            .expect("golden file name")
            .to_str()
            .expect("utf-8 file name");
        let contents = files
            .get(file_name)
            .unwrap_or_else(|| panic!("generated output must contain {file_name}"));
        let artifact = fixture_path(golden);
        if std::env::var("REX_UPDATE_FIXTURES").is_ok() {
            std::fs::create_dir_all(artifact.parent().unwrap()).unwrap();
            std::fs::write(&artifact, contents).unwrap();
            eprintln!("updated {}", artifact.display());
            continue;
        }
        let expected = std::fs::read_to_string(&artifact).unwrap_or_else(|_| {
            panic!(
                "missing golden {}; regenerate with REX_UPDATE_FIXTURES=1",
                artifact.display()
            )
        });
        assert_eq!(
            contents.as_str(),
            expected.as_str(),
            "Cedar output for {file_name} drifted from the golden file"
        );
    }
}

/// A model without `actors` blocks generates no files at all.
#[test]
fn actorless_model_generates_nothing() {
    let source = "package demo\n\nclass Book { String title }\n";
    let compilation = compile_str("bookless.mox", source);
    assert!(compilation.diagnostics.is_empty());
    let model = compilation.model.expect("model lowered");
    let files = generate(&model).expect("generate");
    assert!(files.is_empty(), "got: {:?}", files.keys());
}
