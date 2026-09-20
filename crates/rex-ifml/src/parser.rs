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

fn parse_string(pair: &Pair<Rule>) -> String {
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
            Rule::number => ValueExpression::Number(parse_number(&p)),
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
        Rule::number => ValueExpression::Number(parse_number(&pair)),
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
            let object = inner
                .next()
                .map(|p| p.as_str().to_string())
                .unwrap_or_default();
            let field = inner
                .next()
                .map(|p| p.as_str().to_string())
                .unwrap_or_default();
            ValueExpression::FieldAccess {
                object: Box::new(ValueExpression::Identifier(object)),
                field,
            }
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
        Rule::number => Expression::NumLit(parse_number(&pair)),
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
            let mut inner = pair.clone().into_inner();
            if let Some(obj_pair) = inner.next() {
                let object = obj_pair.as_str().to_string();
                if let Some(field_pair) = inner.next() {
                    let field = field_pair.as_str().to_string();
                    Expression::FieldExpr {
                        object: Box::new(Expression::Ident(object)),
                        field,
                    }
                } else {
                    Expression::Ident(object)
                }
            } else {
                fallback_expr(&pair)
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
        Rule::number => Expression::NumLit(parse_number(&pair)),
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
            let mut inner = pair.clone().into_inner();
            if let Some(obj_pair) = inner.next() {
                let object = obj_pair.as_str().to_string();
                if let Some(field_pair) = inner.next() {
                    let field = field_pair.as_str().to_string();
                    Expression::FieldExpr {
                        object: Box::new(Expression::Ident(object)),
                        field,
                    }
                } else {
                    Expression::Ident(object)
                }
            } else {
                Expression::Ident(pair.as_str().to_string())
            }
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
        _ => ChartKind::Bar,
    }
}

