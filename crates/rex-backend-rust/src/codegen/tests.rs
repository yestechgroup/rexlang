//! The codegen unit tests, pinned to byte-exact emission.

use super::*;

use rex_ir::{
    DefaultValue, EnumLiteral, Feature, FeatureKind, Multiplicity, Operation, OperationParam,
    Package, PrimitiveType, VocabularyDef, VocabularyEntry,
};

const PKG: &str = "nz.example.payments";

fn vocabulary_ref() -> TypeRef {
    TypeRef::Vocabulary {
        package: PKG.to_string(),
        name: "Currency".to_string(),
    }
}

fn entry(key: &str, symbol: &str, minor_units: i64) -> VocabularyEntry {
    VocabularyEntry {
        key: key.to_string(),
        facets: BTreeMap::from([
            (
                "symbol".to_string(),
                DefaultValue::String(symbol.to_string()),
            ),
            ("minorUnits".to_string(), DefaultValue::Int(minor_units)),
        ]),
    }
}

/// The IR equivalent of `tests/conformance/models/currency.mox`.
fn currency_model() -> Model {
    let mut model = Model::new();
    let mut package = Package::new(PKG);
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
            entry("USD", "$", 2),
            entry("EUR", "€", 2),
            entry("JPY", "¥", 0),
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
                vocabulary_ref(),
                Multiplicity::REQUIRED,
            ),
            Feature::new(
                "backup",
                FeatureKind::Attribute,
                vocabulary_ref(),
                Multiplicity::OPTIONAL,
            ),
            Feature::new(
                "watchlist",
                FeatureKind::Attribute,
                vocabulary_ref(),
                Multiplicity::MANY,
            ),
        ],
    ));
    model.packages.push(package);
    model
}

fn generate_currency() -> String {
    generate(&currency_model())
        .expect("generate")
        .remove("models.rs")
        .expect("models.rs")
}

/// A model with a required, an optional, and a many-valued date
/// attribute (issue #9): the shape of the `date` primitive's lowering.
fn date_model() -> Model {
    let mut model = Model::new();
    let mut package = Package::new(PKG);
    package.classes.push(ClassDef::new(
        "Loan",
        vec![],
        vec![
            Feature::new(
                "effectiveDate",
                FeatureKind::Attribute,
                TypeRef::Primitive(PrimitiveType::Date),
                Multiplicity::REQUIRED,
            ),
            Feature::new(
                "maturityDate",
                FeatureKind::Attribute,
                TypeRef::Primitive(PrimitiveType::Date),
                Multiplicity::OPTIONAL,
            ),
            Feature::new(
                "renewals",
                FeatureKind::Attribute,
                TypeRef::Primitive(PrimitiveType::Date),
                Multiplicity::MANY,
            ),
        ],
    ));
    model.packages.push(package);
    model
}

fn generate_dates() -> String {
    generate(&date_model())
        .expect("generate")
        .remove("models.rs")
        .expect("models.rs")
}

#[test]
fn date_attributes_map_to_the_runtime_date_type() {
    let code = generate_dates();
    // Required date attribute: a `rex_runtime::Date` field defaulting to
    // the epoch (dates take no declared defaults in this milestone).
    assert!(
        code.contains("pub effective_date: rex_runtime::Date,"),
        "{code}"
    );
    assert!(
        code.contains("effective_date: rex_runtime::Date::EPOCH,"),
        "{code}"
    );
    // Optional and many-valued follow the usual shapes.
    assert!(
        code.contains("pub maturity_date: Option<rex_runtime::Date>,"),
        "{code}"
    );
    assert!(
        code.contains("pub renewals: Vec<rex_runtime::Date>,"),
        "{code}"
    );
}

#[test]
fn date_attributes_serialize_iso_strings_and_load_strictly() {
    let code = generate_dates();
    // Save: ISO-8601 strings (required, optional, and many shapes).
    assert!(
        code.contains("serde_json::Value::String(obj.effective_date.to_string())"),
        "required date save:\n{code}"
    );
    assert!(
        code.contains("serde_json::Value::String((*v).to_string())"),
        "optional/many date save:\n{code}"
    );
    // Load: the strict ISO parse; a non-ISO string is InvalidInstance.
    assert!(
        code.contains("expect_date(value, \"Loan.effectiveDate\")?"),
        "required date load:\n{code}"
    );
    assert!(
        code.contains("expect_date(element, \"Loan.renewals\")?"),
        "{code}"
    );
    assert!(
        code.contains("must be an ISO-8601 date (YYYY-MM-DD)"),
        "strict loader helper:\n{code}"
    );
}

