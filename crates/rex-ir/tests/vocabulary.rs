//! Wire-format tests for vocabulary declarations: `VocabularyDef` on
//! [`Package`], the additive [`TypeRef::Vocabulary`] variant, and the
//! guarantee that non-vocabulary artifacts stay byte-identical.

use std::collections::BTreeMap;

use rex_ir::{
    ClassDef, DefaultValue, Feature, FeatureKind, Multiplicity, Package, PrimitiveType, TypeRef,
    VocabularyDef, VocabularyEntry, VocabularyFacet,
};

const PKG: &str = "nz.example.library";

fn currency_def() -> VocabularyDef {
    VocabularyDef {
        name: "Currency".to_string(),
        source: "iso:4217".to_string(),
        version: Some("2024-01-01".to_string()),
        key: "alpha3".to_string(),
        facets: vec![
            VocabularyFacet {
                name: "minorUnits".to_string(),
                type_: PrimitiveType::Int,
            },
            VocabularyFacet {
                name: "symbol".to_string(),
                type_: PrimitiveType::String,
            },
            VocabularyFacet {
                name: "displayName".to_string(),
                type_: PrimitiveType::String,
            },
        ],
        entries: vec![
            VocabularyEntry {
                key: "USD".to_string(),
                facets: BTreeMap::from([
                    ("minorUnits".to_string(), DefaultValue::Int(2)),
                    ("symbol".to_string(), DefaultValue::String("$".to_string())),
                    (
                        "displayName".to_string(),
                        DefaultValue::String("US Dollar".to_string()),
                    ),
                ]),
            },
            VocabularyEntry {
                key: "EUR".to_string(),
                facets: BTreeMap::from([
                    ("minorUnits".to_string(), DefaultValue::Int(2)),
                    ("symbol".to_string(), DefaultValue::String("€".to_string())),
                    (
                        "displayName".to_string(),
                        DefaultValue::String("Euro".to_string()),
                    ),
                ]),
            },
        ],
    }
}

#[test]
fn vocabulary_def_round_trips_through_json() {
    let mut package = Package::new(PKG);
    package.vocabularies.push(currency_def());

    let json = serde_json::to_string(&package).expect("serialize");
    let parsed: Package = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(package, parsed);
    assert_eq!(parsed.vocabularies[0].entries.len(), 2);
    assert_eq!(parsed.vocabularies[0].entries[0].key, "USD");
    assert_eq!(
        parsed.vocabularies[0].entries[0].facets["symbol"],
        DefaultValue::String("$".to_string())
    );
}

#[test]
fn vocabulary_def_uses_camel_case_wire_names() {
    let json = serde_json::to_value(currency_def()).expect("serialize");
    assert_eq!(json["name"], "Currency");
    assert_eq!(json["source"], "iso:4217");
    assert_eq!(json["version"], "2024-01-01");
    assert_eq!(json["key"], "alpha3");
    assert_eq!(json["facets"][0]["name"], "minorUnits");
    assert_eq!(json["facets"][0]["type"], "int");
    assert_eq!(json["entries"][0]["key"], "USD");
}

#[test]
fn vocabulary_entries_reuse_default_value_tags() {
    let def = currency_def();
    let json = serde_json::to_value(&def.entries).expect("serialize");
    assert_eq!(json[0]["facets"]["minorUnits"], serde_json::json!({ "type": "int", "value": 2 }));
    assert_eq!(
        json[0]["facets"]["symbol"],
        serde_json::json!({ "type": "string", "value": "$" })
    );
}

#[test]
fn type_ref_vocabulary_uses_adjacent_camel_case_tag() {
    let type_ref = TypeRef::Vocabulary {
        package: PKG.to_string(),
        name: "Currency".to_string(),
    };
    let json = serde_json::to_value(&type_ref).expect("serialize");
    assert_eq!(
        json,
        serde_json::json!({
            "type": "vocabulary",
            "value": { "package": PKG, "name": "Currency" }
        })
    );
    let parsed: TypeRef = serde_json::from_value(json).expect("deserialize");
    assert_eq!(parsed, type_ref);
    assert_eq!(
        type_ref.qualified_name().as_deref(),
        Some("nz.example.library::Currency")
    );
}

#[test]
fn package_serializes_vocabularies_in_camel_case() {
    let mut package = Package::new(PKG);
    package.vocabularies.push(currency_def());
    let json = serde_json::to_value(&package).expect("serialize");
    assert_eq!(json["vocabularies"][0]["name"], "Currency");
}

/// The field is omitted entirely when empty so artifacts for models without
/// vocabularies stay byte-identical to pre-vocabulary output (the golden
/// artifacts pin this).
#[test]
fn package_without_vocabularies_omits_the_field() {
    let package = Package::new(PKG);
    let json = serde_json::to_string(&package).expect("serialize");
    assert!(!json.contains("vocabularies"), "json was: {json}");
}

/// Documents the version gate: a v1 artifact carrying the additive
/// `"vocabulary"` tag requires a rex-ir new enough to know the tag, and such
/// an artifact parses fine here.
#[test]
fn from_json_accepts_vocabulary_tagged_artifact() {
    let json = format!(
        r#"{{
          "formatVersion": 1,
          "packages": [
            {{
              "name": "{PKG}",
              "classes": [
                {{
                  "name": "Price",
                  "features": [
                    {{
                      "id": 0,
                      "name": "ccy",
                      "kind": "attribute",
                      "type": {{
                        "type": "vocabulary",
                        "value": {{ "package": "{PKG}", "name": "Currency" }}
                      }},
                      "multiplicity": {{ "lower": 1, "upper": {{ "finite": 1 }} }},
                      "default": {{ "type": "string", "value": "USD" }}
                    }}
                  ]
                }}
              ],
              "vocabularies": [
                {{
                  "name": "Currency",
                  "source": "iso:4217",
                  "version": "2024-01-01",
                  "key": "alpha3",
                  "facets": [{{ "name": "minorUnits", "type": "int" }}],
                  "entries": [
                    {{
                      "key": "USD",
                      "facets": {{ "minorUnits": {{ "type": "int", "value": 2 }} }}
                    }}
                  ]
                }}
              ]
            }}
          ]
        }}"#
    );
    let model = rex_ir::Model::from_json(&json).expect("deserialize vocabulary artifact");
    let package = &model.packages[0];
    assert_eq!(package.vocabularies[0].entries[0].key, "USD");
    let Feature { type_, default, .. } = &package.classes[0].features[0];
    assert_eq!(
        type_,
        &TypeRef::Vocabulary { package: PKG.to_string(), name: "Currency".to_string() }
    );
    assert_eq!(default.as_ref(), Some(&DefaultValue::String("USD".to_string())));
}

/// A `ClassDef::new` construction touching `Feature`/`Multiplicity` keeps the
/// pre-existing shape intact (guards against accidental Eq/serde drift).
#[test]
fn feature_with_vocabulary_type_stays_eq() {
    let feature = Feature::new(
        "ccy",
        FeatureKind::Attribute,
        TypeRef::Vocabulary { package: PKG.to_string(), name: "Currency".to_string() },
        Multiplicity::REQUIRED,
    )
    .with_default(DefaultValue::String("USD".to_string()));
    let class = ClassDef::new("Price", vec![], vec![feature]);
    assert_eq!(class.features[0].default, Some(DefaultValue::String("USD".to_string())));
}
