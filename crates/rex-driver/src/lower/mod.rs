//! Semantic lowering: name resolution, validation, and AST → Core IR
//! translation for a single `.mox` source (one package per file).
//!
//! This module is salsa-free; the driver's tracked queries call into it.
//! Vocabulary declarations additionally read the vendored `vocab/` snapshots
//! and `model.lock` from disk, relative to the source file's directory.
//!
//! Layout: the shared types every section consumes live here (the
//! resolution records and `Scope` namespace, the lowering records, the
//! per-package `DomainUnit`); the function clusters live in their own
//! files — [`resolve`] (name resolution), [`imports`] (`import schema`),
//! [`compile`] (the one- and multi-file pipelines and the `.actor`
//! engine), [`conditions`] (`when` gates), [`classes`]
//! (class/feature/constraint lowering and class-graph validation),
//! [`vocabulary`] (vocabulary lowering plus the lockfile/vocab disk
//! reads), and [`actors`] (union policy-set lowering and its passes).

pub(crate) mod actors;
pub(crate) mod classes;
pub(crate) mod compile;
pub(crate) mod conditions;
pub(crate) mod imports;
pub(crate) mod resolve;
pub(crate) mod vocabulary;

pub(crate) use compile::{
    compile, compile_actor_file, compile_multi, compile_union_per_file, MultiFile,
};
pub(crate) use imports::collect_schema_imports;
pub(crate) use resolve::{resolve_ddd_class, resolve_ddd_type};

use std::collections::{HashMap, HashSet};

use rex_ir as ir;
use rex_syntax::ast as mox;
use rex_syntax::Span;

use crate::diagnostic::Diagnostic;

use resolve::{resolve_domains, resolve_single};

/// The kind of a top-level declaration that occupies the package namespace.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TopKind {
    Class,
    Interface,
    Enum,
    Datatype,
    Vocabulary,
    Actors,
}

impl TopKind {
    /// The lowercase noun used in diagnostics, e.g. "a class".
    fn label(self) -> &'static str {
        match self {
            TopKind::Class => "class",
            TopKind::Interface => "interface",
            TopKind::Enum => "enum",
            TopKind::Datatype => "datatype",
            TopKind::Vocabulary => "vocabulary",
            TopKind::Actors => "actors block",
        }
    }
}

/// A fully classified type reference.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Resolved {
    Primitive(ir::PrimitiveType),
    Enum,
    Datatype,
    Class,
    Interface,
    Vocabulary,
}

impl Resolved {
    /// The lowercase noun used in diagnostics, e.g. "class" or "enum".
    fn label(self) -> &'static str {
        match self {
            Resolved::Primitive(_) => "primitive",
            Resolved::Enum => "enum",
            Resolved::Datatype => "datatype",
            Resolved::Class => "class",
            Resolved::Interface => "interface",
            Resolved::Vocabulary => "vocabulary",
        }
    }

    /// Builds the resolved IR type reference.
    fn to_ir(self, package: &str, name: &str) -> ir::TypeRef {
        match self {
            Resolved::Primitive(primitive) => ir::TypeRef::Primitive(primitive),
            Resolved::Enum => ir::TypeRef::Enum {
                package: package.to_string(),
                name: name.to_string(),
            },
            Resolved::Datatype => ir::TypeRef::Datatype {
                package: package.to_string(),
                name: name.to_string(),
            },
            Resolved::Class => ir::TypeRef::Class {
                package: package.to_string(),
                name: name.to_string(),
            },
            Resolved::Interface => ir::TypeRef::Interface {
                package: package.to_string(),
                name: name.to_string(),
            },
            Resolved::Vocabulary => ir::TypeRef::Vocabulary {
                package: package.to_string(),
                name: name.to_string(),
            },
        }
    }
}

/// The outcome of resolving one type reference.
pub(crate) struct Resolution {
    kind: Resolved,
    /// The package that owns the resolved type.
    package: String,
    /// The local (final-segment) type name.
    name: String,
}

/// The type namespace of one package in a combined multi-package namespace
/// (imported domains of `.actor` files, or the files of a multi-file
/// compile).
pub(crate) struct DomainPackage {
    /// The package name (the `.mox` `package` declaration).
    pub(crate) name: String,
    /// The top-level declarations the package contributes, by name.
    pub(crate) kinds: HashMap<String, TopKind>,
}

/// The namespace a block resolves type references against: either a single
/// `.mox` package (the inline blocks of a plain model compile) or the
/// combined namespace of the imported domain packages (`.actor` files).
pub(crate) enum Scope<'a> {
    Single {
        package: &'a str,
        kinds: &'a HashMap<String, TopKind>,
    },
    Domains {
        packages: &'a [DomainPackage],
    },
}

