//! The `ifml.check` band: `E0100`–`E0149`, the typed checker diagnostics
//! of the `.ifml` compiler (rex-ifml's `check` module re-exports these
//! constants; the interaction cross-reference and authorization passes
//! included). Claim a new code by adding a documented constant plus an
//! `Entry` to `ENTRIES` — the meanings and fixes render into
//! `docs/DIAGNOSTICS.md`.

use crate::{Entry, Severity};

/// The band's registry id ([`crate::all`] entries reference it).
pub const CHECK_BAND: &str = "ifml.check";

/// A component's `data:` entity does not resolve to exactly one class in
/// the domain model.
pub const E_UNKNOWN_ENTITY: &str = "E0100";
/// A checked expression contradicts the type its site expects, or another
/// rex-expr semantic rule fires.
pub const E_TYPE_MISMATCH: &str = "E0101";
/// Unknown feature or operation on a navigation path.
pub const E_UNKNOWN_FEATURE: &str = "E0102";
/// An unbound name (not a parameter, not a feature, not an event
/// parameter).
pub const E_UNKNOWN_VARIABLE: &str = "E0103";
/// An `action("X")` invocation whose name resolves to no
/// `action "X" { ... }` declaration.
pub const E_UNKNOWN_ACTION: &str = "E0104";
/// A `navigate("X")` target that is neither a declared view nor a
/// declared container.
pub const E_UNKNOWN_VIEW: &str = "E0105";
/// A `refresh("X")` target that is not a declared component.
pub const E_UNKNOWN_COMPONENT: &str = "E0106";
/// A `requires:` capability name that no provided actor model declares.
pub const E_UNKNOWN_CAPABILITY: &str = "E0107";
/// A `roles:` name that no provided actor model declares as an actor.
pub const E_UNKNOWN_ROLE: &str = "E0108";
/// An action declaration's `target:` entity does not resolve to exactly
/// one class in the domain model.
pub const E_UNKNOWN_ACTION_TARGET: &str = "E0109";
/// A component's `data:` names a decision service that declares no
/// outputs.
pub const E_EMPTY_DECISION_SERVICE: &str = "E0110";
/// A component's `data:` names a decision service declared by two or more
/// of the provided decision models.
pub const E_AMBIGUOUS_DECISION_SERVICE: &str = "E0111";

pub(crate) const ENTRIES: &[Entry] = &[
    Entry {
        code: E_UNKNOWN_ENTITY,
        band: CHECK_BAND,
        severity: Severity::Error,
        title: "Unknown data entity",
        meaning: "A component's `data:` entity does not resolve to exactly one class in the domain model (`unknown data entity 'X'`, or `ambiguous data entity 'X'` when several packages declare the name).",
        fix: "Make the name unique in the domain union, or qualify it so exactly one `(package, class)` matches.",
    },
    Entry {
        code: E_TYPE_MISMATCH,
        band: CHECK_BAND,
        severity: Severity::Error,
        title: "Type mismatch",
        meaning: "An expression contradicts the type its site expects (`expected <ty>, found <ty> in '<expr>'`), or another rex-expr semantic rule fires (R1/R2/R4/R9/...), carried through verbatim.",
        fix: "Align the expression with the site's expected type — docs/EXPRESSIONS.md rules R1–R4 pin the arithmetic, equality, and division behavior.",
    },
    Entry {
        code: E_UNKNOWN_FEATURE,
        band: CHECK_BAND,
        severity: Severity::Error,
        title: "Unknown feature",
        meaning: "Unknown feature or operation on a navigation path (rex-expr's `unknown feature ...` / `unknown operation ...`), or an unknown or ambiguous bare package-function call, prefixed with site context.",
        fix: "Check the spelling against the entity's features or the package function table — the did-you-mean hint names the closest declared one.",
    },
    Entry {
        code: E_UNKNOWN_VARIABLE,
        band: CHECK_BAND,
        severity: Severity::Error,
        title: "Unknown variable",
        meaning: "A name that is not a view/container/module parameter, not a `data:` feature, and not an event parameter (`unknown name 'x'`), prefixed with site context.",
        fix: "Declare it as a parameter, read a feature of the component's entity, or fix the spelling.",
    },
    Entry {
        code: E_UNKNOWN_ACTION,
        band: CHECK_BAND,
        severity: Severity::Error,
        title: "Unknown action reference",
        meaning: "An `action(\"X\")` invocation whose name resolves to no `action \"X\" { target: ...; }` declaration (`action 'X' is not declared`).",
        fix: "add action \"X\" { target: <Entity>; }",
    },
    Entry {
        code: E_UNKNOWN_VIEW,
        band: CHECK_BAND,
        severity: Severity::Error,
        title: "Unknown navigate target",
        meaning: "A `navigate(\"X\")` target that is neither a declared view nor a declared container (`navigate target 'X' is not a declared view or container`).",
        fix: "Declare the view or container, or spell the target as declared (wizard steps count as containers).",
    },
    Entry {
        code: E_UNKNOWN_COMPONENT,
        band: CHECK_BAND,
        severity: Severity::Error,
        title: "Unknown refresh target",
        meaning: "A `refresh(\"X\")` target that is not a declared component (`refresh target 'X' is not a declared component`).",
        fix: "Spell the target exactly as the component is declared.",
    },
    Entry {
        code: E_UNKNOWN_CAPABILITY,
        band: CHECK_BAND,
        severity: Severity::Error,
        title: "Unknown capability",
        meaning: "A `requires:` capability name no provided actor model declares (`capability 'X' is not declared`).",
        fix: "add \"capability X on <Class>;\" to an actors block",
    },
    Entry {
        code: E_UNKNOWN_ROLE,
        band: CHECK_BAND,
        severity: Severity::Error,
        title: "Unknown role",
        meaning: "A `roles:` name no provided actor model declares as an actor (`role 'X' is not declared`).",
        fix: "add \"actor X;\" to an actors block",
    },
    Entry {
        code: E_UNKNOWN_ACTION_TARGET,
        band: CHECK_BAND,
        severity: Severity::Error,
        title: "Unknown action target",
        meaning: "An action declaration's `target:` entity does not resolve to exactly one class in the domain model (`unknown target entity 'X'`, or `ambiguous target entity 'X'` when several packages declare the name) — the `data:` discipline applied to the lowering's payload.",
        fix: "Point `target:` at a domain class that resolves to exactly one `(package, class)`.",
    },
    Entry {
        code: E_EMPTY_DECISION_SERVICE,
        band: CHECK_BAND,
        severity: Severity::Error,
        title: "Decision service without outputs",
        meaning: "A component's `data:` names a decision service that declares no outputs (`decision service 'X' declares no outputs`) — the service exposes nothing, so the component has no data contract.",
        fix: "Declare at least one output decision on the service (`outputs { ... }`), or bind the component to a domain class.",
    },
    Entry {
        code: E_AMBIGUOUS_DECISION_SERVICE,
        band: CHECK_BAND,
        severity: Severity::Error,
        title: "Ambiguous decision service",
        meaning: "A component's `data:` names a decision service that two or more of the provided decision models declare (`decision service 'X' is declared by two models — 'A' and 'B'`, `… declared by multiple models — 'A', 'B' and 'C'`); resolution is bare-name union-wide, so the reference is ambiguous. The models are named by their declared `decision_model` names (the checker receives artifacts, not paths).",
        fix: "Rename the service in one of the decision models, or bind the component to a domain class.",
    },
];