#[test]
fn date_setters_take_runtime_dates() {
    let code = generate_dates();
    assert!(
        code.contains("pub fn set_effective_date(&mut self, effective_date: rex_runtime::Date)"),
        "{code}"
    );
    assert!(
        code.contains("pub fn set_maturity_date(&mut self, maturity_date: rex_runtime::Date)"),
        "{code}"
    );
}

#[test]
fn vocabulary_enum_with_key_and_facet_accessors() {
    let code = generate_currency();
    assert!(
        code.contains("#[repr(u8)]\npub enum Currency {\n    USD,\n    EUR,\n    JPY,\n}"),
        "enum shape:\n{code}"
    );
    assert!(code.contains("#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]"));
    assert!(
        code.contains("Vocabulary Currency")
            && code.contains("iso:4217")
            && code.contains("2024-01-01")
            && code.contains("3 entries"),
        "doc comment must name vocabulary, source, version, and entry count:\n{code}"
    );
    assert!(code.contains("pub fn key(self) -> &'static str"));
    assert!(code.contains("Self::USD => \"USD\","));
    assert!(code.contains("pub fn symbol(self) -> &'static str"));
    assert!(code.contains("Self::JPY => \"¥\""));
    assert!(code.contains("pub fn minor_units(self) -> i64"));
    assert!(code.contains("Self::JPY => 0,"));
}

#[test]
fn vocabulary_try_from_display_and_default() {
    let code = generate_currency();
    assert!(code.contains("impl TryFrom<&str> for Currency {"));
    assert!(
        code.contains("RexError::InvalidInstance") && code.contains("unknown Currency"),
        "lookup failure must be an InvalidInstance naming the vocabulary:\n{code}"
    );
    assert!(code.contains("impl std::fmt::Display for Currency {"));
    assert!(code.contains("impl Default for Currency {"));
    assert!(
        code.contains("        Self::USD\n"),
        "Default is the first entry:\n{code}"
    );
}

#[test]
fn vocabulary_attributes_follow_enum_field_rules() {
    let code = generate_currency();
    assert!(
        code.contains("pub currency: Currency,"),
        "required → plain:\n{code}"
    );
    assert!(
        code.contains("pub backup: Option<Currency>,"),
        "optional → Option:\n{code}"
    );
    assert!(
        code.contains("pub watchlist: Vec<Currency>,"),
        "many → Vec:\n{code}"
    );
    assert!(code.contains("pub fn set_currency(&mut self, currency: Currency)"));
    assert!(code.contains("pub fn set_backup(&mut self, backup: Currency)"));
    assert!(
        code.contains("self.backup = Some(backup);"),
        "optional scalar setters wrap in Some:\n{code}"
    );
    assert!(code.contains("pub fn set_watchlist(&mut self, watchlist: Vec<Currency>)"));
    assert!(
        code.contains("currency: Currency::USD,"),
        "required vocabulary attribute defaults to the first entry:\n{code}"
    );
    assert!(code.contains("backup: None,"));
    assert!(code.contains("watchlist: Vec::new(),"));
}

#[test]
fn declared_vocabulary_default_resolves_to_a_variant() {
    let mut model = currency_model();
    model.packages[0].classes[0].features[1] = Feature::new(
        "currency",
        FeatureKind::Attribute,
        vocabulary_ref(),
        Multiplicity::REQUIRED,
    )
    .with_default(DefaultValue::String("EUR".to_string()));
    let code = generate(&model)
        .expect("generate")
        .remove("models.rs")
        .unwrap();
    assert!(
        code.contains("currency: Currency::EUR,"),
        "declared default must lower to the matching variant:\n{code}"
    );
}

/// A declared default on a datatype-typed attribute must construct the
/// newtype, not the bare inner String (regression: the generated
/// `Default` impl assigned a `String` to the datatype's field and the
/// emitted crate did not compile — caught by the compile-everything
/// fixture, issue #16).
#[test]
fn datatype_attribute_default_wraps_the_newtype() {
    let mut model = currency_model();
    model.packages[0]
        .datatypes
        .push(DatatypeDef::new("Sku", None));
    model.packages[0].classes[0].features.push(
        Feature::new(
            "sku",
            FeatureKind::Attribute,
            TypeRef::Datatype {
                package: PKG.to_string(),
                name: "Sku".to_string(),
            },
            Multiplicity::REQUIRED,
        )
        .with_default(DefaultValue::String("FIX-0001".to_string())),
    );
    let code = generate(&model)
        .expect("generate")
        .remove("models.rs")
        .unwrap();
    assert!(
        code.contains("sku: Sku(\"FIX-0001\".to_string()),"),
        "datatype default must wrap the newtype:\n{code}"
    );
}

