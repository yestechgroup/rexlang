//! Abstract syntax tree for `.mox` sources.
//!
//! Every node carries a [`Span`] (a byte range into the original source text).
//! All types are owned (no borrows of the source), which keeps the AST easy to
//! cache in long-lived services such as a language server.
//!
//! The `.ddd` design DSL reuses the shared node types ([`Name`],
//! [`QualifiedName`], [`TypeRef`], [`Multiplicity`], [`ImportDecl`]) and adds
//! the `Ddd`-prefixed declarations at the bottom of this file, mirroring the
//! `rex_ir::ddd` wire artifact field for field.

use chumsky::span::SimpleSpan;

/// Byte-offset span into the source text.
pub type Span = SimpleSpan<usize>;

/// A single identifier, e.g. `Book` or an escaped keyword such as `^class`.
#[derive(Debug, Clone, PartialEq)]
pub struct Name {
    /// The identifier text. For escaped keywords the leading `^` is stripped.
    pub text: String,
    /// Span of the identifier as written, including the leading `^` for escaped keywords.
    pub span: Span,
    /// Whether the identifier was escaped with a leading `^`.
    pub escaped: bool,
}

/// A dot-separated qualified name, e.g. `nz.example.library.Book`.
#[derive(Debug, Clone, PartialEq)]
pub struct QualifiedName {
    /// At least one segment.
    pub segments: Vec<Name>,
    /// Span covering all segments and separators.
    pub span: Span,
}

impl QualifiedName {
    /// The qualified name joined with `.`, e.g. `nz.example.library.Book`.
    pub fn full_name(&self) -> String {
        self.segments
            .iter()
            .map(|segment| segment.text.as_str())
            .collect::<Vec<_>>()
            .join(".")
    }
}

/// A type reference: a qualified name used where a type is expected.
#[derive(Debug, Clone, PartialEq)]
pub struct TypeRef {
    /// The referenced type name.
    pub name: QualifiedName,
    /// Span of the reference.
    pub span: Span,
}

/// Upper bound of a multiplicity range.
#[derive(Debug, Clone, PartialEq)]
pub enum MultBound {
    /// A numeric upper bound, e.g. the `5` in `[3..5]`.
    Int(i64),
    /// An unbounded upper bound, e.g. the `*` in `[1..*]`.
    Star,
}

/// Shape of a multiplicity annotation.
#[derive(Debug, Clone, PartialEq)]
pub enum MultiplicityKind {
    /// `[]` - unbounded collection (0..*).
    Unbounded,
    /// `[n]` - exactly `n` elements.
    Exact(i64),
    /// `[a..b]` or `[a..*]`.
    Range(i64, MultBound),
}

/// A multiplicity annotation such as `[]`, `[2]`, `[0..1]`, `[1..*]` or `[3..5]`.
#[derive(Debug, Clone, PartialEq)]
pub struct Multiplicity {
    /// The parsed shape of the annotation.
    pub kind: MultiplicityKind,
    /// Span including the brackets.
    pub span: Span,
}

/// A target binding entry, e.g. `rust "chrono::NaiveDate"`.
#[derive(Debug, Clone, PartialEq)]
pub struct BindingEntry {
    /// Target key (a target name such as `rust`, `csharp` or `java`).
    pub key: Name,
    /// Bound value (the unescaped string literal text).
    pub value: String,
    /// Span covering the whole entry.
    pub span: Span,
}

/// A target-tagged body block: `<target> { ... }`, used as an operation body
/// and inside datatype `create`/`convert` blocks.
///
/// The block's contents are deliberately not parsed; the body text is the
/// source text strictly inside [`TargetBody::span`] (the braces), captured
/// verbatim (spacing, newlines, comments, and non-grammar characters intact)
/// by whoever holds the source text.
#[derive(Debug, Clone, PartialEq)]
pub struct TargetBody {
    /// The target name, e.g. `rust`.
    pub target: Name,
    /// Span of the balanced `{ ... }` block, braces inclusive.
    pub span: Span,
}

/// The target of a `wraps` clause on a datatype.
#[derive(Debug, Clone, PartialEq)]
pub enum Wraps {
    /// The `opaque` keyword.
    Opaque(Span),
    /// A qualified foreign type name.
    Named(QualifiedName),
}

/// A default value assigned to an attribute with `=`.
#[derive(Debug, Clone, PartialEq)]
pub enum DefaultValue {
    /// A string literal (unescaped).
    Str { value: String, span: Span },
    /// An integer literal.
    Int { value: i64, span: Span },
    /// `true` or `false`.
    Bool { value: bool, span: Span },
    /// A (possibly qualified-by-context) identifier.
    Name(Name),
}

/// A single operation parameter: `type name`.
#[derive(Debug, Clone, PartialEq)]
pub struct Param {
    /// The parameter type.
    pub type_ref: TypeRef,
    /// The parameter name.
    pub name: Name,
    /// Span covering the whole parameter.
    pub span: Span,
}

/// A single constraint entry of an attribute's constraint block, e.g.
/// `pattern "[A-Z]{3}-[0-9]{4}"`.
#[derive(Debug, Clone, PartialEq)]
pub struct Constraint {
    /// The constraint keyword, e.g. `pattern`. Only the constraint keywords
    /// are accepted (`pattern`, `minLength`, `maxLength`, `minimum`,
    /// `maximum`, and the value-less `unique`); other identifiers do not
    /// enter the block.
    pub name: Name,
    /// The constraint value.
    pub value: ConstraintValue,
    /// Span covering the whole entry.
    pub span: Span,
}

