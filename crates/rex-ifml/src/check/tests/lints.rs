//! Check tests: lints.

use super::support::*;
use super::*;

// --- lints (issue #119) ----------------------------------------------

/// Compiles `source` with no imports and checks with no domains: the lint
/// pass is domain-independent, so pure-lint cases need no fixture model.
fn lint_diags(source: &str) -> Vec<IfmlDiagnostic> {
    let compilation = crate::compile_ifml_str("app.ifml", source, &crate::IfmlImports::default())
        .expect("source must parse and resolve");
    check_ifml(&compilation, None)
}

/// [`lint_diags`] with one import provided.
fn lint_diags_with_import(source: &str, imports: &crate::IfmlImports) -> Vec<IfmlDiagnostic> {
    let compilation = crate::compile_ifml_str("app.ifml", source, imports)
        .expect("source must parse and resolve");
    check_ifml(&compilation, None)
}

#[test]
fn dead_action_warns_w0150_at_its_name() {
    let source = r#"
view "Home" {
    params { q: String };
}

action "Orphan" {
    target: Customer;
}
"#;
    let diags = lint_diags(source);
    assert_eq!(diags.len(), 1, "{diags:?}");
    assert_eq!(diags[0].code, W_DEAD_ACTION);
    assert_eq!(diags[0].severity, crate::Severity::Warning);
    assert!(!diags[0].severity.is_error());
    assert_eq!(
        diags[0].message,
        "action 'Orphan' is declared but never invoked"
    );
    let (start, end) = diags[0].span;
    assert_eq!(&source[start..end], "\"Orphan\"");
}

#[test]
fn invoked_action_is_not_dead() {
    let source = r#"
view "Home" {
    params { q: String };

    on save(values) -> action("Orphan");
}

action "Orphan" {
    target: Customer;
}
"#;
    assert!(lint_diags(source).is_empty());
}

#[test]
fn action_invoked_from_an_imported_module_is_not_dead() {
    let mut imports = crate::IfmlImports::default();
    imports.provide(
        "app.ifml",
        "lib.ifml",
        r#"module "Panel" {
    input { page: Int = 1 }
    output { n: Int }

    component "body" {
        type: list;

        on go -> action("Orphan");
    }
}"#,
    );
    let source = r#"
import "lib.ifml";

view "Home" {
    params { q: String };

    use "Panel" as panel;
}

action "Orphan" {
    target: Customer;
}
"#;
    assert!(lint_diags_with_import(source, &imports).is_empty());
}

#[test]
fn unreachable_view_warns_w0151_at_its_name() {
    let source = r#"view "Entry" {
    params { q: String };
}

view "Ghost" {
    params { q: String };
}
"#;
    let diags = lint_diags(source);
    assert_eq!(diags.len(), 1, "{diags:?}");
    assert_eq!(diags[0].code, W_UNREACHABLE_VIEW);
    assert_eq!(diags[0].severity, crate::Severity::Warning);
    assert!(
        diags[0]
            .message
            .starts_with("view 'Ghost' is never navigated to"),
        "{}",
        diags[0].message
    );
    let (start, end) = diags[0].span;
    assert_eq!(&source[start..end], "\"Ghost\"");
}

#[test]
fn navigated_and_landmark_and_first_views_are_exempt() {
    let navigated = r#"view "Entry" {
    component "menu" {
        type: list;

        on select(row) -> navigate("Ghost");
    }
}

view "Ghost" {
    params { q: String };
}
"#;
    assert!(lint_diags(navigated).is_empty());

    let landmark = r#"view "Entry" {
    params { q: String };
}

view "Ghost" {
    params { q: String };
    landmark: true;
}
"#;
    assert!(lint_diags(landmark).is_empty());

    let first = "view \"Shell\" { params { q: String }; }\n";
    assert!(lint_diags(first).is_empty());
}

#[test]
fn shadowed_module_warns_w0152_at_the_main_declaration() {
    let source = r#"
import "lib.ifml";

view "Home" {
    params { q: String };

    use "Pager" { page: 1; };
}

module "Pager" {
    input { page: Int = 1 }
    output { total: Int }
}
"#;
    let mut imports = crate::IfmlImports::default();
    imports.provide(
        "app.ifml",
        "lib.ifml",
        "module \"Pager\" { input { page: Int = 1, pageSize: Int = 25 } output { total: Int } }",
    );
    let diags = lint_diags_with_import(source, &imports);
    assert_eq!(diags.len(), 1, "{diags:?}");
    assert_eq!(diags[0].code, W_SHADOWED_MODULE);
    assert_eq!(diags[0].severity, crate::Severity::Warning);
    assert_eq!(
        diags[0].message,
        "module 'Pager' shadows an imported module of the same name (from 'lib.ifml')"
    );
    let (start, end) = diags[0].span;
    assert_eq!(&source[start..end], "\"Pager\"");
}

#[test]
fn renamed_module_silences_the_shadow_warning() {
    let source = r#"
import "lib.ifml";

view "Home" {
    params { q: String };

    use "LocalPager" { page: 1; };
}

module "LocalPager" {
    input { page: Int = 1 }
    output { total: Int }
}
"#;
    let mut imports = crate::IfmlImports::default();
    imports.provide(
        "app.ifml",
        "lib.ifml",
        "module \"Pager\" { input { page: Int = 1, pageSize: Int = 25 } output { total: Int } }",
    );
    assert!(lint_diags_with_import(source, &imports).is_empty());
}

