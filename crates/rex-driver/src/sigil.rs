//! Sigil lowering: `import sigil "<path>"` declarations → synthetic rexlang
//! packages.
//!
//! A `.mox` package may import a Rune DSL (`.rosetta`) file. The driver is
//! filesystem-free: the host (CLI, test, language server) provides rosetta
//! source texts through [`crate::SigilImports`], keyed by the
//! **(mox path, import path)** pair exactly like
//! [`crate::SchemaImports`](crate::SchemaImports). Besides the named file,
//! the host may provide additional candidate files (the CLI walks the named
//! file's directory for `*.rosetta` files); the driver selects the transitive
//! closure the named file actually needs, so extra candidates never leak into
//! the artifact.
//!
//! # Normative contract
//!
//! ## File selection
//!
//! 1. Every `ImportSchema` declaration with
//!    [`ImportKind::Sigil`](rex_syntax::ast::ImportKind::Sigil) names a
//!    rosetta file. Its text must be provided under
//!    `(mox path, decl.path)`; a missing entry is the error
//!    ``imported sigil '<path>' was not provided``.
//! 2. All provided entries of the compilation's mox files form the candidate
//!    pool; each is parsed once. Candidates that end up unselected are never
//!    diagnosed — an unrelated broken file next to the import must not block
//!    the model.
//! 3. The selected set is the namespace closure of the named files: start
//!    from the named files' namespaces; repeatedly add every pool file whose
//!    namespace is in the frontier (an `import a.b.*` contributes namespace
//!    `a.b`; an `import a.b.C` contributes `a.b`), and so on transitively. A
//!    file's own namespace is always in its frontier, so same-namespace
//!    siblings of selected files are included (rosetta files of one namespace
//!    reference each other unqualified). Duplicate selection (same namespace
//!    and same text — e.g. two `.mox` files importing the same rosetta
//!    project) keeps the first occurrence.
//! 4. The selected files go to `sigil_resolve::resolve` in one call (sigil
//!    prepends its builtins automatically). Parse and resolution diagnostics
//!    map to rexlang diagnostics tagged with the rosetta entry's key path;
//!    any ERROR severity fails the namespace carrying it: that namespace's
//!    package is dropped and every declaring file whose import targets it
//!    loses its model. An import whose rosetta file failed to parse names no
//!    namespace — only its declaring file's model drops. Both shapes are
//!    carried by the structured [`ImportOutcome`] (origins, failed
//!    namespaces, per-file blocking) the callers consume.
//!
//! ## Mapping
//!
//! 1. **Namespaces → synthetic packages.** One rexlang package per rosetta
//!    namespace, appearing in `Model.packages` after every declared `.mox`
//!    package; namespaces in first-appearance order across the compilation's
//!    import declarations (file order, then declaration order, then
//!    selection order). The same namespace across files merges into one
//!    package. A synthetic package name colliding with a declared `.mox`
//!    package name is an error naming both sources. A namespace is emitted
//!    even when every element in it is skipped (an empty synthetic package
//!    keeps the emission rule trivial); the builtin namespace is the one
//!    exception (rule 6).
//! 2. **`Data` (non-choice) → `ClassDef`.** Attributes → attribute features
//!    (the attribute `definition` → the feature description). `Cardinality`
//!    maps verbatim (`min` → lower, `Finite(n)`/`Unbounded` → upper);
//!    sigil's parser fills `(0..1)` for the implicit choice-option form and
//!    requires explicit cardinalities elsewhere, so no default is invented
//!    here. `super_type` → `ClassDef.extends`. `is_override` attributes
//!    lower as ordinary features. Conditions, annotation references, labels,
//!    rule references, and doc references are **skipped in v1**.
//! 3. **`Data` with `is_choice` → `InterfaceDef`** named by the choice
//!    (choice options do not survive: a rexlang interface declares no
//!    features, and a choice's `super_type` is skipped for the same reason —
//!    `InterfaceDef` has no extends). Data types whose `super_type` resolves
//!    to a choice reference the interface via `extends` (rexlang classes may
//!    extend interfaces).
//! 4. **`Enumeration` → `EnumDef`.** Values in declaration order with
//!    synthesized integer values `0..n-1` (rexlang enums require ints);
//!    `display` → the literal's `as` label; `definition` → description. An
//!    enum `super_type` is skipped (`EnumDef` has no extends).
//! 5. **`TypeAlias` → `DatatypeDef`** (`type X wraps <target>`). When the
//!    target maps to a rexlang primitive, the datatype wraps it
//!    (`platform`); otherwise the datatype is opaque. Type parameters are
//!    skipped in v1.
//! 6. **Builtin mapping.** The `com.rosetta.model` builtin namespace lowers
//!    to primitives: `boolean` → `boolean`, `string` → `String`, `int` →
//!    `int`, `number` → `double`, `date` → `date`. Every other builtin
//!    basicType/recordType/typeAlias without a rexlang primitive counterpart
//!    (`time`, `pattern`, the `dateTime`/`zonedDateTime` family, and
//!    `calculation`, which keeps wrapping `string`) lowers to an **opaque
//!    `DatatypeDef`** in the `com.rosetta.model` package so references
//!    resolve; the builtin `SerializationFormat` enum lowers like any
//!    enumeration. Builtin `library function` and `annotation` declarations
//!    are skipped. The `com.rosetta.model` package is emitted **only when
//!    something from it is actually referenced** by lowered content, keeping
//!    artifacts minimal.
//! 7. **Skipped elements** (v1): `Function`, `LibraryFunction`, `Rule`,
//!    `Report`, `ExternalRuleSource`, `Schema`, `Body`, `Corpus`,
//!    `Segment`, `MetaType`, and `Annotation` declarations. References to
//!    skipped kinds (e.g. an attribute typed by a rule) are sigil
//!    resolution errors (`E0106`), which surface as blocking rexlang
//!    diagnostics; the lowering fills a placeholder type for them (the
//!    model is discarded whenever any error exists).
//! 8. **Type references.** Sigil fills `TypeRef.resolved` element ids; the
//!    lowering maps each resolved element to the rexlang equivalent
//!    (`Class`/`Interface`/`Enum`/`Datatype`) qualified with the element's
//!    namespace package, or to the mapped primitive. Unresolved references
//!    are already sigil errors (`E0101`); no duplicate diagnostic is
//!    emitted. Only the **user** namespaces join the `.mox`-side resolution
//!    namespace: `com.rosetta.model` does not, so a `.mox` reference to
//!    `number` or `time` stays an unknown-type error (the coinciding
//!    `string`/`int`/`boolean`/`date` resolve as the rexlang primitives they
//!    always were).
//!
//! Everything lowered is ordinary `Model` content: downstream resolution
//! (the importing package's bare/qualified name rules see the synthetic
//! packages), `.actor` capability targets, `.ddd` designs, and backends work
//! on it unchanged.

