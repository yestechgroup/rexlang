//! Single source of truth for the `.deploy` deployment DSL's contextual
//! keywords.
//!
//! Every `.deploy` contextual keyword is an ordinary identifier that is
//! special only in a particular grammar position (the `.ddd` discipline —
//! the `id`/`readonly` precedent), so two consumers must agree on the same
//! word list: the parser (which matches them positionally) and the
//! formatter (which recognizes them to dispatch its scans). This module is
//! the one table both consume. All words are escapable with `^`; the
//! component-kind and target *vocabularies* (`api`, `kubernetes`, ...) are
//! deliberately **not** listed here — they are driver-side validation, not
//! syntax, so a `.deploy` source may use them as ordinary names anywhere.

/// The `application <name> { ... }` keyword.
pub(crate) const APPLICATION: &str = "application";
/// The `component <name>: <kind> { ... }` keyword of an application body.
pub(crate) const COMPONENT: &str = "component";
/// The `connects <from> -> <to>` keyword of an application body.
pub(crate) const CONNECTS: &str = "connects";
/// The `profile <name> { ... }` keyword.
pub(crate) const PROFILE: &str = "profile";
/// The `deployment <name> for <application> { ... }` keyword.
pub(crate) const DEPLOYMENT: &str = "deployment";
/// The `target: <name>` line of a profile body.
pub(crate) const TARGET: &str = "target";
/// The `defaults { ... }` block of a profile body.
pub(crate) const DEFAULTS: &str = "defaults";
/// The `mappings { ... }` block of a profile body.
pub(crate) const MAPPINGS: &str = "mappings";
/// The `require (...)` policy keyword of a profile body.
pub(crate) const REQUIRE: &str = "require";
/// The `prohibit (...)` policy keyword of a profile body.
pub(crate) const PROHIBIT: &str = "prohibit";
/// The `requires <capability>, ...` clause of a component body.
pub(crate) const REQUIRES: &str = "requires";
/// The `design "<path>[#<Module>]"` binding of a component body.
pub(crate) const DESIGN: &str = "design";
/// The `flow "<path>[#<Module>]"` binding of a component body.
pub(crate) const FLOW: &str = "flow";
/// The `use <profile>` line of a deployment body.
pub(crate) const USE: &str = "use";
/// The `configure { ... }` block of a deployment body.
pub(crate) const CONFIGURE: &str = "configure";
/// The `for` word introducing a deployment's application (`deployment
/// production for Svc`).
pub(crate) const FOR: &str = "for";
