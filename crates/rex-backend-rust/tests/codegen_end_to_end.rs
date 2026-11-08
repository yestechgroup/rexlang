//! End-to-end proof for the Rust backend: build the resolved IR for the
//! flagship example programmatically, generate Rust code, compile it as a
//! scratch crate, and run behavioral tests in that crate (arena semantics,
//! opposite maintenance, navigation, enums, datatypes, defaults). A second
//! scenario exercises vocabulary codegen with the conformance currency model.
//!
//! The scratch crate additionally embeds every canonical `examples/*.mox` as
//! a namespaced module (`pub mod library; pub mod ecommerce; …`, generated
//! through `rex_driver::compile_str` — see `scratch/examples_flow.rs`) with
//! the behavioral assertions from `tests/scratch/example_assertions.rs`, so
//! the same binary proves both the hand-built IR surface and the whole
//! examples suite. The hand-built library IR is deliberately kept: it is the
//! Tier-1-enriched sibling of the source flagship (rust-bodied `getBook`,
//! `totalPages`/`anyLongBook` ops, Date create/convert bodies) and pins the
//! conformance instance golden.

use std::path::Path;

#[path = "scratch/examples_flow.rs"]
mod examples_flow;

use rex_ir::{
    ClassDef, DatatypeDef, DefaultValue, EnumDef, EnumLiteral, Feature, FeatureKind, Model,
    Multiplicity, Operation, OperationParam, Package, PrimitiveType, TypeRef, VocabularyDef,
    VocabularyEntry,
};

const PKG: &str = "nz.example.library";

fn class_ref(name: &str) -> TypeRef {
    TypeRef::Class {
        package: PKG.to_string(),
        name: name.to_string(),
    }
}

/// The verbatim rust body lowered from the source (span-sliced between the
/// braces — note the leading/trailing spaces the source carries). The
/// generated `Resource.books` is a `SlotMap`, so iteration yields
/// `(BookId, &Book)` pairs; the body unwraps with `expect` because the
/// fixed signature returns `BookId`, not `Option<BookId>`.
const GET_BOOK_BODY: &str = " res.books.iter().find_map(|(id, b)| (b.title == title).then_some(id)).expect(\"no book with the given title\") ";

