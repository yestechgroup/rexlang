//! `rexlang` CLI tests: verification receipts (`rexlang verify`, issues
//! #197/#206/#207).
//!
//! The tamper suite is #206's exit gate: every mutation of a committed
//! receipt must fail `--check` with the **exact field** named. The
//! freshness suite is #207's: a source or register change without
//! regeneration fails `--check-fresh` with the command to run.

use std::path::{Path, PathBuf};

use crate::support::*;

/// The flagship receipt and the artifacts it names, all under
/// `examples/artifacts/`.
const FLAGSHIP: &str = "examples/artifacts/support.receipt.json";

/// The nine `support` surfaces the flagship receipt covers, in the sorted
/// order the receipt records them (the canonical document sorts `inputs`).
const SUPPORT_INPUTS: [&str; 9] = [
    "examples/support.actor",
    "examples/support.cmmn",
    "examples/support.ddd",
    "examples/support.deploy",
    "examples/support.dmn",
    "examples/support.evt",
    "examples/support.ifml",
    "examples/support.mox",
    "examples/support.uc",
];

fn receipt_path(relative: &str) -> PathBuf {
    workspace_root().join(relative)
}

fn read_receipt(relative: &str) -> String {
    std::fs::read_to_string(receipt_path(relative))
        .unwrap_or_else(|error| panic!("missing committed receipt {relative}: {error}"))
}

fn parse_receipt(relative: &str) -> serde_json::Value {
    serde_json::from_str(&read_receipt(relative)).expect("committed receipt is JSON")
}

/// Suffix of a receipt-relative artifact path, e.g. `support.ddd.json`.
fn artifact_names(receipt: &serde_json::Value) -> Vec<String> {
    receipt["artifacts"]
        .as_array()
        .expect("artifacts array")
        .iter()
        .filter_map(|artifact| artifact["path"].as_str().map(str::to_string))
        .collect()
}

// --- the flagship receipt -----------------------------------------------------

#[test]
fn the_flagship_receipt_reports_the_register_and_its_closure() {
    let receipt = parse_receipt(FLAGSHIP);
    assert_eq!(receipt["formatVersion"], 1);
    assert_eq!(receipt["kind"], "verificationReceipt");
    assert_eq!(receipt["subject"], "support");
    assert_eq!(receipt["generator"]["name"], "rexlang");
    assert_eq!(
        receipt["generator"]["version"],
        env!("CARGO_PKG_VERSION"),
        "the receipt records the rexVersion provenance of its generator"
    );

    // The inputs are exactly the nine support surfaces, sorted.
    let inputs: Vec<&str> = receipt["inputs"]
        .as_array()
        .expect("inputs")
        .iter()
        .map(|input| input.as_str().expect("input paths are strings"))
        .collect();
    assert_eq!(inputs, SUPPORT_INPUTS, "the generation command's inputs");

    // The derived closure: every input plus the imports the compile read,
    // including the pattern-library modules the flow pulls in transitively
    // and the external decision table the `.dmn` model names.
    let sources: Vec<&str> = receipt["sources"]
        .as_array()
        .expect("sources")
        .iter()
        .map(|source| source["path"].as_str().expect("source path"))
        .collect();
    for required in [
        "patterns/ifml/communication.ifml",
        "patterns/ifml/workflow.ifml",
        "patterns/ifml/collections.ifml",
        "patterns/ifml/data.ifml",
        "patterns/ifml/navigation.ifml",
        "examples/tables/refund-authority.md",
    ] {
        assert!(
            sources.contains(&required),
            "the closure must include {required}; got {sources:?}"
        );
    }
    for input in SUPPORT_INPUTS {
        assert!(
            sources.contains(&input),
            "{input} must be a recorded source"
        );
    }
    for source in &sources {
        assert!(
            receipt_path(source).is_file(),
            "{source} must be a real workspace file"
        );
        assert!(!source.starts_with('/'), "no absolute paths: {source}");
        assert!(!source.contains(".."), "no parent escapes: {source}");
    }
    assert!(
        !sources.contains(&"patterns/ifml/social.ifml"),
        "the closure is precise, not the whole pattern library: {sources:?}"
    );

    // The register is reported, never reinterpreted: every row, the closed
    // status vocabulary, and the interim `mutantKilled: null`.
    assert_eq!(
        receipt["register"]["source"],
        "docs/verification/register.json"
    );
    assert_eq!(receipt["register"]["reviewedAt"], "2026-11-14");
    let rows = receipt["register"]["rows"].as_array().expect("rows");
    assert_eq!(rows.len(), 17, "V1..V17 are all reported");
    assert_eq!(rows[0]["id"], "V1");
    for row in rows {
        let status = row["status"].as_str().expect("status");
        assert!(
            matches!(status, "proved" | "tested" | "argued" | "assumed"),
            "row {} has status {status:?} outside the register vocabulary",
            row["id"]
        );
        assert_ne!(status, "verified");
        assert!(
            row["mutantKilled"].is_null(),
            "row {} must record a null mutant fact until #208",
            row["id"]
        );
        assert!(!row["claim"].as_str().unwrap_or_default().is_empty());
        assert!(!row["evidence"].as_array().expect("evidence").is_empty());
    }
    assert_eq!(receipt["limits"], serde_json::json!(["mutants-unrecorded"]));

    // Trusted assumptions: every register item, mapped to a real row.
    let trusted = receipt["trusted"].as_array().expect("trusted");
    assert!(!trusted.is_empty(), "the receipt lists the trusted set");
    for entry in trusted {
        let row = entry["row"].as_str().expect("row id");
        assert!(
            rows.iter().any(|candidate| candidate["id"] == row),
            "trusted item {} references missing row {row}",
            entry["item"]
        );
    }
    let items: Vec<&str> = trusted
        .iter()
        .map(|entry| entry["item"].as_str().expect("item"))
        .collect();
    assert!(
        items.contains(&"push_containment"),
        "the trusted mutation primitives are listed: {items:?}"
    );
}

