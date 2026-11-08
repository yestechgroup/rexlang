//! `rexlang` CLI tests: import_schema.

use std::path::PathBuf;

use crate::support::*;

// --- `import schema` -----------------------------------------------------------

/// Writes a `.mox` file plus the JSON schemas it imports, resolved relative
/// to the file's directory like the CLI does.
fn write_import_schema_pair() -> PathBuf {
    let mox = scratch_dir().join("with_imports.mox");
    std::fs::create_dir_all(scratch_dir().join("schemas")).expect("create schemas dir");
    std::fs::write(
        scratch_dir().join("schemas/todo_item.json"),
        r#"{"title": "Todo item"}"#,
    )
    .expect("write schema");
    std::fs::write(
        &mox,
        concat!(
            "package demo\n\n",
            "import schema \"schemas/todo_item.json\" as TodoItem\n\n",
            "class Wrapper { refers TodoItem item }\n"
        ),
    )
    .expect("write mox");
    mox
}

#[test]
fn check_passes_on_a_mox_with_resolvable_schema_imports() {
    let mox = write_import_schema_pair();
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
fn ir_contains_the_nominal_imported_class() {
    let mox = write_import_schema_pair();
    let output = rexlang()
        .args(["ir", mox.to_str().unwrap()])
        .output()
        .expect("run rexlang ir");
    assert!(output.status.success());
    let ir: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("the IR artifact on stdout");
    let classes: Vec<&str> = ir["packages"][0]["classes"]
        .as_array()
        .expect("classes")
        .iter()
        .map(|class| class["name"].as_str().expect("class name"))
        .collect();
    assert_eq!(classes, vec!["Wrapper", "TodoItem"]);
    assert_eq!(
        ir["packages"][0]["classes"][0]["features"][0]["type"]["value"]["name"],
        "TodoItem"
    );
}

#[test]
fn check_fails_with_a_clear_error_when_an_import_file_is_missing() {
    let mox = scratch_dir().join("missing_import.mox");
    std::fs::write(
        &mox,
        "package demo\n\nimport schema \"schemas/gone.json\" as Gone\n\nclass C {}\n",
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
        stderr.contains("gone.json"),
        "the import path must be named: {stderr}"
    );
    assert!(!stderr.contains("panicked"), "must not panic: {stderr}");
}

#[test]
fn check_rejects_invalid_import_json_with_the_path() {
    let mox = scratch_dir().join("bad_json_import.mox");
    std::fs::create_dir_all(scratch_dir().join("schemas")).expect("create schemas dir");
    std::fs::write(scratch_dir().join("schemas/bad.json"), "{ not json").expect("write schema");
    std::fs::write(
        &mox,
        "package demo\n\nimport schema \"schemas/bad.json\" as Bad\n\nclass C {}\n",
    )
    .expect("write mox");
    let output = rexlang()
        .args(["check", mox.to_str().unwrap()])
        .output()
        .expect("run rexlang check");
    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("bad.json") && stderr.contains("JSON"),
        "the error must name the path and the JSON problem: {stderr}"
    );
}
