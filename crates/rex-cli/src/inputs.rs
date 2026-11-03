//! Turning command-line inputs into in-memory sources and driver
//! compilations: input expansion, source reading, the
//! `.actor`/`.ddd`/`.evt`/`.deploy`/`.uc` readers, and the shared compile
//! step over collected sources.

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

/// Expands `rexlang check` inputs: directories scan recursively for **every
/// supported surface** (`.mox`, `.actor`, `.ddd`, `.evt`, `.uc`, `.ifml`),
/// not just `.mox` — formatter-golden `X.fmt.<ext>` artifacts excepted (see
/// [`is_formatter_golden`]). Used by the directory batch mode, where each
/// expanded file is checked independently; explicit file arguments pass
/// through as-is.
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
        &["mox", "actor", "ddd", "evt", "uc", "cmmn", "ifml", "deploy"],
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
    /// Every imported `.actor` policy, compiled — the authorization
    /// dimension [`rex_ifml::check_ifml_with_actors`] resolves
    /// `requires:`/`roles:` against. Empty when the file imports no actor
    /// models (authorization checking is then skipped).
    pub(crate) actors: Vec<rex_ir::ActorModel>,
}

/// Reads an `.ifml` file plus every `.ifml`, `.mox`, and `.actor` file it
/// imports, transitively. Import paths resolve relative to the importing
/// file's own directory on disk, while the resolver keys each import by
/// the importing file's identity and the import string **as written** (the
/// resolver is filesystem-free and sees only the strings). Imported `.mox`
/// files are compiled to Core IR and unioned for typed expression
/// binding; imported `.actor` files are compiled to actor models for
/// authorization checking; a domain or actor model that fails to compile
/// is a hard error. A missing import file is a clean error naming the
/// resolved path.
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
        actors: Vec::new(),
    };
    let mut domain_sources: Vec<(String, String)> = Vec::new();
    let mut actor_models: Vec<rex_ir::ActorModel> = Vec::new();
    collect_ifml_imports(
        &path,
        file,
        &inputs.source,
        &mut inputs.ifml_imports,
        &mut inputs.sources,
        &mut domain_sources,
        &mut actor_models,
    )?;
    inputs.actors = actor_models;
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
/// domains (deduplicated by resolved path) plus compiled `.actor` models
/// as a side effect. The first unreadable import surfaces as an error
/// after the walk. Disk paths are tracked per identity here — the walker
/// only ever sees identities.
fn collect_ifml_imports(
    identity: &str,
    disk_path: &Path,
    source: &str,
    bundle: &mut rex_ifml::IfmlImports,
    sources: &mut Vec<(String, String)>,
    domains: &mut Vec<(String, String)>,
    actors: &mut Vec<rex_ir::ActorModel>,
) -> anyhow::Result<()> {
    let mut disk_paths: BTreeMap<String, PathBuf> = BTreeMap::new();
    disk_paths.insert(identity.to_string(), disk_path.to_path_buf());
    let mut read_error: Option<anyhow::Error> = None;
    let mut actor_paths: Vec<String> = Vec::new();
    let dir_of = |path: &Path| -> PathBuf {
        path.parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."))
            .to_path_buf()
    };
    let walk = rex_ifml::walk_ifml_imports(identity, source, &mut |importer, import| {
        let importer_disk = disk_paths.get(importer)?;
        let resolved = dir_of(importer_disk).join(import);
        let resolved_path = resolved.display().to_string();
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
        } else if import.ends_with(".actor") {
            if actor_paths
                .iter()
                .any(|existing| *existing == resolved_path)
            {
                return None;
            }
            match read_actor_model(&resolved) {
                Ok(model) => {
                    actor_paths.push(resolved_path);
                    actors.push(model);
                    None
                }
                Err(error) => {
                    if read_error.is_none() {
                        read_error = Some(error);
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
    /// the surface file, an imported domain, or a rosetta source.
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

/// Reads a surface file plus every domain it imports; `import_paths`
/// yields the file's raw (unresolved) import path strings and is empty
/// when the file does not parse — the driver reports that.
///
/// Import paths resolve relative to the importing file's own directory,
/// and the **resolved** path is passed to the driver (the driver matches
/// imports by exact string or by lexical resolution, and the domain's
/// driver path also locates its `vocab/` directory — with raw import
/// strings that lookup would be relative to the process CWD). A missing
/// import file is a clean error naming the resolved path. Duplicate
/// imports are read once.
fn read_domain_pair(
    file: &Path,
    import_paths: impl Fn(&str) -> Vec<String>,
) -> anyhow::Result<DomainPair> {
    let source = std::fs::read_to_string(file)
        .map_err(|error| anyhow::anyhow!("cannot read {}: {error}", file.display()))?;
    let path = file.display().to_string();
    let mut domains: Vec<(String, String)> = Vec::new();
    let dir = file
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    for import in import_paths(&source) {
        if domains.iter().any(|(existing, _)| *existing == import) {
            continue;
        }
        let resolved = dir.join(&import);
        let text = std::fs::read_to_string(&resolved).map_err(|error| {
            anyhow::anyhow!(
                "cannot read imported file {} (imported by {}): {error}",
                resolved.display(),
                path
            )
        })?;
        domains.push((resolved.display().to_string(), text));
    }
    let schemas = collect_schema_imports(&domains)?;
    let sigil = collect_sigil_imports(&domains)?;
    Ok(DomainPair {
        path,
        source,
        domains,
        schemas,
        sigil,
    })
}

/// An `.actor` file plus the domain sources it imports, ready for
/// [`rex_driver::compile_actors_str`].
pub(crate) type ActorPair = DomainPair;

/// Reads an `.actor` file plus every domain it imports.
pub(crate) fn read_actor_pair(file: &Path) -> anyhow::Result<ActorPair> {
    read_domain_pair(file, |source| {
        rex_syntax::parse_actors(source)
            .ast
            .map(|ast| ast.imports.into_iter().map(|import| import.path).collect())
            .unwrap_or_default()
    })
}

/// A `.ddd` design file plus the domain sources it imports, ready for
/// [`rex_driver::compile_ddd_str`].
pub(crate) type DddDesignPair = DomainPair;

/// Reads a `.ddd` file plus every domain it imports.
pub(crate) fn read_ddd_design(file: &Path) -> anyhow::Result<DddDesignPair> {
    read_domain_pair(file, |source| {
        rex_syntax::parse_ddd(source)
            .ast
            .map(|ast| ast.imports.into_iter().map(|import| import.path).collect())
            .unwrap_or_default()
    })
}

/// An `.evt` contract file plus the domain sources it imports, ready for
/// [`rex_driver::compile_evt_str`].
pub(crate) type EvtPair = DomainPair;

/// Reads an `.evt` file plus every domain it imports.
pub(crate) fn read_evt_pair(file: &Path) -> anyhow::Result<EvtPair> {
    read_domain_pair(file, |source| {
        rex_syntax::parse_evt(source)
            .ast
            .map(|ast| ast.imports.into_iter().map(|import| import.path).collect())
            .unwrap_or_default()
    })
}

/// The raw `.mox` domains plus the pre-compiled `.ddd`/`.actor`/`.evt`
/// imports of a `.uc`/`.cmmn` file, keyed by resolved path.
pub(crate) struct GatheredImports {
    /// Each imported `.mox` domain as `(resolved path, source)`.
    pub(crate) domains: Vec<(String, String)>,
    /// Each imported `.ddd` design, compiled.
    pub(crate) designs: Vec<(String, rex_ir::ddd::DddModel)>,
    /// Each imported `.actor` policy, compiled.
    pub(crate) actors: Vec<(String, rex_ir::ActorModel)>,
    /// Each imported `.evt` contract, compiled.
    pub(crate) events: Vec<(String, rex_ir::events::EventModel)>,
}

impl GatheredImports {
    /// Reads `file`'s imports — `import_paths` yields the file's raw
    /// import path strings — bucketed by resolved extension: `.mox` is
    /// kept raw, `.ddd`/`.actor`/`.evt` are compiled to their artifacts,
    /// and unknown extensions pass through so the driver reports them.
    /// Duplicate paths are gathered once.
    fn gather(
        file: &Path,
        source: &str,
        import_paths: impl Fn(&str) -> Vec<String>,
    ) -> anyhow::Result<GatheredImports> {
        let mut gathered = GatheredImports {
            domains: Vec::new(),
            designs: Vec::new(),
            actors: Vec::new(),
            events: Vec::new(),
        };
        let path = file.display().to_string();
        let dir = file
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        for import in import_paths(source) {
            let resolved = dir.join(&import);
            let resolved_path = resolved.display().to_string();
            match resolved
                .extension()
                .and_then(|extension| extension.to_str())
            {
                Some("mox") => {
                    if gathered
                        .domains
                        .iter()
                        .any(|(existing, _)| *existing == resolved_path)
                    {
                        continue;
                    }
                    let text = std::fs::read_to_string(&resolved).map_err(|error| {
                        anyhow::anyhow!(
                            "cannot read imported file {} (imported by {}): {error}",
                            resolved.display(),
                            path
                        )
                    })?;
                    gathered.domains.push((resolved_path, text));
                }
                Some("ddd") => {
                    if gathered
                        .designs
                        .iter()
                        .any(|(existing, _)| *existing == resolved_path)
                    {
                        continue;
                    }
                    let model = read_design_model(&resolved)?;
                    gathered.designs.push((resolved_path, model));
                }
                Some("actor") => {
                    if gathered
                        .actors
                        .iter()
                        .any(|(existing, _)| *existing == resolved_path)
                    {
                        continue;
                    }
                    let model = read_actor_model(&resolved)?;
                    gathered.actors.push((resolved_path, model));
                }
                Some("evt") => {
                    if gathered
                        .events
                        .iter()
                        .any(|(existing, _)| *existing == resolved_path)
                    {
                        continue;
                    }
                    let model = read_event_model(&resolved)?;
                    gathered.events.push((resolved_path, model));
                }
                _ => {}
            }
        }
        Ok(gathered)
    }
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

/// Reads a `.uc` file plus every import it declares.
///
/// Import paths resolve relative to the use case file's own directory,
/// bucketed by extension exactly as [`GatheredImports::gather`] describes;
/// the **resolved** path is passed to the driver. A missing import file is
/// a clean error naming the resolved path. Duplicate imports are read once.
pub(crate) fn read_uc_pair(file: &Path) -> anyhow::Result<UcPair> {
    let source = std::fs::read_to_string(file)
        .map_err(|error| anyhow::anyhow!("cannot read {}: {error}", file.display()))?;
    let gathered = GatheredImports::gather(file, &source, |source| {
        rex_syntax::parse_usecases(source)
            .ast
            .map(|ast| ast.imports.into_iter().map(|import| import.path).collect())
            .unwrap_or_default()
    })?;
    let schemas = collect_schema_imports(&gathered.domains)?;
    let sigil = collect_sigil_imports(&gathered.domains)?;
    Ok(UcPair {
        pair: DomainPair {
            path: file.display().to_string(),
            source,
            domains: gathered.domains,
            schemas,
            sigil,
        },
        designs: gathered.designs,
        actors: gathered.actors,
        events: gathered.events,
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
/// [`read_uc_pair`]: paths relative to the case file's directory, bucketed
/// by extension (`.mox` raw, `.ddd`/`.actor`/`.evt` compiled). Unknown
/// extensions pass through so the driver reports them.
pub(crate) fn read_cmmn_pair(file: &Path) -> anyhow::Result<CmmnPair> {
    let source = std::fs::read_to_string(file)
        .map_err(|error| anyhow::anyhow!("cannot read {}: {error}", file.display()))?;
    let gathered = GatheredImports::gather(file, &source, |source| {
        rex_syntax::parse_cmmn(source)
            .ast
            .map(|ast| ast.imports.into_iter().map(|import| import.path).collect())
            .unwrap_or_default()
    })?;
    let schemas = collect_schema_imports(&gathered.domains)?;
    let sigil = collect_sigil_imports(&gathered.domains)?;
    Ok(CmmnPair {
        pair: DomainPair {
            path: file.display().to_string(),
            source,
            domains: gathered.domains,
            schemas,
            sigil,
        },
        designs: gathered.designs,
        actors: gathered.actors,
        events: gathered.events,
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
/// imports, each compiled to its artifact. Import paths resolve relative to
/// the deploy file's own directory; the **resolved** path is passed to the
/// driver. A missing or non-compiling import is a clean error. Duplicate
/// imports are read once.
pub(crate) fn read_deploy_pair(file: &Path) -> anyhow::Result<DeployPair> {
    let source = std::fs::read_to_string(file)
        .map_err(|error| anyhow::anyhow!("cannot read {}: {error}", file.display()))?;
    let path = file.display().to_string();
    let mut designs: Vec<(String, rex_ir::ddd::DddModel)> = Vec::new();
    let mut flows: Vec<(String, rex_ir::ifml::IfmlModel)> = Vec::new();
    if let Some(ast) = &rex_syntax::parse_deploy(&source).ast {
        let dir = file
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        for import in &ast.imports {
            let resolved = dir.join(&import.path);
            let resolved_path = resolved.display().to_string();
            if import.path.ends_with(".ddd") {
                if designs
                    .iter()
                    .any(|(existing, _)| *existing == resolved_path)
                {
                    continue;
                }
                let model = read_design_model(&resolved)?;
                designs.push((resolved_path, model));
            } else if import.path.ends_with(".ifml") {
                if flows.iter().any(|(existing, _)| *existing == resolved_path) {
                    continue;
                }
                let model = read_flow_model(&resolved)?;
                flows.push((resolved_path, model));
            }
        }
    }
    Ok(DeployPair {
        path,
        source,
        designs,
        flows,
    })
}

/// Reads and compiles one imported `.ddd` design (its own domain imports
/// resolve relative to it, exactly like `rexlang check` on the design).
fn read_design_model(file: &Path) -> anyhow::Result<rex_ir::ddd::DddModel> {
    let design = read_ddd_design(file)?;
    let compilation = rex_driver::compile_ddd_str(
        &design.path,
        &design.source,
        &design.domains,
        &design.imports(),
    );
    let Some(model) = compilation.model else {
        let messages: Vec<String> = compilation
            .diagnostics
            .iter()
            .filter(|(_, diagnostic)| diagnostic.is_error())
            .map(|(path, diagnostic)| format!("{path}: {}", diagnostic.message))
            .collect();
        anyhow::bail!(
            "imported design {} does not compile{}",
            file.display(),
            if messages.is_empty() {
                String::new()
            } else {
                format!(":\n  {}", messages.join("\n  "))
            }
        );
    };
    Ok(model)
}

/// Reads and compiles one imported `.actor` policy (its own domain imports
/// resolve relative to it, exactly like `rexlang check` on the policy).
fn read_actor_model(file: &Path) -> anyhow::Result<rex_ir::ActorModel> {
    let actors = read_actor_pair(file)?;
    let compilation = rex_driver::compile_actors_str(
        &actors.path,
        &actors.source,
        &actors.domains,
        &actors.imports(),
    );
    let Some(model) = compilation.model else {
        let messages: Vec<String> = compilation
            .diagnostics
            .iter()
            .filter(|(_, diagnostic)| diagnostic.is_error())
            .map(|(path, diagnostic)| format!("{path}: {}", diagnostic.message))
            .collect();
        anyhow::bail!(
            "imported actor model {} does not compile{}",
            file.display(),
            if messages.is_empty() {
                String::new()
            } else {
                format!(":\n  {}", messages.join("\n  "))
            }
        );
    };
    Ok(model)
}

/// Reads and compiles one imported `.evt` contract (its own domain imports
/// resolve relative to it, exactly like `rexlang check` on the contract).
fn read_event_model(file: &Path) -> anyhow::Result<rex_ir::events::EventModel> {
    let contract = read_evt_pair(file)?;
    let compilation = rex_driver::compile_evt_str(
        &contract.path,
        &contract.source,
        &contract.domains,
        &contract.imports(),
    );
    let Some(model) = compilation.model else {
        let messages: Vec<String> = compilation
            .diagnostics
            .iter()
            .filter(|(_, diagnostic)| diagnostic.is_error())
            .map(|(path, diagnostic)| format!("{path}: {}", diagnostic.message))
            .collect();
        anyhow::bail!(
            "imported contract {} does not compile{}",
            file.display(),
            if messages.is_empty() {
                String::new()
            } else {
                format!(":\n  {}", messages.join("\n  "))
            }
        );
    };
    Ok(model)
}

/// Reads and compiles one imported `.ifml` flow (its `.ifml` imports walk
/// relative to it, exactly like `rexlang check` on the flow; domain typing
/// is not needed for binding-name validation). The flow's `.actor`
/// imports resolve the same way — a missing or non-compiling policy is
/// the same clean error — but the deploy lowering consumes no actor
/// models, so they are discarded here.
fn read_flow_model(file: &Path) -> anyhow::Result<rex_ir::ifml::IfmlModel> {
    let source = std::fs::read_to_string(file)
        .map_err(|error| anyhow::anyhow!("cannot read {}: {error}", file.display()))?;
    let path = file.display().to_string();
    let mut imports = rex_ifml::IfmlImports::default();
    let mut sources = Vec::new();
    let mut domains = Vec::new();
    let mut actors = Vec::new();
    collect_ifml_imports(
        &path,
        file,
        &source,
        &mut imports,
        &mut sources,
        &mut domains,
        &mut actors,
    )?;
    match rex_ifml::compile_ifml_str(&path, &source, &imports) {
        Ok(compilation) => Ok(compilation.model),
        Err(_) => anyhow::bail!("imported flow {} does not compile", file.display()),
    }
}
