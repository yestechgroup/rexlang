use std::collections::BTreeMap;

use super::*;

const PKG: &str = "nz.example.library";

fn class_ref(name: &str) -> TypeRef {
    TypeRef::Class {
        package: PKG.to_string(),
        name: name.to_string(),
    }
}

fn library_model() -> Model {
    let mut model = Model::new();
    let mut package = Package::new(PKG);
    package.annotations.push(Annotation {
        source: "urn:rex:example".to_string(),
        details: BTreeMap::from([("author".to_string(), "rex".to_string())]),
    });
    package.enums.push(EnumDef::new(
        "BookCategory",
        vec![
            EnumLiteral::new("Mystery", Some("M".to_string()), 0),
            EnumLiteral::new("ScienceFiction", Some("S".to_string()), 1),
        ],
    ));
    package.datatypes.push(
        DatatypeDef::new("Date", None)
            .bind("rust", "chrono::NaiveDate")
            .bind("csharp", "System.DateOnly")
            .bind("java", "java.time.LocalDate"),
    );
    package
        .interfaces
        .push(InterfaceDef::new("Lendable").bind("rust", "traits::Lendable"));
    package.classes.push(ClassDef::new(
        "Library",
        vec![],
        vec![
            Feature::new(
                "name",
                FeatureKind::Attribute,
                TypeRef::Primitive(PrimitiveType::String),
                Multiplicity::OPTIONAL,
            )
            .with_default(DefaultValue::String("The Library".to_string())),
            Feature::new(
                "books",
                FeatureKind::Containment,
                class_ref("Book"),
                Multiplicity::MANY,
            )
            .with_opposite("Book", "library"),
        ],
    ));
    package.classes.push(ClassDef::new(
        "Book",
        vec![TypeRef::Interface {
            package: PKG.to_string(),
            name: "Lendable".to_string(),
        }],
        vec![
            Feature::new(
                "library",
                FeatureKind::Container,
                class_ref("Library"),
                Multiplicity::REQUIRED,
            )
            .with_opposite("Library", "books"),
            Feature::new(
                "title",
                FeatureKind::Attribute,
                TypeRef::Primitive(PrimitiveType::String),
                Multiplicity::OPTIONAL,
            ),
            Feature::new(
                "pages",
                FeatureKind::Attribute,
                TypeRef::Primitive(PrimitiveType::Int),
                Multiplicity::REQUIRED,
            ),
            Feature::new(
                "copyright",
                FeatureKind::Attribute,
                TypeRef::Datatype {
                    package: PKG.to_string(),
                    name: "Date".to_string(),
                },
                Multiplicity::OPTIONAL,
            ),
            Feature::new(
                "category",
                FeatureKind::Attribute,
                TypeRef::Enum {
                    package: PKG.to_string(),
                    name: "BookCategory".to_string(),
                },
                Multiplicity::OPTIONAL,
            )
            .with_default(DefaultValue::EnumLiteral("Mystery".to_string())),
            Feature::new(
                "authors",
                FeatureKind::CrossReference,
                class_ref("Writer"),
                Multiplicity::MANY,
            )
            .with_opposite("Writer", "books"),
            Feature::new(
                "citation",
                FeatureKind::Attribute,
                TypeRef::Primitive(PrimitiveType::String),
                Multiplicity::OPTIONAL,
            )
            .derived(),
        ],
    ));
    package.classes.push(ClassDef::new(
        "Writer",
        vec![],
        vec![
            Feature::new(
                "name",
                FeatureKind::Attribute,
                TypeRef::Primitive(PrimitiveType::String),
                Multiplicity::OPTIONAL,
            ),
            Feature::new(
                "books",
                FeatureKind::CrossReference,
                class_ref("Book"),
                Multiplicity::MANY,
            )
            .with_opposite("Book", "authors"),
        ],
    ));
    model.packages.push(package);
    model
}

