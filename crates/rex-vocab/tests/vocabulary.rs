//! Integration tests for the rex-vocab crate: snapshot parsing/validation,
//! digests, lockfiles, and providers.

use std::collections::BTreeMap;
use std::path::PathBuf;

use rex_ir::{DefaultValue, PrimitiveType, VocabularyFacet};
use rex_vocab::{
    digest, parse_snapshot, sanitize_source, validate_entries, FileProvider, HttpProvider,
    LockEntry, Lockfile, VocabularyDeclaration, VocabularyProvider,
};

const SNAPSHOT_JSON: &str = r#"{
  "vocabulary": "iso:4217",
  "version": "2024-01-01",
  "entries": [
    { "alpha3": "USD", "symbol": "$", "minorUnits": 2, "displayName": "US Dollar" },
    { "alpha3": "EUR", "symbol": "€", "minorUnits": 2, "displayName": "Euro" }
  ]
}"#;

fn currency_declaration() -> VocabularyDeclaration {
    VocabularyDeclaration {
        source: "iso:4217".to_string(),
        version: Some("2024-01-01".to_string()),
        key: "alpha3".to_string(),
        facets: vec![
            VocabularyFacet {
                name: "minorUnits".to_string(),
                type_: PrimitiveType::Int,
            },
            VocabularyFacet {
                name: "symbol".to_string(),
                type_: PrimitiveType::String,
            },
            VocabularyFacet {
                name: "displayName".to_string(),
                type_: PrimitiveType::String,
            },
        ],
    }
}

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rex-vocab-tests-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

#[test]
fn sanitize_source_replaces_colons_and_strips_unsafe_chars() {
    assert_eq!(sanitize_source("iso:4217"), "iso-4217");
    assert_eq!(sanitize_source("plain"), "plain");
    assert_eq!(sanitize_source("ok.Name-1_2"), "ok.Name-1_2");
    assert_eq!(sanitize_source("a b/c@d"), "abcd");
    assert_eq!(sanitize_source(":::"), "---");
}

#[test]
fn digest_matches_known_sha256_vectors() {
    assert_eq!(
        digest(b"abc"),
        "sha256:ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
    assert_eq!(
        digest(b""),
        "sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
    );
}

#[test]
fn parse_snapshot_parses_vocabulary_version_and_entries() {
    let parsed = parse_snapshot(SNAPSHOT_JSON.as_bytes()).expect("parse");
    assert_eq!(parsed.vocabulary, "iso:4217");
    assert_eq!(parsed.version, "2024-01-01");
    assert_eq!(parsed.entries.len(), 2);
    assert_eq!(parsed.entries[0]["alpha3"], "USD");
    assert_eq!(parsed.entries[0]["minorUnits"], 2);
}

#[test]
fn parse_snapshot_rejects_invalid_and_incomplete_json() {
    assert!(parse_snapshot(b"not json").is_err());
    assert!(parse_snapshot(br#"{"vocabulary": "iso:4217"}"#).is_err());
    assert!(parse_snapshot(br#"{"vocabulary": "iso:4217", "version": "1"}"#).is_err());
}

#[test]
fn validate_entries_builds_ir_entries_in_snapshot_order() {
    let parsed = parse_snapshot(SNAPSHOT_JSON.as_bytes()).expect("parse");
    let entries = validate_entries(&parsed, &currency_declaration()).expect("valid");
    assert_eq!(entries.len(), 2);

    assert_eq!(entries[0].key, "USD");
    assert_eq!(entries[0].facets["minorUnits"], DefaultValue::Int(2));
    assert_eq!(
        entries[0].facets["symbol"],
        DefaultValue::String("$".to_string())
    );
    assert_eq!(
        entries[0].facets["displayName"],
        DefaultValue::String("US Dollar".to_string())
    );
    assert_eq!(entries[1].key, "EUR");
    assert_eq!(
        entries[1].facets["symbol"],
        DefaultValue::String("€".to_string())
    );
}

#[test]
fn validate_entries_rejects_a_foreign_snapshot() {
    let parsed = parse_snapshot(SNAPSHOT_JSON.as_bytes()).expect("parse");
    let mut decl = currency_declaration();
    decl.source = "iso:639".to_string();
    let err = validate_entries(&parsed, &decl).unwrap_err();
    assert!(err.to_string().contains("iso:4217"), "error was: {err}");
    assert!(err.to_string().contains("iso:639"), "error was: {err}");
}

#[test]
fn validate_entries_rejects_empty_snapshots() {
    let parsed =
        parse_snapshot(br#"{ "vocabulary": "iso:4217", "version": "2024-01-01", "entries": [] }"#)
            .expect("parse");
    let err = validate_entries(&parsed, &currency_declaration()).unwrap_err();
    assert!(
        err.to_string()
            .contains("vocabulary snapshot has no entries"),
        "error was: {err}"
    );
}

#[test]
fn validate_entries_requires_a_string_key_field() {
    let parsed = parse_snapshot(
        br#"{ "vocabulary": "iso:4217", "version": "1", "entries": [
            { "symbol": "$" }
        ] }"#,
    )
    .expect("parse");
    let err = validate_entries(&parsed, &currency_declaration()).unwrap_err();
    assert!(err.to_string().contains("alpha3"), "error was: {err}");

    let parsed = parse_snapshot(
        br#"{ "vocabulary": "iso:4217", "version": "1", "entries": [
            { "alpha3": 7 }
        ] }"#,
    )
    .expect("parse");
    let err = validate_entries(&parsed, &currency_declaration()).unwrap_err();
    assert!(err.to_string().contains("alpha3"), "error was: {err}");
}

#[test]
fn validate_entries_rejects_duplicate_keys() {
    let parsed = parse_snapshot(
        br#"{ "vocabulary": "iso:4217", "version": "1", "entries": [
            { "alpha3": "USD" }, { "alpha3": "USD" }
        ] }"#,
    )
    .expect("parse");
    let err = validate_entries(&parsed, &currency_declaration()).unwrap_err();
    assert!(err.to_string().contains("USD"), "error was: {err}");
}

