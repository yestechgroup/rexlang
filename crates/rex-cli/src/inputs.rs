//! Turning command-line inputs into in-memory sources and driver
//! compilations: input expansion, source reading, and the thin
//! `.ifml`/`.actor`/`.ddd`/`.evt`/`.dmn`/`.uc`/`.cmmn`/`.deploy` readers — import
//! gathering itself lives in [`rex_driver::project`], so each reader here
//! only builds the disk fetch closure (the CLI's read-error policy is a
//! closure side-channel) and compiles the gathered project. In project
//! mode ([`crate::project::Project`]) the fetch closures are map-first,
//! disk-second — the snapshot serves project files, the disk fallback
//! fills it. Plus the shared compile step over collected sources.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use rex_driver::{
    compile_actor_imports, compile_decision_imports, compile_domain_union, compile_files,
    compile_str, gather_deploy_imports, gather_domain_sources, gather_ifml_project,
    gather_usecase_imports, DomainImports, MultiCompilation, SchemaImports,
};

use crate::imports::{collect_schema_imports, collect_sigil_imports, SigilSources};
use crate::project::{Artifact, Project};

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

/// Expands `rexlang check` inputs: directories scan recursively for **every
/// supported surface** (`.mox`, `.actor`, `.ddd`, `.evt`, `.dmn`, `.uc`,
/// `.ifml`), not just `.mox` — formatter-golden `X.fmt.<ext>` artifacts
/// excepted (see [`is_formatter_golden`]). Used by the directory batch
/// mode, where each expanded file is checked independently; explicit file
/// arguments pass through as-is.
pub(crate) fn expand_check_inputs(files: &[PathBuf]) -> anyhow::Result<Vec<PathBuf>> {
    let mut expanded = Vec::new();
    for entry in files {
        if entry.is_dir() {
            collect_supported_files(entry, &mut expanded)?;
        } else {
            expanded.push(entry.clone());
        }
    }
    if expanded.is_empty() {
        let named = files
            .iter()
            .map(|file| file.display().to_string())
            .collect::<Vec<_>>()
            .join(", ");
        anyhow::bail!("no model files found in {named}");
    }
    expanded.sort();
    Ok(expanded)
}

/// Recursively collects `*.mox` files under `dir` (subdirectories first come
/// out in name order; the caller sorts the final list anyway).
fn collect_mox_files(dir: &Path, out: &mut Vec<PathBuf>) -> anyhow::Result<()> {
    collect_with_extensions(dir, &["mox"], out)
}

/// Recursively collects files under `dir` whose extension is one of
/// `extensions` (subdirectories first come out in name order; the caller
/// sorts the final list anyway). Formatter-golden artifacts (see
/// [`is_formatter_golden`]) are never collected.
fn collect_with_extensions(
    dir: &Path,
    extensions: &[&str],
    out: &mut Vec<PathBuf>,
) -> anyhow::Result<()> {
    let read = |error: std::io::Error| anyhow::anyhow!("cannot read {}: {error}", dir.display());
    let mut entries: Vec<std::fs::DirEntry> = std::fs::read_dir(dir)
        .map_err(read)?
        .collect::<Result<_, _>>()
        .map_err(read)?;
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let path = entry.path();
        if path.is_dir() {
            collect_with_extensions(&path, extensions, out)?;
        } else if path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extensions.contains(&extension))
            && !is_formatter_golden(&path)
        {
            out.push(path);
        }
    }
    Ok(())
}

/// `true` for formatter-golden artifacts: the `X.fmt.<ext>` files the
/// formatter's golden tests pin byte-for-byte against a canonical source
/// that lives elsewhere (the GOLDENS table in rex-syntax's fmt tests). They
/// are outputs, not standalone models — their imports and vocabulary
/// snapshots only resolve next to their *sources* — so directory scans
/// skip them. Explicit file arguments pass through untouched.
fn is_formatter_golden(path: &Path) -> bool {
    path.file_stem()
        .and_then(|stem| stem.to_str())
        .is_some_and(|stem| stem.ends_with(".fmt"))
}

