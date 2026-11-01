//! Event handlers and the actions they guard: `on <event>(params)
//! requires: [...] if <cond> -> <action>` plus action bodies
//! (`navigate`/`refresh`/action invocation/stay).

use super::Rule;
use pest::iterators::Pair;

use rex_ir::ifml::*;

use crate::parser::expressions::{parse_expression, parse_property_assignment};
use crate::parser::utils::parse_string;

fn parse_event_type(pair: Pair<Rule>) -> EventType {
    match pair.as_str() {
        "select" => EventType::Select,
        "submit" => EventType::Submit,
        "click" => EventType::Click,
        "change" => EventType::Change,
        "load" => EventType::Load,
        "save" => EventType::Save,
        "cancel" => EventType::Cancel,
        "delete" => EventType::Delete,
        "confirm" => EventType::Confirm,
        "back" => EventType::Back,
        s => EventType::Custom(s.to_string()),
    }
}

fn parse_event_param(pair: Pair<Rule>) -> Vec<String> {
    let mut params = Vec::new();
    for child in pair.clone().into_inner() {
        params.push(child.as_str().to_string());
    }
    params
}

// ── Event / Action parsing ───────────────────────────────────────

fn parse_parameter_binding(pair: Pair<Rule>) -> ParameterBinding {
    let mut pairs = Vec::new();
    for child in pair.clone().into_inner() {
        if child.as_rule() == Rule::parameter_binding_pair {
            let mut inner = child.into_inner();
            let key = inner
                .next()
                .map(|p| p.as_str().to_string())
                .unwrap_or_default();
            let value = inner
                .next()
                .map(parse_expression)
                .unwrap_or(Expression::Ident("".to_string()));
            pairs.push((key, value));
        }
    }
    ParameterBinding { pairs }
}

fn parse_event_action(pair: Pair<Rule>) -> EventAction {
    let inner = if let Some(inner) = pair.clone().into_inner().next() {
        inner
    } else {
        return EventAction::Stay;
    };
    match inner.as_rule() {
        Rule::navigate_action => {
            let mut nav_inner = inner.clone().into_inner();
            let target = nav_inner
                .next()
                .map(|p| parse_string(&p))
                .unwrap_or_default();
            let binding = nav_inner.next().map(parse_parameter_binding);
            EventAction::Navigate { target, binding }
        }
        Rule::refresh_action => {
            let mut ref_inner = inner.clone().into_inner();
            let target = ref_inner
                .next()
                .map(|p| parse_string(&p))
                .unwrap_or_default();
            let binding = ref_inner.next().map(parse_parameter_binding);
            EventAction::Refresh { target, binding }
        }
        Rule::action_invocation => {
            let mut act_inner = inner.clone().into_inner();
            let name = act_inner
                .next()
                .map(|p| parse_string(&p))
                .unwrap_or_default();
            let body = act_inner.next().map(parse_action_body);
            EventAction::ActionInvocation { name, body }
        }
        Rule::stay_statement => EventAction::Stay,
        _ => EventAction::Stay,
    }
}

fn parse_action_body(pair: Pair<Rule>) -> ActionBody {
    let mut properties = Vec::new();
    let mut handlers = Vec::new();
    for child in pair.clone().into_inner() {
        match child.as_rule() {
            Rule::property_assignment => properties.push(parse_property_assignment(child)),
            Rule::event_handler => handlers.push(parse_event_handler(child)),
            _ => {}
        }
    }
    ActionBody {
        properties,
        handlers,
    }
}

pub(crate) fn parse_event_handler(pair: Pair<Rule>) -> EventHandler {
    let mut inner = pair.clone().into_inner();
    let event_type = inner
        .next()
        .map(parse_event_type)
        .unwrap_or(EventType::Custom("unknown".to_string()));

    let mut params = Vec::new();
    let mut requires: Vec<String> = Vec::new();
    let mut condition = None;
    let mut action = EventAction::Stay;

    for child in inner {
        match child.as_rule() {
            Rule::event_param => params = parse_event_param(child),
            Rule::event_requires => {
                requires = child
                    .into_inner()
                    .map(|cap| cap.as_str().to_string())
                    .collect();
            }
            Rule::event_condition => {
                condition = child.into_inner().next().map(parse_expression);
            }
            _ => action = parse_event_action(child),
        }
    }

    EventHandler {
        event_type,
        params,
        requires,
        condition,
        action,
    }
}