#[test]
fn component_without_data_warns_w0153() {
    let source = r#"
view "Entry" {
    params { q: String };

    component "ghost" {
        type: list;
        fields: [name];
    }
}
"#;
    let diags = lint_diags(source);
    assert_eq!(diags.len(), 1, "{diags:?}");
    assert_eq!(diags[0].code, W_COMPONENT_WITHOUT_DATA);
    assert_eq!(diags[0].severity, crate::Severity::Warning);
    assert!(
        diags[0]
            .message
            .contains("component 'ghost' binds no entity data"),
        "{}",
        diags[0].message
    );
    let (start, end) = diags[0].span;
    assert_eq!(&source[start..end], "fields: [name];");
}

#[test]
fn data_bound_and_interactive_components_are_exempt() {
    let bound = r#"
view "Entry" {
    params { q: String };

    component "grid" {
        type: list;
        data: Item;
        fields: [name];
    }
}
"#;
    assert!(lint_diags(bound).is_empty());

    let interactive = r#"
view "Entry" {
    params { q: String };

    component "actions" {
        type: list;

        on select(row) -> stay;
    }
}
"#;
    assert!(lint_diags(interactive).is_empty());
}

#[test]
fn view_without_domain_binding_warns_w0154() {
    let source = r#"view "Entry" {
    params { q: String };
}

view "Ghost" {
    component "metric" {
        type: chart;
        chart metric { label: m; values: [v]; }
    }
}
"#;
    let diags = lint_diags(source);
    assert_eq!(diags.len(), 1, "{diags:?}");
    assert_eq!(diags[0].code, W_VIEW_WITHOUT_DOMAIN);
    assert_eq!(diags[0].severity, crate::Severity::Warning);
    assert!(
        diags[0]
            .message
            .starts_with("view 'Ghost' has no domain binding"),
        "{}",
        diags[0].message
    );
    let (start, end) = diags[0].span;
    assert_eq!(&source[start..end], "\"Ghost\"");
}

#[test]
fn data_params_and_composition_are_domain_ties() {
    let data = r#"
view "Entry" {
    params { q: String };
}

view "Bound" {
    component "grid" {
        type: chart;
        data: Item;
        chart metric { label: m; values: [v]; }
    }
}
"#;
    assert!(lint_diags(data).is_empty());

    let params = r#"
view "Entry" {
    params { q: String };
}

view "Shell" {
    params { id: Uuid };
    landmark: true;
}
"#;
    assert!(lint_diags(params).is_empty());

    let composed = r#"
view "Entry" {
    params { q: String };
}

view "Composed" {
    use "Pager" as pager;
}
"#;
    let composed = format!(
        "{composed}\n\nmodule \"Pager\" {{ input {{ page: Int = 1 }} output {{ total: Int }} }}"
    );
    assert!(lint_diags(&composed).is_empty());
}

#[test]
fn lints_only_consider_the_main_file() {
    // The imported pattern declares a dead action and a data-less
    // component — library content, never the app's warnings.
    let source = r#"
import "lib.ifml";

view "Home" {
    params { q: String };
}
"#;
    let mut imports = crate::IfmlImports::default();
    imports.provide(
        "app.ifml",
        "lib.ifml",
        r#"action "Ghost" { target: Customer; }

module "Unused" {
    input { page: Int = 1 }
    output { n: Int }

    component "lost" {
        type: list;
    }
}"#,
    );
    assert!(lint_diags_with_import(source, &imports).is_empty());
}

#[test]
fn lints_have_warning_severity_and_errors_keep_error_severity() {
    let source = r#"
view "Home" {
    params { q: String };

    component "intake" {
        type: form;

        on raise(values) -> action("Missing");
    }
}

action "Orphan" {
    target: Customer;
}
"#;
    let diags = lint_diags(source);
    assert_eq!(diags.len(), 2, "{diags:?}");
    let dangling = diags
        .iter()
        .find(|diagnostic| diagnostic.code == E_UNKNOWN_ACTION)
        .expect("the dangling invocation is diagnosed");
    assert_eq!(dangling.severity, crate::Severity::Error);
    let dead = diags
        .iter()
        .find(|diagnostic| diagnostic.code == W_DEAD_ACTION)
        .expect("the dead declaration warns");
    assert_eq!(dead.severity, crate::Severity::Warning);
}

#[test]
fn empty_actor_models_skip_authorization_checking() {
    // The compatibility contract: without an actor dimension no
    // authorization diagnostic is ever produced.
    let source = r#"
view "Edit" {
    params { id: Uuid };
    requires: [Ghost];
    roles: [ghost];
}
"#;
    let compilation = crate::compile_ifml_str("app.ifml", source, &crate::IfmlImports::default())
        .expect("source must parse and resolve");
    assert!(check_ifml(&compilation, Some(&shop_model())).is_empty());
    assert!(check_ifml_with_actors(&compilation, Some(&shop_model()), &[]).is_empty());
    assert!(check_ifml_with_actors(&compilation, None, &[]).is_empty());
}