/// Recursively collects files of every supported surface under `dir`.
fn collect_supported_files(dir: &Path, out: &mut Vec<PathBuf>) -> anyhow::Result<()> {
    collect_with_extensions(
        dir,
        &[
            "mox", "actor", "ddd", "evt", "dmn", "uc", "cmmn", "ifml", "deploy",
        ],
        out,
    )
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

/// `true` for `.dmn` paths: the standalone decision-model surface, compiled
/// against the domain models it imports and the decision-table contents its
/// `logic table` clauses name.
pub(crate) fn is_dmn_file(file: &Path) -> bool {
    file.extension().and_then(|extension| extension.to_str()) == Some("dmn")
}

/// `true` for `.uc` paths: the use case surface, compiled against the
/// domain models it imports and the pre-compiled `.ddd`/`.actor`/`.evt`
/// artifacts its other imports resolve to.
pub(crate) fn is_uc_file(file: &Path) -> bool {
    file.extension().and_then(|extension| extension.to_str()) == Some("uc")
}

/// `true` for `.cmmn` paths: the case-management surface, compiled against
/// its imported domains, events, actors, and designs.
pub(crate) fn is_cmmn_file(file: &Path) -> bool {
    file.extension().and_then(|extension| extension.to_str()) == Some("cmmn")
}

/// `true` for `.deploy` paths: the deployment surface, compiled against the
/// `.ddd` designs and `.ifml` flows it imports.
pub(crate) fn is_deploy_file(file: &Path) -> bool {
    file.extension().and_then(|extension| extension.to_str()) == Some("deploy")
}

/// `true` for `.ifml` paths: the interaction-flow surface, compiled against
/// the domain models it imports and the `.ifml` pattern libraries it pulls
/// in.
pub(crate) fn is_ifml_file(file: &Path) -> bool {
    file.extension().and_then(|extension| extension.to_str()) == Some("ifml")
}

/// An `.ifml` file plus everything it imports, ready for
/// [`rex_ifml::compile_ifml_str`] and [`rex_ifml::check_ifml_with_actors`].
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
    /// Every imported `.actor` policy, compiled — the authorization
    /// dimension [`rex_ifml::check_ifml_with_actors`] resolves
    /// `requires:`/`roles:` against. Empty when the file imports no actor
    /// models (authorization checking is then skipped).
    pub(crate) actors: Vec<rex_ir::ActorModel>,
    /// Every imported `.dmn` decision model, compiled — the decision
    /// dimension [`rex_ifml::check_ifml_with_decisions`] resolves `data:`
    /// names against. Empty when the file imports no decision models
    /// (decision checking is then skipped).
    pub(crate) decisions: Vec<rex_ir::dmn::DmnModel>,
}

