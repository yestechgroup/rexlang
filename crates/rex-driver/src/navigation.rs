//! A navigation index over a parsed `.mox` model: definitions, references,
//! and span-based lookups for editors and language tools.
//!
//! The index is a pure function of the AST — no I/O, no async runtime, no
//! LSP types. Build it once per parse (see [`NavigationIndex::build`] and
//! [`NavigationIndex::build_or_empty`]) and query it by byte offset:
//!
//! ```
//! let source = "package demo\n\nclass Book { String title }";
//! let index = rex_driver::navigation::NavigationIndex::build_or_empty(source);
//! let (_, book) = index.definitions().find(|(_, d)| d.name == "Book").unwrap();
//! let lookup = index.at(book.name_span.start);
//! assert!(matches!(lookup, rex_driver::navigation::Lookup::Definition(..)));
//! ```
//!
//! # Resolution rules
//!
//! The index mirrors the resolver in the driver's lowering stage (one
//! package per file):
//!
//! * A type reference resolves to the top-level declaration whose name
//!   matches its **last** segment. Both the single-segment form (`Book`) and
//!   the two-segment `<package>.<Name>` form are supported; primitives
//!   (`String`, `int`, ...) and anything else unresolvable become references
//!   with a target of `None`.
//! * An `opposite <name>` mention on a feature of class `A` typed `B`
//!   resolves to the feature `<name>` owned by `B`, best-effort: when the
//!   opposite is invalid (missing or mismatched feature), the reference
//!   resolves to `None` but is still indexed.
//! * `extends` type references resolve to class or interface definitions.
//! * Features are one definition each (attributes, containments,
//!   cross-references, containers, operations, derived features), owned by
//!   their class; enum literals are owned by their enum; vocabulary facets
//!   are owned by their vocabulary.

use std::collections::HashMap;

use rex_syntax::ast as mox;
use rex_syntax::Span;

/// Stable identifier of a [`Definition`] within one [`NavigationIndex`].
///
/// Ids are assigned in source order and are only meaningful for the index
/// that produced them.
pub type DefId = usize;

/// The kind of a feature definition in a [`NavigationIndex`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FeatureSymbolKind {
    /// A plain `type name` attribute.
    Attribute,
    /// A `contains` composition end.
    Containment,
    /// A `refers` association end.
    CrossReference,
    /// A `container` back-pointer.
    Container,
    /// An `op` operation.
    Operation,
    /// A `derived` feature.
    Derived,
}

impl FeatureSymbolKind {
    /// Maps an AST feature declaration to its symbol kind.
    fn of(feature: &mox::FeatureDecl) -> Self {
        match feature {
            mox::FeatureDecl::Attribute { .. } => Self::Attribute,
            mox::FeatureDecl::Containment { .. } => Self::Containment,
            mox::FeatureDecl::Reference { .. } => Self::CrossReference,
            mox::FeatureDecl::Container { .. } => Self::Container,
            mox::FeatureDecl::Op { .. } => Self::Operation,
            mox::FeatureDecl::Derived { .. } => Self::Derived,
        }
    }
}

/// The kind of a [`Definition`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SymbolKind {
    /// A `class` declaration.
    Class,
    /// An `interface` declaration.
    Interface,
    /// An `enum` declaration.
    Enum,
    /// A `type ... wraps ...` declaration.
    Datatype,
    /// A `vocabulary ... from ...` declaration.
    Vocabulary,
    /// A feature of a class, interface member, or vocabulary facet.
    Feature(FeatureSymbolKind),
    /// An enum literal, owned by its enum.
    EnumLiteral,
}

/// A named symbol declared in the model: a top-level declaration, a feature,
/// an enum literal, or a vocabulary facet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Definition {
    /// The declared name.
    pub name: String,
    /// What kind of symbol this is.
    pub kind: SymbolKind,
    /// Span of the identifier itself (excluding keywords and types).
    pub name_span: Span,
    /// The definition that owns this one: the class owning a feature, the
    /// enum owning a literal, or the vocabulary owning a facet. `None` for
    /// top-level declarations.
    pub owner: Option<DefId>,
    /// The feature's declared type as written (e.g. `Book`), for features
    /// and operations; `None` for everything else.
    pub type_text: Option<String>,
    /// The feature's multiplicity rendered as written (e.g. `[]`, `[2]`,
    /// `[1..*]`); `None` when the feature has no multiplicity annotation.
    pub multiplicity_text: Option<String>,
    /// The feature's `opposite` end name as written; `None` when absent.
    pub opposite_text: Option<String>,
    /// The enum literal's numeric value as written (e.g. `0`); `None` for
    /// everything but literals with an `=` clause.
    pub value_text: Option<String>,
    /// The class's `extends` clause as written, entries comma-separated;
    /// `None` when the class has none (or is not a class).
    pub extends_text: Option<String>,
    /// Span of the whole declaration this definition comes from (e.g. the
    /// full `class Book { … }` source, the full feature line).
    pub full_span: Span,
}

