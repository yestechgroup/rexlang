//! The lowering pipelines: the single-file compile, the all-or-nothing
//! multi-file compile, the per-file union compile behind the artifact
//! surfaces, and the `.actor` engine that originated the per-surface
//! engine shape.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};

use rex_expr::TypeContext;
use rex_ir as ir;
use rex_syntax::ast as mox;
use rex_syntax::Span;

use crate::diagnostic::Diagnostic;
use crate::pipeline;

use super::actors::{
    detect_actor_cycles, lower_actors, validate_actor_delegations,
    validate_actor_delegations_union, validate_actor_never_both, validate_actor_never_both_union,
    validate_actor_obligations, validate_actor_self_narrowing, validate_actor_self_narrowing_union,
    validate_capability_coverage,
};
use super::classes::{
    detect_inheritance_cycles, lower_class, lower_datatype, lower_enum, lower_interface,
    validate_opposites,
};
use super::conditions::check_pending_conditions;
use super::imports::{import_class_def, SchemaImport};
use super::vocabulary::lower_vocabulary;
use super::vocabulary::primitive_facet_type;
use super::*;

/// The per-target backends known to the driver, in warning-message order.
/// `expr` is the Tier-2 pseudo-target holding the neutral expression
/// language (the Rust backend lowers it; see `docs/EXPRESSIONS.md`).
pub(crate) const KNOWN_TARGETS: [&str; 4] = ["rust", "csharp", "java", "expr"];

/// Lowers target-tagged bodies into a verbatim `target -> code` map,
/// warning on unknown target names, erroring on duplicates, and erroring on
/// the `rust` + `expr` conflict (one operation, one body language).
pub(crate) fn lower_target_bodies(
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

/// The multi-file outcome: the model (or `None` when blocked) plus every
/// file's diagnostics, each tagged with its file path, and the tagged
/// diagnostics of the compilation's `import sigil` declarations.
pub(crate) type MultiOutcome = (
    Option<ir::Model>,
    Vec<(String, Diagnostic)>,
    Vec<(String, Diagnostic)>,
);

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
    imports: &crate::DomainImports,
) -> MultiOutcome {
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
    let mut declared_packages: Vec<(String, &str)> = Vec::new();
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
        declared_packages.push((name.clone(), file.path));
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
        let file_schemas = collect_schema_imports(
            file.path,
            ast,
            &imports.schemas,
            &kinds,
            &mut buckets[index],
        );
        for import in &file_schemas {
            kinds.insert(import.name.clone(), TopKind::Class);
        }
        file_imports[index] = file_schemas;
        file_package[index] = Some(packages.len());
        packages.push(DomainPackage { name, kinds });
    }

    // Lower the compilation's `import sigil` declarations into synthetic
    // packages (one per rosetta namespace, after every declared package).
    // Their namespaces join the union namespace so every file's declarations
    // can reference the imported types bare or qualified.
    let sigil_files: Vec<(&str, &mox::Model)> = files
        .iter()
        .enumerate()
        .filter(|(index, _)| file_package[*index].is_some())
        .filter_map(|(_, file)| file.ast.map(|ast| (file.path, ast)))
        .collect();
    let declared_refs: Vec<(&str, &str)> = declared_packages
        .iter()
        .map(|(name, path)| (name.as_str(), *path))
        .collect();
    let sigil = crate::sigil::compile_sigil_imports(&sigil_files, &declared_refs, &imports.sigil);
    packages.extend(sigil.namespaces);

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
        validate_actor_obligations(&actors, &mut tagged);
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

    // The synthetic sigil packages follow every declared `.mox` package.
    model.packages.extend(sigil.packages);

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
    let sigil_diagnostics: Vec<(String, Diagnostic)> = sigil
        .diagnostics
        .into_iter()
        .map(|(origin, diagnostic)| (origin.tag().to_string(), diagnostic))
        .collect();
    let blocked = diags.iter().any(|(_, diagnostic)| diagnostic.is_error())
        || sigil_diagnostics
            .iter()
            .any(|(_, diagnostic)| diagnostic.is_error());
    let model = (!blocked).then_some(model);
    (model, diags, sigil_diagnostics)
}

