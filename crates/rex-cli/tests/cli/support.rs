//! Shared CLI-test helpers: the CLI invocation, scratch-dir plumbing,
//! the small inline sources the core tests reuse, and the per-surface
//! harness for the five-test skeleton (check-succeeds / check-fails /
//! missing-import / ir / fmt) shared by the artifact surfaces.

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

// --- per-surface five-test harness ---------------------------------------------
//
// The artifact surfaces (`.actor`/`.ddd`/`.evt`/`.deploy`/`.dmn`/`.uc`/
// `.cmmn`) repeat one five-test skeleton: check succeeds, check fails on a
// broken source with pinned diagnostics, a missing import file is a clean
// error, `ir` writes the artifact, and `fmt` rewrites in place with
// `fmt --check` as a fixpoint. Each surface declares its case below; the
// runners execute every step with the exact commands, arguments, and pinned
// messages the per-surface tests have always used. Carried message strings
// are brace-free labels — the dynamic values (`stderr`, the formatted text)
// are interpolated at the run sites, so panic output is unchanged.

/// Where a skeleton step's model file comes from: the surface's canonical
/// conformance fixture on disk, or the scratch `good` source written into
/// the test's scratch directory.
pub(crate) enum InputSource {
    CanonicalFixture,
    ScratchGood,
}

/// How the closing `fmt --check` fixpoint is asserted; the payload is the
/// panic-message label.
pub(crate) enum FixpointAssert {
    /// `fmt --check` must succeed (label on stderr) and the file must still
    /// hold the formatted text.
    ContentEq(&'static str),
    /// `fmt --check` must succeed (label on stdout) and print nothing.
    StdoutEmpty(&'static str),
}

/// The fmt skeleton step: format a messy (or already-canonical) source in
/// place, pin the canonical layout, then prove `fmt --check` is a fixpoint.
pub(crate) struct FmtCase {
    /// The scrambled source fed to `fmt`; when absent the scratch `good`
    /// source is the input (the fixpoint-on-fixture shape).
    pub messy: Option<&'static str>,
    pub messy_name: &'static str,
    /// Whether the surface's `prereqs` are written first — only the
    /// surfaces whose original fmt test wrote them.
    pub write_prereqs: bool,
    /// (expected canonical prefix, panic-message label) pinned after the
    /// rewrite.
    pub prefix: Option<(&'static str, &'static str)>,
    /// (expected canonical suffix, panic-message label) for the surfaces
    /// that pin trailing layout too.
    pub suffix: Option<(&'static str, &'static str)>,
    /// Whether the first `fmt` pins `stderr: {:?}` on failure; absent is
    /// the bare success assert.
    pub pins_stderr_on_first_fmt: bool,
    pub read_message: &'static str,
    pub fixpoint: FixpointAssert,
}

/// The `ir` skeleton step: emit the artifact JSON and hand it to the
/// surface's verifier. The step's input source is the runner's parameter.
pub(crate) struct ArtifactCase {
    pub out_name: &'static str,
    pub read_message: &'static str,
    /// Parse + shape assertions, moved verbatim from the original test body.
    pub verify: fn(&str),
}

/// One artifact surface's five-test skeleton. Assembled per surface in its
/// module; the runners below execute each step.
pub(crate) struct SurfaceCase {
    /// (file name, contents) written before every scratch-based step: the
    /// imported domain/design/flow files the model's imports name.
    pub prereqs: &'static [(&'static str, &'static str)],
    pub good_name: &'static str,
    pub good_source: &'static str,
    /// Workspace-root-relative canonical fixture, for `CanonicalFixture`.
    pub canonical_fixture: Option<&'static str>,
    /// Whether the check-succeeds step also asserts empty stderr (every
    /// surface but deploy pins it).
    pub assert_stderr_empty: bool,
    pub broken_name: &'static str,
    pub broken_source: &'static str,
    /// stderr fragments the broken check must contain, each with its panic
    /// message label (all surfaces use `"stderr was: "` except dmn's
    /// unprovided-table fragment).
    pub broken_fragments: &'static [(&'static str, &'static str)],
    /// The broken file name that must appear on stderr.
    pub broken_path_fragment: &'static str,
    /// Noun in the path-assert message ("use case file", "actor file", …).
    pub broken_path_noun: &'static str,
    /// The dangling-import step: (file name, contents); absent where a
    /// surface has no such step (dmn, deploy).
    pub dangling: Option<(&'static str, &'static str)>,
    pub artifact: Option<ArtifactCase>,
    pub fmt: Option<FmtCase>,
}

fn fixture_path(case: &SurfaceCase) -> PathBuf {
    workspace_root().join(
        case.canonical_fixture
            .expect("surface declares a canonical fixture"),
    )
}

fn write_good_source(case: &SurfaceCase) -> PathBuf {
    for (name, contents) in case.prereqs {
        write_source(name, contents);
    }
    write_source(case.good_name, case.good_source)
}

/// Skeleton (1): `check` succeeds — `OK <path>` on stdout, empty stderr
/// where pinned.
pub(crate) fn check_succeeds(case: &SurfaceCase, input: InputSource) {
    let path = match input {
        InputSource::CanonicalFixture => fixture_path(case),
        InputSource::ScratchGood => write_good_source(case),
    };
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
    if case.assert_stderr_empty {
        assert!(String::from_utf8_lossy(&output.stderr).is_empty());
    }
}

/// Skeleton (2): `check` fails on the broken source — exit code 1, the
/// pinned diagnostic fragments on stderr, and the file path naming the
/// report.
pub(crate) fn check_fails_on_broken(case: &SurfaceCase) {
    for (name, contents) in case.prereqs {
        write_source(name, contents);
    }
    let path = write_source(case.broken_name, case.broken_source);
    let output = rexlang()
        .args(["check", path.to_str().unwrap()])
        .output()
        .expect("run rexlang check");
    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&output.stderr);
    for (fragment, label) in case.broken_fragments {
        assert!(stderr.contains(fragment), "{label}{stderr:?}");
    }
    assert!(
        stderr.contains(case.broken_path_fragment),
        "the {} path must name the report: {stderr}",
        case.broken_path_noun
    );
}

/// Skeleton (3): a missing import file is a clean clap-style error — never
/// a panic — and the import path is named.
pub(crate) fn missing_import_file_is_a_clean_error(case: &SurfaceCase) {
    let (name, source) = case
        .dangling
        .expect("surface declares a dangling-import case");
    let path = write_source(name, source);
    let output = rexlang()
        .args(["check", path.to_str().unwrap()])
        .output()
        .expect("run rexlang check");
    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.starts_with("error:"),
        "clean clap-style error, no panic: {stderr}"
    );
    assert!(
        stderr.contains("missing-domain.mox"),
        "the import path must be named: {stderr}"
    );
    assert!(!stderr.contains("panicked"), "must not panic: {stderr}");
}

