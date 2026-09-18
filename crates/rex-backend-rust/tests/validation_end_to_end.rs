//! End-to-end proof for runtime constraint validation (issue #12): the
//! generated `validate` method must report violations for objects that
//! offend a declared bound and report nothing for fully valid objects. The
//! model is the hand-built IR sibling of `validation_model()` in
//! `codegen.rs` — every v1-supported constraint shape (string bounds, many-
//! valued, optional, int bounds, enum value bounds, datatype length bounds,
//! String-keyed and Int-keyed vocabulary bounds) plus an unconstrained
//! sibling — compiled into its own scratch crate like the library and
//! currency scenarios.

use std::path::Path;

use rex_ir::{
    ClassDef, DatatypeDef, DefaultValue, EnumDef, EnumLiteral, Feature, FeatureKind, Model,
    Multiplicity, Package, PrimitiveType, TypeRef, VocabularyDef, VocabularyEntry,
};

const PKG: &str = "nz.example.validation";

fn constrained(feature: Feature, constraints: rex_ir::FeatureConstraints) -> Feature {
    let mut feature = feature;
    feature.constraints = constraints;
    feature
}

fn string_bounds(min: u64, max: u64) -> rex_ir::FeatureConstraints {
    rex_ir::FeatureConstraints {
        min_length: Some(min),
        max_length: Some(max),
        ..Default::default()
    }
}

fn numeric_bounds(min: i64, max: i64) -> rex_ir::FeatureConstraints {
    rex_ir::FeatureConstraints {
        minimum: Some(min),
        maximum: Some(max),
        ..Default::default()
    }
}

fn keyed_entry(key: &str, code: DefaultValue) -> VocabularyEntry {
    VocabularyEntry {
        key: key.to_string(),
        facets: std::collections::BTreeMap::from([("code".to_string(), code)]),
    }
}

fn validation_model() -> Model {
    let mut model = Model::new();
    let mut package = Package::new(PKG);
    package.enums.push(EnumDef::new(
        "Rank",
        vec![
            EnumLiteral::new("Low", None, 1),
            EnumLiteral::new("High", None, 9),
        ],
    ));
    package.datatypes.push(DatatypeDef::new("Meta", None));
    package.vocabularies.push(VocabularyDef {
        name: "Grade".to_string(),
        description: None,
        source: "test:grades".to_string(),
        version: None,
        key: "code".to_string(),
        facets: vec![rex_ir::VocabularyFacet {
            name: "code".to_string(),
            type_: PrimitiveType::String,
        }],
        entries: vec![
            keyed_entry("US", DefaultValue::String("US".to_string())),
            keyed_entry("EURO", DefaultValue::String("EURO".to_string())),
        ],
    });
    package.vocabularies.push(VocabularyDef {
        name: "Level".to_string(),
        description: None,
        source: "test:levels".to_string(),
        version: None,
        key: "code".to_string(),
        facets: vec![rex_ir::VocabularyFacet {
            name: "code".to_string(),
            type_: PrimitiveType::Int,
        }],
        entries: vec![
            keyed_entry("A", DefaultValue::Int(2)),
            keyed_entry("B", DefaultValue::Int(7)),
        ],
    });
    let string = || TypeRef::Primitive(PrimitiveType::String);
    package.classes.push(ClassDef::new(
        "Profile",
        vec![],
        vec![
            constrained(
                Feature::new(
                    "name",
                    FeatureKind::Attribute,
                    string(),
                    Multiplicity::REQUIRED,
                ),
                string_bounds(2, 8),
            ),
            constrained(
                Feature::new(
                    "nicknames",
                    FeatureKind::Attribute,
                    string(),
                    Multiplicity::MANY,
                ),
                string_bounds(2, 4),
            ),
            constrained(
                Feature::new(
                    "score",
                    FeatureKind::Attribute,
                    TypeRef::Primitive(PrimitiveType::Int),
                    Multiplicity::REQUIRED,
                ),
                numeric_bounds(0, 100),
            ),
            constrained(
                Feature::new(
                    "rank",
                    FeatureKind::Attribute,
                    TypeRef::Enum {
                        package: PKG.to_string(),
                        name: "Rank".to_string(),
                    },
                    Multiplicity::REQUIRED,
                ),
                numeric_bounds(2, 10),
            ),
            constrained(
                Feature::new(
                    "meta",
                    FeatureKind::Attribute,
                    TypeRef::Datatype {
                        package: PKG.to_string(),
                        name: "Meta".to_string(),
                    },
                    Multiplicity::REQUIRED,
                ),
                string_bounds(4, 64),
            ),
            constrained(
                Feature::new(
                    "grade",
                    FeatureKind::Attribute,
                    TypeRef::Vocabulary {
                        package: PKG.to_string(),
                        name: "Grade".to_string(),
                    },
                    Multiplicity::REQUIRED,
                ),
                rex_ir::FeatureConstraints {
                    max_length: Some(3),
                    ..Default::default()
                },
            ),
            constrained(
                Feature::new(
                    "level",
                    FeatureKind::Attribute,
                    TypeRef::Vocabulary {
                        package: PKG.to_string(),
                        name: "Level".to_string(),
                    },
                    Multiplicity::REQUIRED,
                ),
                rex_ir::FeatureConstraints {
                    minimum: Some(5),
                    ..Default::default()
                },
            ),
            constrained(
                Feature::new(
                    "alt_name",
                    FeatureKind::Attribute,
                    string(),
                    Multiplicity::OPTIONAL,
                ),
                string_bounds(2, 32),
            ),
            Feature::new(
                "note",
                FeatureKind::Attribute,
                string(),
                Multiplicity::REQUIRED,
            ),
        ],
    ));
    model.packages.push(package);
    model
}

fn scratch_crate_files() -> Vec<(std::path::PathBuf, String)> {
    let generated = rex_backend_rust::generate(&validation_model()).expect("generate");
    let models_rs = &generated["models.rs"];

    let manifest = env!("CARGO_MANIFEST_DIR");
    let runtime_path = Path::new(manifest)
        .join("../../crates/rex-runtime")
        .canonicalize()
        .expect("rex-runtime path");

    let lib_rs = format!(
        "{models_rs}\n{}",
        include_str!("scratch/validation_tests.rs")
    );

    vec![
        (
            Path::new("Cargo.toml").to_path_buf(),
            format!(
                "[package]\n\
                 name = \"rex-validation-test\"\n\
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

/// Runs `cargo test` inside the validation scratch crate: the generated
/// `validate` methods must compile and behave (violations on bad values,
/// empty on valid).
fn run_validation_scratch_crate() {
    let cargo_available = std::process::Command::new("cargo")
        .arg("--version")
        .output()
        .map(|out| out.status.success())
        .unwrap_or(false);
    if !cargo_available {
        eprintln!("skipping validation scratch-crate test: cargo not on PATH");
        return;
    }

    let manifest = env!("CARGO_MANIFEST_DIR");
    let scratch = Path::new(manifest).join("../../target/scratch/rex-validation-test");
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
        "validation scratch crate cargo test failed:\n--- stdout ---\n{stdout}\n--- stderr ---\n{stderr}"
    );
    eprintln!("{stdout}");
}

#[test]
fn generated_validate_reports_violations_and_accepts_valid_objects() {
    run_validation_scratch_crate();
}
