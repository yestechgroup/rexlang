//! The `.uc` use case DSL AST.

use super::*;

// --- .uc use case DSL --------------------------------------------------------

/// The root node of a parsed `.uc` source: `import` declarations followed by
/// `usecase` declarations. Imports must precede the first usecase (a later
/// `import` is a syntax error); duplicate imports are a driver concern, not
/// a syntax error. An empty import or usecase list is legal.
#[derive(Debug, Clone, PartialEq)]
pub struct UcFile {
    /// The `import` declarations in source order.
    pub imports: Vec<UcImport>,
    /// Declared usecases, in source order.
    pub use_cases: Vec<UcDecl>,
}

/// An `import "<path>" (as <name>)?` declaration of a `.uc` file: a
/// dependency on another source, optionally namespaced under an alias.
#[derive(Debug, Clone, PartialEq)]
pub struct UcImport {
    /// The imported path (the unescaped string literal payload).
    pub path: String,
    /// The alias from the `as` clause, when present.
    pub alias: Option<String>,
    /// Span of the whole declaration, `import` keyword included.
    pub span: Span,
}

/// A `usecase <name> { ... }` declaration: one use case pattern in the
/// SysML v2 spirit — its subject, participating actors, the messages
/// exchanged, the successions ordering them, and the domain services the
/// use case realizes. The interleaved source order of the member kinds is
/// not preserved; each kind is collected into its own source-ordered list.
#[derive(Debug, Clone, PartialEq)]
pub struct UcDecl {
    /// The usecase name.
    pub name: String,
    /// Description from the doc comment directly above the declaration
    /// (contiguous `///` lines, joined).
    pub description: Option<String>,
    /// The `subject` participant; the usecase's initiating classifier.
    /// Recovered as `None` when the source declares none, and a repeated
    /// `subject` line is a reported syntax error (the first one wins).
    pub subject: Option<UcParticipant>,
    /// The `actor` participants in source order.
    pub actors: Vec<UcParticipant>,
    /// Declared messages in source order.
    pub messages: Vec<UcMessage>,
    /// Declared successions in source order.
    pub successions: Vec<UcSuccession>,
    /// `realizes` bindings in source order.
    pub realizes: Vec<UcRealizes>,
    /// Span of the whole declaration, `usecase` keyword included.
    pub span: Span,
}

/// A `subject`/`actor` participant of a [`UcDecl`]: a named classifier with
/// an optional type and an optional body of `occurrence`/`perform` steps.
/// An actor without a type is a bare *policy* actor — a principal the
/// driver knows only by name.
#[derive(Debug, Clone, PartialEq)]
pub struct UcParticipant {
    /// The participant name.
    pub name: String,
    /// The participant's type reference; `None` for a bare policy actor
    /// (and a recovered subject whose `: <type>` clause was missing).
    pub type_name: Option<String>,
    /// Declared occurrences in source order.
    pub occurrences: Vec<UcOccurrence>,
    /// Declared perform steps in source order.
    pub steps: Vec<UcStep>,
    /// Span of the whole participant declaration.
    pub span: Span,
}

/// One `occurrence` line of a [`UcParticipant`] body: a named occurrence of
/// the participant's behavior, either standalone or bound to one end of a
/// message (`occurrence m.source` / `occurrence m.target`).
#[derive(Debug, Clone, PartialEq)]
pub enum UcOccurrence {
    /// `occurrence <name>;`
    Named(String),
    /// `occurrence <message>.source;` (`target: false`) or
    /// `occurrence <message>.target;` (`target: true`).
    MessageEnd {
        /// The message whose end this occurrence is.
        message: String,
        /// `false` for the `.source` tail, `true` for `.target`.
        target: bool,
    },
}

/// One `perform` step of a [`UcParticipant`] body: a named behavior step,
/// optionally bound to a named operation, carrying required capability
/// names and an optional `when (...)` guard. The bare `perform <x>` form
/// names the operation after the step itself.
#[derive(Debug, Clone, PartialEq)]
pub struct UcStep {
    /// The step name.
    pub name: String,
    /// The performed operation: the `: <op>` clause when written, otherwise
    /// the step's own name (the bare `perform <x>` form).
    pub operation: String,
    /// Capability names from the `requires [...]` clause, in source order.
    pub requires: Vec<String>,
    /// Span of the `when (...)` guard clause's raw parens, both parens
    /// inclusive (the driver slices strictly inside), when written.
    pub when: Option<Span>,
    /// Span of the whole step declaration, `;` included.
    pub span: Span,
}

/// A `message <name> [of <type>] [from <endpoint>] [to <endpoint>]`
/// declaration of a [`UcDecl`]: one named communication, optionally typed
/// by its payload and anchored at the participants' occurrence/step ends.
#[derive(Debug, Clone, PartialEq)]
pub struct UcMessage {
    /// The message name.
    pub name: String,
    /// The payload type from the `of` clause, if any.
    pub payload: Option<TypeRef>,
    /// The sending `from` endpoint, if any.
    pub source: Option<UcEndpoint>,
    /// The receiving `to` endpoint, if any.
    pub target: Option<UcEndpoint>,
    /// Span of the whole declaration, `;` included.
    pub span: Span,
}

/// One `participant.anchor` endpoint of a [`UcMessage`]: the participant
/// (or actor) name plus the anchor — an occurrence or step name declared
/// in that participant's body.
#[derive(Debug, Clone, PartialEq)]
pub struct UcEndpoint {
    /// The endpoint's participant name.
    pub participant: String,
    /// The occurrence or step name anchoring the message end.
    pub anchor: String,
    /// Span of the whole endpoint pair.
    pub span: Span,
}

/// A `first <node> then <node>;` statement of a [`UcDecl`]: an ordering
/// constraint between two behavior nodes.
#[derive(Debug, Clone, PartialEq)]
pub struct UcSuccession {
    /// The first node of the succession.
    pub first: UcNodeRef,
    /// The succeeding node of the succession.
    pub then: UcNodeRef,
    /// Span of the whole statement, `;` included.
    pub span: Span,
}

/// One node reference of a [`UcSuccession`]. A `participant.step` pair
/// (or a `message.target` pair naming a message end — which anchor a
/// segment is is the driver's reading) is a qualified reference.
#[derive(Debug, Clone, PartialEq)]
pub enum UcNodeRef {
    /// `start` — the usecase's initial node.
    Start,
    /// `done` — the usecase's final node.
    Done,
    /// A bare name — a step, occurrence, or message of the usecase.
    Bare(String),
    /// `participant.anchor` — a node qualified by its owning participant
    /// (or a message-end pair).
    Qualified(String, String),
}

/// A `realizes <alias>.<Service>.<operation>;` binding of a [`UcDecl`]:
/// the use case is realized by an imported service's operation, named
/// through an import alias.
#[derive(Debug, Clone, PartialEq)]
pub struct UcRealizes {
    /// The import alias naming the dependency set.
    pub alias: String,
    /// The realizing service's name.
    pub service: String,
    /// The realizing operation's name.
    pub operation: String,
    /// Span of the whole binding, `;` included.
    pub span: Span,
}
