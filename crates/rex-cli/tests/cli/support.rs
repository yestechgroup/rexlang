//! Shared CLI-test helpers: the CLI invocation, scratch-dir plumbing,
//! and the small inline sources the core tests reuse.

//! End-to-end CLI tests: run the `rexlang` binary as a subprocess.

use std::path::{Path, PathBuf};
use std::process::Command;

pub(crate) const GOOD: &str = "package demo\n\nclass Book { String title }\n";
pub(crate) const BAD: &str = "package demo\n\nclass Book { Book oops }\n";

pub(crate) fn rexlang() -> Command {
    Command::new(env!("CARGO_BIN_EXE_rexlang"))
}

/// A unique scratch directory per test, so concurrently running tests never
/// share files even when they use the same source file name.
pub(crate) fn scratch_dir() -> PathBuf {
    let thread = std::thread::current();
    let test = thread
        .name()
        .unwrap_or("test")
        .replace(['/', ' ', ':'], "_");
    let dir = std::env::temp_dir().join(format!("rex-cli-tests-{}-{}", std::process::id(), test));
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

pub(crate) fn write_source(name: &str, contents: &str) -> PathBuf {
    let path = scratch_dir().join(name);
    std::fs::write(&path, contents).expect("write source");
    path
}

pub(crate) fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../")
        .canonicalize()
        .expect("workspace root")
}

/// The workspace-root-relative fixture path for the manifest tests.
pub(crate) fn conformance_fixture(relative: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../")
        .join(relative)
}