#[test]
fn imported_files_authorization_sites_are_that_files_own_report() {
    // Main-file-only discipline, same as the interaction
    // cross-reference: the imported file's dangling `requires` is
    // diagnosed when that file is compiled as the main file, not in
    // the importer's report.
    let main = r#"import "part.ifml";

view "Home" {
    roles: [admin];
}
"#;
    let part = r#"view "Sub" {
    requires: [Ghost];
    roles: [admin];
}
"#;
    let mut imports = crate::IfmlImports::default();
    imports.provide("app.ifml", "part.ifml", part.to_string());
    let compilation = crate::compile_ifml_str("app.ifml", main, &imports).expect("main compiles");
    let diags = check_ifml_with_actors(&compilation, Some(&shop_model()), &[auth_model()]);
    assert!(diags.is_empty(), "{diags:?}");

    let own = crate::compile_ifml_str("part.ifml", part, &crate::IfmlImports::default())
        .expect("part compiles");
    let diags = check_ifml_with_actors(&own, Some(&shop_model()), &[auth_model()]);
    assert_eq!(diags.len(), 1, "{diags:?}");
    assert_eq!(diags[0].code, E_UNKNOWN_CAPABILITY);
}

#[test]
fn authorization_and_expression_checks_compose() {
    let source = r#"
view "Edit" {
    requires: [Ghost];

    component "grid" {
        type: list;
        data: Widget;
    }
}
"#;
    let diags = diags_with_actors(source, &shop_model(), &[auth_model()]);
    let codes: Vec<&str> = diags.iter().map(|diagnostic| diagnostic.code).collect();
    assert!(codes.contains(&E_UNKNOWN_CAPABILITY), "{diags:?}");
    assert!(codes.contains(&E_UNKNOWN_ENTITY), "{diags:?}");
}

#[test]
fn command_options_and_arguments_bind_typed_expressions() {
    let source = r#"
command "customers" {
    argument customer: Customer;
    option limit: Int = 20;

    on run if limit > 10 -> action("ListCustomers", {
        name: customer.name;
    });
}

action "ListCustomers" {
    target: Customer;
}
"#;
    assert!(diags(source, &shop_model()).is_empty());

    let invalid = source.replace("limit > 10", "limit > \"ten\"");
    let diags = diags(&invalid, &shop_model());
    assert_eq!(diags.len(), 1, "{diags:?}");
    assert_eq!(diags[0].code, E_TYPE_MISMATCH);
    assert!(
        diags[0].message.contains("command 'customers' event 'run'"),
        "{}",
        diags[0].message
    );
    assert!(
        diags[0]
            .message
            .contains("requires numeric operands, found int and string"),
        "{}",
        diags[0].message
    );
}

#[test]
fn command_argument_feature_errors_name_the_command() {
    let source = r#"
command "customers" {
    argument customer: Customer;

    on run if customer.nope == "x" -> stay;
}
"#;
    let diags = diags(source, &shop_model());
    assert_eq!(diags.len(), 1, "{diags:?}");
    assert_eq!(diags[0].code, E_UNKNOWN_FEATURE);
    assert_eq!(
        diags[0].message,
        "command 'customers' event 'run': unknown feature 'nope' on class 'Customer'"
    );
}

#[test]
fn command_generic_types_and_prompts_are_skipped() {
    let source = r#"
command "onboard" {
    option tier: Tier = "gold";
    prompt confirm: confirm;

    on run if confirm == true -> action("Onboard", {
        tier: tier;
    });
}

action "Onboard" {
    target: Customer;
}
"#;
    assert!(
        diags(source, &shop_model()).is_empty(),
        "generic option types and prompt names are deferred, not errors"
    );
}

#[test]
fn command_option_defaults_must_suit_their_type() {
    let source = r#"
command "customers" {
    option limit: Int = "twenty";
}
"#;
    let reported = diags(source, &shop_model());
    assert_eq!(reported.len(), 1, "{reported:?}");
    assert_eq!(reported[0].code, E_TYPE_MISMATCH);
    assert!(
        reported[0]
            .message
            .contains("default of option 'limit' does not suit Int: '\"twenty\"'"),
        "{}",
        reported[0].message
    );

    let valid = source.replace("Int = \"twenty\"", "CustomerStatus = \"active\"");
    assert!(
        diags(&valid, &shop_model()).is_empty(),
        "custom/domain option types take string defaults (the resolver's rule)"
    );
}

#[test]
fn command_prompt_defaults_must_suit_their_kind() {
    let source = r#"
command "onboard" {
    prompt count: number = "lots";
}
"#;
    let reported = diags(source, &shop_model());
    assert_eq!(reported.len(), 1, "{reported:?}");
    assert_eq!(reported[0].code, E_TYPE_MISMATCH);
    assert!(
        reported[0]
            .message
            .contains("default of prompt 'count' does not suit number: '\"lots\"'"),
        "{}",
        reported[0].message
    );

    let valid = source.replace("number = \"lots\"", "text = \"Ada\"");
    assert!(diags(&valid, &shop_model()).is_empty());
}