/// The shared wire-contract characterization (see
/// `wire::tests::assert_wire_contract`): the empty model serializes as
/// exactly the canonical bytes — the version marker, the crate
/// `rexVersion`, and the always-present `packages` array — and the
/// populated library model round-trips byte-stably through both
/// serializers.
#[test]
fn model_wire_contract_holds_through_the_shared_helper() {
    crate::wire::tests::assert_wire_contract(
        r#"{"formatVersion":1,"rexVersion":"0.1.0","packages":[]}"#,
        &library_model(),
    );
}

#[test]
fn round_trip_library_model() {
    let model = library_model();
    let json = model.to_json().expect("serialize");
    let parsed = Model::from_json(&json).expect("deserialize");
    assert_eq!(model, parsed);

    let package = &parsed.packages[0];
    assert_eq!(package.name, PKG);
    assert_eq!(package.enums[0].literals[0].label.as_deref(), Some("M"));
    assert_eq!(
        package.datatypes[0].target_bindings["rust"],
        "chrono::NaiveDate"
    );

    for class in &package.classes {
        for (index, feature) in class.features.iter().enumerate() {
            assert_eq!(feature.id, index as u32, "class {}", class.name);
        }
    }
    assert_eq!(
        package.classes[0].feature(1).map(|f| f.name.as_str()),
        Some("books")
    );
    assert_eq!(
        package.classes[1]
            .feature(0)
            .and_then(|f| f.opposite.clone()),
        Some(OppositeRef {
            class: "Library".to_string(),
            feature: "books".to_string(),
        })
    );
    assert_eq!(package.classes[1].features.len(), 7);
    assert!(package.classes[1].features[6].is_derived);
}

#[test]
fn golden_json_matches_expected_wire_format() {
    let mut package = Package::new("nz.example.demo");
    package.classes.push(ClassDef::new(
        "Person",
        vec![],
        vec![Feature::new(
            "fullName",
            FeatureKind::Attribute,
            TypeRef::Primitive(PrimitiveType::String),
            Multiplicity::REQUIRED,
        )],
    ));
    let mut model = Model::new();
    model.packages.push(package);

    let expected = r#"{
  "formatVersion": 1,
  "rexVersion": "0.1.0",
  "packages": [
    {
      "name": "nz.example.demo",
      "annotations": [],
      "enums": [],
      "datatypes": [],
      "interfaces": [],
      "classes": [
        {
          "name": "Person",
          "extends": [],
          "features": [
            {
              "id": 0,
              "name": "fullName",
              "kind": "attribute",
              "type": {
                "type": "primitive",
                "value": "string"
              },
              "multiplicity": {
                "lower": 1,
                "upper": {
                  "finite": 1
                }
              },
              "isDerived": false,
              "isId": false,
              "isReadOnly": false
            }
          ]
        }
      ]
    }
  ]
}"#;

    assert_eq!(model.to_json_pretty().expect("serialize"), expected);
    let parsed = Model::from_json(expected).expect("deserialize golden");
    assert_eq!(model, parsed);
}

