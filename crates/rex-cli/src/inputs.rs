//! Turning command-line inputs into in-memory sources and driver
//! compilations: input expansion, source reading, the `.actor`/`.ddd`/`.evt`
//! readers, and the shared compile step over collected sources.

use std::collections::BTreeMap;
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

/// `true` for `.evt` paths: the standalone event-contract surface, compiled
/// against the domain models it imports.
pub(crate) fn is_evt_file(file: &Path) -> bool {
    file.extension().and_then(|extension| extension.to_str()) == Some("evt")
}

/// `true` for `.ifml` paths: the interaction-flow surface, compiled against
/// the domain models it imports and the `.ifml` pattern libraries it pulls
/// in.
pub(crate) fn is_ifml_file(file: &Path) -> bool {
    file.extension().and_then(|extension| extension.to_str()) == Some("ifml")
}

/// An `.ifml` file plus everything it imports, ready for
/// [`rex_ifml::compile_ifml_str`] and [`rex_ifml::check_ifml`].
pub(crate) struct IfmlInputs {
    /// The interaction file's path as given on the command line.
    pub(crate) path: String,
    /// The interaction file's source text.
    pub(crate) source: String,
    /// Every transitively imported `.ifml` source, keyed
    /// `(importing-file path, import path)` — the bundle
    /// [`rex_ifml::compile_ifml_str`] resolves against.
    pub(crate) ifml_imports: rex_ifml::IfmlImports,
    /// Every `.ifml` file in the compilation as `(resolver identity,
    /// source)` — the main file first, then each import under its as-written
    /// path. Used to render diagnostics for files the resolver rejected
    /// before building its index.
    pub(crate) sources: Vec<(String, String)>,
    /// The union of all imported `.mox` domain models, for typed binding.
    /// `None` when the file imports no domains (binding checks are then
    /// skipped).
    pub(crate) domains: Option<rex_ir::Model>,
}

/// Reads an `.ifml` file plus every `.ifml` and `.mox` file it imports,
/// transitively. Import paths resolve relative to the importing file's own
/// directory on disk, while the resolver keys each import by the importing
/// file's identity and the import string **as written** (the resolver is
/// filesystem-free and sees only the strings). Imports with other extensions
/// (`.actor`) pass through untouched. Imported `.mox` files are compiled to
/// Core IR and unioned for typed expression binding; a domain that fails to
/// compile is a hard error. A missing import file is a clean error naming
/// the resolved path.
pub(crate) fn read_ifml_inputs(file: &Path) -> anyhow::Result<IfmlInputs> {
    let source = std::fs::read_to_string(file)
        .map_err(|error| anyhow::anyhow!("cannot read {}: {error}", file.display()))?;
    let path = file.display().to_string();
    let mut inputs = IfmlInputs {
        sources: vec![(path.clone(), source.clone())],
        path: path.clone(),
        source,
        ifml_imports: rex_ifml::IfmlImports::default(),
        domains: None,
    };
    let mut domain_sources: Vec<(String, String)> = Vec::new();
    collect_ifml_imports(
        &path,
        file,
        &inputs.source,
        &mut inputs.ifml_imports,
        &mut inputs.sources,
        &mut domain_sources,
    )?;
    if domain_sources.is_empty() {
        return Ok(inputs);
    }
    let schemas = collect_schema_imports(&domain_sources)?;
    let sigil = collect_sigil_imports(&domain_sources)?;
    let imports = DomainImports {
        schemas,
        sigil: sigil.imports,
    };
    let mut union = rex_ir::Model::new();
    for (domain_path, domain_source) in &domain_sources {
        let compilation = compile_str(domain_path, domain_source, &imports);
        if !compilation.diagnostics.is_empty() {
            let rendered = diagnostics_preview(domain_path, domain_source, &compilation);
            anyhow::bail!(
                "imported domain {} does not compile:\n{rendered}",
                domain_path
            );
        }
        let Some(model) = compilation.model else {
            anyhow::bail!("imported domain {} does not compile", domain_path);
        };
        union.packages.extend(model.packages);
    }
    inputs.domains = Some(union);
    Ok(inputs)
}

fn diagnostics_preview(path: &str, source: &str, compilation: &rex_driver::Compilation) -> String {
    rex_driver::render(path, source, &compilation.diagnostics)
}

