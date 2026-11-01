//! Wire rule 3 (register V10): a [`Feature`]'s stable id equals its
//! 0-based declaration index within the owning class, and
//! [`ClassDef::assign_feature_ids`] establishes exactly that. The
//! Creusot contract proves it for all inputs; these tests pin the
//! behavior executably.

use rex_ir::{ClassDef, Feature, FeatureKind, Multiplicity, TypeRef};

fn feature(name: &str) -> Feature {
    Feature::new(
        name,
        FeatureKind::Attribute,
        TypeRef::Primitive(rex_ir::PrimitiveType::String),
        Multiplicity::REQUIRED,
    )
}

#[test]
fn ids_equal_declaration_indices() {
    let class = ClassDef::new(
        "Account",
        vec![],
        vec![feature("bank"), feature("number"), feature("balance")],
    );
    for (index, f) in class.features.iter().enumerate() {
        assert_eq!(f.id, index as u32, "feature '{}'", f.name);
    }
}

#[test]
fn reassignment_follows_reordering() {
    let mut class = ClassDef::new(
        "Account",
        vec![],
        vec![feature("bank"), feature("number"), feature("balance")],
    );
    // Authoring-time reorder: ids must follow the new indices.
    class.features.swap(0, 2);
    class.assign_feature_ids();
    for (index, f) in class.features.iter().enumerate() {
        assert_eq!(f.id, index as u32, "feature '{}'", f.name);
    }
}

#[test]
fn assignment_is_idempotent() {
    let mut class = ClassDef::new("Account", vec![], vec![feature("bank"), feature("number")]);
    let before: Vec<(String, u32)> = class
        .features
        .iter()
        .map(|f| (f.name.clone(), f.id))
        .collect();
    class.assign_feature_ids();
    let after: Vec<(String, u32)> = class
        .features
        .iter()
        .map(|f| (f.name.clone(), f.id))
        .collect();
    assert_eq!(before, after);
}

#[test]
fn feature_lookup_resolves_by_id() {
    let class = ClassDef::new(
        "Account",
        vec![],
        vec![feature("bank"), feature("number"), feature("balance")],
    );
    assert_eq!(class.feature(0).map(|f| f.name.as_str()), Some("bank"));
    assert_eq!(class.feature(2).map(|f| f.name.as_str()), Some("balance"));
    assert!(class.feature(3).is_none(), "no feature past the last index");
}