#[test]
fn valid_filter_on_data_component_produces_no_diagnostics() {
    let source = r#"
view "Catalog" {
    component "orders" {
        type: list;
        data: Product;
        filter: status == "active";
    }
}
"#;
    assert!(diags(source, &shop_model()).is_empty());
}

#[test]
fn valid_override_navigation_produces_no_diagnostics() {
    let source = r#"
view "Catalog" {
    params { product: Product };

    use "Card" as card {
        title: product.name;
    };
}

module "Card" {
    input { title: String }
    output { total: Int }
}
"#;
    assert!(diags(source, &shop_model()).is_empty());
}

#[test]
fn valid_deep_override_navigation_produces_no_diagnostics() {
    // The grammar's `field_expr` is single-level (`a.b`); deeper chains
    // are representable in the IR (and in the domain), so the parsed
    // `product.name` override is deepened into `product.category.name`
    // by hand.
    let source = r#"
view "Catalog" {
    params { product: Product };

    use "Card" as card {
        title: product.name;
    };
}

module "Card" {
    input { title: String }
    output { total: Int }
}
"#;
    let mut compilation =
        crate::compile_ifml_str("app.ifml", source, &crate::IfmlImports::default())
            .expect("source must parse and resolve");
    let deep = |field: &str| ValueExpression::FieldAccess {
        object: Box::new(ValueExpression::FieldAccess {
            object: Box::new(ValueExpression::Identifier("product".to_string())),
            field: "category".to_string(),
        }),
        field: field.to_string(),
    };
    compilation.model.views[0].module_uses[0].properties[0].value = deep("name");
    assert!(check_ifml(&compilation, Some(&shop_model())).is_empty());

    // The same chain with an unknown final feature reports against
    // the deep receiver's class, with the override site context.
    compilation.model.views[0].module_uses[0].properties[0].value = deep("nope");
    let diags = check_ifml(&compilation, Some(&shop_model()));
    assert_eq!(diags.len(), 1, "{diags:?}");
    assert_eq!(diags[0].code, E_UNKNOWN_FEATURE);
    assert!(
        diags[0]
            .message
            .contains("unknown feature 'nope' on class 'Category'"),
        "{}",
        diags[0].message
    );
    assert!(
        diags[0]
            .message
            .contains("override 'title' of module 'Card'"),
        "{}",
        diags[0].message
    );
}

#[test]
fn unknown_feature_reports_site_context() {
    let source = r#"
view "Catalog" {
    params { product: Product };

    component "orders" {
        type: list;
        data: Product;
        filter: product.nope == "x";
    }
}
"#;
    let diags = diags(source, &shop_model());
    assert_eq!(diags.len(), 1, "{diags:?}");
    assert_eq!(diags[0].code, E_UNKNOWN_FEATURE);
    assert_eq!(diags[0].file, "app.ifml");
    assert_eq!(
        diags[0].message,
        "view 'Catalog' component 'orders' property 'filter': \
             unknown feature 'nope' on class 'Product'"
    );
}

#[test]
fn override_type_mismatch_reports_expected_found_and_source() {
    let source = r#"
view "Catalog" {
    params { product: Product };

    use "Card" as card {
        title: product.price;
    };
}

module "Card" {
    input { title: String }
    output { total: Int }
}
"#;
    let diags = diags(source, &shop_model());
    assert_eq!(diags.len(), 1, "{diags:?}");
    assert_eq!(diags[0].code, E_TYPE_MISMATCH);
    assert!(
        diags[0]
            .message
            .contains("override 'title' of module 'Card'"),
        "{}",
        diags[0].message
    );
    assert!(
        diags[0]
            .message
            .contains("expected string, found int in 'product.price'"),
        "{}",
        diags[0].message
    );
    let (start, end) = diags[0].span;
    assert!(source[start..end].starts_with("title: product.price"));
}

#[test]
fn unknown_data_entity_is_reported() {
    let source = r#"
view "Catalog" {
    component "orders" {
        type: list;
        data: Widget;
    }
}
"#;
    let diags = diags(source, &shop_model());
    assert_eq!(diags.len(), 1, "{diags:?}");
    assert_eq!(diags[0].code, E_UNKNOWN_ENTITY);
    assert_eq!(
        diags[0].message,
        "view 'Catalog' component 'orders': unknown data entity 'Widget'"
    );
    let (start, end) = diags[0].span;
    assert!(source[start..end].starts_with("data: Widget"));
}

#[test]
fn ambiguous_data_entity_is_reported() {
    let mut model = Model::new();
    let mut first = Package::new("shop.a");
    first.classes.push(ClassDef::new("Product", vec![], vec![]));
    let mut second = Package::new("shop.b");
    second
        .classes
        .push(ClassDef::new("Product", vec![], vec![]));
    model.packages.push(first);
    model.packages.push(second);

    let source = r#"
view "Catalog" {
    component "orders" {
        type: list;
        data: Product;
    }
}
"#;
    let diags = diags(source, &model);
    assert_eq!(diags.len(), 1, "{diags:?}");
    assert_eq!(diags[0].code, E_UNKNOWN_ENTITY);
    assert!(
        diags[0].message.contains("ambiguous data entity 'Product'"),
        "{}",
        diags[0].message
    );
}

