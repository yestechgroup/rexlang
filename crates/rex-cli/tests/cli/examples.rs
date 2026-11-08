//! `rexlang` CLI tests: examples.

use std::path::{Path, PathBuf};

use crate::support::*;

// --- the canonical examples suite --------------------------------------------

/// The seven canonical examples, hardcoded so a missing file fails loudly.
const EXAMPLES: [&str; 7] = [
    "library",
    "ecommerce",
    "org",
    "iot",
    "shapes",
    "support",
    "pricing",
];

/// Canonical `.actor` policy files: compiled against the domain they import.
const ACTOR_EXAMPLES: [&str; 1] = ["support"];

/// Canonical artifact-surface examples: each compiled against the imports
/// it declares (and, for `.dmn`, the decision tables its `logic table`
/// clauses name), all resolved relative to the file (the CLI gathers and
/// compiles `.ddd`/`.evt`/`.dmn`/`.uc`/`.deploy`/`.cmmn` imports exactly
/// like a `rexlang check` of those surfaces). Their artifact bytes are
/// pinned by `crates/rex-driver/tests/examples_artifacts.rs` and the cmmn
/// backend's examples test.
const ARTIFACT_EXAMPLES: [(&str, &str); 9] = [
    ("ecommerce", "evt"),
    ("support", "evt"),
    ("lending", "dmn"),
    ("support", "dmn"),
    ("orders", "ddd"),
    ("support", "ddd"),
    ("support", "uc"),
    ("support", "deploy"),
    ("support", "cmmn"),
];

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../")
        .canonicalize()
        .expect("workspace root")
}

#[test]
fn examples_suite_checks_clean_and_is_fmt_canonical() {
    let paths: Vec<PathBuf> = EXAMPLES
        .iter()
        .map(|name| {
            workspace_root()
                .join("examples")
                .join(format!("{name}.mox"))
        })
        .collect();
    let actor_paths: Vec<PathBuf> = ACTOR_EXAMPLES
        .iter()
        .map(|name| {
            workspace_root()
                .join("examples")
                .join(format!("{name}.actor"))
        })
        .collect();
    let artifact_paths: Vec<PathBuf> = ARTIFACT_EXAMPLES
        .iter()
        .map(|(name, extension)| {
            workspace_root()
                .join("examples")
                .join(format!("{name}.{extension}"))
        })
        .collect();
    for (name, path) in EXAMPLES.iter().zip(&paths) {
        assert!(
            path.is_file(),
            "missing canonical example {}: the examples suite is part of the harness",
            path.display()
        );
        let output = rexlang()
            .args(["check", path.to_str().unwrap()])
            .output()
            .expect("run rexlang check");
        assert!(
            output.status.success(),
            "rexlang check {name}.mox failed: {:?}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(
            String::from_utf8_lossy(&output.stdout),
            format!("OK {}\n", path.display()),
            "rexlang check {name}.mox stdout"
        );
    }
    for (name, path) in ACTOR_EXAMPLES.iter().zip(&actor_paths) {
        assert!(
            path.is_file(),
            "missing canonical example {name}.actor: the examples suite is part of the harness"
        );
        let output = rexlang()
            .args(["check", path.to_str().unwrap()])
            .output()
            .expect("run rexlang check");
        assert!(
            output.status.success(),
            "rexlang check {name}.actor failed: {:?}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(
            String::from_utf8_lossy(&output.stdout),
            format!("OK {}\n", path.display()),
            "rexlang check {name}.actor stdout"
        );
    }

    for ((name, extension), path) in ARTIFACT_EXAMPLES.iter().zip(&artifact_paths) {
        assert!(
            path.is_file(),
            "missing canonical example {name}.{extension}: the examples suite is part of the harness"
        );
        let output = rexlang()
            .args(["check", path.to_str().unwrap()])
            .output()
            .expect("run rexlang check");
        assert!(
            output.status.success(),
            "rexlang check {name}.{extension} failed: {:?}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(
            String::from_utf8_lossy(&output.stdout),
            format!("OK {}\n", path.display()),
            "rexlang check {name}.{extension} stdout"
        );
    }

    // The examples must be in canonical format (same gate the conformance
    // models get in CI).
    let mut check = rexlang();
    check.arg("fmt").arg("--check");
    for path in paths.iter().chain(&actor_paths).chain(&artifact_paths) {
        check.arg(path.to_str().unwrap());
    }
    let output = check.output().expect("run rexlang fmt --check");
    assert!(
        output.status.success(),
        "examples must be fmt-canonical. stdout: {:?}",
        String::from_utf8_lossy(&output.stdout)
    );
    assert!(String::from_utf8_lossy(&output.stdout).is_empty());
}