impl Scope<'_> {
    /// The package named in unresolved-type fallbacks (the model is
    /// discarded whenever errors exist, so the exact value does not matter
    /// for the union path).
    fn fallback_package(&self) -> &str {
        match self {
            Scope::Single { package, .. } => package,
            Scope::Domains { packages } => packages
                .first()
                .map(|package| package.name.as_str())
                .unwrap_or(""),
        }
    }

    /// Resolves a type reference, emitting a diagnostic (and returning
    /// `None`) when it cannot be resolved.
    fn resolve(&self, type_ref: &mox::TypeRef, diags: &mut Vec<Diagnostic>) -> Option<Resolution> {
        match self {
            Scope::Single { package, kinds } => resolve_single(type_ref, package, kinds, diags),
            Scope::Domains { packages } => resolve_domains(type_ref, packages, diags),
        }
    }
}

/// The indefinite article for a lowercase kind label ("an enum",
/// "a class").
fn article(label: &str) -> &'static str {
    match label {
        "enum" | "interface" | "actors block" => "an",
        _ => "a",
    }
}

/// Classifies a found namespace entry; `None` (with a diagnostic) when the
/// name is unknown or is an actors block, which is not a data type.
fn classify(
    full_name: &str,
    _local: &str,
    span: Span,
    found: Option<&TopKind>,
    diags: &mut Vec<Diagnostic>,
) -> Option<Resolved> {
    match found {
        Some(TopKind::Class) => Some(Resolved::Class),
        Some(TopKind::Interface) => Some(Resolved::Interface),
        Some(TopKind::Enum) => Some(Resolved::Enum),
        Some(TopKind::Datatype) => Some(Resolved::Datatype),
        Some(TopKind::Vocabulary) => Some(Resolved::Vocabulary),
        Some(TopKind::Actors) => {
            diags.push(
                Diagnostic::error(
                    format!("type '{full_name}' is an actors block, not a data type"),
                    Some(span),
                )
                .with_help(
                    "actors blocks declare authorization models and cannot be used as types",
                ),
            );
            None
        }
        None => {
            diags.push(Diagnostic::error(
                format!("unknown type '{full_name}'"),
                Some(span),
            ));
            None
        }
    }
}

// --- `import schema` declarations --------------------------------------------

/// Feature kind as written in the source (superset of the IR's kinds: ops and
/// derived features exist syntactically even though neither lowers into the
/// stored feature list — ops lower into `ClassDef.operations` instead).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FKind {
    Attribute,
    Containment,
    Reference,
    Container,
    Op,
    Derived,
}

impl FKind {
    /// The source keyword naming this feature kind.
    fn keyword(self) -> &'static str {
        match self {
            FKind::Attribute => "attr",
            FKind::Containment => "contains",
            FKind::Reference => "refers",
            FKind::Container => "container",
            FKind::Op => "op",
            FKind::Derived => "derived",
        }
    }

    /// Human-readable description used in opposite-mismatch messages.
    fn describe(self) -> &'static str {
        match self {
            FKind::Attribute => "an attribute",
            FKind::Containment => "a `contains` feature",
            FKind::Reference => "a `refers` feature",
            FKind::Container => "a `container` feature",
            FKind::Op => "an operation",
            FKind::Derived => "a derived feature",
        }
    }

    /// The IR kind this lowers to; ops lower into `ClassDef.operations`
    /// instead of the stored feature list.
    fn ir_kind(self) -> Option<ir::FeatureKind> {
        match self {
            FKind::Attribute | FKind::Derived => Some(ir::FeatureKind::Attribute),
            FKind::Containment => Some(ir::FeatureKind::Containment),
            FKind::Reference => Some(ir::FeatureKind::CrossReference),
            FKind::Container => Some(ir::FeatureKind::Container),
            FKind::Op => None,
        }
    }
}

/// A per-feature record kept alongside the IR for cross-class validation
/// (opposite pairing).
pub(crate) struct FeatureRecord {
    name: String,
    kind: FKind,
    /// `(package, name)` of the target class when the declared type resolved
    /// to a class.
    type_class: Option<(String, String)>,
    /// The declared type as written (for messages).
    type_text: String,
    /// Span of the declared type reference.
    type_span: Option<Span>,
    /// The declared `opposite <name>` (the opposite feature's name).
    opposite: Option<String>,
    /// Span of the `opposite <name>` clause.
    opposite_span: Option<Span>,
}

