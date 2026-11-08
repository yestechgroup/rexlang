//! `rexlang` CLI tests: import_sigil.

use std::path::PathBuf;

use crate::support::*;

// --- `import sigil` -----------------------------------------------------------

/// Writes a `.mox` file plus the rosetta files it imports: the named file
/// declares a namespace import satisfied by a sibling in a subdirectory
/// (discovered by the CLI's directory walk), mirroring sigil's own
/// multi-file projects.
fn write_sigil_project() -> PathBuf {
    let mox = scratch_dir().join("with_sigil.mox");
    std::fs::create_dir_all(scratch_dir().join("oracle/extra")).expect("create rosetta dirs");
    std::fs::write(
        scratch_dir().join("oracle/trade.rosetta"),
        concat!(
            "namespace oracle.basic\n\n",
            "import oracle.extra.*\n\n",
            "type Trade:\n",
            "    id string (1..1)\n",
            "    extra Extra (1..1)\n"
        ),
    )
    .expect("write named rosetta");
    std::fs::write(
        scratch_dir().join("oracle/extra/extra.rosetta"),
        "namespace oracle.extra\n\ntype Extra:\n    x int (1..1)\n",
    )
    .expect("write candidate rosetta");
    std::fs::write(
        &mox,
        concat!(
            "package demo\n\n",
            "import sigil \"oracle/trade.rosetta\"\n\n",
            "class Book { refers oracle.basic.Trade about }\n"
        ),
    )
    .expect("write mox");
    mox
}

#[test]
fn check_passes_on_a_mox_with_transitively_discovered_sigil_imports() {
    let mox = write_sigil_project();
    let output = rexlang()
        .args(["check", mox.to_str().unwrap()])
        .output()
        .expect("run rexlang check");
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        format!("OK {}\n", mox.display())
    );
}

#[test]
fn ir_contains_the_synthetic_sigil_packages() {
    let mox = write_sigil_project();
    let output = rexlang()
        .args(["ir", mox.to_str().unwrap()])
        .output()
        .expect("run rexlang ir");
    assert!(output.status.success());
    let ir: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("the IR artifact on stdout");
    let names: Vec<&str> = ir["packages"]
        .as_array()
        .expect("packages")
        .iter()
        .map(|package| package["name"].as_str().expect("package name"))
        .collect();
    assert_eq!(names, vec!["demo", "oracle.basic", "oracle.extra"]);
    assert_eq!(
        ir["packages"][0]["classes"][0]["features"][0]["type"]["value"],
        serde_json::json!({"package": "oracle.basic", "name": "Trade"})
    );
    assert_eq!(
        ir["packages"][1]["classes"][0]["features"][0]["type"],
        serde_json::json!({"type": "primitive", "value": "string"})
    );
}

#[test]
fn check_fails_with_a_clear_error_when_a_sigil_file_is_missing() {
    let mox = scratch_dir().join("missing_sigil.mox");
    std::fs::write(
        &mox,
        "package demo\n\nimport sigil \"oracle/gone.rosetta\"\n\nclass C {}\n",
    )
    .expect("write mox");
    let output = rexlang()
        .args(["check", mox.to_str().unwrap()])
        .output()
        .expect("run rexlang check");
    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.starts_with("error:"),
        "clean error, no panic: {stderr}"
    );
    assert!(
        stderr.contains("gone.rosetta"),
        "the resolved import path must be named: {stderr}"
    );
    assert!(!stderr.contains("panicked"), "must not panic: {stderr}");
}

#[test]
fn a_rosetta_parse_error_renders_against_the_rosetta_source() {
    let mox = scratch_dir().join("broken_sigil.mox");
    std::fs::create_dir_all(scratch_dir().join("oracle")).expect("create rosetta dir");
    std::fs::write(
        scratch_dir().join("oracle/broken.rosetta"),
        "namespace oracle.basic\n\ntype Trade:\n    id (1..1)\n",
    )
    .expect("write broken rosetta");
    std::fs::write(
        &mox,
        "package demo\n\nimport sigil \"oracle/broken.rosetta\"\n\nclass C {}\n",
    )
    .expect("write mox");
    let output = rexlang()
        .args(["check", mox.to_str().unwrap()])
        .output()
        .expect("run rexlang check");
    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("broken.rosetta"),
        "the diagnostic is tagged with the rosetta path: {stderr}"
    );
    assert!(
        stderr.contains("expected"),
        "the sigil parse message surfaces: {stderr}"
    );
}

const SIGIL_GOLDEN: &str = include_str!("../../../../tests/conformance/sigil/basic.mox.json");

#[test]
fn check_succeeds_on_the_sigil_conformance_fixture() {
    let path = workspace_root().join("tests/conformance/sigil/basic.mox");
    let output = rexlang()
        .args(["check", path.to_str().unwrap()])
        .output()
        .expect("run rexlang check");
    assert!(
        output.status.success(),
        "stderr: {:?}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        format!("OK {}\n", path.display())
    );
    assert!(String::from_utf8_lossy(&output.stderr).is_empty());
}

#[test]
fn ir_sigil_file_emits_the_golden_artifact() {
    let path = workspace_root().join("tests/conformance/sigil/basic.mox");
    let output = rexlang()
        .args(["ir", path.to_str().unwrap()])
        .output()
        .expect("run rexlang ir");
    assert!(
        output.status.success(),
        "stderr: {:?}",
        String::from_utf8_lossy(&output.stderr)
    );
    // `ir` prints the artifact with one trailing newline (println!).
    let emitted = String::from_utf8_lossy(&output.stdout);
    assert_eq!(
        emitted.trim_end_matches('\n'),
        SIGIL_GOLDEN.trim_end_matches('\n'),
        "the emitted artifact matches the golden"
    );
}

#[test]
fn fmt_check_passes_on_the_canonical_sigil_fixture() {
    let path = workspace_root().join("tests/conformance/sigil/basic.mox");
    let output = rexlang()
        .args(["fmt", "--check", path.to_str().unwrap()])
        .output()
        .expect("run rexlang fmt --check");
    assert!(
        output.status.success(),
        "stdout: {:?}",
        String::from_utf8_lossy(&output.stdout)
    );
    assert!(String::from_utf8_lossy(&output.stdout).is_empty());
}
