//! Disk-facing import collection: `import schema` and `import sigil`
//! resolution for the CLI's compiles — the CLI owns filesystem access, the
//! driver receives texts only.

use std::path::{Path, PathBuf};

use rex_driver::{SchemaImports, SigilImports};

/// Collects the JSON content of every `import schema` declaration across
/// `sources`, resolving each import path **relative to the declaring `.mox`
/// file's directory** (the `.actor` import-resolution precedent: the CLI
/// owns filesystem access, the driver receives texts only). A missing
/// import file is a clean error naming the resolved path. Syntax errors in
/// a source surface later as compile diagnostics, so they are not fatal
/// here.
pub(crate) fn collect_schema_imports(
    sources: &[(String, String)],
) -> anyhow::Result<SchemaImports> {
    let mut imports = SchemaImports::new();
    for (path, source) in sources {
        let Some(ast) = &rex_syntax::parse(source).ast else {
            continue;
        };
        let dir = Path::new(path)
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        for decl in &ast.declarations {
            let rex_syntax::ast::Decl::ImportSchema(import) = decl else {
                continue;
            };
            let resolved = dir.join(&import.path);
            let json = std::fs::read_to_string(&resolved).map_err(|error| {
                anyhow::anyhow!(
                    "cannot read imported schema {} (imported by {}): {error}",
                    resolved.display(),
                    path
                )
            })?;
            imports.insert(path.clone(), import.path.clone(), json);
        }
    }
    Ok(imports)
}

/// The rosetta sources a compilation's `import sigil` declarations resolve
/// to: every file handed to the driver, as `(tag path, source text)`. The
/// tag path is what sigil diagnostics are labeled with.
pub(crate) struct SigilSources {
    /// The provider entries keyed `(mox path, import path)` — the named
    /// imports plus the discovered candidates.
    pub(crate) imports: SigilImports,
    /// Every provided rosetta source keyed by its tag path, for rendering
    /// sigil diagnostics against their own text.
    pub(crate) sources: Vec<(String, String)>,
}

/// Collects the rosetta content of every `import sigil` declaration across
/// `sources`, resolving each import path **relative to the declaring `.mox`
/// file's directory** (the `.actor` import-resolution precedent: the CLI
/// owns filesystem access, the driver receives texts only). A missing import
/// file is a clean error naming the resolved path. Syntax errors in a source
/// surface later as compile diagnostics, so they are not fatal here.
///
/// Rosetta-internal namespace imports are satisfied transitively: every
/// `*.rosetta` file under the named file's directory is read as a candidate
/// (mirroring sigil's project discovery: dot and `target` directories are
/// skipped, depth-capped), keyed by its resolved path. The driver lowers
/// only the namespace closure the named file actually needs, so unrelated
/// siblings never leak into the artifact.
pub(crate) fn collect_sigil_imports(sources: &[(String, String)]) -> anyhow::Result<SigilSources> {
    let mut sigil = SigilSources {
        imports: SigilImports::new(),
        sources: Vec::new(),
    };
    for (path, source) in sources {
        let Some(ast) = &rex_syntax::parse(source).ast else {
            continue;
        };
        let dir = Path::new(path)
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        for decl in &ast.declarations {
            let rex_syntax::ast::Decl::ImportSchema(import) = decl else {
                continue;
            };
            if import.kind != rex_syntax::ast::ImportKind::Sigil {
                continue;
            }
            let resolved = dir.join(&import.path);
            let text = std::fs::read_to_string(&resolved).map_err(|error| {
                anyhow::anyhow!(
                    "cannot read imported sigil {} (imported by {}): {error}",
                    resolved.display(),
                    path
                )
            })?;
            sigil
                .imports
                .insert(path.clone(), import.path.clone(), text.clone());
            sigil.sources.push((import.path.clone(), text));
            // Candidate pool: every rosetta file next to the named import.
            for candidate in discover_rosetta_files(&resolved) {
                let candidate_path = candidate.display().to_string();
                if sigil
                    .sources
                    .iter()
                    .any(|(existing, _)| *existing == candidate_path)
                {
                    continue;
                }
                let text = std::fs::read_to_string(&candidate).map_err(|error| {
                    anyhow::anyhow!(
                        "cannot read imported sigil {} (discovered for {} imported by {}): {error}",
                        candidate.display(),
                        import.path,
                        path
                    )
                })?;
                sigil
                    .imports
                    .insert(path.clone(), candidate_path.clone(), text.clone());
                sigil.sources.push((candidate_path, text));
            }
        }
    }
    Ok(sigil)
}

/// Recursively collects `*.rosetta` files under `file`'s directory,
/// mirroring sigil's project discovery: entries whose name starts with `.`
/// or is exactly `target` are skipped with their subtree, and the walk is
/// depth-capped (bounding symlink cycles). The named file itself is not
/// included.
fn discover_rosetta_files(file: &Path) -> Vec<PathBuf> {
    const MAX_DEPTH: usize = 16;
    fn collect(dir: &Path, depth: usize, named: &Path, out: &mut Vec<PathBuf>) {
        if depth > MAX_DEPTH {
            return;
        }
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        let mut entries: Vec<std::fs::DirEntry> = entries.filter_map(Result::ok).collect();
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries {
            let path = entry.path();
            let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
                continue;
            };
            if name.starts_with('.') || name == "target" {
                continue;
            }
            if path.is_dir() {
                collect(&path, depth + 1, named, out);
            } else if name.ends_with(".rosetta") && path != named {
                out.push(path);
            }
        }
    }
    let mut out = Vec::new();
    let base = file
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    collect(base, 0, file, &mut out);
    out
}
