//! rex-diagnostics — the central diagnostic-code registry.
//!
//! Every diagnostic code any rexlang surface emits is defined **here**
//! and nowhere else: a code is a documented `&str` constant plus an
//! [`Entry`] in its band module, and a band is claimed by a row in the
//! allocation ledger ([`ledger`]). Surfaces re-export the constants they
//! own (`rex_ifml::resolve` re-exports the `ifml.resolver` band) or look
//! them up dynamically ([`lookup`]); the CLI's `rexlang explain` command
//! renders any entry, and `docs/DIAGNOSTICS.md` is generated from this
//! registry (byte-pinned by a test).
//!
//! # Two code schemes, one registry
//!
//! Numeric bands (`E####` errors, `W####` warnings, `X####` cross-surface
//! integrity) are **surface-local, human-facing codes**: the CLI prefixes
//! them onto messages and humans quote them in issues. The driver's
//! camelCase names (`attributeWithClassType` and friends) are
//! **payload-carrying LSP codes**: they travel as
//! `rex_driver::DiagnosticCode` values whose serialization is pinned by
//! goldens, so structured tooling can consume the payload. Both schemes
//! are first-class registry entries — the numeric bands by range, the
//! camelCase names by registration in the `driver.payload` band — and
//! `rexlang explain` resolves both.
//!
//! # Allocation contract
//!
//! To allocate a code: claim (or find) your band in [`ledger`], add a
//! documented constant plus its [`Entry`] to the owning band module's
//! `ENTRIES`, and add the band's ledger row if it is new. A numeric
//! band's next-free pointer is one past its highest registered code (or
//! the band start when empty); the registry tests pin ledger honesty and
//! regenerate `docs/DIAGNOSTICS.md`. The crate is dependency-free on
//! purpose, so every surface can depend on it.

use std::sync::LazyLock;

pub mod ddd;
pub mod dmn;
pub mod domain;
pub mod driver;
pub mod ifml_check;
pub mod ifml_lints;
pub mod ifml_resolver;
pub mod integrity;
pub mod suggest;

pub use ddd::*;
pub use dmn::*;
pub use domain::*;
pub use driver::*;
pub use ifml_check::*;
pub use ifml_lints::*;
pub use ifml_resolver::*;
pub use integrity::*;
pub use suggest::*;

/// How severe a diagnostic is — each registry entry's severity class.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    /// A problem that prevents compilation.
    Error,
    /// A suspicious construct that does not prevent compilation.
    Warning,
}

impl Severity {
    /// `true` when the severity is [`Severity::Error`].
    pub fn is_error(self) -> bool {
        matches!(self, Severity::Error)
    }

    /// The lowercase class name the CLI prints (`error` / `warning`).
    pub fn as_str(self) -> &'static str {
        match self {
            Severity::Error => "error",
            Severity::Warning => "warning",
        }
    }
}

/// One registered diagnostic code: its identity, its band, and the
/// human-facing documentation `rexlang explain` and `docs/DIAGNOSTICS.md`
/// render.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Entry {
    /// The stable code string users quote (`E0104`,
    /// `attributeWithClassType`).
    pub code: &'static str,
    /// The id of the band this code belongs to ([`Band::id`]).
    pub band: &'static str,
    /// The severity class every diagnostic carrying this code has.
    pub severity: Severity,
    /// Short noun phrase naming the problem.
    pub title: &'static str,
    /// What the diagnostic means (message shape included).
    pub meaning: &'static str,
    /// How to fix it.
    pub fix: &'static str,
}

/// One allocated band of the ledger: a range a surface owns.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Band {
    /// The registry id entries reference (`ifml.resolver`).
    pub id: &'static str,
    /// The human-readable range (`E0001–E0099`); camelCase bands carry a
    /// name instead of a numeric range.
    pub range: &'static str,
    /// The surface that owns the band.
    pub owner: &'static str,
    /// The severity classes this band's diagnostics use.
    pub severity_note: &'static str,
    /// The next free code of a numeric band (one past its highest
    /// registered code, or the band start when empty); `n/a` for the
    /// camelCase band.
    pub next_free: &'static str,
}

/// Every band's entries, inner order = ledger order.
const BAND_ENTRIES: &[&[Entry]] = &[
    ifml_resolver::ENTRIES,
    ifml_check::ENTRIES,
    ifml_lints::ENTRIES,
    driver::ENTRIES,
    integrity::ENTRIES,
    dmn::ENTRIES,
    domain::ENTRIES,
    ddd::ENTRIES,
];

/// All registered entries, flattened once.
static ALL_ENTRIES: LazyLock<Vec<Entry>> = LazyLock::new(|| {
    BAND_ENTRIES
        .iter()
        .copied()
        .flatten()
        .copied()
        .collect::<Vec<Entry>>()
});

