//! `rexlang` CLI tests: multi.

use crate::support::*;

// --- multi-file and directory inputs ------------------------------------------

const MULTI_A: &str = "package alpha\n\nclass Book { String title }\n";
const MULTI_B: &str = "package beta\n\nclass Shelf { refers alpha.Book[] links }\n";

#[test]
fn check_accepts_multiple_files_and_reports_per_file() {
    let a = write_source("multi_a.mox", MULTI_A);
    let b = write_source("multi_b.mox", MULTI_B);
    let output = rexlang()
        .args(["check", b.to_str().unwrap(), a.to_str().unwrap()])
        .output()
        .expect("run rexlang check");
    assert!(
        output.status.success(),
        "stderr: {:?}",
        String::from_utf8_lossy(&output.stderr)
    );
    // One OK line per file, in sorted-path order (not argument order).
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        format!("OK {}\nOK {}\n", a.display(), b.display())
    );
    assert!(String::from_utf8_lossy(&output.stderr).is_empty());
}

#[test]
fn check_multiple_files_renders_the_failing_file_diagnostics() {
    let good = write_source("multi_good.mox", MULTI_A);
    // Uniquely named class: a shared bare name (like `Book`) would be
    // ambiguous across packages in a multi-file compile.
    let bad = write_source(
        "multi_bad.mox",
        "package demo\n\nclass Widget { Widget w }\n",
    );
    let output = rexlang()
        .args(["check", good.to_str().unwrap(), bad.to_str().unwrap()])
        .output()
        .expect("run rexlang check");
    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("feature 'w' has class type 'Widget'"),
        "stderr was: {stderr}"
    );
    assert!(
        stderr.contains("multi_bad.mox"),
        "the failing file's path must name the report: {stderr}"
    );
}

#[test]
fn ir_accepts_multiple_files_and_emits_one_model() {
    let a = write_source("ir_multi_a.mox", MULTI_A);
    let b = write_source("ir_multi_b.mox", MULTI_B);
    let output = rexlang()
        .args(["ir", a.to_str().unwrap(), b.to_str().unwrap()])
        .output()
        .expect("run rexlang ir");
    assert!(
        output.status.success(),
        "stderr: {:?}",
        String::from_utf8_lossy(&output.stderr)
    );
    let model =
        rex_ir::Model::from_json(&String::from_utf8_lossy(&output.stdout)).expect("rex-ir JSON");
    assert_eq!(model.packages.len(), 2);
    assert_eq!(model.packages[0].name, "alpha");
    assert_eq!(model.packages[1].name, "beta");
    let links = &model.packages[1].classes[0].features[0];
    assert_eq!(
        links.type_,
        rex_ir::TypeRef::Class {
            package: "alpha".to_string(),
            name: "Book".to_string(),
        }
    );
}

#[test]
fn directory_arguments_expand_recursively_in_sorted_order() {
    // Creation order is the reverse of the sorted path order, pinning that
    // the scan — not the filesystem — decides package order. Non-`.mox`
    // files are ignored.
    let dir = scratch_dir().join(format!("scan-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("nested")).expect("create nested dir");
    std::fs::write(dir.join("z.mox"), "package zed\n\nclass Zed { int x }").expect("write z.mox");
    std::fs::write(dir.join("nested/a.mox"), MULTI_A).expect("write nested/a.mox");
    std::fs::write(dir.join("ignored.txt"), "not a model").expect("write txt");
    std::fs::write(dir.join("ignored.actor"), "actors X { actor A }").expect("write actor");

    let output = rexlang()
        .args(["ir", dir.to_str().unwrap()])
        .output()
        .expect("run rexlang ir on a directory");
    assert!(
        output.status.success(),
        "stderr: {:?}",
        String::from_utf8_lossy(&output.stderr)
    );
    let model =
        rex_ir::Model::from_json(&String::from_utf8_lossy(&output.stdout)).expect("rex-ir JSON");
    let names: Vec<&str> = model.packages.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names, ["alpha", "zed"], "sorted-path package order");
}

#[test]
fn directory_scan_covers_non_mox_surfaces() {
    let dir = scratch_dir().join(format!("empty-scan-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create dir");
    std::fs::write(dir.join("notes.txt"), "nothing to compile").expect("write txt");
    std::fs::write(dir.join("flow.ifml"), "view \"Home\" {\n}\n").expect("write ifml");

    // A directory with no `.mox` at all still checks its other surfaces;
    // unsupported extensions are ignored.
    let output = rexlang()
        .args(["check", dir.to_str().unwrap()])
        .output()
        .expect("run rexlang check on a .mox-less directory");
    assert_eq!(output.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("flow.ifml"), "stdout was: {stdout}");
}

#[test]
fn directory_without_any_model_files_is_a_clean_error() {
    let dir = scratch_dir().join(format!("no-models-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create dir");
    std::fs::write(dir.join("notes.txt"), "nothing to compile").expect("write txt");

    let output = rexlang()
        .args(["check", dir.to_str().unwrap()])
        .output()
        .expect("run rexlang check on a model-less directory");
    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("no model files found"),
        "stderr was: {stderr}"
    );
    assert!(stderr.contains("no-models"), "stderr was: {stderr}");
}

#[test]
fn gen_rust_accepts_multiple_files() {
    let a = write_source("gen_multi_a.mox", MULTI_A);
    let b = write_source("gen_multi_b.mox", MULTI_B);
    let out = scratch_dir().join(format!("gen-multi-out-{}", std::process::id()));
    let output = rexlang()
        .args([
            "gen",
            "rust",
            a.to_str().unwrap(),
            b.to_str().unwrap(),
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
    assert!(models.contains("pub struct Book"));
    assert!(models.contains("pub struct Shelf"));
}