use std::collections::{BTreeSet, HashMap, HashSet};

use rex_ir as ir;
use rex_syntax::ast as mox;
use rex_syntax::Span;
use sigil_model::{CardinalityMax, ModelFile, SemanticElement};
use sigil_resolve::{ElementId, Resolution};

use crate::diagnostic::Diagnostic;
use crate::lower::{DomainPackage, TopKind};

/// The builtin library namespace sigil implicitly wildcard-imports into
/// every model. Its elements map to rexlang primitives where counterparts
/// exist (rule 6); everything else it declares lowers to opaque datatypes in
/// the synthetic `com.rosetta.model` package, which is emitted only when
/// referenced.
const BUILTIN_NAMESPACE: &str = "com.rosetta.model";

/// Where an `import sigil` diagnostic belongs.
#[derive(Debug, Clone)]
pub(crate) enum DiagnosticOrigin {
    /// An import-level diagnostic concerning the `.mox` file at `path`
    /// (a missing provider entry, a namespace collision anchored here):
    /// callers route it into that file's own diagnostics, where its
    /// severity blocks that file's model.
    DeclaringFile { path: String },
    /// A content diagnostic raised by the rosetta file at this key path:
    /// rosetta-keyed (rendered against the retained rosetta texts, not the
    /// domain's), and a namespace failure when its severity is an error.
    Content { path: String },
}

impl DiagnosticOrigin {
    /// The diagnostic's tag: the declaring `.mox` path for import-level
    /// diagnostics, the rosetta key path for content diagnostics. The
    /// single-file and multi-file pipelines surface these tags unchanged.
    pub(crate) fn tag(&self) -> &str {
        match self {
            DiagnosticOrigin::DeclaringFile { path } => path,
            DiagnosticOrigin::Content { path } => path,
        }
    }
}

/// The structured outcome of a compilation's `import sigil` declarations:
/// what lowered, what failed, and where each diagnostic belongs. Callers
/// consume this instead of inferring ownership and routing at assembly
/// time.
pub(crate) struct ImportOutcome {
    /// The synthetic packages of the namespaces that lowered without
    /// errors, in emission order. Owned by [`ImportOutcome::owner`]: that
    /// `.mox` file's model carries them. Failed namespaces are already
    /// excluded, so a caller cannot attach their dropped content.
    pub(crate) packages: Vec<ir::Package>,
    /// The path of the `.mox` file whose model carries
    /// [`ImportOutcome::packages`]: the first file declaring an `import
    /// sigil` (the same first-declared ownership the package declarations
    /// follow). `None` when no file declares one.
    pub(crate) owner: Option<String>,
    /// The successfully lowered namespaces as resolution packages, so
    /// `.mox` declarations resolve bare and qualified references into the
    /// synthetic packages. Failed namespaces are already excluded.
    pub(crate) namespaces: Vec<DomainPackage>,
    /// Every diagnostic with its structured origin.
    pub(crate) diagnostics: Vec<(DiagnosticOrigin, Diagnostic)>,
    /// The namespaces whose rosetta content produced error diagnostics. A
    /// namespace's files share its fate: its package is dropped, and a
    /// declaring file's model drops when its import targets the namespace.
    pub(crate) failed_namespaces: BTreeSet<String>,
    /// The named imports whose rosetta file failed to parse, keyed
    /// `(mox path, import path)`. A parse failure names no namespace, so
    /// only the declaring file's model drops.
    pub(crate) failed_imports: BTreeSet<(String, String)>,
    /// The namespace each parsed import targets: `(mox path, namespace)`
    /// per named import, in declaration order. A declaring file's model
    /// depends on its imports' namespaces.
    pub(crate) dependencies: Vec<(String, String)>,
}