/// Every registered entry, in band order (ledger order within bands).
pub fn all() -> &'static [Entry] {
    &ALL_ENTRIES
}

/// Looks up one registered entry by its code string — numeric
/// (`E0104`) or camelCase (`attributeWithClassType`).
pub fn lookup(code: &str) -> Option<&'static Entry> {
    all().iter().find(|entry| entry.code == code)
}

/// The allocation ledger: one row per band. A numeric band's `next_free`
/// is one past its highest registered code (its range start while
/// empty); the registry tests pin the pointers.
const BANDS: &[Band] = &[
    Band {
        id: "ifml.resolver",
        range: "E0001–E0099",
        owner: "rex-ifml resolver",
        severity_note: "error",
        next_free: "E0010",
    },
    Band {
        id: "ifml.check",
        range: "E0100–E0149",
        owner: "rex-ifml typed checker",
        severity_note: "error",
        next_free: "E0112",
    },
    Band {
        id: "ifml.lints",
        range: "W0150–W0199",
        owner: "rex-ifml lints",
        severity_note: "warning",
        next_free: "W0155",
    },
    Band {
        id: "driver.payload",
        range: "camelCase names",
        owner: "rex-driver DiagnosticCode",
        severity_note: "error",
        next_free: "n/a",
    },
    Band {
        id: "driver.integrity",
        range: "X0001–X0099",
        owner: "rex-driver integrity pass",
        severity_note: "error",
        next_free: "X0009",
    },
    Band {
        id: "dmn.check",
        range: "E0200–E0299",
        owner: "rex-driver .dmn surface and .ddd decision links",
        severity_note: "error",
        next_free: "E0255",
    },
    Band {
        id: "driver.domain",
        range: "E0300–E0349",
        owner: "rex-driver .mox domain-surface checks",
        severity_note: "error",
        next_free: "E0302",
    },
    Band {
        id: "driver.ddd",
        range: "E0350–E0399",
        owner: "rex-driver `.ddd` lifecycle checks",
        severity_note: "error",
        next_free: "E0361",
    },
];

/// The allocation ledger: one row per band — a new band is a row here,
/// and a new code is an entry in its band module, never a local constant.
pub fn ledger() -> &'static [Band] {
    BANDS
}

/// The intro of the generated `docs/DIAGNOSTICS.md`.
const DOC_INTRO: &str = "Every diagnostic a rexlang surface emits carries a stable string code. \
The codes live in one place — the `rex-diagnostics` crate — and this page \
is generated from that registry: a new code is a table row in \
`rex-diagnostics`, never a local constant. Two schemes are registered side \
by side: the numeric bands (`E####` errors, `W####` warnings, `X####` \
cross-surface integrity) are surface-local, human-facing codes the CLI \
prints, while the driver's camelCase names are payload-carrying LSP \
diagnostic codes (`rex_driver::DiagnosticCode`); both are first-class \
registry entries, cross-referenced by band. To allocate a code, claim (or \
find) your band in the ledger and add the constant plus its entry to the \
band module. Run `rexlang explain <code>` for one entry, or `rexlang \
explain --list` for everything.";

/// Renders the registry as the markdown of `docs/DIAGNOSTICS.md`: title,
/// intro, the allocation ledger, and one table per band (`Code | Class |
/// Title | Meaning | Fix`).
pub fn render_markdown() -> String {
    let mut out = String::new();
    out.push_str("# Diagnostic codes\n\n");
    out.push_str(DOC_INTRO);
    out.push_str("\n\n## Allocation ledger\n\n");
    out.push_str("| Band | Range | Owner | Severity | Next free |\n");
    out.push_str("|------|-------|-------|----------|-----------|\n");
    for band in ledger() {
        out.push_str(&format!(
            "| `{}` | {} | {} | {} | {} |\n",
            band.id, band.range, band.owner, band.severity_note, band.next_free
        ));
    }
    for band in ledger() {
        out.push_str(&format!("\n## Band `{}` ({})\n\n", band.id, band.range));
        let entries: Vec<&Entry> = all().iter().filter(|entry| entry.band == band.id).collect();
        if entries.is_empty() {
            out.push_str(&format!(
                "No codes allocated yet — the next free code is `{}`.\n",
                band.next_free
            ));
            continue;
        }
        out.push_str("| Code | Class | Title | Meaning | Fix |\n");
        out.push_str("|------|-------|-------|---------|-----|\n");
        for entry in entries {
            out.push_str(&format!(
                "| `{}` | {} | {} | {} | {} |\n",
                entry.code,
                entry.severity.as_str(),
                cell(entry.title),
                cell(entry.meaning),
                cell(entry.fix)
            ));
        }
    }
    out
}

/// Escapes a cell's text for a markdown table row.
fn cell(text: &str) -> String {
    text.replace('|', "\\|")
}
