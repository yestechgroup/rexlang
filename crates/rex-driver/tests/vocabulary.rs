//! Integration tests for vocabulary resolution in the driver: snapshot
//! lookup, lockfile pinning, hermetic diagnostics, and IR lowering.
//!
//! Fixtures are written to a per-process scratch directory (no dev-deps).

use std::path::PathBuf;

use rex_driver::compile_str;
use rex_ir::{DefaultValue, TypeRef};
use rex_vocab::digest;

const MODEL: &str = r#"
package demo

vocabulary Currency from "iso:4217" {
    version "2024-01-01"
    key alpha3
    facet int minorUnits
    facet String symbol
    facet String displayName
}

class Price {
    Currency ccy = USD
    int amount
}
"#;

const MODEL_NO_VERSION: &str = r#"
package demo

vocabulary Currency from "iso:4217" {
    key alpha3
    facet int minorUnits
}

class Price {
    Currency ccy
}
"#;

const SNAPSHOT: &str = r#"{
  "vocabulary": "iso:4217",
  "version": "2024-01-01",
  "entries": [
    { "alpha3": "USD", "symbol": "$", "minorUnits": 2, "displayName": "US Dollar" },
    { "alpha3": "EUR", "symbol": "€", "minorUnits": 2, "displayName": "Euro" }
  ]
}"#;

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rex-driver-vocab-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

/// A fixture directory laid out as `<dir>/model.mox`, `<dir>/vocab/`, and —
/// when `lock` is `Some` — `<dir>/model.lock`. Returns the model path and
/// the fixture directory.
fn setup(tag: &str, source: &str, snapshot: bool, lock: Option<&str>) -> (String, PathBuf) {
    let dir = scratch(tag);
    let model_path = dir.join("model.mox");
    std::fs::write(&model_path, source).expect("write model");
    let vocab_dir = dir.join("vocab");
    std::fs::create_dir_all(&vocab_dir).expect("create vocab dir");
    if snapshot {
        std::fs::write(vocab_dir.join("iso-4217@2024-01-01.json"), SNAPSHOT)
            .expect("write snapshot");
    }
    if let Some(lock) = lock {
        std::fs::write(dir.join("model.lock"), lock).expect("write lockfile");
    }
    let path = model_path.to_str().expect("utf-8 model path").to_string();
    (path, dir)
}

fn lockfile_for() -> String {
    let mut lockfile = rex_vocab::Lockfile::default();
    lockfile.insert(rex_vocab::LockEntry {
        source: "iso:4217".to_string(),
        version: "2024-01-01".to_string(),
        digest: digest(SNAPSHOT.as_bytes()),
        fetched_at: "2026-09-10T00:00:00Z".to_string(),
    });
    serde_json::to_string_pretty(&lockfile).expect("serialize lockfile")
}

#[test]
fn vocabulary_model_compiles_with_entries_and_resolved_types() {
    let (path, _dir) = setup("happy", MODEL, true, Some(&lockfile_for()));
    let compilation = compile_str(&path, MODEL);
    assert!(
        compilation.diagnostics.is_empty(),
        "unexpected diagnostics: {:?}",
        compilation.diagnostics
    );
    let model = compilation.model.expect("model lowered");
    let package = &model.packages[0];

    let vocab = &package.vocabularies[0];
    assert_eq!(vocab.name, "Currency");
    assert_eq!(vocab.source, "iso:4217");
    assert_eq!(vocab.version.as_deref(), Some("2024-01-01"));
    assert_eq!(vocab.key, "alpha3");
    assert_eq!(vocab.facets.len(), 3);
    assert_eq!(vocab.entries.len(), 2);
    assert_eq!(vocab.entries[0].key, "USD");
    assert_eq!(vocab.entries[0].facets["minorUnits"], DefaultValue::Int(2));
    assert_eq!(
        vocab.entries[1].facets["symbol"],
        DefaultValue::String("€".to_string())
    );

    let ccy = &package.classes[0].features[0];
    assert_eq!(ccy.name, "ccy");
    assert_eq!(
        ccy.type_,
        TypeRef::Vocabulary {
            package: "demo".to_string(),
            name: "Currency".to_string()
        }
    );
    assert_eq!(ccy.default, Some(DefaultValue::String("USD".to_string())));
}

#[test]
fn missing_snapshot_is_an_error_with_fetch_help() {
    let (path, dir) = setup("missing-snapshot", MODEL, true, Some(&lockfile_for()));
    std::fs::remove_file(dir.join("vocab/iso-4217@2024-01-01.json")).expect("remove snapshot");
    let compilation = compile_str(&path, MODEL);
    assert!(compilation.model.is_none(), "errors block lowering");
    let missing: Vec<_> = compilation
        .diagnostics
        .iter()
        .filter(|d| d.message.contains("iso-4217@2024-01-01.json"))
        .collect();
    assert_eq!(
        missing.len(),
        1,
        "diagnostics: {:?}",
        compilation.diagnostics
    );
    assert!(missing[0].is_error());
    let help = missing[0].help.as_deref().expect("help present");
    assert!(help.contains("rexlang vocab fetch"), "help was: {help}");
    assert!(help.contains("hermetic"), "help was: {help}");
}

