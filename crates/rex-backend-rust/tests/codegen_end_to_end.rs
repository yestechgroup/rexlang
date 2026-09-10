//! End-to-end proof for the Rust backend: build the resolved IR for the
//! flagship example programmatically, generate Rust code, compile it as a
//! scratch crate, and run behavioral tests in that crate (arena semantics,
//! opposite maintenance, navigation, enums, datatypes, defaults). A second
//! scenario exercises vocabulary codegen with the conformance currency model.

use std::path::Path;

use rex_ir::{
    ClassDef, DatatypeDef, DefaultValue, EnumDef, EnumLiteral, Feature, FeatureKind, Model,
    Multiplicity, Package, PrimitiveType, TypeRef, VocabularyDef, VocabularyEntry,
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

    let lib_rs = format!("{models_rs}\n{}", include_str!("scratch/scratch_tests.rs"));

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
                  slotmap = \"1\"\n\
                  serde_json = \"1\"\n\n\
                  [dev-dependencies]\n\
                  proptest = \"1\"\n",
                runtime_path
            ),
        ),
        (Path::new("src/lib.rs").to_path_buf(), lib_rs),
    ]
}

/// Runs `cargo test` inside the scratch crate, then checks the conformance
/// instance JSON it produced against the committed golden fixture (or updates
/// the fixture with `REX_UPDATE_FIXTURES=1`). Skips gracefully when cargo is
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
    let instance_out = scratch.join("library.instance.json");
    std::fs::create_dir_all(&scratch).expect("create scratch root");
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
        command.env("REX_INSTANCE_OUT", &instance_out);
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

    check_instance_fixture(&instance_out);
}

/// Conformance: the canonical instance JSON produced by the generated code
/// must byte-match the committed golden fixture.
fn check_instance_fixture(instance_out: &Path) {
    let produced = std::fs::read_to_string(instance_out).expect("scratch instance output");
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../")
        .canonicalize()
        .expect("workspace root");
    let fixture = root.join("tests/conformance/instances/library.instance.json");
    if std::env::var("REX_UPDATE_FIXTURES").is_ok() {
        std::fs::create_dir_all(fixture.parent().unwrap()).unwrap();
        std::fs::write(&fixture, &produced).unwrap();
        eprintln!("updated {}", fixture.display());
        return;
    }
    let expected = std::fs::read_to_string(&fixture).expect("golden instance fixture");
    assert_eq!(
        produced, expected,
        "canonical instance JSON drifted from the golden fixture"
    );
}

#[test]
fn generated_code_compiles_and_maintains_opposites() {
    run_scratch_crate();
}

const CURRENCY_PKG: &str = "nz.example.payments";

fn currency_entry(key: &str, symbol: &str, minor_units: i64) -> VocabularyEntry {
    VocabularyEntry {
        key: key.to_string(),
        facets: std::collections::BTreeMap::from([
            (
                "symbol".to_string(),
                DefaultValue::String(symbol.to_string()),
            ),
            ("minorUnits".to_string(), DefaultValue::Int(minor_units)),
        ]),
    }
}

/// The IR equivalent of `tests/conformance/models/currency.mox`: one
/// vocabulary-typed attribute (REQUIRED) plus its String sibling.
fn currency_model() -> Model {
    let mut model = Model::new();
    let mut package = Package::new(CURRENCY_PKG);
    package.vocabularies.push(VocabularyDef {
        name: "Currency".to_string(),
        source: "iso:4217".to_string(),
        version: Some("2024-01-01".to_string()),
        key: "alpha3".to_string(),
        facets: vec![
            rex_ir::VocabularyFacet {
                name: "symbol".to_string(),
                type_: PrimitiveType::String,
            },
            rex_ir::VocabularyFacet {
                name: "minorUnits".to_string(),
                type_: PrimitiveType::Int,
            },
        ],
        entries: vec![
            currency_entry("USD", "$", 2),
            currency_entry("EUR", "€", 2),
            currency_entry("JPY", "¥", 0),
            currency_entry("GBP", "£", 2),
            currency_entry("CHF", "CHF", 2),
        ],
    });
    package.classes.push(ClassDef::new(
        "Account",
        vec![],
        vec![
            Feature::new(
                "owner",
                FeatureKind::Attribute,
                TypeRef::Primitive(PrimitiveType::String),
                Multiplicity::REQUIRED,
            ),
            Feature::new(
                "currency",
                FeatureKind::Attribute,
                TypeRef::Vocabulary {
                    package: CURRENCY_PKG.to_string(),
                    name: "Currency".to_string(),
                },
                Multiplicity::REQUIRED,
            ),
        ],
    ));
    model.packages.push(package);
    model
}