/// The IR equivalent of `examples/library.mox` after driver lowering, plus the
/// Tier 1 bodies that model gains in the body-bearing scenario: attributes
/// without multiplicity are REQUIRED, `refers`/`contains` are MANY,
/// `container` is forced OPTIONAL, `getBook` lowers into `Library.operations`
/// with a verbatim rust body, and the Date datatype carries create/convert
/// bodies. Derived features stay in the IR but are not materialized.
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
            .bind("java", "java.time.LocalDate")
            .with_body("create", "rust", " Date(it) ")
            .with_body("convert", "rust", " self.0.clone() "),
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
    // Tier 1: `op Book getBook(String title) { rust { ... } }` lowers into
    // `Library.operations` with the body carried verbatim.
    package.classes[0].operations.push(
        Operation::new(
            "getBook",
            class_ref("Book"),
            vec![OperationParam {
                name: "title".to_string(),
                type_: TypeRef::Primitive(PrimitiveType::String),
                multiplicity: None,
            }],
        )
        .with_body("rust", GET_BOOK_BODY),
    );
    // Tier 2: neutral `expr` bodies lower into typed methods — the derived
    // `citation` accessor and two collection-algebra operations.
    package.classes[0].operations.push(Operation::new(
        "totalPages",
        TypeRef::Primitive(PrimitiveType::Int),
        vec![],
    ));
    package.classes[0].operations[1].bodies.insert(
        "expr".to_string(),
        "books.map(b => b.pages).sum()".to_string(),
    );
    package.classes[0].operations.push(Operation::new(
        "anyLongBook",
        TypeRef::Primitive(PrimitiveType::Boolean),
        vec![],
    ));
    package.classes[0].operations[2].bodies.insert(
        "expr".to_string(),
        "books.any(b => b.pages > 300)".to_string(),
    );
    package.classes[0].operations.push(
        Operation::new(
            "findBook",
            class_ref("Book"),
            vec![OperationParam {
                name: "title".to_string(),
                type_: TypeRef::Primitive(PrimitiveType::String),
                multiplicity: None,
            }],
        )
        .with_body("expr", "books.first(b => b.title == title)"),
    );
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
            .derived()
            .with_body("expr", "if pages > 400 { title } else { \"short read\" }"),
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

    // The hand-built library IR stays at the crate root; the five canonical
    // examples land as namespaced modules, and both assertion suites
    // (hand-built + examples) are appended.
    let lib_rs = format!(
        "{models_rs}\n{}\n{}\n{}",
        examples_flow::example_modules(),
        include_str!("scratch/scratch_tests.rs"),
        include_str!("scratch/example_assertions.rs"),
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
        command.env("REX_EXAMPLES_DIR", examples_flow::examples_dir());
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
        description: None,
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

// --- sigil-lowered package functions (wire contract rule 15) ----------------

const SIGIL_FUNCS_MOX: &str = "tests/conformance/sigil/funcs.mox";
const SIGIL_FUNCS_ROSETTA: &str = "tests/conformance/sigil/funcs.rosetta";

/// The conformance sigil-func pair compiled through the full `import sigil`
/// pipeline: the mox names the rosetta file, the driver lowers its
/// namespaces to synthetic packages carrying `Package::functions`.
fn sigil_funcs_model() -> Model {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../")
        .canonicalize()
        .expect("workspace root");
    let mox = std::fs::read_to_string(root.join(SIGIL_FUNCS_MOX)).expect("read funcs.mox");
    let rosetta =
        std::fs::read_to_string(root.join(SIGIL_FUNCS_ROSETTA)).expect("read funcs.rosetta");
    let compilation = rex_driver::compile_str(
        SIGIL_FUNCS_MOX,
        &mox,
        &rex_driver::DomainImports {
            schemas: rex_driver::SchemaImports::new(),
            sigil: rex_driver::SigilImports::new().provide(
                SIGIL_FUNCS_MOX,
                "funcs.rosetta",
                rosetta,
            ),
        },
    );
    assert!(
        compilation.diagnostics.is_empty() && compilation.sigil_diagnostics.is_empty(),
        "the sigil funcs pair must compile clean: {:?} {:?}",
        compilation.diagnostics,
        compilation.sigil_diagnostics
    );
    compilation.model.expect("funcs.mox lowered to the Core IR")
}

/// Sigil-lowered functions generate free functions named after the
/// declarations, with the backend's multiplicity conventions in their
/// signatures. A function with a checked `rune` body (RUNE-BODIES stage 2)
/// emits real straight-line Rust; a function without one — the library
/// function and the statement-less `tradeIds` — still emits a `todo!()`
/// stub. The verbatim `rosetta` body text is never emitted as code (wire
/// contract rule 15).
#[test]
fn sigil_functions_emit_bodies_or_stubs_by_body_target() {
    let generated = rex_backend_rust::generate(&sigil_funcs_model()).expect("generate");
    let code = &generated["models.rs"];
    assert!(
        code.contains("pub fn any_active(ids: Vec<String>) -> bool"),
        "library-function stub with the 0..* convention:\n{code}"
    );
    assert!(
        code.contains("pub fn calc_rate(notional: f64, leg_ids: Vec<String>) -> f64"),
        "func with mixed multiplicities:\n{code}"
    );
    assert!(
        code.contains("pub fn trade_ids(leg_id: String) -> Vec<String>"),
        "func stub with a (0..*) output — the return multiplicity lifts the\n\
         return type:\n{code}"
    );
    // calcRate carries the checked `rune` body: the guard asserts and the
    // step assigns the output local, which the function returns.
    assert!(
        code.contains("assert!((notional > 0f64), \"guard 'PositiveNotional' failed\");"),
        "the pre guard emits as an assert naming the guard:\n{code}"
    );
    assert!(
        code.contains("let mut rate: f64 = 0.0;"),
        "the scalar output local is introduced with the declared default\n\
         shape:\n{code}"
    );
    assert!(
        code.contains("    rate = notional;\n    rate\n}"),
        "the step assigns and the function returns the output local:\n{code}"
    );
    assert_eq!(
        code.matches("\n    todo!()").count(),
        2,
        "one stub body per body-less function — `anyActive` and `tradeIds` (the\n\
          header note's todo!() mention doesn't count):\n{code}"
    );
    // The verbatim Rune text stays in the IR, never in generated code.
    assert!(!code.contains("set rate:"), "{code}");
    assert!(!code.contains("library function anyActive"), "{code}");
    // The header documents the function contract (only when functions exist).
    assert!(code.contains("sigil-lowered package functions"), "{code}");
}

/// A function stub generation-time model: one package carrying a single
/// signature-only function with the given name.
fn one_function_model(package: &str, function: &str) -> Model {
    let mut model = Model::new();
    let mut package = Package::new(package);
    package.functions.push(Operation::new(
        function,
        TypeRef::Primitive(PrimitiveType::Boolean),
        vec![],
    ));
    model.packages.push(package);
    model
}

/// `fooBar` and `foo_bar` (or two same-named functions in different
/// packages) collide once snake_cased into the flattened module: generation
/// bails naming both declarations, like the duplicate-class precedent.
#[test]
fn emit_functions_bails_on_duplicate_snake_case_names() {
    let mut model = one_function_model("sigil::example", "fooBar");
    model.packages[0].functions.push(Operation::new(
        "foo_bar",
        TypeRef::Primitive(PrimitiveType::Boolean),
        vec![],
    ));
    let error = rex_backend_rust::generate(&model).expect_err("duplicate snake_case names");
    let message = error.to_string();
    assert!(
        message.contains("duplicate function name 'foo_bar'"),
        "{message}"
    );
    assert!(message.contains("'fooBar'"), "{message}");
    assert!(message.contains("'foo_bar'"), "{message}");
    assert!(
        message.contains("flattens packages into one module"),
        "{message}"
    );
}

#[test]
fn emit_functions_bails_on_the_same_function_name_in_two_packages() {
    let mut model = one_function_model("sigil::a", "calcRate");
    model.packages.push(
        one_function_model("sigil::b", "calcRate")
            .packages
            .remove(0),
    );
    let error = rex_backend_rust::generate(&model).expect_err("same name in two packages");
    let message = error.to_string();
    assert!(
        message.contains("duplicate function name 'calc_rate'"),
        "{message}"
    );
    assert!(
        message.contains("flattens packages into one module"),
        "{message}"
    );
}

/// The conformance funcs model's `expr`-body op on `Desk` lowers its bare
/// `calcRate(...)` call to the emitted free stub: the generated method's
/// body is the stub call with the backend's literal conventions, and the
/// scratch crate compiles it with the stub-panic assertions appended.
#[test]
fn the_expr_body_op_lowering_calls_the_emitted_function_stub() {
    let generated = rex_backend_rust::generate(&sigil_funcs_model()).expect("generate");
    let code = &generated["models.rs"];
    assert!(
        code.contains("pub fn double_rate(&self, res: &Resource, u: UnderlierId) -> f64"),
        "the Desk op lowered from its expr body:\n{code}"
    );
    assert!(
        code.contains("calc_rate(2f64, vec![\"leg-1\".to_string()])"),
        "the bare call lowers to the exact stub name:\n{code}"
    );
    // The scratch crate (with the appended stub-panic assertions) must
    // compile and run against this generated code.
    run_sigil_funcs_scratch_crate();
}

/// Byte-identity at the string level: a function-less model emits no
/// free-function section at all — no stub doc comments, no header bullet
/// (the golden artifacts already pin the output-level identity).
#[test]
fn function_less_models_emit_no_function_section() {
    let mut model = Model::new();
    let mut package = Package::new("plain.example");
    package.classes.push(ClassDef::new("Thing", vec![], vec![]));
    model.packages.push(package);
    let code = &rex_backend_rust::generate(&model).expect("generate")["models.rs"];
    assert!(!code.contains("Package function"), "{code}");
    assert!(!code.contains("sigil-lowered package functions"), "{code}");
}

/// A function with a non-singleton return cardinality generates the lifted
/// return shape: `0..*` → `Vec<_>` (the `fn_param_type` convention, return
/// position), `0..1` → `Option<_>` (the feature-field convention, matching
/// rex-expr's call typing); the single-valued convention stays the plain
/// type.
#[test]
fn a_function_return_multiplicity_lifts_the_generated_return_type() {
    let mut model = Model::new();
    let mut package = Package::new("sigil::example");
    let mut many = Operation::new(
        "legRates",
        TypeRef::Primitive(PrimitiveType::Double),
        vec![],
    );
    many.return_multiplicity = Some(Multiplicity::MANY);
    package.functions.push(many);
    let mut optional = Operation::new(
        "maybeRate",
        TypeRef::Primitive(PrimitiveType::Double),
        vec![],
    );
    optional.return_multiplicity = Some(Multiplicity::OPTIONAL);
    package.functions.push(optional);
    package.functions.push(Operation::new(
        "scalarRate",
        TypeRef::Primitive(PrimitiveType::Double),
        vec![],
    ));
    model.packages.push(package);

    let code = &rex_backend_rust::generate(&model).expect("generate")["models.rs"];
    assert!(
        code.contains("pub fn leg_rates() -> Vec<f64>"),
        "the (0..*) return lifts to Vec:\n{code}"
    );
    assert!(
        code.contains("pub fn maybe_rate() -> Option<f64>"),
        "the (0..1) return lifts to Option:\n{code}"
    );
    assert!(
        code.contains("pub fn scalar_rate() -> f64"),
        "the (1..1) return stays the plain type:\n{code}"
    );
}

fn sigil_funcs_scratch_crate_files() -> Vec<(std::path::PathBuf, String)> {
    let generated = rex_backend_rust::generate(&sigil_funcs_model()).expect("generate");
    let models_rs = &generated["models.rs"];

    let manifest = env!("CARGO_MANIFEST_DIR");
    let runtime_path = Path::new(manifest)
        .join("../../crates/rex-runtime")
        .canonicalize()
        .expect("rex-runtime path");

    let lib_rs = format!(
        "{models_rs}\n{}",
        include_str!("scratch/sigil_func_tests.rs")
    );

    vec![
        (
            Path::new("Cargo.toml").to_path_buf(),
            format!(
                "[package]\n\
                 name = \"rex-sigil-func-test\"\n\
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

/// Runs `cargo test` inside the sigil-func scratch crate: the stubs compile
/// with their declared signatures and panic as `todo!()` when called.
fn run_sigil_funcs_scratch_crate() {
    let cargo_available = std::process::Command::new("cargo")
        .arg("--version")
        .output()
        .map(|out| out.status.success())
        .unwrap_or(false);
    if !cargo_available {
        eprintln!("skipping sigil-func scratch-crate test: cargo not on PATH");
        return;
    }

    let manifest = env!("CARGO_MANIFEST_DIR");
    let scratch = Path::new(manifest).join("../../target/scratch/rex-sigil-func-test");
    std::fs::create_dir_all(&scratch).expect("create scratch root");
    for (relative, contents) in sigil_funcs_scratch_crate_files() {
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
        "sigil-func scratch crate cargo test failed:\n--- stdout ---\n{stdout}\n--- stderr ---\n{stderr}"
    );
    eprintln!("{stdout}");
}

#[test]
fn generated_sigil_function_stubs_compile_and_panic_as_todo() {
    run_sigil_funcs_scratch_crate();
}

// ------------------------------------------------------------------
// RUNE-BODIES stage 2: a checked `rune` body emits straight-line Rust
// (docs/RUNE-BODIES.md per-backend lowering order, item 1). The fixtures
// below hand-build IR models carrying the canonical `"rune"` payload
// exactly as the driver stores it (rex-expr's `RuneBody`, serde_json'd),
// mirroring `tests/conformance/sigil/funcs.mox.json`.
// ------------------------------------------------------------------

/// Serializes a hand-built `RuneBody` into the `"rune"` payload shape the
/// driver stores (canonical serde_json, key order = declaration order).
fn rune_payload(body: &rex_expr::body::RuneBody) -> String {
    serde_json::to_string(body).expect("the rune body serializes")
}

fn rune_function(
    name: &str,
    ret: TypeRef,
    ret_multiplicity: Option<Multiplicity>,
    params: Vec<OperationParam>,
    body: &rex_expr::body::RuneBody,
) -> Operation {
    let mut operation = Operation::new(name, ret, params);
    operation.return_multiplicity = ret_multiplicity;
    operation
        .bodies
        .insert("rune".to_string(), rune_payload(body));
    operation
}

fn fn_param(name: &str, type_: TypeRef, multiplicity: Option<Multiplicity>) -> OperationParam {
    OperationParam {
        name: name.to_string(),
        type_,
        multiplicity,
    }
}

fn class_qname(package: &str, name: &str) -> TypeRef {
    TypeRef::Class {
        package: package.to_string(),
        name: name.to_string(),
    }
}

/// A package with a `Quote` class (required `symbol` string, required
/// `notional` double, many `legs` string attributes) plus the given
/// package functions — the pricing-shaped fixture for body emission.
fn quote_model(functions: Vec<Operation>) -> Model {
    let mut model = Model::new();
    let mut package = Package::new("sigil::example");
    package.classes.push(ClassDef::new(
        "Quote",
        vec![],
        vec![
            Feature::new(
                "symbol",
                FeatureKind::Attribute,
                TypeRef::Primitive(PrimitiveType::String),
                Multiplicity::REQUIRED,
            ),
            Feature::new(
                "notional",
                FeatureKind::Attribute,
                TypeRef::Primitive(PrimitiveType::Double),
                Multiplicity::REQUIRED,
            ),
            Feature::new(
                "legs",
                FeatureKind::Attribute,
                TypeRef::Primitive(PrimitiveType::String),
                Multiplicity::MANY,
            ),
        ],
    ));
    package.functions = functions;
    model.packages.push(package);
    model
}

/// A corrupt `"rune"` payload is a generate-time ERROR naming the function —
/// the crate's bail convention (the duplicate-name and expr-on-function
/// precedents); a body the driver deferred never carries the target at all,
/// so a present-but-undecodable payload is artifact corruption, not a
/// deferral.
#[test]
fn a_corrupt_rune_payload_is_a_generate_error_naming_the_function() {
    let mut model = quote_model(vec![Operation::new(
        "broken",
        TypeRef::Primitive(PrimitiveType::Double),
        vec![],
    )]);
    model.packages[0].functions[0]
        .bodies
        .insert("rune".to_string(), "{not json".to_string());
    let error = rex_backend_rust::generate(&model).expect_err("corrupt rune payload");
    let message = error.to_string();
    assert!(message.contains("function 'broken'"), "{message}");
    assert!(message.contains("\"rune\""), "{message}");
}

/// `(0..*)` output + `add` steps: the output local starts at the empty vec
/// and each step pushes, in declaration order; the function returns it.
#[test]
fn a_many_output_accumulates_add_steps_into_a_vec() {
    let mut body = rex_expr::body::RuneBody::default();
    body.steps = vec![
        rex_expr::body::BodyStep::Add {
            root: "ids".to_string(),
            path: vec![],
            expr: "\"a\"".to_string(),
        },
        rex_expr::body::BodyStep::Add {
            root: "ids".to_string(),
            path: vec![],
            expr: "\"b\"".to_string(),
        },
    ];
    let model = quote_model(vec![rune_function(
        "legRates",
        TypeRef::Primitive(PrimitiveType::String),
        Some(Multiplicity::MANY),
        vec![],
        &body,
    )]);
    let code = &rex_backend_rust::generate(&model).expect("generate")["models.rs"];
    let body_text = code
        .split("pub fn leg_rates")
        .nth(1)
        .expect("the generated function")
        .split("pub fn ")
        .next()
        .expect("the function body block");
    assert!(
        body_text.contains("let mut ids: Vec<String> = Vec::new();"),
        "the Vec output local starts empty:\n{body_text}"
    );
    assert!(
        body_text.contains("ids.push(\"a\".to_string());")
            && body_text.contains("ids.push(\"b\".to_string());"),
        "each add step pushes in order:\n{body_text}"
    );
    assert!(
        body_text.contains("\n    ids\n}"),
        "returns the vec:\n{body_text}"
    );
}

/// `(0..1)` output: the output local starts at `None` and a `set` assigns it
/// directly — the checker requires the assigned expression to carry the
/// `Option` lift itself (the slot type keeps its lift), so no implicit
/// `Some` wrap is invented.
#[test]
fn an_optional_output_local_starts_at_none_and_set_assigns_directly() {
    let mut body = rex_expr::body::RuneBody::default();
    body.steps = vec![rex_expr::body::BodyStep::Set {
        root: "head".to_string(),
        path: vec![],
        expr: "legIds.first()".to_string(),
    }];
    let model = quote_model(vec![rune_function(
        "firstLeg",
        TypeRef::Primitive(PrimitiveType::String),
        Some(Multiplicity::OPTIONAL),
        vec![fn_param(
            "legIds",
            TypeRef::Primitive(PrimitiveType::String),
            Some(Multiplicity::MANY),
        )],
        &body,
    )]);
    let code = &rex_backend_rust::generate(&model).expect("generate")["models.rs"];
    let body_text = code
        .split("pub fn first_leg")
        .nth(1)
        .expect("the generated function")
        .split("pub fn ")
        .next()
        .expect("the function body block");
    assert!(
        code.contains("pub fn first_leg(leg_ids: Vec<String>) -> Option<String>"),
        "the 0..1 return lifts to Option:\n{code}"
    );
    assert!(
        body_text.contains("let mut head: Option<String> = None;"),
        "the Option output local starts at None:\n{body_text}"
    );
    assert!(
        body_text.contains("head = "),
        "the set assigns the Option-typed expression directly:\n{body_text}"
    );
}

/// Unnamed guards get the phase-derived fallback label (the checker's slot
/// label), post guards emit after the steps but before the return, aliases
/// become sequential `let`s, and repeated `set`s overwrite (last wins).
#[test]
fn guards_bindings_and_repeated_sets_emit_in_declaration_order() {
    let mut body = rex_expr::body::RuneBody::default();
    body.guards = vec![
        rex_expr::body::BodyGuard {
            expr: "notional > 0.0".to_string(),
            name: None,
            phase: rex_expr::body::GuardPhase::Pre,
        },
        rex_expr::body::BodyGuard {
            expr: "rate > 0.0".to_string(),
            name: Some("Filled".to_string()),
            phase: rex_expr::body::GuardPhase::Post,
        },
    ];
    body.bindings = vec![
        rex_expr::body::BodyBinding {
            name: "base".to_string(),
            expr: "notional".to_string(),
        },
        rex_expr::body::BodyBinding {
            name: "doubled".to_string(),
            expr: "base * 2.0".to_string(),
        },
    ];
    body.steps = vec![
        rex_expr::body::BodyStep::Set {
            root: "rate".to_string(),
            path: vec![],
            expr: "notional".to_string(),
        },
        rex_expr::body::BodyStep::Set {
            root: "rate".to_string(),
            path: vec![],
            expr: "doubled".to_string(),
        },
    ];
    let model = quote_model(vec![rune_function(
        "scaled",
        TypeRef::Primitive(PrimitiveType::Double),
        None,
        vec![fn_param(
            "notional",
            TypeRef::Primitive(PrimitiveType::Double),
            None,
        )],
        &body,
    )]);
    let code = &rex_backend_rust::generate(&model).expect("generate")["models.rs"];
    let expected = "\
pub fn scaled(notional: f64) -> f64 {
    assert!((notional > 0f64), \"guard 'unnamed pre guard' failed\");
    let base = notional;
    let doubled = (base * 2f64);
    let mut rate: f64 = 0.0;
    rate = notional;
    rate = doubled;
    assert!((rate > 0f64), \"guard 'Filled' failed\");
    rate
}";
    assert!(
        code.contains(expected),
        "the exact emission shape (guards → assert, aliases → let, steps →\n\
         assign, post guard after the steps, last set wins):\n{code}"
    );
}

/// The dispatch head's synthetic pre guard lowers the bare enum-literal name
/// to the generated enum path (`RateType::FIXED`).
#[test]
fn the_dispatch_head_literal_lowers_to_the_enum_path() {
    let mut model = quote_model(vec![]);
    let package = &mut model.packages[0];
    package.enums.push(EnumDef::new(
        "RateType",
        vec![
            EnumLiteral::new("FIXED", None, 0),
            EnumLiteral::new("FLOATING", None, 1),
        ],
    ));
    let mut body = rex_expr::body::RuneBody::default();
    body.guards = vec![rex_expr::body::BodyGuard {
        expr: "status == FIXED".to_string(),
        name: Some("FIXED".to_string()),
        phase: rex_expr::body::GuardPhase::Pre,
    }];
    body.steps = vec![rex_expr::body::BodyStep::Set {
        root: "rate".to_string(),
        path: vec![],
        expr: "1.0".to_string(),
    }];
    package.functions.push(rune_function(
        "fixedRate",
        TypeRef::Primitive(PrimitiveType::Double),
        None,
        vec![fn_param(
            "status",
            TypeRef::Enum {
                package: "sigil::example".to_string(),
                name: "RateType".to_string(),
            },
            None,
        )],
        &body,
    ));
    let code = &rex_backend_rust::generate(&model).expect("generate")["models.rs"];
    assert!(
        code.contains("assert!((status == RateType::FIXED), \"guard 'FIXED' failed\");"),
        "the dispatch literal lowers to the generated enum path:\n{code}"
    );
}

/// A step path navigates features from the output root through the arena:
/// a class-typed output makes the function take `res`, the output local is
/// the arena's default-initialized object, attribute `set`s assign the
/// resolved field, attribute `add`s push, and the function returns the id.
#[test]
fn a_step_path_mutates_through_the_arena_and_the_function_takes_res() {
    let mut body = rex_expr::body::RuneBody::default();
    body.guards = vec![rex_expr::body::BodyGuard {
        expr: "legIds.size() > 0".to_string(),
        name: None,
        phase: rex_expr::body::GuardPhase::Pre,
    }];
    body.steps = vec![
        rex_expr::body::BodyStep::Set {
            root: "q".to_string(),
            path: vec!["symbol".to_string()],
            expr: "\"AAPL\"".to_string(),
        },
        rex_expr::body::BodyStep::Set {
            root: "q".to_string(),
            path: vec!["notional".to_string()],
            expr: "10.0".to_string(),
        },
        rex_expr::body::BodyStep::Add {
            root: "q".to_string(),
            path: vec!["legs".to_string()],
            expr: "\"leg-1\"".to_string(),
        },
    ];
    let model = quote_model(vec![rune_function(
        "makeQuote",
        class_qname("sigil::example", "Quote"),
        None,
        vec![fn_param(
            "legIds",
            TypeRef::Primitive(PrimitiveType::String),
            Some(Multiplicity::MANY),
        )],
        &body,
    )]);
    let code = &rex_backend_rust::generate(&model).expect("generate")["models.rs"];
    let expected = "\
pub fn make_quote(res: &Resource, leg_ids: Vec<String>) -> QuoteId {
    assert!((leg_ids.clone().len() as i32 > 0i32), \"guard 'unnamed pre guard' failed\");
    let mut q = res.new_quote();
    res.quote_mut(q).expect(\"dangling `Quote` id\").symbol = \"AAPL\".to_string();
    res.quote_mut(q).expect(\"dangling `Quote` id\").notional = 10f64;
    res.quote_mut(q).expect(\"dangling `Quote` id\").legs.push(\"leg-1\".to_string());
    q
}";
    assert!(
        code.contains(expected),
        "arena-path emission shape:\n{code}"
    );
}

/// Navigation inside a slot expression (a guard reading a class-typed
/// input's feature) makes the function take `res` and lowers through the
/// arena lookup, exactly like the `expr`-body lowering.
#[test]
fn feature_access_in_slot_expressions_takes_res_and_reads_through_the_arena() {
    let mut body = rex_expr::body::RuneBody::default();
    body.guards = vec![rex_expr::body::BodyGuard {
        expr: "q.notional > 0.0".to_string(),
        name: Some("Positive".to_string()),
        phase: rex_expr::body::GuardPhase::Pre,
    }];
    body.steps = vec![rex_expr::body::BodyStep::Set {
        root: "price".to_string(),
        path: vec![],
        expr: "q.notional".to_string(),
    }];
    let model = quote_model(vec![rune_function(
        "priceOf",
        TypeRef::Primitive(PrimitiveType::Double),
        None,
        vec![fn_param("q", class_qname("sigil::example", "Quote"), None)],
        &body,
    )]);
    let code = &rex_backend_rust::generate(&model).expect("generate")["models.rs"];
    assert!(
        code.contains("pub fn price_of(res: &Resource, q: QuoteId) -> f64"),
        "navigation makes the function take res:\n{code}"
    );
    let lookup = "res.quote(q).map(|o| o.notional).expect(\"dangling `Quote` id\")";
    assert!(
        code.matches(lookup).count() == 2,
        "the guard and the step both read the feature through the arena:\n{code}"
    );
}

/// An `expr`-body op calling a res-taking rune-bodied function lowers the
/// bare call with the arena as the first argument, so the generated code
/// compiles and the call surface stays closed but callable (issue #63 is
/// the typing side, not this text shape).
#[test]
fn an_expr_call_to_a_res_taking_function_passes_the_arena() {
    let mut body = rex_expr::body::RuneBody::default();
    body.steps = vec![rex_expr::body::BodyStep::Set {
        root: "q".to_string(),
        path: vec!["symbol".to_string()],
        expr: "\"AAPL\"".to_string(),
    }];
    let make_quote = rune_function(
        "makeQuote",
        class_qname("sigil::example", "Quote"),
        None,
        vec![],
        &body,
    );
    let op = Operation::new("newQuote", class_qname("sigil::example", "Quote"), vec![])
        .with_body("expr", "makeQuote()");
    let mut model = quote_model(vec![make_quote]);
    model.packages[0].classes[0].operations.push(op.clone());
    let code = &rex_backend_rust::generate(&model).expect("generate")["models.rs"];
    assert!(
        code.contains("make_quote(res)"),
        "the bare call passes the arena to the res-taking function:\n{code}"
    );
    // A non-res function keeps the bare argument list (the pinned stub-call
    // convention).
    let plain = quote_model(vec![rune_function(
        "answer",
        TypeRef::Primitive(PrimitiveType::Int),
        None,
        vec![],
        &{
            let mut body = rex_expr::body::RuneBody::default();
            body.steps = vec![rex_expr::body::BodyStep::Set {
                root: "n".to_string(),
                path: vec![],
                expr: "42".to_string(),
            }];
            body
        },
    )]);
    let code = &rex_backend_rust::generate(&plain).expect("generate")["models.rs"];
    assert!(
        !code.contains("answer(res"),
        "an arena-free body takes no res parameter:\n{code}"
    );
    assert!(
        code.contains("pub fn answer() -> i32"),
        "signature unchanged:\n{code}"
    );
    let _ = op;
}
