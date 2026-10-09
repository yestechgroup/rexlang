//! Resolver conformance tests: `compile_ifml_str` import resolution, module
//! table semantics (shadowing, ambiguity, duplicates), use-site validation,
//! and cycle detection. Every test exercises only the public API — the
//! resolver is filesystem-free, all import content arrives through
//! [`IfmlImports`].

use rex_ifml::{
    compile_ifml_str, IfmlCompilation, IfmlImports, E_AMBIGUOUS_MODULE, E_CIRCULAR,
    E_DUPLICATE_MODULE, E_IMPORT_NOT_PROVIDED, E_INPUT_TYPE, E_MISSING_INPUT, E_PARSE,
    E_UNKNOWN_INPUT, E_UNKNOWN_MODULE,
};

const PAGER_IFML: &str = r#"
module "Pagination" {
    input { page: Int = 1, pageSize: Int }
    output { total: Int }

    component "pager" { type: list; data: Item; }
}
"#;

const APP_WITH_USE: &str = r#"
import "pager.ifml";

view "Home" {
    use "Pagination" as pager {
        page: 2;
        pageSize: 25;
    };

    component "grid" { type: list; data: Item; }
}
"#;

fn bundle(entries: &[(&str, &str, &str)]) -> IfmlImports {
    let mut imports = IfmlImports::default();
    for (importing, import_path, text) in entries {
        imports.provide(importing, import_path, *text);
    }
    imports
}

fn compile_ok(path: &str, text: &str, imports: &IfmlImports) -> IfmlCompilation {
    let compilation =
        compile_ifml_str(path, text, imports).expect("compilation must produce no diagnostics");
    assert_eq!(compilation.index.files[0].path, path);
    compilation
}

#[test]
fn happy_path_import_and_use_resolves() {
    let imports = bundle(&[("app.ifml", "pager.ifml", PAGER_IFML)]);
    let compilation = compile_ok("app.ifml", APP_WITH_USE, &imports);

    // zero diagnostics — asserted by compile_ok

    // The artifact stays byte-identical to a plain parse: main file only,
    // imports verbatim.
    assert_eq!(compilation.model.imports, vec!["pager.ifml".to_string()]);
    assert!(compilation.model.modules.is_empty());
    let plain = rex_ifml::parse_ifml(APP_WITH_USE).unwrap();
    assert_eq!(compilation.model, plain);

    // Resolution lives in the index: (file 1 = pager.ifml, decl 0).
    assert_eq!(compilation.index.files.len(), 2);
    assert_eq!(compilation.index.files[1].path, "pager.ifml");
    assert_eq!(compilation.index.module_uses.len(), 1);
    assert_eq!(
        compilation.index.module_uses[0].resolved_module,
        Some((1, 0))
    );
    assert_eq!(
        compilation
            .index
            .module_decl((1, 0))
            .expect("decl site exists")
            .name,
        "Pagination"
    );
    // The alias and overrides survive in the index for downstream phases.
    assert_eq!(
        compilation.index.module_uses[0].alias.as_deref(),
        Some("pager")
    );
    assert_eq!(compilation.index.module_uses[0].overrides.len(), 2);
}

#[test]
fn use_without_alias_resolves() {
    let imports = bundle(&[("app.ifml", "pager.ifml", PAGER_IFML)]);
    let text = r#"
import "pager.ifml";

view "Home" {
    use "Pagination" {
        page: 1;
        pageSize: 10;
    };
}
"#;
    let compilation = compile_ok("app.ifml", text, &imports);
    assert_eq!(compilation.index.module_uses[0].alias, None);
    assert_eq!(
        compilation.index.module_uses[0].resolved_module,
        Some((1, 0))
    );
}

#[test]
fn import_not_provided() {
    let diags = compile_ifml_str(
        "app.ifml",
        "import \"pager.ifml\";",
        &IfmlImports::default(),
    )
    .expect_err("missing import content must be a diagnostic");
    assert_eq!(diags.len(), 1);
    assert_eq!(diags[0].code, E_IMPORT_NOT_PROVIDED);
    assert_eq!(
        diags[0].message,
        "imported ifml 'pager.ifml' was not provided"
    );
    assert_eq!(diags[0].span, (0, 0));
    assert_eq!(diags[0].file, "app.ifml");
}

#[test]
fn non_ifml_import_passes_through_untouched() {
    let compilation = compile_ok(
        "app.ifml",
        "import \"rexlang/auth.actor\";\n\nview \"Home\" { component \"g\" { type: list; data: Item; } }",
        &IfmlImports::default(),
    );
    assert_eq!(
        compilation.model.imports,
        vec!["rexlang/auth.actor".to_string()]
    );
}