#[test]
fn golden_json_with_bodies_pins_the_tier_1_wire_format() {
    // Additive (wire-contract rule 5): body-less models stay byte-identical
    // (see `golden_json_matches_expected_wire_format`); this golden pins the
    // NEW fields — `operations[].bodies`, datatype `create`/`convert` —
    // which are omitted when empty.
    let mut package = Package::new("nz.example.demo");
    package.datatypes.push(
        DatatypeDef::new("Date", None)
            .with_body("create", "rust", "Date(it)".to_string())
            .with_body("convert", "rust", "self.0.clone()".to_string()),
    );
    package
        .classes
        .push(ClassDef::new("Person", vec![], vec![]));
    package.classes[0].operations.push(
        Operation::new(
            "getBook",
            TypeRef::Class {
                package: "nz.example.demo".to_string(),
                name: "Book".to_string(),
            },
            vec![OperationParam {
                name: "title".to_string(),
                type_: TypeRef::Primitive(PrimitiveType::String),
                multiplicity: None,
            }],
        )
        .with_body(
            "rust",
            "res.books.iter().find_map(|b| { (b.title == title).then_some(*b) })",
        ),
    );
    let mut model = Model::new();
    model.packages.push(package);

    let expected = r#"{
  "formatVersion": 1,
  "rexVersion": "0.1.0",
  "packages": [
    {
      "name": "nz.example.demo",
      "annotations": [],
      "enums": [],
      "datatypes": [
        {
          "name": "Date",
          "create": {
            "rust": "Date(it)"
          },
          "convert": {
            "rust": "self.0.clone()"
          }
        }
      ],
      "interfaces": [],
      "classes": [
        {
          "name": "Person",
          "extends": [],
          "features": [],
          "operations": [
            {
              "name": "getBook",
              "returnType": {
                "type": "class",
                "value": {
                  "package": "nz.example.demo",
                  "name": "Book"
                }
              },
              "params": [
                {
                  "name": "title",
                  "type": {
                    "type": "primitive",
                    "value": "string"
                  }
                }
              ],
              "bodies": {
                "rust": "res.books.iter().find_map(|b| { (b.title == title).then_some(*b) })"
              }
            }
          ]
        }
      ]
    }
  ]
}"#;

    assert_eq!(model.to_json_pretty().expect("serialize"), expected);
    let parsed = Model::from_json(expected).expect("deserialize golden");
    assert_eq!(model, parsed);
}

#[test]
fn golden_json_with_functions_pins_the_wire_format() {
    // Additive (wire-contract rule 15): function-less models stay
    // byte-identical (see `golden_json_matches_expected_wire_format`);
    // this golden pins the NEW field — `functions`, with the one
    // non-codegen body target `"rosetta"` — which is omitted when empty.
    let mut package = Package::new("nz.example.demo");
    package.functions.push(
            Operation::new(
                "calcRate",
                TypeRef::Primitive(PrimitiveType::Double),
                vec![OperationParam {
                    name: "notional".to_string(),
                    type_: TypeRef::Primitive(PrimitiveType::Double),
                    multiplicity: None,
                }],
            )
            .with_body("rosetta", "func calcRate:\n\tinputs:\n\t\tnotional number (1..1)\n\toutput:\n\t\trate number (1..1)"),
        );
    let mut model = Model::new();
    model.packages.push(package);

    let expected = r#"{
  "formatVersion": 1,
  "rexVersion": "0.1.0",
  "packages": [
    {
      "name": "nz.example.demo",
      "annotations": [],
      "enums": [],
      "datatypes": [],
      "interfaces": [],
      "classes": [],
      "functions": [
        {
          "name": "calcRate",
          "returnType": {
            "type": "primitive",
            "value": "double"
          },
          "params": [
            {
              "name": "notional",
              "type": {
                "type": "primitive",
                "value": "double"
              }
            }
          ],
          "bodies": {
            "rosetta": "func calcRate:\n\tinputs:\n\t\tnotional number (1..1)\n\toutput:\n\t\trate number (1..1)"
          }
        }
      ]
    }
  ]
}"#;

    assert_eq!(model.to_json_pretty().expect("serialize"), expected);
    let parsed = Model::from_json(expected).expect("deserialize golden");
    assert_eq!(model, parsed);
    // The empty case: a function-less package never carries the key —
    // byte-identical to pre-rule-15 output.
    let mut plain = Model::new();
    plain.packages.push(Package::new("p"));
    assert_eq!(
        plain.to_json().expect("serialize"),
        r#"{"formatVersion":1,"rexVersion":"0.1.0","packages":[{"name":"p","annotations":[],"enums":[],"datatypes":[],"interfaces":[],"classes":[]}]}"#
    );
}

