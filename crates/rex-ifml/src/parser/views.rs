//! Views and containers. Both declarations share one body shape
//! (params, label, properties, nested containers/components, event
//! handlers, `use` statements, condition); `parse_view_body_content`
//! and `parse_container_body_content` walk it for their respective
//! callers.

use super::IfmlParseError;
use super::Rule;
use pest::iterators::Pair;

use rex_ir::ifml::*;

use crate::parser::components::parse_component_declaration;
use crate::parser::events::parse_event_handler;
use crate::parser::expressions::{parse_expression, parse_property_assignment};
use crate::parser::modules::parse_module_use_statement;
use crate::parser::utils::{
    extract_bool_property, extract_position_property, extract_requires_property,
    extract_roles_property, parse_parameter_block, parse_string,
};

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

pub(crate) fn parse_container_declaration(
    pair: Pair<Rule>,
) -> Result<ContainerDeclaration, IfmlParseError> {
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

pub(crate) fn parse_view_declaration(pair: Pair<Rule>) -> Result<ViewDeclaration, IfmlParseError> {
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
