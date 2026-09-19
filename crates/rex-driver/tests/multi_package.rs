//! Integration tests for multi-file compilation (`compile_files`): one
//! package per file lowered against the union namespace of all packages, so
//! declarations may reference types across packages.

use rex_driver::{compile_files, render, Diagnostic, MultiCompilation};
use rex_ir::{DefaultValue, FeatureKind, GrantEffect, TypeRef};

fn compile(files: &[(&str, &str)]) -> MultiCompilation {
    let files: Vec<(String, String)> = files
        .iter()
        .map(|(path, source)| (path.to_string(), source.to_string()))
        .collect();
    compile_files(&files)
}

/// The diagnostics whose message contains `needle`, paired with their file
/// path.
fn containing<'a>(
    compilation: &'a MultiCompilation,
    needle: &str,
) -> Vec<(&'a str, &'a Diagnostic)> {
    compilation
        .diagnostics
        .iter()
        .filter(|(_, diagnostic)| diagnostic.message.contains(needle))
        .map(|(path, diagnostic)| (path.as_str(), diagnostic))
        .collect()
}

/// The single diagnostic whose message contains `needle`, with its file
/// path, or a panic.
fn single<'a>(compilation: &'a MultiCompilation, needle: &str) -> (&'a str, &'a Diagnostic) {
    let mut matches = containing(compilation, needle);
    assert_eq!(
        matches.len(),
        1,
        "expected exactly one diagnostic containing '{needle}', got: {:?}",
        compilation.diagnostics
    );
    matches.remove(0)
}

#[test]
fn cross_package_refers_compiles_to_a_two_package_model() {
    let first = "package a\n\nclass Book { String title }";
    let second = "package b\n\nclass Shelf { refers a.Book[] links }";
    let compilation = compile(&[("first.mox", first), ("second.mox", second)]);
    assert!(
        compilation.diagnostics.is_empty(),
        "unexpected diagnostics:\n{}",
        compilation
            .diagnostics
            .iter()
            .map(|(path, diagnostic)| render(
                path,
                if path == "first.mox" { first } else { second },
                std::slice::from_ref(diagnostic)
            ))
            .collect::<String>()
    );
    let model = compilation.model.expect("a model on success");
    // Packages in input-file order.
    assert_eq!(model.packages.len(), 2);
    assert_eq!(model.packages[0].name, "a");
    assert_eq!(model.packages[1].name, "b");

    let links = &model.packages[1].classes[0].features[0];
    assert_eq!(links.kind, FeatureKind::CrossReference);
    assert_eq!(
        links.type_,
        TypeRef::Class {
            package: "a".to_string(),
            name: "Book".to_string(),
        }
    );
}

#[test]
fn bare_and_qualified_self_references_resolve_against_the_union() {
    // A qualified self-reference (`a.Node`) and a bare reference from another
    // package to the uniquely-named `Node` both resolve to package `a`.
    let first = "package a\n\nclass Node { refers a.Node[] parent }";
    let second = "package b\n\nclass Other { refers Node[] handle }";
    let compilation = compile(&[("a.mox", first), ("b.mox", second)]);
    assert!(
        compilation.diagnostics.is_empty(),
        "unexpected diagnostics: {:?}",
        compilation.diagnostics
    );
    let model = compilation.model.expect("a model on success");
    let parent = &model.packages[0].classes[0].features[0];
    assert_eq!(
        parent.type_,
        TypeRef::Class {
            package: "a".to_string(),
            name: "Node".to_string(),
        }
    );
    let handle = &model.packages[1].classes[0].features[0];
    assert_eq!(
        handle.type_,
        TypeRef::Class {
            package: "a".to_string(),
            name: "Node".to_string(),
        }
    );
}

#[test]
fn ambiguous_bare_names_error_with_a_qualify_hint() {
    let first = "package a\n\nclass Widget { int x }";
    let second = "package b\n\nclass Widget { int y }\n\nclass Holder { refers Widget[] w }";
    let compilation = compile(&[("a.mox", first), ("b.mox", second)]);
    assert!(compilation.model.is_none(), "errors must block the model");
    let (path, diagnostic) = single(&compilation, "ambiguous type `Widget`");
    assert!(path.ends_with("b.mox"), "tagged with the referencing file");
    assert_eq!(
        diagnostic.message,
        "ambiguous type `Widget`; qualify as `a.Widget`"
    );
    assert!(
        diagnostic
            .help
            .as_deref()
            .unwrap_or_default()
            .contains("matching types: a.Widget, b.Widget"),
        "unexpected help: {:?}",
        diagnostic.help
    );
}

