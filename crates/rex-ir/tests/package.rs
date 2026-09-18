//! Wire-format contract tests for the package-level `description`
//! (additive field, wire contract rule 11): a package without a description
//! serializes byte-identically to pre-package-doc output.

use rex_ir::Package;

/// The compact serialization of a bare package. Committed here verbatim so
/// any byte drift — including a spuriously emitted `"description": null` —
/// fails the test.
#[test]
fn package_without_description_stays_byte_identical() {
    let package = Package::new("nz.example.demo");
    let json = serde_json::to_string(&package).expect("serialize");
    assert_eq!(
        json,
        r#"{"name":"nz.example.demo","annotations":[],"enums":[],"datatypes":[],"interfaces":[],"classes":[]}"#
    );
    assert!(!json.contains("description"), "json was: {json}");
}

#[test]
fn package_description_serializes_after_name_and_round_trips() {
    let mut package = Package::new("nz.example.demo");
    package.description = Some("A demo package.".to_string());

    let json = serde_json::to_string(&package).expect("serialize");
    assert!(
        json.starts_with(
            r#"{"name":"nz.example.demo","description":"A demo package.","annotations":"#
        ),
        "description must follow `name`, json was: {json}"
    );

    let parsed: Package = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(parsed.description.as_deref(), Some("A demo package."));

    // Absent optional keys deserialize to `None`.
    let sparse: Package = serde_json::from_str(r#"{ "name": "nz.example.demo" }"#)
        .expect("deserialize sparse package");
    assert_eq!(sparse.description, None);
}
