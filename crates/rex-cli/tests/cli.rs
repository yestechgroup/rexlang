//! End-to-end CLI tests: run the `rexlang` binary as a subprocess.

use std::path::{Path, PathBuf};
use std::process::Command;

const GOOD: &str = "package demo\n\nclass Book { String title }\n";
const BAD: &str = "package demo\n\nclass Book { Book oops }\n";

fn rexlang() -> Command {
    Command::new(env!("CARGO_BIN_EXE_rexlang"))
}

/// A unique scratch directory per test, so concurrently running tests never
/// share files even when they use the same source file name.
fn scratch_dir() -> PathBuf {
    let thread = std::thread::current();
    let test = thread
        .name()
        .unwrap_or("test")
        .replace(['/', ' ', ':'], "_");
    let dir = std::env::temp_dir().join(format!("rex-cli-tests-{}-{}", std::process::id(), test));
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

fn write_source(name: &str, contents: &str) -> PathBuf {
    let path = scratch_dir().join(name);
    std::fs::write(&path, contents).expect("write source");
    path
}

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

#[test]
fn missing_file_is_a_clean_error() {
    let missing = scratch_dir().join("does-not-exist.mox");
    let output = rexlang()
        .args(["check", missing.to_str().unwrap()])
        .output()
        .expect("run rexlang check");
    assert_eq!(output.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&output.stderr).starts_with("error: cannot read"));
}

#[test]
fn gen_rust_writes_generated_code() {
    let path = write_source("gen.mox", GOOD);
    let out = scratch_dir().join(format!("gen-out-{}", std::process::id()));
    let output = rexlang()
        .args([
            "gen",
            "rust",
            path.to_str().unwrap(),
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
    assert!(models.contains("pub struct Resource"));
    assert!(models.contains("pub struct Book"));
    assert!(models.contains("pub fn to_instance_json"));
}

#[test]
fn gen_rust_fails_on_invalid_source() {
    let path = write_source("gen_bad.mox", BAD);
    let out = scratch_dir().join(format!("gen-bad-out-{}", std::process::id()));
    let output = rexlang()
        .args([
            "gen",
            "rust",
            path.to_str().unwrap(),
            "-o",
            out.to_str().unwrap(),
        ])
        .output()
        .expect("run rexlang gen rust");
    assert_eq!(output.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&output.stderr).contains("has class type"));
}

#[test]
fn gen_json_schema_wire_writes_schema_json() {
    let path = write_source("schema_wire.mox", GOOD);
    let out = scratch_dir().join(format!("schema-wire-out-{}", std::process::id()));
    let output = rexlang()
        .args([
            "gen",
            "json-schema",
            path.to_str().unwrap(),
            "--profile",
            "wire",
            "-o",
            out.to_str().unwrap(),
        ])
        .output()
        .expect("run rexlang gen json-schema");
    assert!(
        output.status.success(),
        "stderr: {:?}",
        String::from_utf8_lossy(&output.stderr)
    );
    let json = std::fs::read_to_string(out.join("schema.json")).expect("schema.json written");
    let schema: serde_json::Value = serde_json::from_str(&json).expect("valid JSON");
    assert_eq!(schema["$id"], "urn:rex:model:demo:wire");
    assert_eq!(schema["properties"]["$type"]["const"], "rex.instance");
    assert_eq!(
        schema["$defs"]["Book"]["properties"]["$type"]["const"],
        "Book"
    );
}

#[test]
fn gen_json_schema_api_writes_api_flavor() {
    let path = write_source("schema_api.mox", GOOD);
    let out = scratch_dir().join(format!("schema-api-out-{}", std::process::id()));
    let output = rexlang()
        .args([
            "gen",
            "json-schema",
            path.to_str().unwrap(),
            "--profile",
            "api",
            "-o",
            out.to_str().unwrap(),
        ])
        .output()
        .expect("run rexlang gen json-schema");
    assert!(
        output.status.success(),
        "stderr: {:?}",
        String::from_utf8_lossy(&output.stderr)
    );
    let json = std::fs::read_to_string(out.join("schema.json")).expect("schema.json written");
    let schema: serde_json::Value = serde_json::from_str(&json).expect("valid JSON");
    assert_eq!(schema["$id"], "urn:rex:model:demo:api");
    assert!(
        schema.get("properties").is_none(),
        "api root validates nothing"
    );
    assert_eq!(schema["$defs"]["Book"]["additionalProperties"], false);
}

#[test]
fn gen_json_schema_fails_on_invalid_source() {
    let path = write_source("schema_bad.mox", BAD);
    let out = scratch_dir().join(format!("schema-bad-out-{}", std::process::id()));
    let output = rexlang()
        .args([
            "gen",
            "json-schema",
            path.to_str().unwrap(),
            "--profile",
            "wire",
            "-o",
            out.to_str().unwrap(),
        ])
        .output()
        .expect("run rexlang gen json-schema");
    assert_eq!(output.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&output.stderr).contains("has class type"));
}

