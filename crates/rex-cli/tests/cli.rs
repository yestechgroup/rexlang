//! End-to-end CLI tests: run the `rexlang` binary as a subprocess.

use std::path::PathBuf;
use std::process::Command;

const GOOD: &str = "package demo\n\nclass Book { String title }\n";
const BAD: &str = "package demo\n\nclass Book { Book oops }\n";

fn rexlang() -> Command {
    Command::new(env!("CARGO_BIN_EXE_rexlang"))
}

/// A unique scratch directory per test process.
fn scratch_dir() -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rex-cli-tests-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

fn write_source(name: &str, contents: &str) -> PathBuf {
    let path = scratch_dir().join(name);
    std::fs::write(&path, contents).expect("write source");
    path
}

#[test]
fn check_succeeds_on_valid_source() {
    let path = write_source("good.mox", GOOD);
    let output = rexlang()
        .args(["check", path.to_str().unwrap()])
        .output()
        .expect("run rexlang check");
    assert!(output.status.success(), "stderr: {:?}", String::from_utf8_lossy(&output.stderr));
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        format!("OK {}\n", path.display())
    );
    assert!(String::from_utf8_lossy(&output.stderr).is_empty());
}

#[test]
fn check_fails_on_invalid_source_with_diagnostic_on_stderr() {
    let path = write_source("bad.mox", BAD);
    let output = rexlang()
        .args(["check", path.to_str().unwrap()])
        .output()
        .expect("run rexlang check");
    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("feature 'oops' has class type 'Book'"),
        "stderr was: {stderr}"
    );
    assert!(
        stderr.contains("did you mean `contains Book[..] oops`"),
        "stderr was: {stderr}"
    );
}

#[test]
fn ir_prints_parseable_json_to_stdout() {
    let path = write_source("ir.mox", GOOD);
    let output = rexlang()
        .args(["ir", path.to_str().unwrap()])
        .output()
        .expect("run rexlang ir");
    assert!(output.status.success(), "stderr: {:?}", String::from_utf8_lossy(&output.stderr));
    let json = String::from_utf8_lossy(&output.stdout);
    let model = rex_ir::Model::from_json(&json).expect("stdout is rex-ir JSON");
    assert_eq!(model.format_version, rex_ir::FORMAT_VERSION);
    assert_eq!(model.packages[0].name, "demo");
    assert_eq!(model.packages[0].classes[0].name, "Book");
}

#[test]
fn ir_writes_json_to_output_file() {
    let path = write_source("ir_out.mox", GOOD);
    let out = scratch_dir().join("ir_out.json");
    let output = rexlang()
        .args(["ir", path.to_str().unwrap(), "-o", out.to_str().unwrap()])
        .output()
        .expect("run rexlang ir");
    assert!(output.status.success(), "stderr: {:?}", String::from_utf8_lossy(&output.stderr));
    assert!(String::from_utf8_lossy(&output.stdout).is_empty());
    let json = std::fs::read_to_string(&out).expect("read IR output");
    let model = rex_ir::Model::from_json(&json).expect("file is rex-ir JSON");
    assert_eq!(model.packages[0].name, "demo");
}

#[test]
fn ir_fails_on_invalid_source() {
    let path = write_source("ir_bad.mox", BAD);
    let output = rexlang()
        .args(["ir", path.to_str().unwrap()])
        .output()
        .expect("run rexlang ir");
    assert_eq!(output.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&output.stderr).contains("has class type"));
}

#[test]
fn missing_file_is_a_clean_error() {
    let missing = scratch_dir().join("does-not-exist.mox");
    let output = rexlang()
        .args(["check", missing.to_str().unwrap()])
        .output()
        .expect("run rexlang check");
    assert_eq!(output.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&output.stderr).starts_with("error: cannot read"));
}

#[test]
fn gen_rust_writes_generated_code() {
    let path = write_source("gen.mox", GOOD);
    let out = scratch_dir().join(format!("gen-out-{}", std::process::id()));
    let output = rexlang()
        .args([
            "gen",
            "rust",
            path.to_str().unwrap(),
            "-o",
            out.to_str().unwrap(),
        ])
        .output()
        .expect("run rexlang gen rust");
    assert!(
        output.status.success(),
        "stderr: {:?}",
        String::from_utf8_lossy(&output.stderr)
    );
    let models = std::fs::read_to_string(out.join("models.rs")).expect("models.rs written");
    assert!(models.contains("pub struct Resource"));
    assert!(models.contains("pub struct Book"));
    assert!(models.contains("pub fn to_instance_json"));
}

#[test]
fn gen_rust_fails_on_invalid_source() {
    let path = write_source("gen_bad.mox", BAD);
    let out = scratch_dir().join(format!("gen-bad-out-{}", std::process::id()));
    let output = rexlang()
        .args([
            "gen",
            "rust",
            path.to_str().unwrap(),
            "-o",
            out.to_str().unwrap(),
        ])
        .output()
        .expect("run rexlang gen rust");
    assert_eq!(output.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&output.stderr).contains("has class type"));
}