#[test]
fn digest_mismatch_is_an_error() {
    let mut lockfile = rex_vocab::Lockfile::default();
    lockfile.insert(rex_vocab::LockEntry {
        source: "iso:4217".to_string(),
        version: "2024-01-01".to_string(),
        digest: "sha256:0000000000000000000000000000000000000000000000000000000000000000"
            .to_string(),
        fetched_at: "2026-09-10T00:00:00Z".to_string(),
    });
    let lock = serde_json::to_string(&lockfile).expect("lockfile");
    let (path, _dir) = setup("digest-mismatch", MODEL, true, Some(&lock));
    let compilation = compile_str(&path, MODEL);
    assert!(compilation.model.is_none(), "errors block lowering");
    let mismatch: Vec<_> = compilation
        .diagnostics
        .iter()
        .filter(|d| d.message.contains("digest mismatch") || d.message.contains("digest"))
        .collect();
    assert_eq!(
        mismatch.len(),
        1,
        "diagnostics: {:?}",
        compilation.diagnostics
    );
    assert!(mismatch[0].is_error());
}

#[test]
fn missing_lockfile_warns_but_compiles() {
    let (path, _dir) = setup("no-lockfile", MODEL, true, None);
    let compilation = compile_str(&path, MODEL);
    assert!(
        compilation.model.is_some(),
        "warnings must not block lowering: {:?}",
        compilation.diagnostics
    );
    let warnings: Vec<_> = compilation
        .diagnostics
        .iter()
        .filter(|d| !d.is_error())
        .collect();
    assert_eq!(
        warnings.len(),
        1,
        "diagnostics: {:?}",
        compilation.diagnostics
    );
    assert!(
        warnings[0].message.contains("not pinned"),
        "warning was: {}",
        warnings[0].message
    );
    assert!(
        warnings[0].message.contains("rexlang vocab fetch"),
        "warning was: {}",
        warnings[0].message
    );
}

#[test]
fn facet_type_must_be_primitive() {
    let source = r#"
        package demo

        class Book { String title }

        vocabulary Currency from "iso:4217" {
            version "2024-01-01"
            key alpha3
            facet Book minorUnits
        }
    "#;
    let (path, _dir) = setup("facet-class", source, true, Some(&lockfile_for()));
    let compilation = compile_str(&path, source);
    assert!(compilation.model.is_none(), "errors block lowering");
    let facet_errors: Vec<_> = compilation
        .diagnostics
        .iter()
        .filter(|d| d.message.contains("minorUnits"))
        .collect();
    assert_eq!(
        facet_errors.len(),
        1,
        "diagnostics: {:?}",
        compilation.diagnostics
    );
    assert!(facet_errors[0].is_error());
    assert!(
        facet_errors[0].message.contains("primitive"),
        "message was: {}",
        facet_errors[0].message
    );
}

#[test]
fn missing_key_is_an_error() {
    let source = r#"
        package demo

        vocabulary Currency from "iso:4217" {
            version "2024-01-01"
            facet int minorUnits
        }
    "#;
    let (path, _dir) = setup("missing-key", source, true, Some(&lockfile_for()));
    let compilation = compile_str(&path, source);
    assert!(compilation.model.is_none(), "errors block lowering");
    let key_errors: Vec<_> = compilation
        .diagnostics
        .iter()
        .filter(|d| d.message.contains("key"))
        .collect();
    assert_eq!(
        key_errors.len(),
        1,
        "diagnostics: {:?}",
        compilation.diagnostics
    );
    assert!(key_errors[0].is_error());
}

#[test]
fn unresolvable_version_suggests_pin_or_fetch() {
    let (path, _dir) = setup("no-version", MODEL_NO_VERSION, false, None);
    let compilation = compile_str(&path, MODEL_NO_VERSION);
    assert!(compilation.model.is_none(), "errors block lowering");
    let version_errors: Vec<_> = compilation
        .diagnostics
        .iter()
        .filter(|d| d.message.contains("version"))
        .collect();
    assert_eq!(
        version_errors.len(),
        1,
        "diagnostics: {:?}",
        compilation.diagnostics
    );
    let combined = format!("{} {:?}", version_errors[0].message, version_errors[0].help);
    assert!(combined.contains("version"), "was: {combined}");
    assert!(combined.contains("rexlang vocab fetch"), "was: {combined}");
}

#[test]
fn unique_snapshot_resolves_version_from_filename() {
    let (path, _dir) = setup("glob-unique", MODEL_NO_VERSION, true, None);
    let compilation = compile_str(&path, MODEL_NO_VERSION);
    assert!(
        compilation.model.is_some(),
        "unexpected diagnostics: {:?}",
        compilation.diagnostics
    );
    let package = compilation.model.unwrap().packages.pop().unwrap();
    assert_eq!(
        package.vocabularies[0].version.as_deref(),
        Some("2024-01-01")
    );
}

