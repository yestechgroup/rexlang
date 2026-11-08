//! `rexlang` CLI tests: deploy.

use std::path::PathBuf;

use crate::support::*;

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