/// A per-class record kept alongside the IR for cross-class validation.
pub(crate) struct ClassRecord {
    /// The package (and file) the class was declared in.
    package: String,
    file: String,
    name: String,
    name_span: Span,
    def: ir::ClassDef,
    features: Vec<FeatureRecord>,
    /// Lowered operations; appended to `def.operations` after validation.
    operations: Vec<ir::Operation>,
}

/// The type-knowledge tables shared by attribute lowering (default checks
/// and constraint family checks), keyed by `(package, type name)` so
/// cross-package references resolve in multi-file compiles.
pub(crate) struct PrepMaps<'a> {
    /// Enum declarations by owning package and local name (literal defaults
    /// and constraint closure checks).
    enum_decls: HashMap<(&'a str, &'a str), &'a mox::EnumDecl>,
    /// Entry-key sets of successfully lowered vocabularies.
    vocab_keys: HashMap<(&'a str, &'a str), HashSet<&'a str>>,
    /// The declared primitive type of each vocabulary's key facet, which
    /// classifies the constraint families admitted on vocabulary-typed
    /// attributes (see `lower_constraints`).
    vocab_key_facet_types: HashMap<(&'a str, &'a str), ir::PrimitiveType>,
}

/// A per-block record kept alongside the IR for cross-declaration actors
/// validation (inheritance cycles, separation of duty, self-narrowing,
/// delegations).
pub(crate) struct ActorsRecord {
    /// The file the block was lowered from: diagnostics produced by the
    /// cross-file passes are tagged with it.
    file: String,
    def: ir::ActorsDef,
    /// Actor names with their declaration-name spans, in declaration order.
    actor_names: Vec<(String, Span)>,
    /// Declared actor kinds, parallel to `actor_names`; `None` for plain
    /// `actor` declarations (only `agent` declares a kind).
    actor_kinds: Vec<(String, Option<ir::ActorKind>)>,
    /// Actor → parent actor name (`extends`), in declaration order.
    parents: Vec<(String, Option<String>)>,
    /// Declared purposes with their declaration-name spans, in declaration
    /// order.
    purposes: Vec<(String, Span)>,
    /// Declared obligations with their declaration-name spans, in
    /// declaration order.
    obligations: Vec<(String, Span)>,
    /// Obligation references from grant and delegation entries:
    /// (host context, name, name span), in source order.
    obligation_refs: Vec<ObligationRef>,
    /// `never_both` constraints: capability names plus the decl span.
    never_both: Vec<(Vec<String>, Span)>,
    /// `forbid` entries: (actor, capability, capability-name span).
    forbids: Vec<(String, String, Span)>,
    /// Delegations, in declaration order.
    delegations: Vec<DelegationRecord>,
}

/// A per-delegation record kept alongside the IR for the delegation passes
/// (endpoint resolution, target-kind check, permit invariant): the endpoint
/// names with their spans and the `permit` entries with theirs.
pub(crate) struct DelegationRecord {
    name: String,
    from: String,
    from_span: Span,
    to: String,
    to_span: Span,
    /// The delegation's declared purpose with its name span, if any.
    purpose: Option<(String, Span)>,
    /// `permit` entries whose capability is declared: (capability,
    /// capability-name span).
    permits: Vec<(String, Span)>,
}

/// One obligation reference of a grant or delegation entry: the host
/// context the diagnostics name, the obligation name, and its span.
pub(crate) struct ObligationRef {
    context: String,
    name: String,
    span: Span,
}

/// A `when` condition awaiting type checking once the class universe is
/// known. Lowering only syntax-checks conditions (so parse errors keep their
/// source order); the type check runs after every class is lowered.
pub(crate) struct PendingCondition {
    /// The file whose spans the condition refers to.
    file: String,
    /// The trimmed condition text.
    text: String,
    /// Host-file offset of the condition text's first byte.
    base: usize,
    /// The condition's inner region (strictly between the parens); mapped
    /// spans are clamped to it.
    inner_start: usize,
    inner_end: usize,
    /// The capability's owning class (package, name); `None` when the
    /// capability's class failed to resolve (that error is already reported).
    class: Option<(String, String)>,
}

/// One imported domain model handed to [`compile_actor_file`] and
/// [`crate::ddd::compile_ddd_file`]: its lowered model (when error-free),
/// its AST, and its own diagnostics.
pub(crate) struct DomainUnit {
    /// The path the actor file's import named (and the file was provided
    /// under); tags the unit's diagnostics and spans.
    pub path: String,
    /// The domain's full source text (condition and body slicing).
    pub source: String,
    /// The lowered domain model, `None` when the domain has errors (its
    /// blocks then stay out of the union).
    pub model: Option<ir::Model>,
    /// The domain's parsed AST.
    pub ast: Option<mox::Model>,
    /// The domain's own compile diagnostics (tagged with `path` here).
    pub diagnostics: Vec<Diagnostic>,
}

