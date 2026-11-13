//! The shared host-side project resolver: cross-surface import gathering
//! in one filesystem-free home — the `workspace_imports` pattern
//! generalized from `import schema`/`import sigil` content to **every**
//! surface's import closure (Phase B of #117).
//!
//! The driver core stays filesystem-free: every `compile_*` entry point
//! receives texts through [`crate::DomainImports`] and pre-compiled
//! artifact slices. Before Phase B every host re-implemented the same
//! gather — the CLI's readers in `rex-cli/src/inputs.rs`, the LSP's
//! closures in its publish pipeline, and a small army of per-test fetch
//! closures. This module is the substrate those hosts converge on: give
//! it an entry source and a *fetch closure* that resolves one import
//! string to its content, and it returns the gathered project — raw
//! domain sources, compiled actor/design/event/flow artifacts, the
//! [`rex_ifml::IfmlImports`] bundle a flow compilation resolves against,
//! and everything the fetch could not provide.
//!
//! # The two fetch shapes
//!
//! - **Two-key** (`.ifml` walks): `FnMut(&str, &str) ->
//!   Option<(String, String)>` — `(importer identity, import as written)
//!   -> Option<(host-resolved key, text)>`. The host-resolved key is the
//!   identity the host actually resolved the import under (a disk path, a
//!   document URI); the walker and the bundle keys keep the as-written
//!   strings. See [`rex_ifml::walk_ifml_imports`] for the host
//!   obligation: every resolved key handed out may later arrive back as
//!   an *importer* identity, so hosts must map each key they return the
//!   same way they map the entry identity.
//! - **Path-keyed** (`.mox`/`.ddd`/`.actor`/`.evt`/`.uc`/`.deploy`
//!   gathering): `FnMut(&str) -> Option<String>` — resolved path ->
//!   text. Here the caller resolves import strings relative to the
//!   importing file's directory first (`dir_of(importer).join(import)`,
//!   the CLI rule), so the fetch always sees a resolved path and dedup is
//!   naturally by resolved key.
//!
//! # Message discipline
//!
//! Messages that today's CLI tests pin are reproduced verbatim:
//! ``cannot read imported file {resolved} (imported by {importer})``,
//! ``imported domain/design/actor model/contract/flow {key} does not
//! compile`` (with the `:\n  {path}: {message}` diagnostic lines where
//! the readers render them). The path-keyed gatherers hard-error on the
//! first missing import (the CLI's `read_*` discipline); [`gather_domain_sources`]
//! skips missing imports silently (the LSP's degrade-gracefully
//! discipline — hosts wanting hard errors capture them in their own
//! closure side-channel and keep their own messages). [`IfmlProject::unresolved`]
//! is deliberately *not* an error: what to do about an unfetched import
//! is host policy (the CLI hard-errors, the LSP degrades).

use std::collections::BTreeMap;

use crate::engine_common::dir_of;
use crate::workspace_imports::{collect_schema_imports, collect_sigil_imports};
use crate::{
    compile_actors_str, compile_ddd_str, compile_dmn_str, compile_evt_str, compile_str, render,
    Diagnostic, DomainImports,
};

/// One imported `.actor` policy as gathered from a project: the source
/// text plus the `.mox` domains its capabilities are granted on, ready
/// for [`compile_actor_imports`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActorImport {
    /// Host-resolved key the policy was gathered under (dedup identity).
    pub key: String,
    /// The policy source text.
    pub source: String,
    /// The policy's own `.mox` imports as `(resolved key, text)` — the
    /// classes its capabilities are granted on. Dedup by key.
    pub domains: Vec<(String, String)>,
}

/// One imported `.dmn` decision model as gathered from a project: the
/// source text plus the `.mox` domains its field types resolve against
/// and the decision-table contents its `logic table` clauses name, ready
/// for [`compile_decision_imports`] — the [`ActorImport`] shape for the
/// decision dimension.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecisionImport {
    /// Host-resolved key the model was gathered under (dedup identity).
    pub key: String,
    /// The model source text.
    pub source: String,
    /// The model's own `.mox` imports as `(resolved key, text)` — the
    /// types its inputs, outputs, and parameters resolve against. Dedup
    /// by key.
    pub domains: Vec<(String, String)>,
    /// Each `logic table "<path>"` clause's markdown content, keyed by
    /// the path **as written** — the driver matches table paths exactly
    /// as written. A table that cannot be read is simply absent: the
    /// driver's unprovided-table diagnostic names it.
    pub tables: Vec<(String, String)>,
    /// The model's imported `.evt` contracts, compiled and keyed by the
    /// import string **as written** (the emits dimension). A contract
    /// that cannot be read or compiled is simply absent: the DMN rule-1
    /// diagnostic names it.
    pub events: Vec<(String, rex_ir::events::EventModel)>,
}

