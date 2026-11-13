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

use rex_ir::events::{ChannelDef, CloudEventsAttributes, DeliveryMode, EventDef, EventModel};
use rex_syntax::ast as dsl;
use rex_syntax::Span;

use crate::conditions::{
    check_condition, when_interior, ConditionSpanning, ConditionTriage, ConditionWording,
    LintWording, ParseErrorPolicy,
};
use crate::diagnostic::Diagnostic;
use crate::engine_common;
use crate::lower::{self, DomainUnit};
use crate::pipeline;

/// The closed delivery-mode vocabulary of a `delivery` block's `mode`
/// entry (the transition-vocabulary discipline: the words live in the
/// driver, not in the syntax).
const DELIVERY_MODES: [&str; 3] = ["at_most_once", "at_least_once", "effectively_once"];

/// Lowers and validates one parsed `.evt` file against its imported domain
/// models. Returns the event-contract artifact (or `None` when any
/// error-severity diagnostic was produced anywhere) plus the diagnostics
/// from every file, each tagged with its path.
///
/// See [`compile_evt_str`](crate::compile_evt_str) for the normative
/// contract.
pub(crate) fn compile_evt_file(
    path: &str,
    source: &str,
    ast: Option<&dsl::EvtFile>,
    parse_diagnostics: &[Diagnostic],
    domains: &[DomainUnit],
) -> (Option<EventModel>, Vec<(String, Diagnostic)>) {
    // Import resolution: each import must name one of the provided domains
    // (duplicates are fine — the domain joins the union once).
    let imports = ast.map(|file| file.imports.as_slice()).unwrap_or_default();
    let mut import_diagnostics = Vec::new();
    for import in imports {
        if !domains.iter().any(|unit| unit.path == import.path) {
            import_diagnostics.push(Diagnostic::error(
                format!("imported file \"{}\" was not provided", import.path),
                Some(import.span),
            ));
        }
    }

    let diags = pipeline::engine_diags(path, parse_diagnostics, import_diagnostics, domains);

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
    // types) in declaration order, the governance metadata (`pii` names
    // declared payload fields), and the CloudEvents attributes (non-empty
    // `type`/`source`).
    let mut event_names: Vec<(String, Span)> = Vec::new();
    for event in &ast.events {
        // Rule e — event names are unique within the file.
        engine_common::register_unique_declared(
            &mut event_names,
            &event.name.text,
            event.name.span,
            "event",
            &mut local,
        );

        let mut event_out = EventDef::new(event.name.text.clone());
        event_out.version = event.version.clone();
        // Governance metadata rides through verbatim — labels the driver
        // never interprets (the retention period stays whatever string the
        // source authored, e.g. `7y`).
        event_out.owner = event.owner.as_ref().map(|owner| owner.text.clone());
        event_out.classification = event
            .classification
            .as_ref()
            .map(|classification| classification.text.clone());
        event_out.retention = event.retention.clone();

        let mut field_names: Vec<String> = Vec::new();
        for field in &event.fields {
            // Extra rule — payload field names are unique within their
            // event (the wire contract documents the uniqueness as a
            // driver concern).
            engine_common::register_unique(
                &mut field_names,
                &field.name.text,
                field.name.span,
                format!(
                    "duplicate payload field '{}' in event '{}'",
                    field.name.text, event.name.text
                ),
                Some("a payload field is declared once per event"),
                &mut local,
            );

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

        // Rule 9 — `pii` entries must name declared payload fields (after
        // the fields, in source order, whatever the source's member order).
        for field in &event.pii {
            if !field_names.contains(&field.text) {
                local.push(
                    Diagnostic::error(
                        format!(
                            "unknown payload field '{}' in pii of event '{}'",
                            field.text, event.name.text
                        ),
                        Some(field.span),
                    )
                    .with_help("pii entries must name payload fields of the same event"),
                );
            } else {
                event_out.pii.push(field.text.clone());
            }
        }

        // Rule 8 — the CloudEvents attributes are non-empty when the block
        // is present (they are authoritative for a downstream generator).
        if let Some(cloud_events) = &event.cloud_events {
            if cloud_events.r#type.is_empty() {
                local.push(
                    Diagnostic::error(
                        format!(
                            "cloudEvents type on event '{}' must not be empty",
                            event.name.text
                        ),
                        Some(cloud_events.span),
                    )
                    .with_help("the CloudEvents spec requires a non-empty type"),
                );
            }
            if cloud_events.source.is_empty() {
                local.push(
                    Diagnostic::error(
                        format!(
                            "cloudEvents source on event '{}' must not be empty",
                            event.name.text
                        ),
                        Some(cloud_events.span),
                    )
                    .with_help("the CloudEvents spec requires a non-empty source"),
                );
            }
            event_out.cloud_events = Some(CloudEventsAttributes {
                r#type: cloud_events.r#type.clone(),
                source: cloud_events.source.clone(),
                subject: cloud_events.subject.clone(),
            });
        }
        model.events.push(event_out);
    }

    // Channels: unique names, every `publishes` entry naming a declared
    // event.
    let mut channel_names: Vec<(String, Span)> = Vec::new();
    for channel in &ast.channels {
        // Rule e — channel names are unique within the file.
        engine_common::register_unique_declared(
            &mut channel_names,
            &channel.name.text,
            channel.name.span,
            "channel",
            &mut local,
        );

        let mut channel_out = ChannelDef::new(channel.name.text.clone());
        // The channel's `of <scope>` clause rides through verbatim — an
        // opaque topic-template label the driver never validates.
        channel_out.scope = channel.scope.as_ref().map(|scope| scope.text.clone());
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

    // Subscriptions: unique names, a non-empty event list and consumer,
    // every subscribed event declared in this file, a `when` filter that
    // types boolean against EVERY subscribed event, and a delivery policy
    // whose mode, numbers, and dead-letter channel validate.
    let mut subscription_names: Vec<(String, Span)> = Vec::new();

    // The synthetic per-event universe for `when` filters (the `.cmmn`
    // discipline): the imported domains' classes merge first, then each
    // subscribed event inserts a synthetic class whose features are its
    // payload fields — the expression's implicit `self` — so a bare field
    // name refers to the event being filtered. `insert_class` replaces the
    // previous entry, so one universe serves every event.
    let mut types = engine_common::union_types(domains);
    // Dedupes the guard lint across the per-event universes: the same
    // comparison at the same filter span is one warning, not one per
    // subscribed event.
    let mut seen_lints: std::collections::BTreeSet<(usize, String)> =
        std::collections::BTreeSet::new();

    for subscription in &ast.subscriptions {
        // Rule e — subscription names are unique within the file.
        engine_common::register_unique_declared(
            &mut subscription_names,
            &subscription.name.text,
            subscription.name.span,
            "subscription",
            &mut local,
        );

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
        // The subscription's `scope <name>` binding rides through verbatim
        // — an opaque slice label the driver never validates.
        subscription_out.scope = subscription.scope.as_ref().map(|scope| scope.text.clone());

        // Rule 13 — the `when` filter parses and types `boolean` against
        // EVERY subscribed event: a filter is a routing decision per event,
        // so a field absent from any listed event is an error by design.
        // Parse errors leave the filter out of the artifact; type errors
        // still lower the verbatim interior (the `.cmmn` precedent) — the
        // artifact drops on any error either way.
        if let Some(when_span) = subscription.when {
            // The span covers the raw parens; slice strictly inside (the
            // `when`-condition convention).
            let interior = when_interior(source, when_span).to_string();
            // The parse-error pass: one diagnostic per parse error, help
            // naming the subscription.
            let parse_wording = ConditionWording {
                parse_errors: ParseErrorPolicy::Emit,
                parse_prefix: "invalid when filter: ".to_string(),
                type_errors_raw: true,
                boolean_prefix: String::new(),
                lint: LintWording::Skip,
                parse_help: Some(format!(
                    "in the `when` of subscription '{}'",
                    subscription.name.text
                )),
                typed_help: None,
            };
            let outcome = check_condition(
                &interior,
                ConditionSpanning::Fixed(when_span),
                parse_wording,
                None,
                None,
                &mut local,
            );
            if outcome.triage != ConditionTriage::Unparsed {
                for event_name in &subscription_out.events {
                    // Unknown events were diagnosed above; typing skips
                    // them (their payload shape is unknown).
                    let Some(event_def) =
                        model.events.iter().find(|event| &event.name == event_name)
                    else {
                        continue;
                    };
                    let features = event_def
                        .fields
                        .iter()
                        .map(|field| {
                            rex_ir::Feature::new(
                                field.name.clone(),
                                rex_ir::FeatureKind::Attribute,
                                field.ty.clone(),
                                rex_ir::Multiplicity::REQUIRED,
                            )
                        })
                        .collect();
                    types.insert_class("", event_name, Vec::new(), features, Vec::new());
                    let checker =
                        rex_expr::TypeChecker::new(types.clone()).with_self("", event_name);
                    let where_help = format!(
                        "in the `when` of subscription '{}' against event '{}'",
                        subscription.name.text, event_name
                    );
                    let wording = ConditionWording {
                        // The parse errors were emitted above; the
                        // recovered expression still types per event.
                        parse_errors: ParseErrorPolicy::ReportedElsewhere,
                        parse_prefix: String::new(),
                        type_errors_raw: true,
                        boolean_prefix: "when filter ".to_string(),
                        lint: LintWording::Suffixed(format!(
                            " (in the `when` of subscription '{}')",
                            subscription.name.text
                        )),
                        parse_help: None,
                        typed_help: Some(where_help),
                    };
                    let outcome = check_condition(
                        &interior,
                        ConditionSpanning::Fixed(when_span),
                        wording,
                        Some(checker),
                        None,
                        &mut local,
                    );
                    // The use-based guard lint (issue #142), deduped
                    // across the per-event universes: the same comparison
                    // against every listed event is one warning.
                    for lint in outcome.lints {
                        if seen_lints.insert((when_span.start, lint.message.clone())) {
                            local.push(lint);
                        }
                    }
                }
                // Only a successfully parsed filter lowers into the
                // artifact (the `.cmmn` precedent: parse errors leave
                // nothing to carry).
                subscription_out.when = Some(interior);
            }
        }

        // Rule 10–12 — the delivery policy: the mode is one of the three
        // vocabulary words, the tuning knobs are positive, and the
        // dead-letter channel is declared in this file.
        if let Some(delivery) = &subscription.delivery {
            let mut policy = rex_ir::events::DeliveryPolicy::new(DeliveryMode::AtLeastOnce);
            if let Some(mode) = &delivery.mode {
                match DELIVERY_MODES.iter().find(|word| **word == mode.text) {
                    Some(word) => {
                        policy.mode = match *word {
                            "at_most_once" => DeliveryMode::AtMostOnce,
                            "effectively_once" => DeliveryMode::EffectivelyOnce,
                            _ => DeliveryMode::AtLeastOnce,
                        };
                    }
                    None => {
                        local.push(
                            Diagnostic::error(
                                format!(
                                    "unknown delivery mode '{}' on subscription '{}'",
                                    mode.text, subscription.name.text
                                ),
                                Some(mode.span),
                            )
                            .with_help(format!(
                                "delivery modes are {}",
                                DELIVERY_MODES
                                    .iter()
                                    .map(|word| format!("`{word}`"))
                                    .collect::<Vec<_>>()
                                    .join(", ")
                            )),
                        );
                    }
                }
            }
            for (value, entry) in [
                (delivery.max_attempts, "maxAttempts"),
                (delivery.backoff_seconds, "backoffSeconds"),
                (delivery.timeout_seconds, "timeoutSeconds"),
            ] {
                if let Some(value) = value {
                    if value <= 0 {
                        local.push(
                            Diagnostic::error(
                                format!(
                                    "'{entry}' on subscription '{}' must be positive",
                                    subscription.name.text
                                ),
                                Some(delivery.span),
                            )
                            .with_help("a delivery tuning knob must be greater than zero"),
                        );
                    }
                }
            }
            if let Some(value) = delivery.max_attempts {
                policy.max_attempts = Some(value.max(0) as u32);
            }
            if let Some(value) = delivery.backoff_seconds {
                policy.backoff_seconds = Some(value.max(0) as u32);
            }
            if let Some(value) = delivery.timeout_seconds {
                policy.timeout_seconds = Some(value.max(0) as u32);
            }
            if let Some(dead_letter) = &delivery.dead_letter {
                if channel_names
                    .iter()
                    .any(|(name, _)| *name == dead_letter.text)
                {
                    policy.dead_letter = Some(dead_letter.text.clone());
                } else {
                    local.push(
                        Diagnostic::error(
                            format!(
                                "unknown dead-letter channel '{}' on subscription '{}'",
                                dead_letter.text, subscription.name.text
                            ),
                            Some(dead_letter.span),
                        )
                        .with_help("the dead-letter channel must be declared in this file"),
                    );
                }
            }
            subscription_out.delivery = Some(policy);
        }

        model.subscriptions.push(subscription_out);
    }

    pipeline::engine_finish(path, local, diags, model)
}
