//! Pest-based parser for the `.ifml` surface: source text in,
//! [`rex_ir::ifml::IfmlModel`] out. The grammar is normative in
//! `grammar/ifml.pest` (compiled in via `pest_derive`); the submodules
//! lower the parse tree one rule at a time:
//!
//! - `utils` — lexeme helpers and property-bag extractors
//! - `expressions` — the two expression surfaces (`ValueExpression`
//!   for property values, `Expression` for bound conditions/bindings)
//! - `events` — event handlers and their actions
//! - `components` — components, columns, fields, charts
//! - `views` — view/container declarations (shared body shape)
//! - `modules` — module declarations and `use` statements
//! - `commands` — CLI `command` declarations
//! - `declarations` — domain/actor/action/import declarations

mod commands;
mod components;
mod declarations;
mod events;
mod expressions;
mod modules;
mod utils;
mod views;

pub(crate) use utils::parse_string;

use std::path::Path;

use pest::iterators::{Pair, Pairs};
use pest::Parser;
use pest_derive::Parser;

use rex_ir::ifml::*;

use crate::parser::commands::parse_command_declaration;
use crate::parser::declarations::{
    parse_action_declaration, parse_actor_declaration, parse_domain_declaration,
    parse_import_declaration,
};
use crate::parser::modules::parse_module_declaration;
use crate::parser::views::parse_view_declaration;

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
