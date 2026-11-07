use rex_ifml::*;

#[test]
fn test_component_api_property() {
    let input = r#"
view "CustomerList" {
    component "grid" {
        type: list;
        data: Customer;
        api: list_customer;
    }
}
"#;
    let model = parse_ifml(input).unwrap();
    assert_eq!(model.views.len(), 1);
    let comp = &model.views[0].components[0];
    let api_prop = comp
        .properties
        .iter()
        .find(|p| p.key == "api")
        .expect("api property should exist");
    match &api_prop.value {
        ValueExpression::Identifier(s) => assert_eq!(s, "list_customer"),
        _ => panic!("Expected Identifier for api property"),
    }
}

#[test]
fn test_array_fields() {
    let input = r#"
view "Fields" {
    component "c" {
        type: list;
        data: Item;
        fields: [name, price, createdAt];
        filter: status == "active" && price > 100.50;
    }
}
"#;
    let model = parse_ifml(input).unwrap();
    let comp = &model.views[0].components[0];
    let fields_prop = comp.properties.iter().find(|p| p.key == "fields").unwrap();
    match &fields_prop.value {
        ValueExpression::Array(items) => {
            assert_eq!(items.len(), 3);
        }
        _ => panic!("Expected Array"),
    }
}

#[test]
fn test_table_columns_field_lookup_expr() {
    let input = r#"
view "Customers" {
    component "CustomerTable" {
        type: table;
        data: Customer;

        column "Name"   -> field Customer.name;
        column "Status" -> lookup Customer.status via status_labels;
        column "Tenure" -> expr tenure_years(Customer.hire_date);

        on select(row) -> navigate("CustomerDetail", { customerId: row.id });
    }
}
"#;
    let model = parse_ifml(input).unwrap();
    let comp = &model.views[0].components[0];
    assert_eq!(comp.name, "CustomerTable");
    assert_eq!(comp.component_type, Some(ComponentType::Table));
    assert!(!comp.properties.iter().any(|p| p.key == "type"));
    assert!(comp.properties.iter().any(|p| p.key == "data"));
    assert_eq!(comp.events.len(), 1);

    let spec = match &comp.spec {
        Some(ComponentSpec::Table(spec)) => spec,
        other => panic!("Expected Table spec, got {:?}", other),
    };
    assert!(spec.pagination);
    assert_eq!(spec.columns.len(), 3);

    match &spec.columns[0] {
        ColumnDef::Field { label, field } => {
            assert_eq!(label, "Name");
            assert_eq!(field.entity, "Customer");
            assert_eq!(field.property, "name");
        }
        other => panic!("Expected Field column, got {:?}", other),
    }

    match &spec.columns[1] {
        ColumnDef::Lookup {
            label,
            field,
            lookup,
        } => {
            assert_eq!(label, "Status");
            assert_eq!(field.entity, "Customer");
            assert_eq!(field.property, "status");
            assert_eq!(lookup, "status_labels");
        }
        other => panic!("Expected Lookup column, got {:?}", other),
    }

    match &spec.columns[2] {
        ColumnDef::Expression { label, expr } => {
            assert_eq!(label, "Tenure");
            match expr {
                Expression::Call { name, args } => {
                    assert_eq!(name, "tenure_years");
                    assert_eq!(args.len(), 1);
                }
                other => panic!("Expected Call expression, got {:?}", other),
            }
        }
        other => panic!("Expected Expression column, got {:?}", other),
    }
}

#[test]
fn test_columns_imply_table_type_and_expr_columns_parse() {
    let input = r#"
view "V" {
    component "t" {
        column "Age" -> expr Customer.age >= 18;
        column "Birth" -> expr age_years(Customer.birth_date);
    }
}
"#;
    let model = parse_ifml(input).unwrap();
    let comp = &model.views[0].components[0];
    assert_eq!(comp.component_type, Some(ComponentType::Table));

    let spec = match &comp.spec {
        Some(ComponentSpec::Table(spec)) => spec,
        other => panic!("Expected Table spec, got {:?}", other),
    };
    match &spec.columns[0] {
        ColumnDef::Expression { expr, .. } => {
            assert!(matches!(expr, Expression::BinOp { op: BinOp::Ge, .. }));
        }
        other => panic!("Expected Expression column, got {:?}", other),
    }
    match &spec.columns[1] {
        ColumnDef::Expression { expr, .. } => {
            assert!(matches!(
                expr,
                Expression::Call { name, .. } if name == "age_years"
            ));
        }
        other => panic!("Expected Expression column, got {:?}", other),
    }
}