/// The value of a [`Constraint`] entry: a string (`pattern`), an integer
/// (length and numeric bounds), or the value-less `unique` marker.
#[derive(Debug, Clone, PartialEq)]
pub enum ConstraintValue {
    /// A string literal (unescaped).
    Str {
        /// The unescaped string value.
        value: String,
        /// Span of the literal.
        span: Span,
    },
    /// An integer literal.
    Int {
        /// The integer value.
        value: i64,
        /// Span of the literal.
        span: Span,
    },
    /// No value: the `unique` constraint is a bare flag.
    Flag {
        /// Span of the whole entry.
        span: Span,
    },
}

/// The root node of a parsed `.mox` source.
#[derive(Debug, Clone, PartialEq)]
pub struct Model {
    /// The `package` declaration, if present.
    pub package: Option<PackageDecl>,
    /// All other top-level declarations in source order.
    pub declarations: Vec<Decl>,
}

/// A `package` declaration: the model's dotted namespace name.
#[derive(Debug, Clone, PartialEq)]
pub struct PackageDecl {
    /// The declared qualified name.
    pub name: QualifiedName,
    /// Description from the doc comment directly above the declaration.
    pub doc: Option<String>,
    /// Span of the whole declaration, `package` keyword included.
    pub span: Span,
}

/// A top-level declaration.
#[derive(Debug, Clone, PartialEq)]
pub enum Decl {
    /// A `class` declaration.
    Class(ClassDecl),
    /// An `interface` declaration.
    Interface(InterfaceDecl),
    /// An `enum` declaration.
    Enum(EnumDecl),
    /// A `type ... wraps ...` declaration.
    Datatype(DatatypeDecl),
    /// An `annotation` declaration.
    Annotation(AnnotationDecl),
    /// A `vocabulary ... from ...` declaration.
    Vocabulary(VocabularyDecl),
    /// An `actors { ... }` declaration.
    Actors(ActorsDecl),
    /// An `import schema "<path>" (as <name>)?` or `import sigil "<path>"`
    /// declaration: a JSON Schema type or a Rune DSL (`.rosetta`) namespace
    /// set imported into the package's namespace.
    ImportSchema(ImportSchemaDecl),
}

impl Decl {
    /// Span of the whole declaration.
    pub fn span(&self) -> Span {
        match self {
            Decl::Class(decl) => decl.span,
            Decl::Interface(decl) => decl.span,
            Decl::Enum(decl) => decl.span,
            Decl::Datatype(decl) => decl.span,
            Decl::Annotation(decl) => decl.span,
            Decl::Vocabulary(decl) => decl.span,
            Decl::Actors(decl) => decl.span,
            Decl::ImportSchema(decl) => decl.span,
        }
    }

    /// The declared name, if the declaration kind has one. Annotations only
    /// have a name when the `as` clause is present; an import schema only
    /// when the `as` clause is present (otherwise the file stem names it,
    /// which the driver derives; a sigil import never has one).
    pub fn name(&self) -> Option<&Name> {
        match self {
            Decl::Class(decl) => Some(&decl.name),
            Decl::Interface(decl) => Some(&decl.name),
            Decl::Enum(decl) => Some(&decl.name),
            Decl::Datatype(decl) => Some(&decl.name),
            Decl::Annotation(decl) => decl.name.as_ref(),
            Decl::Vocabulary(decl) => Some(&decl.name),
            Decl::Actors(decl) => Some(&decl.name),
            Decl::ImportSchema(decl) => decl.alias.as_ref(),
        }
    }
}

/// A `class` declaration.
#[derive(Debug, Clone, PartialEq)]
pub struct ClassDecl {
    /// The class name.
    pub name: Name,
    /// Description from the doc comment directly above the declaration.
    pub doc: Option<String>,
    /// Direct superclasses from the `extends` clause, in source order.
    pub extends: Vec<TypeRef>,
    /// Features (attributes, containments, references, ops, ...) in source order.
    pub features: Vec<FeatureDecl>,
    /// Span of the whole declaration.
    pub span: Span,
}

/// An `interface` declaration (target bindings only in Tier 0).
#[derive(Debug, Clone, PartialEq)]
pub struct InterfaceDecl {
    /// The interface name.
    pub name: Name,
    /// Description from the doc comment directly above the declaration.
    pub doc: Option<String>,
    /// Target binding entries.
    pub bindings: Vec<BindingEntry>,
    /// Span of the whole declaration.
    pub span: Span,
}

/// An `enum` declaration.
#[derive(Debug, Clone, PartialEq)]
pub struct EnumDecl {
    /// The enum name.
    pub name: Name,
    /// Description from the doc comment directly above the declaration.
    pub doc: Option<String>,
    /// The enum literals (at least one in valid sources).
    pub literals: Vec<EnumLiteral>,
    /// Span of the whole declaration.
    pub span: Span,
}

/// A single enum literal: `Name ("as" string)? ("=" int)?`.
#[derive(Debug, Clone, PartialEq)]
pub struct EnumLiteral {
    /// The literal name.
    pub name: Name,
    /// Description from the doc comment directly above the literal.
    pub doc: Option<String>,
    /// Display label from the `as` clause.
    pub label: Option<String>,
    /// Numeric value from the `=` clause.
    pub value: Option<i64>,
    /// Span of the whole literal.
    pub span: Span,
}

/// A `type ... wraps ...` datatype declaration.
#[derive(Debug, Clone, PartialEq)]
pub struct DatatypeDecl {
    /// The datatype name.
    pub name: Name,
    /// Description from the doc comment directly above the declaration.
    pub doc: Option<String>,
    /// The `wraps` target, if any.
    pub wraps: Option<Wraps>,
    /// Target binding entries.
    pub bindings: Vec<BindingEntry>,
    /// The declared `format` hint from the reserved `format "…"` block key,
    /// e.g. `format "email"`. Never a target binding.
    pub format: Option<String>,
    /// `create { <target-body>+ }` blocks, in source order. More than one is
    /// a driver error ("at most one of each").
    pub create: Vec<TargetBody>,
    /// `convert { <target-body>+ }` blocks, in source order.
    pub convert: Vec<TargetBody>,
    /// Span of the whole declaration.
    pub span: Span,
}

