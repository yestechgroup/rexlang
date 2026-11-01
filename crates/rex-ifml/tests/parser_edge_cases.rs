use rex_ifml::*;

#[test]
fn test_invalid_syntax() {
    let input = "view Broken { this is not valid }";
    let result = parse_ifml(input);
    assert!(result.is_err());
}

#[test]
fn test_invalid_missing_semicolon() {
    let input = r#"
view "Broken" {
    component "c" {
        type: list
    }
}
"#;
    let result = parse_ifml(input);
    assert!(result.is_err());
}

#[test]
fn test_full_docs_example() {
    let input = r#"
domain "sales" {
    schema "sales";
}

view "CustomerList" {
    label "Customer Management";
    landmark: true;
    position: { x: 100; y: 200 };

    component "grid" {
        type: list;
        data: Customer;
        fields: [name, email, phone, status];
        sortable: true;
        paginated: true;

        on select(row) -> navigate("CustomerDetail", {
            customerId: row.id
        });
    }

    component "searchBar" {
        type: form;
        data: CustomerSearchCriteria;

        on submit(values) -> refresh("grid", {
            query: searchBar.query
        });
    }
}

view "CustomerDetail" {
    params { customerId: Uuid };

    component "info" {
        type: details;
        data: Customer;
        fields: [name, email, phone, status, createdAt];

        on edit -> navigate("CustomerEdit", {
            customerId: params.customerId
        });
    }
}

view "CustomerEdit" {
    params { customerId: Uuid };

    component "form" {
        type: form;
        data: Customer;
        mode: edit;

        on save(values) -> action("UpdateCustomer", {
            on success -> navigate("CustomerDetail", {
                customerId: params.customerId
            });
            on error -> stay;
        });

        on cancel -> navigate("CustomerDetail");
    }
}

view "WizardPage" {
    xor: true;

    container "Step1" {
        default: true;

        component "personalInfo" {
            type: form;
            data: Customer;

            on submit(values) -> navigate("Step2", {
                name: values.name,
                email: values.email
            });
        }
    }

    container "Step2" {
        component "addressInfo" {
            type: form;
            data: Address;

            on submit(values) -> navigate("Step3", {
                street: values.street,
                city: values.city
            });
        }
    }
}

view "Dashboard" {
    on load -> refresh("recentOrders");

    component "recentOrders" {
        type: list;
        data: Order;
        fields: [id, customerName, total, date];
        filter: date == today();
    }
}

view "DeleteConfirm" {
    modal: true;

    component "confirmForm" {
        type: form;
        data: ConfirmDelete;

        on submit(values) -> action("DeleteProduct", {
            on success -> navigate("ProductList");
            on error -> stay;
        });

        on cancel -> navigate("ProductList");
    }
}
"#;
    let model = parse_ifml(input).unwrap();
    assert_eq!(model.domains.len(), 1);
    assert_eq!(model.domains[0].name, "sales");
    assert_eq!(model.views.len(), 6);
    assert!(model.actions.is_empty());
    assert!(model.modules.is_empty());

    let cl = &model.views[0];
    assert_eq!(cl.name, "CustomerList");
    assert!(cl.is_landmark);
    assert_eq!(cl.components.len(), 2);
    assert_eq!(cl.containers.len(), 0);
    assert_eq!(
        cl.position,
        Some(Position {
            x: 100.0.into(),
            y: 200.0.into()
        })
    );

    let cd = &model.views[1];
    assert_eq!(cd.name, "CustomerDetail");
    assert_eq!(cd.params.len(), 1);
    assert_eq!(cd.params[0].name, "customerId");

    let db = &model.views[4];
    assert_eq!(db.name, "Dashboard");
    assert_eq!(db.events.len(), 1);

    let dc = &model.views[5];
    assert_eq!(dc.name, "DeleteConfirm");
    assert!(dc.is_modal);
}

#[test]
fn test_string_escape() {
    let input = r#"
view "Esc" {
    component "c" {
        type: list;
        data: Item;
        label: "hello\nworld";
    }
}
"#;
    let model = parse_ifml(input).unwrap();
    let comp = &model.views[0].components[0];
    let label_prop = comp.properties.iter().find(|p| p.key == "label").unwrap();
    match &label_prop.value {
        ValueExpression::String(s) => {
            assert_eq!(s, "hello\nworld");
        }
        _ => panic!("Expected String"),
    }
}

#[test]
fn test_comment_is_ignored() {
    let input = r#"
// This is a comment
view "Commented" {
    // inline comment
    component "c" {
        type: list;
        data: X;
        // another comment
    }
}
"#;
    let model = parse_ifml(input).unwrap();
    assert_eq!(model.views.len(), 1);
    assert_eq!(model.views[0].name, "Commented");
}

#[test]
fn test_empty_model() {
    let input = "";
    let model = parse_ifml(input).unwrap();
    assert_eq!(model.views.len(), 0);
    assert_eq!(model.domains.len(), 0);
}

#[test]
fn test_nested_object_rejected() {
    let input = r#"
view "Nested" {
    position: { x: 10; y: 20; nested: { a: 1 } };
    component "c" {
        type: list;
        data: Item;
    }
}
"#;
    let result = parse_ifml(input);
    assert!(result.is_err(), "nested object literal should be rejected");
}

#[test]
fn test_property_span_recorded() {
    let input = r#"
view "Span" {
    position: { x: 1; y: 2 };
}
"#;
    let model = parse_ifml(input).unwrap();
    let prop = &model.views[0].properties[0];
    let span = prop.span.expect("span should be recorded");
    assert_eq!(span.1 - span.0, "position: { x: 1; y: 2 };".len());
    assert_eq!(&input[span.0..span.1], "position: { x: 1; y: 2 };");
}
