//! Integration tests for `import schema` declarations: a `.mox` file may
//! import a JSON Schema type into its package namespace. v1 lowering is
//! deliberately opaque — an import registers a nominal, feature-less class —
//! and the JSON content is validated even though nothing is lowered from it.
//!
//! The driver is filesystem-free: import content arrives through
//! [`rex_driver::SchemaImports`], keyed by `(mox file path, import path)`.

use rex_driver::{
    compile_actors_str_with_imports, compile_files, compile_files_with_imports, render,
    SchemaImports,
};
use rex_ir::{FeatureKind, TypeRef};

const MOX: &str = "demo.mox";

fn mox_with_imports() -> String {
    concat!(
        "package demo\n\n",
        "import schema \"schemas/todo_item.json\" as TodoItem\n",
        "import schema \"schemas/todo_list.json\"\n\n",
        "class Wrapper {\n",
        "    refers TodoItem item\n",
        "    refers todo_list list\n",
        "}\n"
    )
    .to_string()
}

fn valid_imports() -> SchemaImports {
    SchemaImports::new()
        .provide(MOX, "schemas/todo_item.json", "{\"title\": \"Todo item\"}")
        .provide(MOX, "schemas/todo_list.json", "{\"title\": \"Todo list\"}")
}

fn compile_clean(compilation: &rex_driver::MultiCompilation) -> &rex_ir::Model {
    let source = mox_with_imports();
    assert!(
        compilation.diagnostics.is_empty(),
        "expected a clean compilation:\n{}",
        compilation
            .diagnostics
            .iter()
            .map(|(path, diagnostic)| render(
                path,
                if path == MOX { source.as_str() } else { "" },
                std::slice::from_ref(diagnostic),
            ))
            .collect::<String>()
    );
    compilation.model.as_ref().expect("model lowered")
}

// --- v1 lowering: opaque nominal classes --------------------------------------

#[test]
fn import_with_alias_resolves_as_a_nominal_class() {
    let compilation =
        compile_files_with_imports(&[(MOX.to_string(), mox_with_imports())], &valid_imports());
    let model = compile_clean(&compilation);
    let package = &model.packages[0];

    // The imported names are nominal, feature-less classes of the importing
    // package.
    let imported: Vec<&rex_ir::ClassDef> = package
        .classes
        .iter()
        .filter(|class| class.name == "TodoItem" || class.name == "todo_list")
        .collect();
    assert_eq!(imported.len(), 2, "{:?}", package.classes);
    for class in imported {
        assert!(class.features.is_empty(), "v1 imports are feature-less");
        assert!(class.extends.is_empty());
    }

    // References resolve to `TypeRef::Class` in the importing package.
    let wrapper = package
        .classes
        .iter()
        .find(|class| class.name == "Wrapper")
        .expect("the referring class");
    assert_eq!(
        wrapper.features[0].kind,
        FeatureKind::CrossReference,
        "an imported type is a referencable class"
    );
    assert_eq!(
        wrapper.features[0].type_,
        TypeRef::Class {
            package: "demo".to_string(),
            name: "TodoItem".to_string(),
        }
    );
    assert_eq!(
        wrapper.features[1].type_,
        TypeRef::Class {
            package: "demo".to_string(),
            name: "todo_list".to_string(),
        }
    );
}

#[test]
fn import_without_alias_takes_the_file_stem() {
    let compilation = compile_files_with_imports(
        &[(
            MOX.to_string(),
            "package demo\n\nimport schema \"deep/dir/my_type.json\"\n\nclass C { refers my_type t }\n"
                .to_string(),
        )],
        &SchemaImports::new().provide(MOX, "deep/dir/my_type.json", "{}"),
    );
    let model = compile_clean(&compilation);
    let class = &model.packages[0]
        .classes
        .iter()
        .find(|class| class.name == "C")
        .expect("class C");
    assert_eq!(
        class.features[0].type_,
        TypeRef::Class {
            package: "demo".to_string(),
            name: "my_type".to_string(),
        }
    );
}

// --- content validation --------------------------------------------------------

