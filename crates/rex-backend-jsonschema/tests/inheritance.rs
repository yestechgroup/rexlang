//! Inheritance encoding tests: `extends` must become schema structure
//! (`allOf` + `$type` enum on the base) with closure only on the outermost
//! (leaf) subclass, and instances must validate accordingly.

use rex_backend_jsonschema::{generate, Profile};
use rex_ir::{
    ClassDef, Feature, FeatureKind, Model, Multiplicity, Package, PrimitiveType, TypeRef,
};

const PACKAGE: &str = "nz.example.zoo";

fn class_ref(name: &str) -> TypeRef {
    TypeRef::Class {
        package: PACKAGE.to_string(),
        name: name.to_string(),
    }
}

/// `class Animal { required String name }` / `class Dog extends Animal { int barkVolume }`
fn animal_dog_model() -> Model {
    let mut model = Model::new();
    let mut package = Package::new(PACKAGE);
    package.classes.push(ClassDef::new(
        "Animal",
        vec![],
        vec![Feature::new(
            "name",
            FeatureKind::Attribute,
            TypeRef::Primitive(PrimitiveType::String),
            Multiplicity::REQUIRED,
        )],
    ));
    package.classes.push(ClassDef::new(
        "Dog",
        vec![class_ref("Animal")],
        vec![Feature::new(
            "barkVolume",
            FeatureKind::Attribute,
            TypeRef::Primitive(PrimitiveType::Int),
            Multiplicity::REQUIRED,
        )],
    ));
    model.packages.push(package);
    model
}

fn wire_schema() -> serde_json::Value {
    let files = generate(&animal_dog_model(), Profile::Wire).expect("generate wire schema");
    serde_json::from_str(files.get("schema.json").expect("schema.json")).expect("valid JSON")
}

fn dog_def_validator(schema: &serde_json::Value) -> jsonschema::Validator {
    // Compile a schema that pins exactly Dog's def; the full $defs space is
    // embedded so the allOf `$ref` to Animal resolves.
    let pinned = serde_json::json!({
        "allOf": [{"$ref": "#/$defs/Dog"}],
        "unevaluatedProperties": false,
        "$defs": schema["$defs"],
    });
    jsonschema::validator_for(&pinned).expect("Dog def compiles")
}

fn dog_instance() -> serde_json::Value {
    serde_json::json!({
        "$type": "rex.instance",
        "formatVersion": 1,
        "model": PACKAGE,
        "objects": [
            {"$id": "dog/0", "$type": "Dog", "name": "Rex", "barkVolume": 3}
        ]
    })
}

#[test]
fn schema_encodes_inheritance_with_outermost_only_closure() {
    let schema = wire_schema();
    let defs = &schema["$defs"];

    // Base class: open (no closure keyword), $type is an enum over the whole
    // (transitive) subclass closure instead of a const.
    let animal = &defs["Animal"];
    assert!(
        animal.get("unevaluatedProperties").is_none(),
        "the extended base must not close the object"
    );
    assert_eq!(
        animal["properties"]["$type"],
        serde_json::json!({"enum": ["Animal", "Dog"]}),
        "base $type enumerates itself plus all transitive subclasses"
    );
    let animal_required = animal["required"].as_array().expect("Animal required");
    assert!(animal_required.iter().any(|k| k == "name"));

    // Subclass: allOf the base, const $type, closed only because it is a leaf.
    let dog = &defs["Dog"];
    assert_eq!(
        dog["allOf"],
        serde_json::json!([{"$ref": "#/$defs/Animal"}])
    );
    assert_eq!(
        dog["properties"]["$type"],
        serde_json::json!({"const": "Dog"})
    );
    assert_eq!(dog["properties"]["barkVolume"]["type"], "integer");
    assert_eq!(dog["unevaluatedProperties"], false);
    let dog_required = dog["required"].as_array().expect("Dog required");
    assert!(dog_required.iter().any(|k| k == "barkVolume"));
}

#[test]
fn dog_instance_validates_against_document_root() {
    let schema = wire_schema();
    let validator = jsonschema::validator_for(&schema).expect("schema compiles");
    assert!(
        validator.is_valid(&dog_instance()),
        "a Dog instance with base + own properties must validate against the document root"
    );
}

#[test]
fn dog_instance_validates_against_dog_def() {
    let schema = wire_schema();
    let validator = dog_def_validator(&schema);
    let dog = serde_json::json!({"$id": "dog/0", "$type": "Dog", "name": "Rex", "barkVolume": 3});
    assert!(validator.is_valid(&dog), "Dog def accepts a Dog instance");
}

#[test]
fn dog_instance_with_unknown_extra_property_fails() {
    let schema = wire_schema();
    let validator = dog_def_validator(&schema);
    let dog = serde_json::json!(
        {"$id": "dog/0", "$type": "Dog", "name": "Rex", "barkVolume": 3, "meows": true}
    );
    assert!(
        !validator.is_valid(&dog),
        "an unknown extra property on the subclass must fail"
    );
}

#[test]
fn inherited_required_property_is_enforced() {
    let schema = wire_schema();
    let document_validator = jsonschema::validator_for(&schema).expect("schema compiles");
    let mut instance = dog_instance();
    mutate_object(&mut instance, "dog/0", |dog| {
        dog.remove("name");
    });
    assert!(
        !document_validator.is_valid(&instance),
        "the inherited required 'name' must still be enforced via allOf"
    );

    let def_validator = dog_def_validator(&schema);
    let dog = serde_json::json!({"$id": "dog/0", "$type": "Dog", "barkVolume": 3});
    assert!(!def_validator.is_valid(&dog));
}

fn mutate_object(
    instance: &mut serde_json::Value,
    id: &str,
    mutate: impl FnOnce(&mut serde_json::Map<String, serde_json::Value>),
) {
    for object in instance["objects"].as_array_mut().expect("objects array") {
        if object["$id"] == *id {
            mutate(object.as_object_mut().expect("object is a map"));
            return;
        }
    }
    panic!("object with $id {id:?} not found");
}
