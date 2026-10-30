//! Documentation honesty: every fenced code block in the markdown docs
//! that is tagged with a surface language (` ```mox `, ` ```actor `,
//! ` ```ddd `, ` ```evt `, ` ```deploy `, ` ```uc `, ` ```cmmn `,
//! ` ```ifml `) is a **contract** — it is extracted and compiled with that
//! surface's real entry point, imports resolved from disk exactly the way
//! the CLI resolves them (relative to the declaring document, so snippets
//! import real repo files). An untagged ` ```text ` fence is prose (EBNF,
//! diagrams) and is never compiled.
//!
//! A doc snippet that stops compiling fails CI with the document's path,
//! the fence index, and the diagnostics.
//!
//! Each surface also carries a **minimum fence count**, so stripping a tag
//! (or losing a document's only example) fails as loudly as a broken
//! snippet.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// The documents the harness sweeps (repo-root relative).
const DOCUMENTS: [&str; 10] = [
    "README.md",
    "docs/README.md",
    "docs/LANGUAGE.md",
    "docs/DDD.md",
    "docs/EVENTS.md",
    "docs/DEPLOY.md",
    "docs/USECASES.md",
    "docs/CMMN.md",
    "docs/IFML.md",
    "docs/BACKENDS.md",
];

/// The minimum number of compiling fences per surface across all
/// documents. A surface with zero tagged snippets cannot drift.
const MINIMUMS: [(&str, usize); 8] = [
    ("mox", 3),
    ("actor", 1),
    ("ddd", 1),
    ("evt", 1),
    ("deploy", 1),
    ("uc", 1),
    ("cmmn", 1),
    ("ifml", 5),
];

/// One extracted fence: its surface tag, the document it lives in, and the
/// snippet text.
struct Fence {
    document: &'static str,
    index: usize,
    tag: String,
    source: String,
}

/// Extracts every surface-tagged fence from a document. Only fences whose
/// info string is exactly a surface tag count (indented fences, other
/// languages, and bare ` ``` ` blocks are prose).
fn fences_of(document: &'static str, source: &str) -> Vec<Fence> {
    const TAGS: [&str; 8] = ["mox", "actor", "ddd", "evt", "deploy", "uc", "cmmn", "ifml"];
    let mut fences = Vec::new();
    let mut lines = source.lines().peekable();
    let mut index = 0usize;
    while let Some(line) = lines.next() {
        let trimmed = line.trim_start();
        if let Some(tag) = trimmed.strip_prefix("```") {
            if !TAGS.contains(&tag) {
                continue;
            }
            let tag = tag.to_string();
            index += 1;
            let mut body = Vec::new();
            let mut closed = false;
            for inner in lines.by_ref() {
                if inner.trim_start() == "```" {
                    closed = true;
                    break;
                }
                body.push(inner);
            }
            assert!(closed, "{document}: fence {index} ({tag}) is never closed");
            fences.push(Fence {
                document,
                index,
                tag,
                source: format!("{}\n", body.join("\n")),
            });
        }
    }
    fences
}

/// A resolved import: the bucket it landed in plus its content.
#[derive(Default)]
struct Gathered {
    /// Raw `.mox` domain texts, keyed by resolved path.
    domains: Vec<(String, String)>,
    /// `.ddd` design sources, keyed by resolved path.
    designs: Vec<(String, String)>,
    /// `.actor` policy sources, keyed by resolved path.
    actors: Vec<(String, String)>,
    /// `.evt` contract sources, keyed by resolved path.
    events: Vec<(String, String)>,
    /// `.ifml` sources, keyed by resolved path.
    flows: Vec<(String, String)>,
    /// `import sigil` rosetta sources, keyed by the path as written.
    sigil: BTreeMap<String, String>,
    /// `import schema` JSON documents, keyed by the path as written.
    schemas: BTreeMap<String, String>,
}

/// Resolves one import path relative to the declaring document's directory
/// (the CLI rule), reading it from disk.
fn resolve(base: &Path, import: &str) -> anyhow::Result<Option<PathBuf>> {
    let resolved = base.join(import);
    if !resolved.is_file() {
        return Ok(None);
    }
    Ok(Some(resolved))
}