fn parse_chart_decl(pair: Pair<Rule>) -> Option<ChartSpec> {
    let mut inner = pair.into_inner();
    let kind = parse_chart_kind(inner.next()?.as_str());
    let mut label_field = None;
    let mut value_fields = Vec::new();
    if let Some(body) = inner.next() {
        for child in body.into_inner() {
            if child.as_rule() != Rule::property_assignment {
                continue;
            }
            let prop = parse_property_assignment(child);
            match &prop.value {
                ValueExpression::Identifier(id) if prop.key == "label" => {
                    label_field = Some(id.clone());
                }
                ValueExpression::Array(items) if prop.key == "values" => {
                    for item in items {
                        if let ValueExpression::Identifier(id) = item {
                            value_fields.push(id.clone());
                        }
                    }
                }
                _ => {}
            }
        }
    }
    Some(ChartSpec {
        kind,
        label_field,
        value_fields,
    })
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

fn parse_component_declaration(pair: Pair<Rule>) -> ComponentDeclaration {
    let mut inner = pair.clone().into_inner();
    let name = inner.next().map(|p| parse_string(&p)).unwrap_or_default();
    let body = match inner.next() {
        Some(b) => b,
        None => {
            return ComponentDeclaration {
                name,
                component_type: None,
                spec: None,
                properties: Vec::new(),
                events: Vec::new(),
                condition: None,
            };
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
                if let Some(chart) = parse_chart_decl(child) {
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

    ComponentDeclaration {
        name,
        component_type,
        spec,
        properties,
        events,
        condition,
    }
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

fn parse_view_body_content(pair: Pair<Rule>) -> ViewBodyParts {
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
            Rule::container_declaration => containers.push(parse_container_declaration(child)),
            Rule::component_declaration => components.push(parse_component_declaration(child)),
            Rule::event_handler => events.push(parse_event_handler(child)),
            Rule::module_use_statement => module_uses.push(parse_module_use_statement(child)),
            Rule::condition_statement => {
                condition = child.into_inner().next().map(parse_expression);
            }
            _ => {}
        }
    }

    (
        params,
        label,
        properties,
        containers,
        components,
        events,
        module_uses,
        condition,
    )
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

fn parse_container_body_content(pair: Pair<Rule>) -> ContainerBodyParts {
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
            Rule::container_declaration => containers.push(parse_container_declaration(child)),
            Rule::component_declaration => components.push(parse_component_declaration(child)),
            Rule::event_handler => events.push(parse_event_handler(child)),
            Rule::module_use_statement => module_uses.push(parse_module_use_statement(child)),
            Rule::condition_statement => {
                condition = child.into_inner().next().map(parse_expression);
            }
            _ => {}
        }
    }

    (
        params,
        label,
        properties,
        containers,
        components,
        events,
        module_uses,
        condition,
    )
}

fn parse_container_declaration(pair: Pair<Rule>) -> ContainerDeclaration {
    let mut inner = pair.clone().into_inner();
    let name = inner.next().map(|p| parse_string(&p)).unwrap_or_default();
    let body = if let Some(b) = inner.next() {
        b
    } else {
        return ContainerDeclaration {
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
        };
    };

    let (params, label, properties, containers, components, events, module_uses, condition) =
        parse_container_body_content(body);

    let is_default = extract_bool_property(&properties, "default");
    let is_xor = extract_bool_property(&properties, "xor");

    let position = extract_position_property(&properties);

    ContainerDeclaration {
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
    }
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
            ValueExpression::Number(n) => Some(*n),
            ValueExpression::UnaryOp {
                op: UnaryOp::Neg,
                operand,
            } => match operand.as_ref() {
                ValueExpression::Number(n) => Some(-*n),
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
        (Some(x), Some(y)) => Some(Position { x, y }),
        _ => None,
    }
}

fn extract_roles_property(properties: &[PropertyAssignment]) -> Vec<String> {
    extract_identifier_list_property(properties, "roles")
}

fn extract_requires_property(properties: &[PropertyAssignment]) -> Vec<String> {
    extract_identifier_list_property(properties, "requires")
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

fn parse_view_declaration(pair: Pair<Rule>) -> ViewDeclaration {
    let mut inner = pair.clone().into_inner();
    let name = inner.next().map(|p| parse_string(&p)).unwrap_or_default();
    let body = if let Some(b) = inner.next() {
        b
    } else {
        return ViewDeclaration {
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
        };
    };

    let (params, label, properties, containers, components, events, module_uses, condition) =
        parse_view_body_content(body);

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

    ViewDeclaration {
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
    }
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

fn parse_module_declaration(pair: Pair<Rule>) -> ModuleDeclaration {
    let mut inner = pair.clone().into_inner();
    let name = inner.next().map(|p| parse_string(&p)).unwrap_or_default();

    let mut input_params = Vec::new();
    let mut output_params = Vec::new();
    let mut properties = Vec::new();
    let mut containers = Vec::new();
    let mut components = Vec::new();
    let mut events = Vec::new();

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
            Rule::container_declaration => containers.push(parse_container_declaration(child)),
            Rule::component_declaration => components.push(parse_component_declaration(child)),
            Rule::event_handler => events.push(parse_event_handler(child)),
            _ => {}
        }
    }

    ModuleDeclaration {
        name,
        input_params,
        output_params,
        properties,
        containers,
        components,
        events,
    }
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

fn parse_import_declaration(pair: Pair<Rule>) -> String {
    let mut inner = pair.into_inner();
    inner.next().map(|p| parse_string(&p)).unwrap_or_default()
}

fn parse_ifml_model(pairs: Pairs<Rule>) -> Result<IfmlModel, IfmlParseError> {
    let mut domains = Vec::new();
    let mut views = Vec::new();
    let mut actions = Vec::new();
    let mut modules = Vec::new();
    let mut actors = Vec::new();
    let mut imports = Vec::new();

    for pair in pairs {
        let rule = pair.as_rule();
        match rule {
            Rule::domain_declaration => domains.push(parse_domain_declaration(pair)),
            Rule::view_declaration => views.push(parse_view_declaration(pair)),
            Rule::action_declaration => actions.push(parse_action_declaration(pair)),
            Rule::module_declaration => modules.push(parse_module_declaration(pair)),
            Rule::actor_declaration => actors.push(parse_actor_declaration(pair)),
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
        domains, views, actions, modules, actors, imports,
    ))
}

/// Parse an IFML DSL string into an AST model.
pub fn parse_ifml(input: &str) -> Result<IfmlModel, IfmlParseError> {
    let parsed = IfmlParser::parse(Rule::ifml_model, input).map_err(|e| IfmlParseError::Parse {
        position: "unknown".to_string(),
        message: format!("{}", e),
    })?;

    let top_level_pairs: Vec<Pair<Rule>> = parsed.collect();
    if top_level_pairs.is_empty() {
        return Ok(IfmlModel::default());
    }

    let top = &top_level_pairs[0];
    if top.as_rule() != Rule::ifml_model {
        return Err(IfmlParseError::Parse {
            position: top.line_col().0.to_string(),
            message: format!("Expected ifml_model, got {:?}", top.as_rule()),
        });
    }

    parse_ifml_model(top.clone().into_inner())
}

/// Parse an IFML DSL file into an AST model.
pub fn parse_ifml_file(path: &Path) -> Result<IfmlModel, IfmlParseError> {
    let input = std::fs::read_to_string(path).map_err(IfmlParseError::Io)?;
    parse_ifml(&input)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_minimal_view() {
        let input = r#"
view "Minimal" {
    component "grid" {
        type: list;
        data: Customer;
        fields: [name, email];
    }
}
"#;
        let model = parse_ifml(input).unwrap();
        assert_eq!(model.views.len(), 1);
        assert_eq!(model.views[0].name, "Minimal");
        assert_eq!(model.views[0].components.len(), 1);
        assert_eq!(model.views[0].components[0].name, "grid");
    }

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
    fn test_view_with_params() {
        let input = r#"
view "Detail" {
    params { id: Uuid, mode: String };
    component "info" {
        type: details;
        data: Customer;
    }
}
"#;
        let model = parse_ifml(input).unwrap();
        assert_eq!(model.views.len(), 1);
        let view = &model.views[0];
        assert_eq!(view.params.len(), 2);
        assert_eq!(view.params[0].name, "id");
        assert_eq!(view.params[0].type_ref, "Uuid");
        assert_eq!(view.params[1].name, "mode");
        assert_eq!(view.params[1].type_ref, "String");
    }

    #[test]
    fn test_container_with_navigation() {
        let input = r#"
view "Wizard" {
    xor: true;

    container "Step1" {
        default: true;

        component "form" {
            type: form;
            data: Customer;

            on submit(values) -> navigate("Step2", {
                name: values.name
            });
        }
    }
}
"#;
        let model = parse_ifml(input).unwrap();
        assert_eq!(model.views.len(), 1);
        let view = &model.views[0];
        assert!(view.is_xor);
        assert_eq!(view.containers.len(), 1);
        let container = &view.containers[0];
        assert_eq!(container.name, "Step1");
        assert!(container.is_default);
        assert_eq!(container.components.len(), 1);
        assert_eq!(container.components[0].name, "form");
        assert_eq!(container.components[0].events.len(), 1);

        let event = &container.components[0].events[0];
        assert_eq!(event.event_type, EventType::Submit);
        assert_eq!(event.params, vec!["values"]);
        match &event.action {
            EventAction::Navigate { target, binding } => {
                assert_eq!(target, "Step2");
                assert!(binding.is_some());
                assert_eq!(binding.as_ref().unwrap().pairs[0].0, "name");
            }
            _ => panic!("Expected Navigate action"),
        }
    }

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
    fn test_modal_view() {
        let input = r#"
view "DeleteConfirm" {
    modal: true;

    component "form" {
        type: form;
        data: ConfirmDelete;

        on cancel -> navigate("ProductList");
    }
}
"#;
        let model = parse_ifml(input).unwrap();
        let view = &model.views[0];
        assert!(view.is_modal);
        assert!(!view.is_landmark);
        assert!(!view.is_xor);
    }

    #[test]
    fn test_complex_view_with_label() {
        let input = r#"
view "Dashboard" {
    label "Analytics Dashboard";
    landmark: true;

    on load -> refresh("recentOrders");
    on load -> refresh("topProducts");

    component "recentOrders" {
        type: list;
        data: Order;
        fields: [id, customerName, total, date];
        filter: date == today();
    }
}
"#;
        let model = parse_ifml(input).unwrap();
        let view = &model.views[0];
        assert!(view.is_landmark);
        assert_eq!(view.label.as_deref(), Some("Analytics Dashboard"));
        assert_eq!(view.events.len(), 2);
        assert_eq!(view.components.len(), 1);
    }

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
    fn test_search_page() {
        let input = r#"
view "SearchPage" {
    component "searchForm" {
        type: form;
        data: SearchQuery;

        on submit(values) -> navigate("SearchPage", {
            query: values.term
        });
    }

    component "results" {
        type: list;
        data: Product;
        fields: [name, sku, price];
        filter: name ~= params.query;

        on select(row) -> navigate("ProductDetail", {
            productId: row.id
        });
    }
}
"#;
        let model = parse_ifml(input).unwrap();
        let view = &model.views[0];
        assert_eq!(view.components.len(), 2);

        let form = &view.components[0];
        let submit_event = &form.events[0];
        assert_eq!(submit_event.params, vec!["values"]);
        match &submit_event.action {
            EventAction::Navigate { target, binding } => {
                assert_eq!(target, "SearchPage");
                assert!(binding.is_some());
                assert_eq!(binding.as_ref().unwrap().pairs[0].0, "query");
            }
            _ => panic!("Expected Navigate"),
        }

        let results = &view.components[1];
        let select_event = &results.events[0];
        assert_eq!(select_event.params, vec!["row"]);
        match &select_event.action {
            EventAction::Navigate { target, binding } => {
                assert_eq!(target, "ProductDetail");
                assert!(binding.is_some());
                assert_eq!(binding.as_ref().unwrap().pairs[0].0, "productId");
            }
            _ => panic!("Expected Navigate"),
        }
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
        assert_eq!(cl.position, Some(Position { x: 100.0, y: 200.0 }));

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
    fn test_position_parses_float_and_negative() {
        let input = r#"
view "Pos" {
    position: { x: -12.5; y: 240 };
    component "c" {
        type: list;
        data: Item;
    }
}

view "Pos2" {
    container "c1" {
        position: { x: 30.75; y: -5 };
    }
}
"#;
        let model = parse_ifml(input).unwrap();
        assert_eq!(model.views.len(), 2);

        let view = &model.views[0];
        assert_eq!(view.position, Some(Position { x: -12.5, y: 240.0 }));
        assert_eq!(view.properties.len(), 1);
        assert_eq!(view.properties[0].key, "position");

        let container = &model.views[1].containers[0];
        assert_eq!(container.position, Some(Position { x: 30.75, y: -5.0 }));
    }

    #[test]
    fn test_position_missing_is_none() {
        let input = r#"
view "NoPos" {
    landmark: true;
    component "c" {
        type: list;
        data: Item;
    }
}
"#;
        let model = parse_ifml(input).unwrap();
        assert_eq!(model.views[0].position, None);
    }

    #[test]
    fn test_position_trailing_inner_semicolon() {
        let input = r#"
view "Pos" {
    position: { x: 10; y: 20; };
    component "c" {
        type: list;
        data: Item;
    }
}
"#;
        let model = parse_ifml(input).unwrap();
        assert_eq!(model.views[0].position, Some(Position { x: 10.0, y: 20.0 }));
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

    #[test]
    fn test_position_incomplete_is_none() {
        let input = r#"
view "Partial" {
    position: { x: 10 };
    component "c" {
        type: list;
        data: Item;
    }
}
"#;
        let model = parse_ifml(input).unwrap();
        assert_eq!(model.views[0].position, None);
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
    fn test_container_label_xor_and_nesting() {
        let input = r#"
view "Checkout" {
    container "Shipping" {
        label "Shipping";
        xor: true;

        container "Address" {
            label "Address";
            default: true;

            component "form" {
                type: form;
                data: Customer;
            }
        }
    }
}
"#;
        let model = parse_ifml(input).unwrap();
        let shipping = &model.views[0].containers[0];
        assert_eq!(shipping.label.as_deref(), Some("Shipping"));
        assert!(shipping.is_xor);
        assert!(!shipping.is_default);
        assert_eq!(shipping.containers.len(), 1);
        let address = &shipping.containers[0];
        assert_eq!(address.name, "Address");
        assert_eq!(address.label.as_deref(), Some("Address"));
        assert!(address.is_default);
        assert!(!address.is_xor);
        assert_eq!(address.components.len(), 1);
    }

    #[test]
    fn test_container_without_label_defaults() {
        let input = r#"
view "V" {
    container "Plain" {
        component "grid" {
            type: list;
            data: Customer;
        }
    }
}
"#;
        let model = parse_ifml(input).unwrap();
        let container = &model.views[0].containers[0];
        assert_eq!(container.label, None);
        assert!(!container.is_xor);
        assert!(!container.is_default);
        assert!(container.containers.is_empty());
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

    #[test]
    fn test_param_defaults_parse() {
        let input = r#"
view "Catalog" {
    params { id: Uuid, slug: String = "home", page: Int = 1, verbose: Boolean = true };

    component "grid" {
        type: list;
        data: Product;
    }
}

module "Pager" {
    input { offset: Int = 0 }
    output { rows: String }
}
"#;
        let model = parse_ifml(input).unwrap();
        let params = &model.views[0].params;
        assert_eq!(params.len(), 4);

        assert_eq!(params[0].name, "id");
        assert_eq!(params[0].default, None);

        assert_eq!(params[1].name, "slug");
        assert_eq!(
            params[1].default,
            Some(ValueExpression::String("home".to_string()))
        );

        assert_eq!(params[2].name, "page");
        assert_eq!(params[2].default, Some(ValueExpression::Number(1.0)));

        assert_eq!(params[3].name, "verbose");
        assert_eq!(params[3].default, Some(ValueExpression::Bool(true)));

        assert_eq!(
            model.modules[0].input_params[0].default,
            Some(ValueExpression::Number(0.0))
        );
    }

    #[test]
    fn test_param_negative_number_default() {
        let input = r#"
view "V" {
    params { zoom: Int = -2 };
    component "c" {
        type: list;
        data: Item;
    }
}
"#;
        let model = parse_ifml(input).unwrap();
        assert_eq!(
            model.views[0].params[0].default,
            Some(ValueExpression::Number(-2.0))
        );
    }

    #[test]
    fn test_param_default_requires_literal() {
        let input = r#"
view "V" {
    params { slug: String = slugify(title) };
    component "c" {
        type: list;
        data: Item;
    }
}
"#;
        assert!(parse_ifml(input).is_err());
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
            ValueExpression::Number(25.0)
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
    fn test_view_roles_extracted_from_property_bag() {
        let input = r#"
view "AdminConsole" {
    roles: [admin, manager];
    landmark: true;

    component "grid" {
        type: list;
        data: Item;
    }
}
"#;
        let model = parse_ifml(input).unwrap();
        let view = &model.views[0];
        assert_eq!(view.roles, vec!["admin".to_string(), "manager".to_string()]);
        assert!(view.is_landmark);
        assert!(
            !view.properties.iter().any(|p| p.key == "roles"),
            "roles must be removed from the property bag"
        );
    }

    #[test]
    fn test_view_roles_empty_array() {
        let input = r#"
view "Open" {
    roles: [];

    component "grid" {
        type: list;
        data: Item;
    }
}
"#;
        let model = parse_ifml(input).unwrap();
        assert!(model.views[0].roles.is_empty());
        assert!(
            !model.views[0].properties.iter().any(|p| p.key == "roles"),
            "empty roles must also be removed from the property bag"
        );
    }

    #[test]
    fn test_view_without_roles_parses_identically() {
        let input = r#"
view "Catalog" {
    label "Catalog";
    landmark: true;

    component "grid" {
        type: list;
        data: Product;
        fields: [name, price];
    }

    on load -> stay;
}
"#;
        let model = parse_ifml(input).unwrap();
        let view = &model.views[0];
        assert!(view.roles.is_empty());
        assert_eq!(view.label.as_deref(), Some("Catalog"));
        assert!(view.is_landmark);
        assert_eq!(view.components.len(), 1);
        assert_eq!(view.events.len(), 1);
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
    fn test_view_requires_extracted_from_property_bag() {
        let input = r#"
view "RefundQueue" {
    requires: [RaiseRefund, ApproveRefund];
    landmark: true;

    component "grid" {
        type: list;
        data: Refund;
    }
}
"#;
        let model = parse_ifml(input).unwrap();
        let view = &model.views[0];
        assert_eq!(
            view.requires,
            vec!["RaiseRefund".to_string(), "ApproveRefund".to_string()]
        );
        assert!(view.is_landmark);
        assert!(
            !view.properties.iter().any(|p| p.key == "requires"),
            "requires must be removed from the property bag"
        );
    }

    #[test]
    fn test_view_requires_empty_array() {
        let input = r#"
view "Open" {
    requires: [];

    component "grid" {
        type: list;
        data: Item;
    }
}
"#;
        let model = parse_ifml(input).unwrap();
        assert!(model.views[0].requires.is_empty());
        assert!(
            !model.views[0]
                .properties
                .iter()
                .any(|p| p.key == "requires"),
            "empty requires must also be removed from the property bag"
        );
    }

    #[test]
    fn test_view_requires_and_roles_coexist() {
        let input = r#"
view "Approvals" {
    roles: [admin, manager];
    requires: [ApproveRefund];

    component "grid" {
        type: list;
        data: Refund;
    }
}
"#;
        let model = parse_ifml(input).unwrap();
        let view = &model.views[0];
        assert_eq!(view.roles, vec!["admin".to_string(), "manager".to_string()]);
        assert_eq!(view.requires, vec!["ApproveRefund".to_string()]);
        assert!(!view.properties.iter().any(|p| p.key == "roles"));
        assert!(!view.properties.iter().any(|p| p.key == "requires"));
    }

    #[test]
    fn test_view_without_requires_parses_identically() {
        let input = r#"
view "Catalog" {
    label "Catalog";
    landmark: true;

    component "grid" {
        type: list;
        data: Product;
        fields: [name, price];
    }

    on load -> stay;
}
"#;
        let model = parse_ifml(input).unwrap();
        let view = &model.views[0];
        assert!(view.requires.is_empty());
        assert!(view.roles.is_empty());
        assert_eq!(view.label.as_deref(), Some("Catalog"));
        assert!(view.is_landmark);
        assert_eq!(view.components.len(), 1);
        assert_eq!(view.events.len(), 1);
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
        assert_eq!(
            json["requires"],
            serde_json::json!([]),
            "events without requires carry an empty list (serde default): {json}"
        );
    }
}
