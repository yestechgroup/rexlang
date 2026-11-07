//! Single source of truth for the `.cmmn` case-management DSL's contextual
//! keywords.
//!
//! Every `.cmmn` contextual keyword is an ordinary identifier that is
//! special only in a particular grammar position (the `.ddd` discipline —
//! the `id`/`readonly` precedent), so two consumers must agree on the same
//! word list: the parser (which matches them positionally) and the
//! formatter (which recognizes them to dispatch its scans and to reorder a
//! case body). This module is the one table both consume. All words are
//! escapable with `^`.
//!
//! Four more words of the grammar are *not* listed here because they are
//! real lexer keywords: `import` and the `as` alias introducer
//! ([`Token::Import`]/[`Token::As`]), and the `on`/`when` words of a
//! criterion ([`Token::On`]/[`Token::When`] — the `.actor` condition
//! keywords).
//!
//! The transition words of an on-part (`create`, `complete`, `update`, ...)
//! are deliberately **not** listed here: both segments of an on-part are
//! plain names and the driver validates the tail against its closed
//! transition tables (the `.deploy` vocabulary discipline), so that word
//! list lives in the driver, not here.

/// The `case <name> { ... }` keyword.
pub(crate) const CASE: &str = "case";
/// The `roles { ... }` keyword of a case body.
pub(crate) const ROLES: &str = "roles";
/// The `role <name> (as <actor>)?;` keyword of a roles block.
pub(crate) const ROLE: &str = "role";
/// The `case_file { ... }` keyword of a case body.
pub(crate) const CASE_FILE: &str = "case_file";
/// The `entry` criterion introducer of a plan item body.
pub(crate) const ENTRY: &str = "entry";
/// The `exit` criterion introducer of a plan item body.
pub(crate) const EXIT: &str = "exit";
/// The `performer: <role>;` clause of a task body.
pub(crate) const PERFORMER: &str = "performer";
/// The `execute <alias>.<Service>.<operation>;` clause of a service body.
pub(crate) const EXECUTE: &str = "execute";
/// The `required;` control flag of a plan item body.
pub(crate) const REQUIRED: &str = "required";
/// The `repeats;` control flag of a plan item body.
pub(crate) const REPEATS: &str = "repeats";
/// The `auto;` control flag of a plan item body (auto-start).
pub(crate) const AUTO: &str = "auto";
/// The `discretionary` prefix of a task or service declaration.
pub(crate) const DISCRETIONARY: &str = "discretionary";
/// The `stage <name> { ... }` keyword.
pub(crate) const STAGE: &str = "stage";
/// The `task <name> { ... }` keyword.
pub(crate) const TASK: &str = "task";
/// The `service <name> { ... }` keyword.
pub(crate) const SERVICE: &str = "service";
/// The `milestone <name> { ... }` keyword.
pub(crate) const MILESTONE: &str = "milestone";
