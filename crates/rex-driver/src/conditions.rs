//! The shared `when`-condition pipeline (issue rexlang#161): the one
//! interior-slice/span-rebase/parse/type-check/triage skeleton behind six
//! surfaces — `.ddd` guards, `.cmmn` criteria, `.uc` step guards,
//! `.deploy` policies (the typing half), `.evt` subscription filters, and
//! the deferred capability conditions of the `.mox`/`.actor` engines.
//!
//! Every surface parses the sliced condition with [`rex_expr::parse`],
//! type-checks it against its own universe, and triages the result into
//! parse failure / boolean / non-boolean / type errors — but each keeps
//! EXACTLY its historical diagnostics: its message prefixes, its help
//! text, its span policy (a fixed span versus error spans rebased into
//! the paren interior), whether guard lints run, and its artifact
//! decision. The skeleton parameterizes all of that through
//! [`ConditionWording`] and [`ConditionSpanning`]; the checker
//! construction (self class, machine states, injections, per-event
//! universes, synthetic settings) stays at each call site.
//!
//! The surface-specific halves that are not typing stay where they were:
//! the `.ddd` path walker (`E0353`/`E0354` and `conditionPaths`, hooked
//! into the skeleton between parsing and typing), the events lint dedup
//! set, and the `.deploy` policy evaluation.

use rex_expr::TypeChecker;
use rex_syntax::Span;

use crate::diagnostic::Diagnostic;

/// The verbatim text inside a `when (...)` condition: strictly inside the
/// parens, ends-trimmed (the same span-to-text convention as target bodies).
pub(crate) fn when_text(source: &str, span: Span) -> &str {
    source[span.start + 1..span.end - 1].trim()
}

/// The verbatim text inside a `when (...)` condition, sliced defensively:
/// strictly inside the parens, ends-trimmed, empty when the span is too
/// short to name an interior (the `.ddd`/`.cmmn`/`.uc`/`.evt` slicing
/// convention).
pub(crate) fn when_interior(source: &str, span: Span) -> &str {
    source
        .get(span.start + 1..span.end.saturating_sub(1))
        .unwrap_or("")
        .trim()
}

/// A `when` condition awaiting type checking once the class universe is
/// known. Lowering only syntax-checks conditions (so parse errors keep their
/// source order); the type check runs after every class is lowered.
pub(crate) struct PendingCondition {
    /// The file whose spans the condition refers to.
    pub(crate) file: String,
    /// The trimmed condition text.
    pub(crate) text: String,
    /// Host-file offset of the condition text's first byte.
    pub(crate) base: usize,
    /// The condition's inner region (strictly between the parens); mapped
    /// spans are clamped to it.
    pub(crate) inner_start: usize,
    pub(crate) inner_end: usize,
    /// The capability's owning class (package, name); `None` when the
    /// capability's class failed to resolve (that error is already reported).
    pub(crate) class: Option<(String, String)>,
}

/// Slices a `when (...)` span into a deferred type-check entry.
pub(crate) fn pending_condition(
    file: &str,
    source: &str,
    when_span: Span,
    class: Option<(String, String)>,
) -> PendingCondition {
    let raw = &source[when_span.start + 1..when_span.end - 1];
    let text = raw.trim();
    PendingCondition {
        file: file.to_string(),
        text: text.to_string(),
        base: when_span.start + 1 + (raw.len() - raw.trim_start().len()),
        inner_start: when_span.start + 1,
        inner_end: when_span.end - 1,
        class,
    }
}

/// How a surface spans its condition diagnostics: the whole paren span
/// verbatim, or error spans rebased into the paren interior. The two
/// rebase arithmetics are the historical pair — the pending conditions
/// clamp into the trimmed interior, the `.ddd` guards saturate from the
/// open paren and cap at the close paren — and must never merge.
pub(crate) enum ConditionSpanning {
    /// Every diagnostic carries one fixed span (the paren region, the
    /// deployment declaration, ...).
    Fixed(Span),
    /// The pending-condition arithmetic: `(base + offset)` clamped into
    /// `[inner_start, inner_end]`. The non-boolean diagnostic spans the
    /// interior exactly (`inner_start..inner_end`).
    RebasedClamp {
        /// Host-file offset of the trimmed text's first byte.
        base: usize,
        /// The condition's inner region (strictly between the parens).
        inner_start: usize,
        inner_end: usize,
    },
    /// The `.ddd` arithmetic: `inner_start + offset` saturating, capped at
    /// the close paren. The non-boolean diagnostic spans
    /// `inner_start..cap`.
    RebasedCapped {
        /// Host-file offset just inside the open paren.
        inner_start: usize,
        /// The cap: the offset just past the close paren.
        cap: usize,
    },
}

