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

/// The workspace-root-relative fixture path for the manifest tests.
fn conformance_fixture(relative: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../")
        .join(relative)
}

#[test]
fn manifest_prints_the_golden_manifest_to_stdout() {
    let path = conformance_fixture("tests/conformance/ifml/app.ifml");
    let golden = std::fs::read_to_string(conformance_fixture(
        "tests/conformance/ifml/app.manifest.json",
    ))
    .expect("read app manifest golden");
    let output = rexlang()
        .args(["manifest", path.to_str().unwrap()])
        .output()
        .expect("run rexlang manifest");
    assert!(
        output.status.success(),
        "stderr: {:?}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        format!("{golden}\n"),
        "the CLI manifest must byte-match the committed golden"
    );
}

#[test]
fn manifest_writes_the_golden_manifest_to_an_output_file() {
    let path = conformance_fixture("tests/conformance/ifml/cli.ifml");
    let golden = std::fs::read_to_string(conformance_fixture(
        "tests/conformance/ifml/cli.manifest.json",
    ))
    .expect("read cli manifest golden");
    let out = scratch_dir().join("cli.manifest.json");
    let output = rexlang()
        .args([
            "manifest",
            path.to_str().unwrap(),
            "-o",
            out.to_str().unwrap(),
        ])
        .output()
        .expect("run rexlang manifest");
    assert!(
        output.status.success(),
        "stderr: {:?}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).is_empty());
    let json = std::fs::read_to_string(&out).expect("read manifest output");
    assert_eq!(json, golden, "the file manifest must byte-match the golden");
    let manifest = rex_ir::ifml_manifest::IfmlManifest::from_json(&json)
        .expect("the output is a versioned manifest");
    assert!(!manifest.commands.is_empty());
}

#[test]
fn manifest_fails_on_a_dangling_reference() {
    // The declaration-backed guarantee at the CLI boundary: the checker
    // gates emission, so the missing-button class never reaches a
    // manifest.
    let path = write_source(
        "dangling.ifml",
        r#"view "List" {
    component "grid" {
        type: list;

        on select(row) -> action("Missing");
    }
}
"#,
    );
    let output = rexlang()
        .args(["manifest", path.to_str().unwrap()])
        .output()
        .expect("run rexlang manifest");
    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("E0104"), "{stderr}");
    assert!(stderr.contains("'Missing'"), "{stderr}");
}

#[test]
fn manifest_requires_an_ifml_file() {
    let path = write_source("not_a_flow.mox", GOOD);
    let output = rexlang()
        .args(["manifest", path.to_str().unwrap()])
        .output()
        .expect("run rexlang manifest");
    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains(".ifml"), "{stderr}");
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

// --- `.ddd` design files -------------------------------------------------------

const DDD_DOMAIN: &str = "package demo\n\nclass Thing { String name }\n";

const GOOD_DDD: &str = r#"import "ddd-domain.mox"

application Demo {
    module core {
        service ThingService {
            inject ThingRepository;
            register => ThingRepository.save;
        }
        entity Thing repository ThingRepository {
            findById;
            save;
        }
    }
}
"#;

const BROKEN_DDD: &str = r#"import "ddd-domain.mox"

application Demo {
    module core {
        entity Missing repository MissingRepository { findById; }
    }
}
"#;

#[test]
fn check_succeeds_on_ddd_design() {
    let path = workspace_root().join("tests/conformance/ddd/library.ddd");
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
fn check_succeeds_on_a_scratch_ddd_pair() {
    let _ = write_source("ddd-domain.mox", DDD_DOMAIN);
    let design = write_source("good.ddd", GOOD_DDD);
    let output = rexlang()
        .args(["check", design.to_str().unwrap()])
        .output()
        .expect("run rexlang check");
    assert!(
        output.status.success(),
        "stderr: {:?}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        format!("OK {}\n", design.display())
    );
    assert!(String::from_utf8_lossy(&output.stderr).is_empty());
}

#[test]
fn check_fails_on_broken_ddd_design_with_diagnostic_on_stderr() {
    let _ = write_source("ddd-domain.mox", DDD_DOMAIN);
    let design = write_source("broken.ddd", BROKEN_DDD);
    let output = rexlang()
        .args(["check", design.to_str().unwrap()])
        .output()
        .expect("run rexlang check");
    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("unknown type 'Missing'"),
        "stderr was: {stderr}"
    );
    assert!(
        stderr.contains("broken.ddd"),
        "the design file path must name the report: {stderr}"
    );
}

#[test]
fn missing_ddd_import_file_is_a_clean_error() {
    let design = write_source(
        "dangling.ddd",
        "import \"missing-domain.mox\"\n\napplication A { module m { entity T } }\n",
    );
    let output = rexlang()
        .args(["check", design.to_str().unwrap()])
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

const DDD_GOLDEN: &str = include_str!("../../../tests/conformance/ddd/library.ddd.json");

#[test]
fn ir_ddd_file_emits_the_ddd_design_artifact() {
    let path = workspace_root().join("tests/conformance/ddd/library.ddd");
    let output = rexlang()
        .args(["ir", path.to_str().unwrap()])
        .output()
        .expect("run rexlang ir");
    assert!(
        output.status.success(),
        "stderr: {:?}",
        String::from_utf8_lossy(&output.stderr)
    );
    let emitted = rex_ir::ddd::DddModel::from_json(&String::from_utf8_lossy(&output.stdout))
        .expect("stdout is DDD design JSON");
    let golden = rex_ir::ddd::DddModel::from_json(DDD_GOLDEN).expect("golden is DDD design JSON");
    assert_eq!(
        emitted.format_version,
        rex_ir::ddd::DDD_MODEL_FORMAT_VERSION
    );
    assert_eq!(emitted, golden, "the emitted artifact matches the golden");
}

#[test]
fn ir_ddd_file_emits_the_machine_and_its_derivations() {
    let path = workspace_root().join("tests/conformance/ddd/payrun.ddd");
    let output = rexlang()
        .args(["ir", path.to_str().unwrap()])
        .output()
        .expect("run rexlang ir");
    assert!(
        output.status.success(),
        "stderr: {:?}",
        String::from_utf8_lossy(&output.stderr)
    );
    let emitted = rex_ir::ddd::DddModel::from_json(&String::from_utf8_lossy(&output.stdout))
        .expect("stdout is DDD design JSON");
    let design = &emitted.modules[0].designs[0];
    let machine = design.machine.as_ref().expect("the machine lowers");
    assert_eq!(design.class, "PayRun");
    assert_eq!(machine.field, "status");
    assert_eq!(machine.initial, "Draft");
    assert_eq!(machine.terminals, ["Reversed", "Posted"]);
    assert_eq!(machine.transitions.len(), 6);
    let guarded = &machine.transitions[2];
    assert_eq!(guarded.trigger.as_deref(), Some("Approve"));
    assert_eq!(guarded.guard.as_deref(), Some("total > 0"));

    // The machine derives the state_changed event contract and lowers onto
    // the restingstate xstate-ir form.
    let events = rex_driver::machines::derived_events(&emitted);
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].event, "state_changed");
    assert_eq!(events[0].entity, "PayRun");
    let machines = rex_driver::machines::lower_design_machines(&emitted);
    assert_eq!(machines.len(), 1);
    assert_eq!(machines[0].id, "PayRun");
    assert_eq!(machines[0].initial, "Draft");
}

#[test]
fn fmt_check_passes_on_the_machine_ddd_fixture() {
    let path = workspace_root().join("tests/conformance/ddd/payrun.ddd");
    let output = rexlang()
        .args(["fmt", "--check", path.to_str().unwrap()])
        .output()
        .expect("run rexlang fmt --check");
    assert!(
        output.status.success(),
        "stdout: {:?}",
        String::from_utf8_lossy(&output.stdout)
    );
    assert!(String::from_utf8_lossy(&output.stdout).is_empty());
}

#[test]
fn fmt_check_passes_on_the_canonical_ddd_fixture() {
    let path = workspace_root().join("tests/conformance/ddd/library.ddd");
    let output = rexlang()
        .args(["fmt", "--check", path.to_str().unwrap()])
        .output()
        .expect("run rexlang fmt --check");
    assert!(
        output.status.success(),
        "stdout: {:?}",
        String::from_utf8_lossy(&output.stdout)
    );
    assert!(String::from_utf8_lossy(&output.stdout).is_empty());
}

