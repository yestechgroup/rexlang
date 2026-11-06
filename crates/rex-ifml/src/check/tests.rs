use super::*;

use rex_ir::{
    ActorDef, ActorModel, ActorsDef, CapabilityDef, ClassDef, Feature, FeatureKind, Multiplicity,
    Operation, OperationParam, Package, PrimitiveType, TypeRef,
};

const PKG: &str = "nz.example.shop";

fn class_ref(name: &str) -> TypeRef {
    TypeRef::Class {
        package: PKG.to_string(),
        name: name.to_string(),
    }
}

fn attr(name: &str, primitive: PrimitiveType) -> Feature {
    Feature::new(
        name,
        FeatureKind::Attribute,
        TypeRef::Primitive(primitive),
        Multiplicity::REQUIRED,
    )
}

/// A commerce-ish domain: Product → Category, Order → Customer.
fn shop_model() -> Model {
    let mut model = Model::new();
    let mut package = Package::new(PKG);
    package.classes.push(ClassDef::new(
        "Product",
        vec![],
        vec![
            attr("name", PrimitiveType::String),
            attr("price", PrimitiveType::Int),
            attr("status", PrimitiveType::String),
            Feature::new(
                "category",
                FeatureKind::CrossReference,
                class_ref("Category"),
                Multiplicity::REQUIRED,
            ),
        ],
    ));
    package.classes.push(ClassDef::new(
        "Category",
        vec![],
        vec![attr("name", PrimitiveType::String)],
    ));
    package.classes.push(ClassDef::new(
        "Order",
        vec![],
        vec![
            attr("total", PrimitiveType::Int),
            Feature::new(
                "customer",
                FeatureKind::CrossReference,
                class_ref("Customer"),
                Multiplicity::REQUIRED,
            ),
        ],
    ));
    package.classes.push(ClassDef::new(
        "Customer",
        vec![],
        vec![attr("name", PrimitiveType::String)],
    ));
    model.packages.push(package);
    model
}

/// A `score(int) -> boolean` package function — the shape a lowered
/// sigil import contributes to `Package::functions`.
fn score_function() -> Operation {
    Operation::new(
        "score",
        TypeRef::Primitive(PrimitiveType::Boolean),
        vec![OperationParam {
            name: "value".to_string(),
            type_: TypeRef::Primitive(PrimitiveType::Int),
            multiplicity: None,
        }],
    )
}

/// The shop domain plus the `score(int) -> boolean` package function.
fn scored_shop_model() -> Model {
    let mut model = shop_model();
    model.packages[0].functions.push(score_function());
    model
}

fn diags(source: &str, model: &Model) -> Vec<IfmlDiagnostic> {
    let compilation = crate::compile_ifml_str("app.ifml", source, &crate::IfmlImports::default())
        .expect("source must parse and resolve");
    check_ifml(&compilation, Some(model))
}

/// An actors layer declaring what app-style flows require: two
/// capabilities on `Customer` and two roles.
fn auth_model() -> ActorModel {
    ActorModel::new().block(
        ActorsDef::new("Auth")
            .actor(ActorDef::new("admin"))
            .actor(ActorDef::new("manager"))
            .capability(CapabilityDef::new("ManageCustomer", class_ref("Customer")))
            .capability(CapabilityDef::new("ApproveCustomer", class_ref("Customer"))),
    )
}

fn diags_with_actors(source: &str, model: &Model, actors: &[ActorModel]) -> Vec<IfmlDiagnostic> {
    let compilation = crate::compile_ifml_str("app.ifml", source, &crate::IfmlImports::default())
        .expect("source must parse and resolve");
    check_ifml_with_actors(&compilation, Some(model), actors)
}

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
fn empty_actor_models_skip_authorization_checking() {
    // The compatibility contract: without an actor dimension no
    // authorization diagnostic is ever produced.
    let source = r#"
view "Edit" {
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
    // calls (`len`, `today`, `score`), and the field navigation
    // (`Customer.name`, `Customer.status`).
    let source = include_str!("../../../../tests/conformance/ifml/app.ifml");
    let auth = include_str!("../../../../tests/conformance/ifml/rexlang/auth.actor");
    let mut imports = crate::IfmlImports::default();
    imports.provide("app.ifml", "rexlang/auth.actor", auth.to_string());
    let compilation = crate::compile_ifml_str("app.ifml", source, &imports)
        .expect("the canonical fixture must compile");
    let diags = check_ifml(&compilation, Some(&sales_reference_model()));
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
    assert!(check_ifml(&compilation, None).is_empty());
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

// --- instantiated-module references (issue #114, interaction layer) -

/// Compiles `source` as `app.ifml` with `imports` provided, then
/// checks against `model` (no actor layer).
fn diags_with_import(
    source: &str,
    imports: &crate::IfmlImports,
    model: &Model,
) -> Vec<IfmlDiagnostic> {
    let compilation = crate::compile_ifml_str("app.ifml", source, imports)
        .expect("source must parse and resolve");
    check_ifml(&compilation, Some(model))
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
    component "rows" { type: list; }
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
