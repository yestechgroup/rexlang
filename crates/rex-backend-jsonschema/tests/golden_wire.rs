//! Golden-schema conformance test for the WIRE profile: the schema generated
//! for the flagship model must byte-match the committed golden file, and its
//! structure must satisfy the wire contract.
//!
//! Regenerate with `REX_UPDATE_FIXTURES=1 cargo test -p rex-backend-jsonschema --test golden_wire`.

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

/// Every conformance model and its committed golden wire schema.
const MODELS: &[(&str, &str)] = &[
    (
        "tests/conformance/models/library.mox",
        "tests/conformance/schemas/library.wire.schema.json",
    ),
    (
        "tests/conformance/models/currency.mox",
        "tests/conformance/schemas/currency.wire.schema.json",
    ),
];

#[test]
fn conformance_wire_schemas_match_golden() {
    for (model_path, artifact_path) in MODELS {
        let files =
            generate(&conformance_model(model_path), Profile::Wire).expect("generate wire schema");
        let json = files
            .get("schema.json")
            .expect("single schema.json file")
            .as_str();

        let artifact = fixture_path(artifact_path);
        if std::env::var("REX_UPDATE_FIXTURES").is_ok() {
            std::fs::create_dir_all(artifact.parent().unwrap()).unwrap();
            std::fs::write(&artifact, json).unwrap();
            eprintln!("updated {}", artifact.display());
            continue;
        }
        let expected = std::fs::read_to_string(&artifact).unwrap_or_else(|_| {
            panic!(
                "missing golden schema {}; regenerate with REX_UPDATE_FIXTURES=1",
                artifact.display()
            )
        });
        assert_eq!(
            json, expected,
            "wire schema for {model_path} drifted from the golden file"
        );
    }
}

/// Compiles one of the conformance models by its workspace-relative path.
fn conformance_model(relative_path: &str) -> rex_ir::Model {
    let path = fixture_path(relative_path);
    let source = std::fs::read_to_string(&path).expect("read conformance model");
    let compilation = compile_str(path.to_str().expect("utf-8 path"), &source);
    assert!(
        compilation.diagnostics.is_empty(),
        "conformance model {relative_path} must compile cleanly: {:?}",
        compilation.diagnostics
    );
    compilation.model.expect("model lowered")
}

#[test]
fn generate_returns_exactly_one_schema_file() {
    for (model_path, _) in MODELS {
        let files =
            generate(&conformance_model(model_path), Profile::Wire).expect("generate wire schema");
        assert_eq!(files.len(), 1, "the wire profile emits a single file");
    }
}

/// Vocabulary attributes enumerate the vendored entry keys in both profiles.
#[test]
fn vocabulary_attribute_enumerates_entry_keys() {
    for profile in [Profile::Wire, Profile::Api] {
        let files = generate(
            &conformance_model("tests/conformance/models/currency.mox"),
            profile,
        )
        .expect("generate currency schema");
        let schema: serde_json::Value =
            serde_json::from_str(files.get("schema.json").expect("schema.json"))
                .expect("valid JSON");
        let currency = &schema["$defs"]["Account"]["properties"]["currency"];
        assert_eq!(
            currency["enum"],
            serde_json::json!(["USD", "EUR", "JPY", "GBP", "CHF"]),
            "{profile:?}: vocabulary attribute enumerates entry keys"
        );
        let comment = currency["$comment"].as_str().expect("$comment");
        assert_eq!(
            comment, "vocabulary iso:4217@2024-01-01 with 5 entries",
            "{profile:?}: $comment names source, version, and entry count"
        );
    }
}

