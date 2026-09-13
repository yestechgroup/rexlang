//! Abstract syntax tree for `.mox` sources.
//!
//! Every node carries a [`Span`] (a byte range into the original source text).
//! All types are owned (no borrows of the source), which keeps the AST easy to
//! cache in long-lived services such as a language server.

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
    /// The constraint keyword, e.g. `pattern`. Only the five constraint
    /// keywords are accepted (`pattern`, `minLength`, `maxLength`,
    /// `minimum`, `maximum`); other identifiers do not enter the block.
    pub name: Name,
    /// The constraint value.
    pub value: ConstraintValue,
    /// Span covering the whole entry.
    pub span: Span,
}

/// The value of a [`Constraint`] entry: a string (`pattern`) or an integer
/// (length and numeric bounds).
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
}

/// The root node of a parsed `.mox` source.
#[derive(Debug, Clone, PartialEq)]
pub struct Model {
    /// The `package` declaration, if present.
    pub package: Option<QualifiedName>,
    /// All other top-level declarations in source order.
    pub declarations: Vec<Decl>,
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
        }
    }

    /// The declared name, if the declaration kind has one. Annotations only
    /// have a name when the `as` clause is present.
    pub fn name(&self) -> Option<&Name> {
        match self {
            Decl::Class(decl) => Some(&decl.name),
            Decl::Interface(decl) => Some(&decl.name),
            Decl::Enum(decl) => Some(&decl.name),
            Decl::Datatype(decl) => Some(&decl.name),
            Decl::Annotation(decl) => decl.name.as_ref(),
            Decl::Vocabulary(decl) => Some(&decl.name),
            Decl::Actors(decl) => Some(&decl.name),
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
/// Cedar spirit. Actors, capabilities, grants and `never_both` exclusivity
/// constraints are collected in source order into separate lists.
#[derive(Debug, Clone, PartialEq)]
pub struct ActorsDecl {
    /// The actors-block name.
    pub name: Name,
    /// Declared actors, in source order.
    pub actors: Vec<ActorDecl>,
    /// Declared capabilities, in source order.
    pub capabilities: Vec<CapabilityDecl>,
    /// Declared grants, in source order.
    pub grants: Vec<GrantDecl>,
    /// Declared `never_both` exclusivity constraints, in source order.
    pub never_both: Vec<NeverBothDecl>,
    /// Span of the whole declaration.
    pub span: Span,
}

/// A single `actor <name> ("extends" <name>)?` member of an [`ActorsDecl`].
#[derive(Debug, Clone, PartialEq)]
pub struct ActorDecl {
    /// The actor name.
    pub name: Name,
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