impl ImportOutcome {
    /// `true` when the `.mox` file at `mox_path` depends on failed sigil
    /// content and must drop its model: one of its own imports' rosetta
    /// files failed to parse, or one of its imports targets a namespace
    /// whose lowering failed. Files whose imports all succeeded keep their
    /// models even when other namespaces failed.
    pub(crate) fn blocks(&self, mox_path: &str) -> bool {
        self.failed_imports.iter().any(|(path, _)| path == mox_path)
            || self.dependencies.iter().any(|(path, namespace)| {
                path == mox_path && self.failed_namespaces.contains(namespace)
            })
    }
}

/// A parsed candidate of the pool: its mox path, its entry key (the tag for
/// its diagnostics), its text (duplicate recognition), and the sigil model
/// when it parsed.
struct Candidate {
    mox_path: String,
    key: String,
    text: String,
    file: Option<ModelFile>,
    diagnostics: Vec<(String, Diagnostic)>,
}

/// One lowered sigil element: what a resolved reference to it means in the
/// rexlang world.
#[derive(Debug, Clone)]
enum Target {
    /// Maps to a rexlang primitive (the `boolean`, `string`, `int`,
    /// `number`, and `date` builtins).
    Primitive(ir::PrimitiveType),
    /// A named definition in a synthetic package.
    Def {
        package: String,
        name: String,
        kind: TargetKind,
    },
}

/// The rexlang kind of a named synthetic definition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TargetKind {
    Class,
    Interface,
    Enum,
    Datatype,
}

impl Target {
    /// The resolved IR type reference for this target.
    fn to_ir(&self) -> ir::TypeRef {
        match self {
            Target::Primitive(primitive) => ir::TypeRef::Primitive(*primitive),
            Target::Def {
                package,
                name,
                kind,
            } => match kind {
                TargetKind::Class => ir::TypeRef::Class {
                    package: package.clone(),
                    name: name.clone(),
                },
                TargetKind::Interface => ir::TypeRef::Interface {
                    package: package.clone(),
                    name: name.clone(),
                },
                TargetKind::Enum => ir::TypeRef::Enum {
                    package: package.clone(),
                    name: name.clone(),
                },
                TargetKind::Datatype => ir::TypeRef::Datatype {
                    package: package.clone(),
                    name: name.clone(),
                },
            },
        }
    }
}

