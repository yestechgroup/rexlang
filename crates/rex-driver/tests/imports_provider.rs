//! Characterization tests for the import-content provider contract:
//! [`rex_driver::SchemaImports`] and [`rex_driver::SigilImports`] — the
//! twin `(mox path, import path)` providers bundled in
//! [`rex_driver::DomainImports`].
//!
//! These pin the provider-level behaviors the compile entry points build
//! on (insert last-wins, insertion order, independence of the two
//! buckets) so refactors of the shared store stay behavior-identical.

use rex_driver::{compile_str, DomainImports, SchemaImports, SigilImports};

// --- provider-level contract -----------------------------------------------------

#[test]
fn re_inserting_a_pair_replaces_the_entry_last_wins() {
    let mut schema = SchemaImports::new();
    schema.insert("demo.mox", "x.json", "first");
    schema.insert("demo.mox", "x.json", "second");
    assert_eq!(
        schema.get("demo.mox", "x.json"),
        Some("second"),
        "a re-provided (mox, import) pair must replace the earlier content"
    );
    assert_eq!(schema.get("demo.mox", "y.json"), None);
    let sigil = SigilImports::new()
        .provide("demo.mox", "a.rosetta", "first")
        .provide("demo.mox", "a.rosetta", "second");
    assert_eq!(
        sigil.get("demo.mox", "a.rosetta"),
        Some("second"),
        "provide chains follow the same last-wins rule"
    );
}

#[test]
fn replacement_keeps_the_first_appearance_position() {
    let sigil = SigilImports::new()
        .provide("demo.mox", "a.rosetta", "a1")
        .provide("demo.mox", "b.rosetta", "b1")
        .provide("demo.mox", "c.rosetta", "c1")
        .provide("demo.mox", "b.rosetta", "b2");
    let entries: Vec<_> = sigil
        .entries()
        .iter()
        .map(|(mox, import, text)| (mox.as_str(), import.as_str(), text.as_str()))
        .collect();
    assert_eq!(
        entries,
        [
            ("demo.mox", "a.rosetta", "a1"),
            ("demo.mox", "b.rosetta", "b2"),
            ("demo.mox", "c.rosetta", "c1"),
        ],
        "entries stay in first-appearance order; a replacement rewrites in place"
    );
}

#[test]
fn distinct_import_paths_under_one_mox_path_stay_separate() {
    // The `.sigil` candidate-file shape: every *.rosetta sibling is
    // provided under its own import path; each must stay retrievable.
    let sigil = SigilImports::new()
        .provide("demo.mox", "trade.rosetta", "namespace oracle.trade")
        .provide("demo.mox", "party.rosetta", "namespace oracle.party");
    assert_eq!(
        sigil.get("demo.mox", "trade.rosetta"),
        Some("namespace oracle.trade")
    );
    assert_eq!(
        sigil.get("demo.mox", "party.rosetta"),
        Some("namespace oracle.party")
    );
    assert_eq!(sigil.entries().len(), 2);
    assert_eq!(
        sigil.get("other.mox", "trade.rosetta"),
        None,
        "content provided for another mox file does not satisfy a lookup"
    );
}

#[test]
fn unprovided_pairs_resolve_to_none() {
    let schema = SchemaImports::new().provide("demo.mox", "x.json", "{}");
    assert_eq!(schema.get("demo.mox", "y.json"), None);
    assert_eq!(schema.get("other.mox", "x.json"), None);
    assert_eq!(SchemaImports::new().get("demo.mox", "x.json"), None);
}

#[test]
fn is_empty_tracks_provided_content() {
    let mut schema = SchemaImports::new();
    assert!(schema.is_empty());
    schema.insert("demo.mox", "x.json", "{}");
    assert!(!schema.is_empty());
    let sigil = SigilImports::new();
    assert!(sigil.is_empty());
}

#[test]
fn identically_provided_providers_are_equal() {
    let a = SchemaImports::new()
        .provide("demo.mox", "x.json", "{}")
        .provide("demo.mox", "y.json", "[]");
    let b = SchemaImports::new()
        .provide("demo.mox", "x.json", "{}")
        .provide("demo.mox", "y.json", "[]");
    assert_eq!(a, b);
    let replaced = b.provide("demo.mox", "y.json", "{}");
    assert_ne!(a, replaced);
}

// --- compile-level contract ------------------------------------------------------

