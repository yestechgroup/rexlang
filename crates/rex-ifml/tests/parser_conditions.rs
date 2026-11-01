use rex_ifml::*;

#[test]
fn test_view_condition_statement() {
    let input = r#"
view "AdminDashboard" {
    if user.role == "admin";

    component "grid" {
        type: list;
        data: Customer;
    }
}
"#;
    let model = parse_ifml(input).unwrap();
    let view = &model.views[0];
    assert_eq!(
        view.condition,
        Some(Expression::BinOp {
            left: Box::new(Expression::FieldExpr {
                object: Box::new(Expression::Ident("user".to_string())),
                field: "role".to_string(),
            }),
            op: BinOp::Eq,
            right: Box::new(Expression::StringLit("admin".to_string())),
        })
    );
}

#[test]
fn test_container_condition_statement() {
    let input = r#"
view "Wizard" {
    container "Step1" {
        if params.enabled != false;
        component "form" {
            type: form;
            data: Customer;
        }
    }
}
"#;
    let model = parse_ifml(input).unwrap();
    let container = &model.views[0].containers[0];
    assert!(matches!(
        &container.condition,
        Some(Expression::BinOp { op: BinOp::Ne, .. })
    ));
}

#[test]
fn test_component_condition_statement() {
    let input = r#"
view "Dashboard" {
    component "adminPanel" {
        type: list;
        data: Customer;
        if user.is_admin;
    }
}
"#;
    let model = parse_ifml(input).unwrap();
    let comp = &model.views[0].components[0];
    assert_eq!(
        comp.condition,
        Some(Expression::FieldExpr {
            object: Box::new(Expression::Ident("user".to_string())),
            field: "is_admin".to_string(),
        })
    );
}

#[test]
fn test_condition_statement_last_wins() {
    let input = r#"
view "V" {
    if flag_a;
    if flag_b;
}
"#;
    let model = parse_ifml(input).unwrap();
    assert_eq!(
        model.views[0].condition,
        Some(Expression::Ident("flag_b".to_string()))
    );
}

#[test]
fn test_dangling_if_fails_parse() {
    let cases = [
        r#"
view "Bad" {
    if;
}
"#,
        r#"
view "Bad" {
    component "c" {
        type: list;
        if;
    }
}
"#,
        r#"
view "Bad" {
    component "c" {
        type: list;
        on click if -> stay;
    }
}
"#,
        r#"
view "Bad" {
    if
}
"#,
    ];
    for (i, input) in cases.iter().enumerate() {
        assert!(
            parse_ifml(input).is_err(),
            "dangling if case {} should fail to parse",
            i
        );
    }
}
