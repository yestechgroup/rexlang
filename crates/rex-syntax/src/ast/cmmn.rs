//! The `.cmmn` case-management DSL AST.

use super::*;

// --- .cmmn case management ---------------------------------------------------

/// A `.cmmn` file: `import` declarations followed by `case` declarations
/// (imports must come first).
#[derive(Debug, Clone, PartialEq)]
pub struct CmmnFile {
    /// The `import` declarations in source order.
    pub imports: Vec<Import>,
    /// Declared cases, in source order.
    pub cases: Vec<CmmnCase>,
}

/// A `case <name> { ... }` declaration: one case definition — its roles,
/// typed case file, and plan items. The interleaved source order of the
/// member kinds is not preserved; each kind is collected into its own
/// source-ordered list.
#[derive(Debug, Clone, PartialEq)]
pub struct CmmnCase {
    /// The case name.
    pub name: String,
    /// Span of the declared case name itself (the navigation index's
    /// definition site).
    pub name_span: Span,
    /// Description from the doc comment directly above the declaration
    /// (contiguous `///` lines, joined).
    pub description: Option<String>,
    /// Declared roles in source order.
    pub roles: Vec<CmmnRole>,
    /// Declared case-file items in source order.
    pub case_file: Vec<CmmnFileItem>,
    /// Declared top-level plan items in source order (stages nest their
    /// children).
    pub plan_items: Vec<CmmnItemDecl>,
    /// Span of the whole declaration.
    pub span: Span,
}

/// A `role <name> (as <actor>)?;` declaration: a named case role,
/// optionally bound to an `.actor` surface actor.
#[derive(Debug, Clone, PartialEq)]
pub struct CmmnRole {
    /// The role name.
    pub name: String,
    /// Span of the declared role name itself.
    pub name_span: Span,
    /// The bound actor name from the `as` clause, when present.
    pub actor: Option<String>,
    /// Span of the `as` actor mention as written, when present.
    pub actor_span: Option<Span>,
    /// Span of the whole declaration.
    pub span: Span,
}

/// A case-file item declaration: `Type[mult]? name;` (the `.mox` attribute
/// production, type-first).
#[derive(Debug, Clone, PartialEq)]
pub struct CmmnFileItem {
    /// The item name.
    pub name: String,
    /// Span of the declared item name itself.
    pub name_span: Span,
    /// The item's type reference.
    pub type_ref: TypeRef,
    /// The multiplicity annotation, when present.
    pub multiplicity: Option<Multiplicity>,
    /// Description from the doc comment directly above the declaration.
    pub description: Option<String>,
    /// Span of the whole declaration.
    pub span: Span,
}

/// A plan item declaration: a `task`, `service`, `decision`, `milestone`,
/// or `stage` (optionally prefixed `discretionary` for tasks, services,
/// and decisions), with its body members folded into per-kind fields.
#[derive(Debug, Clone, PartialEq)]
pub struct CmmnItemDecl {
    /// The plan item name.
    pub name: String,
    /// Span of the declared plan item name itself.
    pub name_span: Span,
    /// Description from the doc comment directly above the declaration.
    pub description: Option<String>,
    /// `true` when declared with the `discretionary` prefix.
    pub discretionary: bool,
    /// The structural kind.
    pub kind: CmmnItemKind,
    /// The `performer: <role>;` clause, wherever it was declared — the
    /// driver validates that only a human task carries one.
    pub performer: Option<(String, Span)>,
    /// Span of the `performer` role name itself (the clause span is
    /// [`CmmnItemDecl::performer`]'s second half), when the clause was
    /// declared.
    pub performer_span: Option<Span>,
    /// The `execute <alias>.<Service>.<operation>;` clause, wherever it
    /// was declared — the driver validates that only a service task
    /// carries one.
    pub execute: Option<CmmnExecute>,
    /// The `invokes <qualified-name>;` clause, wherever it was declared —
    /// the driver validates that only a decision task carries one.
    pub invokes: Option<CmmnInvoke>,
    /// The `inputs { <name>* }` lists of a decision body, merged in source
    /// order (the `.dmn` service-list rule: repeated blocks extend).
    pub inputs: Vec<Name>,
    /// The `outputs { <name>* }` lists of a decision body, merged in
    /// source order.
    pub outputs: Vec<Name>,
    /// `entry` criteria in source order.
    pub entry: Vec<CmmnCriterion>,
    /// `exit` criteria in source order.
    pub exit: Vec<CmmnCriterion>,
    /// The `required;` control flag.
    pub required: bool,
    /// The `auto;` control flag (auto-start).
    pub auto: bool,
    /// The `repeats;` control flag.
    pub repeats: bool,
    /// Span of the whole declaration.
    pub span: Span,
}