fn currency_scratch_crate_files() -> Vec<(std::path::PathBuf, String)> {
    let model = currency_model();
    let generated = rex_backend_rust::generate(&model).expect("generate");
    let models_rs = &generated["models.rs"];

    let manifest = env!("CARGO_MANIFEST_DIR");
    let runtime_path = Path::new(manifest)
        .join("../../crates/rex-runtime")
        .canonicalize()
        .expect("rex-runtime path");

    let lib_rs = format!("{models_rs}\n{}", include_str!("scratch/currency_tests.rs"));

    vec![
        (
            Path::new("Cargo.toml").to_path_buf(),
            format!(
                "[package]\n\
                 name = \"rex-vocab-test\"\n\
                 version = \"0.1.0\"\n\
                 edition = \"2021\"\n\
                 publish = false\n\n\
                 # Standalone scratch crate: never a member of any workspace.\n\
                 [workspace]\n\n\
                 [lib]\n\
                 path = \"src/lib.rs\"\n\n\
                 [dependencies]\n\
                 rex-runtime = {{ path = {:?} }}\n\
                 slotmap = \"1\"\n\
                 serde_json = \"1\"\n",
                runtime_path
            ),
        ),
        (Path::new("src/lib.rs").to_path_buf(), lib_rs),
    ]
}

/// Runs `cargo test` inside the currency scratch crate, then checks the
/// canonical instance JSON it produced against the committed golden fixture
/// (or updates the fixture with `REX_UPDATE_FIXTURES=1`).
fn run_currency_scratch_crate() {
    let cargo_available = std::process::Command::new("cargo")
        .arg("--version")
        .output()
        .map(|out| out.status.success())
        .unwrap_or(false);
    if !cargo_available {
        eprintln!("skipping currency scratch-crate test: cargo not on PATH");
        return;
    }

    let manifest = env!("CARGO_MANIFEST_DIR");
    let scratch = Path::new(manifest).join("../../target/scratch/rex-vocab-test");
    let instance_out = scratch.join("currency.instance.json");
    std::fs::create_dir_all(&scratch).expect("create scratch root");
    for (relative, contents) in currency_scratch_crate_files() {
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
        command.env("REX_CURRENCY_INSTANCE_OUT", &instance_out);
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
        "currency scratch crate cargo test failed:\n--- stdout ---\n{stdout}\n--- stderr ---\n{stderr}"
    );
    eprintln!("{stdout}");

    check_currency_instance_fixture(&instance_out);
}

/// Conformance: the canonical instance JSON for the currency scenario must
/// byte-match the committed golden fixture.
fn check_currency_instance_fixture(instance_out: &Path) {
    let produced = std::fs::read_to_string(instance_out).expect("scratch instance output");
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../")
        .canonicalize()
        .expect("workspace root");
    let fixture = root.join("tests/conformance/instances/currency.instance.json");
    if std::env::var("REX_UPDATE_FIXTURES").is_ok() {
        std::fs::create_dir_all(fixture.parent().unwrap()).unwrap();
        std::fs::write(&fixture, &produced).unwrap();
        eprintln!("updated {}", fixture.display());
        return;
    }
    let expected = std::fs::read_to_string(&fixture).expect("golden instance fixture");
    assert_eq!(
        produced, expected,
        "canonical currency instance JSON drifted from the golden fixture"
    );
}

#[test]
fn generated_vocabulary_code_compiles_and_round_trips_by_key() {
    run_currency_scratch_crate();
}
