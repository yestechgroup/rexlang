//! Lexeme helpers (quoted strings, numbers, identifiers), the shared
//! `parameter` declaration lowering, and property-bag extractors that
//! pull well-known keys (`position`, `roles`, `requires`, …) out of a
//! parsed property list.

use super::Rule;
use pest::iterators::Pair;

use rex_ir::ifml::*;

pub(crate) fn parse_string(pair: &Pair<Rule>) -> String {
    let s = pair.as_str();
    let inner = &s[1..s.len() - 1];
    let mut result = String::with_capacity(inner.len());
    let mut chars = inner.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('"') => result.push('"'),
                Some('\\') => result.push('\\'),
                Some('/') => result.push('/'),
                Some('b') => result.push('\u{0008}'),
                Some('f') => result.push('\u{000C}'),
                Some('n') => result.push('\n'),
                Some('r') => result.push('\r'),
                Some('t') => result.push('\t'),
                Some('u') => {
                    let hex: String = chars.by_ref().take(4).collect();
                    if let Ok(code) = u32::from_str_radix(&hex, 16) {
                        if let Some(ch) = char::from_u32(code) {
                            result.push(ch);
                        }
                    }
                }
                _ => {}
            }
        } else {
            result.push(c);
        }
    }
    result
}

pub(crate) fn parse_number(pair: &Pair<Rule>) -> f64 {
    pair.as_str().parse::<f64>().unwrap_or(0.0)
}

pub(crate) fn parse_identifier(pair: &Pair<Rule>) -> String {
    pair.as_str().to_string()
}

fn parse_parameter_decl(pair: Pair<Rule>) -> ParameterDecl {
    let mut inner = pair.clone().into_inner();
    let name = inner
        .next()
        .map(|p| parse_identifier(&p))
        .unwrap_or_default();
    let type_ref = inner
        .next()
        .map(|p| parse_identifier(&p))
        .unwrap_or_default();
    let default = inner
        .next()
        .filter(|p| p.as_rule() == Rule::param_default)
        .and_then(|p| p.into_inner().next())
        .map(|p| match p.as_rule() {
            Rule::string => ValueExpression::String(parse_string(&p)),
            Rule::number => ValueExpression::Number(parse_number(&p).into()),
            Rule::boolean => ValueExpression::Bool(p.as_str() == "true"),
            _ => ValueExpression::Identifier(p.as_str().to_string()),
        });
    ParameterDecl {
        name,
        type_ref,
        default,
    }
}

pub(crate) fn parse_parameter_block(pair: Pair<Rule>) -> Vec<ParameterDecl> {
    let mut params = Vec::new();
    for child in pair.clone().into_inner() {
        if child.as_rule() == Rule::parameter_decl {
            params.push(parse_parameter_decl(child));
        }
    }
    params
}

pub(crate) fn extract_bool_property(properties: &[PropertyAssignment], key: &str) -> bool {
    properties
        .iter()
        .find(|p| p.key == key)
        .and_then(|p| {
            if let ValueExpression::Bool(val) = p.value {
                Some(val)
            } else {
                None
            }
        })
        .unwrap_or(false)
}

pub(crate) fn extract_position_property(properties: &[PropertyAssignment]) -> Option<Position> {
    let prop = properties.iter().find(|p| p.key == "position")?;
    let members = match &prop.value {
        ValueExpression::Object(members) => members,
        _ => return None,
    };
    let as_number = |v: &ValueExpression| -> Option<f64> {
        match v {
            ValueExpression::Number(n) => Some(n.value()),
            ValueExpression::UnaryOp {
                op: UnaryOp::Neg,
                operand,
            } => match operand.as_ref() {
                ValueExpression::Number(n) => Some(-n.value()),
                _ => None,
            },
            _ => None,
        }
    };
    let get = |key: &str| {
        members
            .iter()
            .find(|m| m.key == key)
            .and_then(|m| as_number(&m.value))
    };
    match (get("x"), get("y")) {
        (Some(x), Some(y)) => Some(Position {
            x: x.into(),
            y: y.into(),
        }),
        _ => None,
    }
}

pub(crate) fn extract_roles_property(properties: &[PropertyAssignment]) -> Vec<String> {
    extract_identifier_list_property(properties, "roles")
}

pub(crate) fn extract_requires_property(properties: &[PropertyAssignment]) -> Vec<String> {
    extract_identifier_list_property(properties, "requires")
}

pub(crate) fn extract_description_property(properties: &[PropertyAssignment]) -> Option<String> {
    properties
        .iter()
        .find(|p| p.key == "description")
        .and_then(|p| match &p.value {
            ValueExpression::String(s) => Some(s.clone()),
            _ => None,
        })
}

fn extract_identifier_list_property(properties: &[PropertyAssignment], key: &str) -> Vec<String> {
    properties
        .iter()
        .find(|p| p.key == key)
        .and_then(|p| match &p.value {
            ValueExpression::Array(items) => Some(
                items
                    .iter()
                    .filter_map(|v| match v {
                        ValueExpression::Identifier(s) => Some(s.clone()),
                        _ => None,
                    })
                    .collect::<Vec<_>>(),
            ),
            _ => None,
        })
        .unwrap_or_default()
}
