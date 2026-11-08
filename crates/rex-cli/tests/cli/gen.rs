//! `rexlang` CLI tests: gen.

use crate::support::*;

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