#[test]
fn test_form_fields_required_validations_and_values() {
    let input = r#"
view "Edit" {
    component "EditForm" {
        type: form;
        data: Customer;

        field name   -> input text   { required: true; validations: [len(name) > 2]; }
        field email  -> input email;
        field status -> input dropdown { values: ["gold", "silver"]; }
    }
}
"#;
    let model = parse_ifml(input).unwrap();
    let comp = &model.views[0].components[0];
    assert_eq!(comp.component_type, Some(ComponentType::Form));
    assert!(!comp.properties.iter().any(|p| p.key == "type"));

    let spec = match &comp.spec {
        Some(ComponentSpec::Form(spec)) => spec,
        other => panic!("Expected Form spec, got {:?}", other),
    };
    assert_eq!(spec.fields.len(), 3);

    let name_field = &spec.fields[0];
    assert_eq!(name_field.name, "name");
    assert_eq!(name_field.input, InputFieldType::Text);
    assert!(name_field.required);
    assert!(name_field.values.is_empty());
    assert!(matches!(
        name_field.validations.first(),
        Some(Expression::BinOp { op: BinOp::Gt, .. })
    ));

    let email_field = &spec.fields[1];
    assert_eq!(email_field.name, "email");
    assert_eq!(email_field.input, InputFieldType::Email);
    assert!(!email_field.required);
    assert!(email_field.validations.is_empty());
    assert!(email_field.values.is_empty());

    let status_field = &spec.fields[2];
    assert_eq!(status_field.name, "status");
    assert_eq!(status_field.input, InputFieldType::Dropdown);
    assert_eq!(
        status_field.values,
        vec!["gold".to_string(), "silver".to_string()]
    );
}

#[test]
fn test_form_field_without_body_implies_form_type() {
    let input = r#"
view "F" {
    component "f" {
        field nickname -> input text;
    }
}
"#;
    let model = parse_ifml(input).unwrap();
    let comp = &model.views[0].components[0];
    assert_eq!(comp.component_type, Some(ComponentType::Form));

    let spec = match &comp.spec {
        Some(ComponentSpec::Form(spec)) => spec,
        other => panic!("Expected Form spec, got {:?}", other),
    };
    assert_eq!(spec.fields.len(), 1);
    let field = &spec.fields[0];
    assert_eq!(field.name, "nickname");
    assert_eq!(field.input, InputFieldType::Text);
    assert!(!field.required);
    assert!(field.validations.is_empty());
    assert!(field.values.is_empty());
}

#[test]
fn test_form_field_values_accept_identifiers_and_strings() {
    let input = r#"
view "F" {
    component "f" {
        field tier  -> input dropdown { values: [gold, silver]; }
        field grade -> input dropdown { values: ["a", "b"]; }
    }
}
"#;
    let model = parse_ifml(input).unwrap();
    let comp = &model.views[0].components[0];
    let spec = match &comp.spec {
        Some(ComponentSpec::Form(spec)) => spec,
        other => panic!("Expected Form spec, got {:?}", other),
    };
    assert_eq!(
        spec.fields[0].values,
        vec!["gold".to_string(), "silver".to_string()]
    );
    assert_eq!(
        spec.fields[1].values,
        vec!["a".to_string(), "b".to_string()]
    );
}

#[test]
fn test_input_type_unknown_identifier_becomes_custom() {
    let input = r#"
view "F" {
    component "f" {
        field rating -> input stars;
    }
}
"#;
    let model = parse_ifml(input).unwrap();
    let comp = &model.views[0].components[0];
    let spec = match &comp.spec {
        Some(ComponentSpec::Form(spec)) => spec,
        other => panic!("Expected Form spec, got {:?}", other),
    };
    assert_eq!(
        spec.fields[0].input,
        InputFieldType::Custom("stars".to_string())
    );
}

