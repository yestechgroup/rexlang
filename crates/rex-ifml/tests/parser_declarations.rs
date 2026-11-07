use rex_ifml::*;

#[test]
fn test_domain_declaration() {
    let input = r#"
domain "sales" {
    schema "sales";
}

view "Simple" {
    component "c" {
        type: list;
        data: Item;
    }
}
"#;
    let model = parse_ifml(input).unwrap();
    assert_eq!(model.domains.len(), 1);
    assert_eq!(model.domains[0].name, "sales");
    assert_eq!(model.domains[0].schema_name, "sales");
    assert_eq!(model.views.len(), 1);
}

#[test]
fn test_action_declaration() {
    let input = r#"
action "ArchiveOldOrders" {
    description: "Archives orders older than 90 days";
    on success -> navigate("OrderList");
    on error -> stay;
}
"#;
    let model = parse_ifml(input).unwrap();
    assert_eq!(model.actions.len(), 1);
    let action = &model.actions[0];
    assert_eq!(action.name, "ArchiveOldOrders");
    assert_eq!(action.properties.len(), 1);
    assert_eq!(action.properties[0].key, "description");
    assert_eq!(action.events.len(), 2);
}

#[test]
fn test_domain_only() {
    let input = r#"
domain "hr" {
    schema "hr_schema";
}
"#;
    let model = parse_ifml(input).unwrap();
    assert_eq!(model.domains.len(), 1);
    assert_eq!(model.domains[0].name, "hr");
    assert_eq!(model.domains[0].schema_name, "hr_schema");
}

#[test]
fn test_actor_declaration_parses() {
    let input = r#"
actor "Admin" {
    label: "Administrator";
    description: "Full system access";
}

actor "Auditor" {
    label: "Auditor";
}

view "Dashboard" {
    component "grid" {
        type: list;
        data: Item;
    }
}
"#;
    let model = parse_ifml(input).unwrap();
    assert_eq!(model.actors.len(), 2);
    assert_eq!(model.views.len(), 1);

    let admin = &model.actors[0];
    assert_eq!(admin.name, "Admin");
    assert_eq!(admin.properties.len(), 2);
    assert_eq!(admin.properties[0].key, "label");
    match &admin.properties[0].value {
        ValueExpression::String(s) => assert_eq!(s, "Administrator"),
        other => panic!("Expected String, got {:?}", other),
    }

    let auditor = &model.actors[1];
    assert_eq!(auditor.name, "Auditor");
    assert_eq!(auditor.properties.len(), 1);
}

#[test]
fn test_actor_with_empty_body_parses() {
    let input = r#"
actor "Guest" {}

view "Public" {
    component "grid" {
        type: list;
        data: Item;
    }
}
"#;
    let model = parse_ifml(input).unwrap();
    assert_eq!(model.actors.len(), 1);
    assert_eq!(model.actors[0].name, "Guest");
    assert!(model.actors[0].properties.is_empty());
}

#[test]
fn test_actor_ast_serde_round_trip() {
    let input = r#"
actor "Admin" {
    label: "Administrator";
}

view "Console" {
    roles: [admin];
    component "grid" {
        type: list;
        data: Item;
    }
}
"#;
    let model = parse_ifml(input).unwrap();
    let json = serde_json::to_string(&model).expect("serialize model");
    let round_tripped: IfmlModel = serde_json::from_str(&json).expect("deserialize model");
    let reserialized = serde_json::to_string(&round_tripped).expect("re-serialize model");
    assert_eq!(reserialized, json);
    assert_eq!(round_tripped.actors[0].name, "Admin");
    assert_eq!(round_tripped.views[0].roles, vec!["admin".to_string()]);
}

