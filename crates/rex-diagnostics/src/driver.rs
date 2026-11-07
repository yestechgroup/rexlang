//! The `driver.payload` band: the rex-driver's payload-carrying LSP
//! codes — the camelCase names `rex_driver::DiagnosticCode::name()`
//! emits as the LSP diagnostic `code` string. They are exempt from the
//! numeric ranges (their variants and payload serialization are pinned
//! by goldens in `crates/rex-driver/src/diagnostic.rs` and must not
//! change); they are registered here so `rexlang explain` and the
//! ledger cover every code a user can meet. Claim a new name by adding a
//! documented constant plus an `Entry` to `ENTRIES` once the driver
//! grows another `DiagnosticCode` variant.

use crate::{Entry, Severity};

/// The band's registry id ([`crate::all`] entries reference it).
pub const DRIVER_BAND: &str = "driver.payload";

/// `feature '<feature>' has class type '<class>'` — a quick-fixable
/// `contains`/`refers` suggestion.
pub const ATTRIBUTE_WITH_CLASS_TYPE: &str = "attributeWithClassType";
/// `unknown capability '<capability>' on …` — with a did-you-mean
/// suggestion when one is close.
pub const UNKNOWN_CAPABILITY: &str = "unknownCapability";
/// `capability '<capability>' is granted but no operation or search
/// carries it`.
pub const GRANTED_CAPABILITY_UNCARRIED: &str = "grantedCapabilityUncarried";

pub(crate) const ENTRIES: &[Entry] = &[
    Entry {
        code: ATTRIBUTE_WITH_CLASS_TYPE,
        band: DRIVER_BAND,
        severity: Severity::Error,
        title: "Attribute with class type",
        meaning: "`feature '<feature>' has class type '<class>'` — the author almost certainly meant `contains` or `refers`; the payload drives a two-way quick fix.",
        fix: "Replace the class type with `contains <Class>` (many) or `refers <Class>` (one) — the LSP offers both.",
    },
    Entry {
        code: UNKNOWN_CAPABILITY,
        band: DRIVER_BAND,
        severity: Severity::Error,
        title: "Unknown capability",
        meaning: "`unknown capability '<capability>' on …` — the name carries no resolution to a declared actor capability (rexlang#128's J3, resolved at compile time); a did-you-mean suggestion is attached when one is close.",
        fix: "Declare the capability (`capability X on <Class>;` in an actors block), or rename the reference to the suggested declared name.",
    },
    Entry {
        code: GRANTED_CAPABILITY_UNCARRIED,
        band: DRIVER_BAND,
        severity: Severity::Error,
        title: "Granted capability uncarried",
        meaning: "`capability '<capability>' is granted but no operation or search carries it` — the granted capability guards a transition that cannot happen (rexlang#128's J4, resolved at compile time); the fix lives in another file, so there is no local quick fix.",
        fix: "Carry the capability — add it to an operation or search across the designs — or drop the grant entry.",
    },
];