const ACTORS_MODEL: &str = include_str!("../../../tests/conformance/models/actors.mox");

#[test]
fn gen_cedar_writes_policies_and_schema_deterministically() {
    let path = write_source("actors.mox", ACTORS_MODEL);
    let out = scratch_dir().join(format!("cedar-out-{}", std::process::id()));
    let output = rexlang()
        .args([
            "gen",
            "cedar",
            path.to_str().unwrap(),
            "-o",
            out.to_str().unwrap(),
        ])
        .output()
        .expect("run rexlang gen cedar");
    assert!(
        output.status.success(),
        "stderr: {:?}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        format!(
            "generated Cedar policies and schema into {}\n",
            out.display()
        )
    );
    let cedar = std::fs::read_to_string(out.join("Support.cedar")).expect("policies written");
    assert!(
        cedar.contains(
            "permit(principal is Support::Agent, action == \
             Support::Action::\"RaiseRefund\", resource is Support::Ticket)"
        ),
        "policies: {cedar}"
    );
    let schema: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(out.join("Support.cedarschema.json")).expect("schema written"),
    )
    .expect("schema is valid JSON");
    assert_eq!(
        schema["Support"]["actions"]["ReadTicket"]["appliesTo"]["resourceTypes"],
        serde_json::json!(["Support::Ticket"])
    );

    // A second run into a fresh directory must produce byte-identical files.
    let again = scratch_dir().join(format!("cedar-out-again-{}", std::process::id()));
    let second = rexlang()
        .args([
            "gen",
            "cedar",
            path.to_str().unwrap(),
            "-o",
            again.to_str().unwrap(),
        ])
        .output()
        .expect("run rexlang gen cedar again");
    assert!(
        second.status.success(),
        "stderr: {:?}",
        String::from_utf8_lossy(&second.stderr)
    );
    for name in ["Support.cedar", "Support.cedarschema.json"] {
        assert_eq!(
            std::fs::read(out.join(name)).expect("first run"),
            std::fs::read(again.join(name)).expect("second run"),
            "{name} must be byte-identical across runs"
        );
    }
}

#[test]
fn gen_cedar_fails_on_invalid_source() {
    let path = write_source("cedar_bad.mox", BAD);
    let out = scratch_dir().join(format!("cedar-bad-out-{}", std::process::id()));
    let output = rexlang()
        .args([
            "gen",
            "cedar",
            path.to_str().unwrap(),
            "-o",
            out.to_str().unwrap(),
        ])
        .output()
        .expect("run rexlang gen cedar");
    assert_eq!(output.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&output.stderr).contains("has class type"));
}

const VOCAB_MODEL: &str = r#"package demo

vocabulary Currency from "iso:4217" {
    version "2024-01-01"
    key alpha3
    facet String symbol
    facet int minorUnits
}

class Account {
    String owner
    Currency currency
}
"#;

const VOCAB_MODEL_NO_VERSION: &str = r#"package demo

vocabulary Currency from "iso:4217" {
    key alpha3
    facet String symbol
    facet int minorUnits
}