impl ConditionSpanning {
    /// The span for one parser/checker error (or guard lint): fixed, or
    /// rebased into the interior.
    fn error_span(&self, error: &rex_expr::ast::Span) -> Span {
        match self {
            ConditionSpanning::Fixed(span) => *span,
            ConditionSpanning::RebasedClamp {
                base,
                inner_start,
                inner_end,
            } => {
                let start = (base + error.start).clamp(*inner_start, *inner_end);
                let end = (base + error.end).clamp(start, *inner_end);
                (start..end).into()
            }
            ConditionSpanning::RebasedCapped { inner_start, cap } => {
                let start = inner_start.saturating_add(error.start).min(*cap);
                let end = inner_start.saturating_add(error.end).clamp(start, *cap);
                (start..end).into()
            }
        }
    }

    /// The span of the must-be-boolean diagnostic: the fixed span, or the
    /// surface's interior region.
    fn interior_span(&self) -> Span {
        match self {
            ConditionSpanning::Fixed(span) => *span,
            ConditionSpanning::RebasedClamp {
                inner_start,
                inner_end,
                ..
            } => (*inner_start..*inner_end).into(),
            ConditionSpanning::RebasedCapped { inner_start, cap } => (*inner_start..*cap).into(),
        }
    }
}

/// How the skeleton treats parse errors.
pub(crate) enum ParseErrorPolicy {
    /// Emit one diagnostic per parse error (prefix + span + parse help).
    Emit,
    /// Parse errors were already reported during lowering: emit nothing
    /// here, and do not type a condition that still carries any (the
    /// deferred pending-condition contract).
    AlreadyReported,
    /// Parse errors were already reported elsewhere: emit nothing here,
    /// but still type the recovered expression (the `.evt` per-event
    /// loop, which re-enters the skeleton after the parse-error pass).
    ReportedElsewhere,
}

/// How the ok-boolean path words its guard string lints (issue #142).
pub(crate) enum LintWording {
    /// The raw lint message (`.cmmn`, `.uc`).
    Raw,
    /// The surface prefix, then the raw message (capability `when`,
    /// `.ddd` labels its guards).
    Prefixed(String),
    /// The raw message, then the surface suffix (`.evt` names the
    /// subscription).
    Suffixed(String),
    /// The surface runs no guard lints (`.deploy` policies).
    Skip,
}

/// The per-surface wording of the skeleton's diagnostics: message
/// prefixes, the must-be-boolean prefix, help texts, and the lint
/// policy. Emission ORDER is the skeleton's (parse errors, walker
/// diagnostics, type errors, the non-boolean diagnostic, lints last) and
/// never varies; only this wording does.
pub(crate) struct ConditionWording {
    /// How parse errors surface.
    pub(crate) parse_errors: ParseErrorPolicy,
    /// The prefix before a parser/checker error message (and, on the
    /// surfaces that prefix type errors, before those too).
    pub(crate) parse_prefix: String,
    /// Type errors keep the checker's message raw when set (`.cmmn`,
    /// `.uc`, `.evt`); otherwise they take `parse_prefix` (`.ddd`,
    /// `.deploy`, capability `when`).
    pub(crate) type_errors_raw: bool,
    /// The prefix before `must be boolean, found {ty}` (`"criterion
    /// condition "`, `"when filter "`,
    /// `"invalid condition in {label}: condition "`, ...).
    pub(crate) boolean_prefix: String,
    /// The guard-lint wording.
    pub(crate) lint: LintWording,
    /// The help text of the parse-error diagnostics, when the surface
    /// attaches one.
    pub(crate) parse_help: Option<String>,
    /// The help text of the typing diagnostics (type errors, the
    /// non-boolean diagnostic, lints), when the surface attaches one.
    pub(crate) typed_help: Option<String>,
}