#[test]
fn test_chart_decl_all_kinds_and_optional_body() {
    let input = r#"
view "Charts" {
    component "c1" {
        type: chart;
        chart bar { label: region; values: [revenue]; }
    }

    component "c2" {
        type: chart;
        chart line { label: month; values: [revenue, cost]; }
    }

    component "c3" {
        type: chart;
        chart pie;
    }

    component "c4" {
        type: chart;
        chart radar;
    }

    component "c5" {
        type: chart;
        chart metric;
    }
}
"#;
    let model = parse_ifml(input).unwrap();
    let components = &model.views[0].components;
    assert_eq!(components.len(), 5);

    let expected_kinds = [
        ChartKind::Bar,
        ChartKind::Line,
        ChartKind::Pie,
        ChartKind::Radar,
        ChartKind::Metric,
    ];
    for (comp, expected_kind) in components.iter().zip(expected_kinds.iter()) {
        assert_eq!(comp.component_type, Some(ComponentType::Chart));
        let spec = match &comp.spec {
            Some(ComponentSpec::Chart(spec)) => spec,
            other => panic!("Expected Chart spec, got {:?}", other),
        };
        assert_eq!(spec.kind, *expected_kind);
    }

    let bar = match &components[0].spec {
        Some(ComponentSpec::Chart(spec)) => spec,
        other => panic!("Expected Chart spec, got {:?}", other),
    };
    assert_eq!(bar.label_field, Some("region".to_string()));
    assert_eq!(bar.value_fields, vec!["revenue".to_string()]);

    let line = match &components[1].spec {
        Some(ComponentSpec::Chart(spec)) => spec,
        other => panic!("Expected Chart spec, got {:?}", other),
    };
    assert_eq!(line.label_field, Some("month".to_string()));
    assert_eq!(
        line.value_fields,
        vec!["revenue".to_string(), "cost".to_string()]
    );

    let bodyless = match &components[2].spec {
        Some(ComponentSpec::Chart(spec)) => spec,
        other => panic!("Expected Chart spec, got {:?}", other),
    };
    assert_eq!(bodyless.label_field, None);
    assert!(bodyless.value_fields.is_empty());
}

#[test]
fn test_chart_decl_without_type_implies_chart_type() {
    let input = r#"
view "V" {
    component "spark" {
        chart metric { label: kpi; }
    }
}
"#;
    let model = parse_ifml(input).unwrap();
    let comp = &model.views[0].components[0];
    assert_eq!(comp.component_type, Some(ComponentType::Chart));

    let spec = match &comp.spec {
        Some(ComponentSpec::Chart(spec)) => spec,
        other => panic!("Expected Chart spec, got {:?}", other),
    };
    assert_eq!(spec.kind, ChartKind::Metric);
    assert_eq!(spec.label_field, Some("kpi".to_string()));
    assert!(spec.value_fields.is_empty());
}

#[test]
fn test_chart_decl_org_kind_with_payload_fields() {
    let input = r#"
view "Org" {
    component "tree" {
        type: chart;
        chart org {
            hierarchy: managerId;
            labels: [name, title];
            details: [email, phone];
            maxDepth: 4;
        }
    }
}
"#;
    let model = parse_ifml(input).unwrap();
    let comp = &model.views[0].components[0];
    assert_eq!(comp.component_type, Some(ComponentType::Chart));

    let spec = match &comp.spec {
        Some(ComponentSpec::Chart(spec)) => spec,
        other => panic!("Expected Chart spec, got {:?}", other),
    };
    assert_eq!(spec.kind, ChartKind::Org);
    assert_eq!(spec.hierarchy_field, Some("managerId".to_string()));
    assert_eq!(
        spec.label_fields,
        vec!["name".to_string(), "title".to_string()]
    );
    assert_eq!(
        spec.detail_fields,
        vec!["email".to_string(), "phone".to_string()]
    );
    assert_eq!(spec.max_depth, Some(4));
    assert_eq!(spec.aggregate, None);
    assert_eq!(spec.label_field, None);
    assert!(spec.value_fields.is_empty());
}

#[test]
fn test_chart_decl_org_bodyless_defaults() {
    let model = parse_ifml(
        r#"
view "Org" {
    component "tree" {
        chart org;
    }
}
"#,
    )
    .unwrap();
    let spec = match &model.views[0].components[0].spec {
        Some(ComponentSpec::Chart(spec)) => spec,
        other => panic!("Expected Chart spec, got {:?}", other),
    };
    assert_eq!(spec.kind, ChartKind::Org);
    assert_eq!(spec.hierarchy_field, None);
    assert!(spec.label_fields.is_empty());
    assert!(spec.detail_fields.is_empty());
    assert_eq!(spec.max_depth, None);
    assert_eq!(spec.aggregate, None);
}