/// An `annotation` declaration.
#[derive(Debug, Clone, PartialEq)]
pub struct AnnotationDecl {
    /// The quoted annotation value (unescaped).
    pub value: String,
    /// Optional target name from the `as` clause.
    pub name: Option<Name>,
    /// Span of the whole declaration.
    pub span: Span,
}

/// A `vocabulary ... from ...` declaration: an external, versioned set of
/// enumerated keys usable as a type.
#[derive(Debug, Clone, PartialEq)]
pub struct VocabularyDecl {
    /// The vocabulary name (usable as a type name in the package).
    pub name: Name,
    /// Description from the doc comment directly above the declaration.
    pub doc: Option<String>,
    /// The external source identifier, e.g. `"iso:4217"` (unescaped).
    pub source: String,
    /// Pinned snapshot version from the `version` clause, if present.
    pub version: Option<String>,
    /// The `key` facet naming the unique entry key, if present. Absence is a
    /// *semantic* error reported by the driver, not a syntax error.
    pub key: Option<Name>,
    /// Declared facets in source order.
    pub facets: Vec<VocabularyFacetDecl>,
    /// Span of the whole declaration.
    pub span: Span,
}

/// A single `facet <type_ref> <name>` member of a [`VocabularyDecl`].
#[derive(Debug, Clone, PartialEq)]
pub struct VocabularyFacetDecl {
    /// The facet's declared type (must resolve to a primitive; checked by the
    /// driver).
    pub type_ref: TypeRef,
    /// The facet name.
    pub name: Name,
    /// Span of the whole facet declaration.
    pub span: Span,
}

/// An `actors { ... }` declaration: an actor/authorization model in the
/// Cedar spirit. Actors, capabilities, purposes, grants, delegations and
/// `never_both` exclusivity constraints are collected in source order into
/// separate lists.
#[derive(Debug, Clone, PartialEq)]
pub struct ActorsDecl {
    /// The actors-block name.
    pub name: Name,
    /// Declared actors, in source order.
    pub actors: Vec<ActorDecl>,
    /// Declared capabilities, in source order.
    pub capabilities: Vec<CapabilityDecl>,
    /// Declared purposes, in source order.
    pub purposes: Vec<PurposeDecl>,
    /// Declared grants, in source order.
    pub grants: Vec<GrantDecl>,
    /// Declared delegations, in source order.
    pub delegations: Vec<DelegationDecl>,
    /// Declared `never_both` exclusivity constraints, in source order.
    pub never_both: Vec<NeverBothDecl>,
    /// Span of the whole declaration.
    pub span: Span,
}

/// Whether an [`ActorDecl`] was introduced by `actor` (a human principal) or
/// `agent` (an autonomous LLM agent).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ActorKind {
    /// `actor <name>` — a human actor.
    #[default]
    Human,
    /// `agent <name>` — an LLM agent.
    Agent,
}

/// A single `actor <name> ("extends" <name>)?` or `agent <name>
/// ("extends" <name>)?` member of an [`ActorsDecl`].
#[derive(Debug, Clone, PartialEq)]
pub struct ActorDecl {
    /// The actor name.
    pub name: Name,
    /// Whether this is a human `actor` or an `agent`.
    pub kind: ActorKind,
    /// The direct parent actor from the `extends` clause, if any.
    pub extends: Option<Name>,
    /// Span of the whole actor declaration.
    pub span: Span,
}

/// A single `capability <name> on <type_ref>` member of an [`ActorsDecl`].
#[derive(Debug, Clone, PartialEq)]
pub struct CapabilityDecl {
    /// The capability name.
    pub name: Name,
    /// The type the capability is granted on.
    pub class: TypeRef,
    /// Span of the whole capability declaration.
    pub span: Span,
}

/// A `purpose <name>` member of an [`ActorsDecl`]: a named purpose of the
/// authorization model, usable as a grouping/authorization dimension.
#[derive(Debug, Clone, PartialEq)]
pub struct PurposeDecl {
    /// The purpose name.
    pub name: Name,
    /// Span of the whole purpose declaration.
    pub span: Span,
}

/// A `grant <actor> { ... }` block: the entries granted to one actor.
#[derive(Debug, Clone, PartialEq)]
pub struct GrantDecl {
    /// The actor the entries are granted to.
    pub actor: Name,
    /// Grant entries in source order. Empty bodies are allowed.
    pub entries: Vec<GrantEntryDecl>,
    /// Span of the whole grant declaration.
    pub span: Span,
}

/// One entry of a [`GrantDecl`]: a `permit`/`forbid` effect or a raw `cedar`
/// policy body.
#[derive(Debug, Clone, PartialEq)]
pub enum GrantEntryDecl {
    /// `("permit" | "forbid") <capability> ("when" (...))? obligation*`
    Effect(GrantEffectDecl),
    /// `cedar { ... }` — the raw body is captured verbatim (braces
    /// inclusive), exactly like an operation's target body.
    Cedar(TargetBody),
}

/// The effect of a [`GrantEffectDecl`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Effect {
    /// `permit` — the capability is allowed.
    Permit,
    /// `forbid` — the capability is denied.
    Forbid,
}

