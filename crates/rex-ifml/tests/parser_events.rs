use rex_ifml::*;

#[test]
fn test_all_action_types() {
    let input = r#"
view "Actions" {
    component "c1" {
        type: list;
        data: Customer;

        on select(row) -> navigate("Detail", { id: row.id });
    }

    component "c2" {
        type: list;
        data: Order;

        on load -> refresh("grid");
    }

    component "c3" {
        type: form;
        data: Customer;

        on save(values) -> action("UpdateCustomer", {
            body: values;
            on success -> navigate("List");
            on error -> stay;
        });

        on cancel -> navigate("List");
    }

    component "c4" {
        type: form;
        data: ConfirmDelete;

        on submit(values) -> action("DeleteProduct", {
            on success -> navigate("ProductList");
            on error -> stay;
        });
    }
}
"#;
    let model = parse_ifml(input).unwrap();
    assert_eq!(model.views.len(), 1);
    let view = &model.views[0];
    assert_eq!(view.components.len(), 4);

    let c1_event = &view.components[0].events[0];
    assert_eq!(c1_event.event_type, EventType::Select);
    assert_eq!(c1_event.params, vec!["row"]);
    match &c1_event.action {
        EventAction::Navigate { target, binding } => {
            assert_eq!(target, "Detail");
            assert!(binding.is_some());
            assert_eq!(binding.as_ref().unwrap().pairs[0].0, "id");
        }
        _ => panic!("Expected Navigate"),
    }

    let c2_event = &view.components[1].events[0];
    assert_eq!(c2_event.event_type, EventType::Load);
    match &c2_event.action {
        EventAction::Refresh { target, binding } => {
            assert_eq!(target, "grid");
            assert!(binding.is_none());
        }
        _ => panic!("Expected Refresh"),
    }

    let c3_save = &view.components[2].events[0];
    assert_eq!(c3_save.event_type, EventType::Save);
    assert_eq!(c3_save.params, vec!["values"]);
    match &c3_save.action {
        EventAction::ActionInvocation { name, body } => {
            assert_eq!(name, "UpdateCustomer");
            let body = body.as_ref().expect("Expected action body");
            assert_eq!(body.properties.len(), 1);
            assert_eq!(body.properties[0].key, "body");
            assert_eq!(body.handlers.len(), 2);
        }
        _ => panic!("Expected ActionInvocation"),
    }

    let c3_cancel = &view.components[2].events[1];
    assert_eq!(c3_cancel.event_type, EventType::Cancel);
    match &c3_cancel.action {
        EventAction::Navigate { target, binding } => {
            assert_eq!(target, "List");
            assert!(binding.is_none());
        }
        _ => panic!("Expected Navigate"),
    }

    let c4_event = &view.components[3].events[0];
    match &c4_event.action {
        EventAction::ActionInvocation { name, body } => {
            assert_eq!(name, "DeleteProduct");
            let body = body.as_ref().expect("Expected action body");
            assert_eq!(body.properties.len(), 0);
            assert_eq!(body.handlers.len(), 2);
        }
        _ => panic!("Expected ActionInvocation"),
    }
}

#[test]
fn test_event_condition() {
    let input = r#"
view "List" {
    component "grid" {
        type: list;
        data: Customer;

        on select(row) if row.active == true -> navigate("Detail", { id: row.id });
        on click -> stay;
    }
}
"#;
    let model = parse_ifml(input).unwrap();
    let events = &model.views[0].components[0].events;
    assert_eq!(events.len(), 2);

    assert_eq!(events[0].event_type, EventType::Select);
    assert_eq!(events[0].condition, Some(row_active_eq_true()));
    match &events[0].action {
        EventAction::Navigate { target, binding } => {
            assert_eq!(target, "Detail");
            assert!(binding.is_some());
        }
        other => panic!("Expected Navigate, got {:?}", other),
    }

    assert_eq!(events[1].event_type, EventType::Click);
    assert_eq!(events[1].condition, None);
}

fn row_active_eq_true() -> Expression {
    Expression::BinOp {
        left: Box::new(Expression::FieldExpr {
            object: Box::new(Expression::Ident("row".to_string())),
            field: "active".to_string(),
        }),
        op: BinOp::Eq,
        right: Box::new(Expression::BoolLit(true)),
    }
}

// ── Event-level capability requirements (issue #208 slice) ──────
//
// Pinned syntax (prefix form): the `requires:` clause sits between the
// event param and the `if` condition / `->` action, mirroring the
// view-level declaration order where `requires:` precedes the guarded
// body:
//
//     on select(row) requires: [RaiseRefund] -> navigate("Review");
//     on save requires: [A, B] if row.ready == true -> stay;
//
// The extracted capabilities surface as `EventHandler.requires`
// (serde default empty). These tests read the field through serde so the
// assertions compile before the field exists (RED: the parse itself
// fails until the grammar learns the clause).

fn event_requires_json(source: &str) -> serde_json::Value {
    let model = parse_ifml(source)
        .unwrap_or_else(|e| panic!("event-level requires should parse: {e}\nsource:{source}"));
    let events = &model.views[0].components[0].events;
    assert!(!events.is_empty(), "component must carry events");
    serde_json::to_value(&events[0]).expect("serialize event handler")
}

#[test]
fn test_event_requires_prefix_form_parses() {
    let json = event_requires_json(
        r#"
view "Refunds" {
    component "grid" {
        type: list;
        data: Refund;

        on select(row) requires: [RaiseRefund] -> navigate("Review", { id: row.id });
    }
}
"#,
    );
    assert_eq!(
        json["requires"],
        serde_json::json!(["RaiseRefund"]),
        "the requires clause must extract into EventHandler.requires: {json}"
    );
    assert_eq!(
        json["eventType"]["type"],
        serde_json::json!("select"),
        "event types serialize as camelCase adjacent tags: {json}"
    );
    assert_eq!(
        json["action"]["type"],
        serde_json::json!("navigate"),
        "the action must survive the requires clause: {json}"
    );
    assert_eq!(
        json["action"]["value"]["target"],
        serde_json::json!("Review"),
        "the action payload rides the `value` key: {json}"
    );
}

#[test]
fn test_event_requires_coexists_with_condition_and_multiple_caps() {
    let json = event_requires_json(
        r#"
view "Refunds" {
    component "editor" {
        type: form;
        data: Refund;

        on save requires: [RaiseRefund, ApproveRefund] if row.ready == true -> stay;
    }
}
"#,
    );
    assert_eq!(
        json["requires"],
        serde_json::json!(["RaiseRefund", "ApproveRefund"]),
        "multiple capabilities extract in declaration order: {json}"
    );
    assert!(
        json["condition"].is_object(),
        "the if-condition must stay in `condition`, not leak into requires: {json}"
    );
}

#[test]
fn test_event_without_requires_defaults_to_empty_list() {
    let json = event_requires_json(
        r#"
view "Refunds" {
    component "grid" {
        type: list;
        data: Refund;

        on click -> stay;
    }
}
"#,
    );
    assert!(
        json.get("requires").is_none(),
        "events without requires omit the key (skip_serializing_if): {json}"
    );
    let model = parse_ifml(
        r#"
view "Refunds" {
    component "grid" {
        type: list;
        data: Refund;

        on click -> stay;
    }
}
"#,
    )
    .expect("parse");
    assert!(model.views[0].components[0].events[0].requires.is_empty());
}
