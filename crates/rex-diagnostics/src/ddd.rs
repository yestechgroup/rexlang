//! The `driver.ddd` band: `E0350`–`E0399`, the rex-driver `.ddd` lifecycle
//! checks (issues rexlang#176 and rexlang#180 — the service-operation
//! transition bindings and the `when`-condition scoping over
//! `crates/rex-driver/src/ddd/`). Every check prefixes its code onto the
//! diagnostic message (`E0350 unknown transition trigger …`), the
//! `dmn.check` band's E0250/E0251 precedent. E0350–E0352 are the
//! transition-binding checks; E0353–E0354 are the condition-scope checks.
//! Claim a new code by adding a documented constant plus an `Entry` to
//! `ENTRIES`.

use crate::{Entry, Severity};

/// The band's registry id ([`crate::all`] entries reference it).
pub const DDD_BAND: &str = "driver.ddd";

/// `unknown transition trigger '<trigger>' on operation '<op>' of service
/// '<service>'` — the transition binding names a trigger the machine bound
/// to the delegation target's repository design does not declare (an
/// unknown name, a qualifier that names a foreign machine's enum, or a
/// name both declared `via` and derived from a bare arrow — ambiguous, so
/// it binds nothing).
pub const E_UNKNOWN_TRANSITION_TRIGGER: &str = "E0350";
/// `transition on operation '<op>' of service '<service>', which …` — the
/// transition binding is legal only on a delegation to a repository `save`;
/// a declared operation, or a delegation to anything else, carries none.
pub const E_TRANSITION_NOT_SAVE: &str = "E0351";
/// `transition on operation '<op>' of service '<service>' has no machine
/// qualifier` — a multi-machine module's bindings must name the machine by
/// its enum type (`transition <EnumType>.<Trigger>`); the unqualified
/// spelling cannot say which machine it advances.
pub const E_TRANSITION_QUALIFIER_REQUIRED: &str = "E0352";
/// `unknown condition path '<path>' in <context>` — the condition path's
/// first segment names no scope binding of the operation (and no declared
/// injection it could be a typo of), or a qualified path names no field of
/// the injected repository's class.
pub const E_UNKNOWN_CONDITION_PATH: &str = "E0353";
/// `ambiguous machine scope: '<name>' is declared by more than one machine
/// in module '<module>'` — a condition references a state name (or a
/// state-typed field name) that more than one machine of the module
/// declares, so the unqualified spelling never binds; the qualified
/// spellings (`<EnumType>.<State>`, and lowerCamel(`<EnumType>`) for a
/// field) resolve it.
pub const E_AMBIGUOUS_MACHINE_SCOPE: &str = "E0354";

pub(crate) const ENTRIES: &[Entry] = &[
    Entry {
        code: E_UNKNOWN_TRANSITION_TRIGGER,
        band: DDD_BAND,
        severity: Severity::Error,
        title: "Unknown transition trigger",
        meaning: "`unknown transition trigger '<trigger>' on operation '<op>' of service '<service>'` — the operation's `transition` binding names no trigger of the machine bound to the delegation target's repository design: the spelling is a typo (a did-you-mean names the closest declared trigger when one is plausible), the qualifier names another machine's enum type, the target's design carries no machine, or the name is both declared `via` and derived from a bare arrow — ambiguous, so it binds nothing.",
        fix: "Name a trigger the machine declares — an explicit `via` name or a bare arrow's derived name (the lowerCamel of the target state) — and qualify it with the machine's enum type in a multi-machine module.",
    },
    Entry {
        code: E_TRANSITION_NOT_SAVE,
        band: DDD_BAND,
        severity: Severity::Error,
        title: "Transition on a non-save delegation",
        meaning: "`transition on operation '<op>' of service '<service>', which declares its own signature` or `… which delegates to <target>`.`<operation>`; only a `save` delegation carries a transition` — the transition binding fires the machine-bound transition of the aggregate a repository `save` persists, so a declared operation or a delegation to anything else has no machine to bind.",
        fix: "Delegate with `name => <Repository>.save` and bind the transition there, or drop the clause.",
    },
    Entry {
        code: E_TRANSITION_QUALIFIER_REQUIRED,
        band: DDD_BAND,
        severity: Severity::Error,
        title: "Transition qualifier required",
        meaning: "`transition on operation '<op>' of service '<service>' has no machine qualifier; module '<module>' carries <n> machine-bearing designs` — in a multi-machine module the unqualified spelling cannot say which machine the operation advances, so the binding is refused.",
        fix: "Qualify the trigger by the machine's enum type: `transition PayRunStatus.Submit`.",
    },
    Entry {
        code: E_UNKNOWN_CONDITION_PATH,
        band: DDD_BAND,
        severity: Severity::Error,
        title: "Unknown condition path",
        meaning: "`unknown condition path '<path>' in <context>` — the `when` condition's qualified path names no scope binding of the operation: the first segment is neither an injected repository of the service nor any other bound name, or a later segment names no field of the injected repository's designed class. A path that resolves but mistypes is not this error — the shared expression language's same-type rule reports it.",
        fix: "Declare the injection (`inject <Repository>;`), fix the spelling, or name a field the repository's class carries.",
    },
    Entry {
        code: E_AMBIGUOUS_MACHINE_SCOPE,
        band: DDD_BAND,
        severity: Severity::Error,
        title: "Ambiguous machine scope",
        meaning: "`ambiguous machine scope: '<name>' is declared by more than one machine in module '<module>'` — the condition references a state name or a state-typed field name that several machines of the module declare, so the unqualified spelling never binds (multi-machine modules admit op-level `when` guards only when every referenced name resolves uniquely). The qualified spellings resolve it: `<EnumType>.<State>` for a state, lowerCamel(`<EnumType>`) for a field.",
        fix: "Qualify the reference by its machine's enum type — `PayRunStatus.Draft` for a state, `payRunStatus` for a field.",
    },
];