#[test]
fn invalid_json_is_an_error_naming_the_path() {
    let compilation = compile_files_with_imports(
        &[(
            MOX.to_string(),
            "package demo\n\nimport schema \"broken.json\" as B\n\nclass C { refers B b }\n"
                .to_string(),
        )],
        &SchemaImports::new().provide(MOX, "broken.json", "{ not json"),
    );
    assert!(compilation.model.is_none(), "invalid JSON blocks lowering");
    let (_, diagnostic) = compilation
        .diagnostics
        .iter()
        .find(|(_, d)| d.message.contains("broken.json"))
        .expect("a diagnostic naming the import path");
    assert!(diagnostic.is_error());
    assert!(
        diagnostic.message.contains("JSON"),
        "the diagnostic must say the content is not valid JSON: {}",
        diagnostic.message
    );
}

#[test]
fn missing_content_is_an_error_tagged_with_the_importing_file() {
    let compilation = compile_files_with_imports(
        &[(
            MOX.to_string(),
            "package demo\n\nimport schema \"gone.json\" as G\n\nclass C { refers G g }\n"
                .to_string(),
        )],
        &SchemaImports::new(),
    );
    assert!(compilation.model.is_none());
    let (path, diagnostic) = compilation
        .diagnostics
        .iter()
        .find(|(_, d)| d.message.contains("was not provided"))
        .expect("the not-provided diagnostic");
    assert_eq!(path, &MOX, "the diagnostic is tagged with the .mox file");
    assert_eq!(
        diagnostic.message,
        "imported schema 'gone.json' was not provided"
    );
}

#[test]
fn content_provided_for_another_file_does_not_satisfy_the_import() {
    // The (mox path, import path) pair is the key: another file's content
    // for the same import path must not leak.
    let compilation = compile_files_with_imports(
        &[
            (MOX.to_string(), mox_with_imports()),
            (
                "other.mox".to_string(),
                "package other\n\nclass Unrelated { String x }\n".to_string(),
            ),
        ],
        &SchemaImports::new().provide("other.mox", "schemas/todo_item.json", "{}"),
    );
    assert!(
        compilation
            .diagnostics
            .iter()
            .any(|(path, d)| path == MOX && d.message.contains("was not provided")),
        "content keyed by another .mox file must not satisfy the import: {:?}",
        compilation.diagnostics
    );
}

// --- collisions -----------------------------------------------------------------

#[test]
fn alias_colliding_with_a_declared_class_is_an_error_naming_both_sources() {
    let compilation = compile_files_with_imports(
        &[(
            MOX.to_string(),
            "package demo\n\nimport schema \"x.json\" as Book\n\nclass Book { String title }\n"
                .to_string(),
        )],
        &SchemaImports::new().provide(MOX, "x.json", "{}"),
    );
    assert!(compilation.model.is_none());
    let (_, diagnostic) = compilation
        .diagnostics
        .iter()
        .find(|(_, d)| d.is_error())
        .expect("collision error");
    assert!(
        diagnostic.message.contains("Book") && diagnostic.message.contains("x.json"),
        "the error must name both the declared name and the import path: {}",
        diagnostic.message
    );
}

#[test]
fn alias_colliding_with_enum_datatype_vocabulary_interface_names_is_an_error() {
    for (decl, name) in [
        ("enum Book { Red = 0 }", "enum"),
        ("type Book wraps opaque {}", "datatype"),
        ("interface Book {}", "interface"),
    ] {
        let source = format!("package demo\n\nimport schema \"x.json\" as Book\n\n{decl}\n");
        let compilation = compile_files_with_imports(
            &[(MOX.to_string(), source)],
            &SchemaImports::new().provide(MOX, "x.json", "{}"),
        );
        assert!(
            compilation.diagnostics.iter().any(|(_, d)| d.is_error()),
            "an import alias must not shadow a declared {name}"
        );
    }
}

