//! Module declarations (`module "M" { input/output … }`) and the `use`
//! statements that compose them into views, containers, and commands.

use super::IfmlParseError;
use super::Rule;
use pest::iterators::Pair;

use rex_ir::ifml::*;

use crate::parser::components::parse_component_declaration;
use crate::parser::events::parse_event_handler;
use crate::parser::expressions::parse_property_assignment;
use crate::parser::utils::{parse_parameter_block, parse_string};
use crate::parser::views::parse_container_declaration;

pub(crate) fn parse_module_use_statement(pair: Pair<Rule>) -> ModuleUse {
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

pub(crate) fn parse_module_declaration(
    pair: Pair<Rule>,
) -> Result<ModuleDeclaration, IfmlParseError> {
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
