//! View components: the declaration header (`component "name" { … }`),
//! the per-kind payload rules (table columns, form fields, chart specs,
//! modeled-chart refs), and the type inference that promotes a bare
//! component to table/form/chart from its body contents.

use super::IfmlParseError;
use super::Rule;
use pest::iterators::Pair;

use rex_ir::ifml::*;

use crate::parser::events::parse_event_handler;
use crate::parser::expressions::{
    parse_expression, parse_property_assignment, parse_value_expression,
};
use crate::parser::utils::parse_string;

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

pub(crate) fn parse_component_declaration(
    pair: Pair<Rule>,
) -> Result<ComponentDeclaration, IfmlParseError> {
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