#[test]
fn fmt_check_fails_on_a_scrambled_ddd_copy() {
    let messy = "import  \"ddd-domain.mox\"\n\n\napplication   Demo { module  core { entity   Thing repository  ThingRepository { findById; } } }\n\n";
    let path = write_source("scrambled.ddd", messy);
    let output = rexlang()
        .args(["fmt", "--check", path.to_str().unwrap()])
        .output()
        .expect("run rexlang fmt --check");
    assert_eq!(output.status.code(), Some(1));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(
        stdout,
        format!("would reformat: {}\n", path.display()),
        "stdout was: {stdout}"
    );
    // --check must not rewrite.
    assert_eq!(std::fs::read_to_string(&path).unwrap(), messy);
}

#[test]
fn artifact_check_passes_on_the_ddd_golden_artifact() {
    let path = workspace_root().join("tests/conformance/ddd/library.ddd.json");
    let output = rexlang()
        .args(["artifact", "check", path.to_str().unwrap()])
        .output()
        .expect("run rexlang artifact check");
    assert!(
        output.status.success(),
        "stderr: {:?}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        format!("OK {}\n", path.display())
    );
}

#[test]
fn artifact_check_passes_on_the_ifml_golden_artifact() {
    // The `rexlang ir` output for the canonical interaction flow: a
    // view-and-module IFML model — the classifier's IFML rule must claim
    // it before the DDD `modules` shape.
    let path = workspace_root().join("tests/conformance/ifml/app.ifml.json");
    let output = rexlang()
        .args(["artifact", "check", path.to_str().unwrap()])
        .output()
        .expect("run rexlang artifact check");
    assert!(
        output.status.success(),
        "stderr: {:?}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        format!("OK {}\n", path.display())
    );
}

// --- `.evt` event-contract files -----------------------------------------------

const EVT_DOMAIN: &str = r#"package demo

enum Status {
    Draft as "D" = 0
}

class Order {
    String orderId
    int total
    Status status
}
"#;

const GOOD_EVT: &str = r#"import "evt-domain.mox"

event OrderPlaced version "1.0.0" {
    orderId: String;
    total: int;
    status: Status;
}

event OrderCancelled {}

channel orders {
    publishes OrderPlaced;
    publishes OrderCancelled;
}

subscription billing {
    events [ OrderPlaced OrderCancelled ]
    consumer billing_service
}
"#;

const BROKEN_EVT: &str = r#"import "evt-domain.mox"

event OrderPlaced {
    total: Mystery;
}

channel orders {
    publishes OrderDeleted;
}
"#;

#[test]
fn check_succeeds_on_evt_pair() {
    let _ = write_source("evt-domain.mox", EVT_DOMAIN);
    let contract = write_source("good.evt", GOOD_EVT);
    let output = rexlang()
        .args(["check", contract.to_str().unwrap()])
        .output()
        .expect("run rexlang check");
    assert!(
        output.status.success(),
        "stderr: {:?}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        format!("OK {}\n", contract.display())
    );
    assert!(String::from_utf8_lossy(&output.stderr).is_empty());
}

#[test]
fn check_fails_on_broken_evt_file_with_evt_path_in_output() {
    let _ = write_source("evt-domain.mox", EVT_DOMAIN);
    let contract = write_source("broken.evt", BROKEN_EVT);
    let output = rexlang()
        .args(["check", contract.to_str().unwrap()])
        .output()
        .expect("run rexlang check");
    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("unknown type 'Mystery'"),
        "stderr was: {stderr}"
    );
    assert!(
        stderr.contains("unknown event 'OrderDeleted' on channel 'orders'"),
        "stderr was: {stderr}"
    );
    assert!(
        stderr.contains("broken.evt"),
        "the contract file path must name the report: {stderr}"
    );
}