#[test]
fn test_dsl_feature_ast_serde_round_trip() {
    let input = r#"
view "Catalog" {
    params { slug: String = "home" };
    use "Pagination" as pager { page_size: 25; };

    container "Main" {
        use "Footer";
    }

    component "f" {
        type: form;
        data: Customer;
        field name -> input text { required: true; validations: [len(name) > 2]; messages: ["Too short"]; }
    }
}
"#;
    let model = parse_ifml(input).unwrap();
    let json = serde_json::to_string(&model).expect("serialize model");
    let round_tripped: IfmlModel = serde_json::from_str(&json).expect("deserialize model");
    let reserialized = serde_json::to_string(&round_tripped).expect("re-serialize model");
    assert_eq!(reserialized, json);

    assert_eq!(
        round_tripped.views[0].params[0].default,
        Some(ValueExpression::String("home".to_string()))
    );
    assert_eq!(
        round_tripped.views[0].module_uses[0].alias.as_deref(),
        Some("pager")
    );
    let field = match &round_tripped.views[0].components[0].spec {
        Some(ComponentSpec::Form(spec)) => &spec.fields[0],
        other => panic!("Expected Form spec, got {:?}", other),
    };
    assert_eq!(field.messages, vec!["Too short".to_string()]);
}

#[test]
fn test_import_at_start() {
    let input = r#"
import "rexlang/auth.actor";

domain "sales" {
    schema "sales";
}

view "CustomerList" {
    component "grid" {
        type: list;
        data: Customer;
    }
}
"#;
    let model = parse_ifml(input).unwrap();
    assert_eq!(model.imports, vec!["rexlang/auth.actor".to_string()]);
    assert_eq!(model.domains.len(), 1);
    assert_eq!(model.views.len(), 1);
}

#[test]
fn test_import_mixed_positions() {
    let input = r#"
domain "sales" {
    schema "sales";
}

import "rexlang/middle.actor";

view "CustomerList" {
    component "grid" {
        type: list;
        data: Customer;
    }
}

actor "Admin" {
    label: "Administrator";
}

import "rexlang/end.actor";
"#;
    let model = parse_ifml(input).unwrap();
    assert_eq!(
        model.imports,
        vec![
            "rexlang/middle.actor".to_string(),
            "rexlang/end.actor".to_string()
        ]
    );
    assert_eq!(model.domains.len(), 1);
    assert_eq!(model.views.len(), 1);
    assert_eq!(model.actors.len(), 1);
}

#[test]
fn test_duplicate_imports_preserved_verbatim() {
    let input = r#"
import "rexlang/auth.actor";
import "rexlang/auth.actor";
import "other/thing.actor";
import "rexlang/auth.actor";
"#;
    let model = parse_ifml(input).unwrap();
    assert_eq!(
        model.imports,
        vec![
            "rexlang/auth.actor".to_string(),
            "rexlang/auth.actor".to_string(),
            "other/thing.actor".to_string(),
            "rexlang/auth.actor".to_string(),
        ]
    );
}

#[test]
fn test_import_without_semicolon_parses() {
    let input = r#"
import "rexlang/auth.actor"

view "V" {
    component "c" {
        type: list;
        data: Item;
    }
}
"#;
    let model = parse_ifml(input).unwrap();
    assert_eq!(model.imports, vec!["rexlang/auth.actor".to_string()]);
}

#[test]
fn test_no_imports_yields_empty_vec() {
    let input = r#"
domain "sales" {
    schema "sales";
}

view "CustomerList" {
    component "grid" {
        type: list;
        data: Customer;
    }
}
"#;
    let model = parse_ifml(input).unwrap();
    assert!(model.imports.is_empty());
}

#[test]
fn test_dangling_import_fails_parse() {
    let cases = [
        r#"
import;
"#,
        r#"
import "missing-close
"#,
        r#"
import missing-quotes;
"#,
    ];
    for (i, input) in cases.iter().enumerate() {
        assert!(
            parse_ifml(input).is_err(),
            "dangling import case {} should fail to parse",
            i
        );
    }
}

#[test]
fn test_imports_and_requires_serde_round_trip() {
    let input = r#"
import "rexlang/auth.actor";

view "RefundQueue" {
    roles: [approver];
    requires: [RaiseRefund];

    component "grid" {
        type: list;
        data: Refund;
    }
}
"#;
    let model = parse_ifml(input).unwrap();
    let json = serde_json::to_string(&model).expect("serialize model");
    let round_tripped: IfmlModel = serde_json::from_str(&json).expect("deserialize model");
    let reserialized = serde_json::to_string(&round_tripped).expect("re-serialize model");
    assert_eq!(reserialized, json);
    assert_eq!(
        round_tripped.imports,
        vec!["rexlang/auth.actor".to_string()]
    );
    assert_eq!(
        round_tripped.views[0].requires,
        vec!["RaiseRefund".to_string()]
    );
}