#[test]
fn vocabulary_default_that_is_not_an_entry_is_an_error() {
    let mut model = currency_model();
    model.packages[0].classes[0].features[1] = Feature::new(
        "currency",
        FeatureKind::Attribute,
        vocabulary_ref(),
        Multiplicity::REQUIRED,
    )
    .with_default(DefaultValue::String("XYZ".to_string()));
    assert!(
        generate(&model).is_err(),
        "unknown default key must fail generation"
    );
}

#[test]
fn non_identifier_entry_keys_are_rejected() {
    let mut model = currency_model();
    model.packages[0].vocabularies[0]
        .entries
        .push(entry("not a key!", "?", 1));
    let error = generate(&model).expect_err("non-identifier key must fail generation");
    assert!(
        error.to_string().contains("not a key!"),
        "error must name the offending key: {error:#}"
    );
}

#[test]
fn keyword_entry_keys_are_raw_escaped() {
    let mut model = currency_model();
    model.packages[0].vocabularies[0]
        .entries
        .push(entry("type", "T", 1));
    let code = generate(&model)
        .expect("generate")
        .remove("models.rs")
        .unwrap();
    assert!(
        code.contains("r#type,\n"),
        "keyword keys escape as r#type:\n{code}"
    );
}

#[test]
fn float_facet_uses_the_exact_json_number_text() {
    let mut model = currency_model();
    let vocabulary = &mut model.packages[0].vocabularies[0];
    vocabulary.facets.push(rex_ir::VocabularyFacet {
        name: "rate".to_string(),
        type_: PrimitiveType::Double,
    });
    // validate_entries guarantees every entry carries every facet.
    for (index, entry) in vocabulary.entries.iter_mut().enumerate() {
        entry.facets.insert(
            "rate".to_string(),
            if index == 0 {
                // Fractional values keep their exact JSON number text.
                DefaultValue::String("1.125".to_string())
            } else {
                DefaultValue::Int(1)
            },
        );
    }
    let code = generate(&model)
        .expect("generate")
        .remove("models.rs")
        .unwrap();
    assert!(
        code.contains("pub fn rate(self) -> f64"),
        "accessor:\n{code}"
    );
    assert!(
        code.contains("Self::USD => 1.125,"),
        "exact JSON number text is emitted as the literal:\n{code}"
    );
    assert!(
        code.contains("Self::EUR => 1.0,"),
        "integral values become x.0 float literals:\n{code}"
    );
}

#[test]
fn vocabulary_json_saves_and_loads_by_key() {
    let code = generate_currency();
    assert!(
        code.contains(".key().to_string()"),
        "save writes the entry key string:\n{code}"
    );
    assert!(
        code.contains("Currency::try_from(expect_string("),
        "load resolves the key via TryFrom<&str>:\n{code}"
    );
    assert!(
        code.contains("vocabulary-typed attributes serialize their entry key string"),
        "module header documents the vocabulary JSON contract:\n{code}"
    );
}

#[test]
fn duplicate_vocabulary_names_across_packages_are_rejected() {
    let mut model = currency_model();
    let mut second = Package::new("nz.example.other");
    second.vocabularies.push(VocabularyDef {
        name: "Currency".to_string(),
        description: None,
        source: "other:source".to_string(),
        version: None,
        key: "code".to_string(),
        facets: vec![],
        entries: vec![entry("USD", "$", 2)],
    });
    model.packages.push(second);
    let error = generate(&model).expect_err("duplicate vocabulary names must fail");
    assert!(
        error
            .to_string()
            .contains("duplicate vocabulary name 'Currency'"),
        "error was: {error:#}"
    );
}

#[test]
fn empty_vocabulary_is_rejected() {
    let mut model = currency_model();
    model.packages[0].vocabularies[0].entries.clear();
    let error = generate(&model).expect_err("empty vocabulary must fail");
    assert!(
        error
            .to_string()
            .contains("vocabulary 'Currency' has no entries"),
        "error was: {error:#}"
    );
}

