//! CLI `command` declarations: typed arguments, options and prompts,
//! nested subcommands, and the property-bag fields (`description`,
//! `requires`) the view discipline shares.

use super::Rule;
use pest::iterators::Pair;

use rex_ir::ifml::*;

use crate::parser::events::parse_event_handler;
use crate::parser::expressions::{parse_expression, parse_property_assignment};
use crate::parser::modules::parse_module_use_statement;
use crate::parser::utils::{
    extract_description_property, extract_requires_property, parse_identifier, parse_number,
    parse_string,
};

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
pub(crate) fn parse_command_declaration(pair: Pair<Rule>) -> CommandDeclaration {
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