/// Gathers the `.ifml` import tree of one interaction file through
/// [`rex_ifml::walk_ifml_imports`]: the fetch closure resolves each import
/// relative to its importer's location on disk, provides `.ifml` content to
/// the bundle under the walker's `(importer identity, import-as-written)`
/// keys (the identities the resolver itself uses), and collects `.mox`
/// domains (deduplicated by resolved path) as a side effect. The first
/// unreadable import surfaces as an error after the walk. Disk paths are
/// tracked per identity here — the walker only ever sees identities.
fn collect_ifml_imports(
    identity: &str,
    disk_path: &Path,
    source: &str,
    bundle: &mut rex_ifml::IfmlImports,
    sources: &mut Vec<(String, String)>,
    domains: &mut Vec<(String, String)>,
) -> anyhow::Result<()> {
    let mut disk_paths: BTreeMap<String, PathBuf> = BTreeMap::new();
    disk_paths.insert(identity.to_string(), disk_path.to_path_buf());
    let mut read_error: Option<anyhow::Error> = None;
    let dir_of = |path: &Path| -> PathBuf {
        path.parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."))
            .to_path_buf()
    };
    let walk = rex_ifml::walk_ifml_imports(identity, source, &mut |importer, import| {
        let importer_disk = disk_paths.get(importer)?;
        let resolved = dir_of(importer_disk).join(import);
        if import.ends_with(".ifml") {
            match std::fs::read_to_string(&resolved) {
                Ok(text) => {
                    disk_paths.insert(import.to_string(), resolved);
                    bundle.provide(importer, import, text.clone());
                    Some(text)
                }
                Err(error) => {
                    if read_error.is_none() {
                        read_error = Some(anyhow::anyhow!(
                            "cannot read imported file {} (imported by {}): {error}",
                            resolved.display(),
                            importer
                        ));
                    }
                    None
                }
            }
        } else if import.ends_with(".mox") {
            let resolved_path = resolved.display().to_string();
            if domains
                .iter()
                .any(|(existing, _)| *existing == resolved_path)
            {
                return None;
            }
            match std::fs::read_to_string(&resolved) {
                Ok(text) => {
                    domains.push((resolved_path, text));
                    None
                }
                Err(error) => {
                    if read_error.is_none() {
                        read_error = Some(anyhow::anyhow!(
                            "cannot read imported file {} (imported by {}): {error}",
                            resolved.display(),
                            importer
                        ));
                    }
                    None
                }
            }
        } else {
            None
        }
    });
    if let Some(error) = read_error {
        return Err(error);
    }
    *sources = walk.sources;
    Ok(())
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

/// An `.evt` file plus the domain sources it imports, ready for
/// [`rex_driver::compile_evt_str`].
pub(crate) struct EvtPair {
    /// The contract file's path as given on the command line.
    pub(crate) path: String,
    /// The contract file's source text.
    pub(crate) source: String,
    /// Each imported domain as `(resolved path, source)`; the driver
    /// matches an `.evt` import by exact string or by lexical resolution
    /// relative to the contract file's directory, so resolved paths keep
    /// vocabulary snapshots anchored to the declaring file no matter where
    /// the process runs from.
    pub(crate) domains: Vec<(String, String)>,
    /// The domains' `import schema` content, keyed by domain path.
    schemas: SchemaImports,
    /// The domains' `import sigil` content, plus the rosetta sources for
    /// rendering sigil diagnostics.
    sigil: SigilSources,
}

impl EvtPair {
    /// The import bundle the driver entry point takes.
    pub(crate) fn imports(&self) -> DomainImports {
        DomainImports {
            schemas: self.schemas.clone(),
            sigil: self.sigil.imports.clone(),
        }
    }

    /// The source text of the file with the given driver path, if it is the
    /// contract file, an imported domain, or a rosetta source.
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

/// Reads an `.evt` file plus every domain it imports.
///
/// Import paths resolve relative to the contract file's own directory, and
/// the **resolved** path is passed to the driver (the driver matches
/// imports by exact string or by lexical resolution, and the domain's
/// driver path also locates its `vocab/` directory — with raw import
/// strings that lookup would be relative to the process CWD). A missing
/// import file is a clean error naming the resolved path. Duplicate imports
/// are read once.
pub(crate) fn read_evt_pair(file: &Path) -> anyhow::Result<EvtPair> {
    let source = std::fs::read_to_string(file)
        .map_err(|error| anyhow::anyhow!("cannot read {}: {error}", file.display()))?;
    let path = file.display().to_string();
    let mut domains: Vec<(String, String)> = Vec::new();
    if let Some(ast) = &rex_syntax::parse_evt(&source).ast {
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
    Ok(EvtPair {
        path,
        source,
        domains,
        schemas,
        sigil,
    })
}
