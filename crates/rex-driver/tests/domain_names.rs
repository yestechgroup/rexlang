//! The stripped-name uniqueness check (issue rexlang#177): two classes or
//! enums in one package whose names strip to the same module identifier
//! (E0300), or map to the same effective table name (E0301), collide
//! silently downstream — consumers generate the same module path / pg table
//! and one declaration overwrites the other. These tests pin the check's
//! scope (per package, classes and enums alike), the containment exemption,
//! the `table "…"` escape hatch (which flows through the IR as
//! `ClassDef::table_name`), and the no-cascade discipline (an unrelated
//! error keeps the check silent).

use rex_driver::{compile_files, compile_str, DomainImports};

/// Compiles one in-memory `.mox` source.
fn compile(source: &str) -> rex_driver::Compilation {
    compile_str("test.mox", source, &DomainImports::default())
}

/// The stripped-name errors of one compilation (messages only).
fn errors(source: &str) -> Vec<rex_driver::Diagnostic> {
    compile(source)
        .diagnostics
        .into_iter()
        .filter(|diagnostic| diagnostic.is_error())
        .collect()
}

#[test]
fn two_entities_stripping_to_one_module_name_error() {
    let source = r#"
package nz.example.payroll

class PayRun {
    int total
}

class PayRunType {
    String name
}
"#;
    let compilation = compile(source);
    let model_errors: Vec<_> = compilation
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.is_error())
        .collect();
    assert_eq!(model_errors.len(), 1, "got: {:?}", compilation.diagnostics);
    let error = model_errors[0];
    // The code prefixes the message, and BOTH declarations are named.
    assert!(error.message.contains("E0300"), "got: {}", error.message);
    assert!(
        error.message.contains("'PayRun'") && error.message.contains("'PayRunType'"),
        "got: {}",
        error.message
    );
    assert!(
        error.message.contains("stripped module name 'pay_run'"),
        "got: {}",
        error.message
    );
    // The span sits on the later declaration's name.
    let offset = source
        .find("PayRunType")
        .expect("later declaration in source");
    assert_eq!(
        error.span,
        Some((offset..offset + "PayRunType".len()).into())
    );
    // The help suggests the escape hatch or a rename.
    let help = error.help.as_deref().expect("a help hint");
    assert!(
        help.contains("table") || help.contains("rename"),
        "got: {help}"
    );
    // The collision blocks the artifact.
    assert!(compilation.model.is_none());
}

#[test]
fn enum_pairs_and_class_enum_pairs_error_too() {
    // Enum + enum.
    let source = r#"
package nz.example.payroll

enum PayRunStatus {
    Draft = 0
}

enum PayRunStatusType {
    Draft = 0
}
"#;
    let found = errors(source);
    assert_eq!(found.len(), 1, "got: {found:?}");
    assert!(
        found[0]
            .message
            .contains("E0300 stripped module name 'pay_run_status'"),
        "got: {}",
        found[0].message
    );

    // Class + enum.
    let source = r#"
package nz.example.stuff

class Foo {
    String name
}

enum FooType {
    A = 0
}
"#;
    let found = errors(source);
    assert_eq!(found.len(), 1, "got: {found:?}");
    assert!(
        found[0].message.contains("class 'Foo' and enum 'FooType'"),
        "got: {}",
        found[0].message
    );
}

#[test]
fn a_contained_value_object_is_exempt() {
    // The classic entity + value-object pairing: `PayRunType` is a
    // containment target (`PayRun` declares `contains PayRunType items`), so
    // the stripped-name pair compiles — downstream, a contained value object
    // lives inside its owner's module and never collides with it.
    let source = r#"
package nz.example.payroll

class PayRun {
    int total
    contains PayRunType items
}

class PayRunType {
    String name
}
"#;
    let compilation = compile(source);
    assert!(
        compilation.diagnostics.is_empty(),
        "the containment exemption compiles clean, got: {:?}",
        compilation.diagnostics
    );
    assert!(compilation.model.is_some());
}