/// The skeleton's triage verdict — the piece each surface's artifact
/// decision reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ConditionTriage {
    /// The text did not parse (or the deferred contract skipped it): the
    /// parse-failure artifact rule applies.
    Unparsed,
    /// Parsed, but the caller supplied no checker: typing defers.
    Deferred,
    /// Typed as boolean; only lints were (or were not) produced.
    Boolean,
    /// Typed but not boolean; the must-be-boolean diagnostic was emitted.
    NonBoolean,
    /// Typing produced errors (minus walker-suppressed spans), emitted.
    TypeErrors,
}

/// What a skeleton call hands back: the triage verdict, the parsed
/// expression (the `.deploy` evaluation half re-uses it), and the
/// ok-boolean path's guard lints. Lints are returned rather than emitted
/// so the `.evt` call site can dedup them across its per-event universes;
/// every other surface simply extends its sink, order preserved (lints
/// are the skeleton's last diagnostics on every surface).
pub(crate) struct ConditionOutcome {
    pub(crate) triage: ConditionTriage,
    /// The parsed expression, `None` when the text did not parse (or the
    /// deferred contract skipped it).
    pub(crate) ast: Option<rex_expr::Expr>,
    pub(crate) lints: Vec<Diagnostic>,
}

/// The path walker's contribution (the `.ddd` surface only): diagnostics
/// emitted between the parse-error and typing phases, and the
/// expression-relative spans whose checker errors the walker already
/// diagnosed (`E0353`/`E0354`) — the skeleton drops those duplicates.
pub(crate) struct WalkerPhase {
    pub(crate) suppressed: Vec<rex_expr::ast::Span>,
    pub(crate) diagnostics: Vec<Diagnostic>,
}

/// Attaches the help text when the surface carries one.
fn with_help(diagnostic: Diagnostic, help: Option<&str>) -> Diagnostic {
    match help {
        Some(help) => diagnostic.with_help(help),
        None => diagnostic,
    }
}

/// The shared parse + type-check + triage skeleton. Slicing is the
/// caller's ([`when_text`]/[`when_interior`]/[`pending_condition`] — the
/// surfaces slice at different moments); everything from the parse call
/// to the triage is the skeleton's.
pub(crate) fn check_condition(
    text: &str,
    spanning: ConditionSpanning,
    wording: ConditionWording,
    checker: Option<TypeChecker>,
    walker: Option<&mut dyn FnMut(&rex_expr::Expr) -> WalkerPhase>,
    emit: &mut Vec<Diagnostic>,
) -> ConditionOutcome {
    let parsed = rex_expr::parse(text);
    let mut deferred = false;
    match wording.parse_errors {
        ParseErrorPolicy::Emit => {
            for error in &parsed.errors {
                emit.push(with_help(
                    Diagnostic::error(
                        format!("{}{}", wording.parse_prefix, error.message),
                        Some(spanning.error_span(&error.span)),
                    ),
                    wording.parse_help.as_deref(),
                ));
            }
        }
        ParseErrorPolicy::AlreadyReported => {
            deferred = !parsed.errors.is_empty();
        }
        ParseErrorPolicy::ReportedElsewhere => {}
    }
    let ast = if deferred { None } else { parsed.ast };
    let Some(ast) = ast else {
        return ConditionOutcome {
            triage: ConditionTriage::Unparsed,
            ast: None,
            lints: Vec::new(),
        };
    };
    let mut suppressed = Vec::new();
    if let Some(walker) = walker {
        let phase = walker(&ast);
        suppressed = phase.suppressed;
        emit.extend(phase.diagnostics);
    }
    let Some(checker) = checker else {
        return ConditionOutcome {
            triage: ConditionTriage::Deferred,
            ast: Some(ast),
            lints: Vec::new(),
        };
    };
    match checker.type_of(&ast) {
        Err(errors) => {
            for error in errors {
                if suppressed.contains(&error.span) {
                    // The path walker diagnosed this reference (E0353/E0354);
                    // the checker's unknown-name/unknown-feature error is
                    // the duplicate, not a second finding.
                    continue;
                }
                let message = if wording.type_errors_raw {
                    error.message.clone()
                } else {
                    format!("{}{}", wording.parse_prefix, error.message)
                };
                emit.push(with_help(
                    Diagnostic::error(message, Some(spanning.error_span(&error.span))),
                    wording.typed_help.as_deref(),
                ));
            }
            ConditionOutcome {
                triage: ConditionTriage::TypeErrors,
                ast: Some(ast),
                lints: Vec::new(),
            }
        }
        // A guard is a predicate: anything that types but is not boolean
        // is a mistake every hosting surface refuses.
        Ok(ty) if ty != rex_expr::Ty::boolean() => {
            emit.push(with_help(
                Diagnostic::error(
                    format!("{}must be boolean, found {ty}", wording.boolean_prefix),
                    Some(spanning.interior_span()),
                ),
                wording.typed_help.as_deref(),
            ));
            ConditionOutcome {
                triage: ConditionTriage::NonBoolean,
                ast: Some(ast),
                lints: Vec::new(),
            }
        }
        Ok(_) => {
            // The use-based guard lint (issue #142): a String-typed operand
            // compared to a string literal warns that the field could be an
            // enum. Type-driven, never name-based; `Skip` surfaces run no
            // lints, and the lint defers silently whenever a type cannot be
            // resolved (no lints are produced then).
            let lints = match &wording.lint {
                LintWording::Skip => Vec::new(),
                policy => checker
                    .guard_string_lints(&ast)
                    .into_iter()
                    .map(|lint| {
                        let message = match policy {
                            LintWording::Raw => lint.message.clone(),
                            LintWording::Prefixed(prefix) => {
                                format!("{prefix}{}", lint.message)
                            }
                            LintWording::Suffixed(suffix) => format!("{}{suffix}", lint.message),
                            LintWording::Skip => unreachable!("filtered above"),
                        };
                        with_help(
                            Diagnostic::warning(message, Some(spanning.error_span(&lint.span))),
                            wording.typed_help.as_deref(),
                        )
                    })
                    .collect(),
            };
            ConditionOutcome {
                triage: ConditionTriage::Boolean,
                ast: Some(ast),
                lints,
            }
        }
    }
}