#[test]
fn qualified_reference_to_an_unknown_package_errors() {
    let first = "package a\n\nclass Holder { refers ghost.Widget[] w }";
    let compilation = compile(&[("a.mox", first)]);
    assert!(compilation.model.is_none());
    let (path, diagnostic) = single(&compilation, "unknown type");
    assert_eq!(path, "a.mox");
    assert_eq!(diagnostic.message, "unknown type 'ghost.Widget'");
}

#[test]
fn cross_package_extends_compiles() {
    let first = "package a\n\nclass Base { id String id }";
    let second = "package b\n\nclass Child extends a.Base { int extra }";
    let compilation = compile(&[("a.mox", first), ("b.mox", second)]);
    assert!(
        compilation.diagnostics.is_empty(),
        "unexpected diagnostics: {:?}",
        compilation.diagnostics
    );
    let model = compilation.model.expect("a model on success");
    assert_eq!(
        model.packages[1].classes[0].extends,
        vec![TypeRef::Class {
            package: "a".to_string(),
            name: "Base".to_string(),
        }]
    );
}

#[test]
fn cross_package_inheritance_cycles_are_detected() {
    let first = "package a\n\nclass A1 extends b.B1 { int a }";
    let second = "package b\n\nclass B1 extends a.A1 { int b }";
    let compilation = compile(&[("a.mox", first), ("b.mox", second)]);
    assert!(compilation.model.is_none(), "errors must block the model");
    single(&compilation, "inheritance cycle detected");
}

const FOREIGN_ENUMS_A: &str = "\
package a

enum Level { Low = 0 High = 10 }
";

#[test]
fn foreign_enum_constraints_closure_check_across_packages() {
    // `minimum 5` admits `High = 10`; `minimum 11` admits nothing.
    let admitting = "package b\n\nclass Gauge { a.Level s { minimum 5 } }";
    let compilation = compile(&[("a.mox", FOREIGN_ENUMS_A), ("b.mox", admitting)]);
    assert!(
        compilation.diagnostics.is_empty(),
        "unexpected diagnostics: {:?}",
        compilation.diagnostics
    );
    let model = compilation.model.expect("a model on success");
    assert_eq!(
        model.packages[1].classes[0].features[0].type_,
        TypeRef::Enum {
            package: "a".to_string(),
            name: "Level".to_string(),
        }
    );

    let rejecting = "package b\n\nclass Gauge { a.Level s { minimum 11 } }";
    let compilation = compile(&[("a.mox", FOREIGN_ENUMS_A), ("b.mox", rejecting)]);
    assert!(compilation.model.is_none(), "errors must block the model");
    let (_, diagnostic) = single(&compilation, "admits no literal");
    assert!(
        diagnostic.message.contains("attribute 's'") && diagnostic.message.contains("'Level'"),
        "unexpected message: {}",
        diagnostic.message
    );
}

const CURRENCY_SNAPSHOT: &str = r#"{
  "vocabulary": "iso:4217",
  "version": "2024-01-01",
  "entries": [
    { "alpha3": "USD" },
    { "alpha3": "EUR" }
  ]
}"#;

const FOREIGN_TYPES_A: &str = "\
package a

enum Color { Red = 0 }

type Date wraps opaque

vocabulary Currency from \"iso:4217\" {
    version \"2024-01-01\"
    key alpha3
    facet String alpha3
}
";

