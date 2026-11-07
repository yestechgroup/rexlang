//! The `ifml.lints` band: `W0150`–`W0199`, the warning-severity lints of
//! the `.ifml` compiler (rex-ifml's `check` module's lint pass re-exports
//! these constants). Every lint considers **only the main file's**
//! declarations — imported pattern libraries declare unused
//! modules/actions by design and never warn. Claim a new code by adding a
//! documented constant plus an `Entry` to `ENTRIES` — the meanings and
//! fixes render into `docs/DIAGNOSTICS.md`.

use crate::{Entry, Severity};

/// The band's registry id ([`crate::all`] entries reference it).
pub const LINTS_BAND: &str = "ifml.lints";

/// A main-file `action "X" { ... }` declaration that no `action("X")`
/// invocation anywhere in the compilation names.
pub const W_DEAD_ACTION: &str = "W0150";
/// A main-file view that is not the entry view, not a landmark, carries no
/// body content, and no navigation reaches.
pub const W_UNREACHABLE_VIEW: &str = "W0151";
/// A main-file module declaration shadowing an imported module of the
/// same name.
pub const W_SHADOWED_MODULE: &str = "W0152";
/// A main-file component that binds no entity data, declares no events,
/// carries no chart, and spells no explicit entity columns.
pub const W_COMPONENT_WITHOUT_DATA: &str = "W0153";
/// A main-file view whose body carries no domain tie of any kind.
pub const W_VIEW_WITHOUT_DOMAIN: &str = "W0154";

pub(crate) const ENTRIES: &[Entry] = &[
    Entry {
        code: W_DEAD_ACTION,
        band: LINTS_BAND,
        severity: Severity::Warning,
        title: "Dead action",
        meaning: "A main-file `action \"X\" { ... }` declaration no `action(\"X\")` invocation names (`action 'X' is declared but never invoked`). References count anywhere in the compilation — an uninstantiated module's invocation is still live intent — and only the main file's declarations are judged, so pattern libraries never warn.",
        fix: "Invoke the action from an event handler, or delete the declaration.",
    },
    Entry {
        code: W_UNREACHABLE_VIEW,
        band: LINTS_BAND,
        severity: Severity::Warning,
        title: "Unreachable view",
        meaning: "A main-file view that is neither the first view (the entry view by position), nor `landmark: true`, nor reached by any `navigate` target anywhere in the compilation, and whose body carries no content — no components, containers, event handlers, or module uses (`view 'X' is never navigated to ...`). A content view is intentionally routed (the manifest exports every view as a route); the empty shell no navigation reaches is the dead-UI class.",
        fix: "Navigate to it from another view, mark it `landmark: true`, give it content, or delete it.",
    },
    Entry {
        code: W_SHADOWED_MODULE,
        band: LINTS_BAND,
        severity: Severity::Warning,
        title: "Shadowed module",
        meaning: "A main-file module declaration whose name equals an imported file's module declaration (`module 'X' shadows an imported module of the same name (from '<path>')`). Resolution deliberately lets the local declaration win silently; the warning documents the shadowing at the winning declaration's name.",
        fix: "Rename the local module (and its `use` sites), or drop it if the imported one is wanted.",
    },
    Entry {
        code: W_COMPONENT_WITHOUT_DATA,
        band: LINTS_BAND,
        severity: Severity::Warning,
        title: "Component without data",
        meaning: "A main-file component with no `data:` binding that also declares no event handlers, carries no chart (inline spec, chart reference, or `type: chart`), and spells no explicit `Entity.field` table column (`<context> component 'X' binds no entity data`) — a fully static placeholder. Interactive components wire their data flow through handlers; charts aggregate by construction; explicit-column tables use the generation-time `Class.feature` spelling — all exempt.",
        fix: "Bind `data: <Entity>;`, or leave as-is when the component is deliberately static.",
    },
    Entry {
        code: W_VIEW_WITHOUT_DOMAIN,
        band: LINTS_BAND,
        severity: Severity::Warning,
        title: "View without domain binding",
        meaning: "A main-file view with no domain tie at all: no `data:` binding on any component (recursively through containers), no explicit `Entity.field` column references, no declared `params`, no module composition, no event handlers anywhere in its body, and no `roles:`/`requires:` policy gate (`view 'X' has no domain binding ...`). Views that bind nothing are placeholders — often leftovers from a deleted flow.",
        fix: "Bind `data:` on a component, declare `params`, compose a module, add the interactions — or delete the view.",
    },
];