/// A `permit`/`forbid` entry of a [`GrantDecl`].
#[derive(Debug, Clone, PartialEq)]
pub struct GrantEffectDecl {
    /// Whether the capability is permitted or forbidden.
    pub effect: Effect,
    /// The capability the effect applies to.
    pub capability: Name,
    /// Span of the raw `(...)` condition, parens inclusive. The condition
    /// text is the source strictly inside this span (same convention as
    /// [`TargetBody`]); its contents are not parsed.
    pub when: Option<Span>,
    /// Obligation names attached to the effect, in source order.
    pub obligations: Vec<Name>,
    /// Span of the whole effect entry.
    pub span: Span,
}

/// A `delegation <name> { from <actor> to <actor> ... }` member of an
/// [`ActorsDecl`]: effect entries delegated from one actor to another. The
/// entries reuse [`GrantEffectDecl`], but `cedar { ... }` entries are a
/// syntax error inside a delegation and never appear here.
#[derive(Debug, Clone, PartialEq)]
pub struct DelegationDecl {
    /// The delegation name.
    pub name: Name,
    /// The delegating actor (the required `from` line).
    pub from: Name,
    /// The receiving actor (the required `to` line).
    pub to: Name,
    /// The optional `purpose` line (the required order is `from` → `to` →
    /// `purpose` → entries).
    pub purpose: Option<Name>,
    /// Delegated effect entries in source order. Zero entries is legal.
    pub entries: Vec<GrantEffectDecl>,
    /// Span of the whole delegation declaration.
    pub span: Span,
}

/// A `never_both { <name>, <name> (, ...)? }` exclusivity constraint of an
/// [`ActorsDecl`]. Fewer than two names is a syntax error.
#[derive(Debug, Clone, PartialEq)]
pub struct NeverBothDecl {
    /// The mutually exclusive capability names (at least two).
    pub capabilities: Vec<Name>,
    /// Span of the whole constraint.
    pub span: Span,
}

/// An `import "path"` declaration of an `.actor` file: a dependency on
/// another source, resolved by the driver.
#[derive(Debug, Clone, PartialEq)]
pub struct ImportDecl {
    /// The imported path (the unescaped string literal payload).
    pub path: String,
    /// Span of the whole declaration, `import` keyword included.
    pub span: Span,
}

/// Which import kind an [`ImportSchemaDecl`] declares.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImportKind {
    /// `import schema "<path>" (as <name>)?`: a JSON Schema type imported
    /// into the package's namespace.
    Schema,
    /// `import sigil "<path>"`: a Rune DSL (`.rosetta`) namespace set
    /// imported into the package's namespace. A whole namespace set is
    /// imported, not one type, so the declaration takes no `as` alias.
    Sigil,
}

/// An `import schema "<path>" (as <name>)?` or `import sigil "<path>"`
/// declaration of a `.mox` file, discriminated by [`ImportSchemaDecl::kind`]:
/// a JSON Schema type or a Rune DSL (`.rosetta`) namespace set imported into
/// the package's namespace. The `schema`/`sigil` words are contextual
/// keywords (only special directly after `import`), so existing models may
/// keep using either as an identifier. The driver derives an imported
/// schema's name from the `as` clause, or from the path's file stem when the
/// clause is absent; a sigil import never takes an alias (an `as` clause is
/// a syntax error).
#[derive(Debug, Clone, PartialEq)]
pub struct ImportSchemaDecl {
    /// Which import kind this declaration is.
    pub kind: ImportKind,
    /// The imported path (the unescaped string literal payload), resolved
    /// relative to the declaring `.mox` file's directory.
    pub path: String,
    /// The name the import joins the package namespace as, from the `as`
    /// clause. `None` when the import relies on the path's file stem
    /// (always `None` for a sigil import).
    pub alias: Option<Name>,
    /// Span of the whole declaration, `import` keyword included.
    pub span: Span,
}

/// The root node of a parsed `.actor` source: `import` declarations
/// followed by `actors` blocks. The block grammar is identical to the
/// inline [`ActorsDecl`] of `.mox` sources. An `import` after an actors
/// block is a syntax error; an empty import or block list is legal, and so
/// are duplicate imports — those are semantic questions for the driver.
#[derive(Debug, Clone, PartialEq)]
pub struct ActorFile {
    /// The `import` declarations in source order.
    pub imports: Vec<ImportDecl>,
    /// The `actors` blocks in source order.
    pub blocks: Vec<ActorsDecl>,
}

/// A contextual feature modifier: the `id` or `readonly` keyword written
/// before a feature's type. The lexer emits both as ordinary identifiers;
/// they act as modifiers only in modifier position (before the feature).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModifierKind {
    /// `id` — the feature is part of the class identity.
    Id,
    /// `readonly` — the feature cannot be reassigned after initialization.
    ReadOnly,
}

impl ModifierKind {
    /// The keyword text as written in the source.
    pub fn keyword(self) -> &'static str {
        match self {
            ModifierKind::Id => "id",
            ModifierKind::ReadOnly => "readonly",
        }
    }
}

/// The `id`/`readonly` modifiers of a feature, each carrying the span of the
/// modifier keyword as written. Escaped forms (`^id`, `^readonly`) are never
/// modifiers. Repeating a modifier is idempotent (the first occurrence wins).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Modifiers {
    /// Span of the `id` modifier keyword, if present.
    pub id: Option<Span>,
    /// Span of the `readonly` modifier keyword, if present.
    pub read_only: Option<Span>,
}

impl Modifiers {
    /// Records one modifier; repeats are idempotent (the first span wins).
    pub fn push(&mut self, kind: ModifierKind, span: Span) {
        match kind {
            ModifierKind::Id => self.id = self.id.or(Some(span)),
            ModifierKind::ReadOnly => self.read_only = self.read_only.or(Some(span)),
        }
    }