/// Gathers the full import closure of one fence: raw domains plus the
/// compiled artifact surfaces' sources (each artifact import recursively
/// gathers its own imports, so a design's domain arrives with it).
fn gather(base: &Path, source: &str, surface: &str) -> anyhow::Result<Gathered> {
    let mut gathered = Gathered::default();
    // The queue carries (directory whose imports these are, source, the
    // originating extension). `.ifml` sources may name non-`.ifml`
    // imports (domains, actor policies) that are gathered as side effects
    // when present and tolerated when absent — the resolver only needs
    // the `.ifml` ones.
    let mut queue: Vec<(PathBuf, String, String)> = Vec::new();
    queue.push((base.to_path_buf(), source.to_string(), surface.to_string()));
    let mut seen: Vec<String> = Vec::new();

    let sigil_re = |line: &str| line.trim().starts_with("import sigil");
    let schema_re = |line: &str| line.trim().starts_with("import schema");

    while let Some((origin, text, origin_extension)) = queue.pop() {
        for line in text.lines() {
            let base = origin.as_path();
            let trimmed = line.trim();
            let import = if sigil_re(trimmed) || schema_re(trimmed) {
                // `import schema "<path>" [as Alias]` / `import sigil "<path>"`
                let rest = trimmed
                    .strip_prefix("import schema")
                    .or_else(|| trimmed.strip_prefix("import sigil"))
                    .unwrap()
                    .trim();
                let quoted = rest
                    .split('"')
                    .nth(1)
                    .map(|p| p.to_string())
                    .ok_or_else(|| anyhow::anyhow!("malformed import line: {trimmed:?}"))?;
                let Some(resolved) = resolve(base, &quoted)? else {
                    return Err(anyhow::anyhow!(
                        "imported file {} does not exist (relative to {})",
                        base.join(&quoted).display(),
                        base.display()
                    ));
                };
                if sigil_re(trimmed) {
                    if !gathered.sigil.contains_key(&quoted) {
                        gathered
                            .sigil
                            .insert(quoted.clone(), std::fs::read_to_string(resolved)?);
                    }
                } else if !gathered.schemas.contains_key(&quoted) {
                    gathered
                        .schemas
                        .insert(quoted.clone(), std::fs::read_to_string(resolved)?);
                }
                continue;
            } else if trimmed.starts_with("import ") {
                trimmed
                    .strip_prefix("import")
                    .unwrap()
                    .trim()
                    .split('"')
                    .nth(1)
                    .map(|p| p.to_string())
                    .ok_or_else(|| anyhow::anyhow!("malformed import line: {trimmed:?}"))?
            } else {
                continue;
            };
            let resolved = match resolve(base, &import)? {
                Some(resolved) => resolved,
                None if origin_extension == "ifml" => continue,
                None => {
                    return Err(anyhow::anyhow!(
                        "imported file {} does not exist (relative to {})",
                        base.join(&import).display(),
                        base.display()
                    ))
                }
            };
            let key = resolved.display().to_string();
            if seen.contains(&key) {
                continue;
            }
            seen.push(key.clone());
            let extension = resolved
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or_default()
                .to_string();
            let content = std::fs::read_to_string(&resolved)?;
            match extension.as_str() {
                "mox" => gathered.domains.push((key.clone(), content.clone())),
                "ddd" => gathered.designs.push((key.clone(), content.clone())),
                "actor" => gathered.actors.push((key.clone(), content.clone())),
                "evt" => gathered.events.push((key.clone(), content.clone())),
                "ifml" => gathered.flows.push((key.clone(), content.clone())),
                other => anyhow::bail!("unsupported import extension .{other}: {key}"),
            }
            let _ = &content;
            // Recurse: the imported surface's own imports come along,
            // resolved relative to that file's directory (the CLI rule).
            queue.push((resolved.parent().unwrap().to_path_buf(), content, extension));
        }
        let _ = origin;
    }
    Ok(gathered)
}

/// Compiles one `.ifml` source with its `.ifml` imports gathered from disk
/// through the shared traversal (the `examples_artifacts` pattern).
fn compile_flow(path: &str, source: &str) -> (String, rex_ir::ifml::IfmlModel) {
    let disk = Path::new(path).to_path_buf();
    let mut imports = rex_ifml::IfmlImports::default();
    let mut disk_paths: BTreeMap<String, PathBuf> = BTreeMap::new();
    disk_paths.insert(path.to_string(), disk);
    rex_ifml::walk_ifml_imports(path, source, &mut |importer, import| {
        let importer_disk = disk_paths.get(importer)?;
        let resolved = importer_disk.parent().unwrap().join(import);
        if import.ends_with(".ifml") && resolved.is_file() {
            let imported = std::fs::read_to_string(&resolved).ok()?;
            disk_paths.insert(import.to_string(), resolved);
            imports.provide(importer, import, imported.clone());
            Some(imported)
        } else {
            None
        }
    });
    let compiled = rex_ifml::compile_ifml_str(path, source, &imports).expect("flow compiles");
    (path.to_string(), compiled.model)
}