class Account {
    String owner
    Currency currency
}
"#;

const VOCAB_MODEL_BAD_FACET: &str = r#"package demo

vocabulary Currency from "iso:4217" {
    version "2024-01-01"
    key alpha3
    facet Book minorUnits
}

class Book { String title }
"#;

const VOCAB_MODEL_NO_KEY: &str = r#"package demo

vocabulary Currency from "iso:4217" {
    version "2024-01-01"
    facet String symbol
}
"#;

const SNAPSHOT: &str = r#"{
  "vocabulary": "iso:4217",
  "version": "2024-01-01",
  "entries": [
    { "alpha3": "USD", "symbol": "$", "minorUnits": 2 },
    { "alpha3": "EUR", "symbol": "€", "minorUnits": 2 },
    { "alpha3": "JPY", "symbol": "¥", "minorUnits": 0 },
    { "alpha3": "GBP", "symbol": "£", "minorUnits": 2 },
    { "alpha3": "CHF", "symbol": "CHF", "minorUnits": 2 }
  ]
}
"#;

/// A fresh model dir plus an upstream provider dir holding the snapshot.
fn vocab_fixture(tag: &str, source: &str) -> (PathBuf, PathBuf) {
    let dir = scratch_dir().join(format!("vocab-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let model_dir = dir.join("model");
    let upstream_dir = dir.join("upstream");
    std::fs::create_dir_all(&model_dir).expect("create model dir");
    std::fs::create_dir_all(&upstream_dir).expect("create upstream dir");
    std::fs::write(model_dir.join("model.mox"), source).expect("write model");
    std::fs::write(upstream_dir.join("iso-4217@2024-01-01.json"), SNAPSHOT)
        .expect("write snapshot");
    (model_dir, upstream_dir)
}

#[test]
fn vocab_fetch_vendors_snapshot_and_pins_lockfile() {
    let (model_dir, upstream_dir) = vocab_fixture("happy", VOCAB_MODEL);
    let model_path = model_dir.join("model.mox");
    let output = rexlang()
        .args([
            "vocab",
            "fetch",
            model_path.to_str().unwrap(),
            "--provider",
            &format!("file:{}", upstream_dir.display()),
        ])
        .output()
        .expect("run rexlang vocab fetch");
    assert!(
        output.status.success(),
        "stderr: {:?}",
        String::from_utf8_lossy(&output.stderr)
    );

    // Per-vocabulary line: `vendored iso:4217@2024-01-01 (N entries, sha256:…)`.
    let stdout = String::from_utf8_lossy(&output.stdout);
    let digest = rex_vocab::digest(SNAPSHOT.as_bytes());
    assert!(
        stdout.contains(&format!(
            "vendored iso:4217@2024-01-01 (5 entries, {}…)",
            &digest[..7 + 16]
        )),
        "stdout was: {stdout}"
    );

    // Snapshot vendored next to the model, byte-identical to the source.
    let vendored = std::fs::read(model_dir.join("vocab").join("iso-4217@2024-01-01.json"))
        .expect("vendored snapshot");
    assert_eq!(vendored, SNAPSHOT.as_bytes());

    // Lockfile pin: correct digest, version, RFC 3339 stamp.
    let lockfile =
        rex_vocab::Lockfile::read(&model_dir.join("model.lock")).expect("read model.lock");
    let entry = lockfile.entry_for("iso:4217").expect("iso:4217 pin");
    assert_eq!(entry.version, "2024-01-01");
    assert_eq!(entry.digest, digest);
    assert!(
        entry.fetched_at.ends_with('Z'),
        "fetchedAt: {}",
        entry.fetched_at
    );

    // Hermetic follow-up: the compiled model now embeds the entries.
    let ir = rexlang()
        .args(["ir", model_path.to_str().unwrap()])
        .output()
        .expect("run rexlang ir");
    assert!(
        ir.status.success(),
        "stderr: {:?}",
        String::from_utf8_lossy(&ir.stderr)
    );
    let model = rex_ir::Model::from_json(&String::from_utf8_lossy(&ir.stdout)).expect("IR JSON");
    assert_eq!(model.packages[0].vocabularies[0].entries.len(), 5);
}

#[test]
fn vocab_fetch_defaults_to_the_vendored_dir_provider() {
    let (model_dir, _upstream_dir) = vocab_fixture("default-provider", VOCAB_MODEL);
    // Pre-vendor the snapshot into `<model-dir>/vocab`; the default provider
    // reads from there.
    let vocab_dir = model_dir.join("vocab");
    std::fs::create_dir_all(&vocab_dir).expect("create vocab dir");
    std::fs::write(vocab_dir.join("iso-4217@2024-01-01.json"), SNAPSHOT).expect("write snapshot");
    let model_path = model_dir.join("model.mox");
    let output = rexlang()
        .args(["vocab", "fetch", model_path.to_str().unwrap()])
        .output()
        .expect("run rexlang vocab fetch");
    assert!(
        output.status.success(),
        "stderr: {:?}",
        String::from_utf8_lossy(&output.stderr)
    );
    let lockfile =
        rex_vocab::Lockfile::read(&model_dir.join("model.lock")).expect("read model.lock");
    assert_eq!(
        lockfile.entry_for("iso:4217").expect("pin").digest,
        rex_vocab::digest(SNAPSHOT.as_bytes())
    );
}

#[test]
fn vocab_fetch_falls_back_to_the_lockfile_version() {
    let (model_dir, upstream_dir) = vocab_fixture("lock-version", VOCAB_MODEL_NO_VERSION);
    let model_path = model_dir.join("model.mox");
    // Pre-seed a lockfile pinning the version; the declaration has none.
    let mut lockfile = rex_vocab::Lockfile::default();
    lockfile.insert(rex_vocab::LockEntry {
        source: "iso:4217".to_string(),
        version: "2024-01-01".to_string(),
        digest: "sha256:0000000000000000000000000000000000000000000000000000000000000000"
            .to_string(),
        fetched_at: "2026-09-10T00:00:00Z".to_string(),
    });
    lockfile
        .write(&model_dir.join("model.lock"))
        .expect("write lockfile");

    let output = rexlang()
        .args([
            "vocab",
            "fetch",
            model_path.to_str().unwrap(),
            "--provider",
            &format!("file:{}", upstream_dir.display()),
        ])
        .output()
        .expect("run rexlang vocab fetch");
    assert!(
        output.status.success(),
        "stderr: {:?}",
        String::from_utf8_lossy(&output.stderr)
    );
    let lockfile =
        rex_vocab::Lockfile::read(&model_dir.join("model.lock")).expect("read model.lock");
    assert_eq!(
        lockfile.entry_for("iso:4217").expect("pin").digest,
        rex_vocab::digest(SNAPSHOT.as_bytes()),
        "the stale pin is replaced with the fetched digest"
    );
}

#[test]
fn vocab_fetch_without_any_version_is_an_error() {
    let (model_dir, upstream_dir) = vocab_fixture("no-version", VOCAB_MODEL_NO_VERSION);
    let model_path = model_dir.join("model.mox");
    let output = rexlang()
        .args([
            "vocab",
            "fetch",
            model_path.to_str().unwrap(),
            "--provider",
            &format!("file:{}", upstream_dir.display()),
        ])
        .output()
        .expect("run rexlang vocab fetch");
    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("pin a version"), "stderr was: {stderr}");
}

#[test]
fn vocab_fetch_http_requires_a_url_template() {
    let (model_dir, _upstream_dir) = vocab_fixture("http-no-template", VOCAB_MODEL);
    let model_path = model_dir.join("model.mox");
    let output = rexlang()
        .args([
            "vocab",
            "fetch",
            model_path.to_str().unwrap(),
            "--provider",
            "http",
        ])
        .output()
        .expect("run rexlang vocab fetch");
    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("url-template"), "stderr was: {stderr}");
}