/// Reads an `.ifml` file plus every `.ifml`, `.mox`, `.actor`, and `.dmn`
/// file it imports, transitively — gathered through
/// [`rex_driver::project`], the shared host-side resolver. Import paths
/// resolve relative to the importing file's own directory on disk, while
/// the resolver keys each import by the importing file's identity and the
/// import string **as written** (the resolver is filesystem-free and sees
/// only the strings). The fetch is map-first within `project` (the
/// snapshot serves project files; the CLI's open-documents-first analog),
/// disk-second otherwise — and the disk fallback fills the map. The disk
/// fetch keeps the CLI's read-error policy: the first unreadable surface
/// import (`.ifml`/`.mox`/`.actor`/`.dmn`) surfaces as an error after the
/// walk, naming the resolved path, while a gathered `.dmn` model's
/// decision-table references (leaf files fetched under the model's
/// resolved key) degrade to the driver's unprovided-table diagnostic.
/// Imported `.mox` files are compiled to Core IR and unioned for typed
/// expression binding; imported `.actor` files are compiled to actor
/// models for authorization checking; imported `.dmn` files are compiled
/// to decision models for decision checking — lookup-before-compute
/// within `project`, so a file the batch already checked is reused, not
/// recompiled — and a domain, actor, or decision model that fails to
/// compile is a hard error.
pub(crate) fn read_ifml_inputs(
    file: &Path,
    mut project: Option<&mut Project>,
) -> anyhow::Result<IfmlInputs> {
    let source = std::fs::read_to_string(file)
        .map_err(|error| anyhow::anyhow!("cannot read {}: {error}", file.display()))?;
    let path = file.display().to_string();
    // Disk paths per identity: the walker only ever sees identities — the
    // as-written import strings `.ifml` recursion recurses under, and the
    // resolved keys this closure hands back (an imported `.actor` policy's
    // own `.mox` imports are fetched under the policy's resolved key) — so
    // every key handed out is mapped the same way the entry identity is.
    // Other extensions pass through unread unless the importer is a
    // gathered `.dmn` model, whose decision-table references they are.
    let mut disk_paths: BTreeMap<String, PathBuf> = BTreeMap::new();
    disk_paths.insert(path.clone(), file.to_path_buf());
    let mut read_error: Option<anyhow::Error> = None;
    let mut fetch = |importer: &str, import: &str| -> Option<(String, String)> {
        let known_surface = import.ends_with(".ifml")
            || import.ends_with(".mox")
            || import.ends_with(".actor")
            || import.ends_with(".dmn");
        if !known_surface {
            let importer_is_dmn = disk_paths.get(importer).is_some_and(|disk| {
                disk.extension().and_then(|extension| extension.to_str()) == Some("dmn")
            });
            if !importer_is_dmn {
                return None;
            }
        }
        let resolved = {
            let importer_disk = disk_paths.get(importer)?;
            dir_of(importer_disk).join(import)
        };
        let resolved_key = resolved.display().to_string();
        let fetched = match project.as_deref_mut() {
            Some(project) => project.fetch_path(&resolved_key, &resolved),
            None => std::fs::read_to_string(&resolved),
        };
        let text = match fetched {
            Ok(text) => text,
            Err(error) => {
                // Decision-table contents degrade to the driver's
                // unprovided-table diagnostic (E0209 names the written
                // path); a surface import that cannot be read is the
                // CLI's hard error, raised after the walk.
                if read_error.is_none() && known_surface {
                    read_error = Some(anyhow::anyhow!(
                        "cannot read imported file {} (imported by {}): {error}",
                        resolved.display(),
                        importer
                    ));
                }
                return None;
            }
        };
        disk_paths.insert(import.to_string(), resolved.clone());
        disk_paths.insert(resolved_key.clone(), resolved);
        Some((resolved_key, text))
    };
    let gathered = gather_ifml_project(&path, &source, &mut fetch);
    if let Some(error) = read_error {
        return Err(error);
    }
    let mut actors = Vec::new();
    for actor in &gathered.actors {
        let model = match project.as_deref_mut() {
            Some(project) => {
                let artifact = project.artifact(&actor.key, || {
                    compile_actor_imports(std::slice::from_ref(actor))
                        .map(|mut models| models.pop().expect("one policy, one model"))
                        .map_err(|message| anyhow::anyhow!(message))
                        .map(Artifact::Actor)
                })?;
                match artifact {
                    Artifact::Actor(model) => model,
                    _ => unreachable!("actor cache slots only hold actor artifacts"),
                }
            }
            None => compile_actor_imports(std::slice::from_ref(actor))
                .map(|mut models| models.pop().expect("one policy, one model"))
                .map_err(|message| anyhow::anyhow!(message))?,
        };
        actors.push(model);
    }
    let mut decisions = Vec::new();
    for decision in &gathered.decisions {
        let model = match project.as_deref_mut() {
            Some(project) => {
                let artifact = project.artifact(&decision.key, || {
                    compile_decision_imports(std::slice::from_ref(decision))
                        .map(|mut models| models.pop().expect("one import, one model"))
                        .map_err(|message| anyhow::anyhow!(message))
                        .map(Artifact::Decision)
                })?;
                match artifact {
                    Artifact::Decision(model) => model,
                    _ => unreachable!("decision cache slots only hold decision artifacts"),
                }
            }
            None => compile_decision_imports(std::slice::from_ref(decision))
                .map(|mut models| models.pop().expect("one import, one model"))
                .map_err(|message| anyhow::anyhow!(message))?,
        };
        decisions.push(model);
    }
    let domains = if gathered.domains.is_empty() {
        None
    } else {
        Some(compile_domain_union(&gathered.domains).map_err(|message| anyhow::anyhow!(message))?)
    };
    Ok(IfmlInputs {
        path,
        source,
        ifml_imports: gathered.imports,
        sources: gathered.sources,
        domains,
        actors,
        decisions,
    })
}

