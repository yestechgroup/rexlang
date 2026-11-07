//! Top-level single-purpose declarations: `action`, `domain`, `actor`,
//! and `import`.

use super::Rule;
use pest::iterators::Pair;

use rex_ir::ifml::*;

use crate::parser::events::parse_event_handler;
use crate::parser::expressions::parse_property_assignment;
use crate::parser::utils::parse_string;

pub(crate) fn parse_action_declaration(pair: Pair<Rule>) -> ActionDeclaration {
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

pub(crate) fn parse_domain_declaration(pair: Pair<Rule>) -> DomainDeclaration {
    let mut inner = pair.clone().into_inner();
    let name = inner.next().map(|p| parse_string(&p)).unwrap_or_default();
    let schema_name = inner.next().map(|p| parse_string(&p)).unwrap_or_default();
    DomainDeclaration { name, schema_name }
}

pub(crate) fn parse_actor_declaration(pair: Pair<Rule>) -> ActorDeclaration {
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

pub(crate) fn parse_import_declaration(pair: Pair<Rule>) -> String {
    let mut inner = pair.into_inner();
    inner.next().map(|p| parse_string(&p)).unwrap_or_default()
}