#[test]
fn table_column_expressions_are_checked() {
    let valid = r#"
view "Report" {
    component "rows" {
        type: table;
        data: Product;

        column "Twice" -> expr price * 2;
    }
}
"#;
    assert!(diags(valid, &shop_model()).is_empty());

    let invalid = valid.replace("price * 2", "price.nope");
    let diags = diags(&invalid, &shop_model());
    assert_eq!(diags.len(), 1, "{diags:?}");
    assert_eq!(diags[0].code, E_UNKNOWN_FEATURE);
    assert!(
        diags[0].message.contains("column 'Twice'"),
        "{}",
        diags[0].message
    );
}

#[test]
fn event_guard_referencing_event_param_is_skipped() {
    let source = r#"
view "Catalog" {
    component "orders" {
        type: list;
        data: Product;

        on select(row) if row.status == "active" -> stay;
    }
}
"#;
    assert!(diags(source, &shop_model()).is_empty());
}

#[test]
fn event_guard_on_view_param_is_checked() {
    let source = r#"
view "Catalog" {
    params { product: Product };

    component "orders" {
        type: list;
        data: Product;

        on select(row) if product.status == "active" -> stay;
    }
}
"#;
    assert!(diags(source, &shop_model()).is_empty());

    let invalid = source.replace("product.status == \"active\"", "product.status == 3");
    let diags = diags(&invalid, &shop_model());
    assert_eq!(diags.len(), 1, "{diags:?}");
    assert_eq!(diags[0].code, E_TYPE_MISMATCH);
    assert!(
        diags[0].message.contains("event 'select'"),
        "{}",
        diags[0].message
    );
    assert!(
        diags[0].message.contains("cannot compare string and int"),
        "{}",
        diags[0].message
    );
}

#[test]
fn a_handler_may_call_a_package_function() {
    let source = r#"
view "Catalog" {
    component "orders" {
        type: list;
        data: Product;

        on select(row) if score(price) == true -> stay;
    }
}
"#;
    assert!(diags(source, &scored_shop_model()).is_empty());

    // The call is typed, not silently deferred: a wrong argument type
    // reports against the function signature.
    let invalid = source.replace("score(price)", "score(\"x\")");
    let diags = diags(&invalid, &scored_shop_model());
    assert_eq!(diags.len(), 1, "{diags:?}");
    assert!(
        diags[0]
            .message
            .contains("argument type mismatch in call to 'score'"),
        "{}",
        diags[0].message
    );
}

#[test]
fn an_unknown_function_in_a_handler_is_a_checker_diagnostic() {
    let source = r#"
view "Catalog" {
    component "orders" {
        type: list;
        data: Product;

        on select(row) if today() == date("2026-01-01") -> stay;
    }
}
"#;
    let diags = diags(source, &scored_shop_model());
    assert_eq!(diags.len(), 1, "{diags:?}");
    assert_eq!(diags[0].code, E_UNKNOWN_FEATURE);
    assert!(
        diags[0].message.contains("unknown function 'today'"),
        "{}",
        diags[0].message
    );
}

#[test]
fn an_ambiguous_function_call_is_a_diagnostic_naming_both_packages() {
    let mut model = Model::new();
    for package_name in ["shop.a", "shop.b"] {
        let mut package = Package::new(package_name);
        package.functions.push(score_function());
        if package_name == "shop.a" {
            // The entity resolves uniquely; only the function name is
            // declared twice.
            package.classes.push(ClassDef::new(
                "Product",
                vec![],
                vec![attr("price", PrimitiveType::Int)],
            ));
        }
        model.packages.push(package);
    }

    let source = r#"
view "Catalog" {
    component "orders" {
        type: list;
        data: Product;
        filter: score(price) == true;
    }
}
"#;
    let diags = diags(source, &model);
    assert_eq!(diags.len(), 1, "{diags:?}");
    assert_eq!(diags[0].code, E_UNKNOWN_FEATURE);
    assert!(
        diags[0].message.contains("ambiguous function 'score'"),
        "{}",
        diags[0].message
    );
    assert!(
        diags[0].message.contains("shop.a") && diags[0].message.contains("shop.b"),
        "{}",
        diags[0].message
    );
}

#[test]
fn navigation_binding_pairs_are_checked() {
    let source = r#"
view "Detail" {
    params { product: Product };

    component "orders" {
        type: list;
        data: Order;

        on select(row) -> navigate("Editor", { total: product.price });
    }
}

view "Editor" {
    params { total: Int };
}
"#;
    assert!(diags(source, &shop_model()).is_empty());

    let invalid = source.replace("product.price", "product.nope");
    let diags = diags(&invalid, &shop_model());
    assert_eq!(diags.len(), 1, "{diags:?}");
    assert_eq!(diags[0].code, E_UNKNOWN_FEATURE);
    assert!(
        diags[0].message.contains("binding 'total'"),
        "{}",
        diags[0].message
    );
}

