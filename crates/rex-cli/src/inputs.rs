//! Turning command-line inputs into in-memory sources and driver
//! compilations: input expansion, source reading, the `.actor`/`.ddd`
//! readers, and the shared compile step over collected sources.

use std::path::{Path, PathBuf};

use rex_driver::{compile_files, compile_str, DomainImports, MultiCompilation, SchemaImports};

use crate::imports::{collect_schema_imports, collect_sigil_imports, SigilSources};

fn read_source(file: &Path) -> anyhow::Result<(String, String)> {
    let source = std::fs::read_to_string(file)
        .map_err(|error| anyhow::anyhow!("cannot read {}: {error}", file.display()))?;
    Ok((file.display().to_string(), source))
}

/// Reads every input as `(driver path, source)` pairs.
pub(crate) fn read_sources(files: &[PathBuf]) -> anyhow::Result<Vec<(String, String)>> {
    files.iter().map(|file| read_source(file)).collect()
}

/// Expands command-line inputs: each entry may be a file or a directory;
/// directories are scanned recursively for `*.mox` files (`.actor` files are
/// ignored). Results are sorted lexicographically by path — this ordering
/// determines the IR package order — and must be non-empty.
pub(crate) fn expand_inputs(files: &[PathBuf]) -> anyhow::Result<Vec<PathBuf>> {
    let mut expanded = Vec::new();
    for entry in files {
        if entry.is_dir() {
            collect_mox_files(entry, &mut expanded)?;
        } else {
            expanded.push(entry.clone());
        }
    }
    if expanded.is_empty() {
        if files.is_empty() {
            anyhow::bail!("no input files");
        }
        let named = files
            .iter()
            .map(|file| file.display().to_string())
            .collect::<Vec<_>>()
            .join(", ");
        anyhow::bail!("no .mox files found in {named}");
    }
    expanded.sort();
    Ok(expanded)
}

/// Recursively collects `*.mox` files under `dir` (subdirectories first come
/// out in name order; the caller sorts the final list anyway).
fn collect_mox_files(dir: &Path, out: &mut Vec<PathBuf>) -> anyhow::Result<()> {
    let read = |error: std::io::Error| anyhow::anyhow!("cannot read {}: {error}", dir.display());
    let mut entries: Vec<std::fs::DirEntry> = std::fs::read_dir(dir)
        .map_err(read)?
        .collect::<Result<_, _>>()
        .map_err(read)?;
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let path = entry.path();
        if path.is_dir() {
            collect_mox_files(&path, out)?;
        } else if path.extension().and_then(|extension| extension.to_str()) == Some("mox") {
            out.push(path);
        }
    }
    Ok(())
}

/// Compiles in-memory sources into one model: a single source keeps the
/// single-file pipeline (identical diagnostics and IR); several sources
/// compile as one multi-package model. Schema imports declared by the
/// sources are read from disk first (see [`collect_schema_imports`]), then
/// sigil imports (see [`collect_sigil_imports`]).
pub(crate) fn compile_sources(
    sources: &[(String, String)],
) -> anyhow::Result<(MultiCompilation, SigilSources)> {
    let schema_imports = collect_schema_imports(sources)?;
    let sigil_sources = collect_sigil_imports(sources)?;
    let imports = DomainImports {
        schemas: schema_imports,
        sigil: sigil_sources.imports.clone(),
    };
    if let [(path, source)] = sources {
        let compilation = compile_str(path, source, &imports);
        Ok((
            MultiCompilation {
                model: compilation.model,
                diagnostics: compilation
                    .diagnostics
                    .into_iter()
                    .map(|diagnostic| (path.clone(), diagnostic))
                    .collect(),
                sigil_diagnostics: compilation.sigil_diagnostics,
            },
            sigil_sources,
        ))
    } else {
        Ok((compile_files(sources, &imports), sigil_sources))
    }
}

/// `true` for `.ddd` paths: the standalone DDD design surface, compiled
/// against the domain models it imports.
pub(crate) fn is_ddd_file(file: &Path) -> bool {
    file.extension().and_then(|extension| extension.to_str()) == Some("ddd")
}

/// `true` for `.actor` paths: the standalone actor-policy surface, compiled
/// against the domain models it imports.
pub(crate) fn is_actor_file(file: &Path) -> bool {
    file.extension().and_then(|extension| extension.to_str()) == Some("actor")
}

/// An `.actor` file plus the domain sources it imports, ready for
/// [`rex_driver::compile_actors_str`].
pub(crate) struct ActorPair {
    /// The actor file's path as given on the command line.
    pub(crate) path: String,
    /// The actor file's source text.
    pub(crate) source: String,
    /// Each imported domain as `(resolved path, source)`; the driver
    /// matches an actor-file import by exact string or by lexical
    /// resolution relative to the actor file's directory, so resolved
    /// paths keep vocabulary snapshots anchored to the declaring file no
    /// matter where the process runs from.
    pub(crate) domains: Vec<(String, String)>,
    /// The domains' `import schema` content, keyed by domain path.
    schemas: SchemaImports,
    /// The domains' `import sigil` content, plus the rosetta sources for
    /// rendering sigil diagnostics.
    sigil: SigilSources,
}

impl ActorPair {
    /// The import bundle the driver entry point takes.
    pub(crate) fn imports(&self) -> DomainImports {
        DomainImports {
            schemas: self.schemas.clone(),
            sigil: self.sigil.imports.clone(),
        }
    }

