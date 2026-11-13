//! Cross-engine shared helpers (issue rexlang#162), additive to
//! [`crate::pipeline`] (the shared engine tail — engine_diags/engine_finish
//! — stays there). Import MATCHING itself ([`crate::import_matches`] and
//! each engine's per-import call pattern) stays per-engine; what centralizes
//! here is the hit-array construction over the provided buckets and the
//! diagnostics presentation, plus the smaller idioms the artifact-importing
//! engines share:
//!
//! * the import-hit tables (`.uc`/`.cmmn` four buckets, `.deploy` two) and
//!   their two diagnostics — the missing-import error and the
//!   alias-on-non-design error, whose help wording drifts per surface and
//!   is parameterized;
//! * design imports by alias (first aliased design match per alias wins);
//! * the actor-declaration queries ([`actor_known`] and the "the actor
//!   model declares: ..." capability help);
//! * the imported-domain type-union loop ([`union_types`]);
//! * the AST→IR multiplicity mapping shared by the `.mox`/`.ddd`/`.cmmn`
//!   engines ([`multiplicity_bounds`]);
//! * the host-side `dir_of` path normalization ([`dir_of`]);
//! * the seen-list uniqueness idiom ([`register_unique`] and
//!   [`register_unique_declared`]).

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use rex_ir::ddd::DddModel;
use rex_ir::events::EventModel;
use rex_ir::ifml::IfmlModel;
use rex_ir::ActorModel;
use rex_syntax::ast as dsl;
use rex_syntax::Span;

use crate::diagnostic::Diagnostic;
use crate::import_matches;
use crate::lower::DomainUnit;

/// The provided-artifact bucket an import hits when it names a `.ddd`
/// design — entry 1 of every bucket array, whatever the arity (the
/// four-bucket `.uc`/`.cmmn` arrays order domains, designs, actors,
/// events; the two-bucket `.deploy` array orders designs, flows).
pub(crate) const BUCKET_DESIGNS: usize = 1;

/// Which provided bucket each import of a four-bucket engine (`.uc`,
/// `.cmmn`) resolves against (exact path or lexical resolution, the
/// `.deploy` rule): domains, designs, actors, events.
pub(crate) fn four_bucket_hits(
    imports: &[dsl::Import],
    origin_path: &str,
    domains: &[DomainUnit],
    designs: &[(String, DddModel)],
    actors: &[(String, ActorModel)],
    events: &[(String, EventModel)],
) -> Vec<[bool; 4]> {
    imports
        .iter()
        .map(|import| {
            [
                domains
                    .iter()
                    .any(|unit| import_matches(&import.path, origin_path, &unit.path)),
                designs
                    .iter()
                    .any(|(provided, _)| import_matches(&import.path, origin_path, provided)),
                actors
                    .iter()
                    .any(|(provided, _)| import_matches(&import.path, origin_path, provided)),
                events
                    .iter()
                    .any(|(provided, _)| import_matches(&import.path, origin_path, provided)),
            ]
        })
        .collect()
}

/// The five-bucket `.cmmn` variant: the four use-case buckets plus the
/// decision models a `decision` task's `invokes` clause resolves against.
pub(crate) fn five_bucket_hits(
    imports: &[dsl::Import],
    origin_path: &str,
    domains: &[DomainUnit],
    designs: &[(String, DddModel)],
    actors: &[(String, ActorModel)],
    events: &[(String, EventModel)],
    decisions: &[(String, rex_ir::dmn::DmnModel)],
) -> Vec<[bool; 5]> {
    imports
        .iter()
        .map(|import| {
            [
                domains
                    .iter()
                    .any(|unit| import_matches(&import.path, origin_path, &unit.path)),
                designs
                    .iter()
                    .any(|(provided, _)| import_matches(&import.path, origin_path, provided)),
                actors
                    .iter()
                    .any(|(provided, _)| import_matches(&import.path, origin_path, provided)),
                events
                    .iter()
                    .any(|(provided, _)| import_matches(&import.path, origin_path, provided)),
                decisions
                    .iter()
                    .any(|(provided, _)| import_matches(&import.path, origin_path, provided)),
            ]
        })
        .collect()
}

/// The two-bucket `.deploy` variant: designs, then `.ifml` flows.
pub(crate) fn two_bucket_hits(
    imports: &[dsl::Import],
    origin_path: &str,
    designs: &[(String, DddModel)],
    flows: &[(String, IfmlModel)],
) -> Vec<[bool; 2]> {
    imports
        .iter()
        .map(|import| {
            [
                designs
                    .iter()
                    .any(|(provided, _)| import_matches(&import.path, origin_path, provided)),
                flows
                    .iter()
                    .any(|(provided, _)| import_matches(&import.path, origin_path, provided)),
            ]
        })
        .collect()
}