/// A name mention that refers to a declaration: a type reference, an
/// `opposite` mention, or a facet type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reference {
    /// Span of the mention as written (for qualified names, the whole
    /// `a.b` span; for `opposite` mentions, the identifier itself).
    pub span: Span,
    /// The definition this mention resolves to, or `None` when it could not
    /// be resolved (primitives, unknown names, invalid opposites).
    pub target: Option<DefId>,
}

/// The result of looking up a byte offset in a [`NavigationIndex`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lookup<'a> {
    /// The offset falls inside a definition's name span.
    Definition(DefId, &'a Definition),
    /// The offset falls inside a reference's span.
    Reference(&'a Reference),
    /// The offset hits nothing the index knows about.
    None,
}

/// An index of all definitions and references in one parsed `.mox` model,
/// supporting offset-based navigation (hover, go-to-definition, find
/// references) without re-parsing.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NavigationIndex {
    definitions: Vec<Definition>,
    references: Vec<Reference>,
}

impl NavigationIndex {
    /// Builds an index for a parsed model. Definitions and references are
    /// indexed in source order; [`DefId`]s equal the definition's position.
    pub fn build(model: &mox::Model) -> Self {
        let mut index = Self::default();
        let package = model
            .package
            .as_ref()
            .map(|package| package.full_name())
            .unwrap_or_default();

        // Pass 1: assign ids in source order (a declaration, then its
        // members) so that definitions can be pushed in a single pass.
        // Duplicate names resolve to the first declaration, mirroring the
        // resolver.
        let mut top_level: HashMap<&str, DefId> = HashMap::new();
        let mut decl_ids: Vec<DefId> = Vec::new();
        let mut next_id = 0;
        for decl in &model.declarations {
            let (name, member_count) = match decl {
                mox::Decl::Class(decl) => (&decl.name, decl.features.len()),
                mox::Decl::Interface(decl) => (&decl.name, 0),
                mox::Decl::Enum(decl) => (&decl.name, decl.literals.len()),
                mox::Decl::Datatype(decl) => (&decl.name, 0),
                mox::Decl::Vocabulary(decl) => (&decl.name, decl.facets.len()),
                mox::Decl::Annotation(_) => continue,
            };
            top_level.entry(name.text.as_str()).or_insert(next_id);
            decl_ids.push(next_id);
            next_id += 1 + member_count;
        }

        // Pass 2: push definitions (in id order) and the type references.
        // Opposite mentions may target features of classes declared later,
        // so they are collected and resolved afterwards.
        let mut opposites: Vec<(String, String, Span)> = Vec::new();
        let mut declaration_index = 0;
        for decl in &model.declarations {
            let (name, kind) = match decl {
                mox::Decl::Class(decl) => (&decl.name, SymbolKind::Class),
                mox::Decl::Interface(decl) => (&decl.name, SymbolKind::Interface),
                mox::Decl::Enum(decl) => (&decl.name, SymbolKind::Enum),
                mox::Decl::Datatype(decl) => (&decl.name, SymbolKind::Datatype),
                mox::Decl::Vocabulary(decl) => (&decl.name, SymbolKind::Vocabulary),
                mox::Decl::Annotation(_) => continue,
            };
            let extends_text = match decl {
                mox::Decl::Class(decl) if !decl.extends.is_empty() => Some(
                    decl.extends
                        .iter()
                        .map(|type_ref| type_ref.name.full_name())
                        .collect::<Vec<_>>()
                        .join(", "),
                ),
                _ => None,
            };
            // Pass 1 precomputed ids in exactly this push order.
            let decl_id = index.push(name.text.clone(), kind, name.span, None, decl.span());
            debug_assert_eq!(Some(decl_id), decl_ids.get(declaration_index).copied());
            declaration_index += 1;
            index.definitions[decl_id].extends_text = extends_text;
            match decl {
                mox::Decl::Class(decl) => {
                    for type_ref in &decl.extends {
                        index.add_type_reference(type_ref, &package, &top_level);
                    }
                    for feature in &decl.features {
                        let feature_id = index.push(
                            feature.name().text.clone(),
                            SymbolKind::Feature(FeatureSymbolKind::of(feature)),
                            feature.name().span,
                            Some(decl_id),
                            feature.span(),
                        );
                        match feature {
                            mox::FeatureDecl::Attribute { type_ref, multiplicity, .. }
                            | mox::FeatureDecl::Derived { type_ref, multiplicity, .. } => {
                                index.definitions[feature_id].type_text =
                                    Some(type_ref.name.full_name());
                                index.definitions[feature_id].multiplicity_text =
                                    multiplicity.as_ref().map(render_multiplicity);
                                index.add_type_reference(type_ref, &package, &top_level);
                            }
                            mox::FeatureDecl::Containment {
                                type_ref,
                                multiplicity,
                                opposite,
                                ..
                            }
                            | mox::FeatureDecl::Reference {
                                type_ref,
                                multiplicity,
                                opposite,
                                ..
                            } => {
                                index.definitions[feature_id].type_text =
                                    Some(type_ref.name.full_name());
                                index.definitions[feature_id].multiplicity_text =
                                    multiplicity.as_ref().map(render_multiplicity);
                                index.definitions[feature_id].opposite_text =
                                    opposite.as_ref().map(|opposite| opposite.text.clone());
                                index.add_type_reference(type_ref, &package, &top_level);
                                if let Some(opposite) = opposite {
                                    opposites.push((
                                        type_ref
                                            .name
                                            .segments
                                            .last()
                                            .map(|segment| segment.text.clone())
                                            .unwrap_or_default(),
                                        opposite.text.clone(),
                                        opposite.span,
                                    ));
                                }
                            }
                            mox::FeatureDecl::Container {
                                type_ref, opposite, ..
                            } => {
                                index.definitions[feature_id].type_text =
                                    Some(type_ref.name.full_name());
                                index.definitions[feature_id].opposite_text =
                                    opposite.as_ref().map(|opposite| opposite.text.clone());
                                index.add_type_reference(type_ref, &package, &top_level);
                                if let Some(opposite) = opposite {
                                    opposites.push((
                                        type_ref
                                            .name
                                            .segments
                                            .last()
                                            .map(|segment| segment.text.clone())
                                            .unwrap_or_default(),
                                        opposite.text.clone(),
                                        opposite.span,
                                    ));
                                }
                            }
                            mox::FeatureDecl::Op {
                                return_type, params, ..
                            } => {
                                index.definitions[feature_id].type_text =
                                    Some(return_type.name.full_name());
                                index.add_type_reference(return_type, &package, &top_level);
                                for param in params {
                                    index.add_type_reference(
                                        &param.type_ref,
                                        &package,
                                        &top_level,
                                    );
                                }
                            }
                        }
                    }
                }
                mox::Decl::Enum(decl) => {
                    for literal in &decl.literals {
                        let literal_id = index.push(
                            literal.name.text.clone(),
                            SymbolKind::EnumLiteral,
                            literal.name.span,
                            Some(decl_id),
                            literal.span,
                        );
                        if let Some(value) = literal.value {
                            index.definitions[literal_id].value_text = Some(value.to_string());
                        }
                    }
                }
                mox::Decl::Vocabulary(decl) => {
                    for facet in &decl.facets {
                        let facet_id = index.push(
                            facet.name.text.clone(),
                            SymbolKind::Feature(FeatureSymbolKind::Attribute),
                            facet.name.span,
                            Some(decl_id),
                            facet.span,
                        );
                        index.definitions[facet_id].type_text =
                            Some(facet.type_ref.name.full_name());
                        index.add_type_reference(&facet.type_ref, &package, &top_level);
                    }
                }
                // Interfaces hold only target bindings (no names to index);
                // datatypes wrap foreign names; annotations are unnamed.
                mox::Decl::Interface(_) | mox::Decl::Datatype(_) | mox::Decl::Annotation(_) => {}
            }
        }

        // Pass 3: resolve opposite mentions against the complete definition
        // set, best-effort (no class or no such feature → no target).
        for (class_name, feature_name, span) in opposites {
            let target = top_level
                .get(class_name.as_str())
                .and_then(|&class_id| {
                    index.definitions.iter().position(|definition| {
                        definition.owner == Some(class_id) && definition.name == feature_name
                    })
                });
            index.references.push(Reference { span, target });
        }
        // Opposites were resolved out of order; restore source order.
        index.references.sort_by_key(|reference| reference.span.start);
        index
    }

