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
//! 7. **Skipped elements** (v1): `Rule`, `Report`, `ExternalRuleSource`,
//!    `Schema`, `Body`, `Corpus`, `Segment`, `MetaType`, and `Annotation`
//!    declarations. References to skipped kinds (e.g. an attribute typed by
//!    a rule) are sigil resolution errors (`E0106`), which surface as
//!    blocking rexlang diagnostics; the lowering fills a placeholder type
//!    for them (the model is discarded whenever any error exists).
//! 8. **`Function` / `LibraryFunction` → `Package::functions`.** User
//!    namespace functions lower to signature-only rexlang operations
//!    ([`ir::Operation`], wire contract rule 15): `inputs` → parameters
//!    (an input's cardinality becomes the parameter's `multiplicity`; the
//!    required `(1..1)` stays unset, the domain-operation convention), the
//!    `output` → the return type **and the output's cardinality → the
//!    operation's `return_multiplicity`** (the same `optional_multiplicity`
//!    rule: `(0..1)`/`(0..*)` and every other non-`(1..1)` cardinality lower
//!    verbatim so call sites and backends see the multi-valued return
//!    shape; `(1..1)` stays unset, the single-valued convention), and
//!    `definition` → the description.
//!    A `library function` lowers the same way (`is_array` parameters carry
//!    a `0..*` multiplicity; its return carries no cardinality in the sigil
//!    AST, so the return multiplicity always stays unset). A function without an output cannot become an
//!    operation — rexlang operations always return — so it is skipped with
//!    a **warning** diagnostic naming it. The declaration's complete Rune
//!    source is embedded **ends-trimmed** verbatim as the body target
//!    **`"rosetta"`**: sigil element spans consume trailing whitespace, so
//!    the lowering trims the slice's trailing whitespace (the same
//!    ends-trimmed convention as `when` conditions and `cedar` policy
//!    bodies) while interior indentation survives byte-exact. Builtin
//!    (`com.rosetta.model`) functions
//!    stay skipped — rule 6 keeps that namespace minimal. Functions are not
//!    types: they never join the `.mox`-side resolution namespace, so an
//!    attribute typed by a function remains an `E0106` sigil error. Sigil
//!    resolution already rejects duplicate function names within a kind.
//!
//!    **Statement bodies (the `"rune"` target, RUNE-BODIES stage 1).** A
//!    function carrying statements — a dispatch head, or any non-empty
//!    `conditions`/`post_conditions`/`shortcuts`/`operations` collection —
//!    additionally lowers them into one **`"rune"`** body beside the
//!    verbatim text: the dispatch head's `(attr: Enum->VALUE)` becomes the
//!    leading synthetic **pre** guard `attr == VALUE`, named after the
//!    head's value (dispatch lowers to branching inside one operation —
//!    per-branch operations are a non-goal); `condition`s become pre guards
//!    and `post-condition`s post guards, in written order; `alias`es become
//!    bindings; `set`/`add` statements become steps. Every expression slot
//!    is translated to **rex-expr source text** — the shared expression
//!    language's concrete syntax (`.` navigation, `==`, `&&`,
//!    `true`/`false`, `if c { t } else { e }`, `param => body`,
//!    `.size()/.sum()/.first()/.filter/.map`) — and the whole body is
//!    type-checked by `rex_expr::body::check_body` against the fully
//!    lowered namespace context (types lower before functions for exactly
//!    this): **pre guards** see the inputs plus the head's value bound as
//!    an enum-typed name (the machine-state precedent) and must be
//!    boolean; **binding *i*** sees the inputs and bindings `0..i`; and
//!    **steps** see the inputs and every binding, assigning the output
//!    slot or a binding through the `->` path (`set` unifies exactly with
//!    the slot type, `add` appends the element type of a list-valued
//!    slot); **post guards** additionally see the output slot by its
//!    written name — the sigil AST's rule that non-post conditions cannot
//!    see the output, post-conditions see everything (outputs carry names,
//!    and the `set` root in the grammar IS that name). A `LibraryFunction`
//!    has no statement fields in the sigil AST, so `"rune"` never appears
//!    for one. **Deferral contract:** when a statement fragment has no
//!    rex-expr form (quantifiers, deep navigation, `switch`, `default`,
//!    `then`, implicit closure bodies, ...) or the body fails its
//!    typecheck, NO `"rune"` body is attached and ONE warning names the
//!    function and the failing slot; the `"rosetta"` text and the
//!    operation itself are untouched, and the backend keeps its stub.
//!    Typecheck failure is a warning, never a compile error. The
//!    serialized body is `serde_json` of the `rex_expr::body::RuneBody`
//!    shape, byte-pinned by rex-expr's `tests/body.rs`; rex-ir never
//!    depends on rex-expr (rule 15).
//! 9. **Type references.** Sigil fills `TypeRef.resolved` element ids; the
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