/// The import closure of one `.ifml` entry file, gathered: everything a
/// host needs to compile, type-check, and authorize the flow — and the
/// raw material later streams re-key onto their own host shapes.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct IfmlProject {
    /// Resolver bundle keyed `(importer identity, import-as-written)` —
    /// provided for every fetched `.ifml` import, exactly what
    /// [`rex_ifml::compile_ifml_str`]/[`rex_ifml::compile_ifml_lenient`]
    /// resolve against.
    pub imports: rex_ifml::IfmlImports,
    /// Every walked `.ifml` source `(identity, text)`, entry file first.
    pub sources: Vec<(String, String)>,
    /// Imported `.mox` domain sources `(resolved key, text)`,
    /// deduplicated by resolved key, in walk order.
    pub domains: Vec<(String, String)>,
    /// Imported `.actor` policies with their own gathered domains.
    pub actors: Vec<ActorImport>,
    /// Imported `.dmn` decision models with their own gathered domains
    /// and decision-table contents.
    pub decisions: Vec<DecisionImport>,
    /// Declared `.ifml`/`.mox`/`.actor` imports the fetch provided
    /// nothing for, as `(importer identity, import-as-written)`. Other
    /// extensions are silently ignored (parity with today's
    /// pass-through). What to do about these is host policy (the CLI
    /// hard-errors, the LSP degrades).
    pub unresolved: Vec<(String, String)>,
}

