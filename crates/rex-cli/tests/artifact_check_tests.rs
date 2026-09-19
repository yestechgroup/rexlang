//! `rexlang artifact check` tests: run the binary as a subprocess against
//! known-good and deliberately corrupted artifacts, plus a dogfood pass over
//! every golden artifact under `tests/conformance/artifacts/`.

use std::path::{Path, PathBuf};
use std::process::Command;

fn rexlang() -> Command {
    Command::new(env!("CARGO_BIN_EXE_rexlang"))
}

/// A unique scratch directory per test, so concurrently running tests never
/// share files even when they use the same artifact name.
fn scratch_dir() -> PathBuf {
    let thread = std::thread::current();
    let test = thread
        .name()
        .unwrap_or("test")
        .replace(['/', ' ', ':'], "_");
    let dir = std::env::temp_dir().join(format!(
        "rex-artifact-check-tests-{}-{}",
        std::process::id(),
        test
    ));
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

fn write_artifact(name: &str, contents: &str) -> PathBuf {
    let path = scratch_dir().join(name);
    std::fs::write(&path, contents).expect("write artifact");
    path
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../")
        .canonicalize()
        .expect("workspace root")
}

const GOOD_IR_ARTIFACT: &str = r#"{
  "formatVersion": 1,
  "rexVersion": "0.1.0",
  "packages": [
    {
      "name": "demo",
      "annotations": [],
      "enums": [],
      "datatypes": [],
      "interfaces": [],
      "classes": [
        {
          "name": "Book",
          "extends": [],
          "features": [
            {
              "id": 0,
              "name": "title",
              "kind": "attribute",
              "type": { "type": "primitive", "value": "string" },
              "multiplicity": { "lower": 1, "upper": { "finite": 1 } },
              "isDerived": false,
              "isId": false,
              "isReadOnly": false
            }
          ]
        }
      ]
    }
  ]
}
"#;

const GOOD_ACTOR_ARTIFACT: &str = r#"{
  "formatVersion": 1,
  "blocks": [
    {
      "name": "Support",
      "actors": [
        { "name": "Finance" },
        { "name": "Agent", "extends": "Finance", "kind": "agent" }
      ],
      "capabilities": [
        {
          "name": "Read",
          "class": { "type": "class", "value": { "package": "demo", "name": "Book" } }
        }
      ],
      "grants": [
        { "actor": "Finance", "entries": [ { "effect": "permit", "capability": "Read" } ] }
      ]
    }
  ]
}
"#;

const GOOD_INSTANCE: &str = r#"{
  "$type": "rex.instance",
  "formatVersion": 1,
  "model": "demo",
  "objects": [
    {
      "$id": "library/0",
      "$type": "Library",
      "shelves": [
        {
          "$id": "book/0",
          "$type": "Book",
          "title": "Dune",
          "author": { "$ref": "writer/0" }
        }
      ]
    },
    { "$id": "writer/0", "$type": "Writer", "name": "Frank" }
  ]
}
"#;

#[test]
fn artifact_check_passes_on_valid_ir_actor_and_instance_artifacts() {
    let ir = write_artifact("valid.rex.json", GOOD_IR_ARTIFACT);
    let actor = write_artifact("valid.actors.rex.json", GOOD_ACTOR_ARTIFACT);
    let instance = write_artifact("valid.instance.json", GOOD_INSTANCE);
    let output = rexlang()
        .args([
            "artifact",
            "check",
            ir.to_str().unwrap(),
            actor.to_str().unwrap(),
            instance.to_str().unwrap(),
        ])
        .output()
        .expect("run rexlang artifact check");
    assert!(
        output.status.success(),
        "stderr: {:?}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        format!(
            "OK {}\nOK {}\nOK {}\n",
            ir.display(),
            actor.display(),
            instance.display()
        )
    );
    assert!(String::from_utf8_lossy(&output.stderr).is_empty());
}

#[test]
fn artifact_check_rejects_unsupported_format_version_and_names_it() {
    let cases: Vec<(&str, String)> = vec![
        ("ir_v2.rex.json", version_bump(GOOD_IR_ARTIFACT)),
        (
            "actor_v2.actors.rex.json",
            version_bump(GOOD_ACTOR_ARTIFACT),
        ),
        ("instance_v2.json", version_bump(GOOD_INSTANCE)),
    ];
    for (name, contents) in cases {
        let path = write_artifact(name, &contents);
        let output = rexlang()
            .args(["artifact", "check", path.to_str().unwrap()])
            .output()
            .expect("run rexlang artifact check");
        assert_eq!(output.status.code(), Some(1), "{name} must fail");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains("formatVersion 2"),
            "{name} must name the found version; stderr was: {stderr}"
        );
        assert!(
            stderr.contains("formatVersion 1"),
            "{name} must name the supported version; stderr was: {stderr}"
        );
        assert!(
            stderr.contains(name),
            "the artifact path must name the report: {stderr}"
        );
    }
}

/// Corrupts an artifact by claiming a newer wire format.
fn version_bump(artifact: &str) -> String {
    artifact.replace("\"formatVersion\": 1", "\"formatVersion\": 2")
}

