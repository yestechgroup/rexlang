//! `rexlang` CLI tests: `completions` and `man` (WI-9 DX surface), plus the
//! directory-scan ordering/termination pins behind the `walkdir` input
//! recursion.

use crate::support::*;

// --- `rexlang completions` ----------------------------------------------------

#[test]
fn completions_bash_generates_a_script() {
    let output = rexlang()
        .args(["completions", "bash"])
        .output()
        .expect("run rexlang completions bash");
    assert!(
        output.status.success(),
        "stderr: {:?}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("_rexlang()"), "stdout was: {stdout}");
    assert!(stdout.contains("--help"), "stdout was: {stdout}");
}

#[test]
fn completions_every_supported_shell_is_non_empty() {
    for shell in ["zsh", "fish", "powershell", "elvish"] {
        let output = rexlang()
            .args(["completions", shell])
            .output()
            .expect("run rexlang completions");
        assert!(
            output.status.success(),
            "{shell} stderr: {:?}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(
            !output.stdout.is_empty(),
            "{shell} produced an empty completion script"
        );
    }
}

#[test]
fn completions_without_a_shell_is_a_clap_error() {
    let output = rexlang()
        .arg("completions")
        .output()
        .expect("run rexlang completions");
    assert_ne!(output.status.code(), Some(0));
}

// --- `rexlang man` --------------------------------------------------------------

#[test]
fn man_renders_roff_to_stdout() {
    let output = rexlang().arg("man").output().expect("run rexlang man");
    assert!(
        output.status.success(),
        "stderr: {:?}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains(".TH rexlang 1"), "stdout was: {stdout}");
    // The subcommands are documented in the rendered page.
    assert!(stdout.contains("completions"), "stdout was: {stdout}");
}

#[test]
fn man_writes_to_out() {
    let path = scratch_dir().join("rexlang.1");
    let output = rexlang()
        .args(["man", "--out", path.to_str().unwrap()])
        .output()
        .expect("run rexlang man --out");
    assert!(output.status.success());
    let contents = std::fs::read_to_string(&path).expect("read written man page");
    assert!(contents.contains(".TH rexlang 1"));
}

// --- directory-scan pins (the `walkdir` recursion) --------------------------------

/// The expansion sorts the collected file list lexicographically, so the
/// order diagnostics are reported in is deterministic even though readdir
/// order is not (the files here are created in reverse-sorted order).
#[test]
fn check_directory_reports_files_in_sorted_order() {
    let dir = scratch_dir().join("scan_order");
    std::fs::create_dir_all(&dir).expect("create tree");
    let zz = dir.join("zz_first_created.mox");
    let aa = dir.join("aa_last_created.mox");
    std::fs::write(&zz, BAD).expect("write zz");
    std::fs::write(&aa, BAD).expect("write aa");
    let output = rexlang()
        .arg("check")
        .arg(&dir)
        .output()
        .expect("run rexlang check <dir>");
    assert_eq!(output.status.code(), Some(1));
    // ariadne renders to stderr; the paths live in its snippet headers.
    let rendered = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let aa_at = rendered.find("aa_last_created").expect("aa reported");
    let zz_at = rendered.find("zz_first_created").expect("zz reported");
    assert!(
        aa_at < zz_at,
        "sorted order violated: aa at {aa_at}, zz at {zz_at}"
    );
}

/// Symlinks are never followed: a self-referencing directory link cannot
/// hang or loop the scan, and the real model next to it is still found.
#[test]
fn directory_scan_ignores_symlink_loops() {
    let dir = scratch_dir().join("scan_loop");
    std::fs::create_dir_all(&dir).expect("create tree");
    // Canonical `.mox` text: the formatter expands declaration headers.
    let canonical = "package demo\n\nclass Book {\n    String title\n}\n";
    std::fs::write(dir.join("model.mox"), canonical).expect("write model");
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(&dir, dir.join("self")).expect("create symlink loop");
    }
    let output = rexlang()
        .args(["fmt", "--check", dir.to_str().unwrap()])
        .output()
        .expect("run rexlang fmt --check with a symlink loop");
    assert!(
        output.status.success(),
        "stdout: {:?} stderr: {:?}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