/// A `readonly` feature keeps its stored field but gets **no** setter; an
/// `id` feature is codegen-neutral (stored and settable like any other).
/// The driver's lowering stage sets these flags (see the rex-driver
/// modifier tests); this locks in the backend's contract.
#[test]
fn readonly_feature_has_no_setter_and_id_feature_is_unchanged() {
    let mut model = Model::new();
    let mut package = Package::new("demo");
    package.classes.push(ClassDef::new(
        "Gadget",
        vec![],
        vec![
            Feature::new(
                "plain",
                FeatureKind::Attribute,
                TypeRef::Primitive(PrimitiveType::String),
                Multiplicity::REQUIRED,
            ),
            Feature::new(
                "code",
                FeatureKind::Attribute,
                TypeRef::Primitive(PrimitiveType::String),
                Multiplicity::REQUIRED,
            )
            .read_only(),
            Feature::new(
                "sku",
                FeatureKind::Attribute,
                TypeRef::Primitive(PrimitiveType::String),
                Multiplicity::REQUIRED,
            )
            .identifier(),
        ],
    ));
    model.packages.push(package);
    let code = generate(&model)
        .expect("generate")
        .remove("models.rs")
        .expect("models.rs");

    // All three features are stored as fields.
    assert!(code.contains("pub plain: String,"), "plain field:\n{code}");
    assert!(
        code.contains("pub code: String,"),
        "readonly field stays stored:\n{code}"
    );
    assert!(
        code.contains("pub sku: String,"),
        "id field stays stored:\n{code}"
    );

    // Plain and `id` features get setters; `readonly` does not.
    assert!(code.contains("pub fn set_plain"), "control setter:\n{code}");
    assert!(
        code.contains("pub fn set_sku"),
        "`id` is codegen-neutral: same setters as any other feature:\n{code}"
    );
    assert!(
        !code.contains("pub fn set_code"),
        "`readonly` features must not get a setter:\n{code}"
    );
}

// --- Tier 1: operation bodies and datatype create/convert ----------------

fn op_model() -> Model {
    let mut model = Model::new();
    let mut package = Package::new("demo");
    package.datatypes.push(
        DatatypeDef::new("Date", None)
            .bind("rust", "chrono::NaiveDate")
            .with_body("create", "rust", " Date(it) ")
            .with_body("convert", "rust", " self.0.clone() "),
    );
    package.classes.push(ClassDef::new(
        "Library",
        vec![],
        vec![Feature::new(
            "books",
            FeatureKind::Containment,
            TypeRef::Class {
                package: "demo".to_string(),
                name: "Book".to_string(),
            },
            Multiplicity::MANY,
        )],
    ));
    package.classes[0].operations.push(
        Operation::new(
            "getBook",
            TypeRef::Class {
                package: "demo".to_string(),
                name: "Book".to_string(),
            },
            vec![
                OperationParam {
                    name: "title".to_string(),
                    type_: TypeRef::Primitive(PrimitiveType::String),
                    multiplicity: None,
                },
                OperationParam {
                    name: "shelf".to_string(),
                    type_: TypeRef::Class {
                        package: "demo".to_string(),
                        name: "Shelf".to_string(),
                    },
                    multiplicity: None,
                },
            ],
        )
        .with_body(
            "rust",
            " res.books.iter().find_map(|b| { (b.title == *title).then_some(*b) }) ",
        ),
    );
    package.classes[0].operations.push(Operation::new(
        "countBooks",
        TypeRef::Primitive(PrimitiveType::Int),
        vec![],
    ));
    package.classes.push(ClassDef::new("Book", vec![], vec![]));
    package.classes.push(ClassDef::new("Shelf", vec![], vec![]));
    model.packages.push(package);
    model
}

#[test]
fn operation_signature_convention_and_verbatim_body() {
    let code = generate(&op_model())
        .expect("generate")
        .remove("models.rs")
        .expect("models.rs");
    // Fixed signature: `&self, res: &Resource, params…` with String
    // params as `String`, class params as `ShelfId`, class return as
    // `BookId` — and the body embedded verbatim (byte-identical, on its
    // own line between the braces).
    assert!(
            code.contains("pub fn get_book(&self, res: &Resource, title: String, shelf: ShelfId) -> BookId {\n res.books.iter().find_map(|b| { (b.title == *title).then_some(*b) }) \n    }"),
            "signature + verbatim body:\n{code}"
        );
    // The operation lives inside the class's impl block.
    let impl_start = code.find("impl Library {").expect("Library impl");
    let fn_at = code.find("pub fn get_book").expect("operation in output");
    assert!(fn_at > impl_start, "operation must be in the impl block");
}

#[test]
fn body_less_operation_emits_nothing() {
    let code = generate(&op_model())
        .expect("generate")
        .remove("models.rs")
        .expect("models.rs");
    assert!(
        !code.contains("count_books"),
        "body-less ops are hand-written hooks, not generated:\n{code}"
    );
}

#[test]
fn datatype_create_convert_impl_convention() {
    let code = generate(&op_model())
        .expect("generate")
        .remove("models.rs")
        .expect("models.rs");
    // `create(it: String) -> Self` and `convert(self) -> String`, bodies
    // verbatim. The example bodies compile: `create` returns
    // `Self` directly (the body's choice — no `Some(...)` wrapping),
    // `convert` clones through the newtype's `self.0`.
    assert!(
            code.contains("impl Date {\n    /// Tier 1 `create` body, embedded verbatim.\n    pub fn create(it: String) -> Self {\n Date(it) \n    }\n    /// Tier 1 `convert` body, embedded verbatim.\n    pub fn convert(self) -> String {\n self.0.clone() \n    }\n}"),
            "datatype impl:\n{code}"
        );
}

