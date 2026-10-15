//! Single source of truth for the `.uc` use case DSL's contextual keywords.
//!
//! Every `.uc` contextual keyword is an ordinary identifier that is special
//! only in a particular grammar position (the `.ddd` discipline — the
//! `id`/`readonly` precedent), so two consumers must agree on the same word
//! list: the parser (which matches them positionally) and the formatter
//! (which recognizes them to dispatch its scans and to reorder a usecase
//! body). This module is the one table both consume. All words are escapable
//! with `^`.
//!
//! Three more words of the grammar are *not* listed here because they are
//! real lexer keywords: `actor` (the participant introducer, [`Token::Actor`]
//! in `crate::lexer`), `import`, and the `from` preposition of a message
//! (`Token::From`).

/// The `usecase <name> { ... }` keyword.
pub(crate) const USECASE: &str = "usecase";
/// The `subject <name>: <type>` keyword of a usecase body.
pub(crate) const SUBJECT: &str = "subject";
/// The `occurrence <name> [.<end>];` keyword of a participant body.
pub(crate) const OCCURRENCE: &str = "occurrence";
/// The `perform <name> [: <op>]` keyword of a participant body.
pub(crate) const PERFORM: &str = "perform";
/// The `message <name>` keyword of a usecase body.
pub(crate) const MESSAGE: &str = "message";
/// The `of <type>` payload clause of a message.
pub(crate) const OF: &str = "of";
/// The `to <endpoint>` clause of a message and the `to` step of a
/// succession (`first ... then ...` uses `then`, not `to`).
pub(crate) const TO: &str = "to";
/// The `first <node>` keyword of a succession.
pub(crate) const FIRST: &str = "first";
/// The `then <node>` keyword of a succession.
pub(crate) const THEN: &str = "then";
/// The `start` node reference of a succession.
pub(crate) const START: &str = "start";
/// The `done` node reference of a succession.
pub(crate) const DONE: &str = "done";
/// The `requires [ ... ]` clause of a perform declaration.
pub(crate) const REQUIRES: &str = "requires";
/// The `realizes alias.Service.operation;` keyword of a usecase body.
pub(crate) const REALIZES: &str = "realizes";
/// The message-end tail of an occurrence (`occurrence m.source`).
pub(crate) const SOURCE: &str = "source";
/// The message-end tail of an occurrence (`occurrence m.target`).
pub(crate) const TARGET: &str = "target";
