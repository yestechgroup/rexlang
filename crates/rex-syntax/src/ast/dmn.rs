//! The `.dmn` decision-model DSL AST.

use super::*;

// --- .dmn decision model DSL -------------------------------------------------

/// The root node of a parsed `.dmn` source: `import` declarations followed
/// by the `decision_model` declaration. Imports must precede the model (a
/// later `import` is a syntax error) and there is exactly one model per
/// file (a second is a reported syntax error; the first wins).
#[derive(Debug, Clone, PartialEq)]
pub struct DmnFile {
    /// The `import` declarations in source order.
    pub imports: Vec<Import>,
    /// The `decision_model` declaration, when one was recovered.
    pub model: Option<DmnDecisionModel>,
}

/// A `decision_model <name> { ... }` declaration: one decision model in the
/// DMN spirit — its inputs, knowledge (sources and models), decisions,
/// requirements, services, and views. The interleaved source order of the
/// member kinds is not preserved; each kind is collected into its own
/// source-ordered list.
#[derive(Debug, Clone, PartialEq)]
pub struct DmnDecisionModel {
    /// The decision model name.
    pub name: Name,
    /// The `namespace <qname>;` line, when written. A repeated `namespace`
    /// line is a reported syntax error (the first one wins).
    pub namespace: Option<QualifiedName>,
    /// The `id`/`version`/`effective`/`jurisdiction` metadata lines in
    /// source order (uniqueness is a driver concern, not syntax).
    pub metadata: Vec<DmnMetadata>,
    /// Declared inputs in source order.
    pub inputs: Vec<DmnInput>,
    /// Declared knowledge sources in source order.
    pub knowledge_sources: Vec<DmnKnowledgeSource>,
    /// Declared knowledge models (BKMs) in source order.
    pub models: Vec<DmnBkm>,
    /// Declared decisions in source order.
    pub decisions: Vec<DmnDecision>,
    /// Information-requirement lines in source order.
    pub information_requirements: Vec<DmnRequirement>,
    /// Knowledge-requirement lines in source order.
    pub knowledge_requirements: Vec<DmnRequirement>,
    /// Authority-requirement lines in source order.
    pub authority_requirements: Vec<DmnRequirement>,
    /// Declared services in source order.
    pub services: Vec<DmnService>,
    /// Declared views in source order.
    pub views: Vec<DmnView>,
    /// Span of the whole declaration, `decision_model` keyword included.
    pub span: Span,
}

/// One `id`/`version`/`effective`/`jurisdiction` metadata line of a
/// [`DmnDecisionModel`]: a string-valued governance attribute.
#[derive(Debug, Clone, PartialEq)]
pub struct DmnMetadata {
    /// Which metadata line this is.
    pub kind: DmnMetadataKind,
    /// The line's string value.
    pub value: String,
    /// Span of the whole line.
    pub span: Span,
}

/// Which metadata line a [`DmnMetadata`] is. The `namespace` line is not a
/// [`DmnMetadata`] — it carries a qualified name and lives in
/// [`DmnDecisionModel::namespace`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DmnMetadataKind {
    /// The `id "<value>";` line.
    Id,
    /// The `version "<value>";` line.
    Version,
    /// The `effective "<value>";` line.
    Effective,
    /// The `jurisdiction "<value>";` line.
    Jurisdiction,
}

/// A `name ":" type_ref ";"` entry of a `.dmn` `inputs` block — and of a
/// model's (BKM's) `parameters` block and a decision's `outputs` block,
/// which share the exact production.
#[derive(Debug, Clone, PartialEq)]
pub struct DmnInput {
    /// The entry name.
    pub name: Name,
    /// The entry's type reference.
    pub type_ref: TypeRef,
    /// Span of the whole entry.
    pub span: Span,
}

/// A `source <name> { ... }` declaration of a knowledge block: one
/// knowledge source with its optional display name and reference. The
/// `description` comes from the doc comment directly above the declaration.
#[derive(Debug, Clone, PartialEq)]
pub struct DmnKnowledgeSource {
    /// The knowledge source name.
    pub name: Name,
    /// Description from the doc comment directly above the declaration
    /// (contiguous `///` lines, joined).
    pub description: Option<String>,
    /// The display name from the `name "<value>";` property, when written.
    pub display_name: Option<String>,
    /// The `reference "<value>";` property, when written.
    pub reference: Option<String>,
    /// Span of the whole declaration.
    pub span: Span,
}

/// A `model <name> { ... }` declaration of a knowledge block: one business
/// knowledge model — its parameters, output type, and (required) logic.
/// The `description` comes from the doc comment directly above the
/// declaration.
#[derive(Debug, Clone, PartialEq)]
pub struct DmnBkm {
    /// The model name.
    pub name: Name,
    /// Description from the doc comment directly above the declaration
    /// (contiguous `///` lines, joined).
    pub description: Option<String>,
    /// Declared parameters in source order.
    pub parameters: Vec<DmnInput>,
    /// The `output <type>;` line. The grammar requires it; a recovered
    /// model whose output line was missing carries an empty [`TypeRef`].
    pub output: TypeRef,
    /// The `logic <kind>` clause. The grammar requires it; a recovered
    /// model whose logic clause was missing carries empty expression
    /// logic.
    pub logic: DmnLogic,
    /// Span of the whole declaration.
    pub span: Span,
}