    /// `true` when neither modifier is present.
    pub fn is_empty(&self) -> bool {
        self.id.is_none() && self.read_only.is_none()
    }

    /// `true` when the feature carries the `id` modifier.
    pub fn is_id(&self) -> bool {
        self.id.is_some()
    }

    /// `true` when the feature carries the `readonly` modifier.
    pub fn is_read_only(&self) -> bool {
        self.read_only.is_some()
    }
}

/// A feature declared inside a class body.
#[derive(Debug, Clone, PartialEq)]
pub enum FeatureDecl {
    /// `(modifier)* type_ref multiplicity? name ("=" default)? constraint_block?`
    Attribute {
        /// Declared `id`/`readonly` modifiers.
        modifiers: Modifiers,
        /// Description from the doc comment directly above the feature.
        doc: Option<String>,
        /// Declared type.
        type_ref: TypeRef,
        /// Multiplicity annotation, if any.
        multiplicity: Option<Multiplicity>,
        /// Feature name.
        name: Name,
        /// Default value, if any.
        default: Option<DefaultValue>,
        /// Constraint block entries, in source order. Only attributes carry
        /// a constraint block; ops and derived features have bodies instead.
        constraints: Vec<Constraint>,
        /// Span of the whole feature, modifiers included.
        span: Span,
    },
    /// `(modifier)* contains type_ref multiplicity? name ("opposite" name)?`
    Containment {
        /// Declared `id`/`readonly` modifiers.
        modifiers: Modifiers,
        /// Description from the doc comment directly above the feature.
        doc: Option<String>,
        /// Element type.
        type_ref: TypeRef,
        /// Multiplicity annotation, if any.
        multiplicity: Option<Multiplicity>,
        /// Feature name.
        name: Name,
        /// Opposite end name, if any.
        opposite: Option<Name>,
        /// Span of the whole feature, modifiers included.
        span: Span,
    },
    /// `(modifier)* refers type_ref multiplicity? name ("opposite" name)?`
    Reference {
        /// Declared `id`/`readonly` modifiers.
        modifiers: Modifiers,
        /// Description from the doc comment directly above the feature.
        doc: Option<String>,
        /// Referenced type.
        type_ref: TypeRef,
        /// Multiplicity annotation, if any.
        multiplicity: Option<Multiplicity>,
        /// Feature name.
        name: Name,
        /// Opposite end name, if any.
        opposite: Option<Name>,
        /// Span of the whole feature, modifiers included.
        span: Span,
    },
    /// `(modifier)* container type_ref name ("opposite" name)?`
    Container {
        /// Declared `id`/`readonly` modifiers.
        modifiers: Modifiers,
        /// Description from the doc comment directly above the feature.
        doc: Option<String>,
        /// Container type.
        type_ref: TypeRef,
        /// Feature name.
        name: Name,
        /// Opposite end name, if any.
        opposite: Option<Name>,
        /// Span of the whole feature, modifiers included.
        span: Span,
    },
    /// `(modifier)* op type_ref name "(" params? ")" raw_body?`
    Op {
        /// Declared `id`/`readonly` modifiers.
        modifiers: Modifiers,
        /// Description from the doc comment directly above the feature.
        doc: Option<String>,
        /// Declared return type.
        return_type: TypeRef,
        /// Operation name.
        name: Name,
        /// Parameters in source order.
        params: Vec<Param>,
        /// Span of the raw `{ ... }` body, if present (braces inclusive).
        /// Contents are not parsed.
        body: Option<Span>,
        /// Target-tagged bodies `<target> { ... }` in source order. Empty when
        /// the body is absent *or* is a bare (untagged) `{ ... }` block, which
        /// the driver rejects: bodies must be per-target in Tier 1.
        bodies: Vec<TargetBody>,
        /// Span of the whole feature, modifiers included.
        span: Span,
    },
    /// `(modifier)* derived type_ref multiplicity? name op_body?`
    Derived {
        /// Declared `id`/`readonly` modifiers.
        modifiers: Modifiers,
        /// Description from the doc comment directly above the feature.
        doc: Option<String>,
        /// Declared type.
        type_ref: TypeRef,
        /// Multiplicity annotation, if any.
        multiplicity: Option<Multiplicity>,
        /// Feature name.
        name: Name,
        /// Span of the whole body, if present (braces inclusive): either a
        /// bare `{ ... }` block (Tier 2: rejected by the driver) or the
        /// outer braces of target-tagged blocks. Contents are not parsed.
        body: Option<Span>,
        /// Target-tagged bodies `<target> { ... }` in source order. Empty
        /// when the body is absent *or* is a bare (untagged) `{ ... }`
        /// block. Tier 2: derived features carry their neutral expression
        /// in a single `expr { ... }` block.
        bodies: Vec<TargetBody>,
        /// Span of the whole feature, modifiers included.
        span: Span,
    },
}

impl FeatureDecl {
    /// Span of the whole feature, modifiers included.
    pub fn span(&self) -> Span {
        match self {
            FeatureDecl::Attribute { span, .. }
            | FeatureDecl::Containment { span, .. }
            | FeatureDecl::Reference { span, .. }
            | FeatureDecl::Container { span, .. }
            | FeatureDecl::Op { span, .. }
            | FeatureDecl::Derived { span, .. } => *span,
        }
    }

    /// The feature's declared `id`/`readonly` modifiers.
    pub fn modifiers(&self) -> &Modifiers {
        match self {
            FeatureDecl::Attribute { modifiers, .. }
            | FeatureDecl::Containment { modifiers, .. }
            | FeatureDecl::Reference { modifiers, .. }
            | FeatureDecl::Container { modifiers, .. }
            | FeatureDecl::Op { modifiers, .. }
            | FeatureDecl::Derived { modifiers, .. } => modifiers,
        }
    }