/// Gathers the full import closure of one `.ifml` entry file through
/// [`rex_ifml::walk_ifml_imports`], interposing a classifying closure
/// around the host's two-key fetch: `.ifml` imports feed the resolver
/// bundle under the walker's own keys and recurse; `.mox` imports land in
/// [`IfmlProject::domains`] (deduplicated by resolved key); `.actor`
/// imports become [`ActorImport`]s whose own `.mox` imports are fetched
/// under the policy's resolved key (the host obligation the walker's
/// contract states); `.dmn` imports become [`DecisionImport`]s whose own
/// `.mox` imports and decision-table contents are fetched under the
/// model's resolved key (tables skip silently when unreadable — the
/// driver's unprovided-table diagnostic names them). Imports the fetch
/// declines land in
/// [`IfmlProject::unresolved`]; unknown extensions pass through ignored.
///
/// The host fetch must map every resolved key it hands out — nested
/// gathering calls it back with an actor's resolved key as the importer
/// identity.
#[allow(clippy::type_complexity)]
pub fn gather_ifml_project(
    entry_identity: &str,
    entry_source: &str,
    fetch: &mut dyn FnMut(&str, &str) -> Option<(String, String)>,
) -> IfmlProject {
    let mut project = IfmlProject::default();
    let mut classify = |importer: &str, import: &str| -> Option<(String, String)> {
        let Some((key, text)) = fetch(importer, import) else {
            if is_imported_surface(import) {
                project
                    .unresolved
                    .push((importer.to_string(), import.to_string()));
            }
            return None;
        };
        if import.ends_with(".ifml") {
            project.imports.provide(importer, import, text.clone());
            Some((key, text))
        } else if import.ends_with(".mox") {
            if !project.domains.iter().any(|(existing, _)| *existing == key) {
                project.domains.push((key, text));
            }
            None
        } else if import.ends_with(".actor") {
            if !project.actors.iter().any(|actor| actor.key == key) {
                let mut actor = ActorImport {
                    key: key.clone(),
                    source: text.clone(),
                    domains: Vec::new(),
                };
                let imports = rex_syntax::parse_actors(&text)
                    .ast
                    .map(|ast| {
                        ast.imports
                            .into_iter()
                            .map(|import| import.path)
                            .collect::<Vec<String>>()
                    })
                    .unwrap_or_default();
                for path in imports {
                    match fetch(&key, &path) {
                        Some((domain_key, domain_text)) => {
                            if !actor
                                .domains
                                .iter()
                                .any(|(existing, _)| *existing == domain_key)
                            {
                                actor.domains.push((domain_key, domain_text));
                            }
                        }
                        None => project.unresolved.push((key.clone(), path)),
                    }
                }
                project.actors.push(actor);
            }
            None
        } else if import.ends_with(".dmn") {
            if !project.decisions.iter().any(|decision| decision.key == key) {
                let imports = rex_syntax::parse_dmn(&text)
                    .ast
                    .map(|ast| {
                        ast.imports
                            .into_iter()
                            .map(|import| import.path)
                            .collect::<Vec<String>>()
                    })
                    .unwrap_or_default();
                let mut decision = DecisionImport {
                    key: key.clone(),
                    source: text.clone(),
                    domains: Vec::new(),
                    tables: Vec::new(),
                    events: Vec::new(),
                };
                // The model's `.mox` imports gather under its resolved key
                // (the types its inputs and outputs resolve against); a
                // missing one is unresolved, exactly like an actor's.
                for path in imports {
                    match fetch(&key, &path) {
                        Some((domain_key, domain_text)) => {
                            if !decision
                                .domains
                                .iter()
                                .any(|(existing, _)| *existing == domain_key)
                            {
                                decision.domains.push((domain_key, domain_text));
                            }
                        }
                        None => project.unresolved.push((key.clone(), path)),
                    }
                }
                // The decision-table contents gather under the model's
                // resolved key too; an unreadable one is simply not
                // provided — the driver's unprovided-table diagnostic
                // names the written path (the `read_dmn_pair` discipline,
                // not the hard read-error the surface imports carry).
                let tables = rex_syntax::parse_dmn(&text)
                    .ast
                    .and_then(|ast| ast.model)
                    .map(|model| {
                        model
                            .decisions
                            .iter()
                            .filter_map(|decision| match &decision.logic {
                                Some(rex_syntax::DmnLogic::Table { source, .. }) => {
                                    Some(source.clone())
                                }
                                _ => None,
                            })
                            .collect::<Vec<String>>()
                    })
                    .unwrap_or_default();
                // The model's `.evt` imports compile into the events
                // bucket (the emits dimension); an unreadable or
                // non-compiling contract is absent — the DMN rule-1
                // diagnostic names the authored import string.
                let imports = rex_syntax::parse_dmn(&text)
                    .ast
                    .map(|ast| {
                        ast.imports
                            .iter()
                            .map(|import| import.path.clone())
                            .collect::<Vec<String>>()
                    })
                    .unwrap_or_default();
                for import in imports {
                    if !import.ends_with(".evt")
                        || decision.events.iter().any(|(existing, _)| *existing == import)
                    {
                        continue;
                    }
                    if let Some((event_key, event_text)) = fetch(&key, &import) {
                        let compilation = compile_evt_str(
                            &event_key,
                            &event_text,
                            &decision.domains,
                            &DomainImports::default(),
                        );
                        if let Some(model) = compilation.model {
                            decision.events.push((import, model));
                        }
                    }
                }
                for table in tables {
                    if decision
                        .tables
                        .iter()
                        .any(|(existing, _)| *existing == table)
                    {
                        continue;
                    }
                    if let Some((_, content)) = fetch(&key, &table) {
                        decision.tables.push((table, content));
                    }
                }
                project.decisions.push(decision);
            }
            None
        } else {
            None
        }
    };
    let walk = rex_ifml::walk_ifml_imports(entry_identity, entry_source, &mut classify);
    project.sources = walk.sources;
    project
}

/// Compiles every gathered `.actor` policy in order against its own
/// gathered domains — the CLI's `read_actor_model` discipline. The first
/// policy that fails to compile wins:
/// ``imported actor model {key} does not compile`` plus, when error
/// diagnostics exist, one ``  {path}: {message}`` line each.
pub fn compile_actor_imports(actors: &[ActorImport]) -> Result<Vec<rex_ir::ActorModel>, String> {
    let mut models = Vec::new();
    for actor in actors {
        let schemas = collect_schema_imports(&actor.domains).map_err(|error| error.to_string())?;
        let sigil = collect_sigil_imports(&actor.domains).map_err(|error| error.to_string())?;
        let compilation = compile_actors_str(
            &actor.key,
            &actor.source,
            &actor.domains,
            &DomainImports {
                schemas,
                sigil: sigil.imports,
            },
        );
        let Some(model) = compilation.model else {
            return Err(does_not_compile(
                "imported actor model",
                &actor.key,
                &compilation.diagnostics,
            ));
        };
        models.push(model);
    }
    Ok(models)
}