#[test]
fn golden_json_with_function_return_multiplicity_pins_the_wire_format() {
    // Additive (wire-contract rule 15): operations without a return
    // multiplicity stay byte-identical (see
    // `an_operation_without_return_multiplicity_stays_byte_identical`);
    // this golden pins the NEW field — `returnMultiplicity`, lowered
    // from a Rune output cardinality — which is omitted when `None`.
    let mut package = Package::new("nz.example.demo");
    let mut function = Operation::new(
        "legRates",
        TypeRef::Primitive(PrimitiveType::Double),
        vec![OperationParam {
            name: "legId".to_string(),
            type_: TypeRef::Primitive(PrimitiveType::String),
            multiplicity: None,
        }],
    );
    function.return_multiplicity = Some(Multiplicity::MANY);
    package.functions.push(function);
    let mut model = Model::new();
    model.packages.push(package);

    let expected = r#"{
  "formatVersion": 1,
  "rexVersion": "0.1.0",
  "packages": [
    {
      "name": "nz.example.demo",
      "annotations": [],
      "enums": [],
      "datatypes": [],
      "interfaces": [],
      "classes": [],
      "functions": [
        {
          "name": "legRates",
          "returnType": {
            "type": "primitive",
            "value": "double"
          },
          "returnMultiplicity": {
            "lower": 0,
            "upper": "unbounded"
          },
          "params": [
            {
              "name": "legId",
              "type": {
                "type": "primitive",
                "value": "string"
              }
            }
          ]
        }
      ]
    }
  ]
}"#;

    assert_eq!(model.to_json_pretty().expect("serialize"), expected);
    let parsed = Model::from_json(expected).expect("deserialize golden");
    assert_eq!(model, parsed);
}

#[test]
fn an_operation_without_return_multiplicity_stays_byte_identical() {
    // The empty case of the return-multiplicity field: an operation
    // whose return type carries no cardinality never carries the key —
    // byte-identical to pre-field output (wire-contract rule 5).
    let mut package = Package::new("p");
    package.functions.push(Operation::new(
        "calcRate",
        TypeRef::Primitive(PrimitiveType::Double),
        vec![],
    ));
    let mut model = Model::new();
    model.packages.push(package);
    assert_eq!(
        model.to_json().expect("serialize"),
        r#"{"formatVersion":1,"rexVersion":"0.1.0","packages":[{"name":"p","annotations":[],"enums":[],"datatypes":[],"interfaces":[],"classes":[],"functions":[{"name":"calcRate","returnType":{"type":"primitive","value":"double"},"params":[]}]}]}"#
    );
    let parsed = Model::from_json(&model.to_json().expect("serialize")).expect("deserialize");
    assert_eq!(model, parsed);
}

#[test]
fn golden_json_with_descriptions_and_constraints_pins_the_wire_format() {
    // Additive (wire-contract rule 11): models without doc comments or
    // constraints stay byte-identical (see
    // `golden_json_matches_expected_wire_format`); this golden pins the
    // NEW fields — `description` and `constraints` — which are omitted
    // when absent.
    let mut package = Package::new("nz.example.demo");
    let mut enum_def = EnumDef::new(
        "BookCategory",
        vec![EnumLiteral::new("Mystery", None, 0).with_description("whodunits".to_string())],
    );
    enum_def.description = Some("How the book is shelved.".to_string());
    package.enums.push(enum_def);
    let mut feature = Feature::new(
        "sku",
        FeatureKind::Attribute,
        TypeRef::Primitive(PrimitiveType::String),
        Multiplicity::REQUIRED,
    )
    .with_description("Stock-keeping unit.".to_string());
    feature.constraints = FeatureConstraints {
        pattern: Some("[A-Z]{3}-[0-9]{4}".to_string()),
        min_length: Some(8),
        max_length: Some(8),
        ..FeatureConstraints::default()
    };
    let mut class = ClassDef::new("Product", vec![], vec![feature]);
    class.description = Some("A sellable product.".to_string());
    package.classes.push(class);
    let mut model = Model::new();
    model.packages.push(package);

    let expected = r#"{
  "formatVersion": 1,
  "rexVersion": "0.1.0",
  "packages": [
    {
      "name": "nz.example.demo",
      "annotations": [],
      "enums": [
        {
          "name": "BookCategory",
          "description": "How the book is shelved.",
          "literals": [
            {
              "name": "Mystery",
              "value": 0,
              "description": "whodunits"
            }
          ]
        }
      ],
      "datatypes": [],
      "interfaces": [],
      "classes": [
        {
          "name": "Product",
          "description": "A sellable product.",
          "extends": [],
          "features": [
            {
              "id": 0,
              "name": "sku",
              "description": "Stock-keeping unit.",
              "kind": "attribute",
              "type": {
                "type": "primitive",
                "value": "string"
              },
              "multiplicity": {
                "lower": 1,
                "upper": {
                  "finite": 1
                }
              },
              "isDerived": false,
              "isId": false,
              "isReadOnly": false,
              "constraints": {
                "pattern": "[A-Z]{3}-[0-9]{4}",
                "minLength": 8,
                "maxLength": 8
              }
            }
          ]
        }
      ]
    }
  ]
}"#;

    assert_eq!(model.to_json_pretty().expect("serialize"), expected);
    let parsed = Model::from_json(expected).expect("deserialize golden");
    assert_eq!(model, parsed);
}