    /// Overwrites the feature's whole-declaration span. The parser uses this
    /// to widen the span across leading modifiers.
    pub fn set_span(&mut self, span: Span) {
        match self {
            FeatureDecl::Attribute { span: slot, .. }
            | FeatureDecl::Containment { span: slot, .. }
            | FeatureDecl::Reference { span: slot, .. }
            | FeatureDecl::Container { span: slot, .. }
            | FeatureDecl::Op { span: slot, .. }
            | FeatureDecl::Derived { span: slot, .. } => *slot = span,
        }
    }

    /// Overwrites the feature's declared modifiers. The parser uses this to
    /// attach the modifiers parsed before the feature's type.
    pub fn set_modifiers(&mut self, modifiers: Modifiers) {
        match self {
            FeatureDecl::Attribute {
                modifiers: slot, ..
            }
            | FeatureDecl::Containment {
                modifiers: slot, ..
            }
            | FeatureDecl::Reference {
                modifiers: slot, ..
            }
            | FeatureDecl::Container {
                modifiers: slot, ..
            }
            | FeatureDecl::Op {
                modifiers: slot, ..
            }
            | FeatureDecl::Derived {
                modifiers: slot, ..
            } => *slot = modifiers,
        }
    }

    /// The feature's description from its doc comment, if any.
    pub fn doc(&self) -> Option<&str> {
        match self {
            FeatureDecl::Attribute { doc, .. }
            | FeatureDecl::Containment { doc, .. }
            | FeatureDecl::Reference { doc, .. }
            | FeatureDecl::Container { doc, .. }
            | FeatureDecl::Op { doc, .. }
            | FeatureDecl::Derived { doc, .. } => doc.as_deref(),
        }
    }

    /// Overwrites the feature's description. The parser uses this to attach
    /// the doc comment parsed directly above the feature.
    pub fn set_doc(&mut self, doc: Option<String>) {
        match self {
            FeatureDecl::Attribute { doc: slot, .. }
            | FeatureDecl::Containment { doc: slot, .. }
            | FeatureDecl::Reference { doc: slot, .. }
            | FeatureDecl::Container { doc: slot, .. }
            | FeatureDecl::Op { doc: slot, .. }
            | FeatureDecl::Derived { doc: slot, .. } => *slot = doc,
        }
    }

    /// The feature name.
    pub fn name(&self) -> &Name {
        match self {
            FeatureDecl::Attribute { name, .. }
            | FeatureDecl::Containment { name, .. }
            | FeatureDecl::Reference { name, .. }
            | FeatureDecl::Container { name, .. }
            | FeatureDecl::Op { name, .. }
            | FeatureDecl::Derived { name, .. } => name,
        }
    }

    /// A stable lowercase kind tag, e.g. `"attribute"`, `"op"`.
    pub fn kind(&self) -> &'static str {
        match self {
            FeatureDecl::Attribute { .. } => "attribute",
            FeatureDecl::Containment { .. } => "containment",
            FeatureDecl::Reference { .. } => "reference",
            FeatureDecl::Container { .. } => "container",
            FeatureDecl::Op { .. } => "op",
            FeatureDecl::Derived { .. } => "derived",
        }
    }
}

// --- .ddd design DSL ---------------------------------------------------------

/// The root node of a parsed `.ddd` source: `import` declarations followed by
/// exactly one `application` declaration. The application is mandatory per the
/// grammar; a source without one still parses (with an error) to
/// `application: None` so recovery never loses the imports. An `import` after
/// the application is a syntax error and is dropped from the AST.
#[derive(Debug, Clone, PartialEq)]
pub struct DddFile {
    /// The `import` declarations in source order.
    pub imports: Vec<ImportDecl>,
    /// The `application` declaration, when one was parsed.
    pub application: Option<DddApplication>,
}

/// The `application <name> { ... }` declaration of a `.ddd` file: the
/// designed application with its optional `base` package and its modules.
#[derive(Debug, Clone, PartialEq)]
pub struct DddApplication {
    /// The application name.
    pub name: Name,
    /// The `base` package declaration, when present. The grammar pins it to
    /// the first member position; a misplaced or repeated `base` is a syntax
    /// error reported by the parser.
    pub base: Option<DddBase>,
    /// The modules in source order.
    pub modules: Vec<DddModule>,
    /// Span of the whole declaration, `application` keyword included.
    pub span: Span,
}

/// The `base <qualified-name>` member of a [`DddApplication`]: the default
/// domain package for unqualified references.
#[derive(Debug, Clone, PartialEq)]
pub struct DddBase {
    /// The declared package name.
    pub package: QualifiedName,
    /// Span of the whole declaration, `base` keyword included.
    pub span: Span,
}

/// A `module <name> { ... }` of a [`DddApplication`]: a cohesive slice of
/// application services, designed classes, and search projections.
#[derive(Debug, Clone, PartialEq)]
pub struct DddModule {
    /// The module name.
    pub name: Name,
    /// Application services in source order.
    pub services: Vec<DddService>,
    /// Class designs in source order.
    pub designs: Vec<DddDesign>,
    /// Search projections in source order.
    pub searches: Vec<DddSearch>,
    /// Span of the whole declaration, `module` keyword included.
    pub span: Span,
}

