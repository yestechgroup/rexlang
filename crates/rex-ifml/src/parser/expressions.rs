//! The two expression surfaces of the DSL, lowered from the same Pest
//! parse tree:
//!
//! - `ValueExpression` — property values (`key: <value>;`), object/array
//!   literals included;
//! - `Expression` — bound expressions (event `if` conditions, parameter
//!   and column bindings), the surface `rex_expr` later type-checks.
//!
//! `parse_expression` and `convert_expression_to_value` are mutually
//! recursive with their `*_primary` helpers; both walk the same
//! precedence ladder (`logical_or` → `logical_and` → `comparison` →
//! `addition` → `multiplication` → `unary` → `primary`).

use super::Rule;
use pest::iterators::Pair;

use rex_ir::ifml::*;

use crate::parser::utils::{parse_number, parse_string};

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

pub(crate) fn parse_value_expression(pair: Pair<Rule>) -> ValueExpression {
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

pub(crate) fn parse_property_assignment(pair: Pair<Rule>) -> PropertyAssignment {
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

// ── Expression parsing (for parameter bindings) ──────────────────

fn fallback_expr(pair: &Pair<Rule>) -> Expression {
    Expression::Ident(pair.as_str().to_string())
}

pub(crate) fn parse_expression(pair: Pair<Rule>) -> Expression {
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
