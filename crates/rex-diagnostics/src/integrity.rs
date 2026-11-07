//! The `driver.integrity` band: `X0001`–`X0099`, the rex-driver's
//! cross-surface integrity pass (`check_integrity`, issue rexlang#114 —
//! `crates/rex-driver/src/integrity.rs`). Every check prefixes its code
//! onto the diagnostic message (`X0001 unknown capability …`), the way
//! the CLI's `.ifml` converter prefixes codes; the registry entry is the
//! documentation `rexlang explain` and `docs/DIAGNOSTICS.md` render. Claim
//! a new code by adding a documented constant plus an `Entry` to
//! `ENTRIES`.

use crate::{Entry, Severity};

/// The band's registry id ([`crate::all`] entries reference it).
pub const INTEGRITY_BAND: &str = "driver.integrity";

/// `unknown capability '<capability>' on operation '<op>' of service
/// '<service>'` (or `on search '<search>'`) — the warning lint channel of
/// the compile-time capability join.
pub const X_UNKNOWN_OP_CAPABILITY: &str = "X0001";
/// `capability '<capability>' is granted but no operation or search
/// carries it` — the warning lint channel of the compile-time grant
/// coverage join.
pub const X_GRANTED_CAPABILITY_UNCARRIED: &str = "X0002";
/// `operation '<op>' of service '<service>' emits unknown event '<event>'`.
pub const X_UNKNOWN_EMISSION: &str = "X0003";
/// `event '<event>' is declared but no operation emits it`.
pub const X_ORPHAN_EVENT: &str = "X0004";
/// `action '<action>' does not bind to any service operation`.
pub const X_UNBOUND_ACTION: &str = "X0005";
/// `action '<action>' binds ambiguously to '<service.op>' and
/// '<service.op>'` — the case-insensitive mirror names several operations.
pub const X_AMBIGUOUS_ACTION: &str = "X0006";
/// `subscription '<subscription>' consumer '<consumer>' does not resolve
/// to a declared component`.
pub const X_UNKNOWN_CONSUMER: &str = "X0007";
/// `unknown capability or actor '<name>' in requires of …` / `unknown
/// actor '<name>' in roles of view '<name>'`.
pub const X_UNKNOWN_FLOW_REFERENCE: &str = "X0008";

pub(crate) const ENTRIES: &[Entry] = &[
    Entry {
        code: X_UNKNOWN_OP_CAPABILITY,
        band: INTEGRITY_BAND,
        severity: Severity::Warning,
        title: "Unknown operation capability",
        meaning: "`unknown capability '<capability>' on operation '<op>' of service '<service>'` (or `on search '<search>'`) — the capability clause names no capability declared in any provided actor block. Since rexlang#128 the compile-time join is the language error (code `unknownCapability`); this union-pass finding is the lint channel for hosts that gather artifacts without compiling.",
        fix: "Declare the capability (`capability X on <Class>;` in an actors block), or rename the reference to a declared capability.",
    },
    Entry {
        code: X_GRANTED_CAPABILITY_UNCARRIED,
        band: INTEGRITY_BAND,
        severity: Severity::Warning,
        title: "Granted capability uncarried",
        meaning: "`capability '<capability>' is granted but no operation or search carries it` — the granted (or delegated) capability guards a transition that cannot happen. Since rexlang#128 the compile-time join is the language error (code `grantedCapabilityUncarried`); this union-pass finding is the lint channel.",
        fix: "Carry the capability — add it to an operation or search across the designs — or drop the grant entry.",
    },
    Entry {
        code: X_UNKNOWN_EMISSION,
        band: INTEGRITY_BAND,
        severity: Severity::Error,
        title: "Unknown emitted event",
        meaning: "`operation '<op>' of service '<service>' emits unknown event '<event>'` — the `emits` clause is loosely coupled by design (the design compile has no event bucket), so the name resolves only across the artifact union.",
        fix: "Declare the event in an `.evt` contract, or fix the spelling.",
    },
    Entry {
        code: X_ORPHAN_EVENT,
        band: INTEGRITY_BAND,
        severity: Severity::Error,
        title: "Orphan declared event",
        meaning: "`event '<event>' is declared but no operation emits it` — nothing produces the declared event, so it is dead weight. Channel `publishes` entries and subscription `events [...]` lists are consumption-side wiring and never satisfy production.",
        fix: "Emit the event from a service operation's `emits` clause, or drop the declaration.",
    },
    Entry {
        code: X_UNBOUND_ACTION,
        band: INTEGRITY_BAND,
        severity: Severity::Error,
        title: "Unbound action",
        meaning: "`action '<action>' does not bind to any service operation` — binding is ASCII-case-insensitive name equality (the PascalCase action ↔ camelCase operation mirror), and no operation carries the name. An unbound action is a button whose backend does not exist.",
        fix: "Name the action after the operation it drives (or the reverse), or add the operation to a design.",
    },
    Entry {
        code: X_AMBIGUOUS_ACTION,
        band: INTEGRITY_BAND,
        severity: Severity::Error,
        title: "Ambiguous action binding",
        meaning: "`action '<action>' binds ambiguously to '<service.op>' and '<service.op>'` — several operations across the designs share the action's case-insensitive mirror name, so the binding names no unique backend (listing every carrier).",
        fix: "Rename the colliding operations so each action's case-insensitive mirror names exactly one service operation.",
    },
    Entry {
        code: X_UNKNOWN_CONSUMER,
        band: INTEGRITY_BAND,
        severity: Severity::Error,
        title: "Unknown subscription consumer",
        meaning: "`subscription '<subscription>' consumer '<consumer>' does not resolve to a declared component` — a dead inbox: the events are published, but no deploy component receives them.",
        fix: "Declare the component in a deploy model (any application), or fix the consumer's spelling.",
    },
    Entry {
        code: X_UNKNOWN_FLOW_REFERENCE,
        band: INTEGRITY_BAND,
        severity: Severity::Error,
        title: "Unknown flow actor reference",
        meaning: "`unknown capability or actor '<name>' in requires of view/command '<name>'`, or `unknown actor '<name>' in roles of view '<name>'` — the flow clause names neither a capability nor an actor the actor layer declares (`roles:` is strictly actors).",
        fix: "Declare the capability or actor in an actors block, or fix the spelling.",
    },
];