/// A `service <name> { ... }` of a [`DddModule`]: a stateless application
/// service. The interleaved order of operations and `inject` lines is not
/// preserved; operations and dependencies are collected in their own source
/// orders (the same split the wire artifact makes).
#[derive(Debug, Clone, PartialEq)]
pub struct DddService {
    /// The service name.
    pub name: Name,
    /// Description from the doc comment directly above the declaration
    /// (contiguous `///` lines, joined).
    pub doc: Option<String>,
    /// Declared and delegated operations, in source order.
    pub operations: Vec<DddServiceOp>,
    /// Injected dependency names (`inject <name>;`), in source order.
    pub dependencies: Vec<Name>,
    /// Span of the whole declaration, `service` keyword included.
    pub span: Span,
}

/// One operation of a [`DddService`]: either a declared signature or a
/// delegation to an injected dependency's operation.
#[derive(Debug, Clone, PartialEq)]
pub struct DddServiceOp {
    /// The operation name. Contextual keywords are legal here: a delegated
    /// op may be named `save`.
    pub name: Name,
    /// The declared return type; `None` for delegation operations.
    pub return_type: Option<TypeRef>,
    /// Multiplicity annotation on the return type, if any.
    pub multiplicity: Option<Multiplicity>,
    /// Parameters in source order.
    pub params: Vec<DddParam>,
    /// The delegation target, when this operation forwards instead of
    /// declaring a signature.
    pub delegation: Option<DddDelegation>,
    /// Actor capability names from the `capability` clause, in source order.
    pub capabilities: Vec<Name>,
    /// Span of the whole operation, `;` included.
    pub span: Span,
}

/// The `=> <target>.<operation>` delegation of a [`DddServiceOp`].
#[derive(Debug, Clone, PartialEq)]
pub struct DddDelegation {
    /// The injected dependency receiving the call.
    pub target: QualifiedName,
    /// The operation invoked on the target.
    pub operation: Name,
    /// Span covering `target.operation`.
    pub span: Span,
}

/// One parameter of a declared operation: `type_ref multiplicity? name`.
#[derive(Debug, Clone, PartialEq)]
pub struct DddParam {
    /// The parameter type.
    pub type_ref: TypeRef,
    /// Multiplicity annotation, if any.
    pub multiplicity: Option<Multiplicity>,
    /// The parameter name.
    pub name: Name,
    /// Span covering the whole parameter.
    pub span: Span,
}

/// The DDD stereotype of a [`DddDesign`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DddStereotype {
    /// `entity` — a class with identity.
    Entity,
    /// `value` — an immutable value object.
    Value,
    /// `dto` — a data transfer object.
    Dto,
}

/// Which design flag a [`DddFlags::push`] recorded. The flags are contextual
/// keywords, so — like [`ModifierKind`] — the parser needs the kind to fill
/// the right slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DddFlagKind {
    /// `scaffold` — generate scaffolding.
    Scaffold,
    /// `auditable` — record an audit trail.
    Auditable,
    /// `optimisticLocking` — optimistic locking on persistent state.
    OptimisticLocking,
    /// `nonPersistent` — the class is never persisted.
    NonPersistent,
    /// `cache` — cache instances.
    Cache,
}

impl DddFlagKind {
    /// The keyword text as written in the source.
    pub fn keyword(self) -> &'static str {
        match self {
            DddFlagKind::Scaffold => "scaffold",
            DddFlagKind::Auditable => "auditable",
            DddFlagKind::OptimisticLocking => "optimisticLocking",
            DddFlagKind::NonPersistent => "nonPersistent",
            DddFlagKind::Cache => "cache",
        }
    }
}

/// The design flags of a [`DddDesign`], each carrying the span of the flag
/// keyword as written. Absence means the flag is off. Repeating a flag is
/// idempotent (the first span wins).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DddFlags {
    /// Span of the `scaffold` keyword, if present.
    pub scaffold: Option<Span>,
    /// Span of the `auditable` keyword, if present.
    pub auditable: Option<Span>,
    /// Span of the `optimisticLocking` keyword, if present.
    pub optimistic_locking: Option<Span>,
    /// Span of the `nonPersistent` keyword, if present.
    pub non_persistent: Option<Span>,
    /// Span of the `cache` keyword, if present.
    pub cache: Option<Span>,
}

impl DddFlags {
    /// Records one flag; repeats are idempotent (the first span wins).
    pub fn push(&mut self, kind: DddFlagKind, span: Span) {
        match kind {
            DddFlagKind::Scaffold => self.scaffold = self.scaffold.or(Some(span)),
            DddFlagKind::Auditable => self.auditable = self.auditable.or(Some(span)),
            DddFlagKind::OptimisticLocking => {
                self.optimistic_locking = self.optimistic_locking.or(Some(span))
            }
            DddFlagKind::NonPersistent => self.non_persistent = self.non_persistent.or(Some(span)),
            DddFlagKind::Cache => self.cache = self.cache.or(Some(span)),
        }
    }

    /// `true` when no flag is present.
    pub fn is_empty(&self) -> bool {
        self.scaffold.is_none()
            && self.auditable.is_none()
            && self.optimistic_locking.is_none()
            && self.non_persistent.is_none()
            && self.cache.is_none()
    }
}

/// A design declaration of a [`DddModule`]: the DDD decisions for one
/// referenced `.mox` class — `("abstract")? stereotype name flag*
/// ("repository" ...)?`.
#[derive(Debug, Clone, PartialEq)]
pub struct DddDesign {
    /// The referenced `.mox` class name.
    pub class: Name,
    /// The DDD stereotype.
    pub stereotype: DddStereotype,
    /// Whether the `abstract` modifier was written before the stereotype.
    pub is_abstract: bool,
    /// The design flags, in canonical field order regardless of source order.
    pub flags: DddFlags,
    /// The class's repository, when designed.
    pub repository: Option<DddRepository>,
    /// Span of the whole declaration (`abstract` included, when present).
    pub span: Span,
}