/// The per-file outcome of [`compile_union_per_file`].
pub(crate) struct PerFileCompilation {
    /// The file's own package — plus, on the first file declaring an
    /// `import sigil`, the outcome's synthetic packages — or `None` when
    /// any error-severity diagnostic was produced for this file or one of
    /// its sigil dependencies failed.
    pub model: Option<ir::Model>,
    /// All diagnostics for this file, untagged (the caller knows the file).
    pub diagnostics: Vec<Diagnostic>,
    /// Sigil content diagnostics, each tagged with its rosetta key path
    /// (rendered against the retained rosetta texts, not the domain's).
    /// Collected on the first sigil-declaring file.
    pub sigil_diagnostics: Vec<(String, Diagnostic)>,
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
///
/// `import schema` declarations resolve through `imports.schemas`; `import
/// sigil` declarations resolve through `imports.sigil`. A declaration
/// without a provider entry errors on the declaring file (which blocks its
/// model). Sigil blocking is per namespace: a namespace whose rosetta
/// content produced errors drops its synthetic package, and a file loses
/// its model only when one of its own imports failed outright (the rosetta
/// file failed to parse) or targets a failed namespace — a file whose
/// imports all succeeded keeps its model even when other namespaces failed.
/// Content diagnostics are keyed by rosetta path: they collect in
/// [`PerFileCompilation::sigil_diagnostics`] on the first sigil-declaring
/// file, whose model carries the synthetic packages (the same first-declared
/// ownership the package declarations follow).
pub(crate) fn compile_union_per_file(
    files: &[MultiFile<'_>],
    imports: &crate::DomainImports,
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
        let file_schemas = collect_schema_imports(
            file.path,
            ast,
            &imports.schemas,
            &kinds,
            &mut buckets[index],
        );
        for import in &file_schemas {
            kinds.insert(import.name.clone(), TopKind::Class);
        }
        file_imports[index] = file_schemas;
        file_package[index] = Some(packages.len());
        packages.push(DomainPackage { name, kinds });
    }

    // Sigil imports lower into a structured outcome that carries everything
    // the assembly below needs. Import-level diagnostics route to their
    // declaring file's bucket (where the severity blocks that file's model);
    // content diagnostics stay rosetta-keyed; the successful namespaces join
    // the union resolution namespace; and per-file blocking asks the outcome
    // which files depend on failed content.
    let union_sigil_files: Vec<(&str, &mox::Model)> = files
        .iter()
        .enumerate()
        .filter(|(index, _)| file_package[*index].is_some())
        .filter_map(|(_, file)| file.ast.map(|ast| (file.path, ast)))
        .collect();
    let union_declared: Vec<(String, String)> = packages
        .iter()
        .map(|package| (package.name.clone(), String::new()))
        .collect();
    let union_declared_refs: Vec<(&str, &str)> = union_declared
        .iter()
        .map(|(name, kind)| (name.as_str(), kind.as_str()))
        .collect();
    let mut outcome = crate::sigil::compile_sigil_imports(
        &union_sigil_files,
        &union_declared_refs,
        &imports.sigil,
    );
    let mut union_sigil_diagnostics: Vec<(String, Diagnostic)> = Vec::new();
    for (origin, diagnostic) in std::mem::take(&mut outcome.diagnostics) {
        match origin {
            crate::sigil::DiagnosticOrigin::DeclaringFile { path } => {
                // The declaring path is always one of the compilation's
                // files; the fallback keeps the diagnostic visible if a
                // future origin ever is not.
                match files.iter().position(|file| file.path == path) {
                    Some(index) => buckets[index].push(diagnostic),
                    None => union_sigil_diagnostics.push((path, diagnostic)),
                }
            }
            crate::sigil::DiagnosticOrigin::Content { path } => {
                union_sigil_diagnostics.push((path, diagnostic))
            }
        }
    }
    // The successfully lowered namespaces join the union resolution
    // namespace so every file's declarations (and the actor/ddd pipelines'
    // own resolution passes) see the imported types. Failed namespaces stay
    // out: a reference into one errors on the referencing file, whose model
    // the dependency-based blocking below drops anyway.
    packages.extend(std::mem::take(&mut outcome.namespaces));

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
        validate_actor_obligations(&actors, &mut tagged);
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
    // produced any error or its own sigil dependencies failed (an import
    // whose rosetta file failed to parse, or whose namespace failed). The
    // first sigil-declaring file additionally carries the synthetic
    // packages the outcome owns.
    let union_sigil_owner_index = outcome
        .owner
        .as_ref()
        .and_then(|owner| files.iter().position(|file| file.path == *owner));
    files
        .iter()
        .enumerate()
        .map(|(index, _file)| {
            let blocked = buckets[index].iter().any(Diagnostic::is_error)
                || outcome.blocks(files[index].path);
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
                if Some(index) == union_sigil_owner_index {
                    model.packages.extend(outcome.packages.iter().cloned());
                }
                model
            });
            PerFileCompilation {
                model,
                diagnostics: std::mem::take(&mut buckets[index]),
                sigil_diagnostics: if Some(index) == union_sigil_owner_index {
                    std::mem::take(&mut union_sigil_diagnostics)
                } else {
                    Vec::new()
                },
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
/// Returns the IR (with `Model::rex_version` and `formatVersion` set) plus
/// the model's diagnostics and the tagged diagnostics of its `import sigil`
/// declarations, or a `None` model when any error-severity diagnostic was
/// produced. `path` locates the model on disk: vocabulary snapshots are read
/// from a `vocab/` directory next to the source, and pins from `model.lock`
/// next to that.
pub(crate) fn compile(
    path: &str,
    source: &str,
    model: &mox::Model,
    imports: &crate::DomainImports,
) -> (
    Option<ir::Model>,
    Vec<Diagnostic>,
    Vec<(String, Diagnostic)>,
) {
    let mut diags = Vec::new();

    let Some(package_decl) = &model.package else {
        diags.push(
            Diagnostic::error("missing `package` declaration", None).with_help(
                "add a package clause at the top of the file, e.g. `package com.example.model`",
            ),
        );
        return (None, diags, Vec::new());
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
    let imported = collect_schema_imports(path, model, &imports.schemas, &kinds, &mut diags);
    for import in &imported {
        kinds.insert(import.name.clone(), TopKind::Class);
    }

    // Lower the file's `import sigil` declarations into synthetic packages
    // (one per rosetta namespace, after the declared package). Their
    // namespaces join the resolution scope, so the file's own declarations
    // can reference the imported types bare or qualified.
    let sigil = crate::sigil::compile_sigil_imports(
        &[(path, model)],
        &[(package.as_str(), path)],
        &imports.sigil,
    );

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

    // Lower declarations in source order. With sigil namespaces present the
    // scope becomes the multi-package union (the declaring package first),
    // which is what makes `refers oracle.basic.Trade` resolve.
    let mut out = ir::Package::new(package.clone());
    out.description = package_decl.doc.clone();
    let mut domains: Vec<DomainPackage> = Vec::new();
    let scope = if sigil.namespaces.is_empty() {
        Scope::Single {
            package: &package,
            kinds: &kinds,
        }
    } else {
        domains.push(DomainPackage {
            name: package.clone(),
            kinds: kinds.clone(),
        });
        domains.extend(sigil.namespaces);
        Scope::Domains { packages: &domains }
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
        validate_actor_obligations(&actors, &mut tagged);
    }
    diags.extend(tagged.into_iter().map(|(_, diagnostic)| diagnostic));

    for mut class in classes {
        class.def.operations = std::mem::take(&mut class.operations);
        out.classes.push(class.def);
    }
    // Imported schemas join their package as nominal classes, after the
    // declared ones, in import-declaration order.
    for import in &imported {
        out.classes.push(import_class_def(import));
    }
    for record in actors {
        out.actors.push(record.def);
    }

    let mut model = ir::Model::new();
    model.packages.push(out);
    // The synthetic sigil packages follow every declared `.mox` package.
    model.packages.extend(sigil.packages);
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

    let blocked = diags.iter().any(Diagnostic::is_error)
        || sigil
            .diagnostics
            .iter()
            .any(|(_, diagnostic)| diagnostic.is_error());
    let model = (!blocked).then_some(model);
    let sigil_diagnostics: Vec<(String, Diagnostic)> = sigil
        .diagnostics
        .into_iter()
        .map(|(origin, diagnostic)| (origin.tag().to_string(), diagnostic))
        .collect();
    (model, diags, sigil_diagnostics)
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
    domains: &[DomainUnit],
    designs: &[ir::ddd::DddModel],
) -> (Option<ir::ActorModel>, Vec<(String, Diagnostic)>) {
    // Import resolution: each import must name one of the provided domains.
    let imports = actor_ast
        .map(|ast| ast.imports.as_slice())
        .unwrap_or_default();
    let mut import_diagnostics = Vec::new();
    for import in imports {
        if !domains.iter().any(|unit| unit.path == import.path) {
            import_diagnostics.push(Diagnostic::error(
                format!("imported file \"{}\" was not provided", import.path),
                Some(import.span),
            ));
        }
    }

    let mut diags =
        pipeline::engine_diags(actor_path, actor_diagnostics, import_diagnostics, domains);

    // The combined namespace of the usable domains (imported and
    // error-free), in first-appearance order. A domain that lowered
    // successfully had its `import schema` declarations validated, so they
    // contribute their nominal class names here.
    let packages = domain_namespaces(domains);
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
            if let Some(ast) = &unit.ast {
                for decl in &ast.declarations {
                    if let mox::Decl::Actors(decl) = decl {
                        lower_block(decl, &unit.source, &unit.path);
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
        validate_actor_obligations(&records, &mut diags);
    }
    // Capability coverage (rexlang#128): independent of acyclicity, it
    // resolves names, not lineages.
    validate_capability_coverage(&records, designs, &mut diags);

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
