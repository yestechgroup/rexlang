//! The Cedar policy-set emitter: one policy paragraph per grant-entry
//! authority (with obligations joined into one annotation and
//! delegation/never_both evidence as comments).
use super::banner;
use super::condition::{cedar_string_literal, condition_text};
use super::mapping::capability_class;
use super::reach::actors_inheriting;
use rex_ir::{ActorsDef, GrantEffect, GrantEntry, Model, SessionKind};

/// The session-kind gate (rexlang#181 part B): a grant or delegation entry
/// restricted to `human` or `agent` sessions is a named generate-time
/// rejection. The Cedar backend cannot express session provenance in v1 —
/// the downstream enforcement mapping is deferred pending the codegraph
/// security sign-off — and emitting a restricted entry as an unconditional
/// policy would silently widen the grant, so generation refuses instead.
/// `any` entries (the default) pass untouched, byte-identical.
fn reject_session_kind(entry: &GrantEntry, authority: &str) -> anyhow::Result<()> {
    let kind = match entry.session_kind {
        SessionKind::Any => return Ok(()),
        SessionKind::Human => "human",
        SessionKind::Agent => "agent",
    };
    anyhow::bail!(
        "capability `{capability}` granted to {authority} applies only to {kind} \
         sessions, which the Cedar backend cannot express in v1: emitting it as \
         an unconditional policy would widen the grant, so generation refuses \
         (session-kind enforcement is deferred pending the codegraph security \
         sign-off, rexlang#181)",
        capability = entry.capability,
    )
}

/// Builds the policy set for one actors block: banner, then one paragraph
/// per grant entry (effect policy or verbatim cedar splice) in IR order,
/// then one evidence comment per `never_both` group, then one evidence
/// comment per delegation in IR order. Ends with exactly one newline.
pub(super) fn policy_set(actors: &ActorsDef, model: &Model) -> anyhow::Result<String> {
    let namespace = &actors.name;
    let mut paragraphs = vec![banner(namespace)];
    for grant in &actors.grants {
        for entry in &grant.entries {
            match &entry.cedar {
                Some(text) => paragraphs.push(text.clone()),
                None => {
                    reject_session_kind(entry, &format!("actor `{}`", grant.actor))?;
                    let class = capability_class(actors, model, &entry.capability)?;
                    // Extends confers grant inheritance (the same
                    // effective-permit rule the driver's delegation
                    // containment and `gen tools` use), and Cedar's `is` is
                    // an exact-type match — so every actor type holding the
                    // authority gets its own policy, in declaration order.
                    for principal in actors_inheriting(actors, &grant.actor) {
                        let mut paragraph = String::new();
                        // One `@obligation` annotation per policy: Cedar
                        // annotations carry exactly one string value and
                        // reject duplicate keys, so multiple obligations on
                        // one entry join into a single comma-separated value
                        // (names are identifiers, so no comma can occur).
                        if !entry.obligations.is_empty() {
                            let joined = cedar_string_literal(&entry.obligations.join(","));
                            paragraph.push_str(&format!("@obligation({joined})\n"));
                        }
                        paragraph.push_str(&format!(
                            "{effect}(principal is {namespace}::{actor}, action == \
                             {namespace}::Action::{capability}, resource is \
                             {namespace}::{class})",
                            effect = match entry.effect {
                                GrantEffect::Permit => "permit",
                                GrantEffect::Forbid => "forbid",
                            },
                            actor = principal.name,
                            capability = cedar_string_literal(&entry.capability),
                            class = class.name,
                        ));
                        if let Some(when) = &entry.when {
                            paragraph.push_str(&format!(
                                " when {{ {} }}",
                                condition_text(&entry.capability, class, when)?
                            ));
                        }
                        paragraph.push(';');
                        paragraphs.push(paragraph);
                    }
                }
            }
        }
    }
    for constraint in &actors.never_both {
        paragraphs.push(format!(
            "// never_both: {}",
            constraint.capabilities.join(", ")
        ));
    }
    // Delegations are evidence comments only: like `never_both`, the driver
    // enforces them at compile time and the authorization gateway enforces
    // them at runtime, so neither policies nor schema change. The count is
    // the number of permit entries — what the gateway would grant. A
    // declared purpose appends ` purpose: <P>`; without one the line ends
    // right after the count.
    for delegation in &actors.delegations {
        // A session-kind entry rides the delegation to the same refused
        // v1 posture (rexlang#181 part B): the authority line names the
        // receiving actor and the delegation.
        for entry in &delegation.entries {
            reject_session_kind(
                entry,
                &format!(
                    "actor `{}` via delegation `{}`",
                    delegation.to, delegation.name
                ),
            )?;
        }
        let permits = delegation
            .entries
            .iter()
            .filter(|entry| entry.effect == GrantEffect::Permit)
            .count();
        let mut line = format!(
            "// delegation {}: {} -> {} ({} capabilities)",
            delegation.name, delegation.from, delegation.to, permits
        );
        if let Some(purpose) = &delegation.purpose {
            line.push_str(&format!(" purpose: {purpose}"));
        }
        paragraphs.push(line);
    }
    let mut out = paragraphs.join("\n\n");
    out.push('\n');
    Ok(out)
}