#[test]
fn unknown_module() {
    let text = r#"
view "Home" {
    use "Missing" as gone;

    component "grid" { type: list; data: Item; }
}
"#;
    let diags = compile_ifml_str("app.ifml", text, &IfmlImports::default())
        .expect_err("unknown module must be a diagnostic");
    assert_eq!(diags.len(), 1);
    assert_eq!(diags[0].code, E_UNKNOWN_MODULE);
    assert_eq!(diags[0].message, "unknown module 'Missing'");
    assert_eq!(diags[0].file, "app.ifml");
    let (start, end) = diags[0].span;
    assert_eq!(&text[start..end], "use \"Missing\" as gone;");
}

#[test]
fn unknown_input() {
    let imports = bundle(&[("app.ifml", "pager.ifml", PAGER_IFML)]);
    let text = r#"
import "pager.ifml";

    view "Home" {
    use "Pagination" {
        bogus: 1;
        pageSize: 10;
    };
}
"#;
    let diags = compile_ifml_str("app.ifml", text, &imports)
        .expect_err("unknown override name must be a diagnostic");
    assert_eq!(diags.len(), 1);
    assert_eq!(diags[0].code, E_UNKNOWN_INPUT);
    assert_eq!(
        diags[0].message,
        "unknown input 'bogus' for module 'Pagination'"
    );
    assert_eq!(&text[diags[0].span.0..diags[0].span.1], "bogus: 1;");
    assert_eq!(diags[0].file, "app.ifml");
}

#[test]
fn override_of_module_property_is_fine() {
    let text = r#"
import "pager.ifml";

    view "Home" {
        use "Pagination" {
            title: "dark";
        };
    }
"#;
    let lib = r#"
module "Pagination" {
    input { page: Int = 1, pageSize: Int = 10 }
    output { total: Int }

    title: "Pager";

    component "pager" { type: list; data: Item; }
}
"#;
    let imports = bundle(&[("app.ifml", "pager.ifml", lib)]);
    let compilation = compile_ok("app.ifml", text, &imports);
    assert_eq!(
        compilation.index.module_uses[0].resolved_module,
        Some((1, 0))
    );
}

#[test]
fn missing_required_input() {
    let imports = bundle(&[("app.ifml", "pager.ifml", PAGER_IFML)]);
    let text = r#"
import "pager.ifml";

view "Home" {
    use "Pagination" {
        page: 3;
    };
}
"#;
    let diags = compile_ifml_str("app.ifml", text, &imports)
        .expect_err("default-less input without override must be a diagnostic");
    assert_eq!(diags.len(), 1);
    assert_eq!(diags[0].code, E_MISSING_INPUT);
    assert_eq!(
        diags[0].message,
        "module 'Pagination' requires input 'pageSize'"
    );
    assert_eq!(diags[0].file, "app.ifml");
    let (start, end) = diags[0].span;
    assert!(text[start..end].starts_with("use \"Pagination\""));
}

#[test]
fn literal_type_mismatch() {
    let imports = bundle(&[("app.ifml", "pager.ifml", PAGER_IFML)]);
    let text = r#"
import "pager.ifml";

view "Home" {
    use "Pagination" {
        pageSize: "many";
    };
}
"#;
    let diags = compile_ifml_str("app.ifml", text, &imports)
        .expect_err("string literal for Int input must be a diagnostic");
    assert_eq!(diags.len(), 1);
    assert_eq!(diags[0].code, E_INPUT_TYPE);
    assert_eq!(
        diags[0].message,
        "input 'pageSize' of module 'Pagination' expects Int"
    );
    assert_eq!(
        &text[diags[0].span.0..diags[0].span.1],
        "pageSize: \"many\";"
    );
}

#[test]
fn literal_type_matches_pass_silently() {
    let imports = bundle(&[("app.ifml", "pager.ifml", PAGER_IFML)]);
    let text = r#"
import "pager.ifml";

view "Home" {
    use "Pagination" {
        page: 7;
        pageSize: 25;
    };
}
"#;
    compile_ok("app.ifml", text, &imports);
}

