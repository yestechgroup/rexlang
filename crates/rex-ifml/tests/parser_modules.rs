use rex_ifml::*;

#[test]
fn test_module_declaration() {
    let input = r#"
module "Pagination" {
    input { page: Int, pageSize: Int }
    output { result: String }

    component "pager" {
        type: list;
        data: Item;
        fields: [name];
    }
}
"#;
    let model = parse_ifml(input).unwrap();
    assert_eq!(model.modules.len(), 1);
    let module = &model.modules[0];
    assert_eq!(module.name, "Pagination");
    assert_eq!(module.input_params.len(), 2);
    assert_eq!(module.input_params[0].name, "page");
    assert_eq!(module.input_params[0].type_ref, "Int");
    assert_eq!(module.output_params.len(), 1);
    assert_eq!(module.output_params[0].name, "result");
    assert_eq!(module.components.len(), 1);
}

#[test]
fn test_module_internal_use() {
    let input = r#"
module "MasterDetail" {
    input { entityId: Uuid, pageSize: Int = 20 }
    output { selected: Uuid }

    use "Pagination" as inner {
        page_size: 50;
    }

    component "list" {
        type: list;
        data: Item;
    }

    use "Footer";
}
"#;
    let model = parse_ifml(input).unwrap();
    assert_eq!(model.modules.len(), 1);
    let module = &model.modules[0];
    assert_eq!(module.name, "MasterDetail");
    assert_eq!(module.module_uses.len(), 2);

    assert_eq!(module.module_uses[0].module, "Pagination");
    assert_eq!(module.module_uses[0].alias.as_deref(), Some("inner"));
    assert_eq!(module.module_uses[0].properties.len(), 1);
    assert_eq!(module.module_uses[0].properties[0].key, "page_size");
    assert_eq!(
        module.module_uses[0].properties[0].value,
        ValueExpression::Number(50.0.into())
    );

    assert_eq!(module.module_uses[1].module, "Footer");
    assert_eq!(module.module_uses[1].alias, None);
    assert!(module.module_uses[1].properties.is_empty());
}

/// A `use:` property assignment inside a module body still lowers as a
/// property, not a module use (the colon disambiguates), mirroring the
/// view-body rule.

#[test]
fn test_module_use_still_parses_as_property_name() {
    let input = r#"
module "M" {
    input { page: Int }
    output { total: Int }

    use: "fallback";
}
"#;
    let model = parse_ifml(input).unwrap();
    let module = &model.modules[0];
    assert!(module.module_uses.is_empty());
    assert_eq!(module.properties.len(), 1);
    assert_eq!(module.properties[0].key, "use");
}

#[test]
fn test_module_use_in_view() {
    let input = r#"
view "Catalog" {
    use "Pagination" as pager;
    use "Footer";

    component "grid" {
        type: list;
        data: Product;
    }
}
"#;
    let model = parse_ifml(input).unwrap();
    let uses = &model.views[0].module_uses;
    assert_eq!(uses.len(), 2);

    assert_eq!(uses[0].module, "Pagination");
    assert_eq!(uses[0].alias.as_deref(), Some("pager"));
    assert!(uses[0].properties.is_empty());

    assert_eq!(uses[1].module, "Footer");
    assert_eq!(uses[1].alias, None);
}

#[test]
fn test_module_use_in_container_and_with_properties() {
    let input = r#"
view "Wizard" {
    container "Step1" {
        use "Pagination" as pager {
            page_size: 25;
            compact: true;
        }

        component "form" {
            type: form;
            data: Customer;
        }
    }
}
"#;
    let model = parse_ifml(input).unwrap();
    let container = &model.views[0].containers[0];
    assert_eq!(container.module_uses.len(), 1);

    let module_use = &container.module_uses[0];
    assert_eq!(module_use.module, "Pagination");
    assert_eq!(module_use.alias.as_deref(), Some("pager"));
    assert_eq!(module_use.properties.len(), 2);
    assert_eq!(module_use.properties[0].key, "page_size");
    assert_eq!(
        module_use.properties[0].value,
        ValueExpression::Number(25.0.into())
    );
    assert_eq!(module_use.properties[1].key, "compact");
    assert_eq!(module_use.properties[1].value, ValueExpression::Bool(true));
}

#[test]
fn test_use_still_parses_as_property_name() {
    let input = r#"
view "V" {
    use: "fallback";
    component "c" {
        type: list;
        data: Item;
    }
}
"#;
    let model = parse_ifml(input).unwrap();
    let view = &model.views[0];
    assert!(view.module_uses.is_empty());
    assert_eq!(view.properties.len(), 1);
    assert_eq!(view.properties[0].key, "use");
    match &view.properties[0].value {
        ValueExpression::String(s) => assert_eq!(s, "fallback"),
        other => panic!("Expected String, got {:?}", other),
    }
}

#[test]
fn test_dangling_module_use_fails_parse() {
    let cases = [
        r#"
view "Bad" {
    use "Pagination"
}
"#,
        r#"
view "Bad" {
    use Pagination as pager;
}
"#,
        r#"
view "Bad" {
    use "Pagination" as;
}
"#,
    ];
    for (i, input) in cases.iter().enumerate() {
        assert!(
            parse_ifml(input).is_err(),
            "dangling use case {} should fail to parse",
            i
        );
    }
}