#[test]
fn bare_call_skips_only_poisoned_args_or_absent_domains() {
    let source = r#"
view "Report" {
    component "orders" {
        type: list;
        data: Product;
        filter: score(price % 2) == true;
    }
}
"#;
    // A poisoned argument (`%` has no rex-expr operator) poisons the
    // whole call: no diagnostic even though `score` is defined.
    assert!(diags(source, &scored_shop_model()).is_empty());

    let unknown = r#"
view "Report" {
    component "orders" {
        type: list;
        data: Product;
        filter: today() == "x";
    }
}
"#;
    // With domains present the undefined call is a diagnostic.
    let diags = diags(unknown, &scored_shop_model());
    assert_eq!(diags.len(), 1, "{diags:?}");
    assert!(
        diags[0].message.contains("unknown function 'today'"),
        "{}",
        diags[0].message
    );

    // With domains absent checking is disabled entirely.
    let compilation = crate::compile_ifml_str("app.ifml", unknown, &crate::IfmlImports::default())
        .expect("source must parse and resolve");
    assert!(check_ifml(&compilation, None).is_empty());
}

#[test]
fn datetime_params_check_as_calendar_dates() {
    // `DateTime` is a checking alias for the expression language's
    // `date` (no time-of-day reasoning): comparisons against date
    // literals type-check, mismatches report.
    let source = r#"
view "Report" {
    params { deadline: DateTime };

    if deadline > date("2026-01-01");
}
"#;
    assert!(diags(source, &shop_model()).is_empty());

    let mismatch = r#"
view "Report" {
    params { deadline: DateTime };

    if deadline > "x";
}
"#;
    assert!(
        !diags(mismatch, &shop_model()).is_empty(),
        "a date-vs-string comparison must be reported"
    );
}

#[test]
fn params_record_navigation_type_checks_the_documented_spelling() {
    // The docs-promoted spelling: a binding pair reads the *view's*
    // params through the `params` record (`params.customerId`), not
    // only the bare ident. Both spellings coexist; the record is
    // exactly the body's (view + container / module input) params.
    let source = r#"
view "Detail" {
    params { customerId: Uuid, slug: String };

    component "editor" {
        type: form;
        data: Customer;
        filter: slug != "";

        on save(values) -> navigate("Home", {
            customerId: params.customerId,
            incoming: params.slug
        });
    }
}

view "Home" {
    params { customerId: Uuid, incoming: String };
}
"#;
    let found = diags(source, &shop_model());
    assert!(found.is_empty(), "{found:?}");
}

#[test]
fn params_record_unknown_field_is_reported() {
    let source = r#"
view "Detail" {
    params { customerId: Uuid };

    component "editor" {
        type: form;
        data: Customer;
        filter: params.bogus == "x";
    }
}
"#;
    let diags = diags(source, &shop_model());
    assert_eq!(diags.len(), 1, "{diags:?}");
    assert_eq!(diags[0].code, E_UNKNOWN_FEATURE);
    assert!(
        diags[0].message.contains("unknown feature 'bogus'"),
        "{}",
        diags[0].message
    );
}

#[test]
fn params_record_defers_untyped_param_fields() {
    // A param whose type does not resolve (a sigil/synthetic name) is
    // untyped: `params.<that-field>` defers like the bare ident does —
    // never a false unknown-feature error.
    let source = r#"
view "Detail" {
    params { customerId: Uuid, meta: Mystery };

    component "editor" {
        type: form;
        data: Customer;
        filter: params.meta == "x";
    }
}
"#;
    assert!(diags(source, &shop_model()).is_empty());
}

#[test]
fn a_param_named_params_keeps_bare_binding_precedence() {
    // A view param literally named `params` shadows the record: the
    // bare binding wins, the record is not registered.
    let source = r#"
view "Detail" {
    params { params: String };

    component "editor" {
        type: form;
        data: Customer;
        filter: params == "x";
    }
}
"#;
    assert!(diags(source, &shop_model()).is_empty());
}

#[test]
fn module_input_params_get_the_record() {
    let source = r#"
module "Pager" {
    input { page: Int, pageSize: Int }
    output { total: Int }

    component "pages" {
        type: list;
        data: Product;
        filter: page == params.page;
    }
}
"#;
    assert!(diags(source, &shop_model()).is_empty());
}

#[test]
fn container_params_join_the_view_record() {
    let source = r#"
view "Report" {
    params { region: String };

    container "Summary" {
        params { drill: String };

        component "revenue" {
            type: chart;
            data: Product;
            filter: region != drill;
        }
    }
}
"#;
    assert!(diags(source, &shop_model()).is_empty());

    let unknown = source.replace("filter: region != drill;", "filter: params.nope == \"x\";");
    let diags = diags(&unknown, &shop_model());
    assert_eq!(diags.len(), 1, "{diags:?}");
    assert_eq!(diags[0].code, E_UNKNOWN_FEATURE);
}

