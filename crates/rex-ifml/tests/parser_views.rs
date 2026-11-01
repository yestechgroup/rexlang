use rex_ifml::*;

#[test]
fn test_minimal_view() {
    let input = r#"
view "Minimal" {
    component "grid" {
        type: list;
        data: Customer;
        fields: [name, email];
    }
}
"#;
    let model = parse_ifml(input).unwrap();
    assert_eq!(model.views.len(), 1);
    assert_eq!(model.views[0].name, "Minimal");
    assert_eq!(model.views[0].components.len(), 1);
    assert_eq!(model.views[0].components[0].name, "grid");
}

#[test]
fn test_view_with_params() {
    let input = r#"
view "Detail" {
    params { id: Uuid, mode: String };
    component "info" {
        type: details;
        data: Customer;
    }
}
"#;
    let model = parse_ifml(input).unwrap();
    assert_eq!(model.views.len(), 1);
    let view = &model.views[0];
    assert_eq!(view.params.len(), 2);
    assert_eq!(view.params[0].name, "id");
    assert_eq!(view.params[0].type_ref, "Uuid");
    assert_eq!(view.params[1].name, "mode");
    assert_eq!(view.params[1].type_ref, "String");
}

#[test]
fn test_container_with_navigation() {
    let input = r#"
view "Wizard" {
    xor: true;

    container "Step1" {
        default: true;

        component "form" {
            type: form;
            data: Customer;

            on submit(values) -> navigate("Step2", {
                name: values.name
            });
        }
    }
}
"#;
    let model = parse_ifml(input).unwrap();
    assert_eq!(model.views.len(), 1);
    let view = &model.views[0];
    assert!(view.is_xor);
    assert_eq!(view.containers.len(), 1);
    let container = &view.containers[0];
    assert_eq!(container.name, "Step1");
    assert!(container.is_default);
    assert_eq!(container.components.len(), 1);
    assert_eq!(container.components[0].name, "form");
    assert_eq!(container.components[0].events.len(), 1);

    let event = &container.components[0].events[0];
    assert_eq!(event.event_type, EventType::Submit);
    assert_eq!(event.params, vec!["values"]);
    match &event.action {
        EventAction::Navigate { target, binding } => {
            assert_eq!(target, "Step2");
            assert!(binding.is_some());
            assert_eq!(binding.as_ref().unwrap().pairs[0].0, "name");
        }
        _ => panic!("Expected Navigate action"),
    }
}

#[test]
fn test_modal_view() {
    let input = r#"
view "DeleteConfirm" {
    modal: true;

    component "form" {
        type: form;
        data: ConfirmDelete;

        on cancel -> navigate("ProductList");
    }
}
"#;
    let model = parse_ifml(input).unwrap();
    let view = &model.views[0];
    assert!(view.is_modal);
    assert!(!view.is_landmark);
    assert!(!view.is_xor);
}

#[test]
fn test_complex_view_with_label() {
    let input = r#"
view "Dashboard" {
    label "Analytics Dashboard";
    landmark: true;

    on load -> refresh("recentOrders");
    on load -> refresh("topProducts");

    component "recentOrders" {
        type: list;
        data: Order;
        fields: [id, customerName, total, date];
        filter: date == today();
    }
}
"#;
    let model = parse_ifml(input).unwrap();
    let view = &model.views[0];
    assert!(view.is_landmark);
    assert_eq!(view.label.as_deref(), Some("Analytics Dashboard"));
    assert_eq!(view.events.len(), 2);
    assert_eq!(view.components.len(), 1);
}

#[test]
fn test_search_page() {
    let input = r#"
view "SearchPage" {
    component "searchForm" {
        type: form;
        data: SearchQuery;

        on submit(values) -> navigate("SearchPage", {
            query: values.term
        });
    }

    component "results" {
        type: list;
        data: Product;
        fields: [name, sku, price];
        filter: name ~= params.query;

        on select(row) -> navigate("ProductDetail", {
            productId: row.id
        });
    }
}
"#;
    let model = parse_ifml(input).unwrap();
    let view = &model.views[0];
    assert_eq!(view.components.len(), 2);

    let form = &view.components[0];
    let submit_event = &form.events[0];
    assert_eq!(submit_event.params, vec!["values"]);
    match &submit_event.action {
        EventAction::Navigate { target, binding } => {
            assert_eq!(target, "SearchPage");
            assert!(binding.is_some());
            assert_eq!(binding.as_ref().unwrap().pairs[0].0, "query");
        }
        _ => panic!("Expected Navigate"),
    }

    let results = &view.components[1];
    let select_event = &results.events[0];
    assert_eq!(select_event.params, vec!["row"]);
    match &select_event.action {
        EventAction::Navigate { target, binding } => {
            assert_eq!(target, "ProductDetail");
            assert!(binding.is_some());
            assert_eq!(binding.as_ref().unwrap().pairs[0].0, "productId");
        }
        _ => panic!("Expected Navigate"),
    }
}