#[test]
fn golden_json_with_datatype_format_pins_the_wire_format() {
    // Additive: `format` is omitted when absent (see the byte-identity
    // goldens above); this golden pins the NEW field — a datatype's
    // declared `format` hint — which is omitted when `None`.
    let mut package = Package::new("nz.example.demo");
    package
        .datatypes
        .push(DatatypeDef::new("Email", Some("String".to_string())).with_format("email"));
    package
        .datatypes
        .push(DatatypeDef::new("Plain", Some("String".to_string())));
    let mut model = Model::new();
    model.packages.push(package);

    let expected = r#"{
  "formatVersion": 1,
  "rexVersion": "0.1.0",
  "packages": [
    {
      "name": "nz.example.demo",
      "annotations": [],
      "enums": [],
      "datatypes": [
        {
          "name": "Email",
          "platform": "String",
          "format": "email"
        },
        {
          "name": "Plain",
          "platform": "String"
        }
      ],
      "interfaces": [],
      "classes": []
    }
  ]
}"#;

    assert_eq!(model.to_json_pretty().expect("serialize"), expected);
    let parsed = Model::from_json(expected).expect("deserialize golden");
    assert_eq!(model, parsed);
}

#[test]
fn golden_json_with_unique_constraint_pins_the_wire_format() {
    // Additive (wire-contract rule 11): the `unique` flag is omitted
    // when `false` (the rule-11 golden above stays byte-identical);
    // this golden pins its serialization when declared.
    let mut package = Package::new("nz.example.demo");
    let mut feature = Feature::new(
        "tags",
        FeatureKind::Attribute,
        TypeRef::Primitive(PrimitiveType::String),
        Multiplicity::MANY,
    );
    feature.constraints = FeatureConstraints {
        unique: true,
        ..FeatureConstraints::default()
    };
    package
        .classes
        .push(ClassDef::new("Article", vec![], vec![feature]));
    let mut model = Model::new();
    model.packages.push(package);

    let expected = r#"{
  "formatVersion": 1,
  "rexVersion": "0.1.0",
  "packages": [
    {
      "name": "nz.example.demo",
      "annotations": [],
      "enums": [],
      "datatypes": [],
      "interfaces": [],
      "classes": [
        {
          "name": "Article",
          "extends": [],
          "features": [
            {
              "id": 0,
              "name": "tags",
              "kind": "attribute",
              "type": {
                "type": "primitive",
                "value": "string"
              },
              "multiplicity": {
                "lower": 0,
                "upper": "unbounded"
              },
              "isDerived": false,
              "isId": false,
              "isReadOnly": false,
              "constraints": {
                "unique": true
              }
            }
          ]
        }
      ]
    }
  ]
}"#;

    assert_eq!(model.to_json_pretty().expect("serialize"), expected);
    let parsed = Model::from_json(expected).expect("deserialize golden");
    assert_eq!(model, parsed);
}