#[test]
fn the_table_override_flows_through_the_ir() {
    let source = r#"
package nz.example.payroll

class PayRunType table "pay_run_type" {
    String name
}
"#;
    let compilation = compile(source);
    assert!(
        compilation.diagnostics.is_empty(),
        "got: {:?}",
        compilation.diagnostics
    );
    let model = compilation.model.expect("a model on success");
    let class = &model.packages[0].classes[0];
    assert_eq!(class.name, "PayRunType");
    assert_eq!(class.table_name.as_deref(), Some("pay_run_type"));
    // A class without the annotation keeps deriving: `table_name` stays None.
    let source = r#"
package nz.example.payroll

class PayRun {
    int total
}
"#;
    let model = compile(source).model.expect("a model on success");
    assert_eq!(model.packages[0].classes[0].table_name, None);
}

#[test]
fn a_table_override_colliding_with_a_derived_table_errors() {
    let source = r#"
package nz.example.payroll

class A table "pay_run" {
    String name
}

class PayRun {
    int total
}
"#;
    let compilation = compile(source);
    let model_errors: Vec<_> = compilation
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.is_error())
        .collect();
    assert_eq!(model_errors.len(), 1, "got: {:?}", compilation.diagnostics);
    let error = model_errors[0];
    assert!(error.message.contains("E0301"), "got: {}", error.message);
    assert!(
        error
            .message
            .contains("table name 'pay_run' is shared by class 'A' and class 'PayRun'"),
        "got: {}",
        error.message
    );
    // The span sits on the later declaration's name.
    let offset = source.find("class PayRun").expect("later declaration") + "class ".len();
    assert_eq!(error.span, Some((offset..offset + "PayRun".len()).into()));
    assert!(compilation.model.is_none());
}

#[test]
fn a_table_override_resolves_a_table_only_collision() {
    // Two distinct module names (`pay_run`, `ledger`) forced onto one table
    // by an override pointing at the other's derived default: E0301 names
    // both, and moving the override off the collision compiles clean.
    let source = r#"
package nz.example.payroll

class PayRun {
    int total
}

class Ledger table "pay_run" {
    String label
}
"#;
    let found = errors(source);
    assert_eq!(found.len(), 1, "got: {found:?}");
    assert!(
        found[0]
            .message
            .contains("E0301 table name 'pay_run' is shared by class 'PayRun' and class 'Ledger'"),
        "got: {}",
        found[0].message
    );

    let source = r#"
package nz.example.payroll

class PayRun {
    int total
}

class Ledger table "ledger_main" {
    String label
}
"#;
    let compilation = compile(source);
    assert!(
        compilation.diagnostics.is_empty(),
        "the override resolves the table collision, got: {:?}",
        compilation.diagnostics
    );
}

#[test]
fn the_same_stripped_names_in_two_packages_compile() {
    // The check is per-package: `a.PayRun` and `b.PayRunType` never meet.
    let files = [
        (
            "a.mox".to_string(),
            "package nz.example.a\n\nclass PayRun {\n    int total\n}\n".to_string(),
        ),
        (
            "b.mox".to_string(),
            "package nz.example.b\n\nclass PayRunType {\n    String name\n}\n".to_string(),
        ),
    ];
    let compilation = compile_files(&files, &DomainImports::default());
    assert!(
        compilation.diagnostics.is_empty(),
        "cross-package pairs never collide, got: {:?}",
        compilation.diagnostics
    );
    assert!(compilation.model.is_some());
}

#[test]
fn an_unrelated_error_keeps_the_check_silent() {
    // No cascade: a model that fails for another reason produces only that
    // reason's diagnostics — the uniqueness check has no verdict on a model
    // that did not lower.
    let source = r#"
package nz.example.payroll

class PayRun {
    Unknown thing
}

class PayRunType {
    String name
}
"#;
    let found = errors(source);
    assert_eq!(found.len(), 1, "got: {found:?}");
    assert!(
        !found[0].message.contains("E03"),
        "the uniqueness check must stay silent, got: {}",
        found[0].message
    );
}