/// Lowers every `import sigil` declaration of the compilation into an
/// [`ImportOutcome`].
///
/// `files` are the compilation's `(mox path, parsed ast)` pairs in input
/// order; `declared` names every declared `.mox` package as
/// `(package name, declaring mox path)` for collision errors; `provided`
/// carries the rosetta texts (see [`crate::SigilImports`] — an empty map
/// leaves every declaration unsatisfied).
pub(crate) fn compile_sigil_imports(
    files: &[(&str, &mox::Model)],
    declared: &[(&str, &str)],
    provided: &crate::SigilImports,
) -> ImportOutcome {
    // The import declarations in compilation order; the first one anchors
    // import-level errors (namespace collisions) that have no tighter span,
    // and its file owns the synthetic packages.
    let mut decls: Vec<(&str, &mox::ImportSchemaDecl)> = Vec::new();
    for (path, ast) in files {
        for declaration in &ast.declarations {
            if let mox::Decl::ImportSchema(decl) = declaration {
                if decl.kind == mox::ImportKind::Sigil {
                    decls.push((*path, decl));
                }
            }
        }
    }
    let anchor = decls
        .first()
        .map(|(path, decl)| ((*path).to_string(), decl.span))
        .unwrap_or_else(|| (String::new(), Span::from(0..0)));
    let owner = decls.first().map(|(path, _)| (*path).to_string());

    // Parse the candidate pool once per (mox path, entry key). Only selected
    // candidates report diagnostics.
    let mox_paths: HashSet<&str> = files.iter().map(|(path, _)| *path).collect();
    let mut candidates: Vec<Candidate> = Vec::new();
    let mut seen: HashSet<(&str, &str)> = HashSet::new();
    for (mox_path, import_path, text) in provided.entries() {
        if !mox_paths.contains(mox_path.as_str()) || !seen.insert((mox_path, import_path)) {
            continue;
        }
        candidates.push(parse_candidate(mox_path, import_path, text));
    }

    // The named files: the entry keyed (mox path, decl.path) per declaration.
    // A named file reports its parse diagnostics even when nothing could be
    // parsed — it is what the import asked for. Parsed imports record the
    // namespace their declaring file depends on; unparseable ones record an
    // outright import failure (no namespace exists to attribute them to).
    let mut named: Vec<usize> = Vec::new();
    let mut diagnostics: Vec<(DiagnosticOrigin, Diagnostic)> = Vec::new();
    let mut failed_imports: BTreeSet<(String, String)> = BTreeSet::new();
    let mut dependencies: Vec<(String, String)> = Vec::new();
    for (path, decl) in &decls {
        match candidates
            .iter()
            .position(|candidate| candidate.mox_path == *path && candidate.key == decl.path)
        {
            Some(index) => {
                named.push(index);
                if let Some(file) = &candidates[index].file {
                    dependencies.push(((*path).to_string(), file.namespace.clone()));
                } else {
                    failed_imports.insert(((*path).to_string(), decl.path.clone()));
                    diagnostics.extend(candidates[index].diagnostics.iter().map(
                        |(_, diagnostic)| {
                            (
                                DiagnosticOrigin::Content {
                                    path: decl.path.clone(),
                                },
                                diagnostic.clone(),
                            )
                        },
                    ));
                }
            }
            None => diagnostics.push((
                DiagnosticOrigin::DeclaringFile {
                    path: (*path).to_string(),
                },
                not_provided(decl),
            )),
        }
    }

    // The namespace closure of the named files (see the module contract).
    let mut frontier: Vec<String> = Vec::new();
    let mut selected: Vec<usize> = Vec::new();
    let mut seen_files: HashSet<(String, String)> = HashSet::new();
    for &index in &named {
        select(
            index,
            &candidates,
            &mut frontier,
            &mut selected,
            &mut seen_files,
        );
    }
    let mut changed = true;
    while changed {
        changed = false;
        for index in 0..candidates.len() {
            if selected.contains(&index) {
                continue;
            }
            let Some(file) = &candidates[index].file else {
                continue;
            };
            if frontier.contains(&file.namespace)
                && select(
                    index,
                    &candidates,
                    &mut frontier,
                    &mut selected,
                    &mut seen_files,
                )
            {
                changed = true;
            }
        }
    }

    // Selected candidates report their parse diagnostics; an error fails
    // the candidate's namespace.
    let mut failed_namespaces: BTreeSet<String> = BTreeSet::new();
    for &index in &selected {
        let candidate = &candidates[index];
        let namespace = candidate.file.as_ref().map(|file| file.namespace.clone());
        for (_, diagnostic) in &candidate.diagnostics {
            if diagnostic.is_error() {
                failed_namespaces.extend(namespace.iter().cloned());
            }
            diagnostics.push((
                DiagnosticOrigin::Content {
                    path: candidate.key.clone(),
                },
                diagnostic.clone(),
            ));
        }
    }

    // Resolve the selected files in one call (sigil prepends builtins).
    let files_for_resolution: Vec<ModelFile> = selected
        .iter()
        .filter_map(|&index| candidates[index].file.clone())
        .collect();
    let resolution = sigil_resolve::resolve(files_for_resolution);
    for diagnostic in &resolution.diagnostics {
        let span = diagnostic.span.map(|span| Span::from(span.start..span.end));
        let rex = match diagnostic.severity {
            sigil_diag::Severity::Error => Diagnostic::error(diagnostic.message.clone(), span),
            sigil_diag::Severity::Warning => Diagnostic::warning(diagnostic.message.clone(), span),
        }
        .with_help(format!(
            "sigil {} in {} (at {})",
            diagnostic.code, diagnostic.file, diagnostic.path
        ));
        if rex.is_error() {
            // The diagnostic's file is the rosetta key: every selected
            // namespace carrying that key fails (the key is shared when
            // several mox files import the same path).
            for &index in &selected {
                if candidates[index].key == diagnostic.file {
                    if let Some(file) = &candidates[index].file {
                        failed_namespaces.insert(file.namespace.clone());
                    }
                }
            }
        }
        diagnostics.push((
            DiagnosticOrigin::Content {
                path: diagnostic.file.clone(),
            },
            rex,
        ));
    }

    let mut lowerer = Lowerer::new(&resolution);
    lowerer.register_builtins();
    // The resolved copies of the selected files (sigil fills the resolved
    // element ids on its own files, not the caller's).
    let selected_files: Vec<&ModelFile> = resolution.user_files().iter().collect();
    lowerer.collect_namespaces(&selected_files);
    lowerer.lower_elements(&selected_files);
    // The user namespaces double as resolution packages for the `.mox`
    // declarations (bare and qualified references into the synthetic
    // packages resolve through the union namespace). Failed namespaces stay
    // out: their content is dropped, so referencing declarations must not
    // resolve against it.
    let namespaces: Vec<DomainPackage> = lowerer
        .order
        .iter()
        .filter(|namespace| {
            *namespace != BUILTIN_NAMESPACE && !failed_namespaces.contains(*namespace)
        })
        .map(|namespace| DomainPackage {
            name: (*namespace).clone(),
            kinds: lowerer
                .namespace_kinds
                .get(namespace)
                .cloned()
                .unwrap_or_default(),
        })
        .collect();
    let packages = lowerer.finish(declared, &anchor, &failed_namespaces, &mut diagnostics);
    ImportOutcome {
        packages,
        owner,
        namespaces,
        diagnostics,
        failed_namespaces,
        failed_imports,
        dependencies,
    }
}

