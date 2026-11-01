//! Vocabulary lowering: the vendored `vocab/` snapshots and `model.lock`
//! reads — the only disk I/O in the driver — and the facet vocabulary's
//! primitive mapping.

use std::path::Path;

use rex_ir as ir;
use rex_syntax::ast as mox;
use rex_syntax::Span;

use crate::diagnostic::Diagnostic;

use super::*;
/// Lowers a `vocabulary` declaration: checks its shape, resolves the
/// snapshot version, reads the vendored snapshot, verifies the `model.lock`
/// digest, and validates the entries.
///
/// Returns `None` (with diagnostics) when the declaration cannot be lowered.
pub(crate) fn lower_vocabulary(
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
            // Date facets are out of scope in this milestone (issue #9):
            // snapshot validation and facet codegen are keyed to the
            // numeric/string value spaces.
            Some(ir::PrimitiveType::Date) => {
                facets_ok = false;
                diags.push(Diagnostic::error(
                    format!(
                        "facet '{}' of vocabulary '{name}' declares 'date'; \
                         date facets are not supported in this milestone",
                        facet.name.text
                    ),
                    Some(facet.type_ref.span),
                ));
            }
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
pub(crate) fn read_lockfile(
    base_dir: &Path,
    diags: &mut Vec<Diagnostic>,
) -> Option<rex_vocab::Lockfile> {
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
pub(crate) enum GlobVersion {
    /// No snapshot (or no `vocab/` directory).
    Missing,
    /// Exactly one snapshot; the version parsed from its file name.
    Unique(String),
    /// Several snapshots; the user must pin the version.
    Ambiguous(Vec<String>),
}

pub(crate) fn glob_snapshot_version(vocab_dir: &Path, sanitized: &str) -> GlobVersion {
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
pub(crate) fn primitive_facet_type(type_ref: &mox::TypeRef) -> Option<ir::PrimitiveType> {
    match type_ref.name.segments.as_slice() {
        [segment] => primitive_type(&segment.text),
        _ => None,
    }
}