/// The importing file's directory, `"."` for a bare file name — the rule
/// every import resolution here follows.
fn dir_of(path: &Path) -> PathBuf {
    path.parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."))
        .to_path_buf()
}
/// A surface file plus the domain sources it imports, ready for the
/// driver's `compile_*_str` entry points. The `.actor`, `.ddd`, and `.evt`
/// compiles take this bundle directly; `.uc` and `.cmmn` wrap it with
/// their pre-compiled import buckets.
pub(crate) struct DomainPair {
    /// The surface file's path as given on the command line.
    pub(crate) path: String,
    /// The surface file's source text.
    pub(crate) source: String,
    /// Each imported domain as `(resolved path, source)`; the driver
    /// matches a surface-file import by exact string or by lexical
    /// resolution relative to the importing file's directory, so resolved
    /// paths keep vocabulary snapshots anchored to the declaring file no
    /// matter where the process runs from.
    pub(crate) domains: Vec<(String, String)>,
    /// Each imported domain as `(import string as written, source)` — the
    /// driver tags a domain's diagnostics with its `DomainUnit` path (the
    /// import string), so rendering needs both keys.
    import_keys: Vec<(String, String)>,
    /// The domains' `import schema` content, keyed by domain path.
    schemas: SchemaImports,
    /// The domains' `import sigil` content, plus the rosetta sources for
    /// rendering sigil diagnostics.
    sigil: SigilSources,
}

impl DomainPair {
    /// The import bundle the driver entry point takes.
    pub(crate) fn imports(&self) -> DomainImports {
        DomainImports {
            schemas: self.schemas.clone(),
            sigil: self.sigil.imports.clone(),
        }
    }

    /// The source text of the file with the given driver path, if it is
    /// the surface file, an imported domain (by resolved path or by the
    /// import string the surface names it with — the driver tags a
    /// domain's diagnostics with the latter), or a rosetta source.
    pub(crate) fn source_of(&self, path: &str) -> Option<&str> {
        if path == self.path {
            return Some(&self.source);
        }
        self.domains
            .iter()
            .find(|(name, _)| name == path)
            .map(|(_, source)| source.as_str())
            .or_else(|| {
                self.import_keys
                    .iter()
                    .find(|(name, _)| name == path)
                    .map(|(_, source)| source.as_str())
            })
            .or_else(|| {
                self.sigil
                    .sources
                    .iter()
                    .find(|(name, _)| name == path)
                    .map(|(_, source)| source.as_str())
            })
    }
}