/// Adds one candidate to the selected set when its file parsed and its
/// (namespace, text) pair was not selected before; extends the namespace
/// frontier with its own namespace and everything it imports. Returns
/// whether the candidate was newly selected.
fn select(
    index: usize,
    candidates: &[Candidate],
    frontier: &mut Vec<String>,
    selected: &mut Vec<usize>,
    seen_files: &mut HashSet<(String, String)>,
) -> bool {
    let candidate = &candidates[index];
    let Some(file) = &candidate.file else {
        return false; // parse errors are reported once selected (below)
    };
    if !seen_files.insert((file.namespace.clone(), candidate.text.clone())) {
        return false; // duplicate of an already-selected file
    }
    if !frontier.contains(&file.namespace) {
        frontier.push(file.namespace.clone());
    }
    for namespace in imported_namespaces(file) {
        if !frontier.contains(&namespace) {
            frontier.push(namespace);
        }
    }
    selected.push(index);
    true
}

/// The `imported sigil '<path>' was not provided` error, mirroring the
/// schema-import wording. Content is provided by the host: the CLI reads the
/// rosetta files next to the model; embedded callers pass them through
/// `SigilImports`.
fn not_provided(decl: &mox::ImportSchemaDecl) -> Diagnostic {
    Diagnostic::error(
        format!("imported sigil '{}' was not provided", decl.path),
        Some(decl.span),
    )
    .with_help(
        "import sigil content is provided by the host: the CLI reads the .rosetta files \
         next to the model; embedded callers pass them through SigilImports",
    )
}

/// Parses one pool candidate, mapping sigil's syntax diagnostics (spans are
/// byte offsets into the rosetta text, like ours).
fn parse_candidate(mox_path: &str, key: &str, text: &str) -> Candidate {
    let source = sigil_diag::SourceFile::new(key.to_string(), text.to_string());
    let (unit, diags) = sigil_syntax::parse(&source);
    let diagnostics: Vec<(String, Diagnostic)> = diags
        .iter()
        .map(|diagnostic| {
            let span = Span::from(diagnostic.span.start..diagnostic.span.end);
            let rex = match diagnostic.severity {
                sigil_diag::Severity::Error => {
                    Diagnostic::error(diagnostic.message.clone(), Some(span))
                }
                sigil_diag::Severity::Warning => {
                    Diagnostic::warning(diagnostic.message.clone(), Some(span))
                }
            }
            .with_help(format!("sigil {}", diagnostic.code));
            (key.to_string(), rex)
        })
        .collect();
    let file = unit.as_ref().map(|unit| sigil_syntax::lower(key, unit));
    Candidate {
        mox_path: mox_path.to_string(),
        key: key.to_string(),
        text: text.to_string(),
        file,
        diagnostics,
    }
}

/// The namespaces a rosetta file imports: a wildcard `import a.b.*` and a
/// single-element `import a.b.C` both contribute namespace `a.b`.
fn imported_namespaces(file: &ModelFile) -> Vec<String> {
    file.imports
        .iter()
        .map(|import| {
            let trimmed = import.imported_namespace.trim_end_matches(".*");
            match trimmed.rsplit_once('.') {
                Some((namespace, _)) if !import.wildcard => namespace.to_string(),
                _ => trimmed.to_string(),
            }
        })
        .collect()
}

/// The rexlang primitive mapped to a builtin `com.rosetta.model` element
/// name, when one exists (the module contract's mapping table, rule 6).
fn builtin_primitive(name: &str) -> Option<ir::PrimitiveType> {
    match name {
        "boolean" => Some(ir::PrimitiveType::Boolean),
        "string" => Some(ir::PrimitiveType::String),
        "int" => Some(ir::PrimitiveType::Int),
        "number" => Some(ir::PrimitiveType::Double),
        "date" => Some(ir::PrimitiveType::Date),
        _ => None,
    }
}

/// The lowered definitions of one synthetic namespace.
#[derive(Default)]
struct NamespaceContent {
    enums: Vec<ir::EnumDef>,
    datatypes: Vec<ir::DatatypeDef>,
    interfaces: Vec<ir::InterfaceDef>,
    classes: Vec<ir::ClassDef>,
}

/// A builtin definition emitted into `com.rosetta.model` when referenced.
enum BuiltinDef {
    Datatype(ir::DatatypeDef),
    Enum(ir::EnumDef),
}

/// Lowers the resolved sigil model into synthetic rexlang packages.
struct Lowerer<'a> {
    resolution: &'a Resolution,
    /// What each resolved element id means in rexlang terms.
    targets: HashMap<usize, Target>,
    /// User namespaces in first-appearance order.
    order: Vec<String>,
    /// The lowered content per namespace.
    packages: HashMap<String, NamespaceContent>,
    /// The referencable kinds per namespace, for `.mox`-side resolution.
    namespace_kinds: HashMap<String, HashMap<String, TopKind>>,
    /// Builtin definitions by element name (emitted only when referenced).
    builtins: Vec<(String, BuiltinDef)>,
}

impl<'a> Lowerer<'a> {
    fn new(resolution: &'a Resolution) -> Self {
        Self {
            resolution,
            targets: HashMap::new(),
            order: Vec::new(),
            packages: HashMap::new(),
            namespace_kinds: HashMap::new(),
            builtins: Vec::new(),
        }
    }