/// Skeleton (4): `ir <file> -o <out>` writes the artifact JSON; the
/// surface's verifier asserts the parsed shape.
pub(crate) fn ir_writes_artifact(case: &SurfaceCase, input: InputSource) {
    let artifact = case
        .artifact
        .as_ref()
        .expect("surface declares an artifact case");
    let path = match input {
        InputSource::CanonicalFixture => fixture_path(case),
        InputSource::ScratchGood => write_good_source(case),
    };
    let out = scratch_dir().join(artifact.out_name);
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
    let json = std::fs::read_to_string(&out).expect(artifact.read_message);
    (artifact.verify)(&json);
}

/// Skeleton (5): `fmt` rewrites the source in place, the result is the
/// canonical layout, and `fmt --check` is a fixpoint.
pub(crate) fn fmt_rewrites_in_place_and_check_then_passes(case: &SurfaceCase) {
    let fmt = case.fmt.as_ref().expect("surface declares a fmt case");
    if fmt.write_prereqs {
        for (name, contents) in case.prereqs {
            write_source(name, contents);
        }
    }
    let path = write_source(fmt.messy_name, fmt.messy.unwrap_or(case.good_source));
    let output = rexlang()
        .args(["fmt", path.to_str().unwrap()])
        .output()
        .expect("run rexlang fmt");
    if fmt.pins_stderr_on_first_fmt {
        assert!(
            output.status.success(),
            "stderr: {:?}",
            String::from_utf8_lossy(&output.stderr)
        );
    } else {
        assert!(output.status.success());
    }
    let formatted = std::fs::read_to_string(&path).expect(fmt.read_message);
    if let Some((expected, label)) = fmt.prefix {
        assert!(formatted.starts_with(expected), "{label}{formatted:?}");
    }
    if let Some((expected, label)) = fmt.suffix {
        assert!(formatted.ends_with(expected), "{label}{formatted:?}");
    }

    let check = rexlang()
        .args(["fmt", "--check", path.to_str().unwrap()])
        .output()
        .expect("run rexlang fmt --check");
    match fmt.fixpoint {
        FixpointAssert::ContentEq(label) => {
            let stderr = String::from_utf8_lossy(&check.stderr);
            assert!(check.status.success(), "{label}{stderr:?}");
            assert_eq!(std::fs::read_to_string(&path).unwrap(), formatted);
        }
        FixpointAssert::StdoutEmpty(label) => {
            let stdout = String::from_utf8_lossy(&check.stdout);
            assert!(check.status.success(), "{label}{stdout:?}");
            assert!(stdout.is_empty());
        }
    }
}