    /// Convenience for language tools: parses `source` (fault-tolerantly)
    /// and builds the index, returning an empty index when no AST could be
    /// produced. Never panics on malformed input.
    pub fn build_or_empty(source: &str) -> Self {
        match rex_syntax::parse(source).ast {
            Some(model) => Self::build(&model),
            None => Self::default(),
        }
    }

    /// Returns the definition or reference whose span contains `offset`.
    pub fn at(&self, offset: usize) -> Lookup<'_> {
        for (id, definition) in self.definitions.iter().enumerate() {
            if definition.name_span.start <= offset && offset < definition.name_span.end {
                return Lookup::Definition(id, definition);
            }
        }
        for reference in &self.references {
            if reference.span.start <= offset && offset < reference.span.end {
                return Lookup::Reference(reference);
            }
        }
        Lookup::None
    }

    /// Returns the definition with the given id. Ids are only meaningful
    /// for the index that produced them; passing an id from another index
    /// (or an id of a dropped index) panics.
    pub fn definition(&self, id: DefId) -> &Definition {
        &self.definitions[id]
    }

    /// Resolves a reference produced by this index to its definition, or
    /// `None` when the reference has no target (primitives, unknown names,
    /// invalid opposites).
    pub fn resolve(&self, reference: &Reference) -> Option<&Definition> {
        reference.target.and_then(|id| self.definitions.get(id))
    }

    /// Resolves a reference to its [`DefId`] within this index — the id
    /// form of [`NavigationIndex::resolve`], for callers that need to name
    /// the target rather than inspect it.
    pub fn resolve_id(&self, reference: &Reference) -> Option<DefId> {
        reference
            .target
            .filter(|&id| id < self.definitions.len())
    }

    /// Iterates all references in source order as `(span, reference)` pairs.
    pub fn references(&self) -> impl Iterator<Item = (&Span, &Reference)> {
        self.references
            .iter()
            .map(|reference| (&reference.span, reference))
    }

    /// Returns the span of every mention that resolves to `target` — type
    /// references, `extends` clauses, and `opposite` mentions — in source
    /// order. Use this to power rename and find-references.
    pub fn references_to(&self, target: DefId) -> Vec<Span> {
        self.references
            .iter()
            .filter(|reference| reference.target == Some(target))
            .map(|reference| reference.span)
            .collect()
    }

    /// Iterates all definitions in source order as `(id, definition)` pairs.
    pub fn definitions(&self) -> impl Iterator<Item = (DefId, &Definition)> {
        self.definitions.iter().enumerate()
    }

    /// Appends a definition and returns its id.
    fn push(
        &mut self,
        name: String,
        kind: SymbolKind,
        name_span: Span,
        owner: Option<DefId>,
        full_span: Span,
    ) -> DefId {
        let id = self.definitions.len();
        self.definitions.push(Definition {
            name,
            kind,
            name_span,
            owner,
            type_text: None,
            multiplicity_text: None,
            opposite_text: None,
            value_text: None,
            extends_text: None,
            full_span,
        });
        id
    }

    /// Records a type reference resolved with the standard name rules.
    fn add_type_reference(
        &mut self,
        type_ref: &mox::TypeRef,
        package: &str,
        top_level: &HashMap<&str, DefId>,
    ) {
        let target = type_target(type_ref, package, top_level);
        self.references.push(Reference {
            span: type_ref.span,
            target,
        });
    }
}