#[test]
fn ambiguous_snapshots_are_an_error() {
    let (path, dir) = setup("glob-ambiguous", MODEL_NO_VERSION, true, None);
    let vocab_dir = dir.join("vocab");
    std::fs::write(vocab_dir.join("iso-4217@2025-06-01.json"), SNAPSHOT).expect("second snapshot");
    let compilation = compile_str(&path, MODEL_NO_VERSION);
    assert!(compilation.model.is_none(), "errors block lowering");
    let ambiguous: Vec<_> = compilation
        .diagnostics
        .iter()
        .filter(|d| d.message.contains("multiple snapshots") || d.message.contains("2025-06-01"))
        .collect();
    assert_eq!(
        ambiguous.len(),
        1,
        "diagnostics: {:?}",
        compilation.diagnostics
    );
    assert!(ambiguous[0].is_error());
}

#[test]
fn vocabulary_default_must_be_an_entry_key() {
    let source = r#"
        package demo

        vocabulary Currency from "iso:4217" {
            version "2024-01-01"
            key alpha3
            facet int minorUnits
        }

        class Price {
            Currency ccy = GBP
        }
    "#;
    let (path, _dir) = setup("bad-default", source, true, Some(&lockfile_for()));
    let compilation = compile_str(&path, source);
    assert!(compilation.model.is_none(), "errors block lowering");
    let default_errors: Vec<_> = compilation
        .diagnostics
        .iter()
        .filter(|d| d.message.contains("GBP"))
        .collect();
    assert_eq!(
        default_errors.len(),
        1,
        "diagnostics: {:?}",
        compilation.diagnostics
    );
    assert!(default_errors[0].is_error());
}

#[test]
fn vocabulary_in_relation_position_uses_the_not_a_class_diagnostic() {
    let source = r#"
        package demo

        vocabulary Currency from "iso:4217" {
            version "2024-01-01"
            key alpha3
            facet int minorUnits
        }

        class Wallet {
            contains Currency ccy
        }
    "#;
    let (path, _dir) = setup("relation-position", source, true, Some(&lockfile_for()));
    let compilation = compile_str(&path, source);
    assert!(compilation.model.is_none(), "errors block lowering");
    assert!(
        compilation
            .diagnostics
            .iter()
            .any(|d| d.message.contains("is not a class")),
        "diagnostics: {:?}",
        compilation.diagnostics
    );
}

#[test]
fn string_default_on_vocabulary_attribute_stays_accepted() {
    let source = r#"
        package demo

        vocabulary Currency from "iso:4217" {
            version "2024-01-01"
            key alpha3
            facet int minorUnits
        }

        class Price {
            Currency ccy = "USD"
        }
    "#;
    let (path, _dir) = setup("string-default", source, true, Some(&lockfile_for()));
    let compilation = compile_str(&path, source);
    assert!(
        compilation.diagnostics.is_empty(),
        "a string default names an entry key and stays accepted: {:?}",
        compilation.diagnostics
    );
    let model = compilation.model.expect("model lowered");
    assert_eq!(
        model.packages[0].classes[0].features[0].default,
        Some(DefaultValue::String("USD".to_string()))
    );
}

#[test]
fn int_default_on_vocabulary_attribute_is_rejected() {
    let source = r#"
        package demo

        vocabulary Currency from "iso:4217" {
            version "2024-01-01"
            key alpha3
            facet int minorUnits
        }

        class Price {
            Currency ccy = 5
        }
    "#;
    let (path, _dir) = setup("int-default", source, true, Some(&lockfile_for()));
    let compilation = compile_str(&path, source);
    assert!(compilation.model.is_none(), "errors block lowering");
    let diagnostic = compilation
        .diagnostics
        .iter()
        .find(|d| d.message == "int default '5' on vocabulary-typed attribute 'ccy'")
        .expect("int default on vocabulary attribute is rejected");
    assert!(diagnostic.is_error());
}

#[test]
fn boolean_default_on_vocabulary_attribute_is_rejected() {
    let source = r#"
        package demo

        vocabulary Currency from "iso:4217" {
            version "2024-01-01"
            key alpha3
            facet int minorUnits
        }

        class Price {
            Currency ccy = true
        }
    "#;
    let (path, _dir) = setup("bool-default", source, true, Some(&lockfile_for()));
    let compilation = compile_str(&path, source);
    assert!(compilation.model.is_none(), "errors block lowering");
    assert!(
        compilation
            .diagnostics
            .iter()
            .any(|d| d.message == "boolean default 'true' on vocabulary-typed attribute 'ccy'"),
        "diagnostics: {:?}",
        compilation.diagnostics
    );
}

#[test]
fn unknown_type_diagnostic_is_unchanged() {
    let source = r#"
        package demo

        class Price {
            Crypto ccy
        }
    "#;
    let (path, _dir) = setup("unknown-type", source, false, None);
    let compilation = compile_str(&path, source);
    assert!(compilation.model.is_none());
    assert!(
        compilation
            .diagnostics
            .iter()
            .any(|d| d.message == "unknown type 'Crypto'"),
        "diagnostics: {:?}",
        compilation.diagnostics
    );
}