#[test]
fn wire_schema_structure_contract() {
    let files = generate(&library_model(), Profile::Wire).expect("generate wire schema");
    let schema: serde_json::Value =
        serde_json::from_str(files.get("schema.json").expect("schema.json")).expect("valid JSON");

    // Root document: draft, id, type, closure.
    assert_eq!(
        schema["$schema"],
        "https://json-schema.org/draft/2020-12/schema"
    );
    assert_eq!(schema["$id"], "urn:rex:model:nz.example.library:wire");
    assert_eq!(schema["type"], "object");
    assert_eq!(schema["unevaluatedProperties"], false);

    // Root required covers all four document keys.
    let required = schema["required"].as_array().expect("root required array");
    for key in ["$type", "formatVersion", "model", "objects"] {
        assert!(
            required.iter().any(|k| k == key),
            "root required must contain {key}"
        );
    }

    // Root properties pin the document envelope.
    assert_eq!(schema["properties"]["$type"]["const"], "rex.instance");
    assert_eq!(schema["properties"]["formatVersion"]["const"], 1);
    assert_eq!(schema["properties"]["model"]["const"], "nz.example.library");

    // objects items are an anyOf over all class refs, each resolving to $defs.
    let any_of = schema["properties"]["objects"]["items"]["anyOf"]
        .as_array()
        .expect("objects items anyOf");
    assert_eq!(any_of.len(), 3, "one anyOf arm per class");
    let defs = &schema["$defs"];
    for arm in any_of {
        let reference = arm["$ref"].as_str().expect("anyOf arm is a $ref");
        let name = reference
            .strip_prefix("#/$defs/")
            .expect("anyOf arm targets #/$defs/");
        assert!(
            defs.get(name).is_some(),
            "ref target '{name}' exists in $defs"
        );
    }

    // Book def: closed, typed, ids patterned, container/derived omitted.
    let book = &defs["Book"];
    assert_eq!(book["type"], "object");
    assert_eq!(book["properties"]["$type"]["const"], "Book");
    assert_eq!(book["properties"]["$id"]["pattern"], "^book/[0-9]+$");
    assert!(
        book["properties"].get("library").is_none(),
        "container omitted"
    );
    assert!(
        book["properties"].get("citation").is_none(),
        "derived omitted"
    );
    assert_eq!(
        book["properties"]["pages"],
        serde_json::json!({"type": "integer"})
    );
    assert_eq!(
        book["properties"]["category"],
        serde_json::json!({"enum": ["Mystery", "ScienceFiction"]})
    );
    assert_eq!(book["properties"]["copyright"]["type"], "string");
    let copyright_comment = book["properties"]["copyright"]["$comment"]
        .as_str()
        .expect("datatype $comment");
    assert!(
        copyright_comment.starts_with("datatype Date (opaque); target bindings: "),
        "comment: {copyright_comment}"
    );
    assert!(
        copyright_comment.contains("rust → chrono::NaiveDate"),
        "comment: {copyright_comment}"
    );
    assert_eq!(
        book["properties"]["authors"],
        serde_json::json!({"items": {"$ref": "#/$defs/WriterRef"}, "type": "array"})
    );
    let book_required = book["required"].as_array().expect("book required");
    for key in ["$id", "$type", "pages", "title", "category", "copyright"] {
        assert!(
            book_required.iter().any(|k| k == key),
            "Book required has {key}"
        );
    }
    assert!(
        !book_required.iter().any(|k| k == "authors"),
        "Book required must NOT have the many cross ref 'authors'"
    );
    assert_eq!(book["unevaluatedProperties"], false);
    let book_comment = book["$comment"].as_str().expect("book $comment");
    assert!(
        book_comment.contains("container 'library'"),
        "comment: {book_comment}"
    );
    assert!(
        book_comment.contains("derived 'citation'"),
        "comment: {book_comment}"
    );

    // Library def: closed, containment inlined, many containment required.
    let library = &defs["Library"];
    assert_eq!(
        library["properties"]["books"],
        serde_json::json!({"items": {"$ref": "#/$defs/Book"}, "type": "array"})
    );
    let library_required = library["required"].as_array().expect("library required");
    assert!(library_required.iter().any(|k| k == "books"));
    assert!(
        library_required.iter().any(|k| k == "name"),
        "name is 1..1 in the IR"
    );
    assert!(!library_required.iter().any(|k| k == "$comment"));
    assert_eq!(library["properties"]["$id"]["pattern"], "^library/[0-9]+$");
    assert_eq!(library["unevaluatedProperties"], false);

    // Writer def: closed, cross refs wrapped in Ref objects.
    let writer = &defs["Writer"];
    assert_eq!(
        writer["properties"]["books"],
        serde_json::json!({"items": {"$ref": "#/$defs/BookRef"}, "type": "array"})
    );
    assert_eq!(writer["unevaluatedProperties"], false);

    // WriterRef def: exactly one $ref key, required, closed, patterned.
    let writer_ref = &defs["WriterRef"];
    assert_eq!(
        writer_ref["properties"]["$ref"],
        serde_json::json!({"pattern": "^writer/[0-9]+$", "type": "string"})
    );
    let ref_required = writer_ref["required"]
        .as_array()
        .expect("WriterRef required");
    assert!(ref_required.iter().any(|k| k == "$ref"));
    assert_eq!(writer_ref["unevaluatedProperties"], false);
    assert_eq!(
        defs["BookRef"]["properties"]["$ref"]["pattern"], "^book/[0-9]+$",
        "BookRef def exists for Writer.books"
    );
}
