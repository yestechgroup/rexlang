//! `rexlang` CLI tests: ifml.

use std::path::PathBuf;

use crate::support::*;

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

/// A `.ifml` flow whose only finding is the W0150 dead-action lint: the
/// action is declared, never invoked, and its `target:` resolves against
/// the imported domain.
const LINT_WARNING_IFML: &str = r#"import "ifml-domain.mox";

domain "demo" {
    schema "demo";
}

view "Home" {
    params { q: String };
}

action "Orphan" {
    target: Product;
}
"#;

#[test]
fn ifml_warning_does_not_fail_check() {
    let _ = write_source("ifml-domain.mox", IFML_DOMAIN);
    let flow = write_source("warned.ifml", LINT_WARNING_IFML);
    let output = rexlang()
        .args(["check", flow.to_str().unwrap()])
        .output()
        .expect("run rexlang check");
    assert!(
        output.status.success(),
        "a lint warning never fails the check: stderr {:?}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        format!("OK {}\n", flow.display())
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("W0150"), "stderr: {stderr}");
    assert!(
        stderr.contains("action 'Orphan' is declared but never invoked"),
        "stderr: {stderr}"
    );
}

#[test]
fn deny_warnings_fails_on_ifml_warning() {
    let _ = write_source("ifml-domain.mox", IFML_DOMAIN);
    let flow = write_source("warned.ifml", LINT_WARNING_IFML);
    let output = rexlang()
        .args(["check", "--deny-warnings", flow.to_str().unwrap()])
        .output()
        .expect("run rexlang check --deny-warnings");
    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("W0150"), "stderr: {stderr}");
    assert!(
        String::from_utf8_lossy(&output.stdout).is_empty(),
        "a denied file prints no OK line"
    );
}

#[test]
fn deny_warnings_does_not_break_error_failure() {
    let _ = write_source("ifml-domain.mox", IFML_DOMAIN);
    let flow = write_source(
        "errored.ifml",
        r#"import "ifml-domain.mox";

view "Home" {
    params { q: String };

    component "intake" {
        type: form;

        on raise(values) -> action("Missing");
    }
}
"#,
    );
    let output = rexlang()
        .args(["check", "--deny-warnings", flow.to_str().unwrap()])
        .output()
        .expect("run rexlang check --deny-warnings");
    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("E0104"), "stderr: {stderr}");
}

#[test]
fn canonical_ifml_fixtures_produce_no_warnings() {
    // The compile-and-check closures that stand for the canonical surface:
    // the conformance fixtures and the example apps (which carry the
    // pattern library's app-facing declaration blocks — `patterns/ifml`
    // files are libraries whose dangling targets the consumers declare,
    // so they cannot be checked standalone; their lint silence is pinned
    // by rex-ifml's `every_pattern_file_checks_without_warnings`).
    let fixtures: Vec<PathBuf> = [
        "tests/conformance/ifml/app.ifml",
        "tests/conformance/ifml/cli.ifml",
        "examples/cli.ifml",
        "examples/support.ifml",
        "examples/ecommerce.ifml",
    ]
    .iter()
    .map(|relative| workspace_root().join(relative))
    .collect();
    assert!(!fixtures.is_empty());
    for fixture in &fixtures {
        let output = rexlang()
            .args(["check", fixture.to_str().unwrap()])
            .output()
            .expect("run rexlang check");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            output.status.success(),
            "{}: stderr {stderr}",
            fixture.display()
        );
        assert!(
            !stderr.contains("W015"),
            "{} must be warning-silent: {stderr}",
            fixture.display()
        );
    }
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
