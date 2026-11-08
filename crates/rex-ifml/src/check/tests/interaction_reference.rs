//! Check tests: interaction reference.

use super::support::*;
use super::*;

// --- interaction-reference resolution (issue #112) ------------------

#[test]
fn dangling_action_invocation_is_reported() {
    let source = r#"
view "Queue" {
    component "intake" {
        type: form;

        on raise(values) -> action("RaiseRefund");
    }
}
"#;
    let diags = diags(source, &shop_model());
    assert_eq!(diags.len(), 1, "{diags:?}");
    assert_eq!(diags[0].code, E_UNKNOWN_ACTION);
    assert_eq!(diags[0].file, "app.ifml");
    assert_eq!(
        diags[0].message,
        "action 'RaiseRefund' is not declared — add action \"RaiseRefund\" { target: <Entity>; }"
    );
    // The span is the referenced string token itself, ready for an
    // editor squiggle.
    let (start, end) = diags[0].span;
    assert_eq!(&source[start..end], "\"RaiseRefund\"");
}

#[test]
fn declared_action_invocation_is_clean() {
    let source = r#"
view "Queue" {
    component "intake" {
        type: form;

        on raise(values) -> action("RaiseRefund");
    }
}

action "RaiseRefund" {
    target: Customer;
}
"#;
    assert!(diags(source, &shop_model()).is_empty());
}

#[test]
fn nested_action_invocations_are_checked_too() {
    let source = r#"
view "Queue" {
    component "intake" {
        type: form;

        on raise(values) -> action("Outer", {
            on success -> action("MissingInner");
        });
    }
}

action "Outer" {
    target: Customer;
}
"#;
    let diags = diags(source, &shop_model());
    assert_eq!(diags.len(), 1, "{diags:?}");
    assert_eq!(diags[0].code, E_UNKNOWN_ACTION);
    assert!(
        diags[0].message.contains("'MissingInner'"),
        "{}",
        diags[0].message
    );
}

#[test]
fn action_declared_in_an_imported_file_resolves() {
    let source = r#"
import "lib.ifml";

view "Queue" {
    component "intake" {
        type: form;

        on raise(values) -> action("RaiseRefund");
    }
}
"#;
    let mut imports = crate::IfmlImports::default();
    imports.provide(
        "app.ifml",
        "lib.ifml",
        "action \"RaiseRefund\" { target: Customer; }",
    );
    let compilation = crate::compile_ifml_str("app.ifml", source, &imports)
        .expect("source must parse and resolve");
    assert!(check_ifml(&compilation, Some(&shop_model())).is_empty());
}

#[test]
fn imported_file_sites_are_not_diagnosed() {
    // An imported file's dangling reference is that file's own
    // compilation's report — the resolver's main-file-only rule.
    let source = r#"
import "lib.ifml";

view "Queue" {
    component "intake" {
        type: form;

        on raise(values) -> stay;
    }
}
"#;
    let mut imports = crate::IfmlImports::default();
    imports.provide(
        "app.ifml",
        "lib.ifml",
        "view \"Lib\" { component \"c\" { type: list; on select(row) -> action(\"Missing\"); } }",
    );
    let compilation = crate::compile_ifml_str("app.ifml", source, &imports)
        .expect("source must parse and resolve");
    assert!(check_ifml(&compilation, Some(&shop_model())).is_empty());
}

#[test]
fn interaction_references_resolve_without_domains() {
    let source = r#"
view "Queue" {
    component "intake" {
        type: form;

        on raise(values) -> action("RaiseRefund");
    }
}
"#;
    let compilation = crate::compile_ifml_str("app.ifml", source, &crate::IfmlImports::default())
        .expect("source must parse and resolve");
    let diags = check_ifml(&compilation, None);
    assert_eq!(diags.len(), 1, "{diags:?}");
    assert_eq!(diags[0].code, E_UNKNOWN_ACTION);
}

#[test]
fn navigate_target_must_be_a_declared_view() {
    let source = r#"
view "Queue" {
    component "rows" {
        type: list;

        on select(row) -> navigate("Detail", { id: row.id });
    }
}

view "Detail" {
    component "form" {
        type: form;
    }
}
"#;
    assert!(diags(source, &shop_model()).is_empty());

    let dangling = source.replace("navigate(\"Detail\"", "navigate(\"Missing\"");
    let diags = diags(&dangling, &shop_model());
    assert_eq!(diags.len(), 1, "{diags:?}");
    assert_eq!(diags[0].code, E_UNKNOWN_VIEW);
    assert_eq!(
        diags[0].message,
        "navigate target 'Missing' is not a declared view or container"
    );
    let (start, end) = diags[0].span;
    assert_eq!(&dangling[start..end], "\"Missing\"");
}

#[test]
fn refresh_target_must_be_a_declared_component() {
    let source = r#"
view "Queue" {
    component "rows" {
        type: list;

        on change -> refresh("rows");
    }
}

module "Panel" {
    input { page: Int = 1 }
    output { n: Int }

    component "pager" {
        type: list;
    }
}
"#;
    assert!(
        diags(source, &shop_model()).is_empty(),
        "own and module-declared components resolve"
    );

    let dangling = source.replace("refresh(\"rows\")", "refresh(\"nope\")");
    let diags = diags(&dangling, &shop_model());
    assert_eq!(diags.len(), 1, "{diags:?}");
    assert_eq!(diags[0].code, E_UNKNOWN_COMPONENT);
    assert_eq!(
        diags[0].message,
        "refresh target 'nope' is not a declared component"
    );
    let (start, end) = diags[0].span;
    assert_eq!(&dangling[start..end], "\"nope\"");
}

#[test]
fn command_navigation_targets_resolve_too() {
    let source = r#"
command "customers" {
    on run -> navigate("Missing");
}
"#;
    let diags = diags(source, &shop_model());
    assert_eq!(diags.len(), 1, "{diags:?}");
    assert_eq!(diags[0].code, E_UNKNOWN_VIEW);
    assert!(
        diags[0].message.contains("'Missing'"),
        "{}",
        diags[0].message
    );
}