/// Reads a surface file plus every domain it imports; `import_paths`
/// yields the file's raw (unresolved) import path strings and is empty
/// when the file does not parse — the driver reports that.
///
/// Import paths resolve relative to the importing file's own directory
/// (by [`gather_domain_sources`], deduplicated by resolved key), and the
/// **resolved** path is passed to the driver (the driver matches imports
/// by exact string or by lexical resolution, and the domain's driver path
/// also locates its `vocab/` directory — with raw import strings that
/// lookup would be relative to the process CWD). The fetch is map-first
/// within `project`, disk-second otherwise, and the disk fallback fills
/// the map. A missing import file is a clean error naming the resolved
/// path: the gatherer itself skips misses silently (the LSP's
/// discipline), so this reader captures the failure in its fetch closure
/// and hard-errors after the call.
fn read_domain_pair(
    file: &Path,
    import_paths: impl Fn(&str) -> Vec<String>,
    mut project: Option<&mut Project>,
) -> anyhow::Result<DomainPair> {
    let source = std::fs::read_to_string(file)
        .map_err(|error| anyhow::anyhow!("cannot read {}: {error}", file.display()))?;
    let path = file.display().to_string();
    let mut read_error: Option<anyhow::Error> = None;
    let mut fetch = |resolved: &str| -> Option<String> {
        let fetched = match project.as_deref_mut() {
            Some(project) => project.fetch(resolved),
            None => std::fs::read_to_string(resolved),
        };
        match fetched {
            Ok(text) => Some(text),
            Err(error) => {
                if read_error.is_none() {
                    read_error = Some(anyhow::anyhow!(
                        "cannot read imported file {resolved} (imported by {path}): {error}"
                    ));
                }
                None
            }
        }
    };
    let import_strings = import_paths(&source);
    let domains = gather_domain_sources(&path, &source, import_paths, &mut fetch);
    if let Some(error) = read_error {
        return Err(error);
    }
    let schemas = collect_schema_imports(&domains)?;
    let sigil = collect_sigil_imports(&domains)?;
    Ok(DomainPair {
        path,
        source,
        import_keys: render_keys(file, &import_strings, &domains),
        domains,
        schemas,
        sigil,
    })
}

/// The render keys for a gathered domain set: each import string the
/// surface writes, mapped to the text its resolved gather fetched — the
/// driver tags a domain's diagnostics with the import string (its
/// `DomainUnit` path), and the reporter resolves sources through the pair.
fn render_keys(
    file: &Path,
    imports: &[String],
    domains: &[(String, String)],
) -> Vec<(String, String)> {
    let dir = dir_of(file);
    imports
        .iter()
        .filter_map(|import| {
            let key = dir.join(import).display().to_string();
            domains
                .iter()
                .find(|(name, _)| *name == key)
                .map(|(_, text)| (import.clone(), text.clone()))
        })
        .collect()
}

/// An `.actor` file plus the domain sources it imports, ready for
/// [`rex_driver::compile_actors_str`].
pub(crate) type ActorPair = DomainPair;

/// Reads an `.actor` file plus every domain it imports, without a project
/// context — gen's cedar/tools flows stay disk-only (gen is not part of
/// the check/build project mode).
pub(crate) fn read_actor_pair(file: &Path) -> anyhow::Result<ActorPair> {
    read_actor_pair_in(file, None)
}

/// [`read_actor_pair`] within a project context: the policy's domain
/// imports fetch map-first from the snapshot.
pub(crate) fn read_actor_pair_in(
    file: &Path,
    project: Option<&mut Project>,
) -> anyhow::Result<ActorPair> {
    read_domain_pair(
        file,
        |source| {
            rex_syntax::parse_actors(source)
                .ast
                .map(|ast| ast.imports.into_iter().map(|import| import.path).collect())
                .unwrap_or_default()
        },
        project,
    )
}

/// A `.ddd` design file plus the domain sources it imports, ready for
/// [`rex_driver::compile_ddd_str`].
pub(crate) type DddDesignPair = DomainPair;

/// Reads a `.ddd` file plus every domain it imports.
pub(crate) fn read_ddd_design(
    file: &Path,
    project: Option<&mut Project>,
) -> anyhow::Result<DddDesignPair> {
    read_domain_pair(
        file,
        |source| {
            rex_syntax::parse_ddd(source)
                .ast
                .map(|ast| ast.imports.into_iter().map(|import| import.path).collect())
                .unwrap_or_default()
        },
        project,
    )
}

/// An `.evt` contract file plus the domain sources it imports, ready for
/// [`rex_driver::compile_evt_str`].
pub(crate) type EvtPair = DomainPair;