/// Compiles every gathered `.dmn` decision model in order against its own
/// gathered domains and decision tables — the [`compile_actor_imports`]
/// discipline for the decision dimension. The first model that fails to
/// compile wins:
/// ``imported decision model {key} does not compile`` plus, when error
/// diagnostics exist, one ``  {path}: {message}`` line each.
pub fn compile_decision_imports(
    decisions: &[DecisionImport],
) -> Result<Vec<rex_ir::dmn::DmnModel>, String> {
    let mut models = Vec::new();
    for decision in decisions {
        let schemas =
            collect_schema_imports(&decision.domains).map_err(|error| error.to_string())?;
        let sigil = collect_sigil_imports(&decision.domains).map_err(|error| error.to_string())?;
        let compilation = compile_dmn_str(
            &decision.key,
            &decision.source,
            &decision.domains,
            &DomainImports {
                schemas,
                sigil: sigil.imports,
            },
            &decision.tables,
            &decision.events,
        );
        let Some(model) = compilation.model else {
            return Err(does_not_compile(
                "imported decision model",
                &decision.key,
                &compilation.diagnostics,
            ));
        };
        models.push(model);
    }
    Ok(models)
}

/// Compiles every gathered `.mox` domain and unions the packages into one
/// Core IR model — the union [`rex_ifml::check_ifml`] binds expressions
/// against. Any diagnostics in any domain fail the whole union:
/// ``imported domain {path} does not compile:\n{rendered}`` (rendered via
/// [`crate::render`], the CLI's preview). The empty input is the valid
/// empty union — what empty means is host policy.
pub fn compile_domain_union(domains: &[(String, String)]) -> Result<rex_ir::Model, String> {
    let mut union = rex_ir::Model::new();
    if domains.is_empty() {
        return Ok(union);
    }
    let schemas = collect_schema_imports(domains).map_err(|error| error.to_string())?;
    let sigil = collect_sigil_imports(domains).map_err(|error| error.to_string())?;
    let imports = DomainImports {
        schemas,
        sigil: sigil.imports,
    };
    for (path, source) in domains {
        let compilation = compile_str(path, source, &imports);
        if !compilation.diagnostics.is_empty() {
            let rendered = render(path, source, &compilation.diagnostics);
            return Err(format!(
                "imported domain {path} does not compile:\n{rendered}"
            ));
        }
        let Some(model) = compilation.model else {
            return Err(format!("imported domain {path} does not compile"));
        };
        union.packages.extend(model.packages);
    }
    Ok(union)
}

/// Gathers the `.mox` domains one domain-surface file imports:
/// `import_paths` yields the file's raw import strings (empty when the
/// file does not parse — the compile reports that), each resolved
/// relative to `path_key`'s directory, deduplicated **by resolved key**,
/// and fetched. A missing import skips silently (the LSP's
/// degrade-gracefully discipline) — hosts wanting the CLI's hard error
/// capture it in their own closure side-channel, preserving their own
/// message.
///
/// This deliberately deviates from the CLI's original `read_domain_pair`,
/// which deduplicated by the *raw* import string — identical only when
/// paths carry no `..` segments; here two spellings of one file
/// (`"a.mox"` and `"./a.mox"`) gather once.
pub fn gather_domain_sources(
    path_key: &str,
    source: &str,
    import_paths: impl Fn(&str) -> Vec<String>,
    fetch: &mut dyn FnMut(&str) -> Option<String>,
) -> Vec<(String, String)> {
    // Lenient mode never errors — every miss is a silent skip.
    gather_domains_relative(path_key, source, import_paths, fetch, false).unwrap_or_default()
}

/// The buckets a `.uc` (or `.cmmn`) use-case file's imports resolve to:
/// the raw domains plus the pre-compiled `.ddd`/`.actor`/`.evt` artifacts
/// [`crate::compile_usecase_str`] consumes — the CLI's
/// `GatheredImports::gather` + `read_*_model` discipline, keyed by
/// resolved path. A missing import anywhere (nested gathering included)
/// errors ``cannot read imported file {resolved} (imported by {importer})``;
/// a non-compiling artifact names its surface. Unknown extensions pass
/// through so the driver reports them.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UcBuckets {
    /// Each imported `.mox` domain as `(resolved key, source)`.
    pub domains: Vec<(String, String)>,
    /// Each imported `.ddd` design, compiled.
    pub designs: Vec<(String, rex_ir::ddd::DddModel)>,
    /// Each imported `.actor` policy, compiled.
    pub actors: Vec<(String, rex_ir::ActorModel)>,
    /// Each imported `.evt` contract, compiled.
    pub events: Vec<(String, rex_ir::events::EventModel)>,
}