/// Syntax-checks a `when` condition with the neutral expression parser
/// (`rex-expr`, syntax only — bare condition names are resource features
/// whose typing is a backend concern). Every parse error is re-spanned into
/// the file: the error's byte offsets are relative to the condition text, so
/// they are shifted onto the condition's inner region (clamped to it).
pub(crate) fn check_when_syntax(source: &str, when_span: Span, diags: &mut Vec<Diagnostic>) {
    let raw = &source[when_span.start + 1..when_span.end - 1];
    let text = raw.trim();
    let base = when_span.start + 1 + (raw.len() - raw.trim_start().len());
    let wording = ConditionWording {
        parse_errors: ParseErrorPolicy::Emit,
        parse_prefix: "invalid `when` condition: ".to_string(),
        type_errors_raw: false,
        boolean_prefix: String::new(),
        lint: LintWording::Skip,
        parse_help: None,
        typed_help: None,
    };
    let spanning = ConditionSpanning::RebasedClamp {
        base,
        inner_start: when_span.start + 1,
        inner_end: when_span.end - 1,
    };
    check_condition(text, spanning, wording, None, None, diags);
}

/// Type-checks every deferred `when` condition against its resolved
/// capability class per docs/EXPRESSIONS.md: bare names are that class's
/// features, options propagate per R3, literals adapt per L1, and so on.
/// Parse errors were already reported during lowering. The condition must
/// type as `boolean`. Diagnostics are tagged with the host file.
pub(crate) fn check_pending_conditions(
    pending: &[PendingCondition],
    context: &rex_expr::TypeContext,
) -> Vec<(String, Diagnostic)> {
    let mut diags = Vec::new();
    for condition in pending {
        let Some((package, class)) = &condition.class else {
            continue;
        };
        let checker = TypeChecker::new(context.clone()).with_self(package, class);
        let wording = ConditionWording {
            parse_errors: ParseErrorPolicy::AlreadyReported,
            parse_prefix: "invalid `when` condition: ".to_string(),
            type_errors_raw: false,
            boolean_prefix: "invalid `when` condition: condition ".to_string(),
            lint: LintWording::Prefixed("invalid `when` condition: ".to_string()),
            parse_help: None,
            typed_help: None,
        };
        let spanning = ConditionSpanning::RebasedClamp {
            base: condition.base,
            inner_start: condition.inner_start,
            inner_end: condition.inner_end,
        };
        let mut local = Vec::new();
        let outcome = check_condition(
            &condition.text,
            spanning,
            wording,
            Some(checker),
            None,
            &mut local,
        );
        for diagnostic in local {
            diags.push((condition.file.clone(), diagnostic));
        }
        for lint in outcome.lints {
            diags.push((condition.file.clone(), lint));
        }
    }
    diags
}