#[test]
fn re_provided_schema_content_replaces_the_first() {
    // First content invalid, second valid: the compile consumes the last
    // one, so the compilation is clean. First-wins would error on the
    // invalid JSON.
    let imports = DomainImports {
        schemas: SchemaImports::new()
            .provide("demo.mox", "x.json", "{ not json")
            .provide("demo.mox", "x.json", "{}"),
        sigil: SigilImports::new(),
    };
    let compilation = compile_str(
        "demo.mox",
        "package demo\n\nimport schema \"x.json\" as X\n\nclass C { refers X x }\n",
        &imports,
    );
    assert!(
        compilation.diagnostics.is_empty(),
        "the last-provided schema content must be the one compiled: {:?}",
        compilation
            .diagnostics
            .iter()
            .map(|d| d.message.clone())
            .collect::<Vec<_>>()
    );
    assert!(compilation.model.is_some());
}

#[test]
fn re_provided_sigil_content_replaces_the_first() {
    // The mox source references `oracle.two.Trade`; only the second
    // provided rosetta text declares that namespace. First-wins would
    // lower `oracle.one` and leave the reference unresolved.
    let imports = DomainImports {
        schemas: SchemaImports::new(),
        sigil: SigilImports::new()
            .provide(
                "demo.mox",
                "trade.rosetta",
                "namespace oracle.one\n\ntype Trade:\n    id string (1..1)\n",
            )
            .provide(
                "demo.mox",
                "trade.rosetta",
                "namespace oracle.two\n\ntype Trade:\n    id string (1..1)\n",
            ),
    };
    let compilation = compile_str(
        "demo.mox",
        concat!(
            "package demo\n\n",
            "import sigil \"trade.rosetta\"\n\n",
            "class Book { refers oracle.two.Trade about }\n"
        ),
        &imports,
    );
    assert!(
        compilation.diagnostics.is_empty(),
        "the last-provided sigil content must be the one lowered: {:?}",
        compilation
            .diagnostics
            .iter()
            .map(|d| d.message.clone())
            .collect::<Vec<_>>()
    );
    let model = compilation.model.expect("the clean compile lowers");
    assert_eq!(model.packages.len(), 2);
    assert_eq!(model.packages[1].name, "oracle.two");
}

#[test]
fn schema_and_sigil_buckets_are_independent() {
    // Schema content parked in the sigil bucket (and vice versa) never
    // satisfies the other surface's import.
    let imports = DomainImports {
        schemas: SchemaImports::new(),
        sigil: SigilImports::new().provide("demo.mox", "x.json", "{}"),
    };
    let compilation = compile_str(
        "demo.mox",
        "package demo\n\nimport schema \"x.json\" as X\n\nclass C { refers X x }\n",
        &imports,
    );
    assert!(
        compilation
            .diagnostics
            .iter()
            .any(|d| d.message == "imported schema 'x.json' was not provided"),
        "a sigil-bucket entry must not satisfy a schema import: {:?}",
        compilation
            .diagnostics
            .iter()
            .map(|d| d.message.clone())
            .collect::<Vec<_>>()
    );

    let imports = DomainImports {
        schemas: SchemaImports::new().provide(
            "demo.mox",
            "trade.rosetta",
            "namespace oracle.one\n\ntype Trade:\n    id string (1..1)\n",
        ),
        sigil: SigilImports::new(),
    };
    let compilation = compile_str(
        "demo.mox",
        concat!(
            "package demo\n\n",
            "import sigil \"trade.rosetta\"\n\n",
            "class Book { refers oracle.one.Trade about }\n"
        ),
        &imports,
    );
    // The sigil not-provided error is a sigil diagnostic (it concerns the
    // rosetta side, not the `.mox` file); the `.mox` diagnostics carry the
    // downstream unresolved reference instead.
    assert!(
        compilation
            .sigil_diagnostics
            .iter()
            .any(|(_, d)| d.message == "imported sigil 'trade.rosetta' was not provided"),
        "a schema-bucket entry must not satisfy a sigil import: {:?}",
        compilation
            .sigil_diagnostics
            .iter()
            .map(|(_, d)| d.message.clone())
            .collect::<Vec<_>>()
    );
}

#[test]
fn the_default_bundle_provides_nothing() {
    let compilation = compile_str(
        "demo.mox",
        "package demo\n\nimport schema \"x.json\" as X\n\nclass C { refers X x }\n",
        &DomainImports::default(),
    );
    assert_eq!(
        compilation.diagnostics.first().map(|d| d.message.as_str()),
        Some("imported schema 'x.json' was not provided")
    );
}