#[test]
fn the_flagship_artifact_hashes_equal_the_committed_goldens() {
    let receipt = parse_receipt(FLAGSHIP);
    let artifacts = receipt["artifacts"].as_array().expect("artifacts");
    assert_eq!(
        artifacts.len(),
        SUPPORT_INPUTS.len(),
        "one artifact per input"
    );
    let mut path_bearing = 0;
    for artifact in artifacts {
        let bytes = match artifact["path"].as_str() {
            Some(name) => {
                path_bearing += 1;
                std::fs::read(receipt_path("examples/artifacts").join(name).as_path())
                    .unwrap_or_else(|error| panic!("missing golden {name}: {error}"))
            }
            None => {
                // Digest-only: the domain and actor artifacts ship no
                // example golden.
                assert!(
                    matches!(artifact["kind"].as_str(), Some("irModel" | "actorModel")),
                    "only the golden-less domain/actor artifacts may be digest-only: {artifact}"
                );
                continue;
            }
        };
        assert_eq!(
            artifact["sha256"].as_str().expect("sha256"),
            rex_evidence::receipt::sha256_hex(&bytes),
            "artifact {} drifted from its committed golden",
            artifact["path"]
        );
    }
    assert_eq!(
        path_bearing, 7,
        "the seven golden-bearing surfaces name their files"
    );
}

// --- round trip + determinism -------------------------------------------------

#[test]
fn verify_emits_a_receipt_that_check_accepts() {
    let dir = scratch_dir();
    // The artifacts must live next to the receipt for the golden assertion
    // and the hash re-computation; the scratch dir gets copies.
    for name in artifact_names(&parse_receipt(FLAGSHIP)) {
        std::fs::copy(
            workspace_root().join("examples/artifacts").join(&name),
            dir.join(&name),
        )
        .expect("copy artifact");
    }
    let out = dir.join("support.receipt.json");
    let status = rexlang()
        .args(["verify"])
        .args(SUPPORT_INPUTS)
        .arg("-o")
        .arg(&out)
        .current_dir(workspace_root())
        .status()
        .expect("run verify");
    assert!(status.success(), "verify must succeed");

    let check = rexlang()
        .args(["verify", "--check"])
        .arg(&out)
        .current_dir(&dir)
        .output()
        .expect("run verify --check");
    assert!(
        check.status.success(),
        "--check must accept the generated receipt: {}",
        String::from_utf8_lossy(&check.stderr)
    );
    assert!(
        String::from_utf8_lossy(&check.stdout).starts_with("OK "),
        "the success line names the receipt"
    );
}