/// Compiles one fence against its gathered imports. The returned string is
/// a short failure description (empty on success).
fn compile_fence(fence: &Fence, workspace: &Path) -> String {
    let base = workspace
        .join(fence.document)
        .parent()
        .unwrap()
        .to_path_buf();
    let virtual_path = workspace
        .join(fence.document)
        .with_extension(format!("fence{}", fence.index))
        .display()
        .to_string();
    let gathered = match gather(&base, &fence.source, &fence.tag) {
        Ok(gathered) => gathered,
        Err(error) => return error.to_string(),
    };
    let empty = rex_driver::DomainImports::default();
    // `import schema`/`import sigil` content arrives through the bundle.
    let imports = {
        // `import schema`/`import sigil` keys are `(mox path, import path)`;
        // doc fences have one mox path (the virtual one), so the same
        // content is provided under it for every import path.
        let mut schemas = rex_driver::SchemaImports::default();
        for (path, content) in &gathered.schemas {
            schemas = schemas
                .clone()
                .provide(&virtual_path, path.clone(), content.clone());
        }
        let mut sigil = rex_driver::SigilImports::default();
        for (path, content) in &gathered.sigil {
            sigil = sigil
                .clone()
                .provide(&virtual_path, path.clone(), content.clone());
        }
        rex_driver::DomainImports { schemas, sigil }
    };
    let label = fence_label(fence);
    let problems = |messages: Vec<String>| -> String {
        messages
            .into_iter()
            .map(|message| format!("{label}: {message}"))
            .collect::<Vec<_>>()
            .join("; ")
    };
    let errors_of = |diagnostics: &[rex_driver::Diagnostic]| -> Vec<String> {
        diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.is_error())
            .map(|diagnostic| diagnostic.message.clone())
            .collect()
    };
    let tagged_errors_of = |diagnostics: &[(String, rex_driver::Diagnostic)]| -> Vec<String> {
        diagnostics
            .iter()
            .filter(|(_, diagnostic)| diagnostic.is_error())
            .map(|(path, diagnostic)| format!("{}: {}", path, diagnostic.message))
            .collect()
    };
    match fence.tag.as_str() {
        "mox" => {
            let compilation = rex_driver::compile_str(&virtual_path, &fence.source, &imports);
            problems(errors_of(&compilation.diagnostics))
        }
        "actor" => {
            let compilation = rex_driver::compile_actors_str(
                &virtual_path,
                &fence.source,
                &gathered.domains,
                &imports,
            );
            problems(tagged_errors_of(&compilation.diagnostics))
        }
        "ddd" => {
            let compilation = rex_driver::compile_ddd_str(
                &virtual_path,
                &fence.source,
                &gathered.domains,
                &imports,
            );
            problems(tagged_errors_of(&compilation.diagnostics))
        }
        "evt" => {
            let compilation = rex_driver::compile_evt_str(
                &virtual_path,
                &fence.source,
                &gathered.domains,
                &imports,
            );
            problems(tagged_errors_of(&compilation.diagnostics))
        }
        "deploy" => {
            let designs: Vec<(String, rex_ir::ddd::DddModel)> = gathered
                .designs
                .iter()
                .map(|(path, source)| {
                    let dir = Path::new(path).parent().unwrap().to_path_buf();
                    let inner = gather(&dir, source, "ddd").expect("design imports");
                    let inner_domains = inner.domains.clone();
                    let compiled =
                        rex_driver::compile_ddd_str(path, source, &inner_domains, &empty);
                    (path.clone(), compiled.model.expect("design artifact"))
                })
                .collect();
            let flows: Vec<(String, rex_ir::ifml::IfmlModel)> = gathered
                .flows
                .iter()
                .map(|(path, source)| compile_flow(path, source))
                .collect();
            let compilation =
                rex_driver::compile_deploy_str(&virtual_path, &fence.source, &designs, &flows);
            problems(tagged_errors_of(&compilation.diagnostics))
        }
        "uc" => {
            let designs: Vec<(String, rex_ir::ddd::DddModel)> = gathered
                .designs
                .iter()
                .map(|(path, source)| {
                    let compiled =
                        rex_driver::compile_ddd_str(path, source, &gathered.domains, &empty);
                    (path.clone(), compiled.model.expect("design artifact"))
                })
                .collect();
            let actors: Vec<(String, rex_ir::ActorModel)> = gathered
                .actors
                .iter()
                .map(|(path, source)| {
                    let compiled =
                        rex_driver::compile_actors_str(path, source, &gathered.domains, &empty);
                    (path.clone(), compiled.model.expect("actor artifact"))
                })
                .collect();
            let events: Vec<(String, rex_ir::events::EventModel)> = gathered
                .events
                .iter()
                .map(|(path, source)| {
                    let compiled =
                        rex_driver::compile_evt_str(path, source, &gathered.domains, &empty);
                    (path.clone(), compiled.model.expect("event artifact"))
                })
                .collect();
            let compilation = rex_driver::compile_usecase_str(
                &virtual_path,
                &fence.source,
                &gathered.domains,
                &imports,
                &designs,
                &actors,
                &events,
            );
            problems(tagged_errors_of(&compilation.diagnostics))
        }
        "cmmn" => {
            let designs: Vec<(String, rex_ir::ddd::DddModel)> = gathered
                .designs
                .iter()
                .map(|(path, source)| {
                    let compiled =
                        rex_driver::compile_ddd_str(path, source, &gathered.domains, &empty);
                    (path.clone(), compiled.model.expect("design artifact"))
                })
                .collect();
            let actors: Vec<(String, rex_ir::ActorModel)> = gathered
                .actors
                .iter()
                .map(|(path, source)| {
                    let compiled =
                        rex_driver::compile_actors_str(path, source, &gathered.domains, &empty);
                    (path.clone(), compiled.model.expect("actor artifact"))
                })
                .collect();
            let events: Vec<(String, rex_ir::events::EventModel)> = gathered
                .events
                .iter()
                .map(|(path, source)| {
                    let compiled =
                        rex_driver::compile_evt_str(path, source, &gathered.domains, &empty);
                    (path.clone(), compiled.model.expect("event artifact"))
                })
                .collect();
            let compilation = rex_driver::compile_cmmn_str(
                &virtual_path,
                &fence.source,
                &gathered.domains,
                &imports,
                &designs,
                &actors,
                &events,
            );
            problems(tagged_errors_of(&compilation.diagnostics))
        }
        "ifml" => {
            // Flow imports resolve like the CLI's walker: `.ifml` imports
            // relative to the declaring file; domain imports are gathered
            // as a side effect but the compiler only needs the `.ifml`
            // ones. Untyped checking (`domains: None`) is the documented
            // generation-time contract for doc snippets.
            // The fence compiles as the document itself; `.ifml` imports
            // resolve relative to the document's directory. What the shared
            // expression language cannot type (domains: None) is the
            // documented generation-time contract, not an error.
            let real = workspace.join(fence.document);
            let real_string = real.display().to_string();
            let mut imports = rex_ifml::IfmlImports::default();
            let mut disk_paths: BTreeMap<String, PathBuf> = BTreeMap::new();
            disk_paths.insert(real_string.clone(), real.clone());
            rex_ifml::walk_ifml_imports(&real_string, &fence.source, &mut |importer, import| {
                let importer_disk = disk_paths.get(importer)?;
                let resolved = importer_disk.parent().unwrap().join(import);
                if import.ends_with(".ifml") && resolved.is_file() {
                    let imported = std::fs::read_to_string(&resolved).ok()?;
                    disk_paths.insert(import.to_string(), resolved);
                    imports.provide(importer, import, imported.clone());
                    Some(imported)
                } else {
                    None
                }
            });
            match rex_ifml::compile_ifml_str(&real_string, &fence.source, &imports) {
                Ok(_) => String::new(),
                Err(errors) => {
                    let label = fence_label(fence);
                    errors
                        .iter()
                        .map(|error| format!("{label}: {}: {}", error.code, error.message))
                        .collect::<Vec<_>>()
                        .join("; ")
                }
            }
        }
        other => format!("unsupported surface tag `{other}`"),
    }
}