#[test]
fn missing_evt_import_file_is_a_clean_error() {
    let contract = write_source(
        "dangling.evt",
        "import \"missing-domain.mox\"\n\nevent Ping {}\n",
    );
    let output = rexlang()
        .args(["check", contract.to_str().unwrap()])
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
fn ir_evt_file_writes_event_model_artifact() {
    let _ = write_source("evt-domain.mox", EVT_DOMAIN);
    let contract = write_source("good.evt", GOOD_EVT);
    let out = scratch_dir().join("good.evt.json");
    let output = rexlang()
        .args([
            "ir",
            contract.to_str().unwrap(),
            "-o",
            out.to_str().unwrap(),
        ])
        .output()
        .expect("run rexlang ir");
    assert!(
        output.status.success(),
        "stderr: {:?}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).is_empty());
    let json = std::fs::read_to_string(&out).expect("read event IR output");
    let event_model =
        rex_ir::events::EventModel::from_json(&json).expect("file is EventModel JSON");
    assert_eq!(
        event_model.format_version,
        rex_ir::events::EVENT_MODEL_FORMAT_VERSION
    );
    assert_eq!(event_model.events.len(), 2);
    assert_eq!(event_model.events[0].version.as_deref(), Some("1.0.0"));
    assert_eq!(event_model.channels.len(), 1);
    assert_eq!(event_model.subscriptions.len(), 1);
}

#[test]
fn fmt_evt_files_rewrites_in_place_and_check_then_passes() {
    let messy = "import  \"evt-domain.mox\"\n\n\nevent   OrderPlaced  version  \"1.0.0\"  { orderId : String ; }\n\n\n";
    let path = write_source("fmt_evt.evt", messy);
    let output = rexlang()
        .args(["fmt", path.to_str().unwrap()])
        .output()
        .expect("run rexlang fmt");
    assert!(
        output.status.success(),
        "stderr: {:?}",
        String::from_utf8_lossy(&output.stderr)
    );
    let formatted = std::fs::read_to_string(&path).expect("read formatted contract file");
    assert!(
        formatted.starts_with("import \"evt-domain.mox\"\n\nevent OrderPlaced version \"1.0.0\" {\n    orderId: String;\n"),
        "canonical contract layout: {formatted:?}"
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
/// it declares, all resolved relative to the file (the CLI gathers and
/// compiles `.ddd`/`.evt`/`.uc`/`.deploy`/`.cmmn` imports exactly like a
/// `rexlang check` of those surfaces). Their artifact bytes are pinned by
/// `crates/rex-driver/tests/examples_artifacts.rs` and the cmmn backend's
/// examples test.
const ARTIFACT_EXAMPLES: [(&str, &str); 7] = [
    ("ecommerce", "evt"),
    ("support", "evt"),
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
              "capability": "ReadTicket",
              "obligations": [
                "audit"
              ]
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
    },
    {
      "agent": "Auditor",
      "tools": [
        "EscalateTicket",
        "ApproveRefund"
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

// --- `import sigil` -----------------------------------------------------------

/// Writes a `.mox` file plus the rosetta files it imports: the named file
/// declares a namespace import satisfied by a sibling in a subdirectory
/// (discovered by the CLI's directory walk), mirroring sigil's own
/// multi-file projects.
fn write_sigil_project() -> PathBuf {
    let mox = scratch_dir().join("with_sigil.mox");
    std::fs::create_dir_all(scratch_dir().join("oracle/extra")).expect("create rosetta dirs");
    std::fs::write(
        scratch_dir().join("oracle/trade.rosetta"),
        concat!(
            "namespace oracle.basic\n\n",
            "import oracle.extra.*\n\n",
            "type Trade:\n",
            "    id string (1..1)\n",
            "    extra Extra (1..1)\n"
        ),
    )
    .expect("write named rosetta");
    std::fs::write(
        scratch_dir().join("oracle/extra/extra.rosetta"),
        "namespace oracle.extra\n\ntype Extra:\n    x int (1..1)\n",
    )
    .expect("write candidate rosetta");
    std::fs::write(
        &mox,
        concat!(
            "package demo\n\n",
            "import sigil \"oracle/trade.rosetta\"\n\n",
            "class Book { refers oracle.basic.Trade about }\n"
        ),
    )
    .expect("write mox");
    mox
}

#[test]
fn check_passes_on_a_mox_with_transitively_discovered_sigil_imports() {
    let mox = write_sigil_project();
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
fn ir_contains_the_synthetic_sigil_packages() {
    let mox = write_sigil_project();
    let output = rexlang()
        .args(["ir", mox.to_str().unwrap()])
        .output()
        .expect("run rexlang ir");
    assert!(output.status.success());
    let ir: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("the IR artifact on stdout");
    let names: Vec<&str> = ir["packages"]
        .as_array()
        .expect("packages")
        .iter()
        .map(|package| package["name"].as_str().expect("package name"))
        .collect();
    assert_eq!(names, vec!["demo", "oracle.basic", "oracle.extra"]);
    assert_eq!(
        ir["packages"][0]["classes"][0]["features"][0]["type"]["value"],
        serde_json::json!({"package": "oracle.basic", "name": "Trade"})
    );
    assert_eq!(
        ir["packages"][1]["classes"][0]["features"][0]["type"],
        serde_json::json!({"type": "primitive", "value": "string"})
    );
}

#[test]
fn check_fails_with_a_clear_error_when_a_sigil_file_is_missing() {
    let mox = scratch_dir().join("missing_sigil.mox");
    std::fs::write(
        &mox,
        "package demo\n\nimport sigil \"oracle/gone.rosetta\"\n\nclass C {}\n",
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
        stderr.contains("gone.rosetta"),
        "the resolved import path must be named: {stderr}"
    );
    assert!(!stderr.contains("panicked"), "must not panic: {stderr}");
}

#[test]
fn a_rosetta_parse_error_renders_against_the_rosetta_source() {
    let mox = scratch_dir().join("broken_sigil.mox");
    std::fs::create_dir_all(scratch_dir().join("oracle")).expect("create rosetta dir");
    std::fs::write(
        scratch_dir().join("oracle/broken.rosetta"),
        "namespace oracle.basic\n\ntype Trade:\n    id (1..1)\n",
    )
    .expect("write broken rosetta");
    std::fs::write(
        &mox,
        "package demo\n\nimport sigil \"oracle/broken.rosetta\"\n\nclass C {}\n",
    )
    .expect("write mox");
    let output = rexlang()
        .args(["check", mox.to_str().unwrap()])
        .output()
        .expect("run rexlang check");
    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("broken.rosetta"),
        "the diagnostic is tagged with the rosetta path: {stderr}"
    );
    assert!(
        stderr.contains("expected"),
        "the sigil parse message surfaces: {stderr}"
    );
}

const SIGIL_GOLDEN: &str = include_str!("../../../tests/conformance/sigil/basic.mox.json");

#[test]
fn check_succeeds_on_the_sigil_conformance_fixture() {
    let path = workspace_root().join("tests/conformance/sigil/basic.mox");
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
fn ir_sigil_file_emits_the_golden_artifact() {
    let path = workspace_root().join("tests/conformance/sigil/basic.mox");
    let output = rexlang()
        .args(["ir", path.to_str().unwrap()])
        .output()
        .expect("run rexlang ir");
    assert!(
        output.status.success(),
        "stderr: {:?}",
        String::from_utf8_lossy(&output.stderr)
    );
    // `ir` prints the artifact with one trailing newline (println!).
    let emitted = String::from_utf8_lossy(&output.stdout);
    assert_eq!(
        emitted.trim_end_matches('\n'),
        SIGIL_GOLDEN.trim_end_matches('\n'),
        "the emitted artifact matches the golden"
    );
}

#[test]
fn fmt_check_passes_on_the_canonical_sigil_fixture() {
    let path = workspace_root().join("tests/conformance/sigil/basic.mox");
    let output = rexlang()
        .args(["fmt", "--check", path.to_str().unwrap()])
        .output()
        .expect("run rexlang fmt --check");
    assert!(
        output.status.success(),
        "stdout: {:?}",
        String::from_utf8_lossy(&output.stdout)
    );
    assert!(String::from_utf8_lossy(&output.stdout).is_empty());
}

// --- `.ifml` interaction-flow files ---------------------------------------------

const IFML_DOMAIN: &str = r#"package demo

class Product {
    String name
    int price
}

class Category {
    String name
}
"#;

const GOOD_IFML: &str = r#"import "ifml-domain.mox"
import "pager.ifml";

domain "demo" {
    schema "demo";
}

view "Catalogue" {
    params { product: Product };

    use "Pager" as pager { pageSize: 10; };

    component "grid" {
        type: list;
        data: Product;
        filter: product.name != "";

        on select(row) -> navigate("Detail", {
            productId: row.name
        });
    }
}

view "Detail" {
    params { productId: String };

    component "form" {
        type: details;
        data: Product;
    }
}
"#;

const PAGER_IFML: &str = r#"module "Pager" {
    input { page: Int = 1, pageSize: Int = 25 }
    output { total: Int }

    component "pager" {
        type: list;
        data: Item;
    }
}
"#;

const BROKEN_IFML: &str = r#"import "ifml-domain.mox";

view "Catalogue" {
    params { product: Product };

    component "grid" {
        type: list;
        data: Product;
        filter: product.pric;

        on select(row) -> navigate("Detail", {
            productId: row.nope
        });
    }
}
"#;

#[test]
fn check_succeeds_on_ifml_with_typed_binding() {
    let _ = write_source("ifml-domain.mox", IFML_DOMAIN);
    let _ = write_source("pager.ifml", PAGER_IFML);
    let flow = write_source("good.ifml", GOOD_IFML);
    let output = rexlang()
        .args(["check", flow.to_str().unwrap()])
        .output()
        .expect("run rexlang check");
    assert!(
        output.status.success(),
        "stderr: {:?}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        format!("OK {}\n", flow.display())
    );
}

#[test]
fn check_reports_ifml_type_errors_with_site_context() {
    let _ = write_source("ifml-domain.mox", IFML_DOMAIN);
    let flow = write_source("broken.ifml", BROKEN_IFML);
    let output = rexlang()
        .args(["check", flow.to_str().unwrap()])
        .output()
        .expect("run rexlang check");
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("E0102"), "stderr: {stderr}");
    assert!(stderr.contains("unknown feature"), "stderr: {stderr}");
}

#[test]
fn ir_emits_ifml_artifact_json() {
    let _ = write_source("ifml-domain.mox", IFML_DOMAIN);
    let _ = write_source("pager.ifml", PAGER_IFML);
    let flow = write_source("good.ifml", GOOD_IFML);
    let output = rexlang()
        .args(["ir", flow.to_str().unwrap()])
        .output()
        .expect("run rexlang ir");
    assert!(
        output.status.success(),
        "stderr: {:?}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("\"formatVersion\": 1"));
    assert!(stdout.contains("Catalogue"));
}

#[test]
fn ir_expand_inlines_module_uses_for_ifml() {
    let _ = write_source("ifml-domain.mox", IFML_DOMAIN);
    let _ = write_source("pager.ifml", PAGER_IFML);
    let flow = write_source("good.ifml", GOOD_IFML);

    // Without --expand the use stays a reference.
    let plain = rexlang()
        .args(["ir", flow.to_str().unwrap()])
        .output()
        .expect("run rexlang ir");
    assert!(plain.status.success());
    let plain_json = String::from_utf8_lossy(&plain.stdout);
    assert!(
        plain_json.contains("\"moduleUses\""),
        "use kept: {plain_json}"
    );

    // With --expand the use is inlined: the module's component appears and
    // the use is gone.
    let expanded = rexlang()
        .args(["ir", flow.to_str().unwrap(), "--expand"])
        .output()
        .expect("run rexlang ir --expand");
    assert!(
        expanded.status.success(),
        "stderr: {:?}",
        String::from_utf8_lossy(&expanded.stderr)
    );
    let expanded_json = String::from_utf8_lossy(&expanded.stdout);
    assert!(
        !expanded_json.contains("\"moduleUses\""),
        "uses inlined: {expanded_json}"
    );
    assert!(
        expanded_json.contains("\"PagerBody\"") || expanded_json.contains("pager"),
        "the module's contents spliced: {expanded_json}"
    );
}

const AUTH_MOX: &str = r#"package rexlang.auth

class Customer {
    String name
}
"#;

const AUTH_ACTOR: &str = r#"import "auth.mox"

actors Auth {
    actor admin
    actor manager

    capability ManageCustomer on Customer
    capability ApproveCustomer on Customer

    grant admin {
        permit ManageCustomer
        permit ApproveCustomer
    }

    grant manager {
        permit ManageCustomer
    }
}
"#;

#[test]
fn check_resolves_ifml_requires_and_roles_against_the_actor_import() {
    let _ = write_source("auth.mox", AUTH_MOX);
    let _ = write_source("auth.actor", AUTH_ACTOR);
    let flow = write_source(
        "gated.ifml",
        r#"import "auth.actor";

view "Edit" {
    requires: [ManageCustomer];
    roles: [admin, manager];
}
"#,
    );
    let output = rexlang()
        .args(["check", flow.to_str().unwrap()])
        .output()
        .expect("run rexlang check");
    assert!(
        output.status.success(),
        "stderr: {:?}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        format!("OK {}\n", flow.display())
    );
}

#[test]
fn check_reports_unknown_capabilities_and_roles_from_the_actor_import() {
    let _ = write_source("auth.mox", AUTH_MOX);
    let _ = write_source("auth.actor", AUTH_ACTOR);
    let flow = write_source(
        "gated.ifml",
        r#"import "auth.actor";

view "Edit" {
    requires: [ManageCustomer, Ghost];
    roles: [admin, phantom];
}
"#,
    );
    let output = rexlang()
        .args(["check", flow.to_str().unwrap()])
        .output()
        .expect("run rexlang check");
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("E0107"), "stderr: {stderr}");
    assert!(stderr.contains("E0108"), "stderr: {stderr}");
    assert!(
        stderr.contains("capability 'Ghost' is not declared"),
        "stderr: {stderr}"
    );
    assert!(
        stderr.contains("role 'phantom' is not declared"),
        "stderr: {stderr}"
    );
    assert!(
        stderr.contains("gated.ifml"),
        "the diagnostic names the flow file: {stderr}"
    );
}

#[test]
fn check_resolves_transitive_ifml_actor_domain_across_nested_dirs() {
    let root = scratch_dir().join(format!("nested-ifml-{}", std::process::id()));
    std::fs::create_dir_all(root.join("app/ifml")).expect("mkdir app/ifml");
    std::fs::create_dir_all(root.join("app/policies")).expect("mkdir app/policies");
    std::fs::create_dir_all(root.join("app/domains")).expect("mkdir app/domains");
    std::fs::write(root.join("app/domains/auth.mox"), AUTH_MOX).expect("write domain");
    std::fs::write(
        root.join("app/policies/auth.actor"),
        "import \"../domains/auth.mox\"\n\nactors Auth {\n    actor admin\n\n    capability ManageCustomer on Customer\n\n    grant admin {\n        permit ManageCustomer\n    }\n}\n",
    )
    .expect("write actor");
    let flow = root.join("app/ifml/flow.ifml");
    std::fs::write(
        &flow,
        "import \"../policies/auth.actor\";\n\nview \"Edit\" {\n    requires: [ManageCustomer];\n    roles: [admin];\n}\n",
    )
    .expect("write flow");
    let output = rexlang()
        .args(["check", flow.to_str().unwrap()])
        .output()
        .expect("run rexlang check");
    assert!(
        output.status.success(),
        "stderr: {:?}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        format!("OK {}\n", flow.display())
    );
}

#[test]
fn check_dedupes_duplicate_actor_imports() {
    let _ = write_source("auth.mox", AUTH_MOX);
    let _ = write_source("auth.actor", AUTH_ACTOR);
    let flow = write_source(
        "twice.ifml",
        r#"import "auth.actor";
import "auth.actor";

view "Edit" {
    requires: [ManageCustomer];
    roles: [admin];
}
"#,
    );
    let output = rexlang()
        .args(["check", flow.to_str().unwrap()])
        .output()
        .expect("run rexlang check");
    assert!(
        output.status.success(),
        "stderr: {:?}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        format!("OK {}\n", flow.display()),
        "exactly one OK: the duplicate import gathers once"
    );
}

#[test]
fn missing_ifml_actor_import_is_a_clean_error() {
    let flow = write_source(
        "dangling.ifml",
        r#"import "ghost.actor";

view "Home" {
}
"#,
    );
    let output = rexlang()
        .args(["check", flow.to_str().unwrap()])
        .output()
        .expect("run rexlang check");
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.starts_with("error:"),
        "a clean error, not a panic: {stderr}"
    );
    assert!(
        stderr.contains("ghost.actor"),
        "the error names the resolved path: {stderr}"
    );
}

#[test]
fn check_succeeds_on_the_ifml_conformance_fixture_with_its_actor_import() {
    // app.ifml's `import "rexlang/auth.actor"` is load-bearing: the
    // fixture's requires:/roles: sites resolve against the fixture
    // policy's capabilities and actors.
    let path = workspace_root().join("tests/conformance/ifml/app.ifml");
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

#[test]
fn fmt_check_passes_on_formatted_ifml() {
    let flow = write_source("formatted.ifml", GOOD_IFML);
    let output = rexlang()
        .args(["fmt", "--check", flow.to_str().unwrap()])
        .output()
        .expect("run rexlang fmt --check");
    assert!(
        output.status.success(),
        "stderr: {:?} stdout: {:?}",
        String::from_utf8_lossy(&output.stderr),
        String::from_utf8_lossy(&output.stdout)
    );
}

#[test]
fn fmt_rewrites_scrambled_ifml() {
    let scrambled = "view \"A\"  {\n      label \"x\";\n}\n";
    let flow = write_source("scrambled.ifml", scrambled);
    let check = rexlang()
        .args(["fmt", "--check", flow.to_str().unwrap()])
        .output()
        .expect("run rexlang fmt --check");
    assert!(!check.status.success());
    let format = rexlang()
        .args(["fmt", flow.to_str().unwrap()])
        .output()
        .expect("run rexlang fmt");
    assert!(format.status.success());
    let formatted = std::fs::read_to_string(&flow).unwrap();
    assert_eq!(formatted, "view \"A\" {\n    label \"x\";\n}\n");
}

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

// --- project mode (`rexlang check <dir>`) and `rexlang build` -------------------

/// The cross-surface project fixture: five files, every seam exercised —
/// the policy imports the domain, the flow imports the policy, the design
/// imports the domain, and the deployment imports design + flow.
const PROJECT_MOX: &str = "package auth\n\nclass Customer {\n    String name\n}\n";

const PROJECT_ACTOR: &str = r#"import "auth.mox"

actors Auth {
    actor admin

    capability ManageCustomer on Customer

    grant admin {
        permit ManageCustomer
    }
}
"#;

const PROJECT_DDD: &str = r#"import "auth.mox"

application App {
    module core {
        service CustomerService {
            inject CustomerRepository;
            register => CustomerRepository.save capability ManageCustomer;
        }
        entity Customer repository CustomerRepository {
            findById;
            save;
        }
    }
}
"#;

const PROJECT_FLOW_GOOD: &str = r#"import "auth.actor";

view "Edit" {
    requires: [ManageCustomer];
    roles: [admin];
}
"#;

const PROJECT_FLOW_BAD: &str = r#"import "auth.actor";

view "Edit" {
    requires: [ManageCustomer, Ghost];
}
"#;

const PROJECT_DEPLOY: &str = r#"import "design.ddd"
import "flow.ifml"

application Svc {
    component core: api {
        design "design.ddd#core"
    }
    component web: frontend {
        flow "flow.ifml"
    }
    connects web -> core
}

profile prod {
    target: kubernetes
}

deployment dev for Svc {
    use prod
}
"#;

/// Writes the five-file project fixture into `dir`.
fn write_project_fixture(dir: &Path, flow: &str) {
    std::fs::create_dir_all(dir).expect("mkdir project dir");
    std::fs::write(dir.join("auth.mox"), PROJECT_MOX).expect("write mox");
    std::fs::write(dir.join("auth.actor"), PROJECT_ACTOR).expect("write actor");
    std::fs::write(dir.join("design.ddd"), PROJECT_DDD).expect("write design");
    std::fs::write(dir.join("flow.ifml"), flow).expect("write flow");
    std::fs::write(dir.join("svc.deploy"), PROJECT_DEPLOY).expect("write deploy");
}

#[test]
fn check_project_directory_reports_cross_surface_diagnostics_per_file() {
    let dir = scratch_dir().join(format!("project-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    write_project_fixture(&dir, PROJECT_FLOW_BAD);

    let output = rexlang()
        .args(["check", dir.to_str().unwrap()])
        .output()
        .expect("run rexlang check on a project directory");
    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("E0107"),
        "the authorization diagnostic: {stderr}"
    );
    assert!(
        stderr.contains("capability 'Ghost' is not declared"),
        "stderr: {stderr}"
    );
    assert!(
        stderr.contains("flow.ifml"),
        "the diagnostic is grouped under the flow file: {stderr}"
    );
    assert!(
        stderr.contains("1 of 5 files failed"),
        "the summary counts exactly one failing file: {stderr}"
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(
        stdout.matches("OK").count(),
        4,
        "the other four surfaces pass: {stdout}"
    );
}

#[test]
fn build_emits_all_artifacts_for_a_project_directory() {
    let root = scratch_dir().join(format!("build-project-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    write_project_fixture(&root.join("proj"), PROJECT_FLOW_GOOD);
    let out = root.join("out");

    let output = rexlang()
        .current_dir(&root)
        .args(["build", "proj", "-o", out.to_str().unwrap()])
        .output()
        .expect("run rexlang build on a project directory");
    assert!(
        output.status.success(),
        "stderr: {:?}",
        String::from_utf8_lossy(&output.stderr)
    );

    for (name, marker) in [
        ("auth.mox", "Customer"),
        ("auth.actor", "ManageCustomer"),
        ("design.ddd", "CustomerRepository"),
        ("flow.ifml", "Edit"),
        ("svc.deploy", "Svc"),
    ] {
        let path = out.join(format!("proj/{name}.json"));
        let json = std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("artifact for {name}: {error}"));
        let artifact: serde_json::Value = serde_json::from_str(&json).expect("valid JSON");
        assert_eq!(artifact["formatVersion"], serde_json::json!(1), "{name}");
        assert!(
            json.contains(marker),
            "{name} artifact carries its declaration: {json}"
        );
    }
}

#[test]
fn build_still_emits_compiling_files_when_a_sibling_fails() {
    let dir = scratch_dir().join(format!("build-partial-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("mkdir");
    std::fs::write(dir.join("auth.mox"), PROJECT_MOX).expect("write mox");
    // Capability granted on a class the imported domain does not declare.
    std::fs::write(
        dir.join("broken.actor"),
        "import \"auth.mox\"\n\nactors Auth {\n    actor admin\n\n    capability ManageCustomer on Ghost\n}\n",
    )
    .expect("write broken actor");
    std::fs::write(dir.join("flow.ifml"), "view \"Home\" {\n}\n").expect("write flow");
    let out = scratch_dir().join(format!("build-partial-out-{}", std::process::id()));

    let output = rexlang()
        .args(["build", dir.to_str().unwrap(), "-o", out.to_str().unwrap()])
        .output()
        .expect("run rexlang build with a broken sibling");
    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("broken.actor"),
        "the failure is named: {stderr}"
    );
    assert!(
        stderr.contains("1 of 3 files failed"),
        "the summary counts one failing file: {stderr}"
    );
    // The input path as given is absolute, so its artifact mirrors that
    // absolute path *inside* the out dir (never `Path::join` an absolute
    // string — it would swallow the base).
    let artifact = |name: &str| {
        PathBuf::from(format!(
            "{}/{}.json",
            out.display(),
            dir.join(name).display()
        ))
    };
    assert!(
        artifact("auth.mox").is_file(),
        "the compiling mox still emits its artifact"
    );
    assert!(
        artifact("flow.ifml").is_file(),
        "the compiling flow still emits its artifact"
    );
    assert!(
        !artifact("broken.actor").exists(),
        "the broken actor emits no artifact"
    );
}

#[test]
fn build_out_defaults_to_target_rex_relative_to_the_working_directory() {
    // The subprocess inherits this process's CWD, so the default out dir is
    // pinned against a scratch CWD instead of the repository's own target/.
    let root = scratch_dir().join(format!("build-default-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("proj")).expect("mkdir proj");
    std::fs::write(root.join("proj/good.mox"), GOOD).expect("write mox");

    let output = rexlang()
        .current_dir(&root)
        .args(["build", "proj"])
        .output()
        .expect("run rexlang build without -o");
    assert!(
        output.status.success(),
        "stderr: {:?}",
        String::from_utf8_lossy(&output.stderr)
    );
    let artifact = root.join("target/rex/proj/good.mox.json");
    assert!(
        artifact.is_file(),
        "the default output directory is ./target/rex"
    );
    let model = rex_ir::Model::from_json(&std::fs::read_to_string(&artifact).expect("artifact"))
        .expect("Core IR artifact");
    assert_eq!(model.packages[0].name, "demo");
}

// --- `.deploy` deployment files ------------------------------------------------

const DEPLOY_DOMAIN: &str = "package demo\n\nclass Book { String title }\n";

const DEPLOY_DESIGN: &str = "import \"deploy-domain.mox\";\n\napplication Library {\n    module media {\n        entity Book repository BookRepository {\n            findById;\n        }\n    }\n}\n";

const DEPLOY_FLOW: &str = "view \"Home\" { component \"list\" { type: list; data: Book; } }\n";

const GOOD_DEPLOY: &str = r#"import "deploy-design.ddd"
import "deploy-flow.ifml"

application Svc {
    component lending: api {
        design "deploy-design.ddd#media"
    }
    component web: frontend {
        flow "deploy-flow.ifml"
    }
    component db: database {
        engine: postgres
    }
    connects web -> lending
    connects lending -> db
}

profile prod {
    target: kubernetes
    defaults {
        lending.replicas: 2
    }
    require (lending.replicas >= 2)
}

deployment dev for Svc {
    use prod
}
"#;

const BROKEN_DEPLOY: &str = r#"import "deploy-design.ddd"

application Svc {
    component search: worker {}
}

profile edge {
    target: cloudflareWorkers
}

deployment prod for Svc {
    use edge
}
"#;

fn write_deploy_fixtures() -> PathBuf {
    write_source("deploy-domain.mox", DEPLOY_DOMAIN);
    write_source("deploy-design.ddd", DEPLOY_DESIGN);
    write_source("deploy-flow.ifml", DEPLOY_FLOW);
    write_source("svc.deploy", GOOD_DEPLOY)
}

#[test]
fn check_succeeds_on_deploy_pair() {
    let deploy = write_deploy_fixtures();
    let output = rexlang()
        .args(["check", deploy.to_str().unwrap()])
        .output()
        .expect("run rexlang check");
    assert!(
        output.status.success(),
        "stderr: {:?}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        format!("OK {}\n", deploy.display())
    );
}

/// The nested-flow guard for the driver's deploy gather: the deploy
/// imports a flow whose own imports import a pattern module — depth-2
/// flow nesting, resolved through the gatherer's identity→key map. Without
/// that map a flow's nested imports resolve against the process CWD and
/// the flow "does not compile".
#[test]
fn check_succeeds_on_a_deploy_with_a_nested_flow_import() {
    write_source("deploy-domain.mox", DEPLOY_DOMAIN);
    write_source("deploy-design.ddd", DEPLOY_DESIGN);
    write_source(
        "pattern.ifml",
        "module \"Pager\" {\n    input { pageSize: Int = 25 }\n    output { total: Int }\n\n    component \"pager\" {\n        type: list;\n        data: Item;\n    }\n}\n",
    );
    write_source(
        "flow.ifml",
        "import \"pattern.ifml\";\n\nview \"Home\" {\n    use \"Pager\" as pager { pageSize: 10; };\n\n    component \"list\" {\n        type: list;\n        data: Book;\n    }\n}\n",
    );
    let deploy = write_source(
        "nested.deploy",
        "import \"deploy-design.ddd\"\nimport \"flow.ifml\"\n\napplication Svc {\n    component lending: api {\n        design \"deploy-design.ddd#media\"\n    }\n    component web: frontend {\n        flow \"flow.ifml\"\n    }\n    component db: database {\n        engine: postgres\n    }\n    connects web -> lending\n    connects lending -> db\n}\n\nprofile prod {\n    target: kubernetes\n    defaults {\n        lending.replicas: 2\n    }\n    require (lending.replicas >= 2)\n}\n\ndeployment dev for Svc {\n    use prod\n}\n",
    );
    let output = rexlang()
        .args(["check", deploy.to_str().unwrap()])
        .output()
        .expect("run rexlang check");
    assert!(
        output.status.success(),
        "stderr: {:?}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        format!("OK {}\n", deploy.display())
    );
}

#[test]
fn check_fails_on_broken_deploy_with_capability_error() {
    write_source("deploy-domain.mox", DEPLOY_DOMAIN);
    write_source("deploy-design.ddd", DEPLOY_DESIGN);
    write_source("deploy-flow.ifml", DEPLOY_FLOW);
    let deploy = write_source("broken.deploy", BROKEN_DEPLOY);
    let output = rexlang()
        .args(["check", deploy.to_str().unwrap()])
        .output()
        .expect("run rexlang check");
    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains(
            "component 'search' requires capability 'longRunningProcess', which target \
             'cloudflareWorkers' does not provide"
        ),
        "stderr was: {stderr}"
    );
    assert!(
        stderr.contains("broken.deploy"),
        "the deploy file path must name the report: {stderr}"
    );
}

#[test]
fn ir_deploy_file_writes_deploy_model_artifact() {
    let deploy = write_deploy_fixtures();
    let out = scratch_dir().join("svc.deploy.json");
    let output = rexlang()
        .args(["ir", deploy.to_str().unwrap(), "-o", out.to_str().unwrap()])
        .output()
        .expect("run rexlang ir");
    assert!(
        output.status.success(),
        "stderr: {:?}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).is_empty());
    let json = std::fs::read_to_string(&out).expect("read deploy IR output");
    let deploy_model =
        rex_ir::deploy::DeployModel::from_json(&json).expect("file is DeployModel JSON");
    assert_eq!(
        deploy_model.format_version,
        rex_ir::deploy::DEPLOY_MODEL_FORMAT_VERSION
    );
    assert_eq!(deploy_model.applications.len(), 1);
    assert_eq!(deploy_model.profiles.len(), 1);
    assert_eq!(deploy_model.deployments.len(), 1);
}

#[test]
fn fmt_deploy_is_a_fixpoint_on_the_fixture() {
    let deploy = write_deploy_fixtures();
    let first = rexlang()
        .args(["fmt", deploy.to_str().unwrap()])
        .output()
        .expect("run rexlang fmt");
    assert!(first.status.success());
    let once = std::fs::read_to_string(&deploy).expect("read formatted deploy");
    let second = rexlang()
        .args(["fmt", "--check", deploy.to_str().unwrap()])
        .output()
        .expect("run rexlang fmt --check");
    assert!(
        second.status.success(),
        "formatting must be a fixpoint; stderr: {:?}",
        String::from_utf8_lossy(&second.stderr)
    );
    assert_eq!(std::fs::read_to_string(&deploy).unwrap(), once);
}

// --- `.uc` use case files -------------------------------------------------------

const UC_DOMAIN: &str = "package demo\n\nclass Drone {}\n\nclass Recipient {}\n";

const CMMN_DOMAIN: &str = "package lending\n\nclass Borrower {\n    String name\n}\n";

const GOOD_CMMN: &str = r#"import "cmmn-domain.mox"

case Loan {
    roles {
        role analyst;
    }
    case_file {
        Borrower[0..1] borrower;
    }
    task Verify {
        performer: analyst;
    }
    milestone Ready {
        entry on Verify.complete;
    }
}
"#;

const BROKEN_CMMN: &str = r#"import "cmmn-domain.mox"

case Loan {
    case_file {
        Borrower borrower;
    }
    milestone Ready {
        entry on borrower.explode;
    }
}
"#;

#[test]
fn check_succeeds_on_cmmn_conformance_fixture() {
    let path = workspace_root().join("tests/conformance/cmmn/lending.cmmn");
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
fn check_fails_on_broken_cmmn_file_with_cmmn_path_in_output() {
    let _ = write_source("cmmn-domain.mox", CMMN_DOMAIN);
    let cmmn = write_source("broken.cmmn", BROKEN_CMMN);
    let output = rexlang()
        .args(["check", cmmn.to_str().unwrap()])
        .output()
        .expect("run rexlang check");
    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("unknown transition 'explode' on case-file item 'borrower' in case 'Loan'"),
        "stderr was: {stderr}"
    );
    assert!(
        stderr.contains("broken.cmmn"),
        "the case file path must name the report: {stderr}"
    );
}

#[test]
fn missing_cmmn_import_file_is_a_clean_error() {
    let cmmn = write_source(
        "dangling.cmmn",
        "import \"missing-domain.mox\"\n\ncase C {}\n",
    );
    let output = rexlang()
        .args(["check", cmmn.to_str().unwrap()])
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
fn ir_cmmn_file_writes_case_model_artifact() {
    let _ = write_source("cmmn-domain.mox", CMMN_DOMAIN);
    let cmmn = write_source("good.cmmn", GOOD_CMMN);
    let out = scratch_dir().join("good.cmmn.json");
    let output = rexlang()
        .args(["ir", cmmn.to_str().unwrap(), "-o", out.to_str().unwrap()])
        .output()
        .expect("run rexlang ir");
    assert!(
        output.status.success(),
        "stderr: {:?}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).is_empty());
    let json = std::fs::read_to_string(&out).expect("read case IR output");
    let case_model = rex_ir::cmmn::CmmnModel::from_json(&json).expect("file is CmmnModel JSON");
    assert_eq!(
        case_model.format_version,
        rex_ir::cmmn::CMMN_MODEL_FORMAT_VERSION
    );
    assert_eq!(case_model.cases.len(), 1);
    assert_eq!(case_model.cases[0].name, "Loan");
    assert_eq!(case_model.cases[0].plan_items.len(), 2);
}

#[test]
fn fmt_cmmn_files_rewrites_in_place_and_check_then_passes() {
    let _ = write_source("cmmn-domain.mox", CMMN_DOMAIN);
    let messy = "import  \"cmmn-domain.mox\"\n\n\ncase   Loan { roles { role  analyst ; } task Verify { performer:  analyst } }\n\n\n";
    let path = write_source("fmt_cmmn.cmmn", messy);
    let output = rexlang()
        .args(["fmt", path.to_str().unwrap()])
        .output()
        .expect("run rexlang fmt");
    assert!(
        output.status.success(),
        "stderr: {:?}",
        String::from_utf8_lossy(&output.stderr)
    );
    let formatted = std::fs::read_to_string(&path).expect("read formatted case file");
    assert!(
        formatted.starts_with(
            "import \"cmmn-domain.mox\"\n\ncase Loan {\n    roles {\n        role analyst;\n    }\n"
        ),
        "canonical case layout: {formatted:?}"
    );

    let check = rexlang()
        .args(["fmt", "--check", path.to_str().unwrap()])
        .output()
        .expect("run rexlang fmt --check");
    assert!(
        check.status.success(),
        "formatting must be a fixpoint; stderr: {:?}",
        String::from_utf8_lossy(&check.stderr)
    );
    assert_eq!(std::fs::read_to_string(&path).unwrap(), formatted);
}

const GOOD_UC: &str = r#"import "uc-domain.mox"

usecase HandOver {
    subject Drone: Drone {
        occurrence recipientVerified;
    }
    actor Recipient: Recipient {
        occurrence promptAcknowledged;
    }
    message promptHandover from Drone.recipientVerified to Recipient.promptAcknowledged;
    first start then promptHandover.source;
    first promptHandover.source then promptHandover;
    first promptHandover then done;
}
"#;

const BROKEN_UC: &str = r#"import "uc-domain.mox"

usecase HandOver {
    subject Drone: Drone {
        perform scan: noSuchOp;
    }
}
"#;

#[test]
fn check_succeeds_on_uc_conformance_fixture() {
    // The canonical fixture pair exercises all four import buckets: the
    // `.mox` domain, a `.ddd` design (`as handover`), an `.actor` policy,
    // and an `.evt` contract.
    let path = workspace_root().join("tests/conformance/usecases/delivery.uc");
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
fn check_fails_on_broken_uc_file_with_uc_path_in_output() {
    let _ = write_source("uc-domain.mox", UC_DOMAIN);
    let uc = write_source("broken.uc", BROKEN_UC);
    let output = rexlang()
        .args(["check", uc.to_str().unwrap()])
        .output()
        .expect("run rexlang check");
    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("unknown operation 'noSuchOp' on class 'Drone' in use case 'HandOver'"),
        "stderr was: {stderr}"
    );
    assert!(
        stderr.contains("broken.uc"),
        "the use case file path must name the report: {stderr}"
    );
}

#[test]
fn missing_uc_import_file_is_a_clean_error() {
    let uc = write_source(
        "dangling.uc",
        "import \"missing-domain.mox\"\n\nusecase U {}\n",
    );
    let output = rexlang()
        .args(["check", uc.to_str().unwrap()])
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
fn ir_uc_file_writes_use_case_model_artifact() {
    let _ = write_source("uc-domain.mox", UC_DOMAIN);
    let uc = write_source("good.uc", GOOD_UC);
    let out = scratch_dir().join("good.uc.json");
    let output = rexlang()
        .args(["ir", uc.to_str().unwrap(), "-o", out.to_str().unwrap()])
        .output()
        .expect("run rexlang ir");
    assert!(
        output.status.success(),
        "stderr: {:?}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).is_empty());
    let json = std::fs::read_to_string(&out).expect("read use case IR output");
    let use_case_model =
        rex_ir::usecases::UseCaseModel::from_json(&json).expect("file is UseCaseModel JSON");
    assert_eq!(
        use_case_model.format_version,
        rex_ir::usecases::USE_CASE_MODEL_FORMAT_VERSION
    );
    assert_eq!(use_case_model.use_cases.len(), 1);
    let hand_over = &use_case_model.use_cases[0];
    assert_eq!(hand_over.name, "HandOver");
    assert_eq!(
        hand_over
            .subject
            .as_ref()
            .and_then(|s| s.type_name.as_deref()),
        Some("demo.Drone")
    );
    assert_eq!(hand_over.messages.len(), 1);
    assert_eq!(hand_over.successions.len(), 3);
}

#[test]
fn fmt_uc_files_rewrites_in_place_and_check_then_passes() {
    let _ = write_source("uc-domain.mox", UC_DOMAIN);
    let messy = "import  \"uc-domain.mox\"\n\n\nusecase   HandOver { subject Drone:Drone { occurrence  recipientVerified ; } }\n\n\n";
    let path = write_source("fmt_uc.uc", messy);
    let output = rexlang()
        .args(["fmt", path.to_str().unwrap()])
        .output()
        .expect("run rexlang fmt");
    assert!(
        output.status.success(),
        "stderr: {:?}",
        String::from_utf8_lossy(&output.stderr)
    );
    let formatted = std::fs::read_to_string(&path).expect("read formatted use case file");
    assert!(
        formatted.starts_with(
            "import \"uc-domain.mox\"\n\nusecase HandOver {\n    subject Drone: Drone {\n        occurrence recipientVerified;\n    }\n"
        ),
        "canonical use case layout: {formatted:?}"
    );

    let check = rexlang()
        .args(["fmt", "--check", path.to_str().unwrap()])
        .output()
        .expect("run rexlang fmt --check");
    assert!(
        check.status.success(),
        "formatting must be a fixpoint; stderr: {:?}",
        String::from_utf8_lossy(&check.stderr)
    );
    assert_eq!(std::fs::read_to_string(&path).unwrap(), formatted);
}

// ---------------------------------------------------------------------------
// Cross-surface integrity (issue #114): directory batch mode runs the union
// resolver pass over every import-connected application in the checked set;
// dangling lifecycle wiring is an error, single-file checks stay per-surface.
// ---------------------------------------------------------------------------

/// The shared domain: one class the capabilities hang on.
const UNION_MOX: &str = "package demo\n\nclass Ticket { String title }\n";

/// A design whose only operation carries `capabilities` and emits the
/// event the cluster's `.evt` contract declares — the union pass's
/// emission checks (issue rexlang#114) require a declared event to be
/// produced, so a *fully wired* fixture has to emit it (the flag-tests
/// below break exactly one other wire each; none of their pinned
/// diagnostics involve emissions).
fn union_ddd(capability: Option<&str>) -> String {
    let clause = capability
        .map(|c| format!(" capability {c}"))
        .unwrap_or_default();
    format!(
        "import \"app.mox\";\n\napplication App {{\n    module desk {{\n        service TicketService {{\n            Ticket hire(){clause} emits OrderPlaced;\n        }}\n    }}\n}}\n"
    )
}

/// An actor block granting `TerminateWorker` to Agent.
const UNION_ACTOR: &str = "import \"app.mox\"\n\nactors Support {\n    actor Agent\n    capability TerminateWorker on Ticket\n    grant Agent {\n        permit TerminateWorker\n    }\n}\n";

/// An event contract whose subscription's consumer is `consumer`.
fn union_evt(consumer: &str) -> String {
    format!(
        "import \"app.mox\"\n\nevent OrderPlaced {{\n    orderId: String;\n}}\n\nsubscription ops {{\n    events [ OrderPlaced ]\n    consumer {consumer}\n}}\n"
    )
}

/// A deploy application with one component named `component`.
fn union_deploy(component: &str) -> String {
    format!(
        "import \"desk.ddd\"\n\napplication App {{\n    component {component}: worker {{ }}\n}}\n"
    )
}

/// An interaction file: a view gated on `requires` and an action bound to
/// `action`.
fn union_ifml(action: Option<&str>, requires: Option<&str>) -> String {
    let action_block = action
        .map(|name| format!("\naction \"{name}\" {{\n}}\n"))
        .unwrap_or_default();
    let requires_clause = requires
        .map(|names| format!("    requires: [{names}];\n"))
        .unwrap_or_default();
    format!("import \"app.mox\";\n\nview \"Desk\" {{\n{requires_clause}}}\n{action_block}")
}

/// Writes a five-file application into the test's scratch directory and
/// returns the directory path.
fn write_union_app(
    ddd: &str,
    actor: Option<&str>,
    evt: Option<&str>,
    deploy: Option<&str>,
    ifml: Option<&str>,
) -> PathBuf {
    let dir = scratch_dir();
    std::fs::write(dir.join("app.mox"), UNION_MOX).expect("write mox");
    std::fs::write(dir.join("desk.ddd"), ddd).expect("write ddd");
    if let Some(actor) = actor {
        std::fs::write(dir.join("desk.actor"), actor).expect("write actor");
    }
    if let Some(evt) = evt {
        std::fs::write(dir.join("orders.evt"), evt).expect("write evt");
    }
    if let Some(deploy) = deploy {
        std::fs::write(dir.join("app.deploy"), deploy).expect("write deploy");
    }
    if let Some(ifml) = ifml {
        std::fs::write(dir.join("screen.ifml"), ifml).expect("write ifml");
    }
    dir
}

#[test]
fn directory_check_flags_a_granted_capability_no_op_carries() {
    let dir = write_union_app(&union_ddd(None), Some(UNION_ACTOR), None, None, None);
    let output = rexlang()
        .arg("check")
        .arg(&dir)
        .output()
        .expect("run rexlang check");
    assert_eq!(
        output.status.code(),
        Some(1),
        "dangling grant must fail the batch"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains(
            "capability 'TerminateWorker' is granted but no operation or search carries it"
        ),
        "stderr: {stderr}"
    );
    // rexlang#128: coverage resolves at compile time — the failure is the
    // per-file project pipeline's (the `N of M files failed` tail), with
    // the diagnostic spanned into the actor file, not a phase-2 union
    // finding.
    assert!(
        stderr.contains("files failed"),
        "the dangling grant fails phase 1: {stderr}"
    );
    assert!(
        stderr.contains("permit TerminateWorker"),
        "the rendered diagnostic points at the grant entry: {stderr}"
    );
}

#[test]
fn directory_check_flags_an_unknown_operation_capability() {
    let dir = write_union_app(
        &union_ddd(Some("GhostCapability")),
        Some(UNION_ACTOR),
        None,
        None,
        None,
    );
    let output = rexlang()
        .arg("check")
        .arg(&dir)
        .output()
        .expect("run rexlang check");
    assert_eq!(
        output.status.code(),
        Some(1),
        "unknown capability must fail the batch"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains(
            "unknown capability 'GhostCapability' on operation 'hire' of service 'TicketService'"
        ),
        "stderr: {stderr}"
    );
    // rexlang#128: existence resolves at compile time (the design engine's
    // rule 10 over the project's actor artifacts) — phase 1 owns the
    // failure, and the diagnostic renders spanned against the design.
    assert!(
        stderr.contains("files failed"),
        "the unknown capability fails phase 1: {stderr}"
    );
    assert!(
        stderr.contains("GhostCapability"),
        "the rendered diagnostic points at the capability clause: {stderr}"
    );
}

#[test]
fn directory_check_flags_an_unknown_capability_near_a_declared_name() {
    // One letter off a declared capability: the compile-time diagnostic
    // carries the did-you-mean help (the union pass listed every name).
    let dir = write_union_app(
        &union_ddd(Some("TerminateWorkerX")),
        Some(UNION_ACTOR),
        None,
        None,
        None,
    );
    let output = rexlang()
        .arg("check")
        .arg(&dir)
        .output()
        .expect("run rexlang check");
    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("did you mean 'TerminateWorker'?"),
        "a near-miss names its intended capability: {stderr}"
    );
}

#[test]
fn directory_check_flags_a_consumer_no_component_satisfies() {
    let dir = write_union_app(
        &union_ddd(Some("TerminateWorker")),
        Some(UNION_ACTOR),
        Some(&union_evt("fulfillment_service")),
        Some(&union_deploy("api")),
        None,
    );
    let output = rexlang()
        .arg("check")
        .arg(&dir)
        .output()
        .expect("run rexlang check");
    assert_eq!(
        output.status.code(),
        Some(1),
        "dead inbox must fail the batch"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("subscription 'ops' consumer 'fulfillment_service' does not resolve to a declared component"),
        "stderr: {stderr}"
    );
}

#[test]
fn directory_check_flags_an_action_no_op_backs() {
    let dir = write_union_app(
        &union_ddd(None),
        None,
        None,
        None,
        Some(&union_ifml(Some("HireWorker"), None)),
    );
    let output = rexlang()
        .arg("check")
        .arg(&dir)
        .output()
        .expect("run rexlang check");
    assert_eq!(
        output.status.code(),
        Some(1),
        "unbound action must fail the batch"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("action 'HireWorker' does not bind to any service operation"),
        "stderr: {stderr}"
    );
}

#[test]
fn directory_check_flags_a_requires_name_no_actor_layer_declares() {
    let dir = write_union_app(
        &union_ddd(None),
        Some(UNION_ACTOR),
        None,
        None,
        Some(&union_ifml(None, Some("Ghost"))),
    );
    let output = rexlang()
        .arg("check")
        .arg(&dir)
        .output()
        .expect("run rexlang check");
    assert_eq!(
        output.status.code(),
        Some(1),
        "dangling requires must fail the batch"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("unknown capability or actor 'Ghost' in requires of view 'Desk'"),
        "stderr: {stderr}"
    );
}

#[test]
fn directory_check_passes_a_fully_wired_application() {
    let dir = write_union_app(
        &union_ddd(Some("TerminateWorker")),
        Some(UNION_ACTOR),
        Some(&union_evt("fulfillment_service")),
        Some(&union_deploy("fulfillment_service")),
        Some(&union_ifml(Some("hire"), Some("Agent"))),
    );
    let output = rexlang()
        .arg("check")
        .arg(&dir)
        .output()
        .expect("run rexlang check");
    assert!(
        output.status.success(),
        "a wired application passes: {:?}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stderr).is_empty());
}

#[test]
fn check_design_renders_its_imported_domain_diagnostics() {
    // A domain's own diagnostics reach the report under the import string
    // the design names it by (the driver's DomainUnit path) — the reader
    // keeps both keys renderable, or the diagnostic silently vanishes.
    let dir = scratch_dir().join("design-domain-diags");
    std::fs::create_dir_all(&dir).expect("mkdir");
    let domain = dir.join("broken.mox");
    std::fs::write(&domain, "package broken\n\nclass X { String oops oops }\n").expect("write mox");
    let design = dir.join("design.ddd");
    std::fs::write(
        &design,
        "import \"broken.mox\"\n\napplication A {\n    module m {\n        value X\n    }\n}\n",
    )
    .expect("write ddd");

    let output = rexlang()
        .arg("check")
        .arg(&design)
        .output()
        .expect("run rexlang check");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("expected a feature declaration"),
        "the imported domain's parse diagnostic renders: {stderr}"
    );
    assert!(
        stderr.contains("broken.mox"),
        "the diagnostic renders against the domain file: {stderr}"
    );
}

#[test]
fn single_file_check_stays_per_surface() {
    // The dangling grant of `directory_check_flags_a_granted_capability_no_op_carries`
    // is invisible when the design is checked alone: single-file mode has
    // no union context, and cross-surface checks fire only in directory
    // batch mode where the counterpart surfaces are in hand.
    let dir = write_union_app(&union_ddd(None), Some(UNION_ACTOR), None, None, None);
    let output = rexlang()
        .arg("check")
        .arg(dir.join("desk.ddd"))
        .output()
        .expect("run rexlang check");
    assert!(
        output.status.success(),
        "single-file mode must stay per-surface: {:?}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn explain_prints_the_registry_entry_for_a_known_code() {
    let output = rexlang()
        .args(["explain", "E0104"])
        .output()
        .expect("run rexlang explain");
    assert!(
        output.status.success(),
        "stderr: {:?}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let entry = rex_diagnostics::lookup("E0104").expect("E0104 is registered");
    assert!(stdout.contains(entry.code), "{stdout}");
    assert!(stdout.contains(entry.title), "{stdout}");
    assert!(stdout.contains(entry.fix), "{stdout}");
    assert!(stdout.contains("ifml.check"), "{stdout}");
}

#[test]
fn explain_accepts_driver_camelcase_names() {
    let output = rexlang()
        .args(["explain", "attributeWithClassType"])
        .output()
        .expect("run rexlang explain");
    assert!(
        output.status.success(),
        "stderr: {:?}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let entry = rex_diagnostics::lookup("attributeWithClassType").expect("registered by name");
    assert!(stdout.contains(entry.code), "{stdout}");
    assert!(stdout.contains(entry.title), "{stdout}");
}

#[test]
fn explain_unknown_code_fails_with_a_hint() {
    let output = rexlang()
        .args(["explain", "E9999"])
        .output()
        .expect("run rexlang explain");
    assert_eq!(output.status.code(), Some(1));
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(combined.contains("E9999"), "{combined}");
    assert!(combined.contains("unknown diagnostic code"), "{combined}");
    assert!(combined.contains("explain --list"), "{combined}");
}

#[test]
fn explain_without_code_or_list_fails() {
    let output = rexlang()
        .arg("explain")
        .output()
        .expect("run rexlang explain");
    assert_eq!(output.status.code(), Some(1));
}

#[test]
fn explain_list_prints_the_ledger_and_every_code() {
    let output = rexlang()
        .args(["explain", "--list"])
        .output()
        .expect("run rexlang explain --list");
    assert!(
        output.status.success(),
        "stderr: {:?}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    for band in rex_diagnostics::ledger() {
        assert!(stdout.contains(band.id), "{} missing from --list", band.id);
        assert!(
            stdout.contains(band.range),
            "{} missing from --list",
            band.range
        );
        assert!(
            stdout.contains(band.next_free),
            "{} missing from --list",
            band.next_free
        );
    }
    for entry in rex_diagnostics::all() {
        assert!(
            stdout.contains(entry.code),
            "{} missing from --list",
            entry.code
        );
        assert!(
            stdout.contains(entry.title),
            "{} missing from --list",
            entry.title
        );
    }
}