use rex_expr::body::{
    BodyBinding, BodyDispatch, BodyGuard, BodyOutput, BodyStep, GuardPhase, RuneBody,
};
use rex_ir as ir;
use rex_syntax::ast as mox;
use rex_syntax::Span;
use sigil_model::expr::Expr as SigilExpr;
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

    let mut lowerer = Lowerer::new(
        &resolution,
        // The retained rosetta text per selected parsed file, keyed by the
        // file's name (the candidate key) — verbatim function bodies slice
        // it. Duplicate selection keeps the first occurrence, and so does
        // this map.
        selected
            .iter()
            .filter_map(|&index| {
                let candidate = &candidates[index];
                candidate
                    .file
                    .as_ref()
                    .map(|file| (file.name.as_str(), candidate.text.as_str()))
            })
            .collect(),
    );
    lowerer.register_builtins();
    // The resolved copies of the selected files (sigil fills the resolved
    // element ids on its own files, not the caller's).
    let selected_files: Vec<&ModelFile> = resolution.user_files().iter().collect();
    lowerer.collect_namespaces(&selected_files);
    lowerer.lower_elements(&selected_files);
    // Lowering warnings route like content diagnostics: rosetta-keyed,
    // never blocking.
    for (path, diagnostic) in lowerer.warnings.drain(..) {
        diagnostics.push((DiagnosticOrigin::Content { path }, diagnostic));
    }
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
    functions: Vec<ir::Operation>,
}

/// A builtin definition emitted into `com.rosetta.model` when referenced.
enum BuiltinDef {
    Datatype(ir::DatatypeDef),
    Enum(ir::EnumDef),
}

/// Lowers the resolved sigil model into synthetic rexlang packages.
struct Lowerer<'a> {
    resolution: &'a Resolution,
    /// The rosetta source text per selected file (keyed by the file's name,
    /// which is the candidate key) — verbatim function bodies slice it.
    texts: HashMap<&'a str, &'a str>,
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
    /// Warnings raised while lowering (currently: functions without an
    /// output), keyed by the rosetta file's name for diagnostic routing.
    warnings: Vec<(String, Diagnostic)>,
}

