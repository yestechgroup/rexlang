//! `rexlang` CLI tests: fmt.

use std::process::Stdio;

use crate::support::*;

// --- `rexlang fmt` -----------------------------------------------------------

use std::io::Write as _;

const MESSY: &str = "package demo\n\n\nclass   Book  { int   pages   String title }\n\n\n";
const FORMATTED: &str = "package demo\n\nclass Book {\n    int pages\n    String title\n}\n";

#[test]
fn fmt_rewrites_files_in_place() {
    let path = write_source("fmt_in_place.mox", MESSY);
    let output = rexlang()
        .args(["fmt", path.to_str().unwrap()])
        .output()
        .expect("run rexlang fmt");
    assert!(
        output.status.success(),
        "stderr: {:?}",
        String::from_utf8_lossy(&output.stderr)
    );
    let contents = std::fs::read_to_string(&path).expect("read formatted file");
    assert_eq!(contents, FORMATTED);
}

#[test]
fn fmt_check_reports_unformatted_files_and_exits_1() {
    let messy = write_source("fmt_check_messy.mox", MESSY);
    let clean = write_source("fmt_check_clean.mox", FORMATTED);
    let output = rexlang()
        .args([
            "fmt",
            "--check",
            messy.to_str().unwrap(),
            clean.to_str().unwrap(),
        ])
        .output()
        .expect("run rexlang fmt --check");
    assert_eq!(output.status.code(), Some(1));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(
        stdout,
        format!("would reformat: {}\n", messy.display()),
        "stdout was: {stdout}"
    );
    // --check must not rewrite.
    assert_eq!(std::fs::read_to_string(&messy).unwrap(), MESSY);
}

#[test]
fn fmt_check_exits_0_when_everything_is_formatted() {
    let clean = write_source("fmt_check_ok.mox", FORMATTED);
    let output = rexlang()
        .args(["fmt", "--check", clean.to_str().unwrap()])
        .output()
        .expect("run rexlang fmt --check");
    assert!(output.status.success(), "stdout: {output:?}");
    assert!(String::from_utf8_lossy(&output.stdout).is_empty());
}

#[test]
fn fmt_dash_reads_stdin_and_writes_stdout() {
    let mut child = rexlang()
        .args(["fmt", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn rexlang fmt -");
    child
        .stdin
        .take()
        .expect("stdin piped")
        .write_all(MESSY.as_bytes())
        .expect("write stdin");
    let output = child.wait_with_output().expect("wait for rexlang fmt -");
    assert!(
        output.status.success(),
        "stderr: {:?}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&output.stdout), FORMATTED);
}

#[test]
fn fmt_without_files_reads_stdin() {
    let mut child = rexlang()
        .args(["fmt"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn rexlang fmt");
    child
        .stdin
        .take()
        .expect("stdin piped")
        .write_all(MESSY.as_bytes())
        .expect("write stdin");
    let output = child.wait_with_output().expect("wait for rexlang fmt");
    assert!(
        output.status.success(),
        "stderr: {:?}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&output.stdout), FORMATTED);
}

#[test]
fn fmt_missing_file_is_a_clean_error() {
    let missing = scratch_dir().join("fmt-missing.mox");
    let output = rexlang()
        .args(["fmt", missing.to_str().unwrap()])
        .output()
        .expect("run rexlang fmt");
    assert_eq!(output.status.code(), Some(1));
    assert!(
        String::from_utf8_lossy(&output.stderr).starts_with("error: cannot read"),
        "stderr was: {:?}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn fmt_unformattable_source_is_a_clean_error() {
    // The only true lex error: an integer literal that overflows i64.
    let path = write_source(
        "fmt_lex_error.mox",
        "class B { int x = 99999999999999999999999 }",
    );
    let output = rexlang()
        .args(["fmt", path.to_str().unwrap()])
        .output()
        .expect("run rexlang fmt");
    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.starts_with("error:"), "stderr was: {stderr}");
}

#[test]
fn fmt_help_works() {
    let output = rexlang()
        .args(["fmt", "--help"])
        .output()
        .expect("run rexlang fmt --help");
    assert!(output.status.success(), "stderr: {output:?}");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Usage:"), "stdout was: {stdout}");
    assert!(stdout.contains("--check"), "stdout was: {stdout}");
}