#[test]
fn the_same_alias_imported_twice_is_an_error() {
    let compilation = compile_files_with_imports(
        &[(
            MOX.to_string(),
            concat!(
                "package demo\n\n",
                "import schema \"a.json\" as Dup\n",
                "import schema \"b.json\" as Dup\n\n",
                "class C { String x }\n"
            )
            .to_string(),
        )],
        &SchemaImports::new()
            .provide(MOX, "a.json", "{}")
            .provide(MOX, "b.json", "{}"),
    );
    assert!(compilation.model.is_none());
    let (_, diagnostic) = compilation
        .diagnostics
        .iter()
        .find(|(_, d)| d.is_error())
        .expect("duplicate-import error");
    assert!(
        diagnostic.message.contains("Dup")
            && diagnostic.message.contains("a.json")
            && diagnostic.message.contains("b.json"),
        "the error must name both import paths: {}",
        diagnostic.message
    );
}

#[test]
fn colliding_stems_are_an_error() {
    let compilation = compile_files_with_imports(
        &[(
            MOX.to_string(),
            concat!(
                "package demo\n\n",
                "import schema \"one/stem.json\"\n",
                "import schema \"two/stem.json\"\n\n",
                "class C { String x }\n"
            )
            .to_string(),
        )],
        &SchemaImports::new()
            .provide(MOX, "one/stem.json", "{}")
            .provide(MOX, "two/stem.json", "{}"),
    );
    assert!(compilation.model.is_none());
    let (_, diagnostic) = compilation
        .diagnostics
        .iter()
        .find(|(_, d)| d.is_error())
        .expect("stem-collision error");
    assert!(
        diagnostic.message.contains("stem") && diagnostic.message.contains("json"),
        "the error must name both colliding imports: {}",
        diagnostic.message
    );
}

// --- provider contract -----------------------------------------------------------

#[test]
fn compile_files_is_unchanged_without_imports() {
    // Byte compatibility: a model without `import schema` compiles to the
    // identical IR and diagnostics whether or not the new API is used.
    let files = vec![
        (
            "a.mox".to_string(),
            "package a\n\nclass Book { String title }".to_string(),
        ),
        (
            "b.mox".to_string(),
            "package b\n\nclass Shelf { refers a.Book[] links }".to_string(),
        ),
    ];
    let plain = compile_files(&files);
    let with = compile_files_with_imports(&files, &SchemaImports::new());
    assert_eq!(
        plain
            .model
            .as_ref()
            .map(|model| model.to_json_pretty().expect("serialize")),
        with.model
            .as_ref()
            .map(|model| model.to_json_pretty().expect("serialize")),
        "IR must be identical without import-schema declarations"
    );
    assert_eq!(plain.diagnostics, with.diagnostics);
}

#[test]
fn schema_remains_usable_as_an_identifier_in_the_driver() {
    let compilation = compile_files_with_imports(
        &[(
            MOX.to_string(),
            "package demo\n\nclass schema { String schema }\n\nclass Other { refers schema[] many }\n"
                .to_string(),
        )],
        &SchemaImports::new(),
    );
    let model = compile_clean(&compilation);
    let names: Vec<&str> = model.packages[0]
        .classes
        .iter()
        .map(|class| class.name.as_str())
        .collect();
    assert_eq!(names, vec!["schema", "Other"]);
}

// --- .actor union path -------------------------------------------------------------

#[test]
fn actor_file_capabilities_resolve_imported_schema_names() {
    let domain = concat!(
        "package demo\n\n",
        "import schema \"schemas/todo_item.json\" as TodoItem\n\n",
        "class Wrapper { refers TodoItem item }\n"
    );
    let actor = concat!(
        "import \"demo.mox\"\n\n",
        "actors Ops {\n",
        "    actor Agent\n",
        "    capability TouchItem on TodoItem\n",
        "    grant Agent { permit TouchItem }\n",
        "}\n"
    );
    let imports = SchemaImports::new().provide(
        "demo.mox",
        "schemas/todo_item.json",
        "{\"title\": \"Todo item\"}",
    );
    let compilation = compile_actors_str_with_imports(
        "ops.actor",
        actor,
        &[("demo.mox".to_string(), domain.to_string())],
        &imports,
    );
    assert!(
        compilation.diagnostics.is_empty(),
        "the actor union must resolve imported schema names:\n{:?}",
        compilation.diagnostics
    );
    let model = compilation.model.expect("actor model lowered");
    let block = &model.blocks[0];
    assert_eq!(
        block.capabilities[0].class,
        TypeRef::Class {
            package: "demo".to_string(),
            name: "TodoItem".to_string(),
        }
    );
}