#[test]
fn module_header_documents_the_body_contract() {
    let code = generate(&op_model())
        .expect("generate")
        .remove("models.rs")
        .expect("models.rs");
    assert!(
        code.contains("operations with a `rust` body are emitted"),
        "header must describe Tier 1 bodies:\n{code}"
    );
}

// --- Tier 2: neutral `expr` bodies --------------------------------------

/// `op Book getBook(String title) { expr { books.first(b => b.title ==
/// title) } }`: the expression's type drives the return (`Option` of the
/// declared class id), and the body is the lowered Rust text.
#[test]
fn expr_body_op_lowers_to_a_typed_method() {
    let mut model = Model::new();
    let mut package = Package::new("demo");
    package.classes.push(ClassDef::new(
        "Library",
        vec![],
        vec![Feature::new(
            "books",
            FeatureKind::Containment,
            TypeRef::Class {
                package: "demo".to_string(),
                name: "Book".to_string(),
            },
            Multiplicity::MANY,
        )],
    ));
    package.classes[0].operations.push(
        Operation::new(
            "getBook",
            TypeRef::Class {
                package: "demo".to_string(),
                name: "Book".to_string(),
            },
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
        vec![Feature::new(
            "title",
            FeatureKind::Attribute,
            TypeRef::Primitive(PrimitiveType::String),
            Multiplicity::REQUIRED,
        )],
    ));
    model.packages.push(package);

    let code = generate(&model)
        .expect("generate")
        .remove("models.rs")
        .expect("models.rs");
    assert!(
            code.contains(
                "pub fn get_book(&self, res: &Resource, title: String) -> Option<BookId> {\n        self.books.clone().iter().find(|b| (res.book(**b).map(|o| o.title.clone()).expect(\"dangling `Book` id\") == title.clone())).copied()\n    }"
            ),
            "lowered method:\n{code}"
        );
    assert!(
        code.contains("lowered from the neutral `expr` body"),
        "doc comment must mark lowered bodies:\n{code}"
    );
}

#[test]
fn expr_body_type_error_fails_generation_naming_the_operation() {
    let mut model = Model::new();
    let mut package = Package::new("demo");
    package
        .classes
        .push(ClassDef::new("Library", vec![], vec![]));
    package.classes[0].operations.push(
        Operation::new("broken", TypeRef::Primitive(PrimitiveType::Int), vec![])
            .with_body("expr", "\"text\" + 1"),
    );
    package.classes.push(ClassDef::new("Book", vec![], vec![]));
    model.packages.push(package);

    let error = generate(&model).expect_err("R9 violation must fail generation");
    assert!(
        error.to_string().contains("'broken'"),
        "error must name the operation: {error:#}"
    );
    assert!(
        error
            .to_string()
            .contains("cannot concatenate string and int"),
        "error must carry the checker message: {error:#}"
    );
}

#[test]
fn expr_body_return_type_mismatch_fails_generation() {
    let mut model = Model::new();
    let mut package = Package::new("demo");
    package
        .classes
        .push(ClassDef::new("Library", vec![], vec![]));
    package.classes[0].operations.push(
        Operation::new("wrong", TypeRef::Primitive(PrimitiveType::Int), vec![])
            .with_body("expr", "\"text\""),
    );
    package.classes.push(ClassDef::new("Book", vec![], vec![]));
    model.packages.push(package);

    let error = generate(&model).expect_err("return mismatch must fail generation");
    assert!(
        error.to_string().contains("'wrong'") && error.to_string().contains("int"),
        "error must name the operation and both types: {error:#}"
    );
}

#[test]
fn conflicting_rust_and_expr_bodies_are_rejected() {
    let mut model = Model::new();
    let mut package = Package::new("demo");
    package
        .classes
        .push(ClassDef::new("Library", vec![], vec![]));
    package.classes[0].operations.push(
        Operation::new("both", TypeRef::Primitive(PrimitiveType::Int), vec![])
            .with_body("rust", " 42 ")
            .with_body("expr", "42"),
    );
    package.classes.push(ClassDef::new("Book", vec![], vec![]));
    model.packages.push(package);

    let error = generate(&model).expect_err("conflicting bodies must fail generation");
    assert!(
        error.to_string().contains("'both'"),
        "error must name the operation: {error:#}"
    );
}