/// Gathers a `.uc` file's imports into [`UcBuckets`]. Import strings
/// resolve relative to `path_key`'s directory; duplicates (same resolved
/// key, same bucket) gather once. Each artifact import's own `.mox`
/// imports gather the same way, relative to the artifact's resolved key —
/// the CLI rule (`rexlang check` on the artifact itself).
pub fn gather_usecase_imports(
    path_key: &str,
    source: &str,
    import_paths: impl Fn(&str) -> Vec<String>,
    fetch: &mut dyn FnMut(&str) -> Option<String>,
) -> Result<UcBuckets, String> {
    let mut buckets = UcBuckets::default();
    let dir = dir_of(path_key);
    for import in import_paths(source) {
        let resolved = dir.join(&import);
        let key = resolved.display().to_string();
        match resolved
            .extension()
            .and_then(|extension| extension.to_str())
        {
            Some("mox") => {
                if buckets.domains.iter().any(|(existing, _)| *existing == key) {
                    continue;
                }
                let text = fetch(&key).ok_or_else(|| cannot_read(&key, path_key))?;
                buckets.domains.push((key, text));
            }
            Some("ddd") => {
                if buckets.designs.iter().any(|(existing, _)| *existing == key) {
                    continue;
                }
                let text = fetch(&key).ok_or_else(|| cannot_read(&key, path_key))?;
                let model = gather_design_model(&key, &text, fetch)?;
                buckets.designs.push((key, model));
            }
            Some("actor") => {
                if buckets.actors.iter().any(|(existing, _)| *existing == key) {
                    continue;
                }
                let text = fetch(&key).ok_or_else(|| cannot_read(&key, path_key))?;
                let model = gather_actor_model(&key, &text, fetch)?;
                buckets.actors.push((key, model));
            }
            Some("evt") => {
                if buckets.events.iter().any(|(existing, _)| *existing == key) {
                    continue;
                }
                let text = fetch(&key).ok_or_else(|| cannot_read(&key, path_key))?;
                let model = gather_event_model(&key, &text, fetch)?;
                buckets.events.push((key, model));
            }
            _ => {}
        }
    }
    Ok(buckets)
}

/// The buckets a `.cmmn` case file's imports resolve to: the `.uc` buckets
/// plus the pre-compiled `.dmn` decision models a `decision` task's
/// `invokes` clause resolves against — keyed by resolved path, the same
/// discovery (extension bucketing) and compilation discipline as the
/// `.evt` bucket.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CmmnBuckets {
    /// Each imported `.mox` domain as `(resolved key, source)`.
    pub domains: Vec<(String, String)>,
    /// Each imported `.ddd` design, compiled.
    pub designs: Vec<(String, rex_ir::ddd::DddModel)>,
    /// Each imported `.actor` policy, compiled.
    pub actors: Vec<(String, rex_ir::ActorModel)>,
    /// Each imported `.evt` contract, compiled.
    pub events: Vec<(String, rex_ir::events::EventModel)>,
    /// Each imported `.dmn` decision model, compiled.
    pub decisions: Vec<(String, rex_ir::dmn::DmnModel)>,
}

/// Gathers a `.cmmn` file's imports into [`CmmnBuckets`] — the four
/// use-case buckets through [`gather_usecase_imports`] (identical
/// discovery, dedup, and error text), plus a decision pass over the same
/// imports compiling each `.dmn` into the bucket (its own `.mox` imports
/// and decision tables resolve relative to its resolved key). Unknown
/// extensions pass through so the driver reports them.
pub fn gather_case_imports(
    path_key: &str,
    source: &str,
    import_paths: &dyn Fn(&str) -> Vec<String>,
    fetch: &mut dyn FnMut(&str) -> Option<String>,
) -> Result<CmmnBuckets, String> {
    let uc = gather_usecase_imports(path_key, source, import_paths, fetch)?;
    let mut buckets = CmmnBuckets {
        domains: uc.domains,
        designs: uc.designs,
        actors: uc.actors,
        events: uc.events,
        decisions: Vec::new(),
    };
    let dir = dir_of(path_key);
    for import in import_paths(source) {
        let resolved = dir.join(&import);
        let key = resolved.display().to_string();
        if resolved
            .extension()
            .and_then(|extension| extension.to_str())
            != Some("dmn")
        {
            continue;
        }
        if buckets
            .decisions
            .iter()
            .any(|(existing, _)| *existing == key)
        {
            continue;
        }
        let text = fetch(&key).ok_or_else(|| cannot_read(&key, path_key))?;
        let model = gather_decision_model(&key, &text, fetch)?;
        buckets.decisions.push((key, model));
    }
    Ok(buckets)
}