/// The structural kind of a [`CmmnItemDecl`], with its body payload.
#[derive(Debug, Clone, PartialEq)]
pub enum CmmnItemKind {
    /// A human task.
    Task,
    /// A service task invoking a `.ddd` service operation.
    Service,
    /// A decision task invoking a `.dmn` decision service.
    Decision,
    /// A milestone: a case goal reached when its entry criteria satisfy.
    Milestone,
    /// A stage grouping nested plan items.
    Stage {
        /// The nested plan items in source order.
        items: Vec<CmmnItemDecl>,
    },
}

/// An `execute <alias>.<Service>.<operation>;` clause of a service body.
#[derive(Debug, Clone, PartialEq)]
pub struct CmmnExecute {
    /// The import alias naming the dependency set.
    pub alias: String,
    /// The service's name.
    pub service: String,
    /// The operation's name.
    pub operation: String,
    /// Span of the `Service.operation` mention (the alias is a file-scope
    /// qualifier, not part of the resolved name).
    pub reference_span: Span,
    /// Span of the whole clause, `;` included.
    pub span: Span,
}

/// An `invokes <qualified-name> [version <pin>];` clause of a decision
/// body. Committed once `invokes` matched: a missing `;` is a reported
/// error. The qualified name lowers **as written** (joined segments); the
/// optional `version` pin (rexlang#154) lowers with it.
#[derive(Debug, Clone, PartialEq)]
pub struct CmmnInvoke {
    /// The invoked decision service's qualified name, **as written** — the
    /// driver resolves it by its last segment (the rule-17 convention) and
    /// the artifact lowers the authored spelling.
    pub service: String,
    /// Span of the invoked qualified name as written.
    pub name_span: Span,
    /// The `version compatible` / `version "<X.Y.Z>"` pin, when written
    /// (rexlang#154) — the same shape the `.ddd` decision references take.
    pub pin: Option<DddVersionPin>,
    /// Span of the whole clause, `;` included.
    pub span: Span,
}

/// An `entry`/`exit` criterion of a plan item body: the on-parts the
/// criterion reacts to and the optional `when (...)` if-part.
#[derive(Debug, Clone, PartialEq)]
pub struct CmmnCriterion {
    /// The on-parts in source order.
    pub on_parts: Vec<CmmnOnPart>,
    /// The `when (...)` condition: the span strictly inside the parens
    /// (the `when`-condition convention; the driver slices the source).
    pub when: Option<Span>,
    /// Span of the whole criterion, `;` included.
    pub span: Span,
}

/// One on-part of a [`CmmnCriterion`]: `name` (a domain event) or
/// `name.tail` (a plan item standard event or case-file transition — the
/// driver validates the tail against its transition tables).
#[derive(Debug, Clone, PartialEq)]
pub struct CmmnOnPart {
    /// The watched element's name.
    pub source: String,
    /// Span of the watched element's name itself (the tail is excluded).
    pub source_span: Span,
    /// The tail word, when the reference is dotted.
    pub event: Option<String>,
    /// Span of the whole on-part.
    pub span: Span,
}