    /// The source text of the file with the given driver path, if it is the
    /// actor file, an imported domain, or a rosetta source.
    pub(crate) fn source_of(&self, path: &str) -> Option<&str> {
        if path == self.path {
            return Some(&self.source);
        }
        self.domains
            .iter()
            .find(|(name, _)| name == path)
            .map(|(_, source)| source.as_str())
            .or_else(|| {
                self.sigil
                    .sources
                    .iter()
                    .find(|(name, _)| name == path)
                    .map(|(_, source)| source.as_str())
            })
    }
}

/// Reads an `.actor` file plus every domain it imports.
///
/// Import paths resolve relative to the actor file's own directory, and the
/// **resolved** path is passed to the driver (the driver matches imports by
/// exact string or by lexical resolution, and the domain's driver path also
/// locates its `vocab/` directory — with raw import strings that lookup
/// would be relative to the process CWD). A missing import file is a clean
/// error naming the resolved path. Duplicate imports are read once.
pub(crate) fn read_actor_pair(file: &Path) -> anyhow::Result<ActorPair> {
    let source = std::fs::read_to_string(file)
        .map_err(|error| anyhow::anyhow!("cannot read {}: {error}", file.display()))?;
    let path = file.display().to_string();
    let mut domains: Vec<(String, String)> = Vec::new();
    if let Some(ast) = &rex_syntax::parse_actors(&source).ast {
        let dir = file
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        for import in &ast.imports {
            if domains.iter().any(|(existing, _)| *existing == import.path) {
                continue;
            }
            let resolved = dir.join(&import.path);
            let text = std::fs::read_to_string(&resolved).map_err(|error| {
                anyhow::anyhow!(
                    "cannot read imported file {} (imported by {}): {error}",
                    resolved.display(),
                    path
                )
            })?;
            domains.push((resolved.display().to_string(), text));
        }
    }
    let schemas = collect_schema_imports(&domains)?;
    let sigil = collect_sigil_imports(&domains)?;
    Ok(ActorPair {
        path,
        source,
        domains,
        schemas,
        sigil,
    })
}

/// A `.ddd` file plus the domain sources it imports, ready for
/// [`rex_driver::compile_ddd_str`].
pub(crate) struct DddDesignPair {
    /// The design file's path as given on the command line.
    pub(crate) path: String,
    /// The design file's source text.
    pub(crate) source: String,
    /// Each imported domain as `(resolved path, source)`, in import order;
    /// the driver matches a design import by exact string or by lexical
    /// resolution relative to the design file's directory, so resolved
    /// paths keep vocabulary snapshots anchored to the declaring file no
    /// matter where the process runs from.
    pub(crate) domains: Vec<(String, String)>,
    /// The domains' `import schema` content, keyed by domain path.
    schemas: SchemaImports,
    /// The domains' `import sigil` content, plus the rosetta sources for
    /// rendering sigil diagnostics.
    sigil: SigilSources,
}

impl DddDesignPair {
    /// The import bundle the driver entry point takes.
    pub(crate) fn imports(&self) -> DomainImports {
        DomainImports {
            schemas: self.schemas.clone(),
            sigil: self.sigil.imports.clone(),
        }
    }

    /// The source text of the file with the given driver path, if it is the
    /// design file, an imported domain, or a rosetta source.
    pub(crate) fn source_of(&self, path: &str) -> Option<&str> {
        if path == self.path {
            return Some(&self.source);
        }
        self.domains
            .iter()
            .find(|(name, _)| name == path)
            .map(|(_, source)| source.as_str())
            .or_else(|| {
                self.sigil
                    .sources
                    .iter()
                    .find(|(name, _)| name == path)
                    .map(|(_, source)| source.as_str())
            })
    }
}

/// Reads a `.ddd` file plus every domain it imports.
///
/// Import paths resolve relative to the design file's own directory, and the
/// **resolved** path is passed to the driver (the driver matches imports by
/// exact string or by lexical resolution, and the domain's driver path also
/// locates its `vocab/` directory — with raw import strings that lookup
/// would be relative to the process CWD). A missing import file is a clean
/// error naming the resolved path. Duplicate imports are read once.
pub(crate) fn read_ddd_design(file: &Path) -> anyhow::Result<DddDesignPair> {
    let source = std::fs::read_to_string(file)
        .map_err(|error| anyhow::anyhow!("cannot read {}: {error}", file.display()))?;
    let path = file.display().to_string();
    let mut domains: Vec<(String, String)> = Vec::new();
    if let Some(ast) = &rex_syntax::parse_ddd(&source).ast {
        let dir = file
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        for import in &ast.imports {
            if domains.iter().any(|(existing, _)| *existing == import.path) {
                continue;
            }
            let resolved = dir.join(&import.path);
            let text = std::fs::read_to_string(&resolved).map_err(|error| {
                anyhow::anyhow!(
                    "cannot read imported file {} (imported by {}): {error}",
                    resolved.display(),
                    path
                )
            })?;
            domains.push((resolved.display().to_string(), text));
        }
    }
    let schemas = collect_schema_imports(&domains)?;
    let sigil = collect_sigil_imports(&domains)?;
    Ok(DddDesignPair {
        path,
        source,
        domains,
        schemas,
        sigil,
    })
}