/// Gathers and compiles one imported `.dmn` decision model: its own `.mox`
/// imports resolve relative to its resolved key, its `.evt` imports
/// gather through [`gather_dmn_events`], and each `logic table "<path>"`
/// clause's markdown content is fetched the same way and keyed by the
/// path **as written** (the driver matches table paths exactly as
/// written). Compilation failure reads ``imported decision model {key}
/// does not compile`` plus the error-diagnostic lines.
fn gather_decision_model(
    key: &str,
    text: &str,
    fetch: &mut dyn FnMut(&str) -> Option<String>,
) -> Result<rex_ir::dmn::DmnModel, String> {
    let domains = gather_domains_relative(
        key,
        text,
        |source| {
            rex_syntax::parse_dmn(source)
                .ast
                .map(|ast| ast.imports.into_iter().map(|import| import.path).collect())
                .unwrap_or_default()
        },
        fetch,
        true,
    )?;
    let schemas = collect_schema_imports(&domains).map_err(|error| error.to_string())?;
    let sigil = collect_sigil_imports(&domains).map_err(|error| error.to_string())?;
    let dir = dir_of(key);
    let mut tables: Vec<(String, String)> = Vec::new();
    if let Some(ast) = rex_syntax::parse_dmn(text).ast.and_then(|ast| ast.model) {
        for decision in &ast.decisions {
            let source = match &decision.logic {
                Some(rex_syntax::DmnLogic::Table { source, .. }) => source,
                _ => continue,
            };
            if tables.iter().any(|(name, _)| name == source) {
                continue;
            }
            let resolved = dir.join(source).display().to_string();
            if let Some(content) = fetch(&resolved) {
                tables.push((source.clone(), content));
            }
        }
    }
    let events = gather_dmn_events(key, text, fetch)?;
    let compilation = compile_dmn_str(
        key,
        text,
        &domains,
        &DomainImports {
            schemas,
            sigil: sigil.imports,
        },
        &tables,
        &events,
    );
    let Some(model) = compilation.model else {
        return Err(does_not_compile(
            "imported decision model",
            key,
            &compilation.diagnostics,
        ));
    };
    Ok(model)
}
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DeployPairs {
    /// Each imported `.ddd` design, compiled.
    pub designs: Vec<(String, rex_ir::ddd::DddModel)>,
    /// Each imported `.ifml` flow, compiled.
    pub flows: Vec<(String, rex_ir::ifml::IfmlModel)>,
}

/// Gathers a `.deploy` file's imports into [`DeployPairs`] — the CLI's
/// `read_deploy_pair`/`read_design_model`/`read_flow_model` discipline.
/// Designs gather exactly like the `.uc` bucket; flows fetch their text
/// and walk their own `.ifml` closure through a bridged two-key fetch.
/// The walker recurses under **as-written import strings** (only the
/// entry identity is a resolved disk-path key), so the bridge keeps an
/// identity → resolved-key map per flow: each import resolves relative to
/// the resolved key its importer was actually fetched under, never the
/// process CWD. The flow's `.actor`/`.mox`/`.dmn` imports ride through
/// the same bridge (a `.dmn` model's decision tables with it) but the
/// deploy lowering consumes none, so they are dropped.
pub fn gather_deploy_imports(
    path_key: &str,
    source: &str,
    import_paths: impl Fn(&str) -> Vec<String>,
    fetch: &mut dyn FnMut(&str) -> Option<String>,
) -> Result<DeployPairs, String> {
    let mut pairs = DeployPairs::default();
    let dir = dir_of(path_key);
    for import in import_paths(source) {
        let resolved = dir.join(&import);
        let key = resolved.display().to_string();
        match resolved
            .extension()
            .and_then(|extension| extension.to_str())
        {
            Some("ddd") => {
                if pairs.designs.iter().any(|(existing, _)| *existing == key) {
                    continue;
                }
                let text = fetch(&key).ok_or_else(|| cannot_read(&key, path_key))?;
                let model = gather_design_model(&key, &text, fetch)?;
                pairs.designs.push((key, model));
            }
            Some("ifml") => {
                if pairs.flows.iter().any(|(existing, _)| *existing == key) {
                    continue;
                }
                let text = fetch(&key).ok_or_else(|| cannot_read(&key, path_key))?;
                // The walker recurses under as-written import strings, so
                // map every identity to the resolved key its content was
                // fetched under — seeded with the entry identity (the
                // flow's own resolved key) and extended on every fetch.
                let mut keys: BTreeMap<String, String> = BTreeMap::new();
                keys.insert(key.clone(), key.clone());
                let mut bridge = |importer: &str, import: &str| -> Option<(String, String)> {
                    let base = keys
                        .get(importer)
                        .cloned()
                        .unwrap_or_else(|| importer.to_string());
                    let resolved = dir_of(&base).join(import);
                    let resolved_key = resolved.display().to_string();
                    let text = fetch(&resolved_key)?;
                    keys.insert(import.to_string(), resolved_key.clone());
                    Some((resolved_key, text))
                };
                let project = gather_ifml_project(&key, &text, &mut bridge);
                match rex_ifml::compile_ifml_str(&key, &text, &project.imports) {
                    Ok(compilation) => pairs.flows.push((key, compilation.model)),
                    Err(_) => return Err(format!("imported flow {key} does not compile")),
                }
            }
            _ => {}
        }
    }
    Ok(pairs)
}