/// Reads an `.evt` file plus every domain it imports.
pub(crate) fn read_evt_pair(file: &Path, project: Option<&mut Project>) -> anyhow::Result<EvtPair> {
    read_domain_pair(
        file,
        |source| {
            rex_syntax::parse_evt(source)
                .ast
                .map(|ast| ast.imports.into_iter().map(|import| import.path).collect())
                .unwrap_or_default()
        },
        project,
    )
}

/// A `.dmn` decision-model file and its compiled inputs, ready for
/// [`rex_driver::compile_dmn_str`]: the raw imported domains (in
/// [`DomainPair`]) plus the decision-table contents its `logic table`
/// clauses name.
pub(crate) struct DmnPair {
    /// The decision-model file plus its raw imported domains.
    pub(crate) pair: DomainPair,
    /// Each `logic table "<path>"` clause's markdown content, keyed by the
    /// path **as written** — the driver matches table paths exactly as
    /// written.
    pub(crate) tables: Vec<(String, String)>,
}

/// Reads a `.dmn` file plus every domain it imports; each `logic table
/// "<path>"` clause's markdown file is resolved relative to the `.dmn`
/// file's directory and read from disk. A table file that cannot be read
/// is simply not provided — the driver reports
/// `decision table "<path>" was not provided` with the written path.
pub(crate) fn read_dmn_pair(file: &Path, project: Option<&mut Project>) -> anyhow::Result<DmnPair> {
    let source = std::fs::read_to_string(file)
        .map_err(|error| anyhow::anyhow!("cannot read {}: {error}", file.display()))?;
    let tables = table_sources(file, &source);
    let pair = read_domain_pair(
        file,
        |source| {
            rex_syntax::parse_dmn(source)
                .ast
                .map(|ast| ast.imports.into_iter().map(|import| import.path).collect())
                .unwrap_or_default()
        },
        project,
    )?;
    Ok(DmnPair { pair, tables })
}

/// The decision-table contents a `.dmn` source names: every `logic table
/// "<path>"` clause of a decision, resolved relative to the `.dmn` file's
/// directory and read from disk, keyed by the path as written. A table
/// file that cannot be read is skipped (the driver's unprovided-table
/// diagnostic names it); duplicated table references are gathered once.
fn table_sources(file: &Path, source: &str) -> Vec<(String, String)> {
    let written: Vec<String> = rex_syntax::parse_dmn(source)
        .ast
        .and_then(|ast| ast.model)
        .map(|model| {
            model
                .decisions
                .iter()
                .filter_map(|decision| match &decision.logic {
                    Some(rex_syntax::DmnLogic::Table { source, .. }) => Some(source.clone()),
                    _ => None,
                })
                .collect()
        })
        .unwrap_or_default();
    let dir = dir_of(file);
    let mut tables: Vec<(String, String)> = Vec::new();
    for table in written {
        if tables.iter().any(|(name, _)| *name == table) {
            continue;
        }
        if let Ok(content) = std::fs::read_to_string(dir.join(&table)) {
            tables.push((table, content));
        }
    }
    tables
}

/// The plain disk-read fetch the driver gatherers take: the gatherer
/// hands back resolved paths, and a miss is simply `None` — the driver
/// renders the pinned missing-import message (`cannot read imported file
/// {resolved} (imported by {importer})`) itself.
fn fetch_disk(resolved: &str) -> Option<String> {
    std::fs::read_to_string(resolved).ok()
}

/// A `.uc` file and its imports, ready for
/// [`rex_driver::compile_usecase_str`]: the raw imported domains (in
/// [`DomainPair`]) plus the pre-compiled `.ddd`/`.actor`/`.evt` buckets.
pub(crate) struct UcPair {
    /// The use case file plus its raw imported domains.
    pub(crate) pair: DomainPair,
    /// Each imported `.ddd` design, compiled, keyed by its resolved path;
    /// the driver matches a `.uc` import by exact string or by lexical
    /// resolution relative to the use case file's directory (the `.actor`
    /// rule), so resolved paths work wherever the process runs from.
    pub(crate) designs: Vec<(String, rex_ir::ddd::DddModel)>,
    /// Each imported `.actor` policy, compiled, keyed by its resolved path.
    pub(crate) actors: Vec<(String, rex_ir::ActorModel)>,
    /// Each imported `.evt` contract, compiled, keyed by its resolved path.
    pub(crate) events: Vec<(String, rex_ir::events::EventModel)>,
}