/// The human label a failure names: document + fence index + first line.
fn fence_label(fence: &Fence) -> String {
    let first = fence
        .source
        .lines()
        .find(|line| !line.trim().is_empty())
        .unwrap_or("")
        .trim();
    format!(
        "{} fence {} ({}): {}",
        fence.document, fence.index, fence.tag, first
    )
}

#[test]
fn every_tagged_doc_fence_compiles() {
    let workspace = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("workspace root");
    let mut failures: Vec<String> = Vec::new();
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for document in DOCUMENTS {
        let path = workspace.join(document);
        let source = std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("cannot read {document}: {error}"));
        for fence in fences_of(document, &source) {
            *counts.entry(fence.tag.clone()).or_default() += 1;
            let failure = compile_fence(&fence, &workspace);
            if !failure.is_empty() {
                failures.push(failure);
            }
        }
    }
    for (tag, minimum) in MINIMUMS {
        let count = counts.get(tag).copied().unwrap_or(0);
        assert!(
            count >= minimum,
            "only {count} `{tag}` fences across the docs (minimum {minimum}); \
             a surface whose snippets are untagged cannot be checked"
        );
    }
    assert!(
        failures.is_empty(),
        "documentation code fences failed to compile:\n  {}",
        failures.join("\n  ")
    );
}