/// A `decision <name> { ... }` declaration of a decisions block: one
/// decision — its outputs and optional logic (a decision may declare only
/// its contract). The `description` comes from the doc comment directly
/// above the declaration.
#[derive(Debug, Clone, PartialEq)]
pub struct DmnDecision {
    /// The decision name.
    pub name: Name,
    /// Description from the doc comment directly above the declaration
    /// (contiguous `///` lines, joined).
    pub description: Option<String>,
    /// The declared outputs in source order: named `outputs`-block entries,
    /// and the anonymous `output <type>;` line as an output with an empty
    /// name.
    pub outputs: Vec<DmnOutput>,
    /// The `logic <kind>` clause, when written.
    pub logic: Option<DmnLogic>,
    /// Span of the whole declaration.
    pub span: Span,
}

/// One output of a [`DmnDecision`]: a named `outputs`-block entry, or the
/// anonymous `output <type>;` line (an empty [`Name`]).
#[derive(Debug, Clone, PartialEq)]
pub struct DmnOutput {
    /// The output name; empty for the anonymous `output <type>;` form.
    pub name: Name,
    /// The output's type reference.
    pub type_ref: TypeRef,
    /// Span of the whole output line.
    pub span: Span,
}

/// A `logic <kind>` clause of a [`DmnBkm`] or [`DmnDecision`]: the table
/// source, expression text, or model invocation backing the decision.
#[derive(Debug, Clone, PartialEq)]
pub enum DmnLogic {
    /// `logic table "<source>"` — a decision table reference.
    Table {
        /// The table source string.
        source: String,
        /// Span of the whole clause.
        span: Span,
    },
    /// `logic expression "<text>"` — an expression text.
    Expression {
        /// The expression text.
        text: String,
        /// Span of the whole clause.
        span: Span,
    },
    /// `logic invoke <model> { ... }` — a business knowledge model
    /// invocation with its argument bindings.
    Invoke {
        /// The invoked model's name.
        model: Name,
        /// The argument bindings in source order.
        bindings: Vec<DmnBinding>,
        /// Span of the whole clause.
        span: Span,
    },
}

/// One `name ":" "<expression>" ";"` binding of a [`DmnLogic::Invoke`]
/// body: an argument of the invoked model bound to an expression string.
#[derive(Debug, Clone, PartialEq)]
pub struct DmnBinding {
    /// The bound argument's name.
    pub name: Name,
    /// The binding's expression string.
    pub expression: String,
    /// Span of the whole binding.
    pub span: Span,
}

/// One `<name> <connective> <name>;` line of a `.dmn` requirement group:
/// the requiring element and the required one. The connective
/// (`requires`/`invokes`/`governed_by`) selects the group the line folds
/// into and is not stored.
#[derive(Debug, Clone, PartialEq)]
pub struct DmnRequirement {
    /// The requiring element's name.
    pub requiring: Name,
    /// The required element's name (empty when the line recovered without
    /// its target).
    pub required: Name,
    /// Span of the whole line.
    pub span: Span,
}

/// A `service <name> { ... }` declaration of a services block: one
/// decision service with its inputs, outputs, and encapsulated knowledge.
/// The `description` may come from the doc comment directly above the
/// declaration or from the body's `description "<text>";` line.
#[derive(Debug, Clone, PartialEq)]
pub struct DmnService {
    /// The service name.
    pub name: Name,
    /// The description: the `description "<text>";` body line when written,
    /// else the doc comment directly above the declaration (contiguous
    /// `///` lines, joined).
    pub description: Option<String>,
    /// The input names of the `inputs { ... }` list, in source order.
    pub inputs: Vec<Name>,
    /// The output names of the `outputs { ... }` list, in source order.
    pub outputs: Vec<Name>,
    /// The encapsulated element names of the `encapsulates { ... }` list,
    /// in source order.
    pub encapsulates: Vec<Name>,
    /// Span of the whole declaration.
    pub span: Span,
}

/// A `view <name> { ... }` declaration of a views block: one decision
/// view — the decisions it shows and its direction. The `description` may
/// come from the doc comment directly above the declaration or from the
/// body's `description "<text>";` line.
#[derive(Debug, Clone, PartialEq)]
pub struct DmnView {
    /// The view name.
    pub name: Name,
    /// The description: the `description "<text>";` body line when written,
    /// else the doc comment directly above the declaration (contiguous
    /// `///` lines, joined).
    pub description: Option<String>,
    /// The shown element names of the `show { ... }` list, in source order.
    pub show: Vec<Name>,
    /// The `direction <name>;` line, when written.
    pub direction: Option<Name>,
    /// Span of the whole declaration.
    pub span: Span,
}