#[test]
fn canonical_fixture_checks_clean_against_its_reference_domain() {
    // The conformance golden only pins parse + serialize; this closes
    // the loop the docs promise: `check_ifml` over `app.ifml` with the
    // domain it was written against (`domain "sales"`) is clean —
    // including the doc-promoted `params.customerId` navigation
    // binding, the `params`-free examples' bare spellings, the bare
    // calls (`len`, `today`, `score`), the field navigation
    // (`Customer.name`, `Customer.status`), and the decision-dimension
    // `data: AssessCustomer` binding (the service table the fixture's
    // `rexlang/decisions.dmn` import declares).
    let source = include_str!("../../../../../tests/conformance/ifml/app.ifml");
    let auth = include_str!("../../../../../tests/conformance/ifml/rexlang/auth.actor");
    let mut imports = crate::IfmlImports::default();
    imports.provide("app.ifml", "rexlang/auth.actor", auth.to_string());
    let compilation = crate::compile_ifml_str("app.ifml", source, &imports)
        .expect("the canonical fixture must compile");
    let diags = check_ifml_with_decisions(
        &compilation,
        Some(&sales_reference_model()),
        &[],
        &[fixture_view_decisions()],
    );
    assert!(diags.is_empty(), "{}", {
        let rendered = diags
            .iter()
            .map(|diagnostic| {
                format!(
                    "  {} at {:?}: {}",
                    diagnostic.code, diagnostic.span, diagnostic.message
                )
            })
            .collect::<Vec<_>>()
            .join("\n");
        format!("the canonical fixture must check clean:\n{rendered}")
    });
}

/// The decision model the fixture's `rexlang/decisions.dmn` import
/// declares: the `AssessCustomer` service exposing the `CustomerTier`
/// decision's outputs — the shape the fixture's `data: AssessCustomer`
/// resolves against.
fn fixture_view_decisions() -> DmnModel {
    let mut model = DmnModel::new();
    model.name = Some("ViewDecisions".to_string());
    model.decisions.push(rex_ir::dmn::Decision {
        name: "CustomerTier".to_string(),
        description: None,
        outputs: vec![
            rex_ir::dmn::Output {
                name: "tier".to_string(),
                type_ref: TypeRef::Primitive(PrimitiveType::String),
            },
            rex_ir::dmn::Output {
                name: "discount_cents".to_string(),
                type_ref: TypeRef::Primitive(PrimitiveType::Int),
            },
        ],
        logic: None,
        emits: Vec::new(),
    });
    model.services.push(rex_ir::dmn::DecisionService {
        name: "AssessCustomer".to_string(),
        description: None,
        inputs: vec!["customer".to_string()],
        outputs: vec!["CustomerTier".to_string()],
        encapsulates: Vec::new(),
    });
    model
}

/// The domain the canonical fixture was written against: the classes
/// and free functions its expressions reference (`domain "sales"`).
fn sales_reference_model() -> Model {
    let mut model = Model::new();
    let mut package = Package::new("sales");
    package.classes.push(ClassDef::new(
        "Customer",
        vec![],
        vec![
            attr("name", PrimitiveType::String),
            attr("email", PrimitiveType::String),
            attr("status", PrimitiveType::String),
            attr("tier", PrimitiveType::String),
            attr("title", PrimitiveType::String),
            attr("active", PrimitiveType::Boolean),
            attr("id", PrimitiveType::String),
            attr("history", PrimitiveType::Int),
        ],
    ));
    package.classes.push(ClassDef::new(
        "Item",
        vec![],
        vec![attr("name", PrimitiveType::String)],
    ));
    package.classes.push(ClassDef::new(
        "Order",
        vec![],
        vec![
            attr("id", PrimitiveType::String),
            attr("total", PrimitiveType::Int),
            attr("date", PrimitiveType::Date),
        ],
    ));
    package
        .classes
        .push(ClassDef::new("Person", vec![], vec![]));
    package.functions.push(Operation::new(
        "len",
        TypeRef::Primitive(PrimitiveType::Int),
        vec![OperationParam {
            name: "value".to_string(),
            type_: TypeRef::Primitive(PrimitiveType::String),
            multiplicity: None,
        }],
    ));
    package.functions.push(Operation::new(
        "today",
        TypeRef::Primitive(PrimitiveType::Date),
        vec![],
    ));
    package.functions.push(Operation::new(
        "score",
        TypeRef::Primitive(PrimitiveType::Int),
        vec![OperationParam {
            name: "history".to_string(),
            type_: TypeRef::Primitive(PrimitiveType::Int),
            multiplicity: None,
        }],
    ));
    model.packages.push(package);
    model
}

#[test]
fn filter_literal_type_mismatch_is_reported() {
    let source = r#"
view "Catalog" {
    component "orders" {
        type: list;
        data: Product;
        filter: price == "expensive";
    }
}
"#;
    let diags = diags(source, &shop_model());
    assert_eq!(diags.len(), 1, "{diags:?}");
    assert_eq!(diags[0].code, E_TYPE_MISMATCH);
    assert!(
        diags[0].message.contains("cannot compare int and string"),
        "{}",
        diags[0].message
    );
}

#[test]
fn no_domains_disables_all_checking() {
    let source = r#"
view "Catalog" {
    params { product: Product };

    component "orders" {
        type: list;
        data: Product;
        filter: product.nope == "x";
    }
}
"#;
    let compilation = crate::compile_ifml_str("app.ifml", source, &crate::IfmlImports::default())
        .expect("source must parse and resolve");
    assert!(check_ifml(&compilation, None).is_empty());
}

