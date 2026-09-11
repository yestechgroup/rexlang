//! Semantic lowering: name resolution, validation, and AST → Core IR
//! translation for a single `.mox` source (one package per file).
//!
//! This module is salsa-free; the driver's tracked queries call into it.
//! Vocabulary declarations additionally read the vendored `vocab/` snapshots
//! and `model.lock` from disk, relative to the source file's directory.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};

use rex_ir as ir;
use rex_syntax::ast as mox;
use rex_syntax::Span;

use crate::diagnostic::{Diagnostic, DiagnosticCode};

/// The kind of a top-level declaration that occupies the package namespace.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TopKind {
    Class,
    Interface,
    Enum,
    Datatype,
    Vocabulary,
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
    /// The local (final-segment) type name.
    name: String,
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
    /// Name of the target class when the declared type resolved to a class.
    type_class: Option<String>,
    /// The declared type as written (for messages).
    type_text: String,
    /// The declared `opposite <name>` (the opposite feature's name).
    opposite: Option<String>,
    /// Span of the `opposite <name>` clause.
    opposite_span: Option<Span>,
}

/// A per-class record kept alongside the IR for cross-class validation.
struct ClassRecord {
    name: String,
    name_span: Span,
    def: ir::ClassDef,
    features: Vec<FeatureRecord>,
    /// Lowered operations; appended to `def.operations` after validation.
    operations: Vec<ir::Operation>,
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
    let package = package_decl.full_name();
    let base_dir = Path::new(path)
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));

    // Index the package namespace and report duplicate declarations.
    let mut kinds: HashMap<&str, TopKind> = HashMap::new();
    let mut enum_decls: HashMap<&str, &mox::EnumDecl> = HashMap::new();
    for decl in &model.declarations {
        let (kind, name) = match decl {
            mox::Decl::Class(decl) => (TopKind::Class, &decl.name),
            mox::Decl::Interface(decl) => (TopKind::Interface, &decl.name),
            mox::Decl::Enum(decl) => (TopKind::Enum, &decl.name),
            mox::Decl::Datatype(decl) => (TopKind::Datatype, &decl.name),
            mox::Decl::Vocabulary(decl) => (TopKind::Vocabulary, &decl.name),
            // Actors semantics are a later milestone; ignore for now.
            mox::Decl::Actors(_) => continue,
            mox::Decl::Annotation(_) => continue,
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
            kinds.insert(&name.text, kind);
            if let mox::Decl::Enum(enum_decl) = decl {
                enum_decls.insert(&enum_decl.name.text, enum_decl);
            }
        }
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
    let vocab_keys: HashMap<&str, HashSet<&str>> = vocabulary_defs
        .iter()
        .map(|def| {
            (
                def.name.as_str(),
                def.entries.iter().map(|entry| entry.key.as_str()).collect(),
            )
        })
        .collect();

    // Lower declarations in source order.
    let mut out = ir::Package::new(package.clone());
    let mut classes: Vec<ClassRecord> = Vec::new();
    for decl in &model.declarations {
        match decl {
            mox::Decl::Vocabulary(_) => {}
            // Actors semantics are a later milestone; ignore for now.
            mox::Decl::Actors(_) => {}
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
                decl,
                source,
                &package,
                &kinds,
                &enum_decls,
                &vocab_keys,
                &mut diags,
            )),
        }
    }
    out.vocabularies = vocabulary_defs;

    detect_inheritance_cycles(&classes, &mut diags);
    validate_opposites(&classes, &mut diags);

    for mut class in classes {
        class.def.operations = std::mem::take(&mut class.operations);
        out.classes.push(class.def);
    }

    let blocked = diags.iter().any(Diagnostic::is_error);
    let model = (!blocked).then(|| {
        let mut model = ir::Model::new();
        model.packages.push(out);
        // Feature ids are assigned by `ClassDef::new` in declaration order;
        // keep them in sync with the final feature lists.
        for class in &mut model.packages[0].classes {
            class.assign_feature_ids();
        }
        model
    });
    (model, diags)
}