#[test]
fn test_position_parses_float_and_negative() {
    let input = r#"
view "Pos" {
    position: { x: -12.5; y: 240 };
    component "c" {
        type: list;
        data: Item;
    }
}

view "Pos2" {
    container "c1" {
        position: { x: 30.75; y: -5 };
    }
}
"#;
    let model = parse_ifml(input).unwrap();
    assert_eq!(model.views.len(), 2);

    let view = &model.views[0];
    assert_eq!(
        view.position,
        Some(Position {
            x: (-12.5).into(),
            y: 240.0.into()
        })
    );
    assert_eq!(view.properties.len(), 1);
    assert_eq!(view.properties[0].key, "position");

    let container = &model.views[1].containers[0];
    assert_eq!(
        container.position,
        Some(Position {
            x: 30.75.into(),
            y: (-5.0).into()
        })
    );
}

#[test]
fn test_position_missing_is_none() {
    let input = r#"
view "NoPos" {
    landmark: true;
    component "c" {
        type: list;
        data: Item;
    }
}
"#;
    let model = parse_ifml(input).unwrap();
    assert_eq!(model.views[0].position, None);
}

#[test]
fn test_position_trailing_inner_semicolon() {
    let input = r#"
view "Pos" {
    position: { x: 10; y: 20; };
    component "c" {
        type: list;
        data: Item;
    }
}
"#;
    let model = parse_ifml(input).unwrap();
    assert_eq!(
        model.views[0].position,
        Some(Position {
            x: 10.0.into(),
            y: 20.0.into()
        })
    );
}

#[test]
fn test_position_incomplete_is_none() {
    let input = r#"
view "Partial" {
    position: { x: 10 };
    component "c" {
        type: list;
        data: Item;
    }
}
"#;
    let model = parse_ifml(input).unwrap();
    assert_eq!(model.views[0].position, None);
}

#[test]
fn test_container_label_xor_and_nesting() {
    let input = r#"
view "Checkout" {
    container "Shipping" {
        label "Shipping";
        xor: true;

        container "Address" {
            label "Address";
            default: true;

            component "form" {
                type: form;
                data: Customer;
            }
        }
    }
}
"#;
    let model = parse_ifml(input).unwrap();
    let shipping = &model.views[0].containers[0];
    assert_eq!(shipping.label.as_deref(), Some("Shipping"));
    assert!(shipping.is_xor);
    assert!(!shipping.is_default);
    assert_eq!(shipping.containers.len(), 1);
    let address = &shipping.containers[0];
    assert_eq!(address.name, "Address");
    assert_eq!(address.label.as_deref(), Some("Address"));
    assert!(address.is_default);
    assert!(!address.is_xor);
    assert_eq!(address.components.len(), 1);
}

#[test]
fn test_container_without_label_defaults() {
    let input = r#"
view "V" {
    container "Plain" {
        component "grid" {
            type: list;
            data: Customer;
        }
    }
}
"#;
    let model = parse_ifml(input).unwrap();
    let container = &model.views[0].containers[0];
    assert_eq!(container.label, None);
    assert!(!container.is_xor);
    assert!(!container.is_default);
    assert!(container.containers.is_empty());
}

#[test]
fn test_param_defaults_parse() {
    let input = r#"
view "Catalog" {
    params { id: Uuid, slug: String = "home", page: Int = 1, verbose: Boolean = true };

    component "grid" {
        type: list;
        data: Product;
    }
}

module "Pager" {
    input { offset: Int = 0 }
    output { rows: String }
}
"#;
    let model = parse_ifml(input).unwrap();
    let params = &model.views[0].params;
    assert_eq!(params.len(), 4);

    assert_eq!(params[0].name, "id");
    assert_eq!(params[0].default, None);

    assert_eq!(params[1].name, "slug");
    assert_eq!(
        params[1].default,
        Some(ValueExpression::String("home".to_string()))
    );

    assert_eq!(params[2].name, "page");
    assert_eq!(params[2].default, Some(ValueExpression::Number(1.0.into())));

    assert_eq!(params[3].name, "verbose");
    assert_eq!(params[3].default, Some(ValueExpression::Bool(true)));

    assert_eq!(
        model.modules[0].input_params[0].default,
        Some(ValueExpression::Number(0.0.into()))
    );
}

