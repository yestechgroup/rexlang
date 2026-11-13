//! Characterization test for the cross-group diagnostic emission ORDER of
//! [`compile_deploy_file`](rex_driver::compile_deploy_str) (issue
//! rexlang#164): one deployment file engineered to trip every stage group —
//! the import pass, the application pass (component settings, env,
//! connections), the profile pass (target validation), and the deployment
//! pass (profile defaults, deployment overrides, policy typing) — with the
//! exact ordered sequence of (severity, message, help) pinned.
//!
//! The stage decomposition of `compile_deploy_file` must keep this
//! sequence byte-identical: the goldens pin artifact bytes, this pins the
//! interleaving *between* the groups (and the load-bearing intra-group
//! rules: a duplicate component setting must still skip value validation,
//! and a profile default is diagnosed against the application only in the
//! deployment pass, not in the profile pass). The statement order in
//! `compile_deploy_file` — and now its stage functions — is the spec.

use rex_driver::{compile_deploy_str, DeployCompilation, Severity};

/// One deployment tripping every stage, in the order the stages run: two
/// missing imports (the single per-import pass), application `A` (a
/// duplicate component setting — the duplicate must skip value validation —
/// a duplicate env name, and a connection to an unknown component), the
/// duplicate application `A` (clean), profile `broken` (unknown target) and
/// profile `local` (a shape-valid default whose component only fails in the
/// deployment pass, plus a well-typed and an ill-typed policy), and
/// deployment `dev` (the profile default, an unknown override, and the
/// ill-typed policy evaluation).
const DEPLOY: &str = r#"
import "missing.ddd"
import "missing.ifml"

application A {
    component api: api {
        port: 18082
        port: "wrong"
        env {
            LOG = "a"
            LOG = "b"
        }
    }
    connects api -> ghost
}

application A {
    component web: frontend {
        entrypoint: "index.ts"
    }
}

profile broken {
    target: prometheus
}

profile local {
    target: standalone
    defaults {
        nosuch.setting: 1
    }
    require (api.port == 18082)
    require (nosuch.thing == 1)
}

deployment dev for A {
    use local
    configure {
        alsonot.port: 2
    }
}
"#;

/// The pinned emission order as `(severity, message, help)` triples,
/// ignoring the per-file path tags (every diagnostic here belongs to
/// `app.deploy`).
fn sequence(compilation: &DeployCompilation) -> Vec<(Severity, String, Option<String>)> {
    compilation
        .diagnostics
        .iter()
        .map(|(path, diagnostic)| {
            assert_eq!(path, "app.deploy", "unexpected source file");
            (
                diagnostic.severity,
                diagnostic.message.clone(),
                diagnostic.help.clone(),
            )
        })
        .collect()
}

#[test]
fn the_diagnostic_emission_order_is_stable() {
    let compilation = compile_deploy_str("app.deploy", DEPLOY, &[], &[]);
    let actual = sequence(&compilation);
    // Filled from the current behavior — this test characterizes the
    // emission order so the stage decomposition of `compile_deploy_file`
    // cannot perturb it. The stages in order: the import pass (one
    // diagnostic per import, in import order), the application pass (per
    // application: unique name, then per component — settings, env — then
    // connections), the profile pass (unique name, target, shape-valid
    // defaults), and the deployment pass (effective settings: profile
    // defaults applied before the deployment's overrides, then capability
    // validation, then the profile's policies evaluated per deployment).
    let expected: Vec<(Severity, String, Option<String>)> = vec![
        // Stage 1 — imports.
        (
            Severity::Error,
            "imported file \"missing.ddd\" was not provided".to_string(),
            Some("imports must name a .ddd design or an .ifml flow the host provides".to_string()),
        ),
        (
            Severity::Error,
            "imported file \"missing.ifml\" was not provided".to_string(),
            Some("imports must name a .ddd design or an .ifml flow the host provides".to_string()),
        ),
        // Stage 2 — applications: the duplicate setting fires; the duplicate
        // skips value validation, so no "setting 'port' expects a string"
        // error may follow it.
        (
            Severity::Error,
            "duplicate setting 'port' on component 'api'".to_string(),
            Some("a setting is declared once per level".to_string()),
        ),
        (
            Severity::Error,
            "duplicate env 'LOG' on component 'api'".to_string(),
            Some("an env name is declared once per component".to_string()),
        ),
        (
            Severity::Error,
            "unknown component 'ghost' in connection from 'api' to 'ghost'".to_string(),
            Some("connections must reference components of application 'A'".to_string()),
        ),
        // The duplicate application's name fires after all of application
        // `A`'s own diagnostics; the duplicate's body stays clean.
        (
            Severity::Error,
            "duplicate application 'A'".to_string(),
            Some("'A' is already declared in this file".to_string()),
        ),
        // Stage 3 — profiles: the unknown target fires at profile time;
        // `local` is shape-valid here (its default names no component yet).
        (
            Severity::Error,
            "unknown deployment target 'prometheus'".to_string(),
            Some(
                "known targets: standalone, dockerCompose, kubernetes, cloudflareWorkers"
                    .to_string(),
            ),
        ),
        // Stage 4 — deployments: the profile default (applied before the
        // override), then the override, then the policies — the well-typed
        // one holds silently, the ill-typed one is diagnosed.
        (
            Severity::Error,
            "default of profile 'local' names unknown component 'nosuch' — application 'A' has no such component".to_string(),
            Some(
                "defaults and overrides must name components of the deployed application"
                    .to_string(),
            ),
        ),
        (
            Severity::Error,
            "override of deployment 'dev' names unknown component 'alsonot' — application 'A' has no such component".to_string(),
            Some(
                "defaults and overrides must name components of the deployed application"
                    .to_string(),
            ),
        ),
        (
            Severity::Error,
            "invalid policy condition: unknown name `nosuch`".to_string(),
            Some("in `require (nosuch.thing == 1)`".to_string()),
        ),
    ];
    assert_eq!(actual, expected);
    // Every diagnostic above is an error, so the artifact must have dropped.
    assert!(
        compilation.model.is_none(),
        "an error-severity diagnostic anywhere must drop the artifact"
    );
}