/// Resolves a type reference, emitting a diagnostic (and returning `None`)
/// when it cannot be resolved within the file's single package.
fn resolve(
    type_ref: &mox::TypeRef,
    package: &str,
    kinds: &HashMap<&str, TopKind>,
    diags: &mut Vec<Diagnostic>,
) -> Option<Resolution> {
    let segments = &type_ref.name.segments;
    let full_name = type_ref.name.full_name();

    // Primitives and single-segment names are package-local; the only
    // qualified form accepted is `<package>.<Name>`.
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
                name: local.clone(),
            });
        }
    }

    match kinds.get(local.as_str()) {
        Some(top_kind) => {
            let kind = match top_kind {
                TopKind::Class => Resolved::Class,
                TopKind::Interface => Resolved::Interface,
                TopKind::Enum => Resolved::Enum,
                TopKind::Datatype => Resolved::Datatype,
                TopKind::Vocabulary => Resolved::Vocabulary,
            };
            Some(Resolution {
                kind,
                name: local.clone(),
            })
        }
        None => {
            diags.push(Diagnostic::error(
                format!("unknown type '{full_name}'"),
                Some(type_ref.span),
            ));
            None
        }
    }
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
        Some(resolution) => resolution.kind.to_ir(package, &resolution.name),
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
    match resolution {
        Some(resolution) if resolution.kind == Resolved::Class => Some(resolution.name.clone()),
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
        literals.push(ir::EnumLiteral::new(
            &literal.name.text,
            literal.label.clone(),
            value,
        ));
    }
    ir::EnumDef::new(&decl.name.text, literals)
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
    package: &str,
    kinds: &HashMap<&str, TopKind>,
    enum_decls: &HashMap<&str, &mox::EnumDecl>,
    vocab_keys: &HashMap<&str, HashSet<&str>>,
    diags: &mut Vec<Diagnostic>,
) -> ClassRecord {
    let mut extends = Vec::new();
    for type_ref in &decl.extends {
        if let Some(resolution) = resolve(type_ref, package, kinds, diags) {
            match resolution.kind {
                Resolved::Class | Resolved::Interface => {
                    extends.push(resolution.kind.to_ir(package, &resolution.name));
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
                type_ref,
                multiplicity,
                name,
                default,
                ..
            } => {
                let resolution = resolve(type_ref, package, kinds, diags);
                report_class_typed_feature(name, type_ref, resolution.as_ref(), diags);
                let multiplicity = multiplicity
                    .as_ref()
                    .map(|m| lower_multiplicity(m, diags))
                    .unwrap_or(ir::Multiplicity::REQUIRED);
                let ir_type = ir_type_of(resolution.as_ref(), package, type_ref);
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
                    enum_decls,
                    vocab_keys,
                    diags,
                );
                let ir_feature = apply_modifiers(ir_feature, feature.modifiers());
                features.push(ir_feature);
                records.push(FeatureRecord {
                    name: name.text.clone(),
                    kind: FKind::Attribute,
                    type_class: class_of(resolution.as_ref()),
                    type_text: type_ref.name.full_name(),
                    opposite: None,
                    opposite_span: None,
                });
            }
            mox::FeatureDecl::Containment {
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
                    package,
                    kinds,
                    diags,
                );
                features.push(apply_modifiers(ir_feature, feature.modifiers()));
                records.push(record);
            }
            mox::FeatureDecl::Reference {
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
                    package,
                    kinds,
                    diags,
                );
                features.push(apply_modifiers(ir_feature, feature.modifiers()));
                records.push(record);
            }
            mox::FeatureDecl::Container {
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
                    package,
                    kinds,
                    diags,
                );
                features.push(apply_modifiers(ir_feature, feature.modifiers()));
                records.push(record);
            }
            mox::FeatureDecl::Op {
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
                let return_resolution = resolve(return_type, package, kinds, diags);
                let mut lowered_params = Vec::new();
                for param in params {
                    let resolution = resolve(&param.type_ref, package, kinds, diags);
                    lowered_params.push(ir::OperationParam {
                        name: param.name.text.clone(),
                        type_: ir_type_of(resolution.as_ref(), package, &param.type_ref),
                    });
                }
                let mut operation = ir::Operation::new(
                    &name.text,
                    ir_type_of(return_resolution.as_ref(), package, return_type),
                    lowered_params,
                );
                operation.bodies = lowered_bodies;
                operations.push(operation);
                records.push(FeatureRecord {
                    name: name.text.clone(),
                    kind: FKind::Op,
                    type_class: None,
                    type_text: return_type.name.full_name(),
                    opposite: None,
                    opposite_span: None,
                });
            }
            mox::FeatureDecl::Derived {
                type_ref,
                multiplicity,
                name,
                body,
                bodies,
                ..
            } => {
                let resolution = resolve(type_ref, package, kinds, diags);
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
                let ir_type = ir_type_of(resolution.as_ref(), package, type_ref);
                let mut ir_feature = ir::Feature::new(
                    &name.text,
                    ir::FeatureKind::Attribute,
                    ir_type,
                    multiplicity,
                )
                .derived();
                ir_feature = apply_modifiers(ir_feature, feature.modifiers());
                ir_feature.bodies = lowered_bodies;
                features.push(ir_feature);
                records.push(FeatureRecord {
                    name: name.text.clone(),
                    kind: FKind::Derived,
                    type_class: class_of(resolution.as_ref()),
                    type_text: type_ref.name.full_name(),
                    opposite: None,
                    opposite_span: None,
                });
            }
        }
    }

    ClassRecord {
        name: decl.name.text.clone(),
        name_span: decl.name.span,
        def: ir::ClassDef::new(&decl.name.text, extends, features),
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
    package: &str,
    kinds: &HashMap<&str, TopKind>,
    diags: &mut Vec<Diagnostic>,
) -> (ir::Feature, FeatureRecord) {
    let resolution = resolve(type_ref, package, kinds, diags);
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
    let ir_type = ir_type_of(resolution.as_ref(), package, type_ref);
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
        type_class: class_of(resolution.as_ref()),
        type_text: type_ref.name.full_name(),
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

/// Applies an attribute default value. The `Name` form is an enum literal
/// reference on enum-typed attributes, or an entry-key reference on
/// vocabulary-typed attributes (lowered to `DefaultValue::String`, since
/// vocabulary keys are strings).
#[allow(clippy::too_many_arguments)]
fn apply_default(
    feature: ir::Feature,
    default: Option<&mox::DefaultValue>,
    resolution: Option<&Resolution>,
    enum_decls: &HashMap<&str, &mox::EnumDecl>,
    vocab_keys: &HashMap<&str, HashSet<&str>>,
    diags: &mut Vec<Diagnostic>,
) -> ir::Feature {
    let Some(default) = default else {
        return feature;
    };
    match default {
        mox::DefaultValue::Str { value, .. } => {
            feature.with_default(ir::DefaultValue::String(value.clone()))
        }
        mox::DefaultValue::Int { value, .. } => feature.with_default(ir::DefaultValue::Int(*value)),
        mox::DefaultValue::Bool { value, .. } => {
            feature.with_default(ir::DefaultValue::Bool(*value))
        }
        mox::DefaultValue::Name(name) => match resolution {
            Some(Resolution {
                kind: Resolved::Enum,
                name: enum_name,
            }) => {
                if let Some(enum_decl) = enum_decls.get(enum_name.as_str()) {
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
            }) => {
                // An absent key set means the vocabulary failed to load; that
                // error is already reported, so the default adds nothing.
                if let Some(keys) = vocab_keys.get(vocab_name.as_str()) {
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
/// edges), reporting the first cycle found.
fn detect_inheritance_cycles(classes: &[ClassRecord], diags: &mut Vec<Diagnostic>) {
    let index: HashMap<&str, &ClassRecord> = classes
        .iter()
        .map(|class| (class.name.as_str(), class))
        .collect();
    let mut state: HashMap<&str, u8> = HashMap::new();
    for class in classes {
        let mut path: Vec<&str> = Vec::new();
        if let Some(cycle) = visit_class(class.name.as_str(), &index, &mut state, &mut path) {
            let span = index
                .get(cycle[0])
                .map(|record| record.name_span)
                .unwrap_or_else(|| (0..0).into());
            diags.push(Diagnostic::error(
                format!("inheritance cycle detected: {}", cycle.join(" extends ")),
                Some(span),
            ));
            return;
        }
    }
}

/// Iterative-depth-first visit; returns the cycle path when a back edge is
/// found. `state`: absent = unvisited, `1` = on the current path, `2` = done.
fn visit_class<'a>(
    class: &'a str,
    index: &HashMap<&'a str, &'a ClassRecord>,
    state: &mut HashMap<&'a str, u8>,
    path: &mut Vec<&'a str>,
) -> Option<Vec<&'a str>> {
    match state.get(class) {
        Some(1) => {
            let start = path.iter().position(|name| *name == class).unwrap_or(0);
            let mut cycle: Vec<&str> = path[start..].to_vec();
            cycle.push(class);
            return Some(cycle);
        }
        Some(_) => return None,
        None => {}
    }
    state.insert(class, 1);
    path.push(class);
    if let Some(record) = index.get(class) {
        for superclass in record
            .def
            .extends
            .iter()
            .filter_map(|type_ref| match type_ref {
                ir::TypeRef::Class { name, .. } => Some(name.as_str()),
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
fn validate_opposites(classes: &[ClassRecord], diags: &mut Vec<Diagnostic>) {
    let index: HashMap<&str, &ClassRecord> = classes
        .iter()
        .map(|class| (class.name.as_str(), class))
        .collect();
    for class in classes {
        for feature in &class.features {
            if !matches!(
                feature.kind,
                FKind::Containment | FKind::Reference | FKind::Container
            ) {
                continue;
            }
            let (Some(target), Some(opposite)) = (&feature.type_class, &feature.opposite) else {
                continue;
            };
            let Some(opposite_span) = feature.opposite_span else {
                continue;
            };
            let Some(other) = index.get(target.as_str()) else {
                continue;
            };
            let Some(counterpart) = other
                .features
                .iter()
                .find(|candidate| &candidate.name == opposite)
            else {
                diags.push(Diagnostic::error(
                    format!(
                        "opposite mismatch: '{}.{}' declares opposite '{}.{}', but class '{}' has no feature '{}'",
                        class.name, feature.name, target, opposite, target, opposite,
                    ),
                    Some(opposite_span),
                ));
                continue;
            };
            let expected = match feature.kind {
                FKind::Containment => FKind::Container,
                FKind::Container => FKind::Containment,
                _ => FKind::Reference,
            };
            if counterpart.kind != expected {
                diags.push(Diagnostic::error(
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
                ));
                continue;
            }
            if counterpart.type_class.as_deref() != Some(class.name.as_str()) {
                diags.push(Diagnostic::error(
                    format!(
                        "opposite mismatch: '{}.{}' expects '{}.{}' to have type '{}', but found '{}'",
                        class.name, feature.name, target, opposite, class.name, counterpart.type_text,
                    ),
                    Some(opposite_span),
                ));
                continue;
            }
            if counterpart.opposite.as_deref() != Some(feature.name.as_str()) {
                diags.push(Diagnostic::error(
                    format!(
                        "opposite mismatch: '{}.{}' does not declare opposite '{}.{}'",
                        target, opposite, class.name, feature.name,
                    ),
                    Some(opposite_span),
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