#[test]
fn test_chart_decl_series_aggregate() {
    let expected = [
        ("sum", ChartAggregate::Sum),
        ("avg", ChartAggregate::Avg),
        ("count", ChartAggregate::Count),
        ("min", ChartAggregate::Min),
        ("max", ChartAggregate::Max),
    ];
    for (word, want) in expected {
        let input = format!(
            r#"
view "Charts" {{
    component "c" {{
        chart bar {{ label: region; values: [revenue]; aggregate: {word}; }}
    }}
}}
"#
        );
        let model = parse_ifml(&input).unwrap();
        let comp = &model.views[0].components[0];
        assert_eq!(comp.component_type, Some(ComponentType::Chart));
        let spec = match &comp.spec {
            Some(ComponentSpec::Chart(spec)) => spec,
            other => panic!("Expected Chart spec, got {:?}", other),
        };
        assert_eq!(spec.kind, ChartKind::Bar);
        assert_eq!(spec.aggregate, Some(want), "aggregate word {word:?}");
        assert_eq!(spec.label_field, Some("region".to_string()));
        assert_eq!(spec.value_fields, vec!["revenue".to_string()]);
    }

    // Absent aggregate stays unspecified (consumers default it).
    let model = parse_ifml(
        r#"
view "Charts" {
    component "c" {
        chart bar { label: region; values: [revenue]; }
    }
}
"#,
    )
    .unwrap();
    let spec = match &model.views[0].components[0].spec {
        Some(ComponentSpec::Chart(spec)) => spec,
        other => panic!("Expected Chart spec, got {:?}", other),
    };
    assert_eq!(spec.aggregate, None);
}

#[test]
fn test_chart_decl_unknown_aggregate_is_a_parse_error() {
    let err = parse_ifml(
        r#"
view "Charts" {
    component "c" {
        chart bar { aggregate: median; }
    }
}
"#,
    )
    .expect_err("unknown aggregate must be a parse error");
    assert!(
        err.to_string().contains("median"),
        "error names the offending aggregate: {err}"
    );
    assert!(
        err.to_string().contains("aggregate"),
        "error names the aggregate property: {err}"
    );
}

#[test]
fn test_chart_decl_unknown_kind_is_a_parse_error() {
    assert!(parse_ifml(
        r#"
view "Charts" {
    component "c" {
        chart sankey;
    }
}
"#,
    )
    .is_err());
}

#[test]
fn test_chart_ref_property_places_a_modeled_chart() {
    for source in [
        r#"
view "V" {
    component "headcount" {
        chart: "headcount";
    }
}
"#,
        r#"
view "V" {
    component "c" {
        type: chart;
        chart: "headcount";
    }
}
"#,
    ] {
        let model = parse_ifml(source).unwrap();
        let comp = &model.views[0].components[0];
        assert_eq!(comp.component_type, Some(ComponentType::Chart));
        assert_eq!(
            comp.spec,
            Some(ComponentSpec::ChartRef("headcount".to_string()))
        );
    }
}

#[test]
fn test_chart_ref_and_inline_chart_conflict_is_a_parse_error() {
    let err = parse_ifml(
        r#"
view "V" {
    component "c" {
        chart: "headcount";
        chart bar { label: region; values: [revenue]; }
    }
}
"#,
    )
    .expect_err("a chart reference and an inline chart spec are mutually exclusive");
    assert!(
        err.to_string().contains("c"),
        "error names the component: {err}"
    );
}

#[test]
fn test_unknown_component_type_becomes_custom_and_is_removed() {
    let input = r#"
view "Board" {
    component "lane" {
        type: kanban;
        data: Card;
    }
}
"#;
    let model = parse_ifml(input).unwrap();
    let comp = &model.views[0].components[0];
    assert_eq!(
        comp.component_type,
        Some(ComponentType::Custom("kanban".to_string()))
    );
    assert!(comp.spec.is_none());
    assert!(!comp.properties.iter().any(|p| p.key == "type"));
    assert!(comp.properties.iter().any(|p| p.key == "data"));
}

#[test]
fn test_non_identifier_type_property_stays_in_properties() {
    let input = r#"
view "V" {
    component "c" {
        type: "table";
        data: Customer;
    }
}
"#;
    let model = parse_ifml(input).unwrap();
    let comp = &model.views[0].components[0];
    assert_eq!(comp.component_type, None);
    assert!(comp.spec.is_none());
    let type_prop = comp
        .properties
        .iter()
        .find(|p| p.key == "type")
        .expect("type property should stay");
    match &type_prop.value {
        ValueExpression::String(s) => assert_eq!(s, "table"),
        other => panic!("Expected String value, got {:?}", other),
    }
}

