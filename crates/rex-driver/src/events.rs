//! Semantic lowering and validation for `.evt` event-contract sources: the
//! `.evt` counterpart of [`crate::lower`].
//!
//! This module is salsa-free; the driver's tracked queries call into it via
//! [`compile_evt_file`]. The normative contract — entry points, the
//! resolution rule, and the validation rules — is documented on
//! [`compile_evt_str`](crate::compile_evt_str).
//!
//! The engine mirrors [`crate::lower::compile_actor_file`]: parse
//! diagnostics and domain diagnostics propagate under their own file paths,
//! the `.evt` file's own diagnostics are tagged with its path, and any
//! error-severity diagnostic anywhere drops the artifact. Event field types
//! resolve through the shared domain namespace helper
//! ([`crate::lower::resolve_ddd_type`]) — never re-implemented here.

use rex_ir::events::{ChannelDef, EventDef, EventModel};
use rex_syntax::ast as dsl;
use rex_syntax::Span;

use crate::diagnostic::Diagnostic;
use crate::lower::{self, DomainUnit};

/// Lowers and validates one parsed `.evt` file against its imported domain
/// models. Returns the event-contract artifact (or `None` when any
/// error-severity diagnostic was produced anywhere) plus the diagnostics
/// from every file, each tagged with its path.
///
/// See [`compile_evt_str`](crate::compile_evt_str) for the normative
/// contract.
pub(crate) fn compile_evt_file(
    path: &str,
    ast: Option<&dsl::EvtFile>,
    parse_diagnostics: &[Diagnostic],
    domains: &[DomainUnit],
) -> (Option<EventModel>, Vec<(String, Diagnostic)>) {
    let mut diags: Vec<(String, Diagnostic)> = parse_diagnostics
        .iter()
        .map(|diagnostic| (path.to_string(), diagnostic.clone()))
        .collect();

    // Import resolution: each import must name one of the provided domains
    // (duplicates are fine — the domain joins the union once).
    let imports = ast.map(|file| file.imports.as_slice()).unwrap_or_default();
    for import in imports {
        if !domains.iter().any(|unit| unit.path == import.path) {
            diags.push((
                path.to_string(),
                Diagnostic::error(
                    format!("imported file \"{}\" was not provided", import.path),
                    Some(import.span),
                ),
            ));
        }
    }

    // Domain diagnostics propagate under their own file, in import order.
    for unit in domains {
        for diagnostic in &unit.diagnostics {
            diags.push((unit.path.clone(), diagnostic.clone()));
        }
    }

    let Some(ast) = ast else {
        return (None, diags);
    };

    // The `.evt` file's own diagnostics collect here (tagged with `path` at
    // the end), after the parse/import/domain diagnostics. They are grouped
    // by declaration kind — events, then channels, then subscriptions — each
    // group in source order, matching the artifact's declaration order.
    let mut local: Vec<Diagnostic> = Vec::new();
    let packages = lower::domain_namespaces(domains);

    let mut model = EventModel::new();

    // Events: unique names, then payload fields (unique names, resolved
    // types) in declaration order.
    let mut event_names: Vec<(String, Span)> = Vec::new();
    for event in &ast.events {
        // Rule e — event names are unique within the file.
        if event_names.iter().any(|(name, _)| *name == event.name.text) {
            local.push(
                Diagnostic::error(
                    format!("duplicate event '{}'", event.name.text),
                    Some(event.name.span),
                )
                .with_help(format!(
                    "'{}' is already declared in this file",
                    event.name.text
                )),
            );
        } else {
            event_names.push((event.name.text.clone(), event.name.span));
        }

        let mut event_out = EventDef::new(event.name.text.clone());
        event_out.version = event.version.clone();

        let mut field_names: Vec<String> = Vec::new();
        for field in &event.fields {
            // Extra rule — payload field names are unique within their
            // event (the wire contract documents the uniqueness as a
            // driver concern).
            if field_names.contains(&field.name.text) {
                local.push(
                    Diagnostic::error(
                        format!(
                            "duplicate payload field '{}' in event '{}'",
                            field.name.text, event.name.text
                        ),
                        Some(field.name.span),
                    )
                    .with_help("a payload field is declared once per event"),
                );
            } else {
                field_names.push(field.name.text.clone());
            }

            // Rule b — the field type resolves against the domain namespace
            // (any declared kind) or is a primitive. The parser recovers a
            // missing field type as a type reference with no segments; the
            // syntax error ("expected a field type") already reported it, so
            // resolution is skipped for exactly that recovery shape and the
            // failure placeholder stands (the artifact is dropped whenever
            // errors exist).
            let ty = if field.ty.name.segments.is_empty() {
                rex_ir::TypeRef::Class {
                    package: String::new(),
                    name: String::new(),
                }
            } else {
                lower::resolve_ddd_type(&field.ty, &packages, None, &mut local)
            };
            event_out
                .fields
                .push(rex_ir::events::EventField::new(field.name.text.clone(), ty));
        }
        model.events.push(event_out);
    }

    // Channels: unique names, every `publishes` entry naming a declared
    // event.
    let mut channel_names: Vec<(String, Span)> = Vec::new();
    for channel in &ast.channels {
        // Rule e — channel names are unique within the file.
        if channel_names
            .iter()
            .any(|(name, _)| *name == channel.name.text)
        {
            local.push(
                Diagnostic::error(
                    format!("duplicate channel '{}'", channel.name.text),
                    Some(channel.name.span),
                )
                .with_help(format!(
                    "'{}' is already declared in this file",
                    channel.name.text
                )),
            );
        } else {
            channel_names.push((channel.name.text.clone(), channel.name.span));
        }

        let mut channel_out = ChannelDef::new(channel.name.text.clone());
        for publishes in &channel.publishes {
            // Rule c — the published event must be declared in this file.
            // The parser recovers a missing event name as an empty name; the
            // syntax error already reported it, so the empty name is skipped
            // here.
            if publishes.event.text.is_empty() {
                continue;
            }
            if !event_names
                .iter()
                .any(|(name, _)| *name == publishes.event.text)
            {
                local.push(
                    Diagnostic::error(
                        format!(
                            "unknown event '{}' on channel '{}'",
                            publishes.event.text, channel.name.text
                        ),
                        Some(publishes.event.span),
                    )
                    .with_help("publishes entries must name events declared in this file"),
                );
            }
            channel_out
                .publishes
                .push(rex_ir::events::EventPublication {
                    event: publishes.event.text.clone(),
                });
        }
        model.channels.push(channel_out);
    }

    // Subscriptions: unique names, a non-empty event list and consumer, and
    // every subscribed event declared in this file.
    let mut subscription_names: Vec<(String, Span)> = Vec::new();
    for subscription in &ast.subscriptions {
        // Rule e — subscription names are unique within the file.
        if subscription_names
            .iter()
            .any(|(name, _)| *name == subscription.name.text)
        {
            local.push(
                Diagnostic::error(
                    format!("duplicate subscription '{}'", subscription.name.text),
                    Some(subscription.name.span),
                )
                .with_help(format!(
                    "'{}' is already declared in this file",
                    subscription.name.text
                )),
            );
        } else {
            subscription_names.push((subscription.name.text.clone(), subscription.name.span));
        }

        // Rule f — an empty event list or consumer is an error. The empty
        // event list is reachable from clean syntax (`events [ ]`); the
        // empty consumer is only reachable through parser recovery, whose
        // syntax error fires alongside this one (both name the subscription,
        // so the pairing is deterministic).
        if subscription.events.is_empty() {
            local.push(
                Diagnostic::error(
                    format!(
                        "subscription '{}' subscribes to no events",
                        subscription.name.text
                    ),
                    Some(subscription.span),
                )
                .with_help("add at least one event to the `events [...]` list"),
            );
        }
        if subscription.consumer.text.is_empty() {
            local.push(
                Diagnostic::error(
                    format!(
                        "subscription '{}' names an empty consumer",
                        subscription.name.text
                    ),
                    Some(subscription.span),
                )
                .with_help("add a `consumer <name>` clause to the subscription body"),
            );
        }

        let mut subscription_out =
            rex_ir::events::SubscriptionDef::new(subscription.name.text.clone());
        for event in &subscription.events {
            // Rule d — every subscribed event must be declared in this file.
            if !event_names.iter().any(|(name, _)| *name == event.text) {
                local.push(
                    Diagnostic::error(
                        format!(
                            "unknown event '{}' in subscription '{}'",
                            event.text, subscription.name.text
                        ),
                        Some(event.span),
                    )
                    .with_help("subscriptions must reference events declared in this file"),
                );
            }
            subscription_out.events.push(event.text.clone());
        }
        subscription_out.consumer = subscription.consumer.text.clone();
        model.subscriptions.push(subscription_out);
    }

    diags.extend(local.into_iter().map(|d| (path.to_string(), d)));

    let blocked = diags.iter().any(|(_, diagnostic)| diagnostic.is_error());
    ((!blocked).then_some(model), diags)
}