/// A derived feature with an `expr` body lowers into an accessor on the
/// class. The return type follows the feature's declared typing (a
/// 0..1 derived attribute is `Option<..>`); a total expression of the
/// inner type wraps in `Some` — it simply never yields the absent state.
#[test]
fn derived_expr_body_lowers_to_an_accessor() {
    let mut model = Model::new();
    let mut package = Package::new("demo");
    package.classes.push(ClassDef::new(
        "Book",
        vec![],
        vec![
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
                "citation",
                FeatureKind::Attribute,
                TypeRef::Primitive(PrimitiveType::String),
                Multiplicity::OPTIONAL,
            )
            .derived()
            .with_body("expr", "if pages > 400 { title } else { \"short read\" }"),
        ],
    ));
    model.packages.push(package);

    let code = generate(&model)
        .expect("generate")
        .remove("models.rs")
        .expect("models.rs");
    assert!(
            code.contains(
                "pub fn citation(&self, res: &Resource) -> Option<String> {\n        Some(if (self.pages > 400i32) { self.title.clone() } else { \"short read\".to_string() })\n    }"
            ),
            "lowered accessor:\n{code}"
        );
    assert!(
        code.contains("Derived feature `citation`, lowered from the neutral `expr` body."),
        "doc comment must mark the lowered accessor:\n{code}"
    );
}

#[test]
fn derived_expr_body_type_error_fails_generation_naming_the_feature() {
    let mut model = Model::new();
    let mut package = Package::new("demo");
    package.classes.push(ClassDef::new(
        "Book",
        vec![],
        vec![Feature::new(
            "citation",
            FeatureKind::Attribute,
            TypeRef::Primitive(PrimitiveType::String),
            Multiplicity::OPTIONAL,
        )
        .derived()
        .with_body("expr", "\"text\" + 1")],
    ));
    model.packages.push(package);

    let error = generate(&model).expect_err("L2 violation must fail generation");
    assert!(
        error.to_string().contains("'citation'"),
        "error must name the feature: {error:#}"
    );
}

// --- Constraint validation (runtime reports, never panics) --------------

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