#[test]
fn from_json_rejects_wrong_format_version() {
    let json = library_model().to_json_pretty().expect("serialize");

    let tampered = json.replace("\"formatVersion\": 1", "\"formatVersion\": 2");
    assert!(matches!(
        Model::from_json(&tampered),
        Err(IrError::UnsupportedFormatVersion {
            found: 2,
            expected: 1
        })
    ));

    assert!(matches!(
        Model::from_json("{}"),
        Err(IrError::UnsupportedFormatVersion {
            found: 0,
            expected: 1
        })
    ));
}

#[test]
fn from_json_ignores_unknown_fields_for_forward_compat() {
    let model = library_model();
    let mut json = model.to_json().expect("serialize");
    let at = json.find("\"packages\"").expect("packages key");
    json.insert_str(at, "\"someFutureField\": {\"nested\": true},");
    let parsed = Model::from_json(&json).expect("unknown fields ignored");
    assert_eq!(model, parsed);
    assert!(matches!(
        &parsed.packages[0].annotations[0],
        Annotation { .. }
    ));
}

#[test]
fn multiplicity_helpers() {
    assert!(Multiplicity::OPTIONAL.contains(0));
    assert!(Multiplicity::OPTIONAL.contains(1));
    assert!(!Multiplicity::OPTIONAL.contains(2));
    assert!(!Multiplicity::OPTIONAL.is_many());

    assert!(!Multiplicity::REQUIRED.contains(0));
    assert!(Multiplicity::REQUIRED.contains(1));
    assert!(!Multiplicity::REQUIRED.contains(2));
    assert!(!Multiplicity::REQUIRED.is_many());

    assert!(Multiplicity::MANY.contains(0));
    assert!(Multiplicity::MANY.contains(7));
    assert!(Multiplicity::MANY.is_many());

    let two_to_five = Multiplicity::new(2, Upper::Finite(5));
    assert!(!two_to_five.contains(1));
    assert!(two_to_five.contains(2));
    assert!(two_to_five.contains(5));
    assert!(!two_to_five.contains(6));
    assert!(two_to_five.is_many());
}

#[test]
fn assign_feature_ids_renumbers_in_declaration_order() {
    let mut class = ClassDef::new(
        "Thing",
        vec![],
        vec![
            Feature::new(
                "a",
                FeatureKind::Attribute,
                TypeRef::Primitive(PrimitiveType::Int),
                Multiplicity::REQUIRED,
            ),
            Feature::new(
                "b",
                FeatureKind::Attribute,
                TypeRef::Primitive(PrimitiveType::Int),
                Multiplicity::REQUIRED,
            ),
        ],
    );
    let removed = class.features.remove(0);
    class.features.push(removed);
    class.assign_feature_ids();
    assert_eq!(class.features[0].name, "b");
    assert_eq!(class.features[0].id, 0);
    assert_eq!(class.features[1].name, "a");
    assert_eq!(class.features[1].id, 1);
}

#[test]
fn primitive_type_display_and_numeric() {
    assert_eq!(PrimitiveType::String.to_string(), "string");
    assert_eq!(PrimitiveType::Double.to_string(), "double");
    assert!(PrimitiveType::Int.is_numeric());
    assert!(PrimitiveType::Byte.is_numeric());
    assert!(!PrimitiveType::Boolean.is_numeric());
    assert!(!PrimitiveType::Char.is_numeric());
    assert_eq!(
        serde_json::to_value(PrimitiveType::Boolean).unwrap(),
        serde_json::json!("boolean")
    );
}