#[test]
fn duplicate_module_across_imports_is_ambiguous() {
    let a = PAGER_IFML;
    let b = PAGER_IFML.replace("pager\" {", "pager2\" {");
    let imports = bundle(&[("app.ifml", "a.ifml", a), ("app.ifml", "b.ifml", &b)]);
    let text = r#"
import "a.ifml";
import "b.ifml";

view "Home" {
    use "Pagination" {
        page: 1;
        pageSize: 10;
    };
}
"#;
    let diags = compile_ifml_str("app.ifml", text, &imports)
        .expect_err("module exported by two imports must be ambiguous");
    assert_eq!(
        diags.len(),
        1,
        "the use itself is not re-reported: {diags:?}"
    );
    assert_eq!(diags[0].code, E_AMBIGUOUS_MODULE);
    assert!(diags[0]
        .message
        .starts_with("ambiguous module 'Pagination'"),);
    assert_eq!(diags[0].file, "b.ifml");
}

#[test]
fn local_module_shadows_imported_one() {
    let imports = bundle(&[("app.ifml", "lib.ifml", PAGER_IFML)]);
    let text = r#"
import "lib.ifml";

view "Home" {
    use "Pagination" {
        page: 1;
        pageSize: 10;
    };
}

module "Pagination" {
    input { page: Int, pageSize: Int }
    output { total: Int }
}
"#;
    let compilation = compile_ok("app.ifml", text, &imports);
    // Resolves to the LOCAL declaration: file 0, decl 0.
    assert_eq!(
        compilation.index.module_uses[0].resolved_module,
        Some((0, 0))
    );
    assert_eq!(compilation.index.module_decls.len(), 2);
    assert_eq!(compilation.index.module_decls[0].file, 0);
    assert_eq!(compilation.index.module_decls[1].file, 1);
}

#[test]
fn duplicate_module_within_one_file() {
    let text = r#"
view "Home" {
    use "Dup" {
        x: 1;
    };
}

module "Dup" {
    input { x: Int }
    output { y: Int }
}

module "Dup" {
    input { x: Int }
    output { y: Int }
}
"#;
    let diags = compile_ifml_str("app.ifml", text, &IfmlImports::default())
        .expect_err("duplicate module in one file must be a diagnostic");
    assert_eq!(diags.len(), 1);
    assert_eq!(diags[0].code, E_DUPLICATE_MODULE);
    assert_eq!(diags[0].message, "duplicate module 'Dup'");
    assert_eq!(diags[0].file, "app.ifml");
}

#[test]
fn circular_import_detected() {
    let a = "import \"b.ifml\";\n\nmodule \"A\" { input { x: Int } output { y: Int } }";
    let b = "import \"a.ifml\";\n\nmodule \"B\" { input { x: Int } output { y: Int } }";
    let imports = bundle(&[("app.ifml", "a.ifml", a), ("a.ifml", "b.ifml", b)]);
    let diags = compile_ifml_str("app.ifml", "import \"a.ifml\";", &imports)
        .expect_err("circular import must be a diagnostic");
    assert_eq!(diags.len(), 1);
    assert_eq!(diags[0].code, E_CIRCULAR);
    assert_eq!(diags[0].message, "circular ifml import involving 'a.ifml'");
    assert_eq!(diags[0].file, "b.ifml");
}

#[test]
fn self_import_is_circular() {
    let diags = compile_ifml_str("app.ifml", "import \"app.ifml\";", &IfmlImports::default())
        .expect_err("self-import must be a diagnostic");
    assert_eq!(diags.len(), 1);
    assert_eq!(diags[0].code, E_CIRCULAR);
    assert_eq!(
        diags[0].message,
        "circular ifml import involving 'app.ifml'"
    );
    assert_eq!(diags[0].file, "app.ifml");
}

#[test]
fn diamond_import_parses_shared_file_once() {
    let shared = PAGER_IFML;
    let x = "import \"shared.ifml\";";
    let y = "import \"shared.ifml\";";
    let imports = bundle(&[
        ("app.ifml", "x.ifml", x),
        ("app.ifml", "y.ifml", y),
        ("x.ifml", "shared.ifml", shared),
        ("y.ifml", "shared.ifml", shared),
    ]);
    let text = "import \"x.ifml\";\nimport \"y.ifml\";\n\nview \"Home\" { component \"g\" { type: list; data: Item; } }";
    let compilation = compile_ok("app.ifml", text, &imports);
    assert_eq!(compilation.index.files.len(), 4, "shared.ifml once");
    assert_eq!(compilation.index.module_decls.len(), 1);
}