#[test]
fn validate_entries_requires_every_declared_facet() {
    let parsed = parse_snapshot(
        br#"{ "vocabulary": "iso:4217", "version": "1", "entries": [
            { "alpha3": "USD", "minorUnits": 2 }
        ] }"#,
    )
    .expect("parse");
    let err = validate_entries(&parsed, &currency_declaration()).unwrap_err();
    assert!(err.to_string().contains("symbol"), "error was: {err}");
}

#[test]
fn validate_entries_checks_facet_value_types() {
    let mut decl = currency_declaration();
    decl.facets.push(VocabularyFacet {
        name: "obsolete".to_string(),
        type_: PrimitiveType::Boolean,
    });
    decl.facets.push(VocabularyFacet {
        name: "rate".to_string(),
        type_: PrimitiveType::Double,
    });

    let snapshot_for = |mutate: &dyn Fn(&mut serde_json::Value)| {
        let mut entry = serde_json::json!({
            "alpha3": "USD",
            "minorUnits": 2,
            "symbol": "$",
            "displayName": "US Dollar",
            "obsolete": false,
            "rate": 1.5
        });
        mutate(&mut entry);
        let json = serde_json::json!({
            "vocabulary": "iso:4217",
            "version": "1",
            "entries": [entry]
        });
        parse_snapshot(json.to_string().as_bytes()).expect("parse")
    };

    // int facet given a string / a fraction: errors.
    let parsed = snapshot_for(&|e| e["minorUnits"] = serde_json::json!("2"));
    assert!(validate_entries(&parsed, &decl).is_err());
    let parsed = snapshot_for(&|e| e["minorUnits"] = serde_json::json!(2.5));
    assert!(validate_entries(&parsed, &decl).is_err());

    // String facet given a number: error.
    let parsed = snapshot_for(&|e| e["symbol"] = serde_json::json!(5));
    assert!(validate_entries(&parsed, &decl).is_err());

    // boolean facet given an integer: error.
    let parsed = snapshot_for(&|e| e["obsolete"] = serde_json::json!(1));
    assert!(validate_entries(&parsed, &decl).is_err());

    // double facets accept fractions and integers.
    let parsed = snapshot_for(&|e| e["rate"] = serde_json::json!(2.5));
    let entries = validate_entries(&parsed, &decl).expect("valid double");
    assert_eq!(
        entries[0].facets["rate"],
        DefaultValue::String("2.5".to_string())
    );
    let parsed = snapshot_for(&|e| e["rate"] = serde_json::json!(2));
    let entries = validate_entries(&parsed, &decl).expect("integral double");
    assert_eq!(entries[0].facets["rate"], DefaultValue::Int(2));
}

#[test]
fn lockfile_round_trips_through_disk() {
    let dir = scratch("lockfile");
    let path = dir.join("model.lock");

    assert!(
        Lockfile::read(&path).is_err(),
        "missing lockfile is an error"
    );

    let mut lockfile = Lockfile::default();
    lockfile.insert(LockEntry {
        source: "iso:4217".to_string(),
        version: "2024-01-01".to_string(),
        digest: "sha256:abc".to_string(),
        fetched_at: "2026-09-10T00:00:00Z".to_string(),
    });
    lockfile.write(&path).expect("write lockfile");

    let read = Lockfile::read(&path).expect("read lockfile");
    assert_eq!(read.vocabularies.len(), 1);
    let entry = read.entry_for("iso:4217").expect("entry present");
    assert_eq!(entry.version, "2024-01-01");
    assert_eq!(entry.digest, "sha256:abc");
    assert_eq!(entry.fetched_at, "2026-09-10T00:00:00Z");
    assert!(read.entry_for("iso:639").is_none());

    // insert replaces an existing entry for the same source.
    let mut updated = Lockfile::default();
    updated.insert(LockEntry {
        source: "iso:4217".to_string(),
        version: "2025-01-01".to_string(),
        digest: "sha256:def".to_string(),
        fetched_at: "2026-09-11T00:00:00Z".to_string(),
    });
    updated.write(&path).expect("rewrite lockfile");
    let read = Lockfile::read(&path).expect("read lockfile");
    assert_eq!(read.vocabularies.len(), 1);
    assert_eq!(read.entry_for("iso:4217").unwrap().version, "2025-01-01");
}

