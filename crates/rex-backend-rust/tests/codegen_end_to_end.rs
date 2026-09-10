//! End-to-end proof for the Rust backend: build the resolved IR for the
//! flagship example programmatically, generate Rust code, compile it as a
//! scratch crate, and run behavioral tests in that crate (arena semantics,
//! opposite maintenance, navigation, enums, datatypes, defaults).

use std::path::Path;

use rex_ir::{
    ClassDef, DatatypeDef, DefaultValue, EnumDef, EnumLiteral, Feature, FeatureKind, Model,
    Multiplicity, Package, PrimitiveType, TypeRef,
};

const PKG: &str = "nz.example.library";

fn class_ref(name: &str) -> TypeRef {
    TypeRef::Class {
        package: PKG.to_string(),
        name: name.to_string(),
    }
}

/// The IR equivalent of `examples/library.mox` after driver lowering:
/// attributes without multiplicity are REQUIRED, `refers`/`contains` are MANY,
/// `container` is forced OPTIONAL, ops are omitted (Tier 0), derived features
/// stay in the IR but are not materialized.
fn library_model() -> Model {
    let mut model = Model::new();
    let mut package = Package::new(PKG);
    package.enums.push(EnumDef::new(
        "BookCategory",
        vec![
            EnumLiteral::new("Mystery", Some("M".to_string()), 0),
            EnumLiteral::new("ScienceFiction", Some("S".to_string()), 1),
        ],
    ));
    package.datatypes.push(
        DatatypeDef::new("Date", None)
            .bind("rust", "chrono::NaiveDate")
            .bind("csharp", "System.DateOnly")
            .bind("java", "java.time.LocalDate"),
    );
    package.classes.push(ClassDef::new(
        "Library",
        vec![],
        vec![
            Feature::new(
                "name",
                FeatureKind::Attribute,
                TypeRef::Primitive(PrimitiveType::String),
                Multiplicity::REQUIRED,
            )
            .with_default(DefaultValue::String("Default Name".to_string())),
            Feature::new(
                "books",
                FeatureKind::Containment,
                class_ref("Book"),
                Multiplicity::MANY,
            )
            .with_opposite("Book", "library"),
        ],
    ));
    package.classes.push(ClassDef::new(
        "Book",
        vec![],
        vec![
            Feature::new(
                "library",
                FeatureKind::Container,
                class_ref("Library"),
                Multiplicity::OPTIONAL,
            )
            .with_opposite("Library", "books"),
            Feature::new(
                "title",
                FeatureKind::Attribute,
                TypeRef::Primitive(PrimitiveType::String),
                Multiplicity::REQUIRED,
            ),
            Feature::new(
                "pages",
                FeatureKind::Attribute,
                TypeRef::Primitive(PrimitiveType::Int),
                Multiplicity::REQUIRED,
            ),
            Feature::new(
                "copyright",
                FeatureKind::Attribute,
                TypeRef::Datatype {
                    package: PKG.to_string(),
                    name: "Date".to_string(),
                },
                Multiplicity::REQUIRED,
            ),
            Feature::new(
                "category",
                FeatureKind::Attribute,
                TypeRef::Enum {
                    package: PKG.to_string(),
                    name: "BookCategory".to_string(),
                },
                Multiplicity::REQUIRED,
            ),
            Feature::new(
                "authors",
                FeatureKind::CrossReference,
                class_ref("Writer"),
                Multiplicity::MANY,
            )
            .with_opposite("Writer", "books"),
            Feature::new(
                "citation",
                FeatureKind::Attribute,
                TypeRef::Primitive(PrimitiveType::String),
                Multiplicity::OPTIONAL,
            )
            .derived(),
        ],
    ));
    package.classes.push(ClassDef::new(
        "Writer",
        vec![],
        vec![
            Feature::new(
                "name",
                FeatureKind::Attribute,
                TypeRef::Primitive(PrimitiveType::String),
                Multiplicity::REQUIRED,
            ),
            Feature::new(
                "books",
                FeatureKind::CrossReference,
                class_ref("Book"),
                Multiplicity::MANY,
            )
            .with_opposite("Book", "authors"),
        ],
    ));
    model.packages.push(package);
    model
}

fn scratch_crate_files() -> Vec<(std::path::PathBuf, String)> {
    let model = library_model();
    let generated = rex_backend_rust::generate(&model).expect("generate");
    let models_rs = &generated["models.rs"];

    let manifest = env!("CARGO_MANIFEST_DIR");
    let runtime_path = Path::new(manifest)
        .join("../../crates/rex-runtime")
        .canonicalize()
        .expect("rex-runtime path");

    let lib_rs = format!(
        "{models_rs}\n{}",
        include_str!("scratch/scratch_tests.rs")
    );

    vec![
        (
            Path::new("Cargo.toml").to_path_buf(),
            format!(
                "[package]\n\
                 name = \"rex-codegen-test\"\n\
                 version = \"0.1.0\"\n\
                 edition = \"2021\"\n\
                 publish = false\n\n\
                 # Standalone scratch crate: never a member of any workspace.\n\
                 [workspace]\n\n\
                 [lib]\n\
                 path = \"src/lib.rs\"\n\n\
                 [dependencies]\n\
                 rex-runtime = {{ path = {:?} }}\n\
                 slotmap = \"1\"\n",
                runtime_path
            ),
        ),
        (Path::new("src/lib.rs").to_path_buf(), lib_rs),
    ]
}

/// Runs `cargo test` inside the scratch crate. Skips gracefully when cargo is
/// unavailable (e.g. exotic CI environments).
fn run_scratch_crate() {
    let cargo_available = std::process::Command::new("cargo")
        .arg("--version")
        .output()
        .map(|out| out.status.success())
        .unwrap_or(false);
    if !cargo_available {
        eprintln!("skipping scratch-crate test: cargo not on PATH");
        return;
    }

    let manifest = env!("CARGO_MANIFEST_DIR");
    let scratch = Path::new(manifest).join("../../target/scratch/rex-codegen-test");
    for (relative, contents) in scratch_crate_files() {
        let path = scratch.join(&relative);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("create scratch dir");
        }
        std::fs::write(path, contents).expect("write scratch file");
    }

    let run = |offline: bool| {
        let mut command = std::process::Command::new("cargo");
        command.arg("test").current_dir(&scratch);
        if offline {
            command.arg("--offline");
        }
        command.output().expect("spawn cargo")
    };

    let mut output = run(true);
    if !output.status.success() {
        eprintln!("offline scratch build failed; retrying online");
        output = run(false);
    }
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    assert!(
        output.status.success(),
        "scratch crate cargo test failed:\n--- stdout ---\n{stdout}\n--- stderr ---\n{stderr}"
    );
    eprintln!("{stdout}");
}

#[test]
fn generated_code_compiles_and_maintains_opposites() {
    run_scratch_crate();
}