    /// Registers what every builtin element means: mapped primitives and
    /// opaque datatypes/enums in `com.rosetta.model` (skipped kinds register
    /// nothing — references to them are sigil `E0106` errors).
    fn register_builtins(&mut self) {
        // The builtin prefix is always exactly two files (the sigil
        // embedding contract); their elements occupy the first flat ids.
        let builtin_count: usize = self.resolution.files[..2]
            .iter()
            .map(|file| file.elements.len())
            .sum();
        for id in 0..builtin_count {
            let element = self.resolution.element(ElementId(id));
            let name = element.name().to_string();
            let target = match element {
                SemanticElement::BasicType(_) => match builtin_primitive(&name) {
                    Some(primitive) => Target::Primitive(primitive),
                    None => self.builtin_datatype(&name, element),
                },
                SemanticElement::RecordType(_) => match builtin_primitive(&name) {
                    // The builtin `date` record type is rexlang's date
                    // primitive; the date/time family stays opaque.
                    Some(primitive) => Target::Primitive(primitive),
                    None => self.builtin_datatype(&name, element),
                },
                SemanticElement::TypeAlias(_) => match builtin_primitive(&name) {
                    // The builtin `int` alias is rexlang's int primitive;
                    // other aliases lower as datatypes (e.g. `calculation`
                    // keeps wrapping `string`).
                    Some(primitive) => Target::Primitive(primitive),
                    None => self.builtin_datatype(&name, element),
                },
                SemanticElement::Enumeration(_) => {
                    let mut enum_def = ir::EnumDef::new(name.clone(), lower_enum_values(element));
                    enum_def.description = element_definition(element);
                    self.builtins
                        .push((name.clone(), BuiltinDef::Enum(enum_def)));
                    Target::Def {
                        package: BUILTIN_NAMESPACE.to_string(),
                        name,
                        kind: TargetKind::Enum,
                    }
                }
                // Skipped kinds: library functions and annotations declare
                // nothing referencable.
                _ => continue,
            };
            self.targets.insert(id, target);
        }
    }

    /// The opaque datatype a non-primitive builtin basic type/record
    /// type/type alias lowers into, carrying its declaration's definition.
    fn builtin_datatype(&mut self, name: &str, element: &SemanticElement) -> Target {
        let mut datatype = ir::DatatypeDef::new(name, None);
        datatype.description = element_definition(element);
        // A type alias wrapping a primitive keeps the wrap (`calculation`
        // wraps `string`); basic and record types are fully opaque.
        if let SemanticElement::TypeAlias(alias) = element {
            if let Some(id) = alias.type_ref.resolved {
                if let Some(Target::Primitive(primitive)) = self.targets.get(&id) {
                    datatype.platform = Some(primitive.to_string());
                }
            }
        }
        self.builtins
            .push((name.to_string(), BuiltinDef::Datatype(datatype)));
        Target::Def {
            package: BUILTIN_NAMESPACE.to_string(),
            name: name.to_string(),
            kind: TargetKind::Datatype,
        }
    }

    /// Orders the user namespaces by first appearance and registers every
    /// referencable element's target (references may point forward).
    fn collect_namespaces(&mut self, files: &[&ModelFile]) {
        for file in files {
            if !self.order.contains(&file.namespace) {
                self.order.push(file.namespace.clone());
                self.packages.entry(file.namespace.clone()).or_default();
            }
            // (name, kind, flat id) per referencable element, collected
            // first so the kind table is not borrowed across id lookups.
            let entries: Vec<(String, TargetKind, usize)> = file
                .elements
                .iter()
                .filter_map(|element| {
                    let kind = match element {
                        SemanticElement::Data(data) if data.is_choice => TargetKind::Interface,
                        SemanticElement::Data(_) => TargetKind::Class,
                        SemanticElement::Enumeration(_) => TargetKind::Enum,
                        SemanticElement::TypeAlias(_) | SemanticElement::RecordType(_) => {
                            TargetKind::Datatype
                        }
                        // BasicTypes only exist in the builtin namespace; all
                        // other kinds are skipped in v1 and register nothing.
                        _ => return None,
                    };
                    Some((
                        element.name().to_string(),
                        kind,
                        self.flat_id(file, element.name()),
                    ))
                })
                .collect();
            let kinds = self
                .namespace_kinds
                .entry(file.namespace.clone())
                .or_default();
            for (name, kind, id) in entries {
                kinds.insert(
                    name.clone(),
                    match kind {
                        TargetKind::Interface => TopKind::Interface,
                        TargetKind::Class => TopKind::Class,
                        TargetKind::Enum => TopKind::Enum,
                        TargetKind::Datatype => TopKind::Datatype,
                    },
                );
                self.targets.insert(
                    id,
                    Target::Def {
                        package: file.namespace.clone(),
                        name,
                        kind,
                    },
                );
            }
        }
    }

    /// The flat element id of a named element of `file` (ids are assigned in
    /// file submission order, then declaration order).
    fn flat_id(&self, file: &ModelFile, name: &str) -> usize {
        let mut offset = 0;
        for candidate in &self.resolution.files {
            if candidate.name == file.name {
                for (index, element) in candidate.elements.iter().enumerate() {
                    if element.name() == name {
                        return offset + index;
                    }
                }
            }
            offset += candidate.elements.len();
        }
        unreachable!("element {name} of {} is registered", file.name)
    }