#[test]
fn lockfile_uses_camel_case_wire_names() {
    let mut lockfile = Lockfile::default();
    lockfile.insert(LockEntry {
        source: "iso:4217".to_string(),
        version: "2024-01-01".to_string(),
        digest: "sha256:abc".to_string(),
        fetched_at: "2026-09-10T00:00:00Z".to_string(),
    });
    let json = serde_json::to_value(&lockfile).expect("serialize");
    let entry = &json["vocabularies"][0];
    assert_eq!(entry["source"], "iso:4217");
    assert_eq!(entry["version"], "2024-01-01");
    assert_eq!(entry["digest"], "sha256:abc");
    assert_eq!(entry["fetchedAt"], "2026-09-10T00:00:00Z");
}

#[test]
fn file_provider_reads_sanitized_snapshot_files() {
    let dir = scratch("file-provider");
    let vocab_dir = dir.join("vocab");
    std::fs::create_dir_all(&vocab_dir).expect("create vocab dir");
    std::fs::write(vocab_dir.join("iso-4217@2024-01-01.json"), SNAPSHOT_JSON).expect("snapshot");

    let provider = FileProvider { root: vocab_dir };
    let snapshot = provider
        .fetch("iso:4217", Some("2024-01-01"))
        .expect("fetch from disk");
    assert_eq!(snapshot.version, "2024-01-01");
    assert_eq!(snapshot.bytes, SNAPSHOT_JSON.as_bytes());
}

#[test]
fn file_provider_requires_a_pinned_version() {
    let provider = FileProvider {
        root: scratch("file-provider-unpinned"),
    };
    let err = provider.fetch("iso:4217", None).expect_err("must error");
    assert!(err.to_string().contains("iso:4217"), "error was: {err}");
}

#[test]
fn file_provider_errors_on_missing_snapshot() {
    let provider = FileProvider {
        root: scratch("file-provider-missing").join("vocab"),
    };
    let err = provider
        .fetch("iso:4217", Some("9999-12-31"))
        .expect_err("must error");
    assert!(
        err.to_string().contains("iso-4217@9999-12-31.json"),
        "error was: {err}"
    );
}

#[test]
fn http_provider_builds_urls_from_the_template() {
    let provider = HttpProvider {
        url_template: "https://example.com/vocab/{source}@{version}.json".to_string(),
    };
    let url = provider
        .url_for("iso:4217", Some("2024-01-01"))
        .expect("url");
    assert_eq!(url, "https://example.com/vocab/iso-4217@2024-01-01.json");
    assert!(
        provider.url_for("iso:4217", None).is_err(),
        "unpinned fetch is an error"
    );
}

#[test]
fn http_provider_requires_a_pinned_version_with_clear_message() {
    let provider = HttpProvider {
        url_template: "https://example.com/vocab/{source}@{version}.json".to_string(),
    };
    let err = provider.fetch("iso:4217", None).expect_err("must error");
    assert!(
        err.to_string()
            .contains("vocabulary must be pinned to a version to fetch over http"),
        "error was: {err}"
    );
}

#[test]
fn http_provider_fetch_error_mentions_the_url() {
    // Port 9 (discard) on localhost: connection refused, no real network.
    let provider = HttpProvider {
        url_template: "http://127.0.0.1:9/{source}@{version}.json".to_string(),
    };
    let err = provider
        .fetch("iso:4217", Some("2024-01-01"))
        .expect_err("unroutable endpoint must error");
    let message = err.to_string();
    assert!(
        message.contains("http://127.0.0.1:9/iso-4217@2024-01-01.json"),
        "error was: {message}"
    );
}

#[test]
fn vocabulary_declaration_converts_from_ir_def_minus_entries() {
    let def = rex_ir::VocabularyDef {
        name: "Currency".to_string(),
        description: None,
        source: "iso:4217".to_string(),
        version: Some("2024-01-01".to_string()),
        key: "alpha3".to_string(),
        facets: vec![VocabularyFacet {
            name: "minorUnits".to_string(),
            type_: PrimitiveType::Int,
        }],
        entries: vec![rex_ir::VocabularyEntry {
            key: "USD".to_string(),
            facets: BTreeMap::new(),
        }],
    };
    let declaration: VocabularyDeclaration = (&def).into();
    assert_eq!(declaration.source, "iso:4217");
    assert_eq!(declaration.version.as_deref(), Some("2024-01-01"));
    assert_eq!(declaration.key, "alpha3");
    assert_eq!(declaration.facets.len(), 1);
}
