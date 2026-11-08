//! Check tests: authorization.

use super::support::*;
use super::*;

#[test]
fn unknown_capability_reports_e0107_on_the_requires_site() {
    let source = r#"
view "Edit" {
    requires: [ManageCustomer, Ghost];
}
"#;
    let diags = diags_with_actors(source, &shop_model(), &[auth_model()]);
    assert_eq!(diags.len(), 1, "{diags:?}");
    assert_eq!(diags[0].code, E_UNKNOWN_CAPABILITY);
    assert_eq!(
            diags[0].message,
            "capability 'Ghost' is not declared — add \"capability Ghost on <Class>;\" to an actors block"
        );
    let (start, end) = diags[0].span;
    assert_eq!(&source[start..end], "Ghost");
}

#[test]
fn unknown_role_reports_e0108_on_the_roles_site() {
    let source = r#"
view "List" {
    roles: [admin, ghost];
}
"#;
    let diags = diags_with_actors(source, &shop_model(), &[auth_model()]);
    assert_eq!(diags.len(), 1, "{diags:?}");
    assert_eq!(diags[0].code, E_UNKNOWN_ROLE);
    assert_eq!(
        diags[0].message,
        "role 'ghost' is not declared — add \"actor ghost;\" to an actors block"
    );
    let (start, end) = diags[0].span;
    assert_eq!(&source[start..end], "ghost");
}

#[test]
fn known_capabilities_and_roles_produce_no_diagnostics() {
    let source = r#"
view "Edit" {
    requires: [ManageCustomer, ApproveCustomer];
    roles: [admin, manager];

    component "form" {
        on save(v) requires: [ManageCustomer] -> stay;
    }
}
"#;
    assert!(diags_with_actors(source, &shop_model(), &[auth_model()]).is_empty());
}

#[test]
fn actor_tables_union_across_models_and_blocks() {
    let billing = ActorModel::new().block(
        ActorsDef::new("Billing")
            .actor(ActorDef::new("auditor"))
            .capability(CapabilityDef::new("RunBilling", class_ref("Order"))),
    );
    let source = r#"
view "Ledger" {
    requires: [ManageCustomer, RunBilling];
    roles: [manager, auditor];
}
"#;
    let diags = diags_with_actors(source, &shop_model(), &[auth_model(), billing]);
    assert!(diags.is_empty(), "{diags:?}");
}

#[test]
fn severity_defaults_to_error_for_resolver_and_checker_diagnostics() {
    let source = r#"
view "Queue" {
    use "Missing";

    component "intake" {
        type: form;

        on raise(values) -> action("RaiseRefund");
    }
}
"#;
    let (compilation, resolver_diags) =
        crate::compile_ifml_lenient("app.ifml", source, &crate::IfmlImports::default());
    assert_eq!(resolver_diags.len(), 1, "{resolver_diags:?}");
    assert_eq!(resolver_diags[0].code, crate::E_UNKNOWN_MODULE);
    assert_eq!(resolver_diags[0].severity, crate::Severity::Error);
    assert!(resolver_diags[0].severity.is_error());

    let diags = check_ifml(&compilation, None);
    assert!(
        diags
            .iter()
            .any(|diagnostic| diagnostic.code == E_UNKNOWN_ACTION),
        "{diags:?}"
    );
    assert!(
        diags
            .iter()
            .all(|diagnostic| diagnostic.severity == crate::Severity::Error),
        "{diags:?}"
    );
}