    /// Lowers every selected element into its namespace's content.
    fn lower_elements(&mut self, files: &[&ModelFile]) {
        for file in files {
            let namespace = file.namespace.clone();
            for element in &file.elements {
                match element {
                    SemanticElement::Data(data) if data.is_choice => {
                        // A choice → interface; options and super types are
                        // skipped (rule 3).
                        let interface = interface_def(data);
                        self.packages
                            .get_mut(&namespace)
                            .expect("registered")
                            .interfaces
                            .push(interface);
                    }
                    SemanticElement::Data(data) => {
                        let class = self.lower_class(data, &namespace);
                        self.packages
                            .get_mut(&namespace)
                            .expect("registered")
                            .classes
                            .push(class);
                    }
                    SemanticElement::Enumeration(enumeration) => {
                        let mut enum_def =
                            ir::EnumDef::new(enumeration.name.clone(), lower_enum_values(element));
                        enum_def.description = enumeration.definition.clone();
                        self.packages
                            .get_mut(&namespace)
                            .expect("registered")
                            .enums
                            .push(enum_def);
                    }
                    SemanticElement::TypeAlias(alias) => {
                        let datatype = self.lower_type_alias(alias);
                        self.packages
                            .get_mut(&namespace)
                            .expect("registered")
                            .datatypes
                            .push(datatype);
                    }
                    SemanticElement::BasicType(_) | SemanticElement::RecordType(_)
                        if namespace != BUILTIN_NAMESPACE =>
                    {
                        // User-level basic/record types lower to opaque
                        // datatypes so references resolve.
                        let mut datatype = ir::DatatypeDef::new(element.name(), None);
                        datatype.description = element_definition(element);
                        self.packages
                            .get_mut(&namespace)
                            .expect("registered")
                            .datatypes
                            .push(datatype);
                    }
                    // Everything else (builtin elements, skipped kinds) is
                    // handled by the builtin registration or deliberately
                    // skipped (rule 7).
                    _ => {}
                }
            }
        }
    }

    /// Maps one resolved sigil type reference to the rexlang equivalent.
    /// Unresolved references are already sigil errors; the placeholder type
    /// is discarded with the model.
    fn map_type_ref(&self, namespace: &str, type_ref: &sigil_model::TypeRef) -> ir::TypeRef {
        type_ref
            .resolved
            .map(ElementId)
            .and_then(|id| self.targets.get(&id.0).cloned())
            .map(|target| target.to_ir())
            .unwrap_or_else(|| placeholder(namespace, type_ref))
    }

    /// A non-choice `Data` → `ClassDef`: attributes become attribute
    /// features, the resolved super type becomes extends (a choice parent
    /// becomes an interface reference, which rexlang classes may extend).
    fn lower_class(&self, data: &sigil_model::Data, namespace: &str) -> ir::ClassDef {
        let extends = data
            .super_type
            .as_ref()
            .map(|super_type| vec![self.map_type_ref(namespace, super_type)])
            .unwrap_or_default();
        let features = data
            .attributes
            .iter()
            .map(|attribute| {
                let mut feature = ir::Feature::new(
                    attribute.name.clone(),
                    ir::FeatureKind::Attribute,
                    self.map_type_ref(namespace, &attribute.type_ref),
                    lower_cardinality(&attribute.cardinality),
                );
                feature.description = attribute.definition.clone();
                feature
            })
            .collect();
        let mut class = ir::ClassDef::new(data.name.clone(), extends, features);
        class.description = data.definition.clone();
        class
    }

    /// A `TypeAlias` → `DatatypeDef`, wrapping the target when it maps to a
    /// rexlang primitive and opaque otherwise (parameters are skipped).
    fn lower_type_alias(&self, alias: &sigil_model::TypeAlias) -> ir::DatatypeDef {
        let platform = alias
            .type_ref
            .resolved
            .map(ElementId)
            .and_then(|id| self.targets.get(&id.0))
            .and_then(|target| match target {
                Target::Primitive(primitive) => Some(primitive.to_string()),
                Target::Def { .. } => None,
            });
        let mut datatype = ir::DatatypeDef::new(alias.name.clone(), platform);
        datatype.description = alias.definition.clone();
        datatype
    }

