//! Check tests: instantiated modules.

use super::support::*;
use super::*;

// --- instantiated-module references (issue #114, interaction layer) -

/// Compiles `source` as `app.ifml` with `imports` provided, then
/// checks against `model` (no actor layer) — error-severity only, like
/// [`diags`].
fn diags_with_import(
    source: &str,
    imports: &crate::IfmlImports,
    model: &Model,
) -> Vec<IfmlDiagnostic> {
    let compilation = crate::compile_ifml_str("app.ifml", source, imports)
        .expect("source must parse and resolve");
    check_ifml(&compilation, Some(model))
        .into_iter()
        .filter(|diagnostic| diagnostic.severity.is_error())
        .collect()
}

#[test]
fn navigate_target_may_be_a_declared_container() {
    let source = r#"
view "Wizard" {
    container "stepOne" {
        component "entry" {
            type: form;

            on next(values) -> navigate("stepTwo");
        }
    }
    container "stepTwo" {
        component "review" {
            type: details;

            on previous -> navigate("stepOne");
        }
    }
}

module "Pager" {
    input { page: Int = 1 }
    output { n: Int }

    container "page" {
        component "pager" {
            type: list;

            on first -> navigate("page");
        }
    }
}
"#;
    assert!(
        diags(source, &shop_model()).is_empty(),
        "container targets resolve — wizard steps and module pages"
    );

    let dangling = source.replace("navigate(\"stepTwo\")", "navigate(\"Missing\")");
    let diags = diags(&dangling, &shop_model());
    assert_eq!(diags.len(), 1, "{diags:?}");
    assert_eq!(diags[0].code, E_UNKNOWN_VIEW);
    assert_eq!(
        diags[0].message,
        "navigate target 'Missing' is not a declared view or container"
    );
}

#[test]
fn instantiated_module_dangling_references_are_diagnosed() {
    // A `use` instantiates the module: its body's references are the
    // app's surface now — a dangling navigate is the missing-button
    // class, diagnosed at the referenced token with the imported
    // file's path.
    let source = r#"
import "lib.ifml";

view "Queue" {
    use "Inbox" as inbox;
}
"#;
    let mut imports = crate::IfmlImports::default();
    imports.provide(
        "app.ifml",
        "lib.ifml",
        r#"module "Inbox" {
    input { page: Int = 1 }
    output { n: Int }

    component "list" {
        type: list;

        on select(row) -> navigate("Folder", { id: row.id });
        on markRead -> action("MarkRead");
        on reload -> refresh("nope");
    }
}"#,
    );
    let mut diags = diags_with_import(source, &imports, &shop_model());
    diags.sort_by(|a, b| a.code.cmp(b.code).then(a.message.cmp(&b.message)));
    assert_eq!(diags.len(), 3, "{diags:?}");
    assert_eq!(diags[0].code, E_UNKNOWN_ACTION);
    assert!(
        diags[0].message.contains("'MarkRead'"),
        "{}",
        diags[0].message
    );
    assert_eq!(diags[1].code, E_UNKNOWN_VIEW);
    assert_eq!(diags[2].code, E_UNKNOWN_COMPONENT);
    for diag in &diags {
        assert_eq!(diag.file, "lib.ifml", "{diag:?}");
        let (start, end) = diag.span;
        assert!(start < end, "the span is the referenced token");
    }
}

#[test]
fn instantiated_module_references_resolve_when_declared() {
    // Union-wide resolution: the module's references may terminate at
    // the app's own declarations.
    let source = r#"
import "lib.ifml";

view "Queue" {
    use "Inbox" as inbox;
}

view "Folder" {
    component "rows" { type: list; }
}

action "MarkRead" {
    target: Customer;
}
"#;
    let mut imports = crate::IfmlImports::default();
    imports.provide(
        "app.ifml",
        "lib.ifml",
        r#"module "Inbox" {
    input { page: Int = 1 }
    output { n: Int }

    component "list" {
        type: list;

        on select(row) -> navigate("Folder", { id: row.id });
        on markRead -> action("MarkRead");
        on reload -> refresh("list");
    }
}"#,
    );
    assert!(diags_with_import(source, &imports, &shop_model()).is_empty());
}