/// Reads a `.uc` file plus every import it declares, gathered through
/// [`gather_usecase_imports`]: import paths resolve relative to the use
/// case file's own directory, bucketed by resolved extension (`.mox` raw,
/// `.ddd`/`.actor`/`.evt` compiled), duplicates gathered once. The
/// **resolved** path is passed to the driver. The fetch is map-first
/// within `project`, disk-second otherwise, and the disk fallback fills
/// the map. A missing import file is a clean error naming the resolved
/// path (the driver's message).
pub(crate) fn read_uc_pair(file: &Path, project: Option<&mut Project>) -> anyhow::Result<UcPair> {
    let source = std::fs::read_to_string(file)
        .map_err(|error| anyhow::anyhow!("cannot read {}: {error}", file.display()))?;
    let import_strings = rex_syntax::parse_usecases(&source)
        .ast
        .map(|ast| {
            ast.imports
                .into_iter()
                .map(|import| import.path)
                .collect::<Vec<String>>()
        })
        .unwrap_or_default();
    let mut fetch = project_fetch(project);
    let buckets = gather_usecase_imports(
        &file.display().to_string(),
        &source,
        |source| {
            rex_syntax::parse_usecases(source)
                .ast
                .map(|ast| ast.imports.into_iter().map(|import| import.path).collect())
                .unwrap_or_default()
        },
        &mut fetch,
    )
    .map_err(|message| anyhow::anyhow!(message))?;
    let schemas = collect_schema_imports(&buckets.domains)?;
    let sigil = collect_sigil_imports(&buckets.domains)?;
    Ok(UcPair {
        pair: DomainPair {
            path: file.display().to_string(),
            import_keys: render_keys(file, &import_strings, &buckets.domains),
            source,
            domains: buckets.domains,
            schemas,
            sigil,
        },
        designs: buckets.designs,
        actors: buckets.actors,
        events: buckets.events,
    })
}

/// A `.cmmn` file and its compiled imports: the case-management
/// counterpart of [`UcPair`] — domains, events, actors, and aliased
/// designs, each keyed by its resolved path.
pub(crate) struct CmmnPair {
    /// The case file plus its raw imported domains.
    pub(crate) pair: DomainPair,
    /// Each imported `.ddd` design, compiled, keyed by its resolved path.
    pub(crate) designs: Vec<(String, rex_ir::ddd::DddModel)>,
    /// Each imported `.actor` policy, compiled, keyed by its resolved path.
    pub(crate) actors: Vec<(String, rex_ir::ActorModel)>,
    /// Each imported `.evt` contract, compiled, keyed by its resolved path.
    pub(crate) events: Vec<(String, rex_ir::events::EventModel)>,
}