    /// Assembles the synthetic packages: user namespaces in first-appearance
    /// order, then `com.rosetta.model` with only the definitions lowered
    /// content actually references. Failed namespaces are skipped (their
    /// content is dropped), so the builtin emission only sees references
    /// from surviving content. Namespace/package collisions with the
    /// declared `.mox` packages error naming both sources.
    fn finish(
        self,
        declared: &[(&str, &str)],
        anchor: &(String, Span),
        failed: &BTreeSet<String>,
        diagnostics: &mut Vec<(DiagnosticOrigin, Diagnostic)>,
    ) -> Vec<ir::Package> {
        let mut packages: Vec<ir::Package> = Vec::new();
        for namespace in &self.order {
            // Namespace/package collision: an error naming both sources —
            // the rosetta namespace and the `.mox` package of the same name.
            if let Some((_, mox_path)) = declared.iter().find(|(name, _)| name == namespace) {
                diagnostics.push((
                    DiagnosticOrigin::DeclaringFile {
                        path: anchor.0.clone(),
                    },
                    Diagnostic::error(
                        format!(
                            "imported sigil namespace '{namespace}' collides with the package \
                             declared by '{mox_path}'"
                        ),
                        Some(anchor.1),
                    )
                    .with_help(
                        "rename the `.mox` package or import a rosetta namespace with a \
                         distinct name",
                    ),
                ));
            }
            if namespace == BUILTIN_NAMESPACE || failed.contains(namespace) {
                continue;
            }
            let content = self.packages.get(namespace).expect("registered");
            let mut package = ir::Package::new(namespace.clone());
            package.enums = content.enums.clone();
            package.datatypes = content.datatypes.clone();
            package.interfaces = content.interfaces.clone();
            package.classes = content.classes.clone();
            packages.push(package);
        }
        // `com.rosetta.model` joins only when lowered content references it.
        let referenced = Self::referenced_builtins(&packages);
        if !referenced.is_empty() {
            if let Some((_, mox_path)) =
                declared.iter().find(|(name, _)| *name == BUILTIN_NAMESPACE)
            {
                diagnostics.push((
                    DiagnosticOrigin::DeclaringFile {
                        path: anchor.0.clone(),
                    },
                    Diagnostic::error(
                        format!(
                            "imported sigil namespace '{BUILTIN_NAMESPACE}' collides with the \
                             package declared by '{mox_path}'"
                        ),
                        Some(anchor.1),
                    )
                    .with_help(
                        "rename the `.mox` package or import a rosetta namespace with a \
                         distinct name",
                    ),
                ));
            }
            let mut package = ir::Package::new(BUILTIN_NAMESPACE);
            for (name, def) in &self.builtins {
                if !referenced.contains(name) {
                    continue;
                }
                match def {
                    BuiltinDef::Datatype(datatype) => package.datatypes.push(datatype.clone()),
                    BuiltinDef::Enum(enum_def) => package.enums.push(enum_def.clone()),
                }
            }
            packages.push(package);
        }
        packages
    }

    /// The builtin definition names referenced by `packages`. A fixed point
    /// is not needed: the emitted builtin definitions — opaque datatypes and
    /// a plain enum — reference nothing.
    fn referenced_builtins(packages: &[ir::Package]) -> HashSet<String> {
        fn scan(type_ref: &ir::TypeRef, referenced: &mut HashSet<String>) {
            if let ir::TypeRef::Datatype { package, name }
            | ir::TypeRef::Enum { package, name }
            | ir::TypeRef::Class { package, name }
            | ir::TypeRef::Interface { package, name } = type_ref
            {
                if package == BUILTIN_NAMESPACE {
                    referenced.insert(name.clone());
                }
            }
        }
        let mut referenced = HashSet::new();
        for package in packages {
            for class in &package.classes {
                for type_ref in &class.extends {
                    scan(type_ref, &mut referenced);
                }
                for feature in &class.features {
                    scan(&feature.type_, &mut referenced);
                }
            }
        }
        referenced
    }
}

/// The interface a choice lowers into (options and super types are skipped:
/// a rexlang interface declares no features and has no extends).
fn interface_def(data: &sigil_model::Data) -> ir::InterfaceDef {
    let mut interface = ir::InterfaceDef::new(data.name.clone());
    interface.description = data.definition.clone();
    interface
}

/// Sigil `Cardinality` → rexlang `Multiplicity`, verbatim. Sigil's parser
/// fills `(0..1)` for the implicit choice-option cardinality and requires
/// explicit cardinalities on regular attributes, so no default is applied
/// here.
fn lower_cardinality(cardinality: &sigil_model::Cardinality) -> ir::Multiplicity {
    ir::Multiplicity {
        lower: cardinality.min,
        upper: match cardinality.max {
            CardinalityMax::Finite(max) => ir::Upper::Finite(max),
            CardinalityMax::Unbounded => ir::Upper::Unbounded,
        },
    }
}

/// An enumeration's literals in declaration order with synthesized values
/// `0..n-1` (rexlang enums require integer literal values); `display`
/// becomes the literal's `as` label, `definition` its description.
fn lower_enum_values(element: &SemanticElement) -> Vec<ir::EnumLiteral> {
    let SemanticElement::Enumeration(enumeration) = element else {
        return Vec::new();
    };
    enumeration
        .values
        .iter()
        .enumerate()
        .map(|(index, value)| {
            let mut literal =
                ir::EnumLiteral::new(value.name.clone(), value.display.clone(), index as i64);
            literal.description = value.definition.clone();
            literal
        })
        .collect()
}

/// The definition text of the element kinds that lower to opaque datatypes.
fn element_definition(element: &SemanticElement) -> Option<String> {
    match element {
        SemanticElement::BasicType(basic) => basic.definition.clone(),
        SemanticElement::RecordType(record) => record.definition.clone(),
        SemanticElement::TypeAlias(alias) => alias.definition.clone(),
        SemanticElement::Enumeration(enumeration) => enumeration.definition.clone(),
        _ => None,
    }
}

/// The placeholder type for references sigil could not resolve (the sigil
/// error blocks the artifact, so the exact value is never observed).
fn placeholder(namespace: &str, type_ref: &sigil_model::TypeRef) -> ir::TypeRef {
    ir::TypeRef::Class {
        package: namespace.to_string(),
        name: type_ref
            .name
            .rsplit('.')
            .next()
            .unwrap_or_default()
            .to_string(),
    }
}