#[test]
fn vocab_fetch_rejects_unknown_providers() {
    let (model_dir, _upstream_dir) = vocab_fixture("unknown-provider", VOCAB_MODEL);
    let model_path = model_dir.join("model.mox");
    let output = rexlang()
        .args([
            "vocab",
            "fetch",
            model_path.to_str().unwrap(),
            "--provider",
            "carrier-pigeon",
        ])
        .output()
        .expect("run rexlang vocab fetch");
    assert_eq!(output.status.code(), Some(1));
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("carrier-pigeon"),
        "stderr was: {:?}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn vocab_fetch_rejects_non_primitive_facets() {
    let (model_dir, upstream_dir) = vocab_fixture("bad-facet", VOCAB_MODEL_BAD_FACET);
    let model_path = model_dir.join("model.mox");
    let output = rexlang()
        .args([
            "vocab",
            "fetch",
            model_path.to_str().unwrap(),
            "--provider",
            &format!("file:{}", upstream_dir.display()),
        ])
        .output()
        .expect("run rexlang vocab fetch");
    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("minorUnits"), "stderr was: {stderr}");
    assert!(stderr.contains("primitive"), "stderr was: {stderr}");
}

#[test]
fn vocab_fetch_requires_a_key_declaration() {
    let (model_dir, upstream_dir) = vocab_fixture("no-key", VOCAB_MODEL_NO_KEY);
    let model_path = model_dir.join("model.mox");
    let output = rexlang()
        .args([
            "vocab",
            "fetch",
            model_path.to_str().unwrap(),
            "--provider",
            &format!("file:{}", upstream_dir.display()),
        ])
        .output()
        .expect("run rexlang vocab fetch");
    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("key"), "stderr was: {stderr}");
}

