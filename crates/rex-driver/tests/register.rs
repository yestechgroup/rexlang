//! Register-class lowering (issue rexlang#181, part A): a `register`
//! declaration lowers onto the standard `rex_ir::Model` as a plain
//! `ClassDef` — `kind: register`, its attributes as attribute features with
//! stable declaration-order ids — and joins every existing class-level rule
//! unchanged. The marker is additive (wire-contract rule 16): models
//! without register declarations lower to `kind: entity`, which serializes
//! no `kind` field at all, so their artifacts stay byte-identical (the
//! golden-artifact suite pins that byte-for-byte).

use rex_driver::{compile_str, DomainImports};
use rex_ir::ClassKind;

/// Compiles one in-memory `.mox` source.
fn compile(source: &str) -> rex_driver::Compilation {
    compile_str("test.mox", source, &DomainImports::default())
}

#[test]
fn register_class_lowers_with_the_register_marker_and_stable_ids() {
    let source = "package nz.example.org\n\n/// A role in the organisation.\nregister Role {\n    /// The role's display name.\n    String display_name\n    String[0..1] badge = \"none\"\n}\n";
    let compilation = compile(source);
    assert!(
        compilation.diagnostics.is_empty(),
        "got: {:?}",
        compilation.diagnostics
    );
    let model = compilation.model.expect("model lowered");
    let package = &model.packages[0];
    assert_eq!(package.name, "nz.example.org");
    assert_eq!(package.classes.len(), 1);

    let role = &package.classes[0];
    assert_eq!(role.name, "Role");
    assert_eq!(role.kind, ClassKind::Register);
    assert_eq!(
        role.description.as_deref(),
        Some("A role in the organisation.")
    );
    assert_eq!(role.extends, Vec::new(), "register declares no extends");

    // The attributes lower as ordinary attribute features with stable
    // declaration-order ids.
    assert_eq!(role.features.len(), 2);
    assert_eq!(role.features[0].id, 0);
    assert_eq!(role.features[0].name, "display_name");
    assert_eq!(role.features[0].kind, rex_ir::FeatureKind::Attribute);
    assert_eq!(
        role.features[0].description.as_deref(),
        Some("The role's display name.")
    );
    assert_eq!(role.features[1].id, 1);
    assert_eq!(role.features[1].name, "badge");
    assert_eq!(
        role.features[1].multiplicity,
        rex_ir::Multiplicity::OPTIONAL
    );
    assert_eq!(
        role.features[1].default,
        Some(rex_ir::DefaultValue::String("none".to_string()))
    );
}

#[test]
fn register_class_joins_the_stripped_name_uniqueness_rule() {
    // A register class and a class stripping to the same module identifier
    // collide exactly like two classes would (E0300, both named).
    let source = "package nz.example.org\n\nclass RoleType {\n    String name\n}\n\nregister Role {\n    String display_name\n}\n";
    let compilation = compile(source);
    let errors: Vec<_> = compilation
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.is_error())
        .collect();
    assert_eq!(errors.len(), 1, "got: {:?}", compilation.diagnostics);
    assert!(
        errors[0].message.contains("E0300"),
        "got: {}",
        errors[0].message
    );
    assert!(
        errors[0].message.contains("'RoleType'") && errors[0].message.contains("'Role'"),
        "both declarations must be named: {}",
        errors[0].message
    );
}

#[test]
fn plain_classes_lower_with_the_entity_kind_and_omit_the_marker() {
    // Strictly additive: a model without `register` declarations lowers to
    // `kind: entity` everywhere and serializes no `kind` field at all.
    let source = "package nz.example.org\n\nclass Person {\n    id String email\n    String display_name\n}\n\nenum Kind { A = 0 }\n";
    let compilation = compile(source);
    assert!(
        compilation.diagnostics.is_empty(),
        "got: {:?}",
        compilation.diagnostics
    );
    let model = compilation.model.expect("model lowered");
    for class in &model.packages[0].classes {
        assert_eq!(class.kind, ClassKind::Entity, "class '{}'", class.name);
    }
    let json = model.to_json_pretty().expect("serialize");
    assert!(
        !json.contains("\"kind\": \"entity\""),
        "entity classes must omit the field entirely:\n{json}"
    );
}

#[test]
fn register_class_resolves_as_a_class_for_references() {
    // Register classes join the package namespace as classes: `refers`
    // targets resolve, and the lowered TypeRef names the register class.
    let source = "package nz.example.org\n\nregister Role {\n    String display_name\n}\n\nclass Person {\n    id String email\n    refers Role[0..*] roles\n}\n";
    let compilation = compile(source);
    assert!(
        compilation.diagnostics.is_empty(),
        "got: {:?}",
        compilation.diagnostics
    );
    let model = compilation.model.expect("model lowered");
    let person = model.packages[0]
        .classes
        .iter()
        .find(|class| class.name == "Person")
        .expect("Person lowered");
    assert_eq!(
        person.features[1].type_,
        rex_ir::TypeRef::Class {
            package: "nz.example.org".to_string(),
            name: "Role".to_string(),
        }
    );
}