/// Vendors the snapshot and both sources into a fresh scratch directory and
/// pins the digest in `model.lock`, so the compile is warning-free. Returns
/// the `(path, source)` pairs and the scratch directory.
fn foreign_types_fixture(tag: &str, source_b: &str) -> (Vec<(String, String)>, std::path::PathBuf) {
    let dir = std::env::temp_dir().join(format!("rex-multi-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("vocab")).expect("create vocab dir");
    std::fs::write(
        dir.join("vocab/iso-4217@2024-01-01.json"),
        CURRENCY_SNAPSHOT,
    )
    .expect("write snapshot");
    std::fs::write(dir.join("a.mox"), FOREIGN_TYPES_A).expect("write a.mox");
    std::fs::write(dir.join("b.mox"), source_b).expect("write b.mox");
    let mut lockfile = rex_vocab::Lockfile::default();
    lockfile.insert(rex_vocab::LockEntry {
        source: "iso:4217".to_string(),
        version: "2024-01-01".to_string(),
        digest: rex_vocab::digest(CURRENCY_SNAPSHOT.as_bytes()),
        fetched_at: "2026-09-12T00:00:00Z".to_string(),
    });
    lockfile
        .write(&dir.join("model.lock"))
        .expect("write lockfile");
    let a_path = dir.join("a.mox").to_string_lossy().into_owned();
    let b_path = dir.join("b.mox").to_string_lossy().into_owned();
    let files = vec![
        (a_path, FOREIGN_TYPES_A.to_string()),
        (b_path, source_b.to_string()),
    ];
    (files, dir)
}

#[test]
fn foreign_enum_datatype_and_vocabulary_attributes_resolve_qualified() {
    let source_b = "\
package b

class P {
    a.Color c = Red
    a.Date day
    a.Currency ccy = USD
}
";
    let (files, dir) = foreign_types_fixture("types", source_b);
    let compilation = compile_files(&files);
    assert!(
        compilation.diagnostics.is_empty(),
        "unexpected diagnostics: {:?}",
        compilation.diagnostics
    );
    let model = compilation.model.expect("a model on success");
    let features = &model.packages[1].classes[0].features;
    assert_eq!(
        features[0].type_,
        TypeRef::Enum {
            package: "a".to_string(),
            name: "Color".to_string(),
        }
    );
    assert_eq!(
        features[0].default,
        Some(DefaultValue::EnumLiteral("Red".to_string()))
    );
    assert_eq!(
        features[1].type_,
        TypeRef::Datatype {
            package: "a".to_string(),
            name: "Date".to_string(),
        }
    );
    assert_eq!(
        features[2].type_,
        TypeRef::Vocabulary {
            package: "a".to_string(),
            name: "Currency".to_string(),
        }
    );
    assert_eq!(
        features[2].default,
        Some(DefaultValue::String("USD".to_string()))
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn foreign_vocabulary_constraint_families_are_checked_across_packages() {
    // The key facet of the foreign vocabulary is a String, so a numeric
    // constraint is a family mismatch naming the key facet.
    let source_b = "package b\n\nclass P { a.Currency ccy { minimum 0 } }";
    let (files, dir) = foreign_types_fixture("family", source_b);
    let compilation = compile_files(&files);
    assert!(compilation.model.is_none(), "errors must block the model");
    let (path, diagnostic) = single(&compilation, "'minimum' requires a numeric attribute");
    assert!(
        path.ends_with("b.mox"),
        "tagged with the right file: {path}"
    );
    assert!(
        diagnostic
            .message
            .contains("key facet of vocabulary 'Currency' is string"),
        "unexpected message: {}",
        diagnostic.message
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn contains_of_a_foreign_class_is_rejected_with_refers_guidance() {
    let first = "package a\n\nclass Book { String title }";
    let second = "package b\n\nclass Bad { contains a.Book[] shelf }";
    let compilation = compile(&[("a.mox", first), ("b.mox", second)]);
    assert!(compilation.model.is_none(), "errors must block the model");
    let (path, diagnostic) = single(&compilation, "cross-package ownership is not supported");
    assert_eq!(path, "b.mox");
    assert!(
        diagnostic
            .help
            .as_deref()
            .unwrap_or_default()
            .contains("use `refers`"),
        "unexpected help: {:?}",
        diagnostic.help
    );
}

#[test]
fn cross_package_opposites_are_rejected() {
    // A `refers` pair spanning packages, each side naming the other back.
    let first = "package a\n\nclass Author { refers b.Book[] books opposite authors }";
    let second = "package b\n\nclass Book { refers a.Author[] authors opposite books }";
    let compilation = compile(&[("a.mox", first), ("b.mox", second)]);
    assert!(compilation.model.is_none(), "errors must block the model");
    let matches = containing(&compilation, "cross-package opposites are not supported");
    assert_eq!(matches.len(), 2, "both sides must be rejected");
    let mut tags: Vec<&str> = matches.iter().map(|(path, _)| *path).collect();
    tags.sort();
    assert_eq!(tags, ["a.mox", "b.mox"]);
}

#[test]
fn duplicate_package_names_error_on_the_later_file() {
    let first = "package dup\n\nclass A { int a }";
    let second = "package dup\n\nclass B { int b }";
    let compilation = compile(&[("a.mox", first), ("b.mox", second)]);
    assert!(compilation.model.is_none(), "errors must block the model");
    let (path, _) = single(&compilation, "duplicate package");
    assert_eq!(path, "b.mox", "the later file carries the error");
}

#[test]
fn inline_actors_blocks_join_one_union_policy_set() {
    // Same-named actors pool their permits across files, so a `never_both`
    // in one file is violated by a permit granted only in the other.
    let first = "\
package a

class Ticket { String title }

actors SideA {
    actor Op
    capability Approve on Ticket

    grant Op {
        permit Approve
    }
}
";
    let second = "\
package b

class Payment { int amount }

actors SideB {
    actor Op
    capability Approve on Payment
    capability Execute on Payment

    grant Op {
        permit Execute
    }

    never_both { Approve, Execute }
}
";
    let compilation = compile(&[("a.mox", first), ("b.mox", second)]);
    assert!(
        compilation.model.is_none(),
        "separation of duty spans files"
    );
    let (path, diagnostic) = single(&compilation, "is granted both `Approve` and `Execute`");
    assert!(
        path.ends_with("b.mox"),
        "tagged to the never_both file: {path}"
    );
    assert!(
        diagnostic.message.contains("`Op`"),
        "unexpected message: {}",
        diagnostic.message
    );
}

#[test]
fn diagnostics_are_tagged_with_their_own_file() {
    // A warning in the first file and an error in the second keep their own
    // tags, and the warning does not block the model on its own.
    let first = "package a\n\nclass W { op int f() { kotlin { 1 } } }";
    let second = "package b\n\nclass B { Mystery m }";
    let compilation = compile(&[("a.mox", first), ("b.mox", second)]);
    assert!(compilation.model.is_none());
    let (_, warning) = single(&compilation, "unknown target 'kotlin'");
    assert_eq!(warning.severity, rex_driver::Severity::Warning);
    single_tagged(&compilation, "unknown type 'Mystery'", "b.mox");
    single_tagged(&compilation, "unknown target 'kotlin'", "a.mox");
}

/// Asserts the single diagnostic with `needle` is tagged `path`.
fn single_tagged(compilation: &MultiCompilation, needle: &str, path: &str) {
    let (found, _) = single(compilation, needle);
    assert_eq!(found, path);
}

#[test]
fn syntax_error_file_blocks_the_model_and_tags_its_diagnostics() {
    let first = "package a\n\nclass Book {";
    let second = "package b\n\nclass Shelf { String label }";
    let compilation = compile(&[("a.mox", first), ("b.mox", second)]);
    assert!(compilation.model.is_none(), "syntax errors block the model");
    assert!(
        !containing(&compilation, "").is_empty(),
        "the broken file contributes diagnostics"
    );
    // Every diagnostic belongs to the broken file.
    for (path, _) in &compilation.diagnostics {
        assert_eq!(path, "a.mox", "diagnostics must be tagged a.mox");
    }
}

/// Inline `actors` blocks must land on their declaring package in the
/// multi-file model, exactly as the single-file path joins them. The
/// capability's class resolves against the union namespace — here the
/// block in `b.mox` binds to a class declared in `a.mox` — and the
/// `when` condition type-checks against that class's features.
#[test]
fn inline_actors_blocks_join_their_package_in_compile_files() {
    let first = "package a\n\nclass Ticket {\n    id String id\n    int amount\n}";
    let second = "package b\n\nactors Support {\n    actor Manager\n\n    capability RaiseRefund on a.Ticket\n\n    grant Manager {\n        permit RaiseRefund when (amount > 0)\n    }\n}";
    let compilation = compile(&[("a.mox", first), ("b.mox", second)]);
    assert!(
        compilation.diagnostics.is_empty(),
        "unexpected diagnostics:\n{}",
        compilation
            .diagnostics
            .iter()
            .map(|(path, diagnostic)| render(
                path,
                if path == "a.mox" { first } else { second },
                std::slice::from_ref(diagnostic)
            ))
            .collect::<String>()
    );
    let model = compilation.model.expect("a model on success");
    assert_eq!(model.packages.len(), 2);

    // The file without an actors block carries none.
    assert!(
        model.packages[0].actors.is_empty(),
        "package 'a' declares no actors block"
    );

    // The block joins its own package, with the union-namespace class
    // binding and the type-checked `when` condition intact.
    let actors = &model.packages[1].actors;
    assert_eq!(actors.len(), 1, "package 'b' carries its one block");
    assert_eq!(actors[0].name, "Support");
    assert_eq!(
        actors[0]
            .actors
            .iter()
            .map(|actor| actor.name.as_str())
            .collect::<Vec<_>>(),
        ["Manager"]
    );
    assert_eq!(actors[0].capabilities.len(), 1);
    assert_eq!(
        actors[0].capabilities[0].class,
        TypeRef::Class {
            package: "a".to_string(),
            name: "Ticket".to_string(),
        }
    );
    assert_eq!(actors[0].grants.len(), 1);
    assert_eq!(actors[0].grants[0].actor, "Manager");
    assert_eq!(actors[0].grants[0].entries.len(), 1);
    assert_eq!(actors[0].grants[0].entries[0].effect, GrantEffect::Permit);
    assert_eq!(
        actors[0].grants[0].entries[0].when.as_deref(),
        Some("amount > 0")
    );
}

/// Same-named actors in blocks across files pool their permits at
/// validation (union semantics) — the blocks stay separate, one per
/// package, and the union compiles. Validation still applies: a grant
/// naming an undeclared actor errors.
#[test]
fn inline_actors_blocks_stay_separate_and_still_validate() {
    let first = "package a\n\nclass Ticket { int amount }\n\nactors Support {\n    actor Manager\n\n    capability RaiseRefund on Ticket\n\n    grant Manager {\n        permit RaiseRefund\n    }\n}";
    let second = "package b\n\nclass Ledger { int balance }\n\nactors Support {\n    actor Auditor\n\n    capability AuditLedger on Ledger\n\n    grant Ghost {\n        permit AuditLedger\n    }\n}";
    let compilation = compile(&[("a.mox", first), ("b.mox", second)]);
    let (model, diagnostic) = single(&compilation, "unknown actor `Ghost`");
    assert_eq!(model, "b.mox");
    assert_eq!(diagnostic.severity, rex_driver::Severity::Error);
    assert!(
        compilation.model.is_none(),
        "an error-severity actors diagnostic blocks the model"
    );

    // Without the invalid grant, both blocks join their own packages and
    // the same-named blocks pool at validation without erroring.
    let valid_second =
        "package b\n\nclass Ledger { int balance }\n\nactors Support {\n    actor Auditor\n\n    capability AuditLedger on Ledger\n\n    grant Auditor {\n        permit AuditLedger\n    }\n}";
    let compilation = compile(&[("a.mox", first), ("b.mox", valid_second)]);
    assert!(
        compilation.diagnostics.is_empty(),
        "unexpected diagnostics:\n{}",
        compilation
            .diagnostics
            .iter()
            .map(|(path, diagnostic)| render(
                path,
                if path == "a.mox" { first } else { valid_second },
                std::slice::from_ref(diagnostic)
            ))
            .collect::<String>()
    );
    let model = compilation.model.expect("a model on success");
    assert_eq!(model.packages[0].actors.len(), 1);
    assert_eq!(model.packages[1].actors.len(), 1);
    assert_eq!(model.packages[0].actors[0].name, "Support");
    assert_eq!(model.packages[1].actors[0].name, "Support");
}