/// The IR equivalent of the validation scenario: one class carrying every
/// v1-supported constraint shape (string, many-valued, optional, int,
/// enum value bounds, datatype length bounds, String-keyed and Int-keyed
/// vocabulary bounds) plus an unconstrained sibling.
fn validation_model() -> Model {
    let mut model = Model::new();
    let mut package = Package::new("nz.example.validation");
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
            VocabularyEntry {
                key: "US".to_string(),
                facets: BTreeMap::from([(
                    "code".to_string(),
                    DefaultValue::String("US".to_string()),
                )]),
            },
            VocabularyEntry {
                key: "EURO".to_string(),
                facets: BTreeMap::from([(
                    "code".to_string(),
                    DefaultValue::String("EURO".to_string()),
                )]),
            },
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
            VocabularyEntry {
                key: "A".to_string(),
                facets: BTreeMap::from([("code".to_string(), DefaultValue::Int(2))]),
            },
            VocabularyEntry {
                key: "B".to_string(),
                facets: BTreeMap::from([("code".to_string(), DefaultValue::Int(7))]),
            },
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
                        package: "nz.example.validation".to_string(),
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
                        package: "nz.example.validation".to_string(),
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
                        package: "nz.example.validation".to_string(),
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
                        package: "nz.example.validation".to_string(),
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

fn generate_validation() -> String {
    generate(&validation_model())
        .expect("generate")
        .remove("models.rs")
        .expect("models.rs")
}

#[test]
fn constrained_class_gets_validate_reporting_bound_kinds() {
    let code = generate_validation();
    assert!(
        code.contains("pub fn validate(&self) -> Vec<rex_runtime::validation::Violation>"),
        "constrained class must carry a validate method:\n{code}"
    );
    assert!(
        code.contains("rex_runtime::validation::ConstraintKind::MinLength")
            && code.contains("rex_runtime::validation::ConstraintKind::MaxLength")
            && code.contains("rex_runtime::validation::ConstraintKind::Minimum")
            && code.contains("rex_runtime::validation::ConstraintKind::Maximum"),
        "all four bound kinds must be reported:\n{code}"
    );
    assert!(
        code.contains("feature: \"sku\".to_string()")
            || code.contains("feature: \"name\".to_string()"),
        "violations must name the declared feature:\n{code}"
    );
    assert!(
        code.contains("let len = value.chars().count();"),
        "string bounds check the element length:\n{code}"
    );
    assert!(
        code.contains("if len < 2 {") && code.contains("if len > 8 {"),
        "string bounds compare inclusively:\n{code}"
    );
    assert!(
        code.contains("detail: format!(\"length {len} is below minLength 2\")"),
        "min-length detail:\n{code}"
    );
    assert!(
        code.contains("detail: format!(\"length {len} exceeds maxLength 8\")"),
        "max-length detail:\n{code}"
    );
    assert!(
        code.contains("let number = i64::from(*value);"),
        "int bounds widen to i64 (bound literals must not overflow the field type):\n{code}"
    );
    assert!(
        code.contains("detail: format!(\"value {number} is below minimum 0\")")
            && code.contains("detail: format!(\"value {number} exceeds maximum 100\")"),
        "numeric details:\n{code}"
    );
    // The method lives inside the class's impl block.
    let impl_start = code.find("impl Profile {").expect("Profile impl");
    let validate_at = code.find("pub fn validate").expect("validate in output");
    assert!(
        validate_at > impl_start,
        "validate must be in the impl block"
    );
}

#[test]
fn unconstrained_model_emits_no_validate() {
    let code = generate_currency();
    assert!(
        !code.contains("fn validate"),
        "classes without constrained features must not carry validate:\n{code}"
    );
}

#[test]
fn many_valued_and_optional_attributes_check_elements() {
    let code = generate_validation();
    assert!(
        code.contains("for (index, value) in self.nicknames.iter().enumerate()"),
        "many-valued constraints apply per element:\n{code}"
    );
    assert!(
        code.contains("detail: format!(\"[{index}] length {len} is below minLength 2\")"),
        "element index must appear in the detail:\n{code}"
    );
    assert!(
        code.contains("if let Some(value) = &self.alt_name {"),
        "optional attributes check when present:\n{code}"
    );
}

#[test]
fn enum_and_vocabulary_bounds_follow_value_and_key_facets() {
    let code = generate_validation();
    assert!(
        code.contains("let number = value.value();"),
        "enum numeric bounds check the literal value:\n{code}"
    );
    assert!(
        code.contains("let len = value.key().chars().count();"),
        "String-keyed vocabulary length bounds check the entry key:\n{code}"
    );
    assert!(
        code.contains("let number = value.code();"),
        "Int-keyed vocabulary numeric bounds use the key facet accessor:\\n{code}"
    );
}

// ---------------------------------------------------------------------------
// Opposite-maintaining mutators: same-class pairs (issue #3). When both ends
// share one class, the class-derived parameter names collide and the
// generated code cannot compile — the value parameter is named after the
// near-end role instead.
// ---------------------------------------------------------------------------

fn class_ref(name: &str) -> TypeRef {
    TypeRef::Class {
        package: PKG.to_string(),
        name: name.to_string(),
    }
}

/// One class with a same-class opposite pair: `a` (the canonical,
/// lexicographically-first end) and `b`.
fn self_pair_model(
    class: &str,
    a: (&str, FeatureKind, Multiplicity),
    b: (&str, FeatureKind, Multiplicity),
) -> Model {
    let mut model = Model::new();
    let mut package = Package::new(PKG);
    package.classes.push(ClassDef::new(
        class,
        vec![],
        vec![
            Feature::new(a.0, a.1, class_ref(class), a.2).with_opposite(class, b.0),
            Feature::new(b.0, b.1, class_ref(class), b.2).with_opposite(class, a.0),
        ],
    ));
    model.packages.push(package);
    model
}

fn generate_self_pair(
    class: &str,
    a: (&str, FeatureKind, Multiplicity),
    b: (&str, FeatureKind, Multiplicity),
) -> String {
    generate(&self_pair_model(class, a, b))
        .expect("generate")
        .remove("models.rs")
        .expect("models.rs")
}

#[test]
fn same_class_single_many_pair_names_the_value_parameter_by_role() {
    let code = generate_self_pair(
        "Employee",
        (
            "manager",
            FeatureKind::CrossReference,
            Multiplicity::OPTIONAL,
        ),
        ("reports", FeatureKind::CrossReference, Multiplicity::MANY),
    );
    // `set_manager` takes the new manager under its role name, not a second
    // colliding `employee`.
    assert!(
        code.contains(
            "pub fn employee_set_manager(&mut self, employee: EmployeeId, \
             manager: Option<EmployeeId>)"
        ),
        "{code}"
    );
    // The body reads the renamed variable everywhere.
    assert!(code.contains("if let Some(target) = manager {"), "{code}");
    assert!(code.contains("if let Some(new_b) = manager {"), "{code}");
    // `add_reports`/`remove_reports` take the added/removed employee under
    // the near-end role name.
    assert!(
        code.contains(
            "pub fn employee_add_reports(&mut self, employee: EmployeeId, \
             reports: EmployeeId)"
        ),
        "{code}"
    );
    assert!(
        code.contains(
            "pub fn employee_remove_reports(&mut self, employee: EmployeeId, \
             reports: EmployeeId)"
        ),
        "{code}"
    );
    assert!(code.contains("b.manager = Some(employee);"), "{code}");
    // No duplicated parameter name survives anywhere.
    assert!(
        !code.contains("employee: EmployeeId, employee: EmployeeId"),
        "{code}"
    );
}

#[test]
fn same_class_many_many_pair_names_the_value_parameter_by_role() {
    let code = generate_self_pair(
        "Node",
        ("children", FeatureKind::CrossReference, Multiplicity::MANY),
        ("parents", FeatureKind::CrossReference, Multiplicity::MANY),
    );
    assert!(
        code.contains("pub fn node_add_children(&mut self, node: NodeId, children: NodeId)"),
        "{code}"
    );
    assert!(
        code.contains("pub fn node_add_parents(&mut self, node: NodeId, parents: NodeId)"),
        "{code}"
    );
    assert!(!code.contains("node: NodeId, node: NodeId"), "{code}");
}

#[test]
fn same_class_single_single_pair_names_the_value_parameter_by_role() {
    let code = generate_self_pair(
        "Person",
        (
            "sponsor",
            FeatureKind::CrossReference,
            Multiplicity::OPTIONAL,
        ),
        (
            "protege",
            FeatureKind::CrossReference,
            Multiplicity::OPTIONAL,
        ),
    );
    assert!(
        code.contains(
            "pub fn person_set_sponsor(&mut self, person: PersonId, \
             sponsor: Option<PersonId>)"
        ),
        "{code}"
    );
    assert!(
        code.contains(
            "pub fn person_set_protege(&mut self, person: PersonId, \
             protege: Option<PersonId>)"
        ),
        "{code}"
    );
    assert!(
        !code.contains("person: PersonId, person: PersonId"),
        "{code}"
    );
}

#[test]
fn same_class_one_sided_containment_names_the_value_parameter_by_role() {
    // One-way same-class containment (`contains` without an opposite)
    // collides on the class name just the same.
    let mut model = Model::new();
    let mut package = Package::new(PKG);
    package.classes.push(ClassDef::new(
        "TreeNode",
        vec![],
        vec![Feature::new(
            "items",
            FeatureKind::Containment,
            class_ref("TreeNode"),
            Multiplicity::MANY,
        )],
    ));
    model.packages.push(package);
    let code = generate(&model)
        .expect("generate")
        .remove("models.rs")
        .expect("models.rs");
    assert!(
        code.contains(
            "pub fn tree_node_add_items(&mut self, tree_node: TreeNodeId, \
             items: TreeNodeId)"
        ),
        "{code}"
    );
    assert!(
        !code.contains("tree_node: TreeNodeId, tree_node: TreeNodeId"),
        "{code}"
    );
}

#[test]
fn residual_parameter_collision_falls_back_to_a_numbered_name() {
    // When the role name *is* the class name (`Manager.manager`), the role
    // cannot disambiguate either — a numbered fallback keeps the signature
    // deterministic and valid.
    let code = generate_self_pair(
        "Manager",
        (
            "manager",
            FeatureKind::CrossReference,
            Multiplicity::OPTIONAL,
        ),
        ("reports", FeatureKind::CrossReference, Multiplicity::MANY),
    );
    assert!(
        code.contains(
            "pub fn manager_set_manager(&mut self, manager: ManagerId, \
             manager_2: Option<ManagerId>)"
        ),
        "{code}"
    );
    assert!(code.contains("if let Some(target) = manager_2 {"), "{code}");
    assert!(
        !code.contains("manager: ManagerId, manager: ManagerId"),
        "{code}"
    );
}

#[test]
fn distinct_class_pairs_keep_the_class_named_value_parameter() {
    // The disambiguation never touches distinct-class pairs: the value
    // parameter stays the target class's snake name.
    let mut model = Model::new();
    let mut package = Package::new(PKG);
    package.classes.push(ClassDef::new(
        "Library",
        vec![],
        vec![Feature::new(
            "books",
            FeatureKind::Containment,
            class_ref("Book"),
            Multiplicity::MANY,
        )
        .with_opposite("Book", "library")],
    ));
    package.classes.push(ClassDef::new(
        "Book",
        vec![],
        vec![Feature::new(
            "library",
            FeatureKind::CrossReference,
            class_ref("Library"),
            Multiplicity::OPTIONAL,
        )
        .with_opposite("Library", "books")],
    ));
    model.packages.push(package);
    let code = generate(&model)
        .expect("generate")
        .remove("models.rs")
        .expect("models.rs");
    assert!(
        code.contains("pub fn library_add_books(&mut self, library: LibraryId, book: BookId)"),
        "{code}"
    );
    assert!(
        code.contains(
            "pub fn book_set_library(&mut self, book: BookId, \
             library: Option<LibraryId>)"
        ),
        "{code}"
    );
}
