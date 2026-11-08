//! `rexlang` CLI tests: core.

use crate::support::*;

#[test]
fn check_succeeds_on_valid_source() {
    let path = write_source("good.mox", GOOD);
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
fn check_rejects_defaults_that_do_not_match_the_attribute_type() {
    let preamble = "package demo\n\nenum Color { Red = 0 }\n\ntype Money wraps opaque\n\n";
    let cases: Vec<(String, &str)> = vec![
        (
            format!("{preamble}class Thing {{\n    Color c = \"red\"\n}}\n"),
            "string default 'red' on enum-typed attribute 'c'",
        ),
        (
            format!("{preamble}class Thing {{\n    int count = \"many\"\n}}\n"),
            "string default 'many' on int attribute 'count'",
        ),
        (
            format!("{preamble}class Thing {{\n    boolean active = \"yes\"\n}}\n"),
            "string default 'yes' on boolean attribute 'active'",
        ),
        (
            format!("{preamble}class Thing {{\n    String name = 7\n}}\n"),
            "int default '7' on string attribute 'name'",
        ),
        (
            format!("{preamble}class Thing {{\n    boolean active = 1\n}}\n"),
            "int default '1' on boolean attribute 'active'",
        ),
        (
            format!("{preamble}class Thing {{\n    Color c = 3\n}}\n"),
            "int default '3' on enum-typed attribute 'c'",
        ),
        (
            format!("{preamble}class Thing {{\n    String name = true\n}}\n"),
            "boolean default 'true' on string attribute 'name'",
        ),
        (
            format!("{preamble}class Thing {{\n    int count = false\n}}\n"),
            "boolean default 'false' on int attribute 'count'",
        ),
        (
            format!("{preamble}class Thing {{\n    Color c = false\n}}\n"),
            "boolean default 'false' on enum-typed attribute 'c'",
        ),
        (
            format!("{preamble}class Thing {{\n    Money amount = 5\n}}\n"),
            "int default '5' on datatype-typed attribute 'amount'",
        ),
        (
            format!("{preamble}class Thing {{\n    Money amount = true\n}}\n"),
            "boolean default 'true' on datatype-typed attribute 'amount'",
        ),
    ];
    for (index, (source, fragment)) in cases.iter().enumerate() {
        let path = write_source(&format!("bad_default_{index}.mox"), source);
        let output = rexlang()
            .args(["check", path.to_str().unwrap()])
            .output()
            .expect("run rexlang check");
        assert_eq!(output.status.code(), Some(1), "case: {fragment}");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains(fragment),
            "case: {fragment}; stderr was: {stderr}"
        );
    }
}

#[test]
fn check_accepts_valid_defaults() {
    let source = "package demo\n\nenum Color { Red = 0 }\n\ntype Money wraps opaque\n\nclass Thing {\n    String name = \"Default Name\"\n    int count = 5\n    long big = 6\n    float ratio = 2\n    double precise = 3\n    boolean active = true\n    Color c = Red\n    Money amount = \"9.99\"\n}\n";
    let path = write_source("good_defaults.mox", source);
    let output = rexlang()
        .args(["check", path.to_str().unwrap()])
        .output()
        .expect("run rexlang check");
    assert!(
        output.status.success(),
        "stderr: {:?}",
        String::from_utf8_lossy(&output.stderr)
    );
}