#[test]
fn date_primitive_serializes_and_displays() {
    // Wire format (rule 13): camelCase lowercase tag `"date"`, additive to
    // v1 — artifacts for models that use no date attributes stay
    // byte-identical because the tag never appears in them.
    assert_eq!(PrimitiveType::Date.to_string(), "date");
    assert_eq!(
        serde_json::to_value(PrimitiveType::Date).unwrap(),
        serde_json::json!("date")
    );
    assert!(!PrimitiveType::Date.is_numeric());

    // A date-typed feature round-trips through the wire format.
    let mut package = Package::new("nz.example.demo");
    package.classes.push(ClassDef::new(
        "Loan",
        vec![],
        vec![Feature::new(
            "effectiveDate",
            FeatureKind::Attribute,
            TypeRef::Primitive(PrimitiveType::Date),
            Multiplicity::REQUIRED,
        )],
    ));
    let model = Model::new();
    let mut model = model;
    model.packages.push(package);
    let parsed = Model::from_json(&model.to_json().expect("serialize")).expect("deserialize");
    assert_eq!(
        parsed.packages[0].classes[0].features[0].type_,
        TypeRef::Primitive(PrimitiveType::Date)
    );
    assert!(model
        .to_json()
        .expect("serialize")
        .contains("\"value\":\"date\""));
}

#[test]
fn every_artifact_root_derives_eq() {
    // Wire contract rule 14: one Eq policy across all four artifacts —
    // the only floats in any of them are the ordered newtypes, so the
    // roots (and everything nested in them) can and must derive Eq.
    fn assert_eq_type<T: Eq>() {}
    assert_eq_type::<Model>();
    assert_eq_type::<ActorModel>();
    assert_eq_type::<ddd::DddModel>();
    assert_eq_type::<ifml::IfmlModel>();
    // The ordered-float newtypes themselves carry the full package.
    assert_eq_type::<ddd::Boost>();
    assert_eq_type::<ifml::Float>();
    fn assert_ord_type<T: Ord>() {}
    assert_ord_type::<ddd::Boost>();
    assert_ord_type::<ifml::Float>();
}

#[test]
fn register_class_kind_round_trips_through_the_wire_format() {
    // Wire contract rule 16 (issue rexlang#181): a register class carries
    // the additive `kind` marker (`"register"`); entity classes serialize
    // without the field at all, so every pre-register artifact stays
    // byte-identical.
    let mut package = Package::new("nz.example.org");
    package.classes.push(
        ClassDef::new(
            "Role",
            vec![],
            vec![Feature::new(
                "display_name",
                FeatureKind::Attribute,
                TypeRef::Primitive(PrimitiveType::String),
                Multiplicity::REQUIRED,
            )],
        )
        .with_kind(ClassKind::Register),
    );
    let mut model = Model::new();
    model.packages.push(package);

    let json = model.to_json_pretty().expect("serialize");
    assert!(
        json.contains("\"kind\": \"register\""),
        "the register marker must serialize:\n{json}"
    );
    let parsed = Model::from_json(&json).expect("deserialize");
    assert_eq!(parsed.packages[0].classes[0].kind, ClassKind::Register);
    assert_eq!(parsed, model, "serialize → deserialize → equal");

    // The converse: an entity class omits the field entirely, and an
    // artifact written without the marker (an older emitter) loads as an
    // entity class.
    let mut entity = model.clone();
    entity.packages[0].classes[0].kind = ClassKind::Entity;
    let entity_json = entity.to_json_pretty().expect("serialize");
    assert!(
        !entity_json.contains("\"kind\": \"entity\""),
        "entity classes must omit the field:\n{entity_json}"
    );
    let reparsed = Model::from_json(&entity_json).expect("deserialize");
    assert_eq!(reparsed.packages[0].classes[0].kind, ClassKind::Entity);
    assert_eq!(reparsed, entity, "serialize → deserialize → equal");

    // The kind serializes as the camelCase bare tag (wire contract rule 1).
    assert_eq!(
        serde_json::to_value(ClassKind::Register).unwrap(),
        serde_json::json!("register")
    );
    assert_eq!(
        serde_json::to_value(ClassKind::Entity).unwrap(),
        serde_json::json!("entity")
    );
}