/// Builds the combined type namespace of the error-free domains (in
/// first-appearance order): each domain's lowered model is the single
/// source of truth — its declared top-level types, its `import schema`
/// nominal classes, and the synthetic sigil packages it carries are all
/// already lowered into it. This is the shared namespace the `.actor` and
/// `.ddd` pipelines resolve their references against. A domain that failed
/// to lower (its model is absent) contributes no namespaces.
pub(crate) fn domain_namespaces(domains: &[DomainUnit]) -> Vec<DomainPackage> {
    domains
        .iter()
        .filter_map(|unit| {
            let model = unit.model.as_ref()?;
            let mut packages_out: Vec<DomainPackage> = Vec::new();
            for (index, package) in model.packages.iter().enumerate() {
                // A synthetic package named like the file's own package
                // stays out (defensive: sigil namespaces colliding with
                // declared packages error at lowering, so an error-free
                // model cannot carry one).
                if index > 0 && package.name == model.packages[0].name {
                    continue;
                }
                let mut kinds: HashMap<String, TopKind> = HashMap::new();
                for class in &package.classes {
                    kinds.insert(class.name.clone(), TopKind::Class);
                }
                for interface in &package.interfaces {
                    kinds.insert(interface.name.clone(), TopKind::Interface);
                }
                for enum_ in &package.enums {
                    kinds.insert(enum_.name.clone(), TopKind::Enum);
                }
                for datatype in &package.datatypes {
                    kinds.insert(datatype.name.clone(), TopKind::Datatype);
                }
                for vocabulary in &package.vocabularies {
                    kinds.insert(vocabulary.name.clone(), TopKind::Vocabulary);
                }
                for actors in &package.actors {
                    kinds.insert(actors.name.clone(), TopKind::Actors);
                }
                packages_out.push(DomainPackage {
                    name: package.name.clone(),
                    kinds,
                });
            }
            Some(packages_out)
        })
        .flatten()
        .collect()
}

/// The verbatim text inside a target body's braces. Spans are byte offsets
/// into the same source the AST was parsed from; braces are one byte each,
/// so stripping first and last yields the exact inner text (spacing,
/// newlines, comments, and non-grammar characters intact).
pub(crate) fn body_text(source: &str, span: Span) -> &str {
    &source[span.start + 1..span.end - 1]
}

/// The built-in primitive names, matched case-insensitively.
fn primitive_type(name: &str) -> Option<ir::PrimitiveType> {
    match name.to_ascii_lowercase().as_str() {
        "string" => Some(ir::PrimitiveType::String),
        "int" => Some(ir::PrimitiveType::Int),
        "long" => Some(ir::PrimitiveType::Long),
        "short" => Some(ir::PrimitiveType::Short),
        "float" => Some(ir::PrimitiveType::Float),
        "double" => Some(ir::PrimitiveType::Double),
        "boolean" => Some(ir::PrimitiveType::Boolean),
        "byte" => Some(ir::PrimitiveType::Byte),
        "char" => Some(ir::PrimitiveType::Char),
        "date" => Some(ir::PrimitiveType::Date),
        _ => None,
    }
}

/// Builds an IR type reference from a resolution, or a placeholder (the model
/// is discarded whenever errors exist) when resolution failed.
fn ir_type_of(
    resolution: Option<&Resolution>,
    package: &str,
    type_ref: &mox::TypeRef,
) -> ir::TypeRef {
    match resolution {
        Some(resolution) => resolution.kind.to_ir(&resolution.package, &resolution.name),
        None => ir::TypeRef::Class {
            package: package.to_string(),
            name: type_ref
                .name
                .segments
                .last()
                .map(|segment| segment.text.clone())
                .unwrap_or_default(),
        },
    }
}

/// The verbatim text inside a `when (...)` condition: strictly inside the
/// parens, ends-trimmed (the same span-to-text convention as target bodies).
pub(crate) fn when_text(source: &str, span: Span) -> &str {
    source[span.start + 1..span.end - 1].trim()
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
    let inner_start = when_span.start + 1;
    let inner_end = when_span.end - 1;
    for error in rex_expr::parse(text).errors {
        let start = (base + error.span.start).clamp(inner_start, inner_end);
        let end = (base + error.span.end).clamp(start, inner_end);
        diags.push(Diagnostic::error(
            format!("invalid `when` condition: {}", error.message),
            Some((start..end).into()),
        ));
    }
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
