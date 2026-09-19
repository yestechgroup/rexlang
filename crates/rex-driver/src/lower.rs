//! Semantic lowering: name resolution, validation, and AST → Core IR
//! translation for a single `.mox` source (one package per file).
//!
//! This module is salsa-free; the driver's tracked queries call into it.
//! Vocabulary declarations additionally read the vendored `vocab/` snapshots
//! and `model.lock` from disk, relative to the source file's directory.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};

use rex_expr::{Ty, TypeChecker, TypeContext};
use rex_ir as ir;
use rex_syntax::ast as mox;
use rex_syntax::Span;

use crate::diagnostic::{Diagnostic, DiagnosticCode};

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
enum Resolved {
    Primitive(ir::PrimitiveType),
    Enum,
    Datatype,
    Class,
    Interface,
    Vocabulary,
}

impl Resolved {
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
struct Resolution {
    kind: Resolved,
    /// The package that owns the resolved type.
    package: String,
    /// The local (final-segment) type name.
    name: String,
}

/// The type namespace of one package in a combined multi-package namespace
/// (imported domains of `.actor` files, or the files of a multi-file
/// compile).
struct DomainPackage {
    name: String,
    kinds: HashMap<String, TopKind>,
}

/// The namespace a block resolves type references against: either a single
/// `.mox` package (the inline blocks of a plain model compile) or the
/// combined namespace of the imported domain packages (`.actor` files).
enum Scope<'a> {
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

/// Resolves within a single package: primitives and single-segment names are
/// package-local; the only qualified form accepted is `<package>.<Name>`.
fn resolve_single(
    type_ref: &mox::TypeRef,
    package: &str,
    kinds: &HashMap<String, TopKind>,
    diags: &mut Vec<Diagnostic>,
) -> Option<Resolution> {
    let segments = &type_ref.name.segments;
    let full_name = type_ref.name.full_name();

    let local = match segments.len() {
        1 => &segments[0].text,
        2 if segments[0].text == *package => &segments[1].text,
        _ => {
            diags.push(Diagnostic::error(
                "cross-package type references are not supported yet",
                Some(type_ref.span),
            ));
            return None;
        }
    };

    if segments.len() == 1 {
        if let Some(primitive) = primitive_type(local) {
            return Some(Resolution {
                kind: Resolved::Primitive(primitive),
                package: package.to_string(),
                name: local.clone(),
            });
        }
    }

    let resolved = classify(
        &full_name,
        local,
        type_ref.span,
        kinds.get(local.as_str()),
        diags,
    )?;
    Some(Resolution {
        kind: resolved,
        package: package.to_string(),
        name: local.clone(),
    })
}

/// Resolves against the combined namespace of the imported domain packages:
/// a single-segment name is looked up in every package (ambiguous when found
/// in more than one); a qualified name's leading segments must name exactly
/// one package.
fn resolve_domains(
    type_ref: &mox::TypeRef,
    packages: &[DomainPackage],
    diags: &mut Vec<Diagnostic>,
) -> Option<Resolution> {
    let segments = &type_ref.name.segments;
    let full_name = type_ref.name.full_name();

    if segments.len() >= 2 {
        let package_name = segments[..segments.len() - 1]
            .iter()
            .map(|segment| segment.text.as_str())
            .collect::<Vec<_>>()
            .join(".");
        let local = &segments[segments.len() - 1].text;
        if let Some(package) = packages.iter().find(|package| package.name == package_name) {
            let resolved = classify(
                &full_name,
                local,
                type_ref.span,
                package.kinds.get(local.as_str()),
                diags,
            )?;
            return Some(Resolution {
                kind: resolved,
                package: package.name.clone(),
                name: local.clone(),
            });
        }
        diags.push(Diagnostic::error(
            format!("unknown type '{full_name}'"),
            Some(type_ref.span),
        ));
        return None;
    }

    let local = &segments[0].text;
    if let Some(primitive) = primitive_type(local) {
        return Some(Resolution {
            kind: Resolved::Primitive(primitive),
            package: String::new(),
            name: local.clone(),
        });
    }
    let matches: Vec<&DomainPackage> = packages
        .iter()
        .filter(|package| package.kinds.contains_key(local.as_str()))
        .collect();
    match matches.len() {
        0 => {
            diags.push(Diagnostic::error(
                format!("unknown type '{full_name}'"),
                Some(type_ref.span),
            ));
            None
        }
        1 => {
            let package = matches[0];
            let resolved = classify(
                &full_name,
                local,
                type_ref.span,
                package.kinds.get(local.as_str()),
                diags,
            )?;
            Some(Resolution {
                kind: resolved,
                package: package.name.clone(),
                name: local.clone(),
            })
        }
        _ => {
            let mut names: Vec<&str> = matches
                .iter()
                .map(|package| package.name.as_str())
                .collect();
            names.sort();
            diags.push(
                Diagnostic::error(
                    format!(
                        "ambiguous type `{local}`; qualify as `{}.{local}`",
                        names[0]
                    ),
                    Some(type_ref.span),
                )
                .with_help(format!(
                    "matching types: {}",
                    names
                        .iter()
                        .map(|name| format!("{name}.{local}"))
                        .collect::<Vec<_>>()
                        .join(", ")
                )),
            );
            None
        }
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

/// The name an `import schema` declaration joins its package's namespace
/// as: the `as` alias, or the import path's file stem when the alias is
/// absent (`"deep/dir/todo_item.json"` imports as `todo_item`).
pub(crate) fn imported_name(decl: &mox::ImportSchemaDecl) -> String {
    if let Some(alias) = &decl.alias {
        return alias.text.clone();
    }
    Path::new(&decl.path)
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// One validated `import schema` declaration: the name it joins the
/// namespace as (alias or file stem).
#[derive(Debug, Clone)]
pub(crate) struct SchemaImport {
    /// The namespace name (alias, or the path's file stem).
    pub name: String,
}

/// `true` for a valid `.mox` identifier shape (`[A-Za-z_][A-Za-z0-9_]*`),
/// which a stem-derived import name must have to be referencable.
fn is_identifier(text: &str) -> bool {
    let mut chars = text.chars();
    matches!(chars.next(), Some(c) if c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Collects and validates the `import schema` declarations of one file.
///
/// v1 lowering is deliberately **opaque**: an import registers a nominal,
/// feature-less class — no features, enums, or datatypes are derived from
/// the JSON content (structural lowering is future work). The content is
/// still validated: it must be provided (the driver is filesystem-free —
/// the host passes JSON text through [`crate::SchemaImports`]) and it must
/// parse as JSON.
///
/// Collisions are errors naming both sources: an alias (or stem) that
/// matches a declaration of the same package, the same name imported twice,
/// and two stem-derived names that collide. Validated imports are returned
/// in source order; collided ones are skipped so the namespace stays clean
/// for the remaining diagnostics.
pub(crate) fn collect_schema_imports(
    file_path: &str,
    ast: &mox::Model,
    provided: Option<&crate::SchemaImports>,
    declared: &HashMap<String, TopKind>,
    diags: &mut Vec<Diagnostic>,
) -> Vec<SchemaImport> {
    let mut imports: Vec<SchemaImport> = Vec::new();
    let mut seen: HashMap<String, String> = HashMap::new();
    for decl in ast.declarations.iter() {
        let mox::Decl::ImportSchema(decl) = decl else {
            continue;
        };
        let name = imported_name(decl);
        let span = decl.span;
        if let Some(kind) = declared.get(&name).copied() {
            diags.push(
                Diagnostic::error(
                    format!(
                        "import schema '{}' declares '{}', which is already declared in this \
                         package as a {}",
                        decl.path,
                        name,
                        kind.label()
                    ),
                    Some(span),
                )
                .with_help(format!(
                    "rename the import: `import schema \"{}\" as <OtherName>`",
                    decl.path
                )),
            );
            continue;
        }
        if let Some(previous) = seen.get(&name) {
            diags.push(
                Diagnostic::error(
                    format!(
                        "duplicate import name '{name}': imported from '{previous}' and '{}'",
                        decl.path
                    ),
                    Some(span),
                )
                .with_help("import each schema under a distinct `as` name"),
            );
            continue;
        }
        if name.is_empty() {
            diags.push(Diagnostic::error(
                format!(
                    "import schema '{}' has no file stem to name the import",
                    decl.path
                ),
                Some(span),
            ));
            continue;
        }
        if decl.alias.is_none() && !is_identifier(&name) {
            diags.push(
                Diagnostic::error(
                    format!("imported schema name '{name}' is not a valid identifier"),
                    Some(span),
                )
                .with_help(format!(
                    "add an alias: `import schema \"{}\" as <Name>`",
                    decl.path
                )),
            );
            continue;
        }
        match provided.and_then(|schema| schema.get(file_path, &decl.path)) {
            None => {
                diags.push(
                    Diagnostic::error(
                        format!("imported schema '{}' was not provided", decl.path),
                        Some(span),
                    )
                    .with_help(
                        "import schema content is provided by the host: the CLI reads the \
                         JSON next to the model; embedded callers pass it through \
                         SchemaImports",
                    ),
                );
                continue;
            }
            Some(json) => {
                if let Err(error) = serde_json::from_str::<serde_json::Value>(json) {
                    diags.push(Diagnostic::error(
                        format!("imported schema '{}' is not valid JSON: {error}", decl.path),
                        Some(span),
                    ));
                    continue;
                }
            }
        }
        seen.insert(name.clone(), decl.path.clone());
        imports.push(SchemaImport { name });
    }
    imports
}

/// The nominal IR class one validated import lowers into: a feature-less,
/// empty-extends `ClassDef` (v1 is opaque; structural lowering is future
/// work).
pub(crate) fn import_class_def(import: &SchemaImport) -> ir::ClassDef {
    ir::ClassDef::new(import.name.clone(), Vec::new(), Vec::new())
}

/// The per-target backends known to the driver, in warning-message order.
/// `expr` is the Tier-2 pseudo-target holding the neutral expression
/// language (the Rust backend lowers it; see `docs/EXPRESSIONS.md`).
const KNOWN_TARGETS: [&str; 4] = ["rust", "csharp", "java", "expr"];

/// The verbatim text inside a target body's braces. Spans are byte offsets
/// into the same source the AST was parsed from; braces are one byte each,
/// so stripping first and last yields the exact inner text (spacing,
/// newlines, comments, and non-grammar characters intact).
fn body_text(source: &str, span: Span) -> &str {
    &source[span.start + 1..span.end - 1]
}

/// Lowers target-tagged bodies into a verbatim `target -> code` map,
/// warning on unknown target names, erroring on duplicates, and erroring on
/// the `rust` + `expr` conflict (one operation, one body language).
fn lower_target_bodies(
    bodies: &[mox::TargetBody],
    source: &str,
    subject: &str,
    diags: &mut Vec<Diagnostic>,
) -> BTreeMap<String, String> {
    let mut lowered = BTreeMap::new();
    let mut spans: BTreeMap<String, Span> = BTreeMap::new();
    for body in bodies {
        if !KNOWN_TARGETS.contains(&body.target.text.as_str()) {
            diags.push(Diagnostic::warning(
                format!(
                    "unknown target '{}' (known: {})",
                    body.target.text,
                    KNOWN_TARGETS.join(", ")
                ),
                Some(body.target.span),
            ));
        }
        if lowered
            .insert(
                body.target.text.clone(),
                body_text(source, body.span).to_string(),
            )
            .is_some()
        {
            diags.push(Diagnostic::error(
                format!("duplicate target body '{}' for {subject}", body.target.text),
                Some(body.target.span),
            ));
        }
        spans.insert(body.target.text.clone(), body.target.span);
    }
    if lowered.contains_key("rust") && lowered.contains_key("expr") {
        diags.push(Diagnostic::error(
            "conflicting bodies for targets rust and expr",
            spans.get("expr").copied(),
        ));
    }
    lowered
}

/// Feature kind as written in the source (superset of the IR's kinds: ops and
/// derived features exist syntactically even though neither lowers into the
/// stored feature list — ops lower into `ClassDef.operations` instead).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FKind {
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
struct FeatureRecord {
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
struct ClassRecord {
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
struct PrepMaps<'a> {
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
struct ActorsRecord {
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
struct DelegationRecord {
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

/// A `when` condition awaiting type checking once the class universe is
/// known. Lowering only syntax-checks conditions (so parse errors keep their
/// source order); the type check runs after every class is lowered.
struct PendingCondition {
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

/// Slices a `when (...)` span into a deferred type-check entry.
fn pending_condition(
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

/// Type-checks every deferred `when` condition against its resolved
/// capability class per docs/EXPRESSIONS.md: bare names are that class's
/// features, options propagate per R3, literals adapt per L1, and so on.
/// Parse errors were already reported during lowering. The condition must
/// type as `boolean`. Diagnostics are tagged with the host file.
fn check_pending_conditions(
    pending: &[PendingCondition],
    context: &TypeContext,
) -> Vec<(String, Diagnostic)> {
    let mut diags = Vec::new();
    for condition in pending {
        let Some((package, class)) = &condition.class else {
            continue;
        };
        let parsed = rex_expr::parse(&condition.text);
        if !parsed.errors.is_empty() || parsed.ast.is_none() {
            continue;
        }
        let checker = TypeChecker::new(context.clone()).with_self(package, class);
        match checker.type_of(parsed.ast.as_ref().expect("ast checked above")) {
            Ok(ty) if ty == Ty::boolean() => {}
            Ok(ty) => diags.push((
                condition.file.clone(),
                Diagnostic::error(
                    format!("invalid `when` condition: condition must be boolean, found {ty}"),
                    Some((condition.inner_start..condition.inner_end).into()),
                ),
            )),
            Err(errors) => {
                for error in errors {
                    let start = (condition.base + error.span.start)
                        .clamp(condition.inner_start, condition.inner_end);
                    let end = (condition.base + error.span.end).clamp(start, condition.inner_end);
                    diags.push((
                        condition.file.clone(),
                        Diagnostic::error(
                            format!("invalid `when` condition: {}", error.message),
                            Some((start..end).into()),
                        ),
                    ));
                }
            }
        }
    }
    diags
}

/// One `.mox` source of a multi-file compilation: its path, source, parsed
/// AST (when parseable), and the file's own parse diagnostics.
pub(crate) struct MultiFile<'a> {
    /// The file's path (tags diagnostics; locates `vocab/` snapshots).
    pub path: &'a str,
    /// The file's full source text.
    pub source: &'a str,
    /// The parsed AST, `None` when nothing could be produced.
    pub ast: Option<&'a mox::Model>,
    /// The file's own parse diagnostics.
    pub parse_diagnostics: &'a [Diagnostic],
}

/// Compiles several `.mox` files — one package per file — into one
/// multi-package model.
///
/// Every file lowers against the union namespace of all packages
/// ([`Scope::Domains`]), so declarations may reference types across
/// packages (`refers`, `extends`, attribute types; a bare name must be
/// unique across packages, a qualified `<package>.<Name>` must match one).
/// Cross-file rules run over the assembled universe: inheritance cycles are
/// detected across packages, while `contains`/`container` targets and
/// opposites are restricted to the declaring package. Inline `actors` blocks
/// from every file join one union policy set. Returns the model (or `None`
/// when any error exists) plus diagnostics tagged with their file's path,
/// each file's diagnostics contiguous and in input order.
pub(crate) fn compile_multi(
    files: &[MultiFile<'_>],
    schema_imports: Option<&crate::SchemaImports>,
) -> (Option<ir::Model>, Vec<(String, Diagnostic)>) {
    // Per-file diagnostic buckets keep each file's diagnostics contiguous;
    // the cross-file passes append their (already tagged) diagnostics after.
    let mut buckets: Vec<Vec<Diagnostic>> = files.iter().map(|_| Vec::new()).collect();
    for (index, file) in files.iter().enumerate() {
        buckets[index].extend(file.parse_diagnostics.iter().cloned());
    }

    // Package declarations and per-file namespaces. Every file joins the
    // union namespace — including files with a duplicate package name (the
    // duplicate errors on the later file, but its unique declarations still
    // resolve for the remaining diagnostics).
    let mut packages: Vec<DomainPackage> = Vec::new();
    let mut file_package: Vec<Option<usize>> = vec![None; files.len()];
    let mut winning_enums: Vec<(usize, &mox::EnumDecl)> = Vec::new();
    let mut file_imports: Vec<Vec<SchemaImport>> = files.iter().map(|_| Vec::new()).collect();
    for (index, file) in files.iter().enumerate() {
        let Some(ast) = file.ast else {
            continue; // parse diagnostics already reported; nothing to lower
        };
        let Some(package_decl) = &ast.package else {
            buckets[index].push(
                Diagnostic::error("missing `package` declaration", None).with_help(
                    "add a package clause at the top of the file, e.g. `package com.example.model`",
                ),
            );
            continue;
        };
        let name = package_decl.name.full_name();
        if packages.iter().any(|package| package.name == name) {
            buckets[index].push(
                Diagnostic::error(
                    format!("duplicate package '{name}'"),
                    Some(package_decl.name.span),
                )
                    .with_help(
                        "each file must declare a distinct package; rename one package or merge the files",
                    ),
            );
        }
        let mut kinds: HashMap<String, TopKind> = HashMap::new();
        for decl in &ast.declarations {
            let (kind, decl_name) = match decl {
                mox::Decl::Class(decl) => (TopKind::Class, &decl.name),
                mox::Decl::Interface(decl) => (TopKind::Interface, &decl.name),
                mox::Decl::Enum(decl) => (TopKind::Enum, &decl.name),
                mox::Decl::Datatype(decl) => (TopKind::Datatype, &decl.name),
                mox::Decl::Vocabulary(decl) => (TopKind::Vocabulary, &decl.name),
                mox::Decl::Actors(decl) => (TopKind::Actors, &decl.name),
                mox::Decl::Annotation(_) | mox::Decl::ImportSchema(_) => continue,
            };
            if kinds.contains_key(&decl_name.text) {
                buckets[index].push(
                    Diagnostic::error(
                        format!("duplicate declaration of '{}'", decl_name.text),
                        Some(decl_name.span),
                    )
                    .with_help(format!(
                        "'{0}' is already declared in this package",
                        decl_name.text
                    )),
                );
            } else {
                kinds.insert(decl_name.text.clone(), kind);
                if let mox::Decl::Enum(enum_decl) = decl {
                    winning_enums.push((index, enum_decl));
                }
            }
        }
        // Validate and register the file's `import schema` declarations:
        // each joins the namespace as a nominal class (alias or file stem).
        let imports =
            collect_schema_imports(file.path, ast, schema_imports, &kinds, &mut buckets[index]);
        for import in &imports {
            kinds.insert(import.name.clone(), TopKind::Class);
        }
        file_imports[index] = imports;
        file_package[index] = Some(packages.len());
        packages.push(DomainPackage { name, kinds });
    }

    // Qualified constraint prep maps across all files, so enum closure
    // checks and vocabulary key-facet family checks work for cross-package
    // attribute types.
    let mut enum_decls: HashMap<(&str, &str), &mox::EnumDecl> = HashMap::new();
    for (index, enum_decl) in &winning_enums {
        let package = packages[file_package[*index].expect("enum files declare a package")]
            .name
            .as_str();
        enum_decls.insert((package, enum_decl.name.text.as_str()), enum_decl);
    }
    let mut vocab_key_facet_types: HashMap<(&str, &str), ir::PrimitiveType> = HashMap::new();
    for (index, file) in files.iter().enumerate() {
        let Some(package_index) = file_package[index] else {
            continue;
        };
        let Some(ast) = file.ast else {
            continue;
        };
        let package = packages[package_index].name.as_str();
        for decl in &ast.declarations {
            if let mox::Decl::Vocabulary(decl) = decl {
                if let Some(key) = &decl.key {
                    if let Some(facet) =
                        decl.facets.iter().find(|facet| facet.name.text == key.text)
                    {
                        if let Some(primitive) = primitive_facet_type(&facet.type_ref) {
                            vocab_key_facet_types
                                .insert((package, decl.name.text.as_str()), primitive);
                        }
                    }
                }
            }
        }
    }

    // Vocabularies lower per file (snapshots resolve against the file's own
    // directory) and must be known before any class lowers.
    let mut vocab_defs: Vec<Vec<ir::VocabularyDef>> = files.iter().map(|_| Vec::new()).collect();
    for (index, file) in files.iter().enumerate() {
        if file_package[index].is_none() {
            continue;
        }
        let base_dir = Path::new(file.path)
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."));
        let Some(ast) = file.ast else {
            continue;
        };
        for decl in &ast.declarations {
            if let mox::Decl::Vocabulary(decl) = decl {
                if let Some(def) = lower_vocabulary(decl, &base_dir, &mut buckets[index]) {
                    vocab_defs[index].push(def);
                }
            }
        }
    }
    let mut vocab_keys: HashMap<(&str, &str), HashSet<&str>> = HashMap::new();
    for (index, defs) in vocab_defs.iter().enumerate() {
        let Some(package_index) = file_package[index] else {
            continue;
        };
        let package = packages[package_index].name.as_str();
        for def in defs {
            vocab_keys.insert(
                (package, def.name.as_str()),
                def.entries.iter().map(|entry| entry.key.as_str()).collect(),
            );
        }
    }
    let prep = PrepMaps {
        enum_decls,
        vocab_keys,
        vocab_key_facet_types,
    };

    // Lower every file against the union namespace.
    let scope = Scope::Domains {
        packages: &packages,
    };
    let mut out_packages: Vec<ir::Package> = Vec::new();
    let mut classes: Vec<ClassRecord> = Vec::new();
    let mut class_files: Vec<usize> = Vec::new();
    let mut actors: Vec<ActorsRecord> = Vec::new();
    let mut actor_files: Vec<usize> = Vec::new();
    let mut pending: Vec<PendingCondition> = Vec::new();
    for (index, file) in files.iter().enumerate() {
        let Some(package_index) = file_package[index] else {
            continue;
        };
        let Some(ast) = file.ast else {
            continue;
        };
        let mut out = ir::Package::new(packages[package_index].name.clone());
        out.description = ast.package.as_ref().and_then(|decl| decl.doc.clone());
        for decl in &ast.declarations {
            match decl {
                mox::Decl::Vocabulary(_) => {}
                mox::Decl::Actors(decl) => {
                    let (record, conditions) =
                        lower_actors(decl, file.source, file.path, &scope, &mut buckets[index]);
                    actors.push(record);
                    actor_files.push(index);
                    pending.extend(conditions);
                }
                mox::Decl::Annotation(annotation) => out.annotations.push(ir::Annotation {
                    source: annotation.value.clone(),
                    details: Default::default(),
                }),
                mox::Decl::Enum(decl) => out.enums.push(lower_enum(decl, &mut buckets[index])),
                mox::Decl::Datatype(decl) => {
                    out.datatypes
                        .push(lower_datatype(decl, file.source, &mut buckets[index]))
                }
                mox::Decl::Interface(decl) => out.interfaces.push(lower_interface(decl)),
                mox::Decl::Class(decl) => {
                    classes.push(lower_class(
                        decl,
                        file.source,
                        file.path,
                        &packages[package_index].name,
                        &scope,
                        &prep,
                        &mut buckets[index],
                    ));
                    class_files.push(index);
                }
                // Imports lowered as nominal classes after the declared ones.
                mox::Decl::ImportSchema(_) => {}
            }
        }
        out_packages.push(out);
    }

    // Cross-file class-graph passes over the assembled class universe.
    let mut tagged: Vec<(String, Diagnostic)> = Vec::new();
    detect_inheritance_cycles(&classes, &mut tagged);
    validate_opposites(&classes, &mut tagged);

    // Inline actors blocks from every file join one union policy set: cycle
    // detection runs per block first; separation of duty, self-narrowing,
    // and delegations span the union (same-named actors pool their permits
    // across files).
    let cyclic = detect_actor_cycles(&actors, &mut tagged);
    if !cyclic {
        validate_actor_never_both_union(&actors, &mut tagged);
        validate_actor_self_narrowing_union(&actors, &mut tagged);
        validate_actor_delegations_union(&actors, &mut tagged);
    }

    let mut model = ir::Model::new();
    model.packages = out_packages;

    // Classes join their package after the cross-file passes; operations
    // move into the def first, as in the single-file path. The out packages
    // were pushed in file order, so a file's package index is its out index.
    for (class, file_index) in classes.into_iter().zip(class_files) {
        let mut class = class;
        class.def.operations = std::mem::take(&mut class.operations);
        let package_index = file_package[file_index].expect("class files declare a package");
        model.packages[package_index].classes.push(class.def);
    }

    // Imported schemas join their package as nominal classes, after the
    // declared ones, in import-declaration order.
    for (index, imports) in file_imports.iter().enumerate() {
        if imports.is_empty() {
            continue;
        }
        let package_index = file_package[index].expect("import files declare a package");
        for import in imports {
            model.packages[package_index]
                .classes
                .push(import_class_def(import));
        }
    }

    // Vocabularies join their package now that class lowering — the last
    // prep-map consumer borrowing the definitions — is done.
    for (index, defs) in vocab_defs.into_iter().enumerate() {
        if let Some(package_index) = file_package[index] {
            model.packages[package_index].vocabularies = defs;
        }
    }

    // Inline actors blocks join their package after the union policy
    // passes, mirroring the single-file join. Cross-file blocks with the
    // same name stay separate blocks: the union semantics pool same-named
    // actors at validation, and each package carries what it declares.
    for (record, file_index) in actors.into_iter().zip(actor_files) {
        let package_index = file_package[file_index].expect("actors files declare a package");
        model.packages[package_index].actors.push(record.def);
    }

    // Feature ids are assigned by `ClassDef::new` in declaration order;
    // keep them in sync with the final feature lists.
    for package in &mut model.packages {
        for class in &mut package.classes {
            class.assign_feature_ids();
        }
    }

    // Every `when` condition is now fully type-checked against its
    // capability's class in the complete multi-package class universe.
    let context = TypeContext::from_model(&model);
    tagged.extend(check_pending_conditions(&pending, &context));

    let mut diags: Vec<(String, Diagnostic)> = Vec::new();
    for (index, file) in files.iter().enumerate() {
        diags.extend(buckets[index].drain(..).map(|d| (file.path.to_string(), d)));
    }
    diags.extend(tagged);

    let blocked = diags.iter().any(|(_, diagnostic)| diagnostic.is_error());
    let model = (!blocked).then_some(model);
    (model, diags)
}

/// The per-file outcome of [`compile_union_per_file`].
pub(crate) struct PerFileCompilation {
    /// The file's own package in a single-package model, or `None` when any
    /// error-severity diagnostic was produced for this file.
    pub model: Option<ir::Model>,
    /// All diagnostics for this file, untagged (the caller knows the file).
    pub diagnostics: Vec<Diagnostic>,
}

/// Compiles several `.mox` domain files against one shared union namespace,
/// keeping the results per file: every file lowers against the union of all
/// packages (so declarations may reference types across files), but a file
/// with errors yields `(None, its diagnostics)` while the remaining files
/// still compile successfully. This is the shape the `.actor` import path
/// needs — an imported domain that fails to compile must not stop the other
/// imports from resolving for capability bindings and `when` conditions
/// (see [`compile_actor_file`], which re-lowers every error-free file's
/// inline actors blocks against the same union and re-checks conditions
/// against the combined context; identical diagnostics are deduplicated
/// there).
///
/// Unlike [`compile_multi`] there is deliberately no all-or-nothing model:
/// callers assemble what they need from the per-file results.
pub(crate) fn compile_union_per_file(
    files: &[MultiFile<'_>],
    schema_imports: Option<&crate::SchemaImports>,
) -> Vec<PerFileCompilation> {
    // Per-file diagnostic buckets keep each file's diagnostics in its own
    // result; cross-file passes route their (already tagged) diagnostics to
    // the declaring file's bucket.
    let mut buckets: Vec<Vec<Diagnostic>> = files.iter().map(|_| Vec::new()).collect();
    for (index, file) in files.iter().enumerate() {
        buckets[index].extend(file.parse_diagnostics.iter().cloned());
    }

    // Package declarations and per-file namespaces. Every file joins the
    // union namespace — including files with a duplicate package name (the
    // duplicate errors on the later file, but its unique declarations still
    // resolve for the remaining diagnostics).
    let mut packages: Vec<DomainPackage> = Vec::new();
    let mut file_package: Vec<Option<usize>> = vec![None; files.len()];
    let mut winning_enums: Vec<(usize, &mox::EnumDecl)> = Vec::new();
    let mut file_imports: Vec<Vec<SchemaImport>> = files.iter().map(|_| Vec::new()).collect();
    for (index, file) in files.iter().enumerate() {
        let Some(ast) = file.ast else {
            continue; // parse diagnostics already reported; nothing to lower
        };
        let Some(package_decl) = &ast.package else {
            buckets[index].push(
                Diagnostic::error("missing `package` declaration", None).with_help(
                    "add a package clause at the top of the file, e.g. `package com.example.model`",
                ),
            );
            continue;
        };
        let name = package_decl.name.full_name();
        if packages.iter().any(|package| package.name == name) {
            buckets[index].push(
                Diagnostic::error(
                    format!("duplicate package '{name}'"),
                    Some(package_decl.name.span),
                )
                    .with_help(
                        "each file must declare a distinct package; rename one package or merge the files",
                    ),
            );
        }
        let mut kinds: HashMap<String, TopKind> = HashMap::new();
        for decl in &ast.declarations {
            let (kind, decl_name) = match decl {
                mox::Decl::Class(decl) => (TopKind::Class, &decl.name),
                mox::Decl::Interface(decl) => (TopKind::Interface, &decl.name),
                mox::Decl::Enum(decl) => (TopKind::Enum, &decl.name),
                mox::Decl::Datatype(decl) => (TopKind::Datatype, &decl.name),
                mox::Decl::Vocabulary(decl) => (TopKind::Vocabulary, &decl.name),
                mox::Decl::Actors(decl) => (TopKind::Actors, &decl.name),
                mox::Decl::Annotation(_) | mox::Decl::ImportSchema(_) => continue,
            };
            if kinds.contains_key(&decl_name.text) {
                buckets[index].push(
                    Diagnostic::error(
                        format!("duplicate declaration of '{}'", decl_name.text),
                        Some(decl_name.span),
                    )
                    .with_help(format!(
                        "'{0}' is already declared in this package",
                        decl_name.text
                    )),
                );
            } else {
                kinds.insert(decl_name.text.clone(), kind);
                if let mox::Decl::Enum(enum_decl) = decl {
                    winning_enums.push((index, enum_decl));
                }
            }
        }
        // Validate and register the file's `import schema` declarations:
        // each joins the namespace as a nominal class (alias or file stem).
        let imports =
            collect_schema_imports(file.path, ast, schema_imports, &kinds, &mut buckets[index]);
        for import in &imports {
            kinds.insert(import.name.clone(), TopKind::Class);
        }
        file_imports[index] = imports;
        file_package[index] = Some(packages.len());
        packages.push(DomainPackage { name, kinds });
    }

    // Qualified constraint prep maps across all files, so enum closure
    // checks and vocabulary key-facet family checks work for cross-package
    // attribute types.
    let mut enum_decls: HashMap<(&str, &str), &mox::EnumDecl> = HashMap::new();
    for (index, enum_decl) in &winning_enums {
        let package = packages[file_package[*index].expect("enum files declare a package")]
            .name
            .as_str();
        enum_decls.insert((package, enum_decl.name.text.as_str()), enum_decl);
    }
    let mut vocab_key_facet_types: HashMap<(&str, &str), ir::PrimitiveType> = HashMap::new();
    for (index, file) in files.iter().enumerate() {
        let Some(package_index) = file_package[index] else {
            continue;
        };
        let Some(ast) = file.ast else {
            continue;
        };
        let package = packages[package_index].name.as_str();
        for decl in &ast.declarations {
            if let mox::Decl::Vocabulary(decl) = decl {
                if let Some(key) = &decl.key {
                    if let Some(facet) =
                        decl.facets.iter().find(|facet| facet.name.text == key.text)
                    {
                        if let Some(primitive) = primitive_facet_type(&facet.type_ref) {
                            vocab_key_facet_types
                                .insert((package, decl.name.text.as_str()), primitive);
                        }
                    }
                }
            }
        }
    }

    // Vocabularies lower per file (snapshots resolve against the file's own
    // directory) and must be known before any class lowers.
    let mut vocab_defs: Vec<Vec<ir::VocabularyDef>> = files.iter().map(|_| Vec::new()).collect();
    for (index, file) in files.iter().enumerate() {
        if file_package[index].is_none() {
            continue;
        }
        let base_dir = Path::new(file.path)
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."));
        let Some(ast) = file.ast else {
            continue;
        };
        for decl in &ast.declarations {
            if let mox::Decl::Vocabulary(decl) = decl {
                if let Some(def) = lower_vocabulary(decl, &base_dir, &mut buckets[index]) {
                    vocab_defs[index].push(def);
                }
            }
        }
    }
    let mut vocab_keys: HashMap<(&str, &str), HashSet<&str>> = HashMap::new();
    for (index, defs) in vocab_defs.iter().enumerate() {
        let Some(package_index) = file_package[index] else {
            continue;
        };
        let package = packages[package_index].name.as_str();
        for def in defs {
            vocab_keys.insert(
                (package, def.name.as_str()),
                def.entries.iter().map(|entry| entry.key.as_str()).collect(),
            );
        }
    }
    let prep = PrepMaps {
        enum_decls,
        vocab_keys,
        vocab_key_facet_types,
    };

    // Lower every file against the union namespace, keeping the lowered
    // classes, actors and pending conditions per file.
    let scope = Scope::Domains {
        packages: &packages,
    };
    let mut out_packages: Vec<ir::Package> = Vec::new();
    let mut classes: Vec<ClassRecord> = Vec::new();
    let mut actors: Vec<ActorsRecord> = Vec::new();
    let mut actor_files: Vec<usize> = Vec::new();
    let mut pending: Vec<(usize, PendingCondition)> = Vec::new();
    for (index, file) in files.iter().enumerate() {
        let Some(package_index) = file_package[index] else {
            continue;
        };
        let Some(ast) = file.ast else {
            continue;
        };
        let mut out = ir::Package::new(packages[package_index].name.clone());
        out.description = ast.package.as_ref().and_then(|decl| decl.doc.clone());
        for decl in &ast.declarations {
            match decl {
                mox::Decl::Vocabulary(_) => {}
                mox::Decl::Actors(decl) => {
                    let (record, conditions) =
                        lower_actors(decl, file.source, file.path, &scope, &mut buckets[index]);
                    actors.push(record);
                    actor_files.push(index);
                    pending.extend(conditions.into_iter().map(|condition| (index, condition)));
                }
                mox::Decl::Annotation(annotation) => out.annotations.push(ir::Annotation {
                    source: annotation.value.clone(),
                    details: Default::default(),
                }),
                mox::Decl::Enum(decl) => out.enums.push(lower_enum(decl, &mut buckets[index])),
                mox::Decl::Datatype(decl) => {
                    out.datatypes
                        .push(lower_datatype(decl, file.source, &mut buckets[index]))
                }
                mox::Decl::Interface(decl) => out.interfaces.push(lower_interface(decl)),
                mox::Decl::Class(decl) => {
                    classes.push(lower_class(
                        decl,
                        file.source,
                        file.path,
                        &packages[package_index].name,
                        &scope,
                        &prep,
                        &mut buckets[index],
                    ));
                }
                // Imports lowered as nominal classes after the declared ones.
                mox::Decl::ImportSchema(_) => {}
            }
        }
        out_packages.push(out);
    }

    // Cross-file class-graph passes over the assembled class universe. The
    // passes tag diagnostics with the declaring file, so each lands in that
    // file's bucket: a cycle or invalid opposite blocks only its own file's
    // model, and the error-free files keep resolving for the actor path.
    let mut tagged: Vec<(String, Diagnostic)> = Vec::new();
    detect_inheritance_cycles(&classes, &mut tagged);
    validate_opposites(&classes, &mut tagged);

    // Inline actors blocks from every file join one union policy set: cycle
    // detection runs per block first; separation of duty, self-narrowing,
    // and delegations span the union (same-named actors pool their permits
    // across files). `compile_actor_file` re-derives these for error-free
    // files and deduplicates identical diagnostics, so files whose classes
    // failed still get their actor-block diagnostics here.
    let cyclic = detect_actor_cycles(&actors, &mut tagged);
    if !cyclic {
        validate_actor_never_both_union(&actors, &mut tagged);
        validate_actor_self_narrowing_union(&actors, &mut tagged);
        validate_actor_delegations_union(&actors, &mut tagged);
    }
    let mut file_by_path: HashMap<&str, usize> = HashMap::new();
    for (index, file) in files.iter().enumerate() {
        file_by_path.insert(file.path, index);
    }
    for (file, diagnostic) in tagged {
        let index = file_by_path.get(file.as_str()).copied().unwrap_or(0);
        buckets[index].push(diagnostic);
    }

    // Classes join their package after the cross-file passes; operations
    // move into the def first, as in the single-file path.
    for class in classes {
        let mut class = class;
        class.def.operations = std::mem::take(&mut class.operations);
        let class_file = file_by_path.get(class.file.as_str()).copied();
        let package_index = file_package[class_file.expect("class records carry a known file")]
            .expect("class files declare a package");
        model_package(&mut out_packages, packages[package_index].name.as_str())
            .classes
            .push(class.def);
    }

    // Imported schemas join their package as nominal classes, after the
    // declared ones, in import-declaration order.
    for (index, imports) in file_imports.iter().enumerate() {
        if imports.is_empty() {
            continue;
        }
        let package_index = file_package[index].expect("import files declare a package");
        let package_name = packages[package_index].name.as_str();
        for import in imports {
            model_package(&mut out_packages, package_name)
                .classes
                .push(import_class_def(import));
        }
    }

    // Vocabularies join their package now that class lowering — the last
    // prep-map consumer borrowing the definitions — is done.
    for (index, defs) in vocab_defs.into_iter().enumerate() {
        if let Some(package_index) = file_package[index] {
            let package_name = packages[package_index].name.as_str();
            model_package(&mut out_packages, package_name).vocabularies = defs;
        }
    }

    // Inline actors blocks join their package after the union policy
    // passes, mirroring the single-file join. Cross-file blocks with the
    // same name stay separate blocks: the union semantics pool same-named
    // actors at validation, and each package carries what it declares.
    for (record, file_index) in actors.into_iter().zip(actor_files) {
        let package_index = file_package[file_index].expect("actors files declare a package");
        let package_name = packages[package_index].name.as_str();
        model_package(&mut out_packages, package_name)
            .actors
            .push(record.def);
    }

    // Feature ids are assigned by `ClassDef::new` in declaration order;
    // keep them in sync with the final feature lists.
    for package in &mut out_packages {
        for class in &mut package.classes {
            class.assign_feature_ids();
        }
    }

    // Every `when` condition is now fully type-checked against its
    // capability's class in the complete multi-package class universe.
    let mut context_model = ir::Model::new();
    context_model.packages = out_packages.clone();
    let context = TypeContext::from_model(&context_model);
    let pending: Vec<PendingCondition> = pending
        .into_iter()
        .map(|(_, condition)| condition)
        .collect();
    let conditions = check_pending_conditions(&pending, &context);
    for (file, diagnostic) in conditions {
        let index = file_by_path.get(file.as_str()).copied().unwrap_or(0);
        buckets[index].push(diagnostic);
    }

    // Assemble one single-package model per file, dropped when that file
    // produced any error.
    files
        .iter()
        .enumerate()
        .map(|(index, _file)| {
            let blocked = buckets[index].iter().any(Diagnostic::is_error);
            let model = (!blocked).then(|| {
                let package_index = file_package[index].expect("lowered files declare a package");
                let package_name = packages[package_index].name.clone();
                let package = out_packages
                    .iter()
                    .find(|package| package.name == package_name)
                    .expect("every indexed package was lowered")
                    .clone();
                let mut model = ir::Model::new();
                model.packages.push(package);
                model
            });
            PerFileCompilation {
                model,
                diagnostics: std::mem::take(&mut buckets[index]),
            }
        })
        .collect()
}

/// Finds a lowered package by name in `packages` (the union may hold
/// duplicate-named packages when a duplicate-package error fired; the first
/// match wins, mirroring [`compile_multi`]).
fn model_package<'a>(packages: &'a mut [ir::Package], name: &str) -> &'a mut ir::Package {
    packages
        .iter_mut()
        .find(|package| package.name == name)
        .expect("every indexed package was lowered")
}

/// Resolves and validates a parsed model and lowers it into the Core IR.
///
/// Returns the IR (with `Model::rex_version` and `formatVersion` set) or
/// `None` when any error-severity diagnostic was produced. `path` locates the
/// model on disk: vocabulary snapshots are read from a `vocab/` directory
/// next to the source, and pins from `model.lock` next to that.
pub(crate) fn compile(
    path: &str,
    source: &str,
    model: &mox::Model,
    schema_imports: Option<&crate::SchemaImports>,
) -> (Option<ir::Model>, Vec<Diagnostic>) {
    let mut diags = Vec::new();

    let Some(package_decl) = &model.package else {
        diags.push(
            Diagnostic::error("missing `package` declaration", None).with_help(
                "add a package clause at the top of the file, e.g. `package com.example.model`",
            ),
        );
        return (None, diags);
    };
    let package = package_decl.name.full_name();
    let base_dir = Path::new(path)
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));

    // Index the package namespace and report duplicate declarations.
    let mut kinds: HashMap<String, TopKind> = HashMap::new();
    let mut enum_decls: HashMap<(&str, &str), &mox::EnumDecl> = HashMap::new();
    for decl in &model.declarations {
        let (kind, name) = match decl {
            mox::Decl::Class(decl) => (TopKind::Class, &decl.name),
            mox::Decl::Interface(decl) => (TopKind::Interface, &decl.name),
            mox::Decl::Enum(decl) => (TopKind::Enum, &decl.name),
            mox::Decl::Datatype(decl) => (TopKind::Datatype, &decl.name),
            mox::Decl::Vocabulary(decl) => (TopKind::Vocabulary, &decl.name),
            mox::Decl::Actors(decl) => (TopKind::Actors, &decl.name),
            // Import-schema names join the namespace below, collision-checked.
            mox::Decl::Annotation(_) | mox::Decl::ImportSchema(_) => continue,
        };
        if kinds.contains_key(name.text.as_str()) {
            diags.push(
                Diagnostic::error(
                    format!("duplicate declaration of '{}'", name.text),
                    Some(name.span),
                )
                .with_help(format!(
                    "'{0}' is already declared in this package",
                    name.text
                )),
            );
        } else {
            kinds.insert(name.text.clone(), kind);
            if let mox::Decl::Enum(enum_decl) = decl {
                enum_decls.insert((package.as_str(), &enum_decl.name.text), enum_decl);
            }
        }
    }

    // Validate and register the file's `import schema` declarations: each
    // joins the namespace as a nominal class (alias or file stem).
    let imports = collect_schema_imports(path, model, schema_imports, &kinds, &mut diags);
    for import in &imports {
        kinds.insert(import.name.clone(), TopKind::Class);
    }

    // Lower vocabulary declarations first (in source order): their entry
    // keys are needed while validating class attribute defaults below.
    let mut vocabulary_defs = Vec::new();
    for decl in &model.declarations {
        if let mox::Decl::Vocabulary(decl) = decl {
            if let Some(def) = lower_vocabulary(decl, &base_dir, &mut diags) {
                vocabulary_defs.push(def);
            }
        }
    }
    let vocab_keys: HashMap<(&str, &str), HashSet<&str>> = vocabulary_defs
        .iter()
        .map(|def| {
            (
                (package.as_str(), def.name.as_str()),
                def.entries.iter().map(|entry| entry.key.as_str()).collect(),
            )
        })
        .collect();

    // The declared primitive type of each vocabulary's key facet, which
    // classifies the constraint families admitted on vocabulary-typed
    // attributes (see `lower_constraints`).
    let vocab_key_facet_types: HashMap<(&str, &str), ir::PrimitiveType> = model
        .declarations
        .iter()
        .filter_map(|decl| match decl {
            mox::Decl::Vocabulary(decl) => {
                let key = decl.key.as_ref()?;
                let facet = decl
                    .facets
                    .iter()
                    .find(|facet| facet.name.text == key.text)?;
                Some((
                    (package.as_str(), decl.name.text.as_str()),
                    primitive_facet_type(&facet.type_ref)?,
                ))
            }
            _ => None,
        })
        .collect();
    let prep = PrepMaps {
        enum_decls,
        vocab_keys,
        vocab_key_facet_types,
    };

    // Lower declarations in source order.
    let mut out = ir::Package::new(package.clone());
    out.description = package_decl.doc.clone();
    let scope = Scope::Single {
        package: &package,
        kinds: &kinds,
    };
    let mut classes: Vec<ClassRecord> = Vec::new();
    let mut actors: Vec<ActorsRecord> = Vec::new();
    let mut pending: Vec<PendingCondition> = Vec::new();
    for decl in &model.declarations {
        match decl {
            mox::Decl::Vocabulary(_) => {}
            mox::Decl::Actors(decl) => {
                let (record, conditions) = lower_actors(decl, source, path, &scope, &mut diags);
                actors.push(record);
                pending.extend(conditions);
            }
            mox::Decl::Annotation(annotation) => out.annotations.push(ir::Annotation {
                source: annotation.value.clone(),
                details: Default::default(),
            }),
            mox::Decl::Enum(decl) => out.enums.push(lower_enum(decl, &mut diags)),
            mox::Decl::Datatype(decl) => {
                out.datatypes.push(lower_datatype(decl, source, &mut diags))
            }
            mox::Decl::Interface(decl) => out.interfaces.push(lower_interface(decl)),
            mox::Decl::Class(decl) => classes.push(lower_class(
                decl, source, path, &package, &scope, &prep, &mut diags,
            )),
            // Imports lowered as nominal classes after the declared ones.
            mox::Decl::ImportSchema(_) => {}
        }
    }
    out.vocabularies = vocabulary_defs;

    // The class-graph passes produce tagged diagnostics (the multi-file path
    // needs the file); here every tag is this same file, so they are
    // stripped to keep the single-file diagnostic shape unchanged.
    let mut class_diags: Vec<(String, Diagnostic)> = Vec::new();
    detect_inheritance_cycles(&classes, &mut class_diags);
    validate_opposites(&classes, &mut class_diags);
    diags.extend(class_diags.into_iter().map(|(_, diagnostic)| diagnostic));

    // Actors validation: name-resolution errors were emitted during
    // lowering; cycle detection runs before the ancestor walks, which are
    // skipped entirely for cyclic blocks (their results would be
    // meaningless).
    let mut tagged = Vec::new();
    let cyclic = detect_actor_cycles(&actors, &mut tagged);
    if !cyclic {
        validate_actor_never_both(&actors, &mut tagged);
        validate_actor_self_narrowing(&actors, &mut tagged);
        validate_actor_delegations(&actors, &mut tagged);
    }
    diags.extend(tagged.into_iter().map(|(_, diagnostic)| diagnostic));

    for mut class in classes {
        class.def.operations = std::mem::take(&mut class.operations);
        out.classes.push(class.def);
    }
    // Imported schemas join their package as nominal classes, after the
    // declared ones, in import-declaration order.
    for import in &imports {
        out.classes.push(import_class_def(import));
    }
    for record in actors {
        out.actors.push(record.def);
    }

    let mut model = ir::Model::new();
    model.packages.push(out);
    // Feature ids are assigned by `ClassDef::new` in declaration order;
    // keep them in sync with the final feature lists.
    for class in &mut model.packages[0].classes {
        class.assign_feature_ids();
    }

    // Every `when` condition is now fully type-checked against its
    // capability's class (docs/EXPRESSIONS.md), which needs the complete
    // class universe assembled above.
    let context = TypeContext::from_model(&model);
    diags.extend(
        check_pending_conditions(&pending, &context)
            .into_iter()
            .map(|(_, diagnostic)| diagnostic),
    );

    let blocked = diags.iter().any(Diagnostic::is_error);
    let model = (!blocked).then_some(model);
    (model, diags)
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

/// `Some(class name)` when the resolution classified the type as a class.
fn class_of(resolution: Option<&Resolution>) -> Option<String> {
    class_location(resolution).map(|(_, name)| name)
}

/// `Some((package, name))` of the target when the resolution classified the
/// type as a class.
fn class_location(resolution: Option<&Resolution>) -> Option<(String, String)> {
    match resolution {
        Some(resolution) if resolution.kind == Resolved::Class => {
            Some((resolution.package.clone(), resolution.name.clone()))
        }
        _ => None,
    }
}

/// Applies the AST `id`/`readonly` modifiers to a lowered IR feature.
/// Repeats in the source are idempotent, so the builder calls run at most once.
fn apply_modifiers(feature: ir::Feature, modifiers: &mox::Modifiers) -> ir::Feature {
    let feature = if modifiers.is_id() {
        feature.identifier()
    } else {
        feature
    };
    if modifiers.is_read_only() {
        feature.read_only()
    } else {
        feature
    }
}

/// The constraint space an attribute's resolved type admits.
#[derive(Clone, Copy)]
enum ConstraintSpace<'a> {
    /// Only the string family applies (`string` primitives, opaque
    /// datatypes, `String`-keyed vocabularies).
    String,
    /// Only the numeric family applies (numeric primitives, numeric-keyed
    /// vocabularies).
    Numeric,
    /// Neither family applies (`char`, `boolean`).
    Neither,
    /// Both families apply: string constraints bound literal names, numeric
    /// constraints bound literal values.
    Enum(&'a mox::EnumDecl),
}

impl ConstraintSpace<'_> {
    fn admits_string_family(self) -> bool {
        matches!(self, Self::String | Self::Enum(_))
    }

    fn admits_numeric_family(self) -> bool {
        matches!(self, Self::Numeric | Self::Enum(_))
    }
}

/// The diagnostic for a constraint keyword whose family the attribute's
/// type does not admit. The message names the deciding type knowledge so
/// the fix is apparent from the error alone.
fn family_mismatch(
    keyword: &str,
    family: &str,
    subject: &str,
    resolution: Option<&Resolution>,
    prep: &PrepMaps<'_>,
    span: Option<Span>,
) -> Diagnostic {
    let message = match resolution {
        Some(Resolution {
            kind: Resolved::Datatype,
            name,
            ..
        }) => format!(
            "constraint '{keyword}' requires a {family} attribute; datatype '{name}' has an opaque platform type"
        ),
        Some(Resolution {
            kind: Resolved::Vocabulary,
            name,
            package,
        }) => match prep
            .vocab_key_facet_types
            .get(&(package.as_str(), name.as_str()))
        {
            Some(primitive) => format!(
                "constraint '{keyword}' requires a {family} attribute; the key facet of vocabulary '{name}' is {primitive}"
            ),
            None => format!(
                "constraint '{keyword}' requires a {family} attribute; vocabulary '{name}' has no resolvable key facet"
            ),
        },
        _ => format!("constraint '{keyword}' requires a {family} attribute, which {subject} is not"),
    };
    Diagnostic::error(message, span)
}

/// Lowers an attribute's declared constraints into the IR, type-checking
/// them against the attribute's resolved type. For a many-valued attribute
/// the value bounds apply to the elements.
///
/// A constraint family is admitted by the most explicit type knowledge
/// available (violations are errors, never silently dropped):
/// - `pattern`, `minLength`, `maxLength` (the string family) require a
///   `string` attribute; `minimum`, `maximum` (the numeric family) require
///   a numeric attribute (`char` and `boolean` fall under neither rule);
/// - a datatype-typed attribute defaults to the string family — its platform
///   type is opaque — and rejects numeric bounds;
/// - a vocabulary-typed attribute follows its `key` facet's declared
///   primitive type (`String` admits the string family, a numeric key the
///   numeric family); constraints are rejected outright when the key facet
///   cannot be resolved;
/// - an enum-typed attribute carries a dual value space: all five valued
///   keywords apply, the string family bounding literal names and the
///   numeric family literal values, and both may coexist. A numeric or
///   length bound that admits zero literals is a compile error; `pattern`
///   is never statically validated;
/// - the value-less `unique` is family-agnostic: it is admitted on any
///   many-valued attribute regardless of element type and rejected on a
///   single-valued one (a set of one value is trivially unique);
/// - class and interface types take no constraints;
/// - length bounds must be non-negative and ordered; numeric bounds must be
///   ordered.
fn lower_constraints(
    constraints: &[mox::Constraint],
    many: bool,
    resolution: Option<&Resolution>,
    prep: &PrepMaps<'_>,
    feature_name: &str,
    diags: &mut Vec<Diagnostic>,
) -> ir::FeatureConstraints {
    let mut lowered = ir::FeatureConstraints::default();
    if constraints.is_empty() {
        return lowered;
    }
    let subject = format!("attribute '{feature_name}'");
    // The attribute's constraint space, or `None` when checks are skipped:
    // either resolution failed (the resolver already reported the error) or
    // a diagnostic was just raised here — both avoid cascading noise.
    let space = match resolution {
        None => None,
        Some(resolution) => match resolution.kind {
            Resolved::Primitive(ir::PrimitiveType::String) => Some(ConstraintSpace::String),
            Resolved::Primitive(primitive) if primitive.is_numeric() => {
                Some(ConstraintSpace::Numeric)
            }
            Resolved::Primitive(_) => Some(ConstraintSpace::Neither),
            Resolved::Datatype => Some(ConstraintSpace::String),
            Resolved::Enum => prep
                .enum_decls
                .get(&(resolution.package.as_str(), resolution.name.as_str()))
                .copied()
                .map(ConstraintSpace::Enum),
            Resolved::Vocabulary => {
                match prep
                    .vocab_key_facet_types
                    .get(&(resolution.package.as_str(), resolution.name.as_str()))
                {
                    Some(ir::PrimitiveType::String) => Some(ConstraintSpace::String),
                    Some(primitive) if primitive.is_numeric() => Some(ConstraintSpace::Numeric),
                    _ => {
                        diags.push(
                            Diagnostic::error(
                                format!(
                                    "constraints on {subject} cannot be checked: vocabulary '{}' has no resolvable key facet",
                                    resolution.name
                                ),
                                Some(constraints[0].name.span),
                            )
                            .with_help(
                                "declare the key facet (e.g. `facet String alpha3`) so its value space is known",
                            ),
                        );
                        None
                    }
                }
            }
            Resolved::Class | Resolved::Interface => {
                diags.push(Diagnostic::error(
                    format!(
                        "constraints are only allowed on primitive, datatype, vocabulary, and enum-typed attributes, but {subject} has type '{}'",
                        resolution.name
                    ),
                    Some(constraints[0].name.span),
                ));
                None
            }
        },
    };
    let Some(space) = space else {
        return lowered;
    };
    let mut minimum_span = None;
    let mut maximum_span = None;
    let mut min_length_span = None;
    let mut max_length_span = None;
    for constraint in constraints {
        let span = Some(constraint.name.span);
        match (constraint.name.text.as_str(), &constraint.value) {
            ("pattern", mox::ConstraintValue::Str { value, .. }) => {
                if space.admits_string_family() {
                    if lowered.pattern.is_none() {
                        lowered.pattern = Some(value.clone());
                    }
                } else {
                    diags.push(family_mismatch(
                        "pattern", "string", &subject, resolution, prep, span,
                    ));
                }
            }
            (keyword @ ("minLength" | "maxLength"), mox::ConstraintValue::Int { value, .. }) => {
                if !space.admits_string_family() {
                    diags.push(family_mismatch(
                        keyword, "string", &subject, resolution, prep, span,
                    ));
                } else if *value < 0 {
                    diags.push(Diagnostic::error(
                        format!("constraint '{keyword}' must be non-negative, found {value}"),
                        span,
                    ));
                } else if keyword == "minLength" {
                    lowered.min_length = Some(*value as u64);
                    min_length_span = span;
                } else {
                    lowered.max_length = Some(*value as u64);
                    max_length_span = span;
                }
            }
            (keyword @ ("minimum" | "maximum"), mox::ConstraintValue::Int { value, .. }) => {
                if !space.admits_numeric_family() {
                    diags.push(family_mismatch(
                        keyword, "numeric", &subject, resolution, prep, span,
                    ));
                } else if keyword == "minimum" {
                    lowered.minimum = Some(*value);
                    minimum_span = span;
                } else {
                    lowered.maximum = Some(*value);
                    maximum_span = span;
                }
            }
            ("unique", mox::ConstraintValue::Flag { .. }) => {
                // Family-agnostic: any many-valued attribute admits it.
                if many {
                    lowered.unique = true;
                } else {
                    diags.push(Diagnostic::error(
                        format!(
                            "constraint 'unique' requires a many-valued attribute, which {subject} is not"
                        ),
                        span,
                    ));
                }
            }
            ("unique", _) => {
                diags.push(Diagnostic::error(
                    "constraint 'unique' takes no value".to_string(),
                    span,
                ));
            }
            (keyword, mox::ConstraintValue::Flag { .. }) => {
                diags.push(Diagnostic::error(
                    format!("constraint '{keyword}' takes no value"),
                    span,
                ));
            }
            (keyword, mox::ConstraintValue::Str { .. } | mox::ConstraintValue::Int { .. }) => {
                diags.push(Diagnostic::error(
                    format!(
                        "constraint '{keyword}' takes a quoted string (`pattern`) or an integer (length and numeric bounds)"
                    ),
                    span,
                ));
            }
        }
    }
    if let (Some(minimum), Some(maximum)) = (lowered.minimum, lowered.maximum) {
        if minimum > maximum {
            diags.push(Diagnostic::error(
                format!(
                    "attribute '{feature_name}': minimum ({minimum}) must not exceed maximum ({maximum})"
                ),
                Some(constraints[0].name.span),
            ));
        }
    }
    if let (Some(min_length), Some(max_length)) = (lowered.min_length, lowered.max_length) {
        if min_length > max_length {
            diags.push(Diagnostic::error(
                format!(
                    "attribute '{feature_name}': minLength ({min_length}) must not exceed maxLength ({max_length})"
                ),
                Some(constraints[0].name.span),
            ));
        }
    }
    // Enums get closure checks: a bound admitting zero literals is surely a
    // typo. Inverted ranges were already reported above, so the closure
    // check stays silent for them — one clear error each.
    if let ConstraintSpace::Enum(enum_decl) = space {
        let enum_name = &enum_decl.name.text;
        if lowered.minimum.is_some() || lowered.maximum.is_some() {
            let ordered = match (lowered.minimum, lowered.maximum) {
                (Some(min), Some(max)) => min <= max,
                _ => true,
            };
            let admits = |value: i64| {
                lowered.minimum.is_none_or(|min| value >= min)
                    && lowered.maximum.is_none_or(|max| value <= max)
            };
            let any = enum_decl
                .literals
                .iter()
                .filter_map(|literal| literal.value)
                .any(admits);
            if ordered && !any {
                let (keyword, span, detail) = match (lowered.minimum, lowered.maximum) {
                    (Some(min), None) => (
                        "minimum",
                        minimum_span,
                        format!("no literal value is >= {min}"),
                    ),
                    (None, Some(max)) => (
                        "maximum",
                        maximum_span,
                        format!("no literal value is <= {max}"),
                    ),
                    (Some(min), Some(max)) => (
                        "minimum",
                        minimum_span,
                        format!("no literal value lies in [{min}, {max}]"),
                    ),
                    (None, None) => unreachable!("range is bounded"),
                };
                diags.push(Diagnostic::error(
                    format!("constraint '{keyword}' on attribute '{feature_name}' admits no literal of enum '{enum_name}': {detail}"),
                    span,
                ));
            }
        }
        if lowered.min_length.is_some() || lowered.max_length.is_some() {
            let ordered = match (lowered.min_length, lowered.max_length) {
                (Some(min), Some(max)) => min <= max,
                _ => true,
            };
            let admits = |length: usize| {
                lowered.min_length.is_none_or(|min| length >= min as usize)
                    && lowered.max_length.is_none_or(|max| length <= max as usize)
            };
            let any = enum_decl
                .literals
                .iter()
                .map(|literal| literal.name.text.chars().count())
                .any(admits);
            if ordered && !any {
                let (keyword, span, detail) = match (lowered.min_length, lowered.max_length) {
                    (Some(min), None) => (
                        "minLength",
                        min_length_span,
                        format!("no literal name has at least {min} characters"),
                    ),
                    (None, Some(max)) => (
                        "maxLength",
                        max_length_span,
                        format!("no literal name has at most {max} characters"),
                    ),
                    (Some(min), Some(max)) => (
                        "minLength",
                        min_length_span,
                        format!("no literal name has between {min} and {max} characters"),
                    ),
                    (None, None) => unreachable!("range is bounded"),
                };
                diags.push(Diagnostic::error(
                    format!("constraint '{keyword}' on attribute '{feature_name}' admits no literal of enum '{enum_name}': {detail}"),
                    span,
                ));
            }
        }
    }
    lowered
}

fn lower_enum(decl: &mox::EnumDecl, diags: &mut Vec<Diagnostic>) -> ir::EnumDef {
    let mut seen = HashSet::new();
    let mut literals = Vec::new();
    for literal in &decl.literals {
        if !seen.insert(literal.name.text.as_str()) {
            diags.push(Diagnostic::error(
                format!(
                    "duplicate enum literal '{}' in enum '{}'",
                    literal.name.text, decl.name.text
                ),
                Some(literal.name.span),
            ));
        }
        let value = match literal.value {
            Some(value) => value,
            None => {
                diags.push(Diagnostic::error(
                    format!(
                        "enum literal '{}' requires an explicit value (e.g. `= 0`)",
                        literal.name.text
                    ),
                    Some(literal.span),
                ));
                0
            }
        };
        let mut lowered_literal =
            ir::EnumLiteral::new(&literal.name.text, literal.label.clone(), value);
        lowered_literal.description = literal.doc.clone();
        literals.push(lowered_literal);
    }
    let mut enum_def = ir::EnumDef::new(&decl.name.text, literals);
    enum_def.description = decl.doc.clone();
    enum_def
}

fn lower_datatype(
    decl: &mox::DatatypeDecl,
    source: &str,
    diags: &mut Vec<Diagnostic>,
) -> ir::DatatypeDef {
    let platform = match &decl.wraps {
        Some(mox::Wraps::Named(name)) => Some(name.full_name()),
        Some(mox::Wraps::Opaque(_)) | None => None,
    };
    let mut datatype = ir::DatatypeDef::new(&decl.name.text, platform);
    datatype.description = decl.doc.clone();
    datatype.format = decl.format.clone();
    for binding in &decl.bindings {
        datatype = datatype.bind(&binding.key.text, &binding.value);
    }
    // Tier 1: create/convert bodies lower next to the binding entries.
    datatype.create = lower_target_bodies(
        &decl.create,
        source,
        &format!("datatype '{}' create", decl.name.text),
        diags,
    );
    datatype.convert = lower_target_bodies(
        &decl.convert,
        source,
        &format!("datatype '{}' convert", decl.name.text),
        diags,
    );
    datatype
}

fn lower_interface(decl: &mox::InterfaceDecl) -> ir::InterfaceDef {
    let mut interface = ir::InterfaceDef::new(&decl.name.text);
    interface.description = decl.doc.clone();
    for binding in &decl.bindings {
        interface = interface.bind(&binding.key.text, &binding.value);
    }
    interface
}

/// Lowers a `vocabulary` declaration: checks its shape, resolves the
/// snapshot version, reads the vendored snapshot, verifies the `model.lock`
/// digest, and validates the entries.
///
/// Returns `None` (with diagnostics) when the declaration cannot be lowered.
fn lower_vocabulary(
    decl: &mox::VocabularyDecl,
    base_dir: &Path,
    diags: &mut Vec<Diagnostic>,
) -> Option<ir::VocabularyDef> {
    let name = &decl.name.text;

    // `key` is required.
    let Some(key) = &decl.key else {
        diags.push(
            Diagnostic::error(
                format!("vocabulary '{name}' is missing its `key` declaration"),
                Some(decl.name.span),
            )
            .with_help(
                "add `key <facetName>` naming the field that uniquely identifies each entry",
            ),
        );
        return None;
    };

    // Facet types must be primitives.
    let mut facets = Vec::new();
    let mut facets_ok = true;
    for facet in &decl.facets {
        match primitive_facet_type(&facet.type_ref) {
            Some(type_) => facets.push(ir::VocabularyFacet {
                name: facet.name.text.clone(),
                type_,
            }),
            None => {
                facets_ok = false;
                diags.push(
                    Diagnostic::error(
                        format!(
                            "facet '{}' of vocabulary '{name}' must have a primitive type, found '{}'",
                            facet.name.text,
                            facet.type_ref.name.full_name()
                        ),
                        Some(facet.type_ref.span),
                    )
                    .with_help(
                        "facet types must be primitives (String/int/long/short/float/double/boolean/byte/char)",
                    ),
                );
            }
        }
    }
    if !facets_ok {
        return None;
    }

    // Resolve the snapshot version: declared version, else the lockfile pin,
    // else a unique vendored snapshot.
    let sanitized = rex_vocab::sanitize_source(&decl.source);
    let vocab_dir = base_dir.join("vocab");
    let lockfile = read_lockfile(base_dir, diags);
    let version = match &decl.version {
        Some(version) => version.clone(),
        None => match lockfile
            .as_ref()
            .and_then(|lockfile| lockfile.entry_for(&decl.source))
            .map(|entry| entry.version.clone())
        {
            Some(version) => version,
            None => match glob_snapshot_version(&vocab_dir, &sanitized) {
                GlobVersion::Unique(version) => version,
                GlobVersion::Missing => {
                    diags.push(
                        Diagnostic::error(
                            format!("cannot determine the version of vocabulary '{name}'"),
                            Some(decl.name.span),
                        )
                        .with_help(
                            "pin `version \"...\"` in the vocabulary declaration, or run `rexlang vocab fetch`",
                        ),
                    );
                    return None;
                }
                GlobVersion::Ambiguous(candidates) => {
                    diags.push(
                        Diagnostic::error(
                            format!(
                                "multiple snapshots found for vocabulary '{name}': {}; pin `version` in the declaration or run `rexlang vocab fetch`",
                                candidates.join(", ")
                            ),
                            Some(decl.name.span),
                        )
                        .with_help(
                            "vendoring one snapshot per version keeps builds reproducible",
                        ),
                    );
                    return None;
                }
            },
        },
    };

    // Read the vendored snapshot. Builds stay hermetic: never fetched at
    // compile time, only read from disk.
    let file_name = format!("{sanitized}@{version}.json");
    let snapshot_path = vocab_dir.join(&file_name);
    let bytes = match std::fs::read(&snapshot_path) {
        Ok(bytes) => bytes,
        Err(_) => {
            diags.push(
                Diagnostic::error(
                    format!(
                        "missing snapshot file '{}' for vocabulary '{name}'",
                        snapshot_path.display()
                    ),
                    Some(decl.name.span),
                )
                .with_help(
                    "run `rexlang vocab fetch <model>` to vendor it (builds stay hermetic — never fetched at compile time)",
                ),
            );
            return None;
        }
    };

    // Verify the lockfile digest, or warn about the missing pin.
    match &lockfile {
        Some(lockfile) => match lockfile.entry_for(&decl.source) {
            Some(entry) if entry.version == version => {
                let found = rex_vocab::digest(&bytes);
                if entry.digest != found {
                    diags.push(Diagnostic::error(
                        format!(
                            "snapshot digest mismatch for vocabulary '{name}' ({file_name}): model.lock records {}, but the vendored snapshot hashes to {}",
                            entry.digest, found
                        ),
                        Some(decl.name.span),
                    ));
                    return None;
                }
            }
            Some(entry) => {
                diags.push(Diagnostic::warning(
                    format!(
                        "vocabulary '{name}' uses version '{version}', but model.lock pins '{}'; run `rexlang vocab fetch` to reconcile",
                        entry.version
                    ),
                    Some(decl.name.span),
                ));
            }
            None => push_unpinned_warning(name, decl.name.span, diags),
        },
        None => push_unpinned_warning(name, decl.name.span, diags),
    }

    // Parse and validate the snapshot entries.
    let parsed = match rex_vocab::parse_snapshot(&bytes) {
        Ok(parsed) => parsed,
        Err(error) => {
            diags.push(Diagnostic::error(
                format!("vocabulary '{name}' snapshot {file_name} is invalid: {error}"),
                Some(decl.name.span),
            ));
            return None;
        }
    };
    let declaration = rex_vocab::VocabularyDeclaration {
        source: decl.source.clone(),
        version: Some(version.clone()),
        key: key.text.clone(),
        facets: facets.clone(),
    };
    let entries = match rex_vocab::validate_entries(&parsed, &declaration) {
        Ok(entries) => entries,
        Err(error) => {
            diags.push(Diagnostic::error(
                format!("vocabulary '{name}' snapshot {file_name} is invalid: {error}"),
                Some(decl.name.span),
            ));
            return None;
        }
    };

    Some(ir::VocabularyDef {
        name: name.clone(),
        description: decl.doc.clone(),
        source: decl.source.clone(),
        version: Some(version),
        key: key.text.clone(),
        facets,
        entries,
    })
}

/// The "not pinned" warning shown when no lockfile records the vocabulary.
fn push_unpinned_warning(name: &str, span: Span, diags: &mut Vec<Diagnostic>) {
    diags.push(Diagnostic::warning(
        format!(
            "vocabulary '{name}' is not pinned; run `rexlang vocab fetch` to generate model.lock"
        ),
        Some(span),
    ));
}

/// Reads `model.lock` next to the model. A missing lockfile is not an error
/// (an "unpinned" warning is emitted later); an unreadable one is.
fn read_lockfile(base_dir: &Path, diags: &mut Vec<Diagnostic>) -> Option<rex_vocab::Lockfile> {
    let path = base_dir.join("model.lock");
    match rex_vocab::Lockfile::read(&path) {
        Ok(lockfile) => Some(lockfile),
        Err(error) => {
            let not_found = error.chain().any(|cause| {
                cause
                    .downcast_ref::<std::io::Error>()
                    .is_some_and(|io| io.kind() == std::io::ErrorKind::NotFound)
            });
            if !not_found {
                diags.push(Diagnostic::error(
                    format!("cannot read model.lock: {error}"),
                    None,
                ));
            }
            None
        }
    }
}

/// The outcome of scanning `vocab/` for `<sanitized>@<version>.json` files.
enum GlobVersion {
    /// No snapshot (or no `vocab/` directory).
    Missing,
    /// Exactly one snapshot; the version parsed from its file name.
    Unique(String),
    /// Several snapshots; the user must pin the version.
    Ambiguous(Vec<String>),
}

fn glob_snapshot_version(vocab_dir: &Path, sanitized: &str) -> GlobVersion {
    let Ok(read_dir) = std::fs::read_dir(vocab_dir) else {
        return GlobVersion::Missing;
    };
    let prefix = format!("{sanitized}@");
    let mut versions: Vec<String> = read_dir
        .flatten()
        .filter_map(|entry| entry.file_name().into_string().ok())
        .filter_map(|file_name| {
            file_name
                .strip_prefix(&prefix)
                .and_then(|rest| rest.strip_suffix(".json"))
                .map(str::to_string)
        })
        .collect();
    versions.sort();
    match versions.len() {
        0 => GlobVersion::Missing,
        1 => GlobVersion::Unique(versions.pop().expect("len checked")),
        _ => GlobVersion::Ambiguous(versions),
    }
}

/// The primitive type of a facet type reference, when it names one.
fn primitive_facet_type(type_ref: &mox::TypeRef) -> Option<ir::PrimitiveType> {
    match type_ref.name.segments.as_slice() {
        [segment] => primitive_type(&segment.text),
        _ => None,
    }
}

fn lower_class(
    decl: &mox::ClassDecl,
    source: &str,
    path: &str,
    package: &str,
    scope: &Scope<'_>,
    prep: &PrepMaps<'_>,
    diags: &mut Vec<Diagnostic>,
) -> ClassRecord {
    let mut extends = Vec::new();
    for type_ref in &decl.extends {
        if let Some(resolution) = scope.resolve(type_ref, diags) {
            match resolution.kind {
                Resolved::Class | Resolved::Interface => {
                    extends.push(resolution.kind.to_ir(&resolution.package, &resolution.name));
                }
                _ => diags.push(
                    Diagnostic::error(
                        format!(
                            "class '{}' cannot extend '{}'",
                            decl.name.text,
                            type_ref.name.full_name()
                        ),
                        Some(type_ref.span),
                    )
                    .with_help("extends must name a class or interface"),
                ),
            }
        }
    }

    let mut features = Vec::new();
    let mut operations = Vec::new();
    let mut records = Vec::new();
    let mut seen = HashSet::new();
    for feature in &decl.features {
        let name = feature.name();
        if !seen.insert(name.text.as_str()) {
            diags.push(Diagnostic::error(
                format!(
                    "duplicate feature '{}' in class '{}'",
                    name.text, decl.name.text
                ),
                Some(name.span),
            ));
        }
        match feature {
            mox::FeatureDecl::Attribute {
                doc,
                type_ref,
                multiplicity,
                name,
                default,
                constraints,
                ..
            } => {
                let resolution = scope.resolve(type_ref, diags);
                report_class_typed_feature(name, type_ref, resolution.as_ref(), diags);
                let multiplicity = multiplicity
                    .as_ref()
                    .map(|m| lower_multiplicity(m, diags))
                    .unwrap_or(ir::Multiplicity::REQUIRED);
                let ir_type = ir_type_of(resolution.as_ref(), scope.fallback_package(), type_ref);
                let ir_feature = ir::Feature::new(
                    &name.text,
                    ir::FeatureKind::Attribute,
                    ir_type,
                    multiplicity,
                );
                let ir_feature = apply_default(
                    ir_feature,
                    default.as_ref(),
                    resolution.as_ref(),
                    prep,
                    diags,
                );
                let mut ir_feature = apply_modifiers(ir_feature, feature.modifiers());
                ir_feature.description = doc.clone();
                ir_feature.constraints = lower_constraints(
                    constraints,
                    multiplicity.is_many(),
                    resolution.as_ref(),
                    prep,
                    &name.text,
                    diags,
                );
                features.push(ir_feature);
                records.push(FeatureRecord {
                    name: name.text.clone(),
                    kind: FKind::Attribute,
                    type_class: class_location(resolution.as_ref()),
                    type_text: type_ref.name.full_name(),
                    type_span: Some(type_ref.span),
                    opposite: None,
                    opposite_span: None,
                });
            }
            mox::FeatureDecl::Containment {
                doc,
                type_ref,
                multiplicity,
                name,
                opposite,
                ..
            } => {
                let (ir_feature, record) = lower_relation(
                    FKind::Containment,
                    type_ref,
                    multiplicity.as_ref(),
                    name,
                    opposite.as_ref(),
                    scope,
                    diags,
                );
                let mut ir_feature = apply_modifiers(ir_feature, feature.modifiers());
                ir_feature.description = doc.clone();
                features.push(ir_feature);
                records.push(record);
            }
            mox::FeatureDecl::Reference {
                doc,
                type_ref,
                multiplicity,
                name,
                opposite,
                ..
            } => {
                let (ir_feature, record) = lower_relation(
                    FKind::Reference,
                    type_ref,
                    multiplicity.as_ref(),
                    name,
                    opposite.as_ref(),
                    scope,
                    diags,
                );
                let mut ir_feature = apply_modifiers(ir_feature, feature.modifiers());
                ir_feature.description = doc.clone();
                features.push(ir_feature);
                records.push(record);
            }
            mox::FeatureDecl::Container {
                doc,
                type_ref,
                name,
                opposite,
                ..
            } => {
                // Containers never take a multiplicity in the grammar; the
                // driver forces OPTIONAL regardless.
                let (ir_feature, record) = lower_relation(
                    FKind::Container,
                    type_ref,
                    None,
                    name,
                    opposite.as_ref(),
                    scope,
                    diags,
                );
                let mut ir_feature = apply_modifiers(ir_feature, feature.modifiers());
                ir_feature.description = doc.clone();
                features.push(ir_feature);
                records.push(record);
            }
            mox::FeatureDecl::Op {
                doc,
                return_type,
                name,
                params,
                body,
                bodies,
                ..
            } => {
                // The `id`/`readonly` modifiers only apply to stored features,
                // so they warn here.
                let modifiers = feature.modifiers();
                if let Some(span) = modifiers.id {
                    diags.push(Diagnostic::warning(
                        "modifier 'id' has no effect on operations",
                        Some(span),
                    ));
                }
                if let Some(span) = modifiers.read_only {
                    diags.push(Diagnostic::warning(
                        "modifier 'readonly' has no effect on operations",
                        Some(span),
                    ));
                }
                // Tier 1: operations lower into `ClassDef.operations` with
                // their resolved signature; bodies must be per-target and are
                // carried verbatim. Body-less operations are abstract hooks.
                if body.is_some() && bodies.is_empty() {
                    diags.push(Diagnostic::error(
                        "operation bodies must be per-target in Tier 1 (e.g. `rust { ... }`)",
                        Some(body.expect("body span checked above")),
                    ));
                }
                let lowered_bodies = lower_target_bodies(
                    bodies,
                    source,
                    &format!("operation '{}'", name.text),
                    diags,
                );
                let return_resolution = scope.resolve(return_type, diags);
                let mut lowered_params = Vec::new();
                for param in params {
                    let resolution = scope.resolve(&param.type_ref, diags);
                    lowered_params.push(ir::OperationParam {
                        name: param.name.text.clone(),
                        type_: ir_type_of(
                            resolution.as_ref(),
                            scope.fallback_package(),
                            &param.type_ref,
                        ),
                    });
                }
                let mut operation = ir::Operation::new(
                    &name.text,
                    ir_type_of(
                        return_resolution.as_ref(),
                        scope.fallback_package(),
                        return_type,
                    ),
                    lowered_params,
                );
                operation.description = doc.clone();
                operation.bodies = lowered_bodies;
                operations.push(operation);
                records.push(FeatureRecord {
                    name: name.text.clone(),
                    kind: FKind::Op,
                    type_class: None,
                    type_text: return_type.name.full_name(),
                    type_span: None,
                    opposite: None,
                    opposite_span: None,
                });
            }
            mox::FeatureDecl::Derived {
                doc,
                type_ref,
                multiplicity,
                name,
                body,
                bodies,
                ..
            } => {
                let resolution = scope.resolve(type_ref, diags);
                report_class_typed_feature(name, type_ref, resolution.as_ref(), diags);
                // Tier 2: a derived body must be the neutral expression
                // language, in a single `expr { ... }` block.
                if body.is_some() && bodies.is_empty() {
                    diags.push(Diagnostic::error(
                        "derived bodies must use the neutral expression language: \
                         get { expr { ... } }",
                        Some(body.expect("body span checked above")),
                    ));
                }
                let lowered_bodies = lower_target_bodies(
                    bodies,
                    source,
                    &format!("derived feature '{}'", name.text),
                    diags,
                );
                for target in lowered_bodies.keys() {
                    if target != "expr" {
                        let span = bodies
                            .iter()
                            .find(|body| body.target.text == *target)
                            .map(|body| body.target.span);
                        diags.push(Diagnostic::error(
                            "derived bodies must use the neutral expression language: \
                             get { expr { ... } }",
                            span,
                        ));
                    }
                }
                let multiplicity = multiplicity
                    .as_ref()
                    .map(|m| lower_multiplicity(m, diags))
                    .unwrap_or(ir::Multiplicity::OPTIONAL);
                let ir_type = ir_type_of(resolution.as_ref(), scope.fallback_package(), type_ref);
                let mut ir_feature = ir::Feature::new(
                    &name.text,
                    ir::FeatureKind::Attribute,
                    ir_type,
                    multiplicity,
                )
                .derived();
                ir_feature = apply_modifiers(ir_feature, feature.modifiers());
                ir_feature.description = doc.clone();
                ir_feature.bodies = lowered_bodies;
                features.push(ir_feature);
                records.push(FeatureRecord {
                    name: name.text.clone(),
                    kind: FKind::Derived,
                    type_class: class_location(resolution.as_ref()),
                    type_text: type_ref.name.full_name(),
                    type_span: Some(type_ref.span),
                    opposite: None,
                    opposite_span: None,
                });
            }
        }
    }

    let mut class_def = ir::ClassDef::new(&decl.name.text, extends, features);
    class_def.description = decl.doc.clone();
    ClassRecord {
        package: package.to_string(),
        file: path.to_string(),
        name: decl.name.text.clone(),
        name_span: decl.name.span,
        def: class_def,
        features: records,
        operations,
    }
}

/// The flagship diagnostic: an attribute (or derived feature) whose type
/// resolves to a class almost certainly means the author meant `contains` or
/// `refers`.
fn report_class_typed_feature(
    name: &mox::Name,
    type_ref: &mox::TypeRef,
    resolution: Option<&Resolution>,
    diags: &mut Vec<Diagnostic>,
) {
    let Some(resolution) = resolution else {
        return;
    };
    if resolution.kind != Resolved::Class {
        return;
    }
    let class = type_ref.name.full_name();
    diags.push(
        Diagnostic::error(
            format!("feature '{}' has class type '{}'", name.text, class),
            Some(type_ref.span),
        )
        .with_help(format!(
            "did you mean `contains {class}[..] {}` or `refers {class}[..] {}`?",
            name.text, name.text
        ))
        .with_code(DiagnosticCode::AttributeWithClassType {
            feature: name.text.clone(),
            class,
            type_span: type_ref.span,
        }),
    );
}

/// Lowers a `contains` / `refers` / `container` feature.
#[allow(clippy::too_many_arguments)]
fn lower_relation(
    kind: FKind,
    type_ref: &mox::TypeRef,
    multiplicity: Option<&mox::Multiplicity>,
    name: &mox::Name,
    opposite: Option<&mox::Name>,
    scope: &Scope<'_>,
    diags: &mut Vec<Diagnostic>,
) -> (ir::Feature, FeatureRecord) {
    let resolution = scope.resolve(type_ref, diags);
    if let Some(resolution) = &resolution {
        if resolution.kind != Resolved::Class {
            diags.push(Diagnostic::error(
                format!(
                    "feature '{}' is declared `{}` but type '{}' is not a class",
                    name.text,
                    kind.keyword(),
                    type_ref.name.full_name()
                ),
                Some(type_ref.span),
            ));
        }
    }

    let multiplicity = match kind {
        FKind::Container => ir::Multiplicity::OPTIONAL,
        _ => multiplicity
            .map(|m| lower_multiplicity(m, diags))
            .unwrap_or(ir::Multiplicity::MANY),
    };
    let ir_type = ir_type_of(resolution.as_ref(), scope.fallback_package(), type_ref);
    let mut ir_feature = ir::Feature::new(
        &name.text,
        kind.ir_kind()
            .expect("relations always lower to an IR kind"),
        ir_type,
        multiplicity,
    );
    if let (Some(opposite), Some(class)) = (opposite, class_of(resolution.as_ref())) {
        ir_feature = ir_feature.with_opposite(class, &opposite.text);
    }

    let record = FeatureRecord {
        name: name.text.clone(),
        kind,
        type_class: class_location(resolution.as_ref()),
        type_text: type_ref.name.full_name(),
        type_span: Some(type_ref.span),
        opposite: opposite.map(|opposite| opposite.text.clone()),
        opposite_span: opposite.map(|opposite| opposite.span),
    };
    (ir_feature, record)
}

fn lower_multiplicity(
    multiplicity: &mox::Multiplicity,
    diags: &mut Vec<Diagnostic>,
) -> ir::Multiplicity {
    match &multiplicity.kind {
        mox::MultiplicityKind::Unbounded => ir::Multiplicity::MANY,
        mox::MultiplicityKind::Exact(exact) => {
            let bound = bound_value(*exact, multiplicity.span, diags).unwrap_or(1);
            ir::Multiplicity::new(bound, ir::Upper::Finite(bound))
        }
        mox::MultiplicityKind::Range(lower, upper) => {
            let lower = bound_value(*lower, multiplicity.span, diags).unwrap_or(0);
            let upper = match upper {
                mox::MultBound::Star => ir::Upper::Unbounded,
                mox::MultBound::Int(value) => ir::Upper::Finite(
                    bound_value(*value, multiplicity.span, diags).unwrap_or(lower),
                ),
            };
            ir::Multiplicity::new(lower, upper)
        }
    }
}

/// Converts a multiplicity bound, reporting out-of-range values.
fn bound_value(value: i64, span: Span, diags: &mut Vec<Diagnostic>) -> Option<u32> {
    u32::try_from(value).ok().or_else(|| {
        diags.push(Diagnostic::error(
            format!("invalid multiplicity bound {value}: bounds must be between 0 and 4294967295"),
            Some(span),
        ));
        None
    })
}

/// Reports a literal default that cannot initialize the attribute's
/// declared type, pointing at the default literal.
fn report_default_mismatch(
    form: &str,
    literal: &str,
    feature: &ir::Feature,
    span: Span,
    resolution: &Resolution,
    diags: &mut Vec<Diagnostic>,
) {
    let description = match resolution.kind {
        Resolved::Primitive(primitive) => primitive.to_string(),
        Resolved::Enum => "enum-typed".to_string(),
        Resolved::Datatype => "datatype-typed".to_string(),
        Resolved::Class => "class-typed".to_string(),
        Resolved::Interface => "interface-typed".to_string(),
        Resolved::Vocabulary => "vocabulary-typed".to_string(),
    };
    let mut diagnostic = Diagnostic::error(
        format!(
            "{form} default '{literal}' on {description} attribute '{}'",
            feature.name
        ),
        Some(span),
    );
    if resolution.kind == Resolved::Datatype {
        diagnostic =
            diagnostic.with_help("datatype-typed attributes admit only a string literal default");
    }
    diags.push(diagnostic);
}

/// Applies an attribute default value. The `Name` form is an enum literal
/// reference on enum-typed attributes, or an entry-key reference on
/// vocabulary-typed attributes (lowered to `DefaultValue::String`, since
/// vocabulary keys are strings).
///
/// Literal defaults (`string`/`int`/`boolean`) are type-checked against the
/// attribute's declared type: a string literal initializes text (string,
/// char), a datatype's wrapped string, or a vocabulary key; an int literal
/// initializes a numeric primitive; a boolean literal initializes `boolean`
/// only. Unresolvable attribute types are left to the type diagnostic.
#[allow(clippy::too_many_arguments)]
fn apply_default(
    feature: ir::Feature,
    default: Option<&mox::DefaultValue>,
    resolution: Option<&Resolution>,
    prep: &PrepMaps<'_>,
    diags: &mut Vec<Diagnostic>,
) -> ir::Feature {
    let Some(default) = default else {
        return feature;
    };
    match default {
        mox::DefaultValue::Str { value, span } => {
            if let Some(resolution) = resolution {
                let admitted = match resolution.kind {
                    Resolved::Primitive(primitive) => {
                        matches!(
                            primitive,
                            ir::PrimitiveType::String | ir::PrimitiveType::Char
                        )
                    }
                    Resolved::Datatype | Resolved::Vocabulary => true,
                    _ => false,
                };
                if !admitted {
                    report_default_mismatch("string", value, &feature, *span, resolution, diags);
                }
            }
            feature.with_default(ir::DefaultValue::String(value.clone()))
        }
        mox::DefaultValue::Int { value, span } => {
            if let Some(resolution) = resolution {
                let admitted = matches!(resolution.kind, Resolved::Primitive(p) if p.is_numeric());
                if !admitted {
                    report_default_mismatch(
                        "int",
                        &value.to_string(),
                        &feature,
                        *span,
                        resolution,
                        diags,
                    );
                }
            }
            feature.with_default(ir::DefaultValue::Int(*value))
        }
        mox::DefaultValue::Bool { value, span } => {
            if let Some(resolution) = resolution {
                let admitted = resolution.kind == Resolved::Primitive(ir::PrimitiveType::Boolean);
                if !admitted {
                    report_default_mismatch(
                        "boolean",
                        &value.to_string(),
                        &feature,
                        *span,
                        resolution,
                        diags,
                    );
                }
            }
            feature.with_default(ir::DefaultValue::Bool(*value))
        }
        mox::DefaultValue::Name(name) => match resolution {
            Some(Resolution {
                kind: Resolved::Enum,
                name: enum_name,
                package,
            }) => {
                if let Some(enum_decl) =
                    prep.enum_decls.get(&(package.as_str(), enum_name.as_str()))
                {
                    if !enum_decl
                        .literals
                        .iter()
                        .any(|literal| literal.name.text == name.text)
                    {
                        diags.push(Diagnostic::error(
                            format!("enum '{enum_name}' has no literal '{}'", name.text),
                            Some(name.span),
                        ));
                    }
                }
                feature.with_default(ir::DefaultValue::EnumLiteral(name.text.clone()))
            }
            Some(Resolution {
                kind: Resolved::Vocabulary,
                name: vocab_name,
                package,
            }) => {
                // An absent key set means the vocabulary failed to load; that
                // error is already reported, so the default adds nothing.
                if let Some(keys) = prep
                    .vocab_keys
                    .get(&(package.as_str(), vocab_name.as_str()))
                {
                    if !keys.contains(name.text.as_str()) {
                        diags.push(Diagnostic::error(
                            format!("vocabulary '{vocab_name}' has no entry '{}'", name.text),
                            Some(name.span),
                        ));
                    }
                }
                // Keys are strings; `String` is the wire representation.
                feature.with_default(ir::DefaultValue::String(name.text.clone()))
            }
            Some(_) => {
                diags.push(Diagnostic::error(
                    format!(
                        "default value '{}' requires an enum-typed attribute",
                        name.text
                    ),
                    Some(name.span),
                ));
                feature
            }
            // The attribute type failed to resolve; that error is already
            // reported, so the default adds no information.
            None => feature,
        },
    }
}

/// Detects cycles in the class inheritance graph (class → class `extends`
/// edges, keyed by `(package, name)` so cross-package edges are followed),
/// reporting the first cycle found. Diagnostics are tagged with the file
/// that closes the cycle.
fn detect_inheritance_cycles(classes: &[ClassRecord], diags: &mut Vec<(String, Diagnostic)>) {
    let index: HashMap<(&str, &str), &ClassRecord> = classes
        .iter()
        .map(|class| ((class.package.as_str(), class.name.as_str()), class))
        .collect();
    let mut state: HashMap<(&str, &str), u8> = HashMap::new();
    for class in classes {
        let mut path: Vec<(&str, &str)> = Vec::new();
        if let Some(cycle) = visit_class(
            (class.package.as_str(), class.name.as_str()),
            &index,
            &mut state,
            &mut path,
        ) {
            let record = index.get(&cycle[0]).expect("cycle member is indexed");
            let names: Vec<&str> = cycle.iter().map(|(_, name)| *name).collect();
            diags.push((
                record.file.clone(),
                Diagnostic::error(
                    format!("inheritance cycle detected: {}", names.join(" extends ")),
                    Some(record.name_span),
                ),
            ));
            return;
        }
    }
}

/// Iterative-depth-first visit; returns the cycle path when a back edge is
/// found. `state`: absent = unvisited, `1` = on the current path, `2` = done.
fn visit_class<'a>(
    class: (&'a str, &'a str),
    index: &HashMap<(&'a str, &'a str), &'a ClassRecord>,
    state: &mut HashMap<(&'a str, &'a str), u8>,
    path: &mut Vec<(&'a str, &'a str)>,
) -> Option<Vec<(&'a str, &'a str)>> {
    match state.get(&class) {
        Some(1) => {
            let start = path.iter().position(|member| *member == class).unwrap_or(0);
            let mut cycle: Vec<(&str, &str)> = path[start..].to_vec();
            cycle.push(class);
            return Some(cycle);
        }
        Some(_) => return None,
        None => {}
    }
    state.insert(class, 1);
    path.push(class);
    if let Some(record) = index.get(&class) {
        for superclass in record
            .def
            .extends
            .iter()
            .filter_map(|type_ref| match type_ref {
                ir::TypeRef::Class { package, name, .. } => Some((package.as_str(), name.as_str())),
                _ => None,
            })
        {
            if let Some(cycle) = visit_class(superclass, index, state, path) {
                return Some(cycle);
            }
        }
    }
    path.pop();
    state.insert(class, 2);
    None
}

/// Validates opposite pairing, mutually:
///
/// * `contains` (in `A`, typed `B`) ↔ `container` (in `B`, typed `A`);
/// * `refers` ↔ `refers`;
///
/// and requires each side's `opposite` to name the other feature back. Each
/// violated declaration produces one error naming both sides.
///
/// Relations are same-package only: a `contains`/`container` targeting a
/// foreign-package class is rejected outright (ownership does not span
/// packages), and a `refers` may only cross packages one-sided. Diagnostics
/// are tagged with the declaring file.
fn validate_opposites(classes: &[ClassRecord], diags: &mut Vec<(String, Diagnostic)>) {
    let index: HashMap<(&str, &str), &ClassRecord> = classes
        .iter()
        .map(|class| ((class.package.as_str(), class.name.as_str()), class))
        .collect();
    for class in classes {
        for feature in &class.features {
            if !matches!(
                feature.kind,
                FKind::Containment | FKind::Reference | FKind::Container
            ) {
                continue;
            }
            if let Some((target_package, _)) = &feature.type_class {
                if target_package != class.package.as_str() {
                    match feature.kind {
                        FKind::Containment | FKind::Container => {
                            diags.push((
                                class.file.clone(),
                                Diagnostic::error(
                                    format!(
                                        "cross-package ownership is not supported: feature '{}.{}' {} '{}'",
                                        class.name,
                                        feature.name,
                                        feature.kind.keyword(),
                                        feature.type_text,
                                    ),
                                    feature.type_span,
                                )
                                .with_help(
                                    "declare `contains` targets within the same package, or use `refers` for cross-package links",
                                ),
                            ));
                        }
                        _ if feature.opposite.is_some() => {
                            diags.push((
                                class.file.clone(),
                                Diagnostic::error(
                                    "cross-package opposites are not supported; declare the opposite within the same package",
                                    feature.opposite_span,
                                ),
                            ));
                        }
                        _ => {}
                    }
                    continue;
                }
            }
            let (Some((_, target)), Some(opposite)) = (&feature.type_class, &feature.opposite)
            else {
                continue;
            };
            let Some(opposite_span) = feature.opposite_span else {
                continue;
            };
            let Some(other) = index.get(&(class.package.as_str(), target.as_str())) else {
                continue;
            };
            let Some(counterpart) = other
                .features
                .iter()
                .find(|candidate| &candidate.name == opposite)
            else {
                diags.push((
                    class.file.clone(),
                    Diagnostic::error(
                        format!(
                            "opposite mismatch: '{}.{}' declares opposite '{}.{}', but class '{}' has no feature '{}'",
                            class.name, feature.name, target, opposite, target, opposite,
                        ),
                        Some(opposite_span),
                    ),
                ));
                continue;
            };
            let expected = match feature.kind {
                FKind::Containment => FKind::Container,
                FKind::Container => FKind::Containment,
                _ => FKind::Reference,
            };
            if counterpart.kind != expected {
                diags.push((
                    class.file.clone(),
                    Diagnostic::error(
                        format!(
                            "opposite mismatch: '{}.{}' expects '{}.{}' to be {}, but found {}",
                            class.name,
                            feature.name,
                            target,
                            opposite,
                            expected_description(feature.kind, &class.name),
                            counterpart.kind.describe(),
                        ),
                        Some(opposite_span),
                    ),
                ));
                continue;
            }
            if counterpart.type_class != Some((class.package.clone(), class.name.clone())) {
                diags.push((
                    class.file.clone(),
                    Diagnostic::error(
                        format!(
                            "opposite mismatch: '{}.{}' expects '{}.{}' to have type '{}', but found '{}'",
                            class.name, feature.name, target, opposite, class.name, counterpart.type_text,
                        ),
                        Some(opposite_span),
                    ),
                ));
                continue;
            }
            if counterpart.opposite.as_deref() != Some(feature.name.as_str()) {
                diags.push((
                    class.file.clone(),
                    Diagnostic::error(
                        format!(
                            "opposite mismatch: '{}.{}' does not declare opposite '{}.{}'",
                            target, opposite, class.name, feature.name,
                        ),
                        Some(opposite_span),
                    ),
                ));
            }
        }
    }
}

/// The "expected" half of an opposite-mismatch message.
fn expected_description(kind: FKind, class: &str) -> String {
    match kind {
        FKind::Containment => format!("a `container` feature of type '{class}'"),
        FKind::Container => format!("a `contains` feature of type '{class}'"),
        FKind::Reference => format!("a `refers` feature of type '{class}'"),
        _ => String::new(),
    }
}

/// The verbatim text inside a `when (...)` condition: strictly inside the
/// parens, ends-trimmed (the same span-to-text convention as target bodies).
fn when_text(source: &str, span: Span) -> &str {
    source[span.start + 1..span.end - 1].trim()
}

/// Syntax-checks a `when` condition with the neutral expression parser
/// (`rex-expr`, syntax only — bare condition names are resource features
/// whose typing is a backend concern). Every parse error is re-spanned into
/// the file: the error's byte offsets are relative to the condition text, so
/// they are shifted onto the condition's inner region (clamped to it).
fn check_when_syntax(source: &str, when_span: Span, diags: &mut Vec<Diagnostic>) {
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

/// Lowers an `actors` declaration: duplicate actor/capability checks,
/// capability class resolution against the given [`Scope`], grant-entry
/// capability checks, `when` condition syntax checks (type checking is
/// deferred — see [`PendingCondition`]), and `never_both` shape checks.
/// Cross-declaration rules (cycles, separation of duty, self-narrowing) run
/// in the passes below, per-block for single-file compiles and over the
/// union for `.actor` compiles.
fn lower_actors(
    decl: &mox::ActorsDecl,
    source: &str,
    file: &str,
    scope: &Scope<'_>,
    diags: &mut Vec<Diagnostic>,
) -> (ActorsRecord, Vec<PendingCondition>) {
    let mut def = ir::ActorsDef::new(decl.name.text.clone());
    let mut pending: Vec<PendingCondition> = Vec::new();
    let mut actor_names: Vec<(String, Span)> = Vec::new();
    let mut actor_kinds: Vec<(String, Option<ir::ActorKind>)> = Vec::new();
    let mut parents: Vec<(String, Option<String>)> = Vec::new();
    let mut purposes: Vec<(String, Span)> = Vec::new();
    let mut forbids: Vec<(String, String, Span)> = Vec::new();
    let mut never_both: Vec<(Vec<String>, Span)> = Vec::new();
    let mut delegations: Vec<DelegationRecord> = Vec::new();

    let mut seen_actors = HashSet::new();
    for actor in &decl.actors {
        if !seen_actors.insert(actor.name.text.as_str()) {
            diags.push(Diagnostic::error(
                format!("duplicate actor `{}`", actor.name.text),
                Some(actor.name.span),
            ));
        }
        let mut ir_actor = ir::ActorDef::new(&actor.name.text);
        if let Some(parent) = &actor.extends {
            if !actor_names.iter().any(|(name, _)| *name == parent.text) {
                diags.push(Diagnostic::error(
                    format!(
                        "actor `{}` extends unknown actor `{}`",
                        actor.name.text, parent.text
                    ),
                    Some(parent.span),
                ));
            }
            ir_actor = ir_actor.extends(&parent.text);
        }
        let ir_kind = match actor.kind {
            mox::ActorKind::Human => None,
            mox::ActorKind::Agent => Some(ir::ActorKind::Agent),
        };
        if let Some(kind) = ir_kind {
            ir_actor = ir_actor.kind(kind);
        }
        def = def.actor(ir_actor);
        actor_names.push((actor.name.text.clone(), actor.name.span));
        actor_kinds.push((actor.name.text.clone(), ir_kind));
        parents.push((
            actor.name.text.clone(),
            actor.extends.as_ref().map(|parent| parent.text.clone()),
        ));
    }

    let mut seen_capabilities = HashSet::new();
    let mut capabilities: HashSet<&str> = HashSet::new();
    let mut capability_classes: HashMap<&str, Option<(String, String)>> = HashMap::new();
    for capability in &decl.capabilities {
        if !seen_capabilities.insert(capability.name.text.as_str()) {
            diags.push(Diagnostic::error(
                format!("duplicate capability `{}`", capability.name.text),
                Some(capability.name.span),
            ));
        }
        let resolution = scope.resolve(&capability.class, diags);
        let mut class = None;
        if let Some(resolution) = &resolution {
            if resolution.kind == Resolved::Class {
                class = Some((resolution.package.clone(), resolution.name.clone()));
            } else {
                diags.push(
                    Diagnostic::error(
                        format!(
                            "capability `{}` must be granted on a class, but `{}` is not a class",
                            capability.name.text,
                            capability.class.name.full_name()
                        ),
                        Some(capability.class.span),
                    )
                    .with_help("capabilities are granted on class instances"),
                );
            }
        }
        capabilities.insert(&capability.name.text);
        def = def.capability(ir::CapabilityDef::new(
            &capability.name.text,
            ir_type_of(
                resolution.as_ref(),
                scope.fallback_package(),
                &capability.class,
            ),
        ));
        capability_classes.insert(&capability.name.text, class);
    }

    let mut seen_purposes = HashSet::new();
    for purpose in &decl.purposes {
        if !seen_purposes.insert(purpose.name.text.as_str()) {
            diags.push(Diagnostic::error(
                format!("duplicate purpose `{}`", purpose.name.text),
                Some(purpose.name.span),
            ));
        }
        def = def.purpose(&purpose.name.text);
        purposes.push((purpose.name.text.clone(), purpose.name.span));
    }

    for grant in &decl.grants {
        if !actor_names
            .iter()
            .any(|(name, _)| *name == grant.actor.text)
        {
            diags.push(Diagnostic::error(
                format!("grant names unknown actor `{}`", grant.actor.text),
                Some(grant.actor.span),
            ));
        }
        let mut ir_grant = ir::GrantDef::new(&grant.actor.text);
        for entry in &grant.entries {
            match entry {
                mox::GrantEntryDecl::Effect(effect) => {
                    if !capabilities.contains(effect.capability.text.as_str()) {
                        diags.push(Diagnostic::error(
                            format!(
                                "grant names unknown capability `{}`",
                                effect.capability.text
                            ),
                            Some(effect.capability.span),
                        ));
                    }
                    if effect.effect == mox::Effect::Forbid {
                        forbids.push((
                            grant.actor.text.clone(),
                            effect.capability.text.clone(),
                            effect.capability.span,
                        ));
                    }
                    let ir_effect = match effect.effect {
                        mox::Effect::Permit => ir::GrantEffect::Permit,
                        mox::Effect::Forbid => ir::GrantEffect::Forbid,
                    };
                    let mut ir_entry = ir::GrantEntry::new(ir_effect, &effect.capability.text);
                    if let Some(when_span) = effect.when {
                        ir_entry = ir_entry.when(when_text(source, when_span));
                        check_when_syntax(source, when_span, diags);
                        let class = capability_classes
                            .get(effect.capability.text.as_str())
                            .cloned()
                            .flatten();
                        pending.push(pending_condition(file, source, when_span, class));
                    }
                    for obligation in &effect.obligations {
                        ir_entry = ir_entry.obligation(&obligation.text);
                    }
                    ir_grant = ir_grant.entry(ir_entry);
                }
                mox::GrantEntryDecl::Cedar(body) => {
                    // Cedar entries carry their verbatim policy text; the
                    // effect/capability pair is unused and stays empty.
                    ir_grant = ir_grant.entry(
                        ir::GrantEntry::new(ir::GrantEffect::Permit, "")
                            .cedar(body_text(source, body.span).trim()),
                    );
                }
            }
        }
        def = def.grant(ir_grant);
    }

    let mut seen_delegations = HashSet::new();
    for delegation in &decl.delegations {
        if !seen_delegations.insert(delegation.name.text.as_str()) {
            diags.push(Diagnostic::error(
                format!("duplicate delegation `{}`", delegation.name.text),
                Some(delegation.name.span),
            ));
        }
        let mut ir_delegation = ir::DelegationDef::new(
            &delegation.name.text,
            &delegation.from.text,
            &delegation.to.text,
        );
        if let Some(purpose) = &delegation.purpose {
            ir_delegation = ir_delegation.purpose(&purpose.text);
        }
        let mut permits: Vec<(String, Span)> = Vec::new();
        for entry in &delegation.entries {
            let declared = capabilities.contains(entry.capability.text.as_str());
            if !declared {
                diags.push(Diagnostic::error(
                    format!(
                        "delegation `{}` names unknown capability `{}`",
                        delegation.name.text, entry.capability.text
                    ),
                    Some(entry.capability.span),
                ));
            }
            let ir_effect = match entry.effect {
                mox::Effect::Permit => ir::GrantEffect::Permit,
                mox::Effect::Forbid => ir::GrantEffect::Forbid,
            };
            let mut ir_entry = ir::GrantEntry::new(ir_effect, &entry.capability.text);
            if let Some(when_span) = entry.when {
                ir_entry = ir_entry.when(when_text(source, when_span));
                check_when_syntax(source, when_span, diags);
                let class = capability_classes
                    .get(entry.capability.text.as_str())
                    .cloned()
                    .flatten();
                pending.push(pending_condition(file, source, when_span, class));
            }
            for obligation in &entry.obligations {
                ir_entry = ir_entry.obligation(&obligation.text);
            }
            if declared && ir_effect == ir::GrantEffect::Permit {
                permits.push((entry.capability.text.clone(), entry.capability.span));
            }
            ir_delegation = ir_delegation.entry(ir_entry);
        }
        def = def.delegation(ir_delegation);
        delegations.push(DelegationRecord {
            name: delegation.name.text.clone(),
            from: delegation.from.text.clone(),
            from_span: delegation.from.span,
            to: delegation.to.text.clone(),
            to_span: delegation.to.span,
            purpose: delegation
                .purpose
                .as_ref()
                .map(|name| (name.text.clone(), name.span)),
            permits,
        });
    }

    for constraint in &decl.never_both {
        if constraint.capabilities.len() != 2 {
            diags.push(Diagnostic::error(
                "never_both expects exactly two capabilities",
                Some(constraint.span),
            ));
        }
        for name in &constraint.capabilities {
            if !capabilities.contains(name.text.as_str()) {
                diags.push(Diagnostic::error(
                    format!("grant names unknown capability `{}`", name.text),
                    Some(name.span),
                ));
            }
        }
        let names: Vec<String> = constraint
            .capabilities
            .iter()
            .map(|name| name.text.clone())
            .collect();
        def = def.never_both(ir::NeverBothDef {
            capabilities: names.clone(),
        });
        never_both.push((names, constraint.span));
    }

    (
        ActorsRecord {
            file: file.to_string(),
            def,
            actor_names,
            actor_kinds,
            parents,
            purposes,
            forbids,
            never_both,
            delegations,
        },
        pending,
    )
}

/// The actor → parent map of one block (only declared parents are edges).
fn actor_parents(record: &ActorsRecord) -> HashMap<&str, &str> {
    record
        .parents
        .iter()
        .filter_map(|(name, parent)| parent.as_deref().map(|parent| (name.as_str(), parent)))
        .collect()
}

/// Detects cycles in each block's actor `extends` graph, reporting the first
/// cycle per block (span on the actor that closes it). Each diagnostic is
/// tagged with the block's own file. Returns `true` when any cycle was
/// found; callers skip the ancestor-walking passes.
fn detect_actor_cycles(records: &[ActorsRecord], diags: &mut Vec<(String, Diagnostic)>) -> bool {
    let mut cyclic = false;
    for record in records {
        let index = actor_parents(record);
        let spans: HashMap<&str, Span> = record
            .actor_names
            .iter()
            .map(|(name, span)| (name.as_str(), *span))
            .collect();
        let mut state: HashMap<&str, u8> = HashMap::new();
        for (name, span) in &record.actor_names {
            let mut path: Vec<&str> = Vec::new();
            if let Some(cycle) = visit_actor(name.as_str(), &index, &mut state, &mut path) {
                let span = spans.get(cycle[0]).copied().unwrap_or(*span);
                diags.push((
                    record.file.clone(),
                    Diagnostic::error(
                        format!("actor inheritance cycle: {}", cycle.join(" -> ")),
                        Some(span),
                    ),
                ));
                cyclic = true;
                break;
            }
        }
    }
    cyclic
}

/// Iterative-depth-first visit over `extends` edges; returns the cycle path
/// when a back edge is found. `state`: absent = unvisited, `1` = on the
/// current path, `2` = done.
fn visit_actor<'a>(
    actor: &'a str,
    index: &HashMap<&'a str, &'a str>,
    state: &mut HashMap<&'a str, u8>,
    path: &mut Vec<&'a str>,
) -> Option<Vec<&'a str>> {
    match state.get(actor) {
        Some(1) => {
            let start = path.iter().position(|name| *name == actor).unwrap_or(0);
            let mut cycle: Vec<&str> = path[start..].to_vec();
            cycle.push(actor);
            return Some(cycle);
        }
        Some(_) => return None,
        None => {}
    }
    state.insert(actor, 1);
    path.push(actor);
    if let Some(parent) = index.get(actor) {
        if let Some(cycle) = visit_actor(parent, index, state, path) {
            return Some(cycle);
        }
    }
    path.pop();
    state.insert(actor, 2);
    None
}

/// The actor's own name plus every transitive `extends` ancestor
/// (cycle-guarded).
fn actor_lineage<'a>(parents: &HashMap<&'a str, &'a str>, actor: &'a str) -> HashSet<&'a str> {
    let mut lineage: HashSet<&str> = HashSet::new();
    let mut current = Some(actor);
    while let Some(name) = current {
        if !lineage.insert(name) {
            break;
        }
        current = parents.get(name).copied();
    }
    lineage
}

/// The permit capabilities reaching one lineage through grants alone: every
/// `permit` entry of every grant naming a lineage member. Cedar entries are
/// opaque and never contribute.
fn effective_grant_permits<'a>(
    record: &'a ActorsRecord,
    lineage: &HashSet<&'a str>,
) -> HashSet<&'a str> {
    record
        .def
        .grants
        .iter()
        .filter(|grant| lineage.contains(grant.actor.as_str()))
        .flat_map(|grant| grant.entries.iter())
        .filter(|entry| entry.effect == ir::GrantEffect::Permit && entry.cedar.is_none())
        .map(|entry| entry.capability.as_str())
        .collect()
}

/// The effective permit set of one actor: grant permits (see
/// [`effective_grant_permits`]) plus the permits delegated to any lineage
/// member. Only run on acyclic blocks (the lineage walk is cycle-guarded
/// regardless).
fn effective_permits<'a>(record: &'a ActorsRecord, lineage: &HashSet<&'a str>) -> HashSet<&'a str> {
    let mut permits = effective_grant_permits(record, lineage);
    permits.extend(
        record
            .delegations
            .iter()
            .filter(|delegation| lineage.contains(delegation.to.as_str()))
            .flat_map(|delegation| {
                delegation
                    .permits
                    .iter()
                    .map(|(capability, _)| capability.as_str())
            }),
    );
    permits
}

/// Separation of duty, per block: no actor's effective permit set may
/// contain both capabilities of the block's `never_both` constraint. One
/// error per (actor, pair), actors in declaration order.
fn validate_actor_never_both(records: &[ActorsRecord], diags: &mut Vec<(String, Diagnostic)>) {
    for record in records {
        let parents = actor_parents(record);
        for (pair, span) in &record.never_both {
            let [a, b] = pair.as_slice() else {
                continue; // malformed shape; already reported during lowering
            };
            for (name, _) in &record.actor_names {
                let lineage = actor_lineage(&parents, name.as_str());
                let effective = effective_permits(record, &lineage);
                if effective.contains(a.as_str()) && effective.contains(b.as_str()) {
                    diags.push((
                        record.file.clone(),
                        Diagnostic::error(
                            format!("actor `{name}` is granted both `{a}` and `{b}` (never_both)"),
                            Some(*span),
                        ),
                    ));
                }
            }
        }
    }
}

/// Self-narrowing, per block: a `forbid` entry whose capability is already
/// in the actor's effective permit set (own or inherited) can never fire —
/// warn on the deny entry's capability name.
fn validate_actor_self_narrowing(records: &[ActorsRecord], diags: &mut Vec<(String, Diagnostic)>) {
    for record in records {
        let parents = actor_parents(record);
        for (actor, capability, span) in &record.forbids {
            let lineage = actor_lineage(&parents, actor.as_str());
            if effective_permits(record, &lineage).contains(capability.as_str()) {
                diags.push((
                    record.file.clone(),
                    Diagnostic::warning(
                        format!(
                            "actor `{actor}` forbids `{capability}` but inherits or declares a permit for it"
                        ),
                        Some(*span),
                    ),
                ));
            }
        }
    }
}

/// The set of actor names across the whole union, in order of first
/// appearance.
fn union_actor_names(records: &[ActorsRecord]) -> Vec<&str> {
    let mut names: Vec<&str> = Vec::new();
    for record in records {
        for (name, _) in &record.actor_names {
            if !names.contains(&name.as_str()) {
                names.push(name.as_str());
            }
        }
    }
    names
}

/// Whether any block of the union effectively grants `actor` the capability
/// (same-named actors pool their permits in the union).
fn union_grants(records: &[ActorsRecord], actor: &str, capability: &str) -> bool {
    records.iter().any(|record| {
        let parents = actor_parents(record);
        let lineage = actor_lineage(&parents, actor);
        effective_permits(record, &lineage).contains(capability)
    })
}

/// Separation of duty over the union: same-named actors pool their permits
/// across blocks, so a permit granted by an imported domain's inline block
/// satisfies a `never_both` declared in the actor file (and vice versa). One
/// error per (actor, pair), constraints in union order, actors in
/// first-appearance order. Violations a domain already reported on its own
/// are deduplicated by the caller (identical file, message, and span).
fn validate_actor_never_both_union(
    records: &[ActorsRecord],
    diags: &mut Vec<(String, Diagnostic)>,
) {
    let names = union_actor_names(records);
    for record in records {
        for (pair, span) in &record.never_both {
            let [a, b] = pair.as_slice() else {
                continue; // malformed shape; already reported during lowering
            };
            for name in names.iter().copied() {
                if union_grants(records, name, a) && union_grants(records, name, b) {
                    diags.push((
                        record.file.clone(),
                        Diagnostic::error(
                            format!("actor `{name}` is granted both `{a}` and `{b}` (never_both)"),
                            Some(*span),
                        ),
                    ));
                }
            }
        }
    }
}

/// Self-narrowing over the union: a `forbid` entry whose capability any
/// block of the union grants the actor can never fire.
fn validate_actor_self_narrowing_union(
    records: &[ActorsRecord],
    diags: &mut Vec<(String, Diagnostic)>,
) {
    for record in records {
        for (actor, capability, span) in &record.forbids {
            if union_grants(records, actor, capability) {
                diags.push((
                    record.file.clone(),
                    Diagnostic::warning(
                        format!(
                            "actor `{actor}` forbids `{capability}` but inherits or declares a permit for it"
                        ),
                        Some(*span),
                    ),
                ));
            }
        }
    }
}

/// The kind `record` declares for `actor`, if any.
fn declared_kind(record: &ActorsRecord, actor: &str) -> Option<ir::ActorKind> {
    record
        .actor_kinds
        .iter()
        .find_map(|(name, kind)| (name.as_str() == actor).then_some(*kind).flatten())
}

/// The kind the union declares for `actor` (first block wins).
fn union_declared_kind(records: &[ActorsRecord], actor: &str) -> Option<ir::ActorKind> {
    records
        .iter()
        .find_map(|record| declared_kind(record, actor))
}

/// The actor → parent map across a union: the first declared `extends` edge
/// per actor name wins.
fn union_parents(records: &[ActorsRecord]) -> HashMap<&str, &str> {
    let mut map = HashMap::new();
    for record in records {
        for (name, parent) in &record.parents {
            if let Some(parent) = parent {
                map.entry(name.as_str()).or_insert(parent.as_str());
            }
        }
    }
    map
}

/// The effective kind of one actor: the declared kind of the actor itself
/// or, walking its `extends` chain (cycle-guarded), the nearest ancestor
/// with a declared kind; `Human` when no lineage member declares one.
fn lineage_kind(
    actor: &str,
    parents: &HashMap<&str, &str>,
    declared: &dyn Fn(&str) -> Option<ir::ActorKind>,
) -> ir::ActorKind {
    let mut current = Some(actor);
    let mut seen = HashSet::new();
    while let Some(name) = current {
        if !seen.insert(name) {
            break;
        }
        if let Some(kind) = declared(name) {
            return kind;
        }
        current = parents.get(name).copied();
    }
    ir::ActorKind::Human
}

fn delegation_unknown_actor(name: &str, actor: &str) -> String {
    format!("delegation `{name}` names unknown actor `{actor}`")
}

fn delegation_not_agent(name: &str, actor: &str) -> String {
    format!("delegation `{name}` targets actor `{actor}`, which is not an agent")
}

fn delegation_unbacked_permit(name: &str, capability: &str, actor: &str) -> String {
    format!(
        "delegation `{name}` delegates `{capability}` to actor `{actor}`, \
         which has no effective permit for it"
    )
}

fn delegation_unknown_purpose(name: &str, purpose: &str) -> String {
    format!("delegation `{name}` names unknown purpose `{purpose}`")
}

/// The union namespace of declared purpose names across every block (purpose
/// declarations pool across the whole policy set; only same-block duplicates
/// error at lowering time).
fn union_purposes(records: &[ActorsRecord]) -> HashSet<&str> {
    records
        .iter()
        .flat_map(|record| record.purposes.iter().map(|(name, _)| name.as_str()))
        .collect()
}

/// Delegation validation, per block: endpoints must name declared actors,
/// the target's effective kind must be `agent`, and every `permit` entry
/// must delegate a capability the target already holds as an effective
/// grant permit — other delegations are never authority sources. A declared
/// `purpose` must name a purpose of the union namespace (checked after the
/// endpoint gates, mirroring the other checks' cascade suppression).
/// Runs only on acyclic blocks (callers skip it alongside the other
/// ancestor-walking passes).
fn validate_actor_delegations(records: &[ActorsRecord], diags: &mut Vec<(String, Diagnostic)>) {
    let purposes = union_purposes(records);
    for record in records {
        let parents = actor_parents(record);
        for delegation in &record.delegations {
            let from_known = record
                .actor_names
                .iter()
                .any(|(name, _)| *name == delegation.from);
            let to_known = record
                .actor_names
                .iter()
                .any(|(name, _)| *name == delegation.to);
            if !from_known {
                diags.push((
                    record.file.clone(),
                    Diagnostic::error(
                        delegation_unknown_actor(&delegation.name, &delegation.from),
                        Some(delegation.from_span),
                    ),
                ));
            }
            if !to_known {
                diags.push((
                    record.file.clone(),
                    Diagnostic::error(
                        delegation_unknown_actor(&delegation.name, &delegation.to),
                        Some(delegation.to_span),
                    ),
                ));
                continue;
            }
            let kind = lineage_kind(&delegation.to, &parents, &|name| {
                declared_kind(record, name)
            });
            if kind != ir::ActorKind::Agent {
                diags.push((
                    record.file.clone(),
                    Diagnostic::error(
                        delegation_not_agent(&delegation.name, &delegation.to),
                        Some(delegation.to_span),
                    ),
                ));
            }
            let lineage = actor_lineage(&parents, &delegation.to);
            for (capability, span) in &delegation.permits {
                if !effective_grant_permits(record, &lineage).contains(capability.as_str()) {
                    diags.push((
                        record.file.clone(),
                        Diagnostic::error(
                            delegation_unbacked_permit(
                                &delegation.name,
                                capability,
                                &delegation.to,
                            ),
                            Some(*span),
                        ),
                    ));
                }
            }
            if let Some((purpose, span)) = &delegation.purpose {
                if !purposes.contains(purpose.as_str()) {
                    diags.push((
                        record.file.clone(),
                        Diagnostic::error(
                            delegation_unknown_purpose(&delegation.name, purpose),
                            Some(*span),
                        ),
                    ));
                }
            }
        }
    }
}

/// Whether any block of the union grants `actor` the capability via grants
/// alone — delegations are never authority sources.
fn union_grant_permits(
    records: &[ActorsRecord],
    parents: &HashMap<&str, &str>,
    actor: &str,
    capability: &str,
) -> bool {
    let lineage = actor_lineage(parents, actor);
    records
        .iter()
        .any(|record| effective_grant_permits(record, &lineage).contains(capability))
}

/// Delegation validation over the union: endpoints resolve against every
/// block's actors (same-named actors pool), kinds and grant permits are
/// computed union-wide, so a delegation in the `.actor` file may target an
/// agent declared in an imported domain's inline block. A declared
/// `purpose` must name a purpose declared in any block of the union (same
/// cross-file visibility). Violations a domain already reported on its own
/// are deduplicated by the caller (identical file, message, and span).
fn validate_actor_delegations_union(
    records: &[ActorsRecord],
    diags: &mut Vec<(String, Diagnostic)>,
) {
    let names = union_actor_names(records);
    let purposes = union_purposes(records);
    let parents = union_parents(records);
    for record in records {
        for delegation in &record.delegations {
            let from_known = names.contains(&delegation.from.as_str());
            let to_known = names.contains(&delegation.to.as_str());
            if !from_known {
                diags.push((
                    record.file.clone(),
                    Diagnostic::error(
                        delegation_unknown_actor(&delegation.name, &delegation.from),
                        Some(delegation.from_span),
                    ),
                ));
            }
            if !to_known {
                diags.push((
                    record.file.clone(),
                    Diagnostic::error(
                        delegation_unknown_actor(&delegation.name, &delegation.to),
                        Some(delegation.to_span),
                    ),
                ));
                continue;
            }
            let kind = lineage_kind(&delegation.to, &parents, &|name| {
                union_declared_kind(records, name)
            });
            if kind != ir::ActorKind::Agent {
                diags.push((
                    record.file.clone(),
                    Diagnostic::error(
                        delegation_not_agent(&delegation.name, &delegation.to),
                        Some(delegation.to_span),
                    ),
                ));
            }
            for (capability, span) in &delegation.permits {
                if !union_grant_permits(records, &parents, &delegation.to, capability) {
                    diags.push((
                        record.file.clone(),
                        Diagnostic::error(
                            delegation_unbacked_permit(
                                &delegation.name,
                                capability,
                                &delegation.to,
                            ),
                            Some(*span),
                        ),
                    ));
                }
            }
            if let Some((purpose, span)) = &delegation.purpose {
                if !purposes.contains(purpose.as_str()) {
                    diags.push((
                        record.file.clone(),
                        Diagnostic::error(
                            delegation_unknown_purpose(&delegation.name, purpose),
                            Some(*span),
                        ),
                    ));
                }
            }
        }
    }
}

/// One imported domain model handed to [`compile_actor_file`]: its lowered
/// model (when error-free), its AST, and its own diagnostics.
pub(crate) struct DomainUnit<'a> {
    /// The path the actor file's import named (and the file was provided
    /// under); tags the unit's diagnostics and spans.
    pub path: &'a str,
    /// The domain's full source text (condition and body slicing).
    pub source: &'a str,
    /// The lowered domain model, `None` when the domain has errors (its
    /// blocks then stay out of the union).
    pub model: Option<ir::Model>,
    /// The domain's parsed AST.
    pub ast: Option<&'a mox::Model>,
    /// The domain's own compile diagnostics (tagged with `path` here).
    pub diagnostics: &'a [Diagnostic],
}

/// Compiles an `.actor` file against its imported domain models:
///
/// 1. every import must name a provided domain (duplicates are fine — the
///    domain joins the union once);
/// 2. the union policy set is the actor file's blocks (source order)
///    followed by each domain's inline blocks (imports in first-appearance
///    order), each block keeping its own name (duplicate block names error
///    on the later occurrence);
/// 3. capability `on X` resolves against the combined namespace of all
///    domains;
/// 4. cycles, separation of duty, and self-narrowing are validated over the
///    union, and every `when` condition is type-checked against the resolved
///    capability class;
/// 5. any error (in any file) blocks the artifact.
///
/// Returns the actor model (or `None`) plus diagnostics from every file,
/// each tagged with its path. Duplicate diagnostics (a domain already
/// reported an issue on its own that the union passes re-find) are removed.
pub(crate) fn compile_actor_file(
    actor_path: &str,
    actor_source: &str,
    actor_ast: Option<&mox::ActorFile>,
    actor_diagnostics: &[Diagnostic],
    domains: &[DomainUnit<'_>],
) -> (Option<ir::ActorModel>, Vec<(String, Diagnostic)>) {
    let mut diags: Vec<(String, Diagnostic)> = Vec::new();
    for diagnostic in actor_diagnostics {
        diags.push((actor_path.to_string(), diagnostic.clone()));
    }

    // Import resolution: each import must name one of the provided domains.
    let imports = actor_ast
        .map(|ast| ast.imports.as_slice())
        .unwrap_or_default();
    for import in imports {
        if !domains.iter().any(|unit| unit.path == import.path) {
            diags.push((
                actor_path.to_string(),
                Diagnostic::error(
                    format!("imported file \"{}\" was not provided", import.path),
                    Some(import.span),
                ),
            ));
        }
    }

    // Domain diagnostics propagate under their own file, in import order.
    for unit in domains {
        for diagnostic in unit.diagnostics {
            diags.push((unit.path.to_string(), diagnostic.clone()));
        }
    }

    // The combined namespace of the usable domains (imported and
    // error-free), in first-appearance order. A domain that lowered
    // successfully had its `import schema` declarations validated, so they
    // contribute their nominal class names here.
    let packages: Vec<DomainPackage> = domains
        .iter()
        .filter_map(|unit| {
            let model = unit.model.as_ref()?;
            let ast = unit.ast?;
            let mut kinds: HashMap<String, TopKind> = HashMap::new();
            for decl in &ast.declarations {
                let kind = match decl {
                    mox::Decl::Class(_) => TopKind::Class,
                    mox::Decl::Interface(_) => TopKind::Interface,
                    mox::Decl::Enum(_) => TopKind::Enum,
                    mox::Decl::Datatype(_) => TopKind::Datatype,
                    mox::Decl::Vocabulary(_) => TopKind::Vocabulary,
                    mox::Decl::Actors(_) => TopKind::Actors,
                    mox::Decl::Annotation(_) => continue,
                    mox::Decl::ImportSchema(decl) => {
                        kinds.entry(imported_name(decl)).or_insert(TopKind::Class);
                        continue;
                    }
                };
                let name = decl
                    .name()
                    .map(|name| name.text.clone())
                    .unwrap_or_default();
                kinds.entry(name).or_insert(kind);
            }
            Some(DomainPackage {
                name: model.packages[0].name.clone(),
                kinds,
            })
        })
        .collect();
    let scope = Scope::Domains {
        packages: &packages,
    };

    // Lower the union policy set: actor-file blocks first, then each
    // domain's inline blocks.
    let mut records: Vec<ActorsRecord> = Vec::new();
    let mut pending: Vec<PendingCondition> = Vec::new();
    let mut block_names: Vec<(String, Span, String)> = Vec::new();
    {
        let mut lower_block = |decl: &mox::ActorsDecl, source: &str, file: &str| {
            let mut local = Vec::new();
            let (record, conditions) = lower_actors(decl, source, file, &scope, &mut local);
            diags.extend(local.into_iter().map(|d| (file.to_string(), d)));
            records.push(record);
            pending.extend(conditions);
            block_names.push((decl.name.text.clone(), decl.name.span, file.to_string()));
        };
        if let Some(ast) = actor_ast {
            for decl in &ast.blocks {
                lower_block(decl, actor_source, actor_path);
            }
        }
        for unit in domains {
            if unit.model.is_none() {
                continue;
            }
            if let Some(ast) = unit.ast {
                for decl in &ast.declarations {
                    if let mox::Decl::Actors(decl) = decl {
                        lower_block(decl, unit.source, unit.path);
                    }
                }
            }
        }
    }

    // Duplicate block names across the union: the later occurrence errors.
    for (index, (name, span, file)) in block_names.iter().enumerate() {
        if block_names[..index].iter().any(|(seen, _, _)| seen == name) {
            diags.push((
                file.clone(),
                Diagnostic::error(format!("duplicate actors block `{name}`"), Some(*span)),
            ));
        }
    }

    // Set-level validation over the union.
    let cyclic = detect_actor_cycles(&records, &mut diags);
    if !cyclic {
        validate_actor_never_both_union(&records, &mut diags);
        validate_actor_self_narrowing_union(&records, &mut diags);
        validate_actor_delegations_union(&records, &mut diags);
    }

    // Condition typing over the combined class universe.
    let mut context_model = ir::Model::new();
    for unit in domains {
        if let Some(model) = &unit.model {
            context_model
                .packages
                .extend(model.packages.iter().cloned());
        }
    }
    let context = TypeContext::from_model(&context_model);
    diags.extend(check_pending_conditions(&pending, &context));

    // Drop diagnostics the domains already reported on their own and the
    // union passes re-found (identical file, severity, message, span).
    let mut unique: Vec<(String, Diagnostic)> = Vec::new();
    for entry in diags {
        if !unique.contains(&entry) {
            unique.push(entry);
        }
    }
    let diags = unique;

    let blocked = diags.iter().any(|(_, diagnostic)| diagnostic.is_error());
    let model = (!blocked).then(|| {
        let mut actor_model = ir::ActorModel::new();
        for record in records {
            actor_model = actor_model.block(record.def);
        }
        actor_model
    });
    (model, diags)
}
