//! Conformance tests for the `examples/` interaction flows: every
//! `examples/<name>.ifml` listed in [`EXAMPLES`] must exist, compile with
//! zero diagnostics when its imports resolve from disk exactly the way the
//! CLI resolves them, type-check against the union of its imported `.mox`
//! domains, and be byte-identical to its own canonical formatting.

use std::fs;
use std::path::Path;

use rex_driver::{compile_str, DomainImports};
use rex_ifml::{check_ifml, compile_ifml_str, IfmlCompilation, IfmlDiagnostic, IfmlImports};

/// The interaction-flow examples, hardcoded so a missing file fails loudly
/// (the house convention for the `examples/` harnesses).
pub const EXAMPLES: [&str; 2] = ["ecommerce", "support"];

const EXAMPLES_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples");

/// One example, ready for compilation: the main source, the `.ifml` import
/// bundle, and every imported `.mox` domain as `(resolved path, source)`.
struct Example {
    path: String,
    source: String,
    imports: IfmlImports,
    domains: Vec<(String, String)>,
}

/// Reads `examples/<name>.ifml` plus everything it imports, mirroring the
/// CLI's depth-first collection: `.ifml` imports resolve relative to the
/// importing file's directory and recurse, `.mox` imports join the domain
/// list (deduplicated by resolved path), other extensions are ignored. The
/// bundle is keyed the way the resolver looks entries up: the importing
/// file's key plus the import string exactly as written.
fn load(name: &str) -> Example {
    let source_path = Path::new(EXAMPLES_DIR).join(format!("{name}.ifml"));
    let source = fs::read_to_string(&source_path)
        .unwrap_or_else(|error| panic!("example {name}.ifml must exist under examples/: {error}"));
    let mut example = Example {
        path: source_path.display().to_string(),
        source,
        imports: IfmlImports::default(),
        domains: Vec::new(),
    };
    collect(
        &example.path,
        &source_path,
        &example.source,
        &mut example.imports,
        &mut example.domains,
        &mut Vec::new(),
    );
    example
}

/// Depth-first import collection for one source file: `key` is the bundle
/// key the resolver will look this file's own imports up under (the main
/// file's path, or the import string that pulled the file in), while
/// `source_path` is where the file lives on disk.
fn collect(
    key: &str,
    source_path: &Path,
    source: &str,
    bundle: &mut IfmlImports,
    domains: &mut Vec<(String, String)>,
    stack: &mut Vec<String>,
) {
    if stack.iter().any(|seen| seen == key) {
        return; // cycles are diagnosed by the resolver
    }
    stack.push(key.to_string());
    let dir = source_path.parent().unwrap_or_else(|| Path::new("."));
    let imports = rex_ifml::parse_ifml(source)
        .map(|model| model.imports)
        .unwrap_or_default();
    for import in imports {
        let resolved = dir.join(&import);
        let resolved_path = resolved.display().to_string();
        if import.ends_with(".ifml") {
            let imported = fs::read_to_string(&resolved).unwrap_or_else(|error| {
                panic!("read imported file {resolved_path} (imported by {key}): {error}")
            });
            bundle.provide(key, &import, imported.clone());
            collect(&import, &resolved, &imported, bundle, domains, stack);
        } else if import.ends_with(".mox")
            && !domains
                .iter()
                .any(|(existing, _)| *existing == resolved_path)
        {
            let domain = fs::read_to_string(&resolved).unwrap_or_else(|error| {
                panic!("read imported file {resolved_path} (imported by {key}): {error}")
            });
            domains.push((resolved_path, domain));
        }
    }
    stack.pop();
}

fn render(diagnostics: &[IfmlDiagnostic]) -> String {
    diagnostics
        .iter()
        .map(|diagnostic| {
            let (start, end) = diagnostic.span;
            format!(
                "  {} in {} at {start}..{end}: {}",
                diagnostic.code, diagnostic.file, diagnostic.message
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn compile(name: &str) -> IfmlCompilation {
    let example = load(name);
    match compile_ifml_str(&example.path, &example.source, &example.imports) {
        Ok(compilation) => compilation,
        Err(diagnostics) => panic!(
            "{name}.ifml must compile with zero diagnostics:\n{}",
            render(&diagnostics)
        ),
    }
}

/// Compiles every imported `.mox` domain to Core IR and unions the
/// packages — the model [`check_ifml`] binds expressions against.
fn domain_union(name: &str) -> rex_ir::Model {
    let example = load(name);
    assert!(
        !example.domains.is_empty(),
        "{name}.ifml must import at least one .mox domain"
    );
    let mut union = rex_ir::Model::new();
    for (path, source) in &example.domains {
        let compilation = compile_str(path, source, &DomainImports::default());
        assert!(
            compilation.diagnostics.is_empty() && compilation.sigil_diagnostics.is_empty(),
            "imported domain {path} must compile cleanly:\n{:+?}",
            compilation.diagnostics
        );
        union.packages.extend(
            compilation
                .model
                .expect("a clean compile lowers a model")
                .packages,
        );
    }
    union
}

#[test]
fn every_example_compiles_and_resolves_its_module_uses() {
    for name in &EXAMPLES {
        let compilation = compile(name);
        assert!(
            !compilation.model.views.is_empty(),
            "{name}.ifml must declare at least one view"
        );
        assert!(
            !compilation.index.module_uses.is_empty(),
            "{name}.ifml must compose pattern-library modules"
        );
        for use_site in compilation
            .index
            .module_uses
            .iter()
            .filter(|site| site.file == 0)
        {
            assert!(
                use_site.resolved_module.is_some(),
                "{name}.ifml: use '{}' must resolve to a module",
                use_site.target
            );
        }
    }
}

#[test]
fn every_example_type_checks_against_its_domain_union() {
    for name in &EXAMPLES {
        let compilation = compile(name);
        let union = domain_union(name);
        let binding = check_ifml(&compilation, Some(&union));
        assert!(
            binding.is_empty(),
            "{name}.ifml must type-check against its domains:\n{}",
            render(&binding)
        );
    }
}

#[test]
fn every_example_is_fmt_canonical() {
    for name in &EXAMPLES {
        let example = load(name);
        assert_eq!(
            rex_ifml::format_ifml(&example.source),
            example.source,
            "{name}.ifml must already be in canonical form"
        );
    }
}

#[test]
fn the_example_list_covers_every_ifml_file_on_disk() {
    let mut listed: Vec<String> = EXAMPLES.iter().map(|name| format!("{name}.ifml")).collect();
    listed.sort();
    let mut on_disk: Vec<String> = fs::read_dir(EXAMPLES_DIR)
        .expect("examples directory exists")
        .map(|entry| entry.expect("readable entry").path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "ifml")
        })
        .map(|path| {
            path.file_name()
                .expect("named file")
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    on_disk.sort();
    assert_eq!(
        listed, on_disk,
        "examples/*.ifml and EXAMPLES must stay in sync"
    );
}