#[test]
fn two_emissions_are_byte_identical() {
    let dir = scratch_dir();
    for name in artifact_names(&parse_receipt(FLAGSHIP)) {
        std::fs::copy(
            workspace_root().join("examples/artifacts").join(&name),
            dir.join(&name),
        )
        .expect("copy artifact");
    }
    let mut texts = Vec::new();
    for index in 0..2 {
        let out = dir.join(format!("receipt-{index}.json"));
        let status = rexlang()
            .args(["verify"])
            .args(SUPPORT_INPUTS)
            .arg("-o")
            .arg(&out)
            .current_dir(workspace_root())
            .status()
            .expect("run verify");
        assert!(status.success());
        texts.push(std::fs::read(&out).expect("read receipt"));
    }
    assert_eq!(
        texts[0], texts[1],
        "a receipt holds nothing that changes by itself"
    );
    assert_eq!(
        texts[0],
        std::fs::read(receipt_path(FLAGSHIP)).expect("read flagship"),
        "the scratch generation reproduces the committed receipt byte-for-byte"
    );
}

// --- the tamper suite (#206's exit gate) --------------------------------------

/// Copies the flagship receipt and its artifacts into a fresh scratch dir,
/// then hands back the receipt path. Every tamper test mutates this copy.
fn copied_flagship() -> (PathBuf, PathBuf) {
    let dir = scratch_dir();
    let target = dir.join("support.receipt.json");
    std::fs::copy(receipt_path(FLAGSHIP), &target).expect("copy receipt");
    for name in artifact_names(&parse_receipt(FLAGSHIP)) {
        std::fs::copy(
            workspace_root().join("examples/artifacts").join(&name),
            dir.join(&name),
        )
        .expect("copy artifact");
    }
    (dir, target)
}

/// Runs `verify --check` and returns `stderr`; asserts the failure names
/// `field`.
fn assert_tamper_fails_named(dir: &Path, receipt: &Path, field: &str) -> String {
    let output = rexlang()
        .args(["verify", "--check"])
        .arg(receipt)
        .current_dir(dir)
        .output()
        .expect("run verify --check");
    assert!(
        !output.status.success(),
        "a tampered receipt must fail --check ({field})"
    );
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    assert!(
        stderr.contains(field),
        "stderr must name {field:?}; got:\n{stderr}"
    );
    stderr
}

/// Rewrites a tampered copy of the flagship receipt into `dir` and returns
/// its path. `mutate` edits the parsed document.
fn tampered(dir: &Path, mutate: impl FnOnce(&mut serde_json::Value)) -> PathBuf {
    let mut value = parse_receipt(FLAGSHIP);
    mutate(&mut value);
    let path = dir.join("tampered.receipt.json");
    std::fs::write(
        &path,
        serde_json::to_string_pretty(&value).expect("serialize"),
    )
    .expect("write tampered receipt");
    path
}