/// Gathers and compiles one imported `.ddd` design: its own `.mox`
/// imports resolve relative to its resolved key (exactly like `rexlang
/// check` on the design), hard-erroring on the first miss. Compilation
/// failure reads ``imported design {key} does not compile`` plus the
/// error-diagnostic lines.
fn gather_design_model(
    key: &str,
    text: &str,
    fetch: &mut dyn FnMut(&str) -> Option<String>,
) -> Result<rex_ir::ddd::DddModel, String> {
    let domains = gather_domains_relative(
        key,
        text,
        |source| {
            rex_syntax::parse_ddd(source)
                .ast
                .map(|ast| ast.imports.into_iter().map(|import| import.path).collect())
                .unwrap_or_default()
        },
        fetch,
        true,
    )?;
    let schemas = collect_schema_imports(&domains).map_err(|error| error.to_string())?;
    let sigil = collect_sigil_imports(&domains).map_err(|error| error.to_string())?;
    let compilation = compile_ddd_str(
        key,
        text,
        &domains,
        &DomainImports {
            schemas,
            sigil: sigil.imports,
        },
    );
    let Some(model) = compilation.model else {
        return Err(does_not_compile(
            "imported design",
            key,
            &compilation.diagnostics,
        ));
    };
    Ok(model)
}

/// Gathers and compiles one imported `.actor` policy: the
/// [`compile_actor_imports`] shape for a single policy, with its own
/// `.mox` imports resolved relative to its resolved key. Compilation
/// failure reads ``imported actor model {key} does not compile`` plus the
/// error-diagnostic lines.
fn gather_actor_model(
    key: &str,
    text: &str,
    fetch: &mut dyn FnMut(&str) -> Option<String>,
) -> Result<rex_ir::ActorModel, String> {
    let actor = ActorImport {
        key: key.to_string(),
        source: text.to_string(),
        domains: gather_domains_relative(
            key,
            text,
            |source| {
                rex_syntax::parse_actors(source)
                    .ast
                    .map(|ast| ast.imports.into_iter().map(|import| import.path).collect())
                    .unwrap_or_default()
            },
            fetch,
            true,
        )?,
    };
    compile_actor_imports(std::slice::from_ref(&actor))
        .map(|mut models| models.pop().expect("one policy, one model"))
}

/// The raw import strings one `.dmn` source declares (empty when the file
/// does not parse — the compile reports that). Hosts split the list by
/// extension: `.mox` paths join the domain gather, `.evt` paths
/// [`gather_dmn_events`].
pub fn dmn_import_paths(source: &str) -> Vec<String> {
    rex_syntax::parse_dmn(source)
        .ast
        .map(|ast| ast.imports.into_iter().map(|import| import.path).collect())
        .unwrap_or_default()
}

