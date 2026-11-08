//! `rexlang` CLI tests: ir.

use crate::support::*;

#[test]
fn ir_prints_parseable_json_to_stdout() {
    let path = write_source("ir.mox", GOOD);
    let output = rexlang()
        .args(["ir", path.to_str().unwrap()])
        .output()
        .expect("run rexlang ir");
    assert!(
        output.status.success(),
        "stderr: {:?}",
        String::from_utf8_lossy(&output.stderr)
    );
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
    assert!(
        output.status.success(),
        "stderr: {:?}",
        String::from_utf8_lossy(&output.stderr)
    );
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