#[test]
fn vocab_fetch_reports_missing_snapshots_cleanly() {
    let (model_dir, upstream_dir) = vocab_fixture("missing-snapshot", VOCAB_MODEL);
    std::fs::remove_file(upstream_dir.join("iso-4217@2024-01-01.json")).expect("remove snapshot");
    let model_path = model_dir.join("model.mox");
    let output = rexlang()
        .args([
            "vocab",
            "fetch",
            model_path.to_str().unwrap(),
            "--provider",
            &format!("file:{}", upstream_dir.display()),
        ])
        .output()
        .expect("run rexlang vocab fetch");
    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("iso-4217@2024-01-01.json"),
        "stderr was: {stderr}"
    );
}

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

// --- `.actor` files ------------------------------------------------------------

const ACTOR_DOMAIN: &str = r#"package demo

class Ticket {
    String title
    int amount
}
"#;

const GOOD_ACTOR: &str = r#"import "actor-domain.mox"

actors Escalation {
    actor Finance
    capability EscalateTicket on Ticket

    grant Finance {
        permit EscalateTicket when (amount > 0)
    }
}
"#;

const BROKEN_ACTOR: &str = r#"import "actor-domain.mox"

actors Escalation {
    actor Finance
    capability EscalateTicket on Ticket

    grant Finance {
        permit Missing
    }
}
"#;

#[test]
fn check_succeeds_on_actor_pair() {
    let domain = write_source("actor-domain.mox", ACTOR_DOMAIN);
    let _ = domain;
    let actor = write_source("good.actor", GOOD_ACTOR);
    let output = rexlang()
        .args(["check", actor.to_str().unwrap()])
        .output()
        .expect("run rexlang check");
    assert!(
        output.status.success(),
        "stderr: {:?}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        format!("OK {}\n", actor.display())
    );
    assert!(String::from_utf8_lossy(&output.stderr).is_empty());
}

#[test]
fn check_fails_on_broken_actor_file_with_actor_path_in_output() {
    let _ = write_source("actor-domain.mox", ACTOR_DOMAIN);
    let actor = write_source("broken.actor", BROKEN_ACTOR);
    let output = rexlang()
        .args(["check", actor.to_str().unwrap()])
        .output()
        .expect("run rexlang check");
    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("unknown capability `Missing`"),
        "stderr was: {stderr}"
    );
    assert!(
        stderr.contains("broken.actor"),
        "the actor file path must name the report: {stderr}"
    );
}

