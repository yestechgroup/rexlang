use rex_ifml::*;

#[test]
fn test_expressions_in_properties() {
    let input = r#"
view "ExprTest" {
    component "grid" {
        type: list;
        data: Product;
        fields: [name, price];
        filter: price > 10.0 && status == "active";
    }
}
"#;
    let model = parse_ifml(input).unwrap();
    let grid = &model.views[0].components[0];

    let filter_prop = grid
        .properties
        .iter()
        .find(|p| p.key == "filter")
        .expect("Expected filter property");

    match &filter_prop.value {
        ValueExpression::BinOp { op, .. } => {
            assert_eq!(*op, BinOp::And);
        }
        other => {
            panic!("Expected BinOp(And), got {:?}", other);
        }
    }

    assert_eq!(grid.properties.len(), 3);
}

#[test]
fn test_boolean_properties() {
    let input = r#"
view "Test" {
    landmark: true;
    xor: false;
    modal: true;

    component "c" {
        type: list;
        data: X;
    }
}
"#;
    let model = parse_ifml(input).unwrap();
    let view = &model.views[0];
    assert!(view.is_landmark);
    assert!(!view.is_xor);
    assert!(view.is_modal);
}

#[test]
fn test_deep_field_navigation_folds_left() {
    let input = r#"
view "Catalogue" {
    component "grid" {
        type: list;
        data: Product;
        filter: product.category.name == "tools" && product.brand.name != null;
    }
}
"#;
    let model = parse_ifml(input).unwrap();
    let filter = model.views[0].components[0]
        .properties
        .iter()
        .find(|p| p.key == "filter")
        .unwrap();
    let deep = |object: &ValueExpression, field: &str| ValueExpression::FieldAccess {
        object: Box::new(object.clone()),
        field: field.to_string(),
    };
    let product_category_name = deep(
        &ValueExpression::Identifier("product".to_string()),
        "category",
    );
    let product_category_name = deep(&product_category_name, "name");
    let product_brand_name = deep(&ValueExpression::Identifier("product".to_string()), "brand");
    let product_brand_name = deep(&product_brand_name, "name");
    assert_eq!(
        filter.value,
        ValueExpression::BinOp {
            left: Box::new(ValueExpression::BinOp {
                left: Box::new(product_category_name),
                op: BinOp::Eq,
                right: Box::new(ValueExpression::String("tools".to_string())),
            }),
            op: BinOp::And,
            right: Box::new(ValueExpression::BinOp {
                left: Box::new(product_brand_name),
                op: BinOp::Ne,
                right: Box::new(ValueExpression::Identifier("null".to_string())),
            }),
        }
    );
}

#[test]
fn test_condition_operator_structure_round_trip() {
    let input = r#"
view "Guarded" {
    if user.role == "admin" && account.balance >= 100 || !user.locked;
    component "grid" {
        type: list;
        data: Customer;
        if !(a + 1 < 3) && name ~= "^A";
        on select(row) if (row.x != 1 || row.y <= 2) -> stay;
    }
}
"#;
    let model = parse_ifml(input).unwrap();
    let view = &model.views[0];

    match &view.condition {
        Some(Expression::BinOp {
            op: BinOp::Or,
            left,
            ..
        }) => {
            assert!(matches!(
                left.as_ref(),
                Expression::BinOp { op: BinOp::And, .. }
            ));
        }
        other => panic!("Expected Or at top level, got {:?}", other),
    }

    let comp = &view.components[0];
    match &comp.condition {
        Some(Expression::BinOp {
            op: BinOp::And,
            left,
            right,
        }) => {
            assert!(matches!(
                left.as_ref(),
                Expression::UnaryOp {
                    op: UnaryOp::Not,
                    ..
                }
            ));
            assert!(matches!(
                right.as_ref(),
                Expression::BinOp {
                    op: BinOp::RegexMatch,
                    ..
                }
            ));
        }
        other => panic!("Expected And, got {:?}", other),
    }

    let event = &comp.events[0];
    match &event.condition {
        Some(Expression::Group(inner)) => match inner.as_ref() {
            Expression::BinOp {
                op: BinOp::Or,
                left,
                right,
            } => {
                assert!(matches!(
                    left.as_ref(),
                    Expression::BinOp { op: BinOp::Ne, .. }
                ));
                assert!(matches!(
                    right.as_ref(),
                    Expression::BinOp { op: BinOp::Le, .. }
                ));
            }
            other => panic!("Expected Or inside group, got {:?}", other),
        },
        other => panic!("Expected Group, got {:?}", other),
    }
}