#[test]
fn action_target_must_resolve_to_a_declared_entity() {
    let source = r#"
action "ListWidgets" {
    target: Widget;
}
"#;
    let diags = diags(source, &shop_model());
    assert_eq!(diags.len(), 1, "{diags:?}");
    assert_eq!(diags[0].code, E_UNKNOWN_ACTION_TARGET);
    assert_eq!(
        diags[0].message,
        "action 'ListWidgets': unknown target entity 'Widget'"
    );
    let (start, end) = diags[0].span;
    assert!(source[start..end].starts_with("target: Widget"));
}

#[test]
fn action_target_resolves_against_the_domain() {
    let source = r#"
action "ListProducts" {
    target: Product;
}
"#;
    assert!(diags(source, &shop_model()).is_empty());
}

#[test]
fn ambiguous_action_target_is_reported() {
    let mut model = Model::new();
    let mut first = Package::new("shop.a");
    first.classes.push(ClassDef::new("Product", vec![], vec![]));
    let mut second = Package::new("shop.b");
    second
        .classes
        .push(ClassDef::new("Product", vec![], vec![]));
    model.packages.push(first);
    model.packages.push(second);

    let source = r#"
action "ListProducts" {
    target: Product;
}
"#;
    let diags = diags(source, &model);
    assert_eq!(diags.len(), 1, "{diags:?}");
    assert_eq!(diags[0].code, E_UNKNOWN_ACTION_TARGET);
    assert!(
        diags[0]
            .message
            .contains("ambiguous target entity 'Product'"),
        "{}",
        diags[0].message
    );
}

#[test]
fn action_targets_are_skipped_without_domains() {
    let source = r#"
action "ListWidgets" {
    target: Widget;
}
"#;
    let compilation = crate::compile_ifml_str("app.ifml", source, &crate::IfmlImports::default())
        .expect("source must parse and resolve");
    let diags = check_ifml(&compilation, None);
    assert!(
        diags
            .iter()
            .all(|diagnostic| diagnostic.code != E_UNKNOWN_ACTION_TARGET),
        "without domains the target check is off: {diags:?}"
    );
}

#[test]
fn action_declaration_handlers_are_checked() {
    let source = r#"
action "Archive" {
    on save(values) if nonsense -> stay;
}
"#;
    let diags = diags(source, &shop_model());
    assert_eq!(diags.len(), 1, "{diags:?}");
    assert!(
        diags[0].message.contains("action 'Archive'"),
        "{}",
        diags[0].message
    );
    assert!(
        diags[0].message.contains("unknown name 'nonsense'"),
        "{}",
        diags[0].message
    );
}

#[test]
fn lowering_splits_integral_and_fractional_numbers() {
    assert_eq!(
        lower(&Expression::NumLit(1.5.into())).map(|expr| expr.kind),
        Some(ExprKind::Float(1.5))
    );
    assert_eq!(
        lower(&Expression::NumLit(2.0.into())).map(|expr| expr.kind),
        Some(ExprKind::Int(2))
    );
    let modulo = Expression::BinOp {
        left: Box::new(Expression::Ident("a".to_string())),
        op: IfmlBinOp::Mod,
        right: Box::new(Expression::Ident("b".to_string())),
    };
    assert!(lower(&modulo).is_none());
    // One unmappable leaf poisons the whole operator tree.
    let wrapped = Expression::Group(Box::new(Expression::BinOp {
        left: Box::new(Expression::Ident("a".to_string())),
        op: IfmlBinOp::Eq,
        right: Box::new(modulo.clone()),
    }));
    assert!(lower(&wrapped).is_none());
    // A poisoned argument poisons the whole call.
    assert!(lower(&Expression::Call {
        name: "score".to_string(),
        args: vec![modulo],
    })
    .is_none());
    // Every other bare call lowers as a package-function call.
    assert_eq!(
        lower(&Expression::Call {
            name: "today".to_string(),
            args: vec![],
        })
        .map(|expr| expr.kind),
        Some(ExprKind::FunctionCall {
            name: Spanned {
                value: "today".to_string(),
                span: no_span(),
            },
            args: vec![],
        })
    );
    assert_eq!(
        lower_value(&ValueExpression::Call("today".to_string(), vec![])).map(|expr| expr.kind),
        Some(ExprKind::FunctionCall {
            name: Spanned {
                value: "today".to_string(),
                span: no_span(),
            },
            args: vec![],
        })
    );
    assert!(lower_value(&ValueExpression::Array(vec![ValueExpression::Bool(true)])).is_none());
}

#[test]
fn date_literal_stays_the_date_constructor() {
    let lowered = lower(&Expression::Call {
        name: "date".to_string(),
        args: vec![Expression::StringLit("2026-01-01".to_string())],
    })
    .expect("the date constructor maps");
    assert_eq!(
        lowered.kind,
        ExprKind::Date {
            text: "2026-01-01".to_string(),
            literal_span: no_span(),
        }
    );
    // The constructor special-case needs exactly one string literal;
    // any other argument shape falls through to the generic bare-call
    // mapping.
    let generic = lower(&Expression::Call {
        name: "date".to_string(),
        args: vec![],
    })
    .expect("a non-constructor date call still lowers");
    assert!(matches!(generic.kind, ExprKind::FunctionCall { .. }));
}
