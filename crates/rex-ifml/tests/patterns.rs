//! Conformance tests for the `patterns/ifml` reference pattern library:
//! every library file compiles with zero diagnostics when its
//! same-directory `.ifml` imports are resolved from disk, a synthetic
//! consumer composes an imported module, and an unknown override names the
//! `E0007` diagnostic.

use std::fs;
use std::path::Path;

use rex_ifml::{compile_ifml_str, IfmlCompilation, IfmlImports, E_UNKNOWN_INPUT};

const PATTERNS_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../patterns/ifml");

/// The `import "..."` targets of one source, in source order.
fn import_paths(text: &str) -> Vec<String> {
    text.lines()
        .map(str::trim)
        .filter_map(|line| line.strip_prefix("import "))
        .filter_map(|rest| rest.strip_prefix('"'))
        .filter_map(|rest| rest.split_once('"'))
        .map(|(path, _)| path.to_string())
        .collect()
}

/// Reads `dir/import_path` for every `.ifml` import of one source and
/// provides it (plus, recursively, its own imports) under the bundle keys
/// the resolver looks up: the importing file's path key and the import
/// string exactly as written.
fn collect_imports(dir: &Path, importing_key: &str, text: &str, imports: &mut IfmlImports) {
    for import_path in import_paths(text) {
        if !import_path.ends_with(".ifml") {
            continue;
        }
        let source = fs::read_to_string(dir.join(&import_path))
            .unwrap_or_else(|error| panic!("read imported pattern {import_path}: {error}"));
        imports.provide(importing_key, &import_path, source.clone());
        collect_imports(dir, &import_path, &source, imports);
    }
}

fn compile_pattern(file_name: &str) -> IfmlCompilation {
    let text = fs::read_to_string(Path::new(PATTERNS_DIR).join(file_name))
        .unwrap_or_else(|error| panic!("read pattern {file_name}: {error}"));
    let mut imports = IfmlImports::default();
    collect_imports(Path::new(PATTERNS_DIR), file_name, &text, &mut imports);
    match compile_ifml_str(file_name, &text, &imports) {
        Ok(compilation) => compilation,
        Err(diagnostics) => {
            let rendered = diagnostics
                .iter()
                .map(|diagnostic| {
                    let (start, end) = diagnostic.span;
                    format!(
                        "  {} in {} at {start}..{end}: {}",
                        diagnostic.code, diagnostic.file, diagnostic.message
                    )
                })
                .collect::<Vec<_>>()
                .join("\n");
            panic!("{file_name} must compile with zero diagnostics:\n{rendered}");
        }
    }
}

#[test]
fn every_pattern_file_compiles_with_zero_diagnostics() {
    let mut entries: Vec<String> = fs::read_dir(PATTERNS_DIR)
        .expect("patterns directory exists")
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
    entries.sort();
    assert!(
        entries.len() >= 9,
        "nine library files expected: {entries:?}"
    );

    for file_name in &entries {
        let compilation = compile_pattern(file_name);
        assert!(
            !compilation.model.modules.is_empty(),
            "{file_name} must declare at least one module"
        );
        for use_site in compilation
            .index
            .module_uses
            .iter()
            .filter(|site| site.file == 0)
        {
            assert!(
                use_site.resolved_module.is_some(),
                "{file_name}: use '{}' must resolve",
                use_site.target
            );
        }
    }
}

const CONSUMER: &str = r#"
import "navigation.ifml";

view "Browse" {
    use "Pagination" as pager {
        page: 2;
        pageSize: 50;
        total: 250;
    };

    component "grid" {
        type: list;
        data: Item;
        fields: [name];
    }
}
"#;

#[test]
fn consumer_composes_imported_pagination() {
    let navigation = fs::read_to_string(Path::new(PATTERNS_DIR).join("navigation.ifml"))
        .expect("read navigation.ifml");
    let mut imports = IfmlImports::default();
    imports.provide("consumer.ifml", "navigation.ifml", navigation);

    let compilation = compile_ifml_str("consumer.ifml", CONSUMER, &imports)
        .expect("consumer must compile with zero diagnostics");

    assert!(
        compilation.model.modules.is_empty(),
        "the consumer declares no modules of its own"
    );
    assert_eq!(compilation.index.files.len(), 2);
    assert_eq!(compilation.index.module_uses.len(), 1);
    assert_eq!(
        compilation.index.module_uses[0].resolved_module,
        Some((1, 0))
    );
    assert_eq!(
        compilation
            .index
            .module_decl((1, 0))
            .expect("resolved declaration site")
            .name,
        "Pagination"
    );
}

const BAD_OVERRIDE: &str = r#"
import "navigation.ifml";

view "Browse" {
    use "Pagination" as pager {
        bogus: 1;
    };
}
"#;

#[test]
fn unknown_override_names_e0007() {
    let navigation = fs::read_to_string(Path::new(PATTERNS_DIR).join("navigation.ifml"))
        .expect("read navigation.ifml");
    let mut imports = IfmlImports::default();
    imports.provide("consumer.ifml", "navigation.ifml", navigation);

    let diagnostics = compile_ifml_str("consumer.ifml", BAD_OVERRIDE, &imports)
        .expect_err("unknown override name must be a diagnostic");
    assert_eq!(diagnostics.len(), 1);
    assert_eq!(diagnostics[0].code, E_UNKNOWN_INPUT);
    assert_eq!(
        diagnostics[0].message,
        "unknown input 'bogus' for module 'Pagination'"
    );
    assert_eq!(diagnostics[0].file, "consumer.ifml");
    let (start, end) = diagnostics[0].span;
    assert_eq!(&BAD_OVERRIDE[start..end], "bogus: 1;");
}
