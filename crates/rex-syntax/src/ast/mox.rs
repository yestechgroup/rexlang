//! The `.mox` AST: the model root, package declarations, and feature
//! declarations.

use super::*;

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
    /// The declared `table "…"` storage-name override, when present. The
    /// driver carries it verbatim as the IR's `ClassDef.tableName` field;
    /// the uniqueness check uses it as the class's effective table name.
    pub table: Option<String>,
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