/// The shared import-hit diagnostics, one loop per bucket arity: every
/// import must name a provided artifact of some bucket
/// (`imported file "<path>" was not provided`, help per `missing_help`),
/// and — where the surface binds aliases — an alias is rejected on
/// everything but the designs bucket (`import "<path>" is not a design;
/// only design imports take an alias`, help per `alias_help`; `None` keeps
/// the `.deploy` rule, which admits aliases silently). `extra` runs for
/// imports that hit something and carried no alias rejection (the
/// `.deploy` unsupported-extension check), so per-import emission order is
/// preserved.
pub(crate) fn import_hit_diagnostics<const N: usize>(
    imports: &[dsl::Import],
    hits: &[[bool; N]],
    missing_help: Option<&str>,
    alias_help: Option<&str>,
    mut extra: impl FnMut(&dsl::Import) -> Option<Diagnostic>,
) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();
    for (import, hit) in imports.iter().zip(hits) {
        // Rule 1 — every import must name a provided artifact of any
        // surface.
        if !hit.iter().any(|hit| *hit) {
            let mut diagnostic = Diagnostic::error(
                format!("imported file \"{}\" was not provided", import.path),
                Some(import.span),
            );
            if let Some(help) = missing_help {
                diagnostic = diagnostic.with_help(help);
            }
            diagnostics.push(diagnostic);
            continue;
        }
        // Rule 1 — aliases exist for the surface's design binding
        // (`realizes`/`execute`); every other surface rejects them.
        if let Some(alias_help) = alias_help {
            if import.alias.is_some() && !hit[BUCKET_DESIGNS] {
                diagnostics.push(
                    Diagnostic::error(
                        format!(
                            "import \"{}\" is not a design; only design imports take an alias",
                            import.path
                        ),
                        Some(import.span),
                    )
                    .with_help(alias_help),
                );
                continue;
            }
        }
        if let Some(diagnostic) = extra(import) {
            diagnostics.push(diagnostic);
        }
    }
    diagnostics
}

/// Design imports by alias: the first aliased design match per alias wins
/// (rule 12 of the design engine, reused by the `.uc` and `.cmmn`
/// engines), so a duplicate alias binds the first import's design and the
/// rest contribute nothing. Generic over the bucket arity — the `.cmmn`
/// five-bucket hit table slots into the same helper (only the designs
/// column is read).
pub(crate) fn designs_by_alias<'a, const N: usize>(
    imports: &'a [dsl::Import],
    hits: &[[bool; N]],
    designs: &'a [(String, DddModel)],
    origin_path: &str,
) -> Vec<(&'a str, &'a DddModel)> {
    let mut by_alias: Vec<(&str, &DddModel)> = Vec::new();
    for (import, hit) in imports.iter().zip(hits) {
        let Some(alias) = &import.alias else {
            continue;
        };
        if !hit[BUCKET_DESIGNS] || by_alias.iter().any(|(seen, _)| seen == alias) {
            continue;
        }
        if let Some((_, design)) = designs
            .iter()
            .find(|(provided, _)| import_matches(&import.path, origin_path, provided))
        {
            by_alias.push((alias.as_str(), design));
        }
    }
    by_alias
}

/// `true` when some provided actor model declares the actor name. Every
/// member of an `extends` chain is a declared actor itself, so chain
/// members match by name directly.
pub(crate) fn actor_known(name: &str, actors: &[(String, ActorModel)]) -> bool {
    actors.iter().any(|(_, model)| {
        model
            .blocks
            .iter()
            .any(|block| block.actors.iter().any(|actor| actor.name == name))
    })
}

/// The capability names every provided actor model declares, pooled
/// union-wide (two blocks declaring the same capability name are one name
/// for every check).
pub(crate) fn declared_capabilities<'a>(
    actors: impl IntoIterator<Item = &'a ActorModel>,
) -> BTreeSet<String> {
    actors
        .into_iter()
        .flat_map(|model| model.blocks.iter())
        .flat_map(|block| block.capabilities.iter().map(|c| c.name.clone()))
        .collect()
}

/// The exhaustive help line for an unknown capability name: what the
/// actor model declares, or the empty-vocabulary wording when it declares
/// none.
pub(crate) fn declared_capabilities_help(declared: &BTreeSet<String>) -> String {
    if declared.is_empty() {
        "the actor model declares no capabilities".to_string()
    } else {
        format!(
            "the actor model declares: {}",
            declared.iter().cloned().collect::<Vec<_>>().join(", ")
        )
    }
}

