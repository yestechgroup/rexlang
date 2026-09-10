//! Golden-schema conformance test for the API profile: the flattened DTO
//! projection for the flagship model must byte-match the committed golden
//! file, and its structure must satisfy the API contract.
//!
//! Regenerate with `REX_UPDATE_FIXTURES=1 cargo test -p rex-backend-jsonschema --test golden_api`.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use rex_backend_jsonschema::{generate, Profile};
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

fn library_model() -> rex_ir::Model {
    let source =
        std::fs::read_to_string(fixture_path(MODEL_RELATIVE_PATH)).expect("read conformance model");
    let compilation = compile_str(MODEL_RELATIVE_PATH, &source);
    assert!(
        compilation.diagnostics.is_empty(),
        "conformance model must compile cleanly: {:?}",
        compilation.diagnostics
    );
    compilation.model.expect("model lowered")
}

#[test]
fn library_api_schema_matches_golden() {
    let files = generate(&library_model(), Profile::Api).expect("generate api schema");
    let json = files
        .get("schema.json")
        .expect("single schema.json file")
        .as_str();

    let artifact = fixture_path("tests/conformance/schemas/library.api.schema.json");
    if std::env::var("REX_UPDATE_FIXTURES").is_ok() {
        std::fs::create_dir_all(artifact.parent().unwrap()).unwrap();
        std::fs::write(&artifact, json).unwrap();
        eprintln!("updated {}", artifact.display());
        return;
    }
    let expected = std::fs::read_to_string(&artifact).unwrap_or_else(|_| {
        panic!(
            "missing golden schema {}; regenerate with REX_UPDATE_FIXTURES=1",
            artifact.display()
        )
    });
    assert_eq!(json, expected, "api schema drifted from the golden file");
}

#[test]
fn api_schema_structure_contract() {
    let files = generate(&library_model(), Profile::Api).expect("generate api schema");
    let schema: serde_json::Value =
        serde_json::from_str(files.get("schema.json").expect("schema.json")).expect("valid JSON");

    // Root is a components-style document: exactly $schema, $id, $defs.
    let root_keys: Vec<&str> = schema.as_object().expect("root object").keys().map(String::as_str).collect();
    assert_eq!(root_keys, vec!["$defs", "$id", "$schema"]);
    assert!(schema.get("required").is_none(), "root has no required");
    assert!(schema.get("properties").is_none(), "root has no properties");
    assert!(schema.get("type").is_none(), "root has no type");
    assert_eq!(schema["$id"], "urn:rex:model:nz.example.library:api");
    assert_eq!(
        schema["$schema"],
        "https://json-schema.org/draft/2020-12/schema"
    );

    let defs = &schema["$defs"];
    for name in ["Library", "Book", "Writer"] {
        assert_eq!(defs[name]["type"], "object", "{name} is a typed object def");
    }

    // Book: no envelope keys, cross refs are plain id strings, derived kept
    // as readOnly, numeric formats present, closed for extra keys.
    let book = &defs["Book"];
    assert!(book["properties"].get("$id").is_none(), "no $id on Book");
    assert!(book["properties"].get("$type").is_none(), "no $type on Book (no subclasses)");
    assert!(book["properties"].get("library").is_none(), "container omitted");
    assert_eq!(
        book["properties"]["authors"],
        serde_json::json!({
            "items": {"pattern": "^writer/[0-9]+$", "type": "string"},
            "type": "array"
        }),
        "cross refs are plain id strings (array-wrapped when many)"
    );
    assert_eq!(
        book["properties"]["citation"],
        serde_json::json!({"readOnly": true, "type": "string"}),
        "derived features are present and readOnly"
    );
    assert_eq!(
        book["properties"]["pages"],
        serde_json::json!({"format": "int32", "type": "integer"}),
    );
    let book_required = book["required"].as_array().expect("book required");
    assert!(book_required.iter().any(|k| k == "pages"));
    assert!(!book_required.iter().any(|k| k == "citation"), "derived is not required");
    assert_eq!(book["additionalProperties"], false);

    // No *Ref helper defs in the API profile.
    assert!(defs.get("WriterRef").is_none(), "no WriterRef def in api profile");
    assert!(defs.get("BookRef").is_none(), "no BookRef def in api profile");

    // Library: containment is still inlined via $ref to the child def.
    let library = &defs["Library"];
    assert_eq!(
        library["properties"]["books"],
        serde_json::json!({"items": {"$ref": "#/$defs/Book"}, "type": "array"})
    );
    assert!(
        library.get("$comment").is_none(),
        "Library has no omitted container or derived features"
    );
    let book_comment = book["$comment"].as_str().expect("api Book $comment");
    assert!(
        book_comment.contains("container 'library'"),
        "api Book notes the omitted container: {book_comment}"
    );
    assert!(
        !book_comment.contains("citation"),
        "api Book keeps derived 'citation'; it is not listed as omitted"
    );

    // Writer: cross refs to Book are plain strings too.
    assert_eq!(
        defs["Writer"]["properties"]["books"],
        serde_json::json!({"items": {"pattern": "^book/[0-9]+$", "type": "string"}, "type": "array"})
    );
    assert_eq!(defs["Writer"]["additionalProperties"], false);
}