/// Resolves a type reference's last segment to a definition id, mirroring
/// the resolver: single-segment names are package-local (primitives resolve
/// to nothing), and the only supported qualified form is `<package>.<Name>`.
fn type_target(    type_ref: &mox::TypeRef,
    package: &str,
    top_level: &HashMap<&str, DefId>,
) -> Option<DefId> {
    let segments = &type_ref.name.segments;
    let local = match segments.as_slice() {
        [segment] => {
            if is_primitive(&segment.text) {
                return None;
            }
            &segment.text
        }
        [head, tail] if head.text == *package => &tail.text,
        _ => return None,
    };
    top_level.get(local.as_str()).copied()
}

/// `true` for the built-in primitive type names, matched case-insensitively
/// (same set as the driver's resolver).
fn is_primitive(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "string" | "int" | "long" | "short" | "float" | "double" | "boolean" | "byte" | "char"
    )
}

/// Renders a multiplicity annotation as written (e.g. `[]`, `[2]`, `[1..*]`).
fn render_multiplicity(multiplicity: &mox::Multiplicity) -> String {
    match &multiplicity.kind {
        mox::MultiplicityKind::Unbounded => "[]".to_string(),
        mox::MultiplicityKind::Exact(bound) => format!("[{bound}]"),
        mox::MultiplicityKind::Range(lower, upper) => match upper {
            mox::MultBound::Star => format!("[{lower}..*]"),
            mox::MultBound::Int(upper) => format!("[{lower}..{upper}]"),
        },
    }
}
