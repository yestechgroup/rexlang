//! Wire-format contract tests for `ClassDef::table_name` (issue
//! rexlang#177, additive field): a class without the `table "…"` override
//! serializes byte-identically to pre-#177 output (the key is skipped), and
//! a declared override serializes as `tableName` and round-trips. Consumers
//! stop re-deriving table names the moment the override is present.

use rex_ir::ClassDef;

/// The compact serialization of a bare class. Committed here verbatim so
/// any byte drift — including a spuriously emitted `"tableName": null` —
/// fails the test.
#[test]
fn class_without_table_name_stays_byte_identical() {
    let class = ClassDef::new("PayRun", Vec::new(), Vec::new());
    let json = serde_json::to_string(&class).expect("serialize");
    assert_eq!(json, r#"{"name":"PayRun","extends":[],"features":[]}"#);
    assert!(!json.contains("tableName"), "json was: {json}");
}

#[test]
fn table_name_serializes_last_and_round_trips() {
    let class = ClassDef::new("PayRunType", Vec::new(), Vec::new()).with_table_name("pay_run_type");
    let json = serde_json::to_string(&class).expect("serialize");
    assert_eq!(
        json,
        r#"{"name":"PayRunType","extends":[],"features":[],"tableName":"pay_run_type"}"#
    );

    let parsed: ClassDef = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(parsed, class);

    // Absent optional keys deserialize to `None`.
    let sparse: ClassDef =
        serde_json::from_str(r#"{ "name": "PayRun", "extends": [], "features": [] }"#)
            .expect("deserialize sparse class");
    assert_eq!(sparse.table_name, None);
}