/// The expression universe of the imported domains: every unit that
/// lowered, merged into one `DomainTypes` in import order. Callers add
/// their synthetic classes on top (`insert_class`) — never a fake `Model`.
pub(crate) fn union_types(domains: &[DomainUnit]) -> rex_expr::DomainTypes {
    let mut types = rex_expr::DomainTypes::default();
    for unit in domains {
        if let Some(model) = &unit.model {
            types.merge(rex_expr::DomainTypes::from_model(model));
        }
    }
    types
}

/// Which numeric slot a multiplicity bound conversion covers: an `Exact`
/// bound, a `Range` lower, or a `Range` upper (which receives the already
/// converted lower — the `.mox` engine's out-of-range upper falls back to
/// it).
pub(crate) enum MultiplicitySlot {
    Exact,
    Lower,
    Upper { lower: u32 },
}

/// The AST→IR multiplicity mapping shared by the `.mox`, `.ddd`, and
/// `.cmmn` engines: the annotation shape decides the bounds pair (`[]`
/// unbounded, `[n]` exact, `[a..b]`/`[a..*]` ranged), while how each
/// numeric bound becomes a `u32` is the caller's policy — the `.ddd`
/// engine clamps negatives to `0`, the `.mox` engine reports out-of-range
/// values with per-slot fallbacks, the `.cmmn` engine casts directly. The
/// `Option` wrapper and the bound diagnostics stay at the call sites as
/// thin wrappers.
pub(crate) fn multiplicity_bounds(
    kind: &dsl::MultiplicityKind,
    mut to_bound: impl FnMut(MultiplicitySlot, i64) -> u32,
) -> (u32, rex_ir::Upper) {
    match kind {
        dsl::MultiplicityKind::Unbounded => (0, rex_ir::Upper::Unbounded),
        dsl::MultiplicityKind::Exact(exact) => {
            let bound = to_bound(MultiplicitySlot::Exact, *exact);
            (bound, rex_ir::Upper::Finite(bound))
        }
        dsl::MultiplicityKind::Range(lower, upper) => {
            let lower = to_bound(MultiplicitySlot::Lower, *lower);
            let upper = match upper {
                dsl::MultBound::Star => rex_ir::Upper::Unbounded,
                dsl::MultBound::Int(bound) => {
                    rex_ir::Upper::Finite(to_bound(MultiplicitySlot::Upper { lower }, *bound))
                }
            };
            (lower, upper)
        }
    }
}

/// The importing file's directory, `"."` for a bare file name — the CLI's
/// `dir_of` rule (a bare filename's imports resolve next to the working
/// directory).
pub(crate) fn dir_of(path: impl AsRef<Path>) -> PathBuf {
    path.as_ref()
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."))
        .to_path_buf()
}

/// The seen-list uniqueness idiom shared by the engines' per-kind
/// uniqueness rules: `true` when `name` was new (and is now recorded in
/// `seen`, insertion order kept), `false` — with the diagnostic pushed —
/// when it duplicates an earlier entry. The message and help ride in
/// verbatim; callers keep their exact wording.
pub(crate) fn register_unique(
    seen: &mut Vec<String>,
    name: &str,
    span: Span,
    message: String,
    help: Option<&str>,
    local: &mut Vec<Diagnostic>,
) -> bool {
    if seen.iter().any(|seen| seen == name) {
        let mut diagnostic = Diagnostic::error(message, Some(span));
        if let Some(help) = help {
            diagnostic = diagnostic.with_help(help);
        }
        local.push(diagnostic);
        false
    } else {
        seen.push(name.to_string());
        true
    }
}

/// The declared-top-level-name variant: the seen list carries
/// `(name, declaration span)` rows and the message/help are the shared
/// `duplicate <label> '<name>'` / `'<name>' is already declared in this
/// file` pair every engine emits for top-level names.
pub(crate) fn register_unique_declared(
    seen: &mut Vec<(String, Span)>,
    name: &str,
    span: Span,
    label: &str,
    local: &mut Vec<Diagnostic>,
) -> bool {
    if seen.iter().any(|(seen, _)| seen == name) {
        local.push(
            Diagnostic::error(format!("duplicate {label} '{name}'"), Some(span))
                .with_help(format!("'{name}' is already declared in this file")),
        );
        false
    } else {
        seen.push((name.to_string(), span));
        true
    }
}