#[test]
fn composition_reaches_transitive_modules() {
    // M1 composes M2: instantiating M1 instantiates M2 — M2's dangling
    // reference is diagnosed too.
    let source = r#"
import "lib.ifml";

view "Queue" {
    use "Outer" as outer;
}
"#;
    let mut imports = crate::IfmlImports::default();
    imports.provide(
        "app.ifml",
        "lib.ifml",
        r#"module "Outer" {
    input { page: Int = 1 }
    output { n: Int }

    use "Inner" as inner;

    component "shell" { type: list; }
}

module "Inner" {
    input { page: Int = 1 }
    output { n: Int }

    component "list" {
        type: list;

        on select(row) -> navigate("Nowhere");
    }
}"#,
    );
    let diags = diags_with_import(source, &imports, &shop_model());
    assert_eq!(diags.len(), 1, "{diags:?}");
    assert_eq!(diags[0].code, E_UNKNOWN_VIEW);
    assert_eq!(diags[0].file, "lib.ifml");
}

#[test]
fn same_file_unused_modules_stay_undiagnosed() {
    // Granularity is the module body, not the file: instantiating
    // `Inbox` does not instantiate its sibling `Unused` — that
    // module's dangling reference stays its own compilation's report.
    let source = r#"
import "lib.ifml";

view "Queue" {
    use "Inbox" as inbox;
}

view "Folder" {
    component "rows" { type: list; }
}

action "MarkRead" {
    target: Customer;
}
"#;
    let mut imports = crate::IfmlImports::default();
    imports.provide(
        "app.ifml",
        "lib.ifml",
        r#"module "Inbox" {
    input { page: Int = 1 }
    output { n: Int }

    component "list" {
        type: list;

        on select(row) -> navigate("Folder", { id: row.id });
        on markRead -> action("MarkRead");
        on reload -> refresh("list");
    }
}

module "Unused" {
    input { page: Int = 1 }
    output { n: Int }

    component "lost" {
        type: list;

        on select(row) -> navigate("Nowhere");
    }
}"#,
    );
    assert!(diags_with_import(source, &imports, &shop_model()).is_empty());
}

#[test]
fn uninstantiated_import_sites_stay_undiagnosed() {
    // The narrowed own-report rule: an imported file whose modules the
    // app never instantiates keeps its references to itself — a
    // pattern-library file unused by this app is not this app's error.
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
            "module \"Unused\" { input { page: Int = 1 } output { n: Int } component \"c\" { type: list; on select(row) -> action(\"Missing\"); } }",
        );
    assert!(diags_with_import(source, &imports, &shop_model()).is_empty());
}

#[test]
fn instantiated_module_auth_names_are_diagnosed() {
    // The authorization pass runs on instantiated-module sites under
    // the same reachability rule.
    let source = r#"
import "lib.ifml";

view "Queue" {
    use "Inbox" as inbox;
}
"#;
    let mut imports = crate::IfmlImports::default();
    imports.provide(
        "app.ifml",
        "lib.ifml",
        r#"module "Inbox" {
    input { page: Int = 1 }
    output { n: Int }

    component "list" {
        type: list;

        on select(row) requires: [Missing] -> navigate("Folder");
        on reset -> stay;
    }
}"#,
    );
    let compilation = crate::compile_ifml_str("app.ifml", source, &imports)
        .expect("source must parse and resolve");
    let diags = check_ifml_with_actors(&compilation, Some(&shop_model()), &[auth_model()]);
    assert_eq!(diags.len(), 2, "{diags:?}");
    let unknown_view = diags.iter().find(|d| d.code == E_UNKNOWN_VIEW);
    let unknown_capability = diags.iter().find(|d| d.code == E_UNKNOWN_CAPABILITY);
    let capability = unknown_capability.expect("the capability site is diagnosed");
    assert!(
        capability.message.contains("'Missing'"),
        "{}",
        capability.message
    );
    assert_eq!(capability.file, "lib.ifml");
    let view = unknown_view.expect("the navigate site is diagnosed");
    assert_eq!(view.file, "lib.ifml");
}

#[test]
fn instantiated_module_auth_names_resolve_against_the_actor_layer() {
    let source = r#"
import "lib.ifml";

view "Queue" {
    use "Inbox" as inbox;
}

    view "Folder" {
        component "rows" {
            type: list;
            data: Customer;
        }
    }
"#;
    let mut imports = crate::IfmlImports::default();
    imports.provide(
        "app.ifml",
        "lib.ifml",
        r#"module "Inbox" {
    input { page: Int = 1 }
    output { n: Int }

    component "list" {
        type: list;

        on select(row) requires: [ManageCustomer] -> navigate("Folder");
        on reset -> stay;
    }
}"#,
    );
    let compilation = crate::compile_ifml_str("app.ifml", source, &imports)
        .expect("source must parse and resolve");
    assert!(check_ifml_with_actors(&compilation, Some(&shop_model()), &[auth_model()]).is_empty());
}