#[test]
fn check_accepts_the_untampered_copy() {
    let (dir, receipt) = copied_flagship();
    let output = rexlang()
        .args(["verify", "--check"])
        .arg(&receipt)
        .current_dir(&dir)
        .output()
        .expect("run verify --check");
    assert!(
        output.status.success(),
        "the copy must validate: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn a_flipped_status_fails_naming_the_row_field() {
    // V4 is `tested` with only code/test evidence: claiming `proved` is
    // detectable from the receipt alone (the status↔evidence join), even
    // though both words are in the closed vocabulary.
    let (dir, _) = copied_flagship();
    let path = tampered(&dir, |value| {
        value["register"]["rows"][3]["status"] = serde_json::json!("proved");
    });
    let stderr = assert_tamper_fails_named(&dir, &path, "register.rows[3].status");
    assert!(
        stderr.contains("no proof or mirror evidence"),
        "the join names why: {stderr}"
    );
}

#[test]
fn an_edited_artifact_hash_fails_naming_the_hash_field() {
    let (dir, _) = copied_flagship();
    let path = tampered(&dir, |value| {
        value["artifacts"][3]["sha256"] =
            serde_json::json!("0000000000000000000000000000000000000000000000000000000000000000");
    });
    let stderr = assert_tamper_fails_named(&dir, &path, "artifacts[3].sha256");
    assert!(
        stderr.contains("hashes to"),
        "the message names the actual hash too: {stderr}"
    );
}

#[test]
fn a_verified_status_fails_with_the_reason() {
    let (dir, _) = copied_flagship();
    let path = tampered(&dir, |value| {
        value["register"]["rows"][0]["status"] = serde_json::json!("verified");
    });
    let stderr = assert_tamper_fails_named(&dir, &path, "register.rows[0].status");
    assert!(
        stderr.contains("never a legal status"),
        "the failure explains why: {stderr}"
    );
}

#[test]
fn a_root_verified_claim_is_an_unknown_field() {
    let (dir, _) = copied_flagship();
    let path = tampered(&dir, |value| {
        value["verified"] = serde_json::json!(true);
    });
    let stderr = assert_tamper_fails_named(&dir, &path, "verified");
    assert!(
        stderr.contains("unknown field"),
        "a receipt is a closed document: {stderr}"
    );
}

#[test]
fn a_bumped_format_version_fails_by_name() {
    let (dir, _) = copied_flagship();
    let path = tampered(&dir, |value| {
        value["formatVersion"] = serde_json::json!(2);
    });
    let stderr = assert_tamper_fails_named(&dir, &path, "formatVersion");
    assert!(
        stderr.contains("newer rex-evidence"),
        "a higher version names the upgrade path: {stderr}"
    );
}

#[test]
fn a_deleted_artifact_fails_naming_the_path_field() {
    let (dir, _) = copied_flagship();
    let receipt = parse_receipt(FLAGSHIP);
    let artifacts = receipt["artifacts"].as_array().expect("artifacts");
    let (index, name) = artifacts
        .iter()
        .enumerate()
        .find_map(|(index, artifact)| artifact["path"].as_str().map(|name| (index, name)))
        .expect("a path-bearing artifact");
    std::fs::remove_file(dir.join(name)).expect("delete artifact");
    assert_tamper_fails_named(
        &dir,
        &dir.join("support.receipt.json"),
        &format!("artifacts[{index}].path"),
    );
}

#[test]
fn an_absolute_artifact_path_fails_naming_the_path_field() {
    let (dir, _) = copied_flagship();
    let absolute = dir.join("support.ddd.json").display().to_string();
    let path = tampered(&dir, |value| {
        let artifacts = value["artifacts"].as_array_mut().expect("artifacts");
        let artifact = artifacts
            .iter_mut()
            .find(|artifact| artifact["path"] == serde_json::json!("support.ddd.json"))
            .expect("the design artifact");
        artifact["path"] = serde_json::json!(absolute);
    });
    assert_tamper_fails_named(&dir, &path, "artifacts[");
    assert_tamper_fails_named(&dir, &path, "is absolute");
}

#[test]
fn an_absolute_source_path_fails_naming_the_source_field() {
    let (dir, _) = copied_flagship();
    let path = tampered(&dir, |value| {
        value["sources"][0]["path"] = serde_json::json!("/etc/passwd");
    });
    assert_tamper_fails_named(&dir, &path, "sources[0].path");
}

// --- standalone validation (#206) ---------------------------------------------

#[test]
fn check_works_with_no_workspace_knowledge() {
    // A temp dir holding only the receipt and the artifacts it names: no
    // register, no models, no `Cargo.toml` — the auditor's machine.
    let dir = scratch_dir();
    let receipt = dir.join("copy.receipt.json");
    std::fs::copy(receipt_path(FLAGSHIP), &receipt).expect("copy receipt");
    for name in artifact_names(&parse_receipt(FLAGSHIP)) {
        std::fs::copy(
            workspace_root().join("examples/artifacts").join(&name),
            dir.join(&name),
        )
        .expect("copy artifact");
    }
    let output = rexlang()
        .args(["verify", "--check"])
        .arg(&receipt)
        .current_dir(&dir)
        .env_remove("CARGO_MANIFEST_DIR")
        .output()
        .expect("run verify --check");
    assert!(
        output.status.success(),
        "standalone --check must pass: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

// --- freshness (#207) ---------------------------------------------------------

/// A self-contained copy of everything the flagship receipt names, laid out
/// like the repository so relative paths (and the register path) resolve.
fn copied_workspace() -> PathBuf {
    let root = scratch_dir();
    for input in SUPPORT_INPUTS {
        let target = root.join(input);
        std::fs::create_dir_all(target.parent().unwrap()).expect("mkdir");
        std::fs::copy(workspace_root().join(input), &target).expect("copy input");
    }
    for name in artifact_names(&parse_receipt(FLAGSHIP)) {
        let target = root.join("examples/artifacts").join(&name);
        std::fs::create_dir_all(target.parent().unwrap()).expect("mkdir");
        std::fs::copy(
            workspace_root().join("examples/artifacts").join(&name),
            target,
        )
        .expect("copy artifact");
    }
    for pattern in [
        "collections",
        "communication",
        "data",
        "navigation",
        "workflow",
    ] {
        let name = format!("patterns/ifml/{pattern}.ifml");
        let target = root.join(&name);
        std::fs::create_dir_all(target.parent().unwrap()).expect("mkdir");
        std::fs::copy(workspace_root().join(&name), target).expect("copy pattern");
    }
    let receipt = root.join(FLAGSHIP);
    std::fs::create_dir_all(receipt.parent().unwrap()).expect("mkdir artifacts");
    std::fs::copy(workspace_root().join(FLAGSHIP), receipt).expect("copy receipt");
    let tables = root.join("examples/tables");
    std::fs::create_dir_all(&tables).expect("mkdir tables");
    std::fs::copy(
        workspace_root().join("examples/tables/refund-authority.md"),
        tables.join("refund-authority.md"),
    )
    .expect("copy table");
    let register = root.join("docs/verification");
    std::fs::create_dir_all(&register).expect("mkdir register");
    std::fs::copy(
        workspace_root().join("docs/verification/register.json"),
        register.join("register.json"),
    )
    .expect("copy register");
    root
}

fn verify_fresh(root: &Path, receipt: &str) -> std::process::Output {
    rexlang()
        .args(["verify", "--check-fresh", receipt])
        .current_dir(root)
        .output()
        .expect("run verify --check-fresh")
}

#[test]
fn the_committed_receipt_is_fresh() {
    let output = verify_fresh(&workspace_root(), FLAGSHIP);
    assert!(
        output.status.success(),
        "--check-fresh must accept the committed receipt: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn the_copied_receipt_is_fresh_before_anything_changes() {
    let root = copied_workspace();
    let output = verify_fresh(&root, FLAGSHIP);
    assert!(
        output.status.success(),
        "the copy must be fresh: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn a_touched_source_makes_the_receipt_stale() {
    let root = copied_workspace();
    let source = root.join("examples/support.ddd");
    let mut text = std::fs::read_to_string(&source).expect("read source");
    text.push_str("\n// a comment-only edit still changes the source bytes\n");
    std::fs::write(&source, text).expect("touch source");

    // A comment does not change the design artifact, so the receipt's
    // source hash is what moved — exactly the staleness class the gate
    // exists for.
    let output = verify_fresh(&root, FLAGSHIP);
    assert!(!output.status.success(), "a touched source is stale");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("stale receipt"), "{stderr}");
    let expected = format!("rexlang verify {} -o {FLAGSHIP}", SUPPORT_INPUTS.join(" "));
    assert!(
        stderr.contains(&expected),
        "the failure prints the exact regenerate command ({expected}): {stderr}"
    );
}

#[test]
fn a_touched_decision_table_makes_the_receipt_stale() {
    let root = copied_workspace();
    let table = root.join("examples/tables/refund-authority.md");
    let mut text = std::fs::read_to_string(&table).expect("read table");
    text.push('\n');
    std::fs::write(&table, text).expect("touch table");
    let output = verify_fresh(&root, FLAGSHIP);
    assert!(
        !output.status.success(),
        "a decision-table change invalidates the receipt"
    );
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("regenerate it with"),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn a_touched_register_makes_the_receipt_stale() {
    let root = copied_workspace();
    let register = root.join("docs/verification/register.json");
    let mut text = std::fs::read_to_string(&register).expect("read register");
    text.push('\n');
    std::fs::write(&register, text).expect("touch register");
    let output = verify_fresh(&root, FLAGSHIP);
    assert!(!output.status.success(), "a register edit is stale");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("stale receipt"), "{stderr}");
    assert!(stderr.contains("regenerate it with"), "{stderr}");
}

#[test]
fn a_document_change_that_is_not_an_input_does_not_invalidate() {
    let root = copied_workspace();
    std::fs::write(root.join("notes.md"), "release notes are not a source\n")
        .expect("write a non-input document");
    let output = verify_fresh(&root, FLAGSHIP);
    assert!(
        output.status.success(),
        "docs and non-inputs never invalidate a receipt: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

// --- exit discipline ----------------------------------------------------------

#[test]
fn check_rejects_a_missing_receipt_file() {
    let dir = scratch_dir();
    let output = rexlang()
        .args(["verify", "--check", "no-such-receipt.json"])
        .current_dir(&dir)
        .output()
        .expect("run verify --check");
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("cannot read"),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn generation_requires_an_output_path() {
    let output = rexlang()
        .args(["verify"])
        .args(SUPPORT_INPUTS)
        .current_dir(workspace_root())
        .output()
        .expect("run verify");
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("-o <receipt.json>"),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn check_and_check_fresh_are_mutually_exclusive() {
    let output = rexlang()
        .args(["verify", "--check", "a.json", "--check-fresh", "b.json"])
        .output()
        .expect("run verify");
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("not both"),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}
