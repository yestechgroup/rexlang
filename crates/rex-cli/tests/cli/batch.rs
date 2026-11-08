//! `rexlang` CLI tests: batch.

use crate::support::*;

// --- directory batch checking ---------------------------------------------------

#[test]
fn check_directory_batch_checks_every_surface_independently() {
    let dir = scratch_dir().join("batch");
    std::fs::create_dir_all(&dir).expect("mkdir");
    std::fs::write(dir.join("good.mox"), GOOD).expect("write mox");
    std::fs::write(dir.join("flow.ifml"), "view \"Home\" {\n}\n").expect("write ifml");
    std::fs::write(
        dir.join("team.actor"),
        "import \"good.mox\"\n\nactors Team {\n    actor Member\n}\n",
    )
    .expect("write actor");
    std::fs::write(
        dir.join("broken.mox"),
        "package a\n\nclass B {\n    Mystery oops\n}\n",
    )
    .expect("write broken");
    // `.evt` contracts are batch-checked like every other surface: the
    // healthy one compiles clean (its import resolves relative to the file)
    // and the broken one's driver diagnostics fail the batch.
    std::fs::write(
        dir.join("events.evt"),
        "import \"good.mox\"\n\nevent BookAdded {\n    title: String;\n}\n\nchannel library {\n    publishes BookAdded;\n}\n",
    )
    .expect("write evt");
    std::fs::write(
        dir.join("broken.evt"),
        "event Ghosted {\n    kind: String;\n}\n\nchannel trades {\n    publishes Ghost;\n}\n",
    )
    .expect("write broken evt");

    let output = rexlang()
        .args(["check", dir.to_str().unwrap()])
        .output()
        .expect("run rexlang check");
    assert!(!output.status.success(), "one broken file fails the batch");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("OK"), "healthy files report OK: {stdout}");
    assert!(
        stdout.contains("events.evt"),
        "the healthy .evt contract is batch-checked: {stdout}"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("2 of 6 files failed"), "{stderr}");
    assert!(
        stderr.contains("unknown event 'Ghost' on channel 'trades'"),
        "the broken .evt's driver diagnostic fails the batch: {stderr}"
    );
}

#[test]
fn check_empty_directory_is_a_clean_error() {
    let dir = scratch_dir().join("empty-batch");
    std::fs::create_dir_all(&dir).expect("mkdir");
    let output = rexlang()
        .args(["check", dir.to_str().unwrap()])
        .output()
        .expect("run rexlang check");
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("no model files found"), "{stderr}");
}

/// Formatter-golden artifacts (`X.fmt.<ext>` — the byte-golden output of
/// formatting a canonical model elsewhere) are not standalone models: their
/// imports and vocabulary snapshots only resolve next to their *sources*.
/// Directory batch mode must skip them rather than try to compile them.
#[test]
fn check_directory_batch_skips_formatter_goldens() {
    let dir = scratch_dir().join("batch-fmt-goldens");
    std::fs::create_dir_all(&dir).expect("mkdir");
    std::fs::write(dir.join("good.mox"), GOOD).expect("write mox");
    // A well-formed golden whose source-side import dangles in this dir —
    // compiling it would fail with "cannot read imported file".
    std::fs::write(dir.join("library.fmt.mox"), "import \"library.mox\"\n").expect("write golden");
    // A syntactically broken golden proves the skip is unconditional.
    std::fs::write(dir.join("broken.fmt.mox"), "not a model at all").expect("write broken golden");

    let output = rexlang()
        .args(["check", dir.to_str().unwrap()])
        .output()
        .expect("run rexlang check");
    assert!(
        output.status.success(),
        "formatter goldens are skipped: stderr: {:?} stdout: {:?}",
        String::from_utf8_lossy(&output.stderr),
        String::from_utf8_lossy(&output.stdout)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(
        stdout.matches("OK").count(),
        1,
        "only the real model is checked: {stdout}"
    );
    assert!(stdout.contains("good.mox"), "{stdout}");
}
