//! Doc-comment (`description`) and constraint emission for both profiles.
//!
//! The driver lowers doc comments into `description` and constraint blocks
//! into `FeatureConstraints`; the backends surface them as JSON Schema
//! keywords: `description` on the property (or class) schema, and
//! `pattern`/`minLength`/`maxLength`/`minimum`/`maximum` merged into the
//! element schema — inside `items` for a many-valued attribute, so the
//! constraints bind the elements, not the collection.
//!
//! The committed conformance goldens stay byte-identical: their models use
//! neither feature, and both only appear when declared.

use rex_backend_jsonschema::{generate, Profile};
use rex_driver::compile_str;

const MODEL: &str = r#"package demo

/// A sellable product.
class Product {
    /// Stock-keeping unit.
    String sku { pattern "[A-Z]{3}-[0-9]{4}" minLength 8 maxLength 8 }
    /// Customer ratings.
    int[] ratings { minimum 1 maximum 5 }
}
"#;

fn product_of(profile: Profile) -> serde_json::Value {
    let compilation = compile_str("descriptions.mox", MODEL);
    assert!(
        compilation.diagnostics.is_empty(),
        "model must compile cleanly: {:?}",
        compilation.diagnostics
    );
    let model = compilation.model.expect("model lowered");
    let files = generate(&model, profile).expect("generate");
    let json = files.get("schema.json").expect("schema.json");
    serde_json::from_str::<serde_json::Value>(json).expect("valid JSON")["$defs"]["Product"].clone()
}

#[test]
fn class_and_property_descriptions_surface_in_both_profiles() {
    for profile in [Profile::Wire, Profile::Api] {
        let product = product_of(profile);
        assert_eq!(
            product["description"].as_str(),
            Some("A sellable product."),
            "class description ({profile:?})"
        );
        assert_eq!(
            product["properties"]["sku"]["description"].as_str(),
            Some("Stock-keeping unit."),
            "property description ({profile:?})"
        );
        assert_eq!(
            product["properties"]["ratings"]["description"].as_str(),
            Some("Customer ratings."),
            "many-valued property description ({profile:?})"
        );
    }
}

#[test]
fn constraints_merge_into_the_element_schema() {
    for profile in [Profile::Wire, Profile::Api] {
        let product = product_of(profile);
        assert_eq!(
            product["properties"]["sku"]["pattern"],
            serde_json::json!("[A-Z]{3}-[0-9]{4}"),
            "pattern ({profile:?})"
        );
        assert_eq!(
            product["properties"]["sku"]["minLength"],
            serde_json::json!(8)
        );
        assert_eq!(
            product["properties"]["sku"]["maxLength"],
            serde_json::json!(8)
        );
        // A many-valued attribute constrains its items, not the collection.
        let ratings_items = &product["properties"]["ratings"]["items"];
        assert_eq!(ratings_items["minimum"], serde_json::json!(1));
        assert_eq!(ratings_items["maximum"], serde_json::json!(5));
        assert!(
            product["properties"]["ratings"].get("minimum").is_none(),
            "bounds must not leak onto the array wrapper"
        );
    }
}

#[test]
fn constraintless_models_emit_no_constraint_or_description_keys() {
    let compilation = compile_str(
        "plain.mox",
        "package demo\n\nclass Person { String name }\n",
    );
    let model = compilation.model.expect("model lowered");
    for profile in [Profile::Wire, Profile::Api] {
        let files = generate(&model, profile).expect("generate");
        let json = files.get("schema.json").expect("schema.json");
        let value = serde_json::from_str::<serde_json::Value>(json).expect("valid JSON");
        let name = &value["$defs"]["Person"]["properties"]["name"];
        assert_eq!(
            name,
            &serde_json::json!({"type": "string"}),
            "plain attribute stays byte-clean ({profile:?})"
        );
        assert!(
            value["$defs"]["Person"].get("description").is_none(),
            "plain class carries no description ({profile:?})"
        );
    }
}

const TYPED_MODEL: &str = r#"package demo

enum Status { Draft = 0 Published = 1 Archived = 2 }

type Email wraps opaque

vocabulary Currency from "iso:4217" {
    version "2024-01-01"
    key alpha3
    facet String alpha3
}

class Order {
    Email address { pattern "[^@]+@[^@]+" minLength 5 }
    Currency currency { minLength 3 maxLength 3 }
    Status status { pattern "[A-Za-z]+" minimum 0 maximum 2 }
}
"#;

/// Constraints admitted on datatype, vocabulary, and enum attributes merge
/// into the element schema exactly as they do for primitives: the datatype's
/// `{"type": "string"}`, the vocabulary's `{"enum": [keys]}`, and the enum's
/// `{"enum": [literals]}` each carry the declared keywords.
#[test]
fn constraints_merge_into_datatype_vocabulary_and_enum_element_schemas() {
    let dir =
        std::env::temp_dir().join(format!("rex-jsonschema-constraints-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let vocab_dir = dir.join("vocab");
    std::fs::create_dir_all(&vocab_dir).unwrap();
    std::fs::write(
        vocab_dir.join("iso-4217@2024-01-01.json"),
        r#"{"vocabulary":"iso:4217","version":"2024-01-01","entries":[{"alpha3":"USD"},{"alpha3":"EUR"}]}"#,
    )
    .unwrap();
    let model_path = dir.join("constraints.mox");
    std::fs::write(&model_path, TYPED_MODEL).unwrap();
    let compilation = compile_str(model_path.to_str().expect("utf-8 model path"), TYPED_MODEL);
    assert!(
        compilation.diagnostics.iter().all(|d| !d.is_error()),
        "model must compile: {:?}",
        compilation.diagnostics
    );
    let model = compilation.model.expect("model lowered");
    for profile in [Profile::Wire, Profile::Api] {
        let files = generate(&model, profile).expect("generate");
        let json = files.get("schema.json").expect("schema.json");
        let order =
            &serde_json::from_str::<serde_json::Value>(json).expect("valid JSON")["$defs"]["Order"];

        // Datatype element schema: `type: string` plus the string family.
        let address = &order["properties"]["address"];
        assert_eq!(address["type"], serde_json::json!("string"), "{profile:?}");
        assert_eq!(address["pattern"], serde_json::json!("[^@]+@[^@]+"));
        assert_eq!(address["minLength"], serde_json::json!(5));

        // Vocabulary element schema: the closed key set plus the family.
        let currency = &order["properties"]["currency"];
        assert_eq!(currency["enum"], serde_json::json!(["USD", "EUR"]));
        assert_eq!(currency["minLength"], serde_json::json!(3));
        assert_eq!(currency["maxLength"], serde_json::json!(3));

        // Enum element schema: both families over names and values.
        let status = &order["properties"]["status"];
        assert_eq!(
            status["enum"],
            serde_json::json!(["Draft", "Published", "Archived"])
        );
        assert_eq!(status["pattern"], serde_json::json!("[A-Za-z]+"));
        assert_eq!(status["minimum"], serde_json::json!(0));
        assert_eq!(status["maximum"], serde_json::json!(2));
    }
    let _ = std::fs::remove_dir_all(&dir);
}