/// The `repository <name> { ... }` block of a [`DddDesign`].
#[derive(Debug, Clone, PartialEq)]
pub struct DddRepository {
    /// The repository name.
    pub name: Name,
    /// Repository operations in source order: built-ins and declared
    /// signatures interleaved as written.
    pub operations: Vec<DddRepositoryOp>,
    /// Span of the whole declaration, `repository` keyword included.
    pub span: Span,
}

/// One operation of a [`DddRepository`]: either a built-in (whose signature
/// the consumer knows) or a declared operation with an explicit signature.
#[derive(Debug, Clone, PartialEq)]
pub struct DddRepositoryOp {
    /// The operation name; for built-ins this is the written keyword.
    pub name: Name,
    /// The built-in op, when this is one; a built-in has no signature of its
    /// own.
    pub builtin: Option<DddBuiltinOp>,
    /// The declared return type; always `Some` for declared operations (the
    /// grammar requires it) and `None` for built-ins.
    pub return_type: Option<TypeRef>,
    /// Multiplicity annotation on the return type, if any.
    pub multiplicity: Option<Multiplicity>,
    /// Declared parameters in source order.
    pub params: Vec<DddParam>,
    /// Span of the whole operation, `;` included.
    pub span: Span,
}

/// A built-in repository operation, recognized by its contextual keyword in
/// repository-member position.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DddBuiltinOp {
    /// `findById` — fetch one aggregate by identity.
    FindById,
    /// `findAll` — fetch all aggregates.
    FindAll,
    /// `save` — insert or update an aggregate.
    Save,
    /// `delete` — delete an aggregate.
    Delete,
}

impl DddBuiltinOp {
    /// The built-in introduced by the given contextual keyword, or `None`
    /// for any other identifier.
    pub fn from_keyword(text: &str) -> Option<Self> {
        match text {
            "findById" => Some(DddBuiltinOp::FindById),
            "findAll" => Some(DddBuiltinOp::FindAll),
            "save" => Some(DddBuiltinOp::Save),
            "delete" => Some(DddBuiltinOp::Delete),
            _ => None,
        }
    }

    /// The keyword text as written in the source.
    pub fn keyword(self) -> &'static str {
        match self {
            DddBuiltinOp::FindById => "findById",
            DddBuiltinOp::FindAll => "findAll",
            DddBuiltinOp::Save => "save",
            DddBuiltinOp::Delete => "delete",
        }
    }
}

/// A `search <name> { ... }` projection of a [`DddModule`]: the declarative
/// search intent over one designed entity — indexed text fields, filter and
/// sort facets, computed document entries, ranking, pagination, and the
/// actor capabilities guarding it.
///
/// The interleaved source order of the members is not preserved; each
/// member kind is collected in its own source order (the same split the
/// wire artifact makes, and the canonical order the formatter emits).
#[derive(Debug, Clone, PartialEq)]
pub struct DddSearch {
    /// The search projection name.
    pub name: Name,
    /// Description from the doc comment directly above the declaration
    /// (contiguous `///` lines, joined).
    pub doc: Option<String>,
    /// The designed entity the projection is over; `None` when the source
    /// omitted the `entity` line (a driver-side validation error, not a
    /// syntax one).
    pub entity: Option<QualifiedName>,
    /// Indexed text fields in source order.
    pub text: Vec<DddSearchField>,
    /// Filter facet names in source order.
    pub filters: Vec<QualifiedName>,
    /// Sort facet names in source order.
    pub sort: Vec<QualifiedName>,
    /// Document projection entries in source order.
    pub document: Vec<DddDocumentEntry>,
    /// The declared ranking strategy, when present.
    pub ranking: Option<DddRanking>,
    /// The search-wide default analyzer, when present.
    pub analyzer: Option<String>,
    /// The declared pagination, when present.
    pub pagination: Option<DddPagination>,
    /// Actor capability names guarding the search, in source order.
    pub capabilities: Vec<Name>,
    /// Span of the whole declaration, `search` keyword included.
    pub span: Span,
}

/// One entry of a search's `text { ... }` clause: an indexed property with
/// its optional boost and per-field analyzer.
#[derive(Debug, Clone, PartialEq)]
pub struct DddSearchField {
    /// The indexed feature of the search's entity.
    pub property: QualifiedName,
    /// The `boost <int>` weight, when present.
    pub boost: Option<i64>,
    /// The `analyzer "<name>"` override, when present.
    pub analyzer: Option<String>,
    /// Span covering the whole field.
    pub span: Span,
}

/// One `name = <expr>;` entry of a search's `document { ... }` clause. The
/// expression is captured raw — its span slices the expression source
/// strictly between the `=` and the terminating `;` — exactly like the
/// `.mox` op bodies; parsing it is the driver's job.
#[derive(Debug, Clone, PartialEq)]
pub struct DddDocumentEntry {
    /// The document field name.
    pub name: Name,
    /// Span of the raw expression source (the `;` excluded).
    pub expr: Span,
}

/// The `ranking <strategy>` line of a [`DddSearch`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DddRanking {
    /// `ranking bm25`.
    Bm25,
    /// `ranking tfIdf`.
    TfIdf,
    /// `ranking exact`.
    Exact,
    /// `ranking custom "<name>"`.
    Custom(String),
}

/// The `pagination { ... }` block of a [`DddSearch`]: every member is
/// optional and defaults per the consumer's profile.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DddPagination {
    /// The `limit <int>` default page size.
    pub limit: Option<i64>,
    /// The `max <int>` page-size ceiling.
    pub max: Option<i64>,
    /// Whether `cursor` pagination was declared.
    pub cursor: bool,
    /// Span of the whole block, `pagination` keyword included.
    pub span: Span,
}