/// Gathers the `.evt` contracts one `.dmn` decision-model file imports:
/// each `import "<x>.evt"` resolves relative to `path_key`'s directory
/// (exactly how `.mox` imports resolve), is fetched, and is compiled
/// through the ordinary event pipeline — the contract's own `.mox` imports
/// resolve relative to its resolved key, exactly like `rexlang check` on
/// the contract itself. The bucket is keyed by the import string **as
/// written** (the table-path rule): the driver's import check matches
/// `events` entries exactly as written. A missing or non-compiling
/// contract is a clean error naming it; duplicates gather once.
pub fn gather_dmn_events(
    path_key: &str,
    source: &str,
    fetch: &mut dyn FnMut(&str) -> Option<String>,
) -> Result<Vec<(String, rex_ir::events::EventModel)>, String> {
    let dir = dir_of(path_key);
    let mut events: Vec<(String, rex_ir::events::EventModel)> = Vec::new();
    for import in dmn_import_paths(source) {
        if !import.ends_with(".evt") || events.iter().any(|(key, _)| *key == import) {
            continue;
        }
        let resolved = dir.join(&import);
        let key = resolved.display().to_string();
        let text = fetch(&key).ok_or_else(|| cannot_read(&key, path_key))?;
        let model = gather_event_model(&key, &text, fetch)?;
        events.push((import, model));
    }
    Ok(events)
}

/// Gathers and compiles one imported `.evt` contract: its own `.mox`
/// imports resolve relative to its resolved key. Compilation failure
/// reads ``imported contract {key} does not compile`` plus the
/// error-diagnostic lines.
fn gather_event_model(
    key: &str,
    text: &str,
    fetch: &mut dyn FnMut(&str) -> Option<String>,
) -> Result<rex_ir::events::EventModel, String> {
    let domains = gather_domains_relative(
        key,
        text,
        |source| {
            rex_syntax::parse_evt(source)
                .ast
                .map(|ast| ast.imports.into_iter().map(|import| import.path).collect())
                .unwrap_or_default()
        },
        fetch,
        true,
    )?;
    let schemas = collect_schema_imports(&domains).map_err(|error| error.to_string())?;
    let sigil = collect_sigil_imports(&domains).map_err(|error| error.to_string())?;
    let compilation = compile_evt_str(
        key,
        text,
        &domains,
        &DomainImports {
            schemas,
            sigil: sigil.imports,
        },
    );
    let Some(model) = compilation.model else {
        return Err(does_not_compile(
            "imported contract",
            key,
            &compilation.diagnostics,
        ));
    };
    Ok(model)
}

/// The pinned failure message: `{kind} {key} does not compile`, plus one
/// `  {path}: {message}` line per error diagnostic — the exact shape the
/// CLI's `read_design_model`/`read_actor_model`/`read_event_model` emit.
fn does_not_compile(kind: &str, key: &str, diagnostics: &[(String, Diagnostic)]) -> String {
    let messages: Vec<String> = diagnostics
        .iter()
        .filter(|(_, diagnostic)| diagnostic.is_error())
        .map(|(path, diagnostic)| format!("{path}: {}", diagnostic.message))
        .collect();
    format!(
        "{kind} {key} does not compile{}",
        if messages.is_empty() {
            String::new()
        } else {
            format!(":\n  {}", messages.join("\n  "))
        }
    )
}

/// The pinned missing-import message: no io error to name here, so the
/// parenthetical importer is the whole story.
fn cannot_read(key: &str, importer: &str) -> String {
    format!("cannot read imported file {key} (imported by {importer})")
}

/// `true` for the extensions [`IfmlProject::unresolved`] tracks; other
/// extensions pass through ignored (parity with today's walker hosts).
fn is_imported_surface(import: &str) -> bool {
    import.ends_with(".ifml")
        || import.ends_with(".mox")
        || import.ends_with(".actor")
        || import.ends_with(".dmn")
}

/// The domain gather core behind [`gather_domain_sources`]: resolve each
/// raw import string relative to `path_key`'s directory, dedup by
/// resolved key, fetch. `strict` selects the discipline — `false` skips
/// missing imports silently (the LSP rule, never errors), `true` errors
/// on the first miss with the pinned message (the CLI rule behind
/// `.ddd`/`.actor`/`.evt` nested gathering).
fn gather_domains_relative(
    path_key: &str,
    source: &str,
    import_paths: impl Fn(&str) -> Vec<String>,
    fetch: &mut dyn FnMut(&str) -> Option<String>,
    strict: bool,
) -> Result<Vec<(String, String)>, String> {
    let mut domains: Vec<(String, String)> = Vec::new();
    let dir = dir_of(path_key);
    for import in import_paths(source) {
        let resolved = dir.join(&import);
        let key = resolved.display().to_string();
        if domains.iter().any(|(existing, _)| *existing == key) {
            continue;
        }
        match fetch(&key) {
            Some(text) => domains.push((key, text)),
            None if strict => return Err(cannot_read(&key, path_key)),
            None => {}
        }
    }
    Ok(domains)
}