#[test]
fn missing_import_file_is_a_clean_error() {
    let actor = write_source(
        "dangling.actor",
        "import \"missing-domain.mox\"\n\nactors E {\n    actor A\n}\n",
    );
    let output = rexlang()
        .args(["check", actor.to_str().unwrap()])
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

#[test]
fn ir_actor_file_writes_actor_model_artifact() {
    let _ = write_source("actor-domain.mox", ACTOR_DOMAIN);
    let actor = write_source("good.actor", GOOD_ACTOR);
    let out = scratch_dir().join("good.actors.rex.json");
    let output = rexlang()
        .args(["ir", actor.to_str().unwrap(), "-o", out.to_str().unwrap()])
        .output()
        .expect("run rexlang ir");
    assert!(
        output.status.success(),
        "stderr: {:?}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).is_empty());
    let json = std::fs::read_to_string(&out).expect("read actor IR output");
    let actor_model = rex_ir::ActorModel::from_json(&json).expect("file is ActorModel JSON");
    assert_eq!(
        actor_model.format_version,
        rex_ir::ACTOR_MODEL_FORMAT_VERSION
    );
    assert_eq!(actor_model.blocks.len(), 1);
    assert_eq!(actor_model.blocks[0].name, "Escalation");
}

#[test]
fn gen_cedar_from_actor_file_writes_policies_and_schema_deterministically() {
    let _ = write_source("actor-domain.mox", ACTOR_DOMAIN);
    let actor = write_source("good.actor", GOOD_ACTOR);
    let out = scratch_dir().join(format!("cedar-actor-out-{}", std::process::id()));
    let output = rexlang()
        .args([
            "gen",
            "cedar",
            actor.to_str().unwrap(),
            "-o",
            out.to_str().unwrap(),
        ])
        .output()
        .expect("run rexlang gen cedar");
    assert!(
        output.status.success(),
        "stderr: {:?}",
        String::from_utf8_lossy(&output.stderr)
    );
    let cedar = std::fs::read_to_string(out.join("Escalation.cedar")).expect("policies written");
    assert!(
        cedar.contains(
            "permit(principal is Escalation::Finance, action == \
             Escalation::Action::\"EscalateTicket\", resource is Escalation::Ticket) \
             when { (resource.amount > 0) };"
        ),
        "the `when` must typecheck against the domain class: {cedar}"
    );
    let schema: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(out.join("Escalation.cedarschema.json")).expect("schema written"),
    )
    .expect("schema is valid JSON");
    assert_eq!(
        schema["Escalation"]["actions"]["EscalateTicket"]["appliesTo"]["resourceTypes"],
        serde_json::json!(["Escalation::Ticket"])
    );

    // A second run into a fresh directory must produce byte-identical files.
    let again = scratch_dir().join(format!("cedar-actor-again-{}", std::process::id()));
    let second = rexlang()
        .args([
            "gen",
            "cedar",
            actor.to_str().unwrap(),
            "-o",
            again.to_str().unwrap(),
        ])
        .output()
        .expect("run rexlang gen cedar again");
    assert!(
        second.status.success(),
        "stderr: {:?}",
        String::from_utf8_lossy(&second.stderr)
    );
    for name in ["Escalation.cedar", "Escalation.cedarschema.json"] {
        assert_eq!(
            std::fs::read(out.join(name)).expect("first run"),
            std::fs::read(again.join(name)).expect("second run"),
            "{name} must be byte-identical across runs"
        );
    }
}

#[test]
fn fmt_actor_files_rewrites_in_place_and_check_then_passes() {
    let messy = "import  \"actor-domain.mox\"\n\n\nactors   E  { actor F capability C on Ticket grant F { permit C } }\n\n\n";
    let path = write_source("fmt_actor.actor", messy);
    let output = rexlang()
        .args(["fmt", path.to_str().unwrap()])
        .output()
        .expect("run rexlang fmt");
    assert!(
        output.status.success(),
        "stderr: {:?}",
        String::from_utf8_lossy(&output.stderr)
    );
    let formatted = std::fs::read_to_string(&path).expect("read formatted actor file");
    assert!(
        formatted.starts_with("import \"actor-domain.mox\"\n\nactors E {\n"),
        "canonical actor layout: {formatted:?}"
    );

    let check = rexlang()
        .args(["fmt", "--check", path.to_str().unwrap()])
        .output()
        .expect("run rexlang fmt --check");
    assert!(
        check.status.success(),
        "fmt output must be a fixpoint: {:?}",
        String::from_utf8_lossy(&check.stdout)
    );
    assert!(String::from_utf8_lossy(&check.stdout).is_empty());
}

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
fn directory_without_mox_files_is_a_clean_error() {
    let dir = scratch_dir().join(format!("empty-scan-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create dir");
    std::fs::write(dir.join("notes.txt"), "nothing to compile").expect("write txt");

    let output = rexlang()
        .args(["check", dir.to_str().unwrap()])
        .output()
        .expect("run rexlang check on a .mox-less directory");
    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("no .mox files"), "stderr was: {stderr}");
    assert!(stderr.contains("empty-scan"), "stderr was: {stderr}");
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

// --- the canonical examples suite --------------------------------------------

/// The six canonical examples, hardcoded so a missing file fails loudly.
const EXAMPLES: [&str; 6] = ["library", "ecommerce", "org", "iot", "shapes", "support"];

/// Canonical `.actor` policy files: compiled against the domain they import.
const ACTOR_EXAMPLES: [&str; 1] = ["support"];

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

    // The examples must be in canonical format (same gate the conformance
    // models get in CI).
    let mut check = rexlang();
    check.arg("fmt").arg("--check");
    for path in paths.iter().chain(&actor_paths) {
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

// --- `rexlang lsp` -----------------------------------------------------------

use std::io::{BufRead, BufReader, Read as _};
use std::process::Stdio;
use std::sync::mpsc;
use std::time::Duration;

/// Writes one `Content-Length` framed message.
fn write_framed<W: std::io::Write>(sink: &mut W, body: &str) {
    write!(sink, "Content-Length: {}\r\n\r\n{}", body.len(), body).expect("write framed message");
    sink.flush().expect("flush framed message");
}

/// Reads one `Content-Length` framed message.
fn read_framed(source: &mut BufReader<std::process::ChildStdout>) -> String {
    let mut content_length: Option<usize> = None;
    loop {
        let mut line = String::new();
        source.read_line(&mut line).expect("read header line");
        let line = line.trim_end();
        if line.is_empty() {
            break;
        }
        if let Some(value) = line.strip_prefix("Content-Length: ") {
            content_length = Some(value.parse().expect("Content-Length is a number"));
        }
    }
    let length = content_length.expect("a Content-Length header");
    let mut body = vec![0u8; length];
    source.read_exact(&mut body).expect("read framed body");
    String::from_utf8(body).expect("framed body is UTF-8")
}

#[test]
fn lsp_subcommand_speaks_framed_json_rpc_over_stdio() {
    let mut child = rexlang()
        .args(["lsp"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn rexlang lsp");

    let mut stdin = child.stdin.take().expect("stdin piped");
    let mut stdout = BufReader::new(child.stdout.take().expect("stdout piped"));

    // The whole interaction runs on a helper thread so the test can enforce
    // a hard timeout instead of hanging CI.
    let (sender, receiver) = mpsc::channel();
    let worker = std::thread::spawn(move || {
        let initialize = serde_json::json!({
            "jsonrpc": "2.0", "id": 1, "method": "initialize",
            "params": {"capabilities": {}},
        });
        write_framed(&mut stdin, &initialize.to_string());
        let initialize_response: serde_json::Value =
            serde_json::from_str(&read_framed(&mut stdout)).expect("initialize response");

        let shutdown = serde_json::json!({"jsonrpc": "2.0", "id": 2, "method": "shutdown"});
        write_framed(&mut stdin, &shutdown.to_string());
        let shutdown_response: serde_json::Value =
            serde_json::from_str(&read_framed(&mut stdout)).expect("shutdown response");

        let exit = serde_json::json!({"jsonrpc": "2.0", "method": "exit"});
        write_framed(&mut stdin, &exit.to_string());

        sender
            .send((initialize_response, shutdown_response))
            .expect("send results");
    });

    let result = receiver.recv_timeout(Duration::from_secs(10));
    child.kill().expect("kill the server");
    let _ = child.wait();

    if let Err(mpsc::RecvTimeoutError::Timeout) = result {
        panic!("rexlang lsp did not answer within 10s");
    }
    worker.join().expect("worker thread");
    let (initialize_response, shutdown_response) = result.expect("results");

    assert_eq!(initialize_response["id"], 1);
    assert!(
        initialize_response["result"]["capabilities"].is_object(),
        "initialize must return capabilities: {initialize_response}"
    );
    assert_eq!(shutdown_response["id"], 2);
    assert!(
        shutdown_response.get("error").is_none(),
        "shutdown must succeed: {shutdown_response}"
    );
}

// --- `gen tools` ----------------------------------------------------------------

#[test]
fn gen_tools_actor_file_prints_every_agent_manifest() {
    let actor =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/conformance/models/actors.actor");
    let output = rexlang()
        .args(["gen", "tools", actor.to_str().unwrap()])
        .output()
        .expect("run rexlang gen tools");
    assert!(
        output.status.success(),
        "stderr: {:?}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(
        stdout,
        r#"{
  "agents": [
    {
      "agent": "Helper",
      "tools": [
        "EscalateTicket",
        "ApproveRefund"
      ],
      "delegations": [
        {
          "name": "EscalateHelp",
          "entries": [
            {
              "capability": "EscalateTicket"
            }
          ]
        }
      ]
    },
    {
      "agent": "Agent",
      "tools": [
        "ReadTicket",
        "RaiseRefund"
      ],
      "delegations": [
        {
          "name": "SupportHelp",
          "purpose": "RefundTriage",
          "entries": [
            {
              "capability": "ReadTicket"
            }
          ]
        },
        {
          "name": "RefundIntake",
          "purpose": "RefundTriage",
          "entries": [
            {
              "capability": "ReadTicket",
              "when": "!internal",
              "obligations": [
                "ack"
              ]
            }
          ]
        }
      ]
    },
    {
      "agent": "Manager",
      "tools": [
        "ReadTicket",
        "ResolveTicket",
        "RaiseRefund"
      ],
      "delegations": []
    }
  ]
}
"#,
        "the manifest document is the full agent surface of the policy set"
    );
}

#[test]
fn gen_tools_mox_prints_manifests_for_inline_actors() {
    let path = write_source(
        "tools.mox",
        r#"package demo

class Ticket {
    boolean internal
}

actors Support {
    actor Customer
    agent Agent extends Customer
    capability ReadTicket on Ticket

    grant Customer {
        permit ReadTicket
    }

    delegation Intake {
        from Customer
        to Agent
        permit ReadTicket
    }
}
"#,
    );
    let output = rexlang()
        .args(["gen", "tools", path.to_str().unwrap()])
        .output()
        .expect("run rexlang gen tools");
    assert!(
        output.status.success(),
        "stderr: {:?}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(
        stdout,
        r#"{
  "agents": [
    {
      "agent": "Agent",
      "tools": [
        "ReadTicket"
      ],
      "delegations": [
        {
          "name": "Intake",
          "entries": [
            {
              "capability": "ReadTicket"
            }
          ]
        }
      ]
    }
  ]
}
"#
    );
}

#[test]
fn gen_tools_fails_on_unresolvable_import_with_diagnostics() {
    let actor = write_source(
        "tools-dangling.actor",
        "import \"missing-domain.mox\"\n\nactors E {\n    actor A\n}\n",
    );
    let output = rexlang()
        .args(["gen", "tools", actor.to_str().unwrap()])
        .output()
        .expect("run rexlang gen tools");
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