impl<'a> Lowerer<'a> {
    fn new(resolution: &'a Resolution, texts: HashMap<&'a str, &'a str>) -> Self {
        Self {
            resolution,
            texts,
            targets: HashMap::new(),
            order: Vec::new(),
            packages: HashMap::new(),
            namespace_kinds: HashMap::new(),
            builtins: Vec::new(),
            warnings: Vec::new(),
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
                // Skipped builtin kinds: builtin library functions and
                // annotations declare nothing referencable (user functions
                // lower per rule 8, but as operations, never as types).
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

    /// Lowers every selected element into its namespace's content. Types
    /// lower before functions: a function's statement body is type-checked
    /// against the fully lowered namespace context, so the types it names
    /// must already be in place regardless of declaration order.
    fn lower_elements(&mut self, files: &[&ModelFile]) {
        for functions_only in [false, true] {
            for file in files {
                let namespace = file.namespace.clone();
                for element in &file.elements {
                    let is_function = matches!(
                        element,
                        SemanticElement::Function(_) | SemanticElement::LibraryFunction(_)
                    );
                    if is_function != functions_only {
                        continue;
                    }
                    match element {
                        SemanticElement::Data(data) if data.is_choice => {
                            // A choice → interface; options and super types
                            // are skipped (rule 3).
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
                            let mut enum_def = ir::EnumDef::new(
                                enumeration.name.clone(),
                                lower_enum_values(element),
                            );
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
                        SemanticElement::Function(function) => {
                            if let Some(operation) = self.lower_function(file, function, &namespace)
                            {
                                self.packages
                                    .get_mut(&namespace)
                                    .expect("registered")
                                    .functions
                                    .push(operation);
                            }
                        }
                        SemanticElement::LibraryFunction(function) => {
                            if let Some(operation) =
                                self.lower_library_function(file, function, &namespace)
                            {
                                self.packages
                                    .get_mut(&namespace)
                                    .expect("registered")
                                    .functions
                                    .push(operation);
                            }
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
                        // Everything else (builtin elements, skipped kinds)
                        // is handled by the builtin registration or
                        // deliberately skipped (rule 7).
                        _ => {}
                    }
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

    /// A `Function` → signature-only [`ir::Operation`] (rule 8). Inputs map
    /// to parameters, the output to the return type, the definition to the
    /// description; the declaration's complete Rune source rides verbatim as
    /// the `"rosetta"` body target (dispatch, conditions, post-conditions,
    /// aliases, and set/add statements survive only inside that text). A
    /// statement-bearing function additionally lowers its statements into
    /// the checked `"rune"` body (rule 8's statement contract); a function
    /// whose statements cannot be translated or type-checked defers with a
    /// warning. A function without an output cannot become an operation —
    /// rexlang operations always return — and is skipped with a warning.
    fn lower_function(
        &mut self,
        file: &ModelFile,
        function: &sigil_model::Function,
        namespace: &str,
    ) -> Option<ir::Operation> {
        let Some(output) = function.output.as_ref() else {
            self.warnings.push((
                file.name.clone(),
                Diagnostic::warning(
                    format!(
                        "function '{}' declares no output and is skipped: rexlang operations \
                         always return",
                        function.name
                    ),
                    Some(Span::from(function.span.start..function.span.end)),
                )
                .with_help("give the function an `output` block to lower its signature"),
            ));
            return None;
        };
        let params = function
            .inputs
            .iter()
            .map(|input| ir::OperationParam {
                name: input.name.clone(),
                type_: self.map_type_ref(namespace, &input.type_ref),
                multiplicity: optional_multiplicity(&input.cardinality),
            })
            .collect();
        let mut operation = ir::Operation::new(
            function.name.clone(),
            self.map_type_ref(namespace, &output.type_ref),
            params,
        );
        // The output's cardinality lowers into the return multiplicity (the
        // `0..1`/`0..*` return shapes); the required `(1..1)` stays unset.
        operation.return_multiplicity = optional_multiplicity(&output.cardinality);
        operation.description = function.definition.clone();
        self.with_rosetta_body(file, function.span, &mut operation);
        self.with_rune_body(file, function, &mut operation, output);
        Some(operation)
    }

    /// A `LibraryFunction` → signature-only [`ir::Operation`] (rule 8); an
    /// `is_array` parameter carries a `0..*` multiplicity. Library functions
    /// always declare a return type, so nothing is skipped; the sigil AST
    /// carries no return cardinality for them, so the operation's return
    /// multiplicity always stays unset.
    fn lower_library_function(
        &mut self,
        file: &ModelFile,
        function: &sigil_model::LibraryFunction,
        namespace: &str,
    ) -> Option<ir::Operation> {
        let params = function
            .parameters
            .iter()
            .map(|parameter| ir::OperationParam {
                name: parameter.name.clone(),
                type_: self.map_type_ref(namespace, &parameter.type_ref),
                multiplicity: parameter.is_array.then_some(ir::Multiplicity {
                    lower: 0,
                    upper: ir::Upper::Unbounded,
                }),
            })
            .collect();
        let mut operation = ir::Operation::new(
            function.name.clone(),
            self.map_type_ref(namespace, &function.return_type),
            params,
        );
        operation.description = function.definition.clone();
        self.with_rosetta_body(file, function.span, &mut operation);
        Some(operation)
    }

    /// Attaches the declaration's verbatim Rune source — sliced out of the
    /// file's retained text by its span — as the `"rosetta"` body target,
    /// ends-trimmed (contract rule 8): sigil element spans consume trailing
    /// whitespace, so the raw slice typically ends with the declaration's
    /// trailing blank line and the artifact does not carry it. Interior
    /// indentation and whitespace are never reformatted, and the IR never
    /// parses the text (wire contract rule 15).
    fn with_rosetta_body(
        &self,
        file: &ModelFile,
        span: sigil_diag::Span,
        operation: &mut ir::Operation,
    ) {
        if let Some(text) = self.texts.get(file.name.as_str()) {
            operation.bodies.insert(
                "rosetta".to_string(),
                text[span.start..span.end].trim_end().to_string(),
            );
        }
    }

    /// Lowers a statement-bearing function's statements into the checked
    /// **`"rune"`** body target (rule 8's statement contract, RUNE-BODIES
    /// stage 1): the dispatch head becomes the leading synthetic pre guard
    /// `attr == VALUE`, `condition`s become pre guards (in written order),
    /// `post-condition`s become post guards, `alias`es become bindings, and
    /// `set`/`add` statements become steps — all in deterministic source
    /// order. A function with a dispatch head **or** any non-empty
    /// statement collection qualifies; a `LibraryFunction` carries no
    /// statements in the sigil AST, so `"rune"` never appears for one.
    ///
    /// Deferral contract: when a statement fragment has no rex-expr form or
    /// the body fails its typecheck, NO `"rune"` body is attached and ONE
    /// warning names the function and the failing slot. The verbatim
    /// `"rosetta"` text and the operation itself are untouched — the
    /// backend keeps its stub for the function.
    fn with_rune_body(
        &mut self,
        file: &ModelFile,
        function: &sigil_model::Function,
        operation: &mut ir::Operation,
        output: &sigil_model::Attribute,
    ) {
        let statement_bearing = function.dispatch.is_some()
            || !function.shortcuts.is_empty()
            || !function.conditions.is_empty()
            || !function.operations.is_empty()
            || !function.post_conditions.is_empty();
        if !statement_bearing {
            return;
        }
        match self.lower_rune_body(function, operation, output) {
            Ok(body) => {
                operation.bodies.insert(
                    "rune".to_string(),
                    serde_json::to_string(&body).expect("the rune body serializes"),
                );
            }
            Err(reason) => {
                self.warnings.push((
                    file.name.clone(),
                    Diagnostic::warning(
                        format!(
                            "function '{}' lowers without a \"rune\" body: {reason}",
                            function.name
                        ),
                        Some(Span::from(function.span.start..function.span.end)),
                    )
                    .with_help(
                        "the statement stays in the verbatim \"rosetta\" body; the backend \
                         keeps its stub (docs/RUNE-BODIES.md deferral contract)",
                    ),
                ));
            }
        }
    }

    /// Builds the [`RuneBody`] for one function: the AST statement
    /// collections in source order, every expression translated to
    /// rex-expr text, then the whole body type-checked against the lowered
    /// namespace context (`rex_expr::body::check_body`). The first failure
    /// — translation or typing — defers with a slot-tagged reason.
    fn lower_rune_body(
        &self,
        function: &sigil_model::Function,
        operation: &ir::Operation,
        output: &sigil_model::Attribute,
    ) -> Result<RuneBody, String> {
        let mut body = RuneBody::default();

        // The dispatch head leads: the synthetic pre guard pinning the
        // branch (`attr == VALUE`), named after the head's value.
        let mut dispatch_enum = None;
        if let Some(head) = &function.dispatch {
            let attribute = operation
                .params
                .iter()
                .find(|param| param.name == head.attribute)
                .ok_or_else(|| {
                    format!(
                        "dispatch head '{}' is not an input of the function",
                        head.attribute
                    )
                })?;
            match &attribute.type_ {
                ir::TypeRef::Enum { .. } => {
                    dispatch_enum = Some(attribute.type_.clone());
                    body.guards.push(BodyGuard {
                        expr: format!("{} == {}", head.attribute, head.value),
                        name: Some(head.value.clone()),
                        phase: GuardPhase::Pre,
                    });
                }
                _ => {
                    return Err(format!(
                        "dispatch head '{}' does not lower to an enum",
                        head.attribute
                    ));
                }
            }
        }

        // Conditions → pre guards, post-conditions → post guards, in
        // written order. (Sigil types may carry unnamed conditions; a
        // function condition keeps whatever name it has.)
        for (condition, phase) in function
            .conditions
            .iter()
            .map(|condition| (condition, GuardPhase::Pre))
            .chain(
                function
                    .post_conditions
                    .iter()
                    .map(|condition| (condition, GuardPhase::Post)),
            )
        {
            let slot = match &condition.name {
                Some(name) => format!("guard '{name}'"),
                None => "unnamed guard".to_string(),
            };
            body.guards.push(BodyGuard {
                expr: translate_slot(&condition.expression, &slot)?,
                name: condition.name.clone(),
                phase,
            });
        }

        // Aliases → bindings, in declaration order.
        for shortcut in &function.shortcuts {
            let slot = format!("binding '{}'", shortcut.name);
            body.bindings.push(BodyBinding {
                name: shortcut.name.clone(),
                expr: translate_slot(&shortcut.expression, &slot)?,
            });
        }

        // set/add statements → steps, in declaration order; the slot label
        // mirrors the checker's (the root and its `->` path).
        for statement in &function.operations {
            let kind = if statement.add { "add" } else { "set" };
            let slot = if statement.path.is_empty() {
                format!("{kind} '{}'", statement.assign_root)
            } else {
                let path = statement
                    .path
                    .iter()
                    .map(|segment| segment.feature.as_str())
                    .collect::<Vec<_>>()
                    .join("->");
                format!("{kind} '{}->{}'", statement.assign_root, path)
            };
            let step = if statement.add {
                BodyStep::Add {
                    root: statement.assign_root.clone(),
                    path: statement
                        .path
                        .iter()
                        .map(|segment| segment.feature.clone())
                        .collect(),
                    expr: translate_slot(&statement.expression, &slot)?,
                }
            } else {
                BodyStep::Set {
                    root: statement.assign_root.clone(),
                    path: statement
                        .path
                        .iter()
                        .map(|segment| segment.feature.clone())
                        .collect(),
                    expr: translate_slot(&statement.expression, &slot)?,
                }
            };
            body.steps.push(step);
        }

        // Typecheck the whole body against the lowered namespace context:
        // the inputs are the named roots, the output slot is reachable by
        // its written name (post guards and steps), and the dispatch value
        // binds in the pre-guard scope.
        let context = self.body_context();
        let output_ref = BodyOutput {
            name: &output.name,
            type_: &operation.return_type,
            multiplicity: operation.return_multiplicity.as_ref(),
        };
        let dispatch = match (&function.dispatch, dispatch_enum.as_ref()) {
            (Some(head), Some(enum_type)) => Some(BodyDispatch {
                input: &head.attribute,
                enum_type,
                value: &head.value,
            }),
            _ => None,
        };
        rex_expr::body::check_body(
            &context,
            &operation.params,
            Some(output_ref),
            dispatch,
            &body,
        )
        .map_err(|errors| {
            let first = &errors[0];
            format!("{}: {}", first.slot, first.message)
        })?;
        Ok(body)
    }

    /// The type universe a lowered body is checked against: every lowered
    /// user namespace (classes with their features, enums, datatypes,
    /// interfaces) plus the builtin definitions lowered content referenced.
    /// Assembled incrementally — never a synthesized `rex_ir::Model`.
    fn body_context(&self) -> rex_expr::DomainTypes {
        let mut context = rex_expr::DomainTypes::default();
        for namespace in &self.order {
            let Some(content) = self.packages.get(namespace) else {
                continue;
            };
            for class in &content.classes {
                context.insert_class(
                    namespace,
                    &class.name,
                    class.extends.clone(),
                    class.features.clone(),
                    class.operations.clone(),
                );
            }
            for enum_ in &content.enums {
                context.insert_enum(
                    namespace,
                    &enum_.name,
                    enum_
                        .literals
                        .iter()
                        .map(|literal| literal.name.clone())
                        .collect(),
                );
            }
            for datatype in &content.datatypes {
                context.insert_named(namespace, &datatype.name, rex_expr::NamedKind::Datatype);
            }
            for interface in &content.interfaces {
                context.insert_named(namespace, &interface.name, rex_expr::NamedKind::Interface);
            }
        }
        for (name, definition) in &self.builtins {
            match definition {
                BuiltinDef::Datatype(_) => {
                    context.insert_named(BUILTIN_NAMESPACE, name, rex_expr::NamedKind::Datatype);
                }
                BuiltinDef::Enum(enum_def) => {
                    context.insert_enum(
                        BUILTIN_NAMESPACE,
                        name,
                        enum_def
                            .literals
                            .iter()
                            .map(|literal| literal.name.clone())
                            .collect(),
                    );
                }
            }
        }
        context
    }

    /// Assembles the synthetic packages: user namespaces in first-appearance
    /// order, then `com.rosetta.model` with only the definitions lowered
    /// content actually references. Failed namespaces are skipped (their
    /// content is dropped), so the builtin emission only sees references
    /// from surviving content. Namespace/package collisions with the
    /// declared `.mox` packages error naming both sources. The lowerer is
    /// consumed exactly once here, so each namespace's `NamespaceContent`
    /// drains by value instead of cloning its collections.
    fn finish(
        mut self,
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
            // The lowerer is consumed here, so each namespace's content
            // moves out of the map by value — no per-collection clones.
            let content = self.packages.remove(namespace).expect("registered");
            let mut package = ir::Package::new(namespace.clone());
            package.enums = content.enums;
            package.datatypes = content.datatypes;
            package.interfaces = content.interfaces;
            package.classes = content.classes;
            package.functions = content.functions;
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
            // Functions count too: a Rune function typed by a builtin
            // (e.g. a `zonedDateTime` input) must emit the referenced
            // definition, or the artifact carries a dangling type.
            for function in &package.functions {
                scan(&function.return_type, &mut referenced);
                for param in &function.params {
                    scan(&param.type_, &mut referenced);
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

/// A function input's cardinality as an [`ir::OperationParam`] multiplicity:
/// the required `(1..1)` stays unset (the domain-operation convention —
/// parameters are single-valued unless stated), anything else lowers
/// verbatim.
fn optional_multiplicity(cardinality: &sigil_model::Cardinality) -> Option<ir::Multiplicity> {
    let multiplicity = lower_cardinality(cardinality);
    (multiplicity != ir::Multiplicity::REQUIRED).then_some(multiplicity)
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

// --- the Rune statement-expression translator -------------------------------

/// Translates one statement fragment to rex-expr source text, tagging any
/// unmappable construct with the slot it occurred in.
fn translate_slot(expr: &SigilExpr, slot: &str) -> Result<String, String> {
    translate_expr(expr).map_err(|reason| format!("{slot}: {reason}"))
}

/// The rex-expr parse levels the translator emits into, loosest first. A
/// node is parenthesized when its level is below the context's minimum;
/// left-associative chains let the right operand bind one level tighter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum ExprLevel {
    LogicOr,
    LogicAnd,
    Comparison,
    Additive,
    Multiplicative,
    Postfix,
}

/// Translates a sigil statement expression into rex-expr source text —
/// the shared expression language's concrete syntax (`.` navigation,
/// `==`/`!=`, `&&`/`||`, `true`/`false`, `if c { t } else { e }`,
/// `param => body` lambdas, `.size()/.sum()/.first()/.filter/.map`
/// algebra). Constructs with no rex-expr form (quantifiers, deep
/// navigation, `switch`, ...) return a named reason: the deferral contract
/// keeps the statement in the verbatim `"rosetta"` body.
fn translate_expr(expr: &SigilExpr) -> Result<String, String> {
    let mut out = String::new();
    write_expr(expr, ExprLevel::LogicOr, &mut out)?;
    Ok(out)
}

/// Writes `expr`, parenthesizing when its level binds looser than `min`.
fn write_expr(expr: &SigilExpr, min: ExprLevel, out: &mut String) -> Result<(), String> {
    if level_of(expr) < min {
        out.push('(');
        write_bare(expr, out)?;
        out.push(')');
        return Ok(());
    }
    write_bare(expr, out)
}

/// The tightest level at which this node may appear bare.
fn level_of(expr: &SigilExpr) -> ExprLevel {
    match expr {
        SigilExpr::LogicalOperation {
            operator: sigil_model::expr::LogicalOperator::Or,
            ..
        } => ExprLevel::LogicOr,
        SigilExpr::LogicalOperation { .. } => ExprLevel::LogicAnd,
        SigilExpr::EqualityOperation { .. }
        | SigilExpr::ComparisonOperation { .. }
        | SigilExpr::ContainsExpression { .. } => ExprLevel::Comparison,
        SigilExpr::ArithmeticOperation {
            operator:
                sigil_model::expr::ArithmeticOperator::Add
                | sigil_model::expr::ArithmeticOperator::Subtract,
            ..
        } => ExprLevel::Additive,
        SigilExpr::ArithmeticOperation { .. } => ExprLevel::Multiplicative,
        SigilExpr::ConditionalExpression { .. } => ExprLevel::Postfix,
        _ => ExprLevel::Postfix,
    }
}

/// Writes `expr` without surrounding parentheses (its context already
/// allows its level).
#[allow(clippy::too_many_lines)]
fn write_bare(expr: &SigilExpr, out: &mut String) -> Result<(), String> {
    use sigil_model::expr::{
        ArithmeticOperator, ComparisonOperator, EqualityOperator, LogicalOperator,
    };
    match expr {
        SigilExpr::BooleanLiteral { value } => {
            out.push_str(if *value { "true" } else { "false" });
        }
        SigilExpr::StringLiteral { value } => {
            out.push('"');
            for character in value.chars() {
                match character {
                    '"' => out.push_str("\\\""),
                    '\\' => out.push_str("\\\\"),
                    other => out.push(other),
                }
            }
            out.push('"');
        }
        SigilExpr::IntLiteral { text } | SigilExpr::NumberLiteral { text } => {
            out.push_str(text);
        }
        SigilExpr::SymbolReference {
            symbol,
            explicit_arguments: false,
            ..
        } => {
            out.push_str(symbol);
        }
        SigilExpr::SymbolReference {
            symbol,
            explicit_arguments: true,
            raw_args,
        } => {
            out.push_str(symbol);
            out.push('(');
            for (index, arg) in raw_args.iter().enumerate() {
                if index > 0 {
                    out.push_str(", ");
                }
                write_expr(arg, ExprLevel::LogicOr, out)?;
            }
            out.push(')');
        }
        SigilExpr::FeatureCall {
            receiver,
            feature: Some(feature),
        } => {
            write_expr(receiver, ExprLevel::Postfix, out)?;
            out.push('.');
            out.push_str(feature);
        }
        SigilExpr::ArithmeticOperation {
            operator,
            left,
            right,
        } => {
            let (spelling, level) = match operator {
                ArithmeticOperator::Add => ("+", ExprLevel::Additive),
                ArithmeticOperator::Subtract => ("-", ExprLevel::Additive),
                ArithmeticOperator::Multiply => ("*", ExprLevel::Multiplicative),
                ArithmeticOperator::Divide => ("/", ExprLevel::Multiplicative),
            };
            write_operand(level, left, right, spelling, out)?;
        }
        SigilExpr::LogicalOperation {
            operator,
            left,
            right,
        } => {
            let (spelling, level) = match operator {
                LogicalOperator::And => ("&&", ExprLevel::LogicAnd),
                LogicalOperator::Or => ("||", ExprLevel::LogicOr),
            };
            write_operand(level, left, right, spelling, out)?;
        }
        SigilExpr::EqualityOperation {
            operator,
            card_mod: sigil_model::expr::CardinalityModifier::None,
            left,
            right,
        } => {
            let spelling = match operator {
                EqualityOperator::Eq => "==",
                EqualityOperator::NotEq => "!=",
            };
            write_operand(ExprLevel::Comparison, left, right, spelling, out)?;
        }
        SigilExpr::ComparisonOperation {
            operator,
            card_mod: sigil_model::expr::CardinalityModifier::None,
            left,
            right,
        } => {
            let spelling = match operator {
                ComparisonOperator::Ge => ">=",
                ComparisonOperator::Le => "<=",
                ComparisonOperator::Gt => ">",
                ComparisonOperator::Lt => "<",
            };
            write_operand(ExprLevel::Comparison, left, right, spelling, out)?;
        }
        SigilExpr::ConditionalExpression {
            if_,
            ifthen,
            elsethen,
            full: true,
        } => {
            // A rex-expr `if` is a primary in braces, so it never needs
            // parenthesizing; the branches are delimited by the braces too.
            out.push_str("if ");
            write_expr(if_, ExprLevel::LogicAnd, out)?;
            out.push_str(" { ");
            write_expr(ifthen, ExprLevel::LogicOr, out)?;
            out.push_str(" } else { ");
            write_expr(elsethen, ExprLevel::LogicOr, out)?;
            out.push_str(" }");
        }
        SigilExpr::CountOperation { argument } => {
            write_expr(argument, ExprLevel::Postfix, out)?;
            out.push_str(".size()");
        }
        SigilExpr::SumOperation { argument } => {
            write_expr(argument, ExprLevel::Postfix, out)?;
            out.push_str(".sum()");
        }
        SigilExpr::FirstOperation { argument } => {
            write_expr(argument, ExprLevel::Postfix, out)?;
            out.push_str(".first()");
        }
        SigilExpr::FilterOperation {
            argument,
            function: Some(function),
        } => {
            write_inline(argument, function, "filter", out)?;
        }
        SigilExpr::MapOperation {
            argument,
            function: Some(function),
        } => {
            write_inline(argument, function, "map", out)?;
        }
        other => return Err(format!("{} cannot be typed by rex-expr", fragment(other))),
    }
    Ok(())
}

/// One left-associative binary operator at `level`.
fn write_operand(
    level: ExprLevel,
    left: &SigilExpr,
    right: &SigilExpr,
    spelling: &str,
    out: &mut String,
) -> Result<(), String> {
    write_expr(left, level, out)?;
    out.push(' ');
    out.push_str(spelling);
    out.push(' ');
    write_expr(right, next_level(level), out)
}

/// The level one above `level` (the right operand of a left-associative
/// chain), saturating at the tightest.
fn next_level(level: ExprLevel) -> ExprLevel {
    match level {
        ExprLevel::LogicOr => ExprLevel::LogicAnd,
        ExprLevel::LogicAnd => ExprLevel::Comparison,
        ExprLevel::Comparison => ExprLevel::Additive,
        ExprLevel::Additive => ExprLevel::Multiplicative,
        ExprLevel::Multiplicative | ExprLevel::Postfix => ExprLevel::Postfix,
    }
}

/// A collection-algebra step with an explicit single-parameter inline
/// function: `arg.filter(p => body)` / `arg.map(p => body)`.
fn write_inline(
    argument: &SigilExpr,
    function: &sigil_model::expr::InlineFunction,
    name: &str,
    out: &mut String,
) -> Result<(), String> {
    let parameters = &function.parameters;
    let body = &function.body;
    let [parameter] = parameters.as_slice() else {
        return Err(format!(
            "the `{name}` inline function takes one parameter; rex-expr lambdas bind exactly \
             one"
        ));
    };
    write_expr(argument, ExprLevel::Postfix, out)?;
    out.push('.');
    out.push_str(name);
    out.push('(');
    out.push_str(parameter);
    out.push_str(" => ");
    write_expr(body, ExprLevel::LogicOr, out)?;
    out.push(')');
    Ok(())
}

/// A readable name for the statement fragments with no rex-expr form — the
/// deferral warning names the construct, never just "unsupported".
fn fragment(expr: &SigilExpr) -> &'static str {
    use sigil_model::expr::Expr as E;
    match expr {
        E::ImplicitVariable => "the implicit `item` variable",
        E::SymbolReference {
            explicit_arguments: true,
            ..
        } => "the argument-less form of this call",
        E::FeatureCall { feature: None, .. } => "the bare `->` projection",
        E::DeepFeatureCall { .. } => "the `->>` deep navigation",
        E::EqualityOperation { .. } => "the equality cardinality modifier",
        E::ComparisonOperation { .. } => "the comparison cardinality modifier",
        E::ContainsExpression { .. } => "the `contains` operator",
        E::DisjointExpression { .. } => "the `disjoint` operator",
        E::DefaultOperation { .. } => "the `default` operator",
        E::JoinOperation { .. } => "the `join` operator",
        E::ConditionalExpression { full: false, .. } => {
            "a conditional without `else` (rex-expr `if` requires both branches)"
        }
        E::OnlyExistsExpression { .. } => "the `only exists` quantifier",
        E::ExistsExpression { .. } => "the `exists` quantifier",
        E::AbsentExpression { .. } => "the `is absent` test",
        E::OnlyElement { .. } => "the `only-element` operation",
        E::AsKeyOperation { .. } => "the `as-key` operation",
        E::OneOfOperation { .. } => "the `one-of` operation",
        E::ChoiceOperation { .. } => "a `choice` expression",
        E::ToStringOperation { .. }
        | E::ToNumberOperation { .. }
        | E::ToIntOperation { .. }
        | E::ToTimeOperation { .. }
        | E::ToDateOperation { .. }
        | E::ToDateTimeOperation { .. }
        | E::ToZonedDateTimeOperation { .. }
        | E::ToEnumOperation { .. } => "a `to-*` conversion",
        E::SwitchOperation { .. } => "a `switch` expression",
        E::WithMetaOperation { .. } => "a `with-meta` expression",
        E::AsOperation { .. } => "an `as` cast",
        E::ThenOperation { .. } => "the `then` operator",
        E::FilterOperation { function: None, .. } => {
            "an implicit (bracket-less) `filter` body; rex-expr lambdas are explicit"
        }
        E::MapOperation { function: None, .. } => {
            "an implicit (bracket-less) `extract` body; rex-expr lambdas are explicit"
        }
        E::ReduceOperation { .. } => "a `reduce` expression",
        E::SortOperation { .. } => "a `sort` expression",
        E::MinOperation { .. } => "a `min` expression",
        E::MaxOperation { .. } => "a `max` expression",
        E::ListLiteral { .. } => "a list literal",
        E::ConstructorExpression { .. } => "a constructor expression",
        // The literal/name/call forms are written above; this arm is
        // unreachable in practice but keeps the match exhaustive.
        _ => "this expression form",
    }
}