#[test]
fn artifact_check_rejects_snake_case_keys() {
    let corrupted = GOOD_IR_ARTIFACT.replace("\"isReadOnly\": false", "\"is_read_only\": false");
    assert_ne!(corrupted, GOOD_IR_ARTIFACT, "corruption must apply");
    let path = write_artifact("snake_case.rex.json", &corrupted);
    let output = rexlang()
        .args(["artifact", "check", path.to_str().unwrap()])
        .output()
        .expect("run rexlang artifact check");
    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("is_read_only"),
        "the offending key must be named; stderr was: {stderr}"
    );
    assert!(
        stderr.contains("isReadOnly"),
        "the camelCase spelling must be suggested; stderr was: {stderr}"
    );
}

#[test]
fn artifact_check_rejects_unresolved_refs() {
    let corrupted = GOOD_INSTANCE.replace("\"$ref\": \"writer/0\"", "\"$ref\": \"writer/9\"");
    assert_ne!(corrupted, GOOD_INSTANCE, "corruption must apply");
    let path = write_artifact("unresolved_ref.instance.json", &corrupted);
    let output = rexlang()
        .args(["artifact", "check", path.to_str().unwrap()])
        .output()
        .expect("run rexlang artifact check");
    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("writer/9"),
        "the dangling ref must be named; stderr was: {stderr}"
    );
}

#[test]
fn artifact_check_rejects_duplicate_ids() {
    let corrupted = GOOD_INSTANCE.replace(
        r#"{ "$id": "writer/0", "$type": "Writer", "name": "Frank" }"#,
        r#"{ "$id": "book/0", "$type": "Book", "title": "Dup" },
    { "$id": "writer/0", "$type": "Writer", "name": "Frank" }"#,
    );
    assert_ne!(corrupted, GOOD_INSTANCE, "corruption must apply");
    let path = write_artifact("duplicate_id.instance.json", &corrupted);
    let output = rexlang()
        .args(["artifact", "check", path.to_str().unwrap()])
        .output()
        .expect("run rexlang artifact check");
    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("book/0"),
        "the duplicated id must be named; stderr was: {stderr}"
    );
    assert!(
        stderr.to_lowercase().contains("duplicate"),
        "the error must say the id is a duplicate; stderr was: {stderr}"
    );
}

#[test]
fn artifact_check_rejects_unknown_root_type() {
    let corrupted = GOOD_INSTANCE.replace(
        "\"$type\": \"rex.instance\"",
        "\"$type\": \"rogue.instance\"",
    );
    assert_ne!(corrupted, GOOD_INSTANCE, "corruption must apply");
    let path = write_artifact("unknown_root.instance.json", &corrupted);
    let output = rexlang()
        .args(["artifact", "check", path.to_str().unwrap()])
        .output()
        .expect("run rexlang artifact check");
    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("rogue.instance"),
        "the unknown root type must be named; stderr was: {stderr}"
    );
    assert!(
        stderr.contains("rex.instance"),
        "the expected root type must be named; stderr was: {stderr}"
    );
}

#[test]
fn artifact_check_missing_file_is_a_clean_error() {
    let missing = scratch_dir().join("does-not-exist.rex.json");
    let output = rexlang()
        .args(["artifact", "check", missing.to_str().unwrap()])
        .output()
        .expect("run rexlang artifact check");
    assert_eq!(output.status.code(), Some(1));
    assert!(
        String::from_utf8_lossy(&output.stderr).starts_with("error: cannot read"),
        "stderr was: {:?}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn artifact_check_reports_every_file_and_fails_if_any_fails() {
    let good = write_artifact("batch_good.rex.json", GOOD_IR_ARTIFACT);
    let bad = write_artifact("batch_bad.rex.json", &version_bump(GOOD_IR_ARTIFACT));
    let output = rexlang()
        .args([
            "artifact",
            "check",
            good.to_str().unwrap(),
            bad.to_str().unwrap(),
        ])
        .output()
        .expect("run rexlang artifact check");
    assert_eq!(output.status.code(), Some(1));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(
        stdout,
        format!("OK {}\n", good.display()),
        "stdout: {stdout}"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("batch_bad.rex.json"),
        "stderr was: {stderr}"
    );
}

/// Dogfood: every golden artifact under `tests/conformance/artifacts/` must
/// pass the checker, keeping the validator honest as the wire format evolves.
#[test]
fn artifact_check_passes_on_every_golden_conformance_artifact() {
    let dir = workspace_root().join("tests/conformance/artifacts");
    let mut artifacts: Vec<PathBuf> = std::fs::read_dir(&dir)
        .expect("list golden artifacts")
        .map(|entry| entry.expect("dir entry").path())
        .filter(|path| path.extension().and_then(|e| e.to_str()) == Some("json"))
        .collect();
    artifacts.sort();
    assert!(
        artifacts.len() >= 6,
        "expected the full golden artifact set, found {}: {:?}",
        artifacts.len(),
        artifacts
    );
    let mut command = rexlang();
    command.arg("artifact").arg("check");
    for artifact in &artifacts {
        command.arg(artifact.to_str().unwrap());
    }
    let output = command.output().expect("run rexlang artifact check");
    assert!(
        output.status.success(),
        "golden artifacts must pass: stderr: {:?}",
        String::from_utf8_lossy(&output.stderr)
    );
    let expected: String = artifacts
        .iter()
        .map(|path| format!("OK {}\n", path.display()))
        .collect();
    assert_eq!(String::from_utf8_lossy(&output.stdout), expected);
}
