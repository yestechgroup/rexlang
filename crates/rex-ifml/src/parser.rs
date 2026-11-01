use std::path::Path;

use pest::iterators::{Pair, Pairs};
use pest::Parser;
use pest_derive::Parser;

use rex_ir::ifml::*;

/// Errors produced while parsing IFML DSL sources.
#[derive(Debug, thiserror::Error)]
pub enum IfmlParseError {
    #[error("Parse error at {position}: {message}")]
    Parse { position: String, message: String },
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}

#[derive(Parser)]
#[grammar = "grammar/ifml.pest"]
pub struct IfmlParser;

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

fn parse_number(pair: &Pair<Rule>) -> f64 {
    pair.as_str().parse::<f64>().unwrap_or(0.0)
}

fn parse_identifier(pair: &Pair<Rule>) -> String {
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

fn parse_parameter_block(pair: Pair<Rule>) -> Vec<ParameterDecl> {
    let mut params = Vec::new();
    for child in pair.clone().into_inner() {
        if child.as_rule() == Rule::parameter_decl {
            params.push(parse_parameter_decl(child));
        }
    }
    params
}

// ── Value/expression parsing ─────────────────────────────────────
// These two functions are mutually recursive:
//   parse_value_primary   — handles primary: string, number, ident, call, field, group
//   convert_expression_to_value — handles the full operator chain for ValueExpression
// They are defined before parse_value_expression which is the top-level entry.

fn parse_value_primary(pair: Pair<Rule>) -> ValueExpression {
    match pair.as_rule() {
        Rule::string => ValueExpression::String(parse_string(&pair)),
        Rule::number => ValueExpression::Number(parse_number(&pair).into()),
        Rule::boolean => ValueExpression::Bool(pair.as_str() == "true"),
        Rule::identifier => ValueExpression::Identifier(pair.as_str().to_string()),
        Rule::call_expr => {
            let mut inner = pair.clone().into_inner();
            let name = inner
                .next()
                .map(|p| p.as_str().to_string())
                .unwrap_or_default();
            let mut args = Vec::new();
            for arg in inner {
                if arg.as_rule() != Rule::expression {
                    continue;
                }
                args.push(convert_expression_to_value(arg));
            }
            ValueExpression::Call(name, args)
        }
        Rule::field_expr => {
            let mut inner = pair.clone().into_inner();
            let mut object = match inner.next() {
                Some(p) => ValueExpression::Identifier(p.as_str().to_string()),
                None => return ValueExpression::Identifier(String::new()),
            };
            for field_pair in inner {
                object = ValueExpression::FieldAccess {
                    object: Box::new(object),
                    field: field_pair.as_str().to_string(),
                };
            }
            object
        }
        Rule::group_expr => {
            if let Some(inner) = pair.clone().into_inner().next() {
                ValueExpression::Group(Box::new(convert_expression_to_value(inner)))
            } else {
                ValueExpression::Identifier(pair.as_str().to_string())
            }
        }
        Rule::array_literal => {
            let mut elems = Vec::new();
            for child in pair.clone().into_inner() {
                elems.push(parse_value_expression(child));
            }
            ValueExpression::Array(elems)
        }
        _ => ValueExpression::Identifier(pair.as_str().to_string()),
    }
}

fn convert_expression_to_value(pair: Pair<Rule>) -> ValueExpression {
    let rule = pair.as_rule();
    match rule {
        // expression wraps logical_or — delegate
        Rule::expression => {
            if let Some(inner) = pair.clone().into_inner().next() {
                convert_expression_to_value(inner)
            } else {
                ValueExpression::Identifier(pair.as_str().to_string())
            }
        }
        // logical_or: all children are operands with || between them
        Rule::logical_or => {
            let mut children = pair.clone().into_inner();
            if let Some(first) = children.next() {
                let mut result = convert_expression_to_value(first);
                for right in children {
                    let right_val = convert_expression_to_value(right);
                    result = ValueExpression::BinOp {
                        left: Box::new(result),
                        op: BinOp::Or,
                        right: Box::new(right_val),
                    };
                }
                result
            } else {
                ValueExpression::Identifier(pair.as_str().to_string())
            }
        }
        // logical_and: all children are operands with && between them
        Rule::logical_and => {
            let mut children = pair.clone().into_inner();
            if let Some(first) = children.next() {
                let mut result = convert_expression_to_value(first);
                for right in children {
                    let right_val = convert_expression_to_value(right);
                    result = ValueExpression::BinOp {
                        left: Box::new(result),
                        op: BinOp::And,
                        right: Box::new(right_val),
                    };
                }
                result
            } else {
                ValueExpression::Identifier(pair.as_str().to_string())
            }
        }
        // comparison: children are [addition, comparison_op, addition, ...]
        Rule::comparison => {
            let mut children = pair.clone().into_inner();
            if let Some(first) = children.next() {
                let mut result = convert_expression_to_value(first);
                while let Some(op_pair) = children.next() {
                    let op_str = op_pair.as_str();
                    let op = match op_str {
                        "==" => BinOp::Eq,
                        "!=" => BinOp::Ne,
                        "<" => BinOp::Lt,
                        "<=" => BinOp::Le,
                        ">" => BinOp::Gt,
                        ">=" => BinOp::Ge,
                        "~=" => BinOp::RegexMatch,
                        "!~" => BinOp::NegRegex,
                        "+" => BinOp::Add,
                        "-" => BinOp::Sub,
                        "*" => BinOp::Mul,
                        "/" => BinOp::Div,
                        "%" => BinOp::Mod,
                        "&&" => BinOp::And,
                        "||" => BinOp::Or,
                        _ => BinOp::Eq,
                    };
                    if let Some(right) = children.next() {
                        let right_val = convert_expression_to_value(right);
                        result = ValueExpression::BinOp {
                            left: Box::new(result),
                            op,
                            right: Box::new(right_val),
                        };
                    }
                }
                result
            } else {
                ValueExpression::Identifier(pair.as_str().to_string())
            }
        }
        Rule::addition | Rule::multiplication => {
            let mut children = pair.clone().into_inner();
            if let Some(first) = children.next() {
                let mut result = convert_expression_to_value(first);
                while let Some(op_pair) = children.next() {
                    let op_str = op_pair.as_str();
                    let op = match op_str {
                        "+" => BinOp::Add,
                        "-" => BinOp::Sub,
                        "*" => BinOp::Mul,
                        "/" => BinOp::Div,
                        "%" => BinOp::Mod,
                        _ => BinOp::Add,
                    };
                    if let Some(right) = children.next() {
                        let right_val = convert_expression_to_value(right);
                        result = ValueExpression::BinOp {
                            left: Box::new(result),
                            op,
                            right: Box::new(right_val),
                        };
                    }
                }
                result
            } else {
                ValueExpression::Identifier(pair.as_str().to_string())
            }
        }
        Rule::unary => {
            if let Some(inner) = pair.clone().into_inner().next() {
                match inner.as_rule() {
                    Rule::unary_not => {
                        if let Some(operand) = inner.clone().into_inner().next() {
                            ValueExpression::UnaryOp {
                                op: UnaryOp::Not,
                                operand: Box::new(convert_expression_to_value(operand)),
                            }
                        } else {
                            ValueExpression::Identifier(pair.as_str().to_string())
                        }
                    }
                    Rule::unary_neg => {
                        if let Some(operand) = inner.clone().into_inner().next() {
                            ValueExpression::UnaryOp {
                                op: UnaryOp::Neg,
                                operand: Box::new(convert_expression_to_value(operand)),
                            }
                        } else {
                            ValueExpression::Identifier(pair.as_str().to_string())
                        }
                    }
                    Rule::primary => {
                        if let Some(primary_inner) = inner.clone().into_inner().next() {
                            parse_value_primary(primary_inner)
                        } else {
                            ValueExpression::Identifier(inner.as_str().to_string())
                        }
                    }
                    _ => convert_expression_to_value(inner),
                }
            } else {
                ValueExpression::Identifier(pair.as_str().to_string())
            }
        }
        Rule::primary => {
            if let Some(inner) = pair.clone().into_inner().next() {
                parse_value_primary(inner)
            } else {
                ValueExpression::Identifier(pair.as_str().to_string())
            }
        }
        _ => parse_value_primary(pair),
    }
}

fn parse_value_expression(pair: Pair<Rule>) -> ValueExpression {
    if let Some(inner) = pair.clone().into_inner().next() {
        let rule = inner.as_rule();
        match rule {
            Rule::array_literal => {
                let mut elems = Vec::new();
                for child in inner.clone().into_inner() {
                    elems.push(parse_value_expression(child));
                }
                ValueExpression::Array(elems)
            }
            Rule::object_literal => {
                let mut members = Vec::new();
                for child in inner.clone().into_inner() {
                    if child.as_rule() != Rule::object_member {
                        continue;
                    }
                    let mut member_inner = child.into_inner();
                    let key = member_inner
                        .next()
                        .map(|p| p.as_str().to_string())
                        .unwrap_or_default();
                    let value = member_inner
                        .next()
                        .map(parse_value_expression)
                        .unwrap_or(ValueExpression::Identifier("".to_string()));
                    members.push(ObjectMember { key, value });
                }
                ValueExpression::Object(members)
            }
            _ => convert_expression_to_value(inner),
        }
    } else {
        ValueExpression::Identifier(pair.as_str().to_string())
    }
}

fn parse_property_assignment(pair: Pair<Rule>) -> PropertyAssignment {
    let mut inner = pair.clone().into_inner();
    let key = inner
        .next()
        .map(|p| p.as_str().to_string())
        .unwrap_or_default();
    let value = inner
        .next()
        .map(parse_value_expression)
        .unwrap_or(ValueExpression::Identifier("".to_string()));
    let span = Some((pair.as_span().start(), pair.as_span().end()));
    PropertyAssignment { key, value, span }
}

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

// ── Expression parsing (for parameter bindings) ──────────────────

fn fallback_expr(pair: &Pair<Rule>) -> Expression {
    Expression::Ident(pair.as_str().to_string())
}

fn parse_expression(pair: Pair<Rule>) -> Expression {
    let rule = pair.as_rule();
    match rule {
        Rule::expression => {
            if let Some(inner) = pair.clone().into_inner().next() {
                parse_expression(inner)
            } else {
                fallback_expr(&pair)
            }
        }
        Rule::logical_or => {
            let mut children = pair.clone().into_inner();
            if let Some(first) = children.next() {
                let mut result = parse_expression(first);
                for right in children {
                    let right_val = parse_expression(right);
                    result = Expression::BinOp {
                        left: Box::new(result),
                        op: BinOp::Or,
                        right: Box::new(right_val),
                    };
                }
                result
            } else {
                fallback_expr(&pair)
            }
        }
        Rule::logical_and => {
            let mut children = pair.clone().into_inner();
            if let Some(first) = children.next() {
                let mut result = parse_expression(first);
                for right in children {
                    let right_val = parse_expression(right);
                    result = Expression::BinOp {
                        left: Box::new(result),
                        op: BinOp::And,
                        right: Box::new(right_val),
                    };
                }
                result
            } else {
                fallback_expr(&pair)
            }
        }
        Rule::comparison => {
            let mut children = pair.clone().into_inner();
            if let Some(first) = children.next() {
                let mut result = parse_expression(first);
                while let Some(op_pair) = children.next() {
                    let op_str = op_pair.as_str();
                    let op = match op_str {
                        "==" => BinOp::Eq,
                        "!=" => BinOp::Ne,
                        "<" => BinOp::Lt,
                        "<=" => BinOp::Le,
                        ">" => BinOp::Gt,
                        ">=" => BinOp::Ge,
                        "~=" => BinOp::RegexMatch,
                        "!~" => BinOp::NegRegex,
                        "+" => BinOp::Add,
                        "-" => BinOp::Sub,
                        "*" => BinOp::Mul,
                        "/" => BinOp::Div,
                        "%" => BinOp::Mod,
                        "&&" => BinOp::And,
                        "||" => BinOp::Or,
                        _ => BinOp::Eq,
                    };
                    if let Some(right) = children.next() {
                        let right_val = parse_expression(right);
                        result = Expression::BinOp {
                            left: Box::new(result),
                            op,
                            right: Box::new(right_val),
                        };
                    }
                }
                result
            } else {
                fallback_expr(&pair)
            }
        }
        Rule::addition | Rule::multiplication => {
            let mut children = pair.clone().into_inner();
            if let Some(first) = children.next() {
                let mut result = parse_expression(first);
                while let Some(op_pair) = children.next() {
                    let op_str = op_pair.as_str();
                    let op = match op_str {
                        "+" => BinOp::Add,
                        "-" => BinOp::Sub,
                        "*" => BinOp::Mul,
                        "/" => BinOp::Div,
                        "%" => BinOp::Mod,
                        _ => BinOp::Add,
                    };
                    if let Some(right) = children.next() {
                        let right_val = parse_expression(right);
                        result = Expression::BinOp {
                            left: Box::new(result),
                            op,
                            right: Box::new(right_val),
                        };
                    }
                }
                result
            } else {
                fallback_expr(&pair)
            }
        }
        Rule::unary => {
            if let Some(inner) = pair.clone().into_inner().next() {
                match inner.as_rule() {
                    Rule::unary_not => {
                        if let Some(operand) = inner.clone().into_inner().next() {
                            Expression::UnaryOp {
                                op: UnaryOp::Not,
                                operand: Box::new(parse_expression(operand)),
                            }
                        } else {
                            fallback_expr(&pair)
                        }
                    }
                    Rule::unary_neg => {
                        if let Some(operand) = inner.clone().into_inner().next() {
                            Expression::UnaryOp {
                                op: UnaryOp::Neg,
                                operand: Box::new(parse_expression(operand)),
                            }
                        } else {
                            fallback_expr(&pair)
                        }
                    }
                    Rule::primary => {
                        if let Some(primary_inner) = inner.clone().into_inner().next() {
                            parse_primary_expression(primary_inner)
                        } else {
                            Expression::Ident(inner.as_str().to_string())
                        }
                    }
                    _ => parse_expression(inner),
                }
            } else {
                fallback_expr(&pair)
            }
        }
        Rule::primary => {
            if let Some(inner) = pair.clone().into_inner().next() {
                parse_primary_expression(inner)
            } else {
                fallback_expr(&pair)
            }
        }
        Rule::string => Expression::StringLit(parse_string(&pair)),
        Rule::number => Expression::NumLit(parse_number(&pair).into()),
        Rule::boolean => Expression::BoolLit(pair.as_str() == "true"),
        Rule::identifier => Expression::Ident(pair.as_str().to_string()),
        Rule::call_expr => {
            let mut inner = pair.clone().into_inner();
            if let Some(name_pair) = inner.next() {
                let name = name_pair.as_str().to_string();
                let mut args = Vec::new();
                for arg in inner {
                    if arg.as_rule() != Rule::expression {
                        continue;
                    }
                    args.push(parse_expression(arg));
                }
                Expression::Call { name, args }
            } else {
                fallback_expr(&pair)
            }
        }
        Rule::field_expr => {
            let mut inner = pair.clone().into_inner().peekable();
            let Some(object) = inner.next() else {
                return fallback_expr(&pair);
            };
            let mut object = Expression::Ident(object.as_str().to_string());
            for field_pair in inner {
                object = Expression::FieldExpr {
                    object: Box::new(object),
                    field: field_pair.as_str().to_string(),
                };
            }
            if matches!(object, Expression::Ident(_)) {
                fallback_expr(&pair)
            } else {
                object
            }
        }
        Rule::group_expr => {
            if let Some(inner) = pair.clone().into_inner().next() {
                Expression::Group(Box::new(parse_expression(inner)))
            } else {
                fallback_expr(&pair)
            }
        }
        _ => fallback_expr(&pair),
    }
}

fn parse_primary_expression(pair: Pair<Rule>) -> Expression {
    match pair.as_rule() {
        Rule::string => Expression::StringLit(parse_string(&pair)),
        Rule::number => Expression::NumLit(parse_number(&pair).into()),
        Rule::boolean => Expression::BoolLit(pair.as_str() == "true"),
        Rule::identifier => Expression::Ident(pair.as_str().to_string()),
        Rule::call_expr => {
            let mut inner = pair.clone().into_inner();
            if let Some(name_pair) = inner.next() {
                let name = name_pair.as_str().to_string();
                let mut args = Vec::new();
                for arg in inner {
                    if arg.as_rule() != Rule::expression {
                        continue;
                    }
                    args.push(parse_expression(arg));
                }
                Expression::Call { name, args }
            } else {
                Expression::Ident(pair.as_str().to_string())
            }
        }
        Rule::field_expr => {
            let mut inner = pair.clone().into_inner().peekable();
            let Some(object) = inner.next() else {
                return Expression::Ident(String::new());
            };
            let mut object = Expression::Ident(object.as_str().to_string());
            for field_pair in inner {
                object = Expression::FieldExpr {
                    object: Box::new(object),
                    field: field_pair.as_str().to_string(),
                };
            }
            object
        }
        Rule::group_expr => {
            if let Some(inner) = pair.clone().into_inner().next() {
                Expression::Group(Box::new(parse_expression(inner)))
            } else {
                Expression::Ident(pair.as_str().to_string())
            }
        }
        _ => Expression::Ident(pair.as_str().to_string()),
    }
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

fn parse_event_handler(pair: Pair<Rule>) -> EventHandler {
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

// ── Component / Container / View parsing ─────────────────────────

fn empty_property_ref() -> PropertyRef {
    PropertyRef {
        entity: String::new(),
        property: String::new(),
    }
}

fn parse_property_ref(pair: Pair<Rule>) -> PropertyRef {
    let mut inner = pair.into_inner();
    let entity = inner
        .next()
        .map(|p| p.as_str().to_string())
        .unwrap_or_default();
    let property = inner
        .next()
        .map(|p| p.as_str().to_string())
        .unwrap_or_default();
    PropertyRef { entity, property }
}

fn parse_column_kind(pair: Pair<Rule>, label: String) -> ColumnDef {
    match pair.as_rule() {
        Rule::column_field => {
            let field = pair
                .into_inner()
                .next()
                .map(parse_property_ref)
                .unwrap_or_else(empty_property_ref);
            ColumnDef::Field { label, field }
        }
        Rule::column_lookup => {
            let mut inner = pair.into_inner();
            let field = inner
                .next()
                .map(parse_property_ref)
                .unwrap_or_else(empty_property_ref);
            let lookup = inner
                .next()
                .map(|p| p.as_str().to_string())
                .unwrap_or_default();
            ColumnDef::Lookup {
                label,
                field,
                lookup,
            }
        }
        Rule::column_expr => {
            let expr = pair
                .into_inner()
                .next()
                .map(parse_expression)
                .unwrap_or(Expression::Ident(String::new()));
            ColumnDef::Expression { label, expr }
        }
        _ => ColumnDef::Field {
            label,
            field: empty_property_ref(),
        },
    }
}

fn parse_column_decl(pair: Pair<Rule>) -> ColumnDef {
    let mut inner = pair.into_inner();
    let label = inner.next().map(|p| parse_string(&p)).unwrap_or_default();
    match inner.next() {
        Some(kind) => parse_column_kind(kind, label),
        None => ColumnDef::Field {
            label,
            field: empty_property_ref(),
        },
    }
}

fn parse_input_body_property(
    key: &str,
    value: Option<Pair<Rule>>,
    required: &mut bool,
    validations: &mut Vec<Expression>,
    values: &mut Vec<String>,
    messages: &mut Vec<String>,
) {
    match (key, value) {
        ("required", Some(value)) => {
            if let ValueExpression::Bool(b) = parse_value_expression(value) {
                *required = b;
            }
        }
        ("validations", Some(value)) => {
            validations.extend(extract_validation_expressions(value));
        }
        ("values", Some(value)) => {
            *values = extract_string_list(value);
        }
        ("messages", Some(value)) => {
            *messages = extract_string_list(value);
        }
        _ => {}
    }
}

fn extract_string_list(value: Pair<Rule>) -> Vec<String> {
    let mut out = Vec::new();
    if let ValueExpression::Array(items) = parse_value_expression(value) {
        for item in items {
            match item {
                ValueExpression::String(s) => out.push(s),
                ValueExpression::Identifier(id) => out.push(id),
                _ => {}
            }
        }
    }
    out
}

fn extract_validation_expressions(value: Pair<Rule>) -> Vec<Expression> {
    let mut exprs = Vec::new();
    for array in value.into_inner() {
        if array.as_rule() != Rule::array_literal {
            continue;
        }
        for item in array.into_inner() {
            if let Some(inner) = item.into_inner().next() {
                exprs.push(parse_expression(inner));
            }
        }
    }
    exprs
}

fn parse_field_decl(pair: Pair<Rule>) -> Option<FieldDef> {
    let mut inner = pair.into_inner();
    let name = inner.next().map(|p| p.as_str().to_string())?;
    let input = inner
        .next()
        .map(|p| InputFieldType::from(p.as_str()))
        .unwrap_or(InputFieldType::Custom(String::new()));
    let mut required = false;
    let mut validations = Vec::new();
    let mut values = Vec::new();
    let mut messages = Vec::new();
    if let Some(body) = inner.next() {
        for child in body.into_inner() {
            if child.as_rule() != Rule::property_assignment {
                continue;
            }
            let mut prop = child.into_inner();
            let key = prop
                .next()
                .map(|p| p.as_str().to_string())
                .unwrap_or_default();
            parse_input_body_property(
                &key,
                prop.next(),
                &mut required,
                &mut validations,
                &mut values,
                &mut messages,
            );
        }
    }
    Some(FieldDef {
        name,
        input,
        required,
        validations,
        values,
        messages,
    })
}

fn parse_chart_kind(s: &str) -> ChartKind {
    match s.to_ascii_lowercase().as_str() {
        "line" => ChartKind::Line,
        "pie" => ChartKind::Pie,
        "radar" => ChartKind::Radar,
        "metric" => ChartKind::Metric,
        "org" => ChartKind::Org,
        _ => ChartKind::Bar,
    }
}

/// The series-aggregation vocabulary a chart body's `aggregate:` property
/// admits. `aggregate` values are validated post-parse (the grammar's
/// generic `property_assignment` rule cannot express key-dependent value
/// constraints), so unknown aggregates are hard parse errors — the same
/// strictness the grammar gives unknown chart kinds.
fn parse_chart_aggregate(word: &str) -> Option<ChartAggregate> {
    match word.to_ascii_lowercase().as_str() {
        "sum" => Some(ChartAggregate::Sum),
        "avg" => Some(ChartAggregate::Avg),
        "count" => Some(ChartAggregate::Count),
        "min" => Some(ChartAggregate::Min),
        "max" => Some(ChartAggregate::Max),
        _ => None,
    }
}

fn parse_chart_decl(pair: Pair<Rule>) -> Result<Option<ChartSpec>, IfmlParseError> {
    let mut inner = pair.into_inner();
    let Some(kind_pair) = inner.next() else {
        return Ok(None);
    };
    let kind = parse_chart_kind(kind_pair.as_str());
    let mut spec = ChartSpec {
        kind,
        label_field: None,
        value_fields: Vec::new(),
        hierarchy_field: None,
        label_fields: Vec::new(),
        detail_fields: Vec::new(),
        max_depth: None,
        aggregate: None,
    };
    if let Some(body) = inner.next() {
        for child in body.into_inner() {
            if child.as_rule() != Rule::property_assignment {
                continue;
            }
            let line = child.line_col().0;
            let prop = parse_property_assignment(child);
            match &prop.value {
                ValueExpression::Identifier(id) if prop.key == "label" => {
                    spec.label_field = Some(id.clone());
                }
                ValueExpression::Array(items) if prop.key == "values" => {
                    for item in items {
                        if let ValueExpression::Identifier(id) = item {
                            spec.value_fields.push(id.clone());
                        }
                    }
                }
                ValueExpression::Identifier(id) if prop.key == "hierarchy" => {
                    spec.hierarchy_field = Some(id.clone());
                }
                ValueExpression::Array(items) if prop.key == "labels" => {
                    for item in items {
                        if let ValueExpression::Identifier(id) = item {
                            spec.label_fields.push(id.clone());
                        }
                    }
                }
                ValueExpression::Array(items) if prop.key == "details" => {
                    for item in items {
                        if let ValueExpression::Identifier(id) = item {
                            spec.detail_fields.push(id.clone());
                        }
                    }
                }
                ValueExpression::Number(n) if prop.key == "maxDepth" => {
                    spec.max_depth = Some(n.value().max(0.0) as u32);
                }
                ValueExpression::Identifier(id) if prop.key == "aggregate" => {
                    let aggregate =
                        parse_chart_aggregate(id).ok_or_else(|| IfmlParseError::Parse {
                            position: line.to_string(),
                            message: format!(
                                "unknown aggregate `{id}` (expected sum | avg | count | min | max)"
                            ),
                        })?;
                    spec.aggregate = Some(aggregate);
                }
                _ => {}
            }
        }
    }
    Ok(Some(spec))
}

fn extract_component_type(properties: &mut Vec<PropertyAssignment>) -> Option<ComponentType> {
    let mut component_type = None;
    properties.retain(|p| {
        if p.key == "type" && component_type.is_none() {
            if let ValueExpression::Identifier(id) = &p.value {
                component_type = Some(ComponentType::from(id.as_str()));
                return false;
            }
        }
        true
    });
    component_type
}

fn extract_pagination(properties: &mut Vec<PropertyAssignment>) -> bool {
    let mut pagination = true;
    properties.retain(|p| {
        if p.key == "pagination" {
            if let ValueExpression::Bool(b) = p.value {
                pagination = b;
                return false;
            }
        }
        true
    });
    pagination
}

fn extract_chart_ref(properties: &mut Vec<PropertyAssignment>) -> Option<String> {
    let mut chart_ref = None;
    properties.retain(|p| {
        if p.key == "chart" && chart_ref.is_none() {
            if let ValueExpression::String(s) = &p.value {
                chart_ref = Some(s.clone());
                return false;
            }
        }
        true
    });
    chart_ref
}

fn parse_component_declaration(pair: Pair<Rule>) -> Result<ComponentDeclaration, IfmlParseError> {
    let component_line = pair.line_col().0;
    let mut inner = pair.clone().into_inner();
    let name = inner.next().map(|p| parse_string(&p)).unwrap_or_default();
    let body = match inner.next() {
        Some(b) => b,
        None => {
            return Ok(ComponentDeclaration {
                name,
                component_type: None,
                spec: None,
                properties: Vec::new(),
                events: Vec::new(),
                condition: None,
            });
        }
    };

    let mut properties = Vec::new();
    let mut columns = Vec::new();
    let mut fields = Vec::new();
    let mut chart_spec = None;
    let mut events = Vec::new();
    let mut condition = None;
    for child in body.into_inner() {
        match child.as_rule() {
            Rule::property_assignment => properties.push(parse_property_assignment(child)),
            Rule::column_decl => columns.push(parse_column_decl(child)),
            Rule::field_decl => {
                if let Some(field) = parse_field_decl(child) {
                    fields.push(field);
                }
            }
            Rule::chart_decl => {
                if let Some(chart) = parse_chart_decl(child)? {
                    chart_spec = Some(chart);
                }
            }
            Rule::event_handler => events.push(parse_event_handler(child)),
            Rule::condition_statement => {
                condition = child.into_inner().next().map(parse_expression);
            }
            _ => {}
        }
    }

    let mut component_type = extract_component_type(&mut properties);
    let chart_ref = extract_chart_ref(&mut properties);
    let mut spec = None;

    let columns_allowed = component_type.is_none() || component_type == Some(ComponentType::Table);
    if !columns.is_empty() && columns_allowed {
        if component_type.is_none() {
            component_type = Some(ComponentType::Table);
        }
        let pagination = extract_pagination(&mut properties);
        spec = Some(ComponentSpec::Table(TableSpec {
            columns,
            pagination,
        }));
    }

    let fields_allowed = component_type.is_none() || component_type == Some(ComponentType::Form);
    if spec.is_none() && !fields.is_empty() && fields_allowed {
        if component_type.is_none() {
            component_type = Some(ComponentType::Form);
        }
        spec = Some(ComponentSpec::Form(FormSpec { fields }));
    }

    let chart_allowed = component_type.is_none() || component_type == Some(ComponentType::Chart);
    if let Some(reference) = chart_ref {
        if chart_spec.is_some() {
            return Err(IfmlParseError::Parse {
                position: component_line.to_string(),
                message: format!(
                    "component \"{name}\": `chart: \"{reference}\"` cannot be combined with an inline chart declaration"
                ),
            });
        }
        if chart_allowed {
            if component_type.is_none() {
                component_type = Some(ComponentType::Chart);
            }
            spec = Some(ComponentSpec::ChartRef(reference));
        }
    }
    if spec.is_none() {
        if let Some(chart) = chart_spec {
            if chart_allowed {
                if component_type.is_none() {
                    component_type = Some(ComponentType::Chart);
                }
                spec = Some(ComponentSpec::Chart(chart));
            }
        }
    }

    Ok(ComponentDeclaration {
        name,
        component_type,
        spec,
        properties,
        events,
        condition,
    })
}

fn parse_module_use_statement(pair: Pair<Rule>) -> ModuleUse {
    let mut module = String::new();
    let mut alias = None;
    let mut properties = Vec::new();
    for child in pair.into_inner() {
        match child.as_rule() {
            Rule::string => module = parse_string(&child),
            Rule::identifier => alias = Some(child.as_str().to_string()),
            Rule::module_use_body => {
                for body_child in child.into_inner() {
                    if body_child.as_rule() == Rule::property_assignment {
                        properties.push(parse_property_assignment(body_child));
                    }
                }
            }
            _ => {}
        }
    }
    ModuleUse {
        module,
        alias,
        properties,
    }
}

type ViewBodyParts = (
    Vec<ParameterDecl>,
    Option<String>,
    Vec<PropertyAssignment>,
    Vec<ContainerDeclaration>,
    Vec<ComponentDeclaration>,
    Vec<EventHandler>,
    Vec<ModuleUse>,
    Option<Expression>,
);

fn parse_view_body_content(pair: Pair<Rule>) -> Result<ViewBodyParts, IfmlParseError> {
    let mut params = Vec::new();
    let mut label = None;
    let mut properties = Vec::new();
    let mut containers = Vec::new();
    let mut components = Vec::new();
    let mut events = Vec::new();
    let mut module_uses = Vec::new();
    let mut condition = None;

    for child in pair.clone().into_inner() {
        match child.as_rule() {
            Rule::params_block => {
                if let Some(block) = child.into_inner().next() {
                    params = parse_parameter_block(block);
                }
            }
            Rule::label_declaration => {
                if let Some(s) = child.into_inner().next() {
                    label = Some(parse_string(&s));
                }
            }
            Rule::property_assignment => properties.push(parse_property_assignment(child)),
            Rule::container_declaration => containers.push(parse_container_declaration(child)?),
            Rule::component_declaration => components.push(parse_component_declaration(child)?),
            Rule::event_handler => events.push(parse_event_handler(child)),
            Rule::module_use_statement => module_uses.push(parse_module_use_statement(child)),
            Rule::condition_statement => {
                condition = child.into_inner().next().map(parse_expression);
            }
            _ => {}
        }
    }

    Ok((
        params,
        label,
        properties,
        containers,
        components,
        events,
        module_uses,
        condition,
    ))
}

type ContainerBodyParts = (
    Vec<ParameterDecl>,
    Option<String>,
    Vec<PropertyAssignment>,
    Vec<ContainerDeclaration>,
    Vec<ComponentDeclaration>,
    Vec<EventHandler>,
    Vec<ModuleUse>,
    Option<Expression>,
);

fn parse_container_body_content(pair: Pair<Rule>) -> Result<ContainerBodyParts, IfmlParseError> {
    let mut params = Vec::new();
    let mut label = None;
    let mut properties = Vec::new();
    let mut containers = Vec::new();
    let mut components = Vec::new();
    let mut events = Vec::new();
    let mut module_uses = Vec::new();
    let mut condition = None;

    for child in pair.clone().into_inner() {
        match child.as_rule() {
            Rule::params_block => {
                if let Some(block) = child.into_inner().next() {
                    params = parse_parameter_block(block);
                }
            }
            Rule::label_declaration => {
                if let Some(s) = child.into_inner().next() {
                    label = Some(parse_string(&s));
                }
            }
            Rule::property_assignment => properties.push(parse_property_assignment(child)),
            Rule::container_declaration => containers.push(parse_container_declaration(child)?),
            Rule::component_declaration => components.push(parse_component_declaration(child)?),
            Rule::event_handler => events.push(parse_event_handler(child)),
            Rule::module_use_statement => module_uses.push(parse_module_use_statement(child)),
            Rule::condition_statement => {
                condition = child.into_inner().next().map(parse_expression);
            }
            _ => {}
        }
    }

    Ok((
        params,
        label,
        properties,
        containers,
        components,
        events,
        module_uses,
        condition,
    ))
}

fn parse_container_declaration(pair: Pair<Rule>) -> Result<ContainerDeclaration, IfmlParseError> {
    let mut inner = pair.clone().into_inner();
    let name = inner.next().map(|p| parse_string(&p)).unwrap_or_default();
    let body = if let Some(b) = inner.next() {
        b
    } else {
        return Ok(ContainerDeclaration {
            name,
            label: None,
            is_default: false,
            is_xor: false,
            params: Vec::new(),
            properties: Vec::new(),
            containers: Vec::new(),
            components: Vec::new(),
            events: Vec::new(),
            module_uses: Vec::new(),
            condition: None,
            position: None,
        });
    };

    let (params, label, properties, containers, components, events, module_uses, condition) =
        parse_container_body_content(body)?;

    let is_default = extract_bool_property(&properties, "default");
    let is_xor = extract_bool_property(&properties, "xor");

    let position = extract_position_property(&properties);

    Ok(ContainerDeclaration {
        name,
        label,
        is_default,
        is_xor,
        params,
        properties,
        containers,
        components,
        events,
        module_uses,
        condition,
        position,
    })
}

fn extract_bool_property(properties: &[PropertyAssignment], key: &str) -> bool {
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

fn extract_position_property(properties: &[PropertyAssignment]) -> Option<Position> {
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

fn extract_roles_property(properties: &[PropertyAssignment]) -> Vec<String> {
    extract_identifier_list_property(properties, "roles")
}

fn extract_requires_property(properties: &[PropertyAssignment]) -> Vec<String> {
    extract_identifier_list_property(properties, "requires")
}

fn extract_description_property(properties: &[PropertyAssignment]) -> Option<String> {
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

fn parse_view_declaration(pair: Pair<Rule>) -> Result<ViewDeclaration, IfmlParseError> {
    let mut inner = pair.clone().into_inner();
    let name = inner.next().map(|p| parse_string(&p)).unwrap_or_default();
    let body = if let Some(b) = inner.next() {
        b
    } else {
        return Ok(ViewDeclaration {
            name,
            label: None,
            is_landmark: false,
            is_xor: false,
            is_modal: false,
            params: Vec::new(),
            properties: Vec::new(),
            containers: Vec::new(),
            components: Vec::new(),
            events: Vec::new(),
            module_uses: Vec::new(),
            roles: Vec::new(),
            requires: Vec::new(),
            condition: None,
            position: None,
        });
    };

    let (params, label, properties, containers, components, events, module_uses, condition) =
        parse_view_body_content(body)?;

    let is_landmark = extract_bool_property(&properties, "landmark");
    let is_xor = extract_bool_property(&properties, "xor");
    let is_modal = extract_bool_property(&properties, "modal");
    let position = extract_position_property(&properties);
    let roles = extract_roles_property(&properties);
    let requires = extract_requires_property(&properties);
    let properties: Vec<PropertyAssignment> = properties
        .into_iter()
        .filter(|p| p.key != "roles" && p.key != "requires")
        .collect();

    Ok(ViewDeclaration {
        name,
        label,
        is_landmark,
        is_xor,
        is_modal,
        params,
        properties,
        containers,
        components,
        events,
        module_uses,
        roles,
        requires,
        condition,
        position,
    })
}

fn parse_action_declaration(pair: Pair<Rule>) -> ActionDeclaration {
    let mut inner = pair.clone().into_inner();
    let name = inner.next().map(|p| parse_string(&p)).unwrap_or_default();

    let mut properties = Vec::new();
    let mut events = Vec::new();

    for child in inner {
        match child.as_rule() {
            Rule::property_assignment => properties.push(parse_property_assignment(child)),
            Rule::event_handler => events.push(parse_event_handler(child)),
            _ => {}
        }
    }

    ActionDeclaration {
        name,
        properties,
        events,
    }
}

fn parse_module_declaration(pair: Pair<Rule>) -> Result<ModuleDeclaration, IfmlParseError> {
    let mut inner = pair.clone().into_inner();
    let name = inner.next().map(|p| parse_string(&p)).unwrap_or_default();

    let mut input_params = Vec::new();
    let mut output_params = Vec::new();
    let mut properties = Vec::new();
    let mut containers = Vec::new();
    let mut components = Vec::new();
    let mut events = Vec::new();
    let mut module_uses = Vec::new();

    for child in inner {
        match child.as_rule() {
            Rule::parameter_block => {
                if input_params.is_empty() {
                    input_params = parse_parameter_block(child);
                } else {
                    output_params = parse_parameter_block(child);
                }
            }
            Rule::property_assignment => properties.push(parse_property_assignment(child)),
            Rule::container_declaration => containers.push(parse_container_declaration(child)?),
            Rule::component_declaration => components.push(parse_component_declaration(child)?),
            Rule::event_handler => events.push(parse_event_handler(child)),
            Rule::module_use_statement => module_uses.push(parse_module_use_statement(child)),
            _ => {}
        }
    }

    Ok(ModuleDeclaration {
        name,
        input_params,
        output_params,
        properties,
        containers,
        components,
        events,
        module_uses,
    })
}

fn parse_domain_declaration(pair: Pair<Rule>) -> DomainDeclaration {
    let mut inner = pair.clone().into_inner();
    let name = inner.next().map(|p| parse_string(&p)).unwrap_or_default();
    let schema_name = inner.next().map(|p| parse_string(&p)).unwrap_or_default();
    DomainDeclaration { name, schema_name }
}

fn parse_actor_declaration(pair: Pair<Rule>) -> ActorDeclaration {
    let mut inner = pair.clone().into_inner();
    let name = inner.next().map(|p| parse_string(&p)).unwrap_or_default();

    let mut properties = Vec::new();
    for child in inner {
        if child.as_rule() == Rule::property_assignment {
            properties.push(parse_property_assignment(child));
        }
    }

    ActorDeclaration { name, properties }
}

fn parse_argument_decl(pair: Pair<Rule>) -> CommandArgument {
    let mut inner = pair.clone().into_inner();
    let name = inner
        .next()
        .map(|p| parse_identifier(&p))
        .unwrap_or_default();
    let type_ref = inner
        .next()
        .map(|p| parse_identifier(&p))
        .unwrap_or_default();
    CommandArgument { name, type_ref }
}

fn parse_option_decl(pair: Pair<Rule>) -> CommandOption {
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
    CommandOption {
        name,
        type_ref,
        default,
    }
}

fn parse_prompt_decl(pair: Pair<Rule>) -> CommandPrompt {
    let mut inner = pair.clone().into_inner();
    let name = inner
        .next()
        .map(|p| parse_identifier(&p))
        .unwrap_or_default();
    let kind = inner
        .next()
        .map(|p| PromptKind::from(p.as_str()))
        .unwrap_or(PromptKind::Text);
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
    CommandPrompt {
        name,
        kind,
        default,
    }
}

/// Lowers one `command` declaration, recursively through subcommands.
/// `description` (string) and `requires` (identifier list) are extracted
/// from the property bag — the view discipline — and the remaining bag
/// entries stay in `properties`.
fn parse_command_declaration(pair: Pair<Rule>) -> CommandDeclaration {
    let mut inner = pair.clone().into_inner();
    let name = inner.next().map(|p| parse_string(&p)).unwrap_or_default();

    let mut arguments = Vec::new();
    let mut options = Vec::new();
    let mut prompts = Vec::new();
    let mut events = Vec::new();
    let mut module_uses = Vec::new();
    let mut subcommands = Vec::new();
    let mut properties = Vec::new();
    let mut condition = None;

    let body_members = inner
        .next()
        .map(|body| body.into_inner().collect::<Vec<_>>())
        .unwrap_or_default();
    for child in body_members {
        match child.as_rule() {
            Rule::property_assignment => properties.push(parse_property_assignment(child)),
            Rule::argument_decl => arguments.push(parse_argument_decl(child)),
            Rule::option_decl => options.push(parse_option_decl(child)),
            Rule::prompt_decl => prompts.push(parse_prompt_decl(child)),
            Rule::event_handler => events.push(parse_event_handler(child)),
            Rule::module_use_statement => module_uses.push(parse_module_use_statement(child)),
            Rule::condition_statement => {
                condition = child.into_inner().next().map(parse_expression);
            }
            Rule::command_declaration => subcommands.push(parse_command_declaration(child)),
            _ => {}
        }
    }

    let description = extract_description_property(&properties);
    let requires = extract_requires_property(&properties);
    let properties: Vec<PropertyAssignment> = properties
        .into_iter()
        .filter(|p| p.key != "description" && p.key != "requires")
        .collect();

    CommandDeclaration {
        name,
        description,
        arguments,
        options,
        prompts,
        events,
        module_uses,
        subcommands,
        properties,
        requires,
        condition,
    }
}

fn parse_import_declaration(pair: Pair<Rule>) -> String {
    let mut inner = pair.into_inner();
    inner.next().map(|p| parse_string(&p)).unwrap_or_default()
}

pub(crate) fn parse_ifml_model(pairs: Pairs<Rule>) -> Result<IfmlModel, IfmlParseError> {
    let mut domains = Vec::new();
    let mut views = Vec::new();
    let mut actions = Vec::new();
    let mut modules = Vec::new();
    let mut actors = Vec::new();
    let mut commands = Vec::new();
    let mut imports = Vec::new();

    for pair in pairs {
        let rule = pair.as_rule();
        match rule {
            Rule::domain_declaration => domains.push(parse_domain_declaration(pair)),
            Rule::view_declaration => views.push(parse_view_declaration(pair)?),
            Rule::action_declaration => actions.push(parse_action_declaration(pair)),
            Rule::module_declaration => modules.push(parse_module_declaration(pair)?),
            Rule::actor_declaration => actors.push(parse_actor_declaration(pair)),
            Rule::command_declaration => commands.push(parse_command_declaration(pair)),
            Rule::import_declaration => imports.push(parse_import_declaration(pair)),
            Rule::EOI => {}
            _ => {
                return Err(IfmlParseError::Parse {
                    position: pair.line_col().0.to_string(),
                    message: format!("Unexpected rule: {:?}", rule),
                });
            }
        }
    }

    Ok(IfmlModel::new(
        domains, views, actions, modules, actors, commands, imports,
    ))
}

/// Parses `input` and returns the top-level `ifml_model` pair (`None` for
/// empty input), the shared entry for the model lowering and the span index
/// walk.
pub(crate) fn parse_ifml_top_pair(input: &str) -> Result<Option<Pair<'_, Rule>>, IfmlParseError> {
    let parsed = IfmlParser::parse(Rule::ifml_model, input).map_err(|e| IfmlParseError::Parse {
        position: "unknown".to_string(),
        message: format!("{}", e),
    })?;

    let top_level_pairs: Vec<Pair<Rule>> = parsed.collect();
    if top_level_pairs.is_empty() {
        return Ok(None);
    }

    let top = &top_level_pairs[0];
    if top.as_rule() != Rule::ifml_model {
        return Err(IfmlParseError::Parse {
            position: top.line_col().0.to_string(),
            message: format!("Expected ifml_model, got {:?}", top.as_rule()),
        });
    }

    Ok(Some(top.clone()))
}

/// Parse an IFML DSL string into an AST model.
pub fn parse_ifml(input: &str) -> Result<IfmlModel, IfmlParseError> {
    match parse_ifml_top_pair(input)? {
        None => Ok(IfmlModel::default()),
        Some(top) => parse_ifml_model(top.into_inner()),
    }
}

/// Parse an IFML DSL file into an AST model.
pub fn parse_ifml_file(path: &Path) -> Result<IfmlModel, IfmlParseError> {
    let input = std::fs::read_to_string(path).map_err(IfmlParseError::Io)?;
    parse_ifml(&input)
}