#[test]
fn test_pagination_false_property_consumed_into_table_spec() {
    let input = r#"
view "V" {
    component "t" {
        type: table;
        pagination: false;
        column "A" -> field Customer.a;
    }
}
"#;
    let model = parse_ifml(input).unwrap();
    let comp = &model.views[0].components[0];
    assert!(!comp.properties.iter().any(|p| p.key == "pagination"));

    let spec = match &comp.spec {
        Some(ComponentSpec::Table(spec)) => spec,
        other => panic!("Expected Table spec, got {:?}", other),
    };
    assert!(!spec.pagination);
}

#[test]
fn test_legacy_property_bag_component_backward_compat() {
    let input = r#"
view "Legacy" {
    component "grid" {
        type: table;
        data: Customer;
        fields: [name, email];
        sortable: true;
        paginated: true;

        on select(row) -> navigate("Detail", { id: row.id });
    }
}
"#;
    let model = parse_ifml(input).unwrap();
    let comp = &model.views[0].components[0];
    assert_eq!(comp.component_type, Some(ComponentType::Table));
    assert!(comp.spec.is_none());
    assert_eq!(comp.properties.len(), 4);
    assert_eq!(comp.events.len(), 1);
}

#[test]
fn test_component_type_parse_is_case_insensitive() {
    assert_eq!(ComponentType::from("list"), ComponentType::List);
    assert_eq!(ComponentType::from("FORM"), ComponentType::Form);
    assert_eq!(ComponentType::from("Details"), ComponentType::Details);
    assert_eq!(ComponentType::from("Search"), ComponentType::Search);
    assert_eq!(ComponentType::from("tree"), ComponentType::Tree);
    assert_eq!(ComponentType::from("Chart"), ComponentType::Chart);
    assert_eq!(ComponentType::from("TABLE"), ComponentType::Table);
    assert_eq!(ComponentType::from("button"), ComponentType::Button);
    assert_eq!(ComponentType::from("Link"), ComponentType::Link);
    assert_eq!(ComponentType::from("menu"), ComponentType::Menu);
    assert_eq!(ComponentType::from("Image"), ComponentType::Image);
    assert_eq!(ComponentType::from("embedded"), ComponentType::Embedded);
    assert_eq!(
        ComponentType::from("kanban"),
        ComponentType::Custom("kanban".to_string())
    );
    assert_eq!(ComponentType::List.as_str(), "list");
    assert_eq!(ComponentType::Table.as_str(), "table");
    assert_eq!(
        ComponentType::Custom("kanban".to_string()).as_str(),
        "kanban"
    );
}

#[test]
fn test_bad_property_ref_fails_parse() {
    let input = r#"
view "Bad" {
    component "t" {
        column "X" -> field Customer;
    }
}
"#;
    assert!(parse_ifml(input).is_err());
}

#[test]
fn test_dangling_column_fails_parse() {
    let input = r#"
view "Bad" {
    component "t" {
        column "X" -> field Customer.name
    }
}
"#;
    assert!(parse_ifml(input).is_err());
}

#[test]
fn test_form_field_messages_parse() {
    let input = r#"
view "Edit" {
    component "EditForm" {
        type: form;
        data: Customer;

        field name  -> input text   { required: true; validations: [len(name) > 2, name ~= "^[A-Z]"]; messages: ["Name too short", "Must start uppercase"]; }
        field email -> input email  { validations: [is_email(email)]; messages: ["Invalid email"]; }
        field nick  -> input text;
    }
}
"#;
    let model = parse_ifml(input).unwrap();
    let spec = match &model.views[0].components[0].spec {
        Some(ComponentSpec::Form(spec)) => spec,
        other => panic!("Expected Form spec, got {:?}", other),
    };
    assert_eq!(spec.fields.len(), 3);

    let name_field = &spec.fields[0];
    assert_eq!(name_field.validations.len(), 2);
    assert_eq!(
        name_field.messages,
        vec![
            "Name too short".to_string(),
            "Must start uppercase".to_string()
        ]
    );

    let email_field = &spec.fields[1];
    assert_eq!(email_field.validations.len(), 1);
    assert_eq!(email_field.messages, vec!["Invalid email".to_string()]);

    let nick_field = &spec.fields[2];
    assert!(nick_field.validations.is_empty());
    assert!(nick_field.messages.is_empty());
}