#[test]
fn module_internal_use_resolves() {
    let imports = bundle(&[("app.ifml", "inner.ifml", PAGER_IFML)]);
    let text = r#"
import "inner.ifml";

module "MasterDetail" {
    input { entityId: Uuid }
    output { selected: Uuid }

    use "Pagination" as inner {
        page: 1;
        pageSize: 20;
    }
}
"#;
    let compilation = compile_ok("app.ifml", text, &imports);
    // The module-internal use is a main-file use: resolved.
    assert_eq!(compilation.index.module_uses.len(), 1);
    assert_eq!(
        compilation.index.module_uses[0].resolved_module,
        Some((1, 0))
    );
    // And it stays on the artifact's module declaration, verbatim.
    assert_eq!(compilation.model.modules.len(), 1);
    assert_eq!(compilation.model.modules[0].module_uses.len(), 1);
    assert_eq!(
        compilation.model.modules[0].module_uses[0].module,
        "Pagination"
    );
}

#[test]
fn parse_error_in_import_is_a_diagnostic() {
    let imports = bundle(&[("app.ifml", "broken.ifml", "module \"Oops\" {")]);
    let diags = compile_ifml_str("app.ifml", "import \"broken.ifml\";", &imports)
        .expect_err("unparseable import must be a diagnostic");
    assert_eq!(diags.len(), 1);
    assert_eq!(diags[0].code, E_PARSE);
    assert_eq!(diags[0].span, (0, 0));
    assert_eq!(diags[0].file, "broken.ifml");
    assert!(diags[0].message.contains("Parse error"), "{:?}", diags[0]);
}

#[test]
fn parse_error_in_main_file_is_a_diagnostic() {
    let diags = compile_ifml_str("app.ifml", "view \"Broken\"", &IfmlImports::default())
        .expect_err("unparseable main file must be a diagnostic");
    assert_eq!(diags.len(), 1);
    assert_eq!(diags[0].code, E_PARSE);
    assert_eq!(diags[0].file, "app.ifml");
}

#[test]
fn default_imports_and_no_uses_compile_clean() {
    let compilation = compile_ok(
        "app.ifml",
        "view \"Home\" { component \"g\" { type: list; data: Item; } }",
        &IfmlImports::default(),
    );
    assert!(compilation.model.imports.is_empty());
    assert!(compilation.model.modules.is_empty());
    assert!(compilation.index.module_uses.is_empty());
    assert_eq!(compilation.index.files.len(), 1);
}

#[test]
fn nested_imports_resolve_depth_first_in_source_order() {
    let deep = PAGER_IFML;
    let mid = "import \"deep.ifml\";\n\nmodule \"Mid\" { input { x: Int } output { y: Int } }";
    let imports = bundle(&[
        ("app.ifml", "mid.ifml", mid),
        ("app.ifml", "deep.ifml", deep),
        ("mid.ifml", "deep.ifml", deep),
    ]);
    let text = r#"
import "mid.ifml";
import "deep.ifml";

view "Home" {
    use "Mid" {
        x: 1;
    };
    use "Pagination" {
        page: 1;
        pageSize: 5;
    };
}
"#;
    let compilation = compile_ok("app.ifml", text, &imports);
    // Compilation order: app, mid, deep — depth-first per import.
    assert_eq!(
        compilation
            .index
            .files
            .iter()
            .map(|f| f.path.as_str())
            .collect::<Vec<_>>(),
        vec!["app.ifml", "mid.ifml", "deep.ifml"]
    );
    let mid_ref = compilation.index.module_uses[0].resolved_module;
    let pager_ref = compilation.index.module_uses[1].resolved_module;
    assert_eq!(mid_ref, Some((1, 0)));
    assert_eq!(pager_ref, Some((2, 0)));
}

#[test]
fn imported_files_uses_are_recorded_but_not_validated() {
    // wrapper.ifml carries a `use` with an unknown-input override and a
    // missing-required-input; none of that may surface as a diagnostic —
    // only main-file uses are validated.
    let wrapper = r#"
import "pager.ifml";

view "Wrapped" {
    use "Pagination" {
        page: 99;
        not_an_input: true;
    };
}
"#;
    let imports = bundle(&[
        ("app.ifml", "wrapper.ifml", wrapper),
        ("wrapper.ifml", "pager.ifml", PAGER_IFML),
    ]);
    let compilation = compile_ok("app.ifml", "import \"wrapper.ifml\";", &imports);
    assert_eq!(compilation.index.module_uses.len(), 1);
    assert_eq!(compilation.index.module_uses[0].file, 1);
    // Resolution is compilation-wide (expansion and navigation need the
    // full graph); VALIDATION stays main-file only — the invalid overrides
    // produced no diagnostics.
    assert_eq!(
        compilation.index.module_uses[0].resolved_module,
        Some((2, 0)),
        "wrapper's use resolves to Pagination in pager.ifml (file 2)"
    );
}