#[test]
fn test_param_negative_number_default() {
    let input = r#"
view "V" {
    params { zoom: Int = -2 };
    component "c" {
        type: list;
        data: Item;
    }
}
"#;
    let model = parse_ifml(input).unwrap();
    assert_eq!(
        model.views[0].params[0].default,
        Some(ValueExpression::Number((-2.0).into()))
    );
}

#[test]
fn test_param_default_requires_literal() {
    let input = r#"
view "V" {
    params { slug: String = slugify(title) };
    component "c" {
        type: list;
        data: Item;
    }
}
"#;
    assert!(parse_ifml(input).is_err());
}

#[test]
fn test_view_roles_extracted_from_property_bag() {
    let input = r#"
view "AdminConsole" {
    roles: [admin, manager];
    landmark: true;

    component "grid" {
        type: list;
        data: Item;
    }
}
"#;
    let model = parse_ifml(input).unwrap();
    let view = &model.views[0];
    assert_eq!(view.roles, vec!["admin".to_string(), "manager".to_string()]);
    assert!(view.is_landmark);
    assert!(
        !view.properties.iter().any(|p| p.key == "roles"),
        "roles must be removed from the property bag"
    );
}

#[test]
fn test_view_roles_empty_array() {
    let input = r#"
view "Open" {
    roles: [];

    component "grid" {
        type: list;
        data: Item;
    }
}
"#;
    let model = parse_ifml(input).unwrap();
    assert!(model.views[0].roles.is_empty());
    assert!(
        !model.views[0].properties.iter().any(|p| p.key == "roles"),
        "empty roles must also be removed from the property bag"
    );
}

#[test]
fn test_view_without_roles_parses_identically() {
    let input = r#"
view "Catalog" {
    label "Catalog";
    landmark: true;

    component "grid" {
        type: list;
        data: Product;
        fields: [name, price];
    }

    on load -> stay;
}
"#;
    let model = parse_ifml(input).unwrap();
    let view = &model.views[0];
    assert!(view.roles.is_empty());
    assert_eq!(view.label.as_deref(), Some("Catalog"));
    assert!(view.is_landmark);
    assert_eq!(view.components.len(), 1);
    assert_eq!(view.events.len(), 1);
}

#[test]
fn test_view_requires_extracted_from_property_bag() {
    let input = r#"
view "RefundQueue" {
    requires: [RaiseRefund, ApproveRefund];
    landmark: true;

    component "grid" {
        type: list;
        data: Refund;
    }
}
"#;
    let model = parse_ifml(input).unwrap();
    let view = &model.views[0];
    assert_eq!(
        view.requires,
        vec!["RaiseRefund".to_string(), "ApproveRefund".to_string()]
    );
    assert!(view.is_landmark);
    assert!(
        !view.properties.iter().any(|p| p.key == "requires"),
        "requires must be removed from the property bag"
    );
}

#[test]
fn test_view_requires_empty_array() {
    let input = r#"
view "Open" {
    requires: [];

    component "grid" {
        type: list;
        data: Item;
    }
}
"#;
    let model = parse_ifml(input).unwrap();
    assert!(model.views[0].requires.is_empty());
    assert!(
        !model.views[0]
            .properties
            .iter()
            .any(|p| p.key == "requires"),
        "empty requires must also be removed from the property bag"
    );
}

#[test]
fn test_view_requires_and_roles_coexist() {
    let input = r#"
view "Approvals" {
    roles: [admin, manager];
    requires: [ApproveRefund];

    component "grid" {
        type: list;
        data: Refund;
    }
}
"#;
    let model = parse_ifml(input).unwrap();
    let view = &model.views[0];
    assert_eq!(view.roles, vec!["admin".to_string(), "manager".to_string()]);
    assert_eq!(view.requires, vec!["ApproveRefund".to_string()]);
    assert!(!view.properties.iter().any(|p| p.key == "roles"));
    assert!(!view.properties.iter().any(|p| p.key == "requires"));
}

#[test]
fn test_view_without_requires_parses_identically() {
    let input = r#"
view "Catalog" {
    label "Catalog";
    landmark: true;

    component "grid" {
        type: list;
        data: Product;
        fields: [name, price];
    }

    on load -> stay;
}
"#;
    let model = parse_ifml(input).unwrap();
    let view = &model.views[0];
    assert!(view.requires.is_empty());
    assert!(view.roles.is_empty());
    assert_eq!(view.label.as_deref(), Some("Catalog"));
    assert!(view.is_landmark);
    assert_eq!(view.components.len(), 1);
    assert_eq!(view.events.len(), 1);
}