/// Reads a `.cmmn` file and gathers its imports exactly like
/// [`read_uc_pair`], through [`gather_usecase_imports`]: paths relative to
/// the case file's directory, bucketed by resolved extension (`.mox` raw,
/// `.ddd`/`.actor`/`.evt` compiled). Unknown extensions pass through so
/// the driver reports them.
pub(crate) fn read_cmmn_pair(
    file: &Path,
    project: Option<&mut Project>,
) -> anyhow::Result<CmmnPair> {
    let source = std::fs::read_to_string(file)
        .map_err(|error| anyhow::anyhow!("cannot read {}: {error}", file.display()))?;
    let import_strings = rex_syntax::parse_cmmn(&source)
        .ast
        .map(|ast| {
            ast.imports
                .into_iter()
                .map(|import| import.path)
                .collect::<Vec<String>>()
        })
        .unwrap_or_default();
    let mut fetch = project_fetch(project);
    let buckets = gather_usecase_imports(
        &file.display().to_string(),
        &source,
        |source| {
            rex_syntax::parse_cmmn(source)
                .ast
                .map(|ast| ast.imports.into_iter().map(|import| import.path).collect())
                .unwrap_or_default()
        },
        &mut fetch,
    )
    .map_err(|message| anyhow::anyhow!(message))?;
    let schemas = collect_schema_imports(&buckets.domains)?;
    let sigil = collect_sigil_imports(&buckets.domains)?;
    Ok(CmmnPair {
        pair: DomainPair {
            path: file.display().to_string(),
            import_keys: render_keys(file, &import_strings, &buckets.domains),
            source,
            domains: buckets.domains,
            schemas,
            sigil,
        },
        designs: buckets.designs,
        actors: buckets.actors,
        events: buckets.events,
    })
}

/// A `.deploy` file plus the pre-compiled design and flow artifacts its
/// imports resolve to, ready for [`rex_driver::compile_deploy_str`].
pub(crate) struct DeployPair {
    /// The deployment file's path as given on the command line.
    pub(crate) path: String,
    /// The deployment file's source text.
    pub(crate) source: String,
    /// Each imported `.ddd` design, compiled, keyed by its resolved path;
    /// the driver matches a deploy import by exact string or by lexical
    /// resolution relative to the deploy file's directory (the `.actor`
    /// rule), so resolved paths work wherever the process runs from.
    pub(crate) designs: Vec<(String, rex_ir::ddd::DddModel)>,
    /// Each imported `.ifml` flow, compiled, keyed by its resolved path.
    pub(crate) flows: Vec<(String, rex_ir::ifml::IfmlModel)>,
}

impl DeployPair {
    /// The source text of the file with the given driver path — the deploy
    /// file itself (import artifacts arrive pre-compiled, so their
    /// diagnostics were rendered when they were gathered).
    pub(crate) fn source_of(&self, path: &str) -> Option<&str> {
        (path == self.path).then_some(self.source.as_str())
    }
}

/// Reads a `.deploy` file plus every `.ddd` design and `.ifml` flow it
/// imports, each compiled to its artifact — gathered through
/// [`gather_deploy_imports`]. Import paths resolve relative to the deploy
/// file's own directory; the **resolved** path is passed to the driver.
/// The fetch is map-first within `project`, disk-second otherwise, and
/// the disk fallback fills the map. A missing or non-compiling import is
/// a clean error; duplicate imports are gathered once.
pub(crate) fn read_deploy_pair(
    file: &Path,
    project: Option<&mut Project>,
) -> anyhow::Result<DeployPair> {
    let source = std::fs::read_to_string(file)
        .map_err(|error| anyhow::anyhow!("cannot read {}: {error}", file.display()))?;
    let mut fetch = project_fetch(project);
    let pairs = gather_deploy_imports(
        &file.display().to_string(),
        &source,
        |source| {
            rex_syntax::parse_deploy(source)
                .ast
                .map(|ast| ast.imports.into_iter().map(|import| import.path).collect())
                .unwrap_or_default()
        },
        &mut fetch,
    )
    .map_err(|message| anyhow::anyhow!(message))?;
    Ok(DeployPair {
        path: file.display().to_string(),
        source,
        designs: pairs.designs,
        flows: pairs.flows,
    })
}

/// The fetch closure the bucket gatherers take
/// (`FnMut(&str) -> Option<String>`): map-first within `project` (the
/// driver renders the pinned missing-import message itself, so a miss is
/// simply `None` here — no io error to name), plain disk otherwise. The
/// disk fallback fills the map.
fn project_fetch(mut project: Option<&mut Project>) -> impl FnMut(&str) -> Option<String> + '_ {
    move |resolved: &str| -> Option<String> {
        match project.as_deref_mut() {
            Some(project) => project.fetch(resolved).ok(),
            None => fetch_disk(resolved),
        }
    }
}
