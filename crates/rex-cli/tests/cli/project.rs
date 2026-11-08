//! `rexlang` CLI tests: project.

use std::path::{Path, PathBuf};

use crate::support::*;

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
