//! Integration tests for `import sigil` declarations: a `.mox` file may
//! import a Rune DSL (`.rosetta`) file, whose namespaces lower into
//! synthetic rexlang packages (see the `sigil` module's normative contract).
//!
//! The driver is filesystem-free: rosetta content arrives through
//! [`rex_driver::SigilImports`], keyed by `(mox file path, import path)`.
//! Additional entries for the same mox path are candidates; the driver
//! selects the transitive namespace closure the named file needs.

use rex_driver::{compile_files, compile_str, render};
use rex_ir::{FeatureKind, TypeRef};

const MOX: &str = "demo.mox";
const TRADE: &str = "oracle/trade.rosetta";

fn trade_rosetta() -> String {
    concat!(
        "namespace oracle.basic\n\n",
        "type Trade:\n",
        "    id string (1..1) <\"The trade identifier.\">\n",
        "    quantity number (1..1)\n",
        "    legs Trade (1..*)\n"
    )
    .to_string()
}

fn sigil_imports() -> rex_driver::SigilImports {
    rex_driver::SigilImports::new().provide(MOX, TRADE, trade_rosetta())
}

/// Compiles `files` with no schema content and the given sigil content.
fn compile_with_sigil(
    files: &[(String, String)],
    sigil: &rex_driver::SigilImports,
) -> rex_driver::MultiCompilation {
    compile_files(
        files,
        &rex_driver::DomainImports {
            schemas: rex_driver::SchemaImports::new(),
            sigil: sigil.clone(),
        },
    )
}

fn mox_with_sigil() -> String {
    concat!(
        "package demo\n\n",
        "import sigil \"oracle/trade.rosetta\"\n\n",
        "class Book {\n",
        "    refers oracle.basic.Trade about\n",
        "}\n"
    )
    .to_string()
}

fn compile_clean(compilation: &rex_driver::MultiCompilation) -> &rex_ir::Model {
    let sources = [(MOX, mox_with_sigil()), (TRADE, trade_rosetta())];
    let mut all: Vec<(String, rex_driver::Diagnostic)> = compilation.diagnostics.clone();
    all.extend(compilation.sigil_diagnostics.clone());
    assert!(
        all.is_empty(),
        "expected a clean compilation:\n{}",
        all.iter()
            .map(|(path, diagnostic)| render(
                path,
                sources
                    .iter()
                    .find(|(name, _)| *name == path.as_str())
                    .map(|(_, source)| source.as_str())
                    .unwrap_or(""),
                std::slice::from_ref(diagnostic),
            ))
            .collect::<String>()
    );
    compilation.model.as_ref().expect("model lowered")
}

// --- the mapping contract ---------------------------------------------------

#[test]
fn data_lowers_to_a_class_with_cardinalities_and_primitive_references() {
    let compilation = compile_with_sigil(&[(MOX.to_string(), mox_with_sigil())], &sigil_imports());
    let model = compile_clean(&compilation);
    assert_eq!(model.packages.len(), 2);
    assert_eq!(model.packages[0].name, "demo");
    assert_eq!(model.packages[1].name, "oracle.basic");

    let trade = &model.packages[1].classes[0];
    assert_eq!(trade.name, "Trade");
    // `id string (1..1)` with the attribute definition as the description.
    assert_eq!(trade.features[0].name, "id");
    assert_eq!(trade.features[0].kind, FeatureKind::Attribute);
    assert_eq!(
        trade.features[0].type_,
        TypeRef::Primitive(rex_ir::PrimitiveType::String)
    );
    assert_eq!(trade.features[0].multiplicity.lower, 1);
    assert_eq!(
        trade.features[0].multiplicity.upper,
        rex_ir::Upper::Finite(1)
    );
    assert_eq!(
        trade.features[0].description.as_deref(),
        Some("The trade identifier.")
    );
    // `number` maps to the `double` primitive.
    assert_eq!(
        trade.features[1].type_,
        TypeRef::Primitive(rex_ir::PrimitiveType::Double)
    );
    // `legs Trade[1..*]` — an `n..m` cardinality and a same-namespace class
    // reference.
    assert_eq!(
        trade.features[2].type_,
        TypeRef::Class {
            package: "oracle.basic".to_string(),
            name: "Trade".to_string(),
        }
    );
    assert_eq!(trade.features[2].multiplicity.lower, 1);
    assert_eq!(
        trade.features[2].multiplicity.upper,
        rex_ir::Upper::Unbounded
    );
}

#[test]
fn zero_to_many_and_bounded_cardinalities_map_verbatim() {
    let rosetta = concat!(
        "namespace oracle.card\n\n",
        "type Bag:\n",
        "    items string (0..*)\n",
        "    pair string (2..10)\n"
    );
    let compilation = compile_with_sigil(
        &[(
            MOX.to_string(),
            "package demo\n\nimport sigil \"bag.rosetta\"\n\nclass C { String x }\n".to_string(),
        )],
        &rex_driver::SigilImports::new().provide(MOX, "bag.rosetta", rosetta),
    );
    let model = compile_clean(&compilation);
    let bag = &model.packages[1].classes[0];
    assert_eq!(bag.features[0].multiplicity.lower, 0);
    assert_eq!(bag.features[0].multiplicity.upper, rex_ir::Upper::Unbounded);
    assert_eq!(bag.features[1].multiplicity.lower, 2);
    assert_eq!(
        bag.features[1].multiplicity.upper,
        rex_ir::Upper::Finite(10)
    );
}

#[test]
fn choice_lowers_to_an_interface_and_data_extends_it() {
    let rosetta = concat!(
        "namespace oracle.choice\n\n",
        "type Product extends ProductIdentifier:\n",
        "    kind ProductIdentifier (1..1)\n",
        "\n",
        "choice ProductIdentifier:\n",
        "    Isin\n",
        "\n",
        "type Isin:\n",
        "    value string (1..1)\n"
    );
    let compilation = compile_with_sigil(
        &[(
            MOX.to_string(),
            "package demo\n\nimport sigil \"choice.rosetta\"\n\nclass C { String x }\n".to_string(),
        )],
        &rex_driver::SigilImports::new().provide(MOX, "choice.rosetta", rosetta),
    );
    let model = compile_clean(&compilation);
    let package = &model.packages[1];
    let product = package
        .classes
        .iter()
        .find(|class| class.name == "Product")
        .expect("the Product class");
    assert_eq!(
        product.extends,
        vec![TypeRef::Interface {
            package: "oracle.choice".to_string(),
            name: "ProductIdentifier".to_string(),
        }]
    );
    let choice = &package.interfaces[0];
    assert_eq!(choice.name, "ProductIdentifier");
    let isin = package
        .classes
        .iter()
        .find(|class| class.name == "Isin")
        .expect("the Isin class");
    // Choice options are skipped (an interface declares no features); the
    // option's implicit `(0..1)` cardinality never becomes observable.
    assert!(choice_has_no_features_shape(package));
    assert_eq!(isin.features[0].multiplicity.lower, 1);
}

/// rexlang interfaces carry no features: a choice lowers to a name only.
fn choice_has_no_features_shape(package: &rex_ir::Package) -> bool {
    package
        .interfaces
        .iter()
        .all(|interface| interface.name == "ProductIdentifier")
}

#[test]
fn enumeration_lowers_with_synthesized_values_and_display_labels() {
    let rosetta = concat!(
        "namespace oracle.enum\n\n",
        "enum QuotingType: <\"How the quote was made.\">\n",
        "    ORDER <\"Quoted by order.\">\n",
        "    TRADE displayName \"By trade\" <\"Quoted by trade.\">\n",
        "    COUNTER\n"
    );
    let compilation = compile_with_sigil(
        &[(
            MOX.to_string(),
            "package demo\n\nimport sigil \"quoting.rosetta\"\n\nclass C { String x }\n"
                .to_string(),
        )],
        &rex_driver::SigilImports::new().provide(MOX, "quoting.rosetta", rosetta),
    );
    let model = compile_clean(&compilation);
    let enum_def = &model.packages[1].enums[0];
    assert_eq!(enum_def.name, "QuotingType");
    assert_eq!(
        enum_def.description.as_deref(),
        Some("How the quote was made.")
    );
    let values: Vec<(String, Option<String>, i64)> = enum_def
        .literals
        .iter()
        .map(|literal| (literal.name.clone(), literal.label.clone(), literal.value))
        .collect();
    assert_eq!(
        values,
        vec![
            ("ORDER".to_string(), None, 0),
            ("TRADE".to_string(), Some("By trade".to_string()), 1),
            ("COUNTER".to_string(), None, 2),
        ]
    );
    assert_eq!(
        enum_def.literals[1].description.as_deref(),
        Some("Quoted by trade.")
    );
}

#[test]
fn type_alias_lowers_to_a_datatype_wrapping_the_primitive() {
    let rosetta = concat!(
        "namespace oracle.alias\n\n",
        "typeAlias CurrencyCode: <\"An ISO currency code.\">\n",
        "    string\n"
    );
    let compilation = compile_with_sigil(
        &[(
            MOX.to_string(),
            "package demo\n\nimport sigil \"alias.rosetta\"\n\nclass C { String x }\n".to_string(),
        )],
        &rex_driver::SigilImports::new().provide(MOX, "alias.rosetta", rosetta),
    );
    let model = compile_clean(&compilation);
    let datatype = &model.packages[1].datatypes[0];
    assert_eq!(datatype.name, "CurrencyCode");
    assert_eq!(datatype.platform.as_deref(), Some("string"));
    assert_eq!(
        datatype.description.as_deref(),
        Some("An ISO currency code.")
    );
}

#[test]
fn builtin_primitives_map_to_their_rexlang_counterparts() {
    let rosetta = concat!(
        "namespace oracle.builtin\n\n",
        "type All:\n",
        "    a boolean (1..1)\n",
        "    b string (1..1)\n",
        "    c int (1..1)\n",
        "    d number (1..1)\n",
        "    e date (1..1)\n"
    );
    let compilation = compile_with_sigil(
        &[(
            MOX.to_string(),
            "package demo\n\nimport sigil \"all.rosetta\"\n\nclass C { String x }\n".to_string(),
        )],
        &rex_driver::SigilImports::new().provide(MOX, "all.rosetta", rosetta),
    );
    let model = compile_clean(&compilation);
    // No `com.rosetta.model` package: every reference mapped to a primitive.
    assert_eq!(
        model.packages.len(),
        2,
        "{:?}",
        model.packages.iter().map(|p| &p.name).collect::<Vec<_>>()
    );
    let all = &model.packages[1].classes[0];
    let expected = [
        (rex_ir::PrimitiveType::Boolean, "a"),
        (rex_ir::PrimitiveType::String, "b"),
        (rex_ir::PrimitiveType::Int, "c"),
        (rex_ir::PrimitiveType::Double, "d"),
        (rex_ir::PrimitiveType::Date, "e"),
    ];
    for (feature, (primitive, name)) in all.features.iter().zip(expected) {
        assert_eq!(feature.name, name);
        assert_eq!(feature.type_, TypeRef::Primitive(primitive));
    }
}

#[test]
fn unmapped_builtins_lower_to_opaque_datatypes_only_when_referenced() {
    let rosetta = concat!(
        "namespace oracle.when\n\n",
        "type Moment:\n",
        "    at time (1..1)\n",
        "    regex pattern (0..1)\n"
    );
    let compilation = compile_with_sigil(
        &[(
            MOX.to_string(),
            "package demo\n\nimport sigil \"moment.rosetta\"\n\nclass C { String x }\n".to_string(),
        )],
        &rex_driver::SigilImports::new().provide(MOX, "moment.rosetta", rosetta),
    );
    let model = compile_clean(&compilation);
    // `time` and `pattern` are referenced, so the `com.rosetta.model`
    // package joins with exactly those two opaque datatypes.
    let builtin = model
        .packages
        .iter()
        .find(|package| package.name == "com.rosetta.model")
        .expect("the builtin package is emitted when referenced");
    assert_eq!(builtin.datatypes.len(), 2, "{:?}", builtin.datatypes);
    assert!(builtin
        .datatypes
        .iter()
        .any(|d| d.name == "time" && d.platform.is_none()));
    assert!(builtin
        .datatypes
        .iter()
        .any(|d| d.name == "pattern" && d.platform.is_none()));
    assert!(builtin.enums.is_empty() && builtin.classes.is_empty());
    let moment = &model.packages[1].classes[0];
    assert_eq!(
        moment.features[0].type_,
        TypeRef::Datatype {
            package: "com.rosetta.model".to_string(),
            name: "time".to_string(),
        }
    );
}

#[test]
fn builtin_namespace_is_absent_when_nothing_from_it_is_referenced() {
    let rosetta = "namespace oracle.pure\n\ntype Plain:\n    x int (1..1)\n";
    let compilation = compile_with_sigil(
        &[(
            MOX.to_string(),
            "package demo\n\nimport sigil \"plain.rosetta\"\n\nclass C { String x }\n".to_string(),
        )],
        &rex_driver::SigilImports::new().provide(MOX, "plain.rosetta", rosetta),
    );
    let model = compile_clean(&compilation);
    assert_eq!(model.packages.len(), 2);
    assert!(model
        .packages
        .iter()
        .all(|package| package.name != "com.rosetta.model"));
}

#[test]
fn references_to_skipped_kinds_are_clean_errors() {
    let rosetta = concat!(
        "namespace oracle.rules\n\n",
        "reporting rule Bad:\n",
        "    True\n",
        "\n",
        "type Holder:\n",
        "    thing Bad (1..1)\n"
    );
    let compilation = compile_with_sigil(
        &[(
            MOX.to_string(),
            "package demo\n\nimport sigil \"rules.rosetta\"\n\nclass C { String x }\n".to_string(),
        )],
        &rex_driver::SigilImports::new().provide(MOX, "rules.rosetta", rosetta),
    );
    assert!(
        compilation.model.is_none(),
        "a reference to a skipped kind blocks the artifact"
    );
    let all: Vec<&rex_driver::Diagnostic> = compilation
        .sigil_diagnostics
        .iter()
        .map(|(_, diagnostic)| diagnostic)
        .collect();
    assert!(
        all.iter().any(|diagnostic| diagnostic.is_error()),
        "expected a blocking error, got: {all:?}"
    );
}

// --- namespaces -------------------------------------------------------------

#[test]
fn the_same_namespace_across_files_merges_into_one_package() {
    let named = concat!(
        "namespace oracle.merged\n\n",
        "import oracle.extra.*\n\n",
        "type First:\n",
        "    other Extra (1..1)\n"
    );
    let extra = "namespace oracle.extra\n\ntype Extra:\n    x int (1..1)\n";
    // The named file plus a candidate: the namespace import pulls the
    // candidate in transitively.
    let compilation = compile_with_sigil(
        &[(
            MOX.to_string(),
            "package demo\n\nimport sigil \"first.rosetta\"\n\nclass C { String x }\n".to_string(),
        )],
        &rex_driver::SigilImports::new()
            .provide(MOX, "first.rosetta", named)
            .provide(MOX, "extra/extra.rosetta", extra),
    );
    let model = compile_clean(&compilation);
    assert_eq!(model.packages.len(), 3);
    assert_eq!(model.packages[1].name, "oracle.merged");
    assert_eq!(model.packages[2].name, "oracle.extra");
    let merged = &model.packages[1];
    assert_eq!(merged.classes.len(), 1);
    assert_eq!(
        merged.classes[0].features[0].type_,
        TypeRef::Class {
            package: "oracle.extra".to_string(),
            name: "Extra".to_string(),
        }
    );
}

#[test]
fn same_namespace_files_without_imports_still_see_each_other() {
    let named = "namespace oracle.sibling\n\ntype Mine:\n    yours Theirs (1..1)\n";
    let sibling = "namespace oracle.sibling\n\ntype Theirs:\n    x int (1..1)\n";
    let compilation = compile_with_sigil(
        &[(
            MOX.to_string(),
            "package demo\n\nimport sigil \"mine.rosetta\"\n\nclass C { String x }\n".to_string(),
        )],
        &rex_driver::SigilImports::new()
            .provide(MOX, "mine.rosetta", named)
            .provide(MOX, "theirs.rosetta", sibling),
    );
    let model = compile_clean(&compilation);
    assert_eq!(model.packages.len(), 2);
    assert_eq!(
        model.packages[1].classes.len(),
        2,
        "same-namespace siblings merge"
    );
}

#[test]
fn unselected_candidates_never_leak_into_the_artifact() {
    let named = "namespace oracle.only\n\ntype Only:\n    x int (1..1)\n";
    let unrelated = "namespace oracle.other\n\ntype Other:\n    x int (1..1)\n";
    let compilation = compile_with_sigil(
        &[(
            MOX.to_string(),
            "package demo\n\nimport sigil \"only.rosetta\"\n\nclass C { String x }\n".to_string(),
        )],
        &rex_driver::SigilImports::new()
            .provide(MOX, "only.rosetta", named)
            .provide(MOX, "other/other.rosetta", unrelated),
    );
    let model = compile_clean(&compilation);
    assert_eq!(model.packages.len(), 2);
    assert_eq!(model.packages[1].name, "oracle.only");
}

#[test]
fn namespace_collision_with_a_declared_package_is_an_error_naming_both() {
    let rosetta = "namespace demo\n\ntype Clashing:\n    x int (1..1)\n";
    let compilation = compile_with_sigil(
        &[(
            MOX.to_string(),
            "package demo\n\nimport sigil \"clash.rosetta\"\n\nclass C { String x }\n".to_string(),
        )],
        &rex_driver::SigilImports::new().provide(MOX, "clash.rosetta", rosetta),
    );
    assert!(compilation.model.is_none());
    let (_, diagnostic) = compilation
        .sigil_diagnostics
        .iter()
        .find(|(_, diagnostic)| diagnostic.message.contains("collides"))
        .expect("the collision error");
    assert!(
        diagnostic.message.contains("demo") && diagnostic.message.contains(MOX),
        "the error must name both sources: {}",
        diagnostic.message
    );
    assert_eq!(
        compilation.sigil_diagnostics[0].0, MOX,
        "collision errors are tagged with the importing .mox file"
    );
}

// --- diagnostics propagation --------------------------------------------------

#[test]
fn a_sigil_parse_error_surfaces_tagged_with_the_rosetta_path() {
    let compilation = compile_with_sigil(
        &[(
            MOX.to_string(),
            "package demo\n\nimport sigil \"oracle/trade.rosetta\"\n\nclass C { String x }\n"
                .to_string(),
        )],
        &rex_driver::SigilImports::new().provide(
            MOX,
            TRADE,
            "namespace oracle.basic\n\ntype Trade:\n    id (1..1)\n",
        ),
    );
    assert!(
        compilation.model.is_none(),
        "parse errors block the artifact"
    );
    let (path, diagnostic) = compilation
        .sigil_diagnostics
        .iter()
        .find(|(_, diagnostic)| diagnostic.is_error())
        .expect("a tagged error");
    assert_eq!(path, TRADE);
    assert!(
        diagnostic.span.is_some(),
        "the span points into the rosetta text"
    );
}

#[test]
fn a_sigil_resolution_error_surfaces_tagged_with_the_rosetta_path() {
    let compilation = compile_with_sigil(
        &[(
            MOX.to_string(),
            "package demo\n\nimport sigil \"oracle/trade.rosetta\"\n\nclass C { String x }\n"
                .to_string(),
        )],
        &rex_driver::SigilImports::new().provide(
            MOX,
            TRADE,
            "namespace oracle.basic\n\ntype Trade:\n    who NoSuchType (1..1)\n",
        ),
    );
    assert!(
        compilation.model.is_none(),
        "resolution errors block the artifact"
    );
    let (path, diagnostic) = compilation
        .sigil_diagnostics
        .iter()
        .find(|(_, diagnostic)| diagnostic.is_error())
        .expect("a tagged error");
    assert_eq!(path, TRADE);
    assert!(
        diagnostic.message.contains("NoSuchType"),
        "the error names the unresolved type: {}",
        diagnostic.message
    );
}

#[test]
fn an_imported_namespace_no_file_declares_is_an_error() {
    let named = concat!(
        "namespace oracle.missing\n\n",
        "import oracle.nowhere.*\n\n",
        "type Lonely:\n",
        "    other Phantom (1..1)\n"
    );
    let compilation = compile_with_sigil(
        &[(
            MOX.to_string(),
            "package demo\n\nimport sigil \"lonely.rosetta\"\n\nclass C { String x }\n".to_string(),
        )],
        &rex_driver::SigilImports::new().provide(MOX, "lonely.rosetta", named),
    );
    assert!(compilation.model.is_none());
    assert!(compilation
        .sigil_diagnostics
        .iter()
        .any(|(path, diagnostic)| {
            path == "lonely.rosetta"
                && diagnostic.is_error()
                && diagnostic.message.contains("Phantom")
        }));
}

#[test]
fn unselected_candidates_with_parse_errors_do_not_block_the_model() {
    let named = "namespace oracle.fine\n\ntype Fine:\n    x int (1..1)\n";
    let broken = "namespace oracle.broken\n\ntype Broken:\n    x (1..1)\n";
    let compilation = compile_with_sigil(
        &[(
            MOX.to_string(),
            "package demo\n\nimport sigil \"fine.rosetta\"\n\nclass C { String x }\n".to_string(),
        )],
        &rex_driver::SigilImports::new()
            .provide(MOX, "fine.rosetta", named)
            .provide(MOX, "broken/broken.rosetta", broken),
    );
    let model = compile_clean(&compilation);
    assert_eq!(model.packages[1].name, "oracle.fine");
}

// --- provider contract ---------------------------------------------------------

#[test]
fn missing_content_is_an_error_with_the_schema_wording() {
    let compilation = compile_with_sigil(
        &[(
            MOX.to_string(),
            "package demo\n\nimport sigil \"oracle/trade.rosetta\"\n\nclass C { String x }\n"
                .to_string(),
        )],
        &rex_driver::SigilImports::new(),
    );
    assert!(compilation.model.is_none());
    let (path, diagnostic) = compilation
        .sigil_diagnostics
        .iter()
        .find(|(_, diagnostic)| diagnostic.message.contains("was not provided"))
        .expect("the not-provided diagnostic");
    assert_eq!(
        path, MOX,
        "import-level errors are tagged with the .mox file"
    );
    assert_eq!(
        diagnostic.message,
        "imported sigil 'oracle/trade.rosetta' was not provided"
    );
}

#[test]
fn content_provided_for_another_file_does_not_satisfy_the_import() {
    let compilation = compile_with_sigil(
        &[
            (MOX.to_string(), mox_with_sigil()),
            (
                "other.mox".to_string(),
                "package other\n\nclass Unrelated { String x }\n".to_string(),
            ),
        ],
        &rex_driver::SigilImports::new().provide("other.mox", TRADE, trade_rosetta()),
    );
    assert!(
        compilation.sigil_diagnostics.iter().any(
            |(path, diagnostic)| path == MOX && diagnostic.message.contains("was not provided")
        ),
        "content keyed by another .mox file must not satisfy the import: {:?}",
        compilation.sigil_diagnostics
    );
}

#[test]
fn compile_str_and_compile_files_error_without_imports() {
    let compilation = rex_driver::compile_str(
        MOX,
        "package demo\n\nimport sigil \"x.rosetta\"\n\nclass C { String x }\n",
        &rex_driver::DomainImports::default(),
    );
    assert!(compilation.model.is_none());
    assert!(
        compilation
            .sigil_diagnostics
            .iter()
            .any(|(path, diagnostic)| path == MOX
                && diagnostic.message == "imported sigil 'x.rosetta' was not provided"),
        "{:?}",
        compilation.sigil_diagnostics
    );
    let compilation = compile_files(
        &[(
            MOX.to_string(),
            "package demo\n\nimport sigil \"x.rosetta\"\n\nclass C { String x }\n".to_string(),
        )],
        &rex_driver::DomainImports::default(),
    );
    assert!(compilation.model.is_none());
    assert!(
        compilation
            .sigil_diagnostics
            .iter()
            .any(|(_, diagnostic)| diagnostic.message.contains("was not provided")),
        "{:?}",
        compilation.sigil_diagnostics
    );
}

#[test]
fn compile_files_is_unchanged_without_sigil_imports() {
    // Byte compatibility: a model without `import sigil` compiles to the
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
    let plain = compile_files(&files, &rex_driver::DomainImports::default());
    let with = compile_files(
        &files,
        &rex_driver::DomainImports {
            schemas: rex_driver::SchemaImports::new(),
            sigil: rex_driver::SigilImports::new(),
        },
    );
    assert_eq!(
        plain
            .model
            .as_ref()
            .map(|model| model.to_json_pretty().expect("serialize")),
        with.model
            .as_ref()
            .map(|model| model.to_json_pretty().expect("serialize")),
        "IR must be identical without import-sigil declarations"
    );
    assert_eq!(plain.diagnostics, with.diagnostics);
    assert!(with.sigil_diagnostics.is_empty());
}

// --- importing-package interaction -----------------------------------------------

#[test]
fn the_importing_package_resolves_bare_and_qualified_references() {
    let compilation = compile_with_sigil(
        &[(
            MOX.to_string(),
            concat!(
                "package demo\n\n",
                "import sigil \"oracle/trade.rosetta\"\n\n",
                "class Book {\n",
                "    refers Trade bare\n",
                "    refers oracle.basic.Trade qualified\n",
                "}\n"
            )
            .to_string(),
        )],
        &sigil_imports(),
    );
    let model = compile_clean(&compilation);
    let book = &model.packages[0].classes[0];
    assert_eq!(
        book.features[0].type_,
        TypeRef::Class {
            package: "oracle.basic".to_string(),
            name: "Trade".to_string(),
        }
    );
    assert_eq!(book.features[1].type_, book.features[0].type_);
}

#[test]
fn a_bare_name_collision_with_a_mox_class_is_the_ambiguous_type_error() {
    let compilation = compile_with_sigil(
        &[(
            MOX.to_string(),
            concat!(
                "package demo\n\n",
                "import sigil \"oracle/trade.rosetta\"\n\n",
                "class Trade { String x }\n\n",
                "class Book {\n",
                "    refers Trade ambiguous\n",
                "}\n"
            )
            .to_string(),
        )],
        &sigil_imports(),
    );
    assert!(compilation.model.is_none());
    let (_, diagnostic) = compilation
        .diagnostics
        .iter()
        .find(|(_, diagnostic)| diagnostic.message.contains("ambiguous type `Trade`"))
        .expect("the existing ambiguous-type error");
    assert!(
        diagnostic.message.contains("qualify as"),
        "{:?}",
        diagnostic.message
    );
}

#[test]
fn cross_package_mox_declarations_resolve_sigil_names_too() {
    let rosetta = "namespace oracle.shared\n\ntype Shared:\n    x int (1..1)\n";
    let compilation = compile_with_sigil(
        &[
            (
                "a.mox".to_string(),
                "package a\n\nimport sigil \"shared.rosetta\"\n\nclass Holder { refers Shared s }\n"
                    .to_string(),
            ),
            (
                "b.mox".to_string(),
                "package b\n\nclass Other { refers oracle.shared.Shared s }\n".to_string(),
            ),
        ],
        &rex_driver::SigilImports::new().provide("a.mox", "shared.rosetta", rosetta),
    );
    assert!(
        compilation.diagnostics.is_empty() && compilation.sigil_diagnostics.is_empty(),
        "sigil namespaces join the whole compilation's union namespace: {:?}",
        compilation.diagnostics
    );
    let model = compilation.model.expect("model lowered");
    assert_eq!(model.packages.len(), 3);
    assert_eq!(model.packages[2].name, "oracle.shared");
}

#[test]
fn namespaces_order_by_first_appearance_across_import_declarations() {
    let alpha = "namespace oracle.alpha\n\ntype A:\n    x int (1..1)\n";
    let beta = concat!(
        "namespace oracle.beta\n\n",
        "import oracle.alpha.*\n\n",
        "type B:\n",
        "    a A (1..1)\n"
    );
    // `beta` is declared first but its closure pulls `alpha` in; the
    // first-appearance order is by the namespaces the named files declare.
    let compilation = compile_with_sigil(
        &[(
            MOX.to_string(),
            concat!(
                "package demo\n\n",
                "import sigil \"beta.rosetta\"\n\n",
                "class C { String x }\n"
            )
            .to_string(),
        )],
        &rex_driver::SigilImports::new()
            .provide(MOX, "beta.rosetta", beta)
            .provide(MOX, "alpha/alpha.rosetta", alpha),
    );
    let model = compile_clean(&compilation);
    assert_eq!(model.packages.len(), 3);
    assert_eq!(model.packages[1].name, "oracle.beta");
    assert_eq!(model.packages[2].name, "oracle.alpha");
}

// --- single-file pipeline ---------------------------------------------------------

#[test]
fn the_single_file_pipeline_switches_to_the_union_scope_with_sigil_namespaces() {
    let compilation = compile_str(
        MOX,
        &mox_with_sigil(),
        &rex_driver::DomainImports {
            schemas: rex_driver::SchemaImports::new(),
            sigil: sigil_imports(),
        },
    );
    assert!(
        compilation.diagnostics.is_empty() && compilation.sigil_diagnostics.is_empty(),
        "{:?} {:?}",
        compilation.diagnostics,
        compilation.sigil_diagnostics
    );
    let model = compilation.model.expect("model lowered");
    assert_eq!(model.packages.len(), 2);
    let book = &model.packages[0].classes[0];
    assert_eq!(
        book.features[0].type_,
        TypeRef::Class {
            package: "oracle.basic".to_string(),
            name: "Trade".to_string(),
        }
    );
}

// --- .actor union path ---------------------------------------------------------

#[test]
fn a_domain_with_a_sigil_import_errors_cleanly_in_the_actor_path() {
    // Sigil imports are not lowered in the `.actor`/`.ddd` union path yet;
    // declaring one is the not-provided error on the declaring domain.
    let domain = concat!(
        "package demo\n\n",
        "import sigil \"oracle/trade.rosetta\"\n\n",
        "class Wrapper { String x }\n"
    );
    let actor = concat!(
        "import \"demo.mox\"\n\n",
        "actors Ops {\n",
        "    actor Agent\n",
        "    capability Touch on Wrapper\n",
        "    grant Agent { permit Touch }\n",
        "}\n"
    );
    let compilation = rex_driver::compile_actors_str(
        "ops.actor",
        actor,
        &[("demo.mox".to_string(), domain.to_string())],
        &rex_driver::DomainImports::default(),
    );
    assert!(compilation.model.is_none());
    assert!(
        compilation
            .diagnostics
            .iter()
            .any(|(path, diagnostic)| path == "demo.mox"
                && diagnostic.message == "imported sigil 'oracle/trade.rosetta' was not provided"),
        "{:?}",
        compilation.diagnostics
    );
}

// --- the .actor and .ddd pipelines over sigil-importing domains ---------------

fn actor_source() -> String {
    concat!(
        "import \"demo.mox\"\n\n",
        "actors Ops {\n",
        "    actor Agent\n",
        "    capability TouchTrade on Trade\n",
        "    grant Agent {\n",
        "        permit TouchTrade when (id == \"T1\")\n",
        "    }\n",
        "}\n"
    )
    .to_string()
}

#[test]
fn actor_pipeline_resolves_sigil_domain_types() {
    // The capability targets a lowered rosetta class and its `when`
    // condition type-checks against a rosetta feature.
    let compilation = rex_driver::compile_actors_str(
        "ops.actor",
        &actor_source(),
        &[(MOX.to_string(), mox_with_sigil())],
        &rex_driver::DomainImports {
            schemas: rex_driver::SchemaImports::new(),
            sigil: sigil_imports(),
        },
    );
    assert!(
        compilation.diagnostics.is_empty(),
        "unexpected diagnostics: {:?}",
        compilation.diagnostics
    );
    let model = compilation.model.expect("actor model lowered");
    let block = &model.blocks[0];
    assert_eq!(block.capabilities[0].name, "TouchTrade");
    assert_eq!(
        block.capabilities[0].class,
        rex_ir::TypeRef::Class {
            package: "oracle.basic".to_string(),
            name: "Trade".to_string(),
        }
    );
    // The union model carries the synthetic packages for Cedar lookup.
    let domains_model = compilation.domains_model.expect("domains lowered");
    assert!(domains_model
        .packages
        .iter()
        .any(|package| package.name == "oracle.basic"));
}

#[test]
fn ddd_pipeline_designs_sigil_domain_types() {
    // A rosetta class with no containment into it is an aggregate root: a
    // repository is legal, and a search over its string feature type-checks.
    let design = concat!(
        "import \"demo.mox\"\n",
        "\n",
        "application Trading {\n",
        "    base oracle.basic\n",
        "\n",
        "    module trade {\n",
        "        entity Trade repository TradeRepository {\n",
        "            findAll;\n",
        "            Trade findById(String id);\n",
        "        }\n",
        "        search TradeSearch {\n",
        "            entity Trade\n",
        "            text {\n",
        "                id\n",
        "            }\n",
        "        }\n",
        "    }\n",
        "}\n",
    );
    let compilation = rex_driver::compile_ddd_str(
        "design.ddd",
        design,
        &[(MOX.to_string(), mox_with_sigil())],
        &rex_driver::DomainImports {
            schemas: rex_driver::SchemaImports::new(),
            sigil: sigil_imports(),
        },
    );
    assert!(
        compilation.diagnostics.is_empty(),
        "unexpected diagnostics: {:?}",
        compilation.diagnostics
    );
    let model = compilation.model.expect("design lowered");
    let design = &model.modules[0].designs[0];
    assert_eq!(design.class, "Trade");
    assert!(design.repository.is_some());
    // The union model carries the synthetic packages for consumers.
    let domains_model = compilation.domains_model.expect("domains lowered");
    assert!(domains_model
        .packages
        .iter()
        .any(|package| package.name == "oracle.basic"));
}

#[test]
fn missing_sigil_content_blocks_the_actor_domain() {
    let compilation = rex_driver::compile_actors_str(
        "ops.actor",
        &actor_source(),
        &[(MOX.to_string(), mox_with_sigil())],
        &rex_driver::DomainImports::default(),
    );
    assert!(
        compilation
            .diagnostics
            .iter()
            .any(|(path, diagnostic)| path == MOX
                && diagnostic
                    .message
                    .contains("imported sigil 'oracle/trade.rosetta' was not provided")),
        "the not-provided error names the domain: {:?}",
        compilation.diagnostics
    );
    assert!(compilation.model.is_none());
    assert!(compilation.domains_model.is_none());
}

#[test]
fn actor_pipeline_resolves_declared_schema_and_sigil_names_from_one_domain() {
    // The model is the single source of the domain's kind registry: one
    // domain file contributes its declared class (Wrapper), its
    // `import schema` nominal class (TodoItem), and a synthetic sigil
    // package (oracle.basic.Trade) — capabilities on all three resolve.
    let domain = concat!(
        "package demo\n\n",
        "import schema \"schemas/todo_item.json\" as TodoItem\n",
        "import sigil \"oracle/trade.rosetta\"\n\n",
        "class Wrapper { String x }\n"
    );
    let actor = concat!(
        "import \"demo.mox\"\n\n",
        "actors Ops {\n",
        "    actor Agent\n",
        "    capability TouchWrapper on Wrapper\n",
        "    capability TouchItem on TodoItem\n",
        "    capability TouchTrade on Trade\n",
        "    grant Agent { permit TouchWrapper }\n",
        "    grant Agent { permit TouchItem }\n",
        "    grant Agent { permit TouchTrade }\n",
        "}\n"
    );
    let compilation = rex_driver::compile_actors_str(
        "ops.actor",
        actor,
        &[(MOX.to_string(), domain.to_string())],
        &rex_driver::DomainImports {
            schemas: rex_driver::SchemaImports::new().provide(
                MOX,
                "schemas/todo_item.json",
                "{\"title\": \"Todo item\"}",
            ),
            sigil: sigil_imports(),
        },
    );
    assert!(
        compilation.diagnostics.is_empty(),
        "unexpected diagnostics: {:?}",
        compilation.diagnostics
    );
    let model = compilation.model.expect("actor model lowered");
    let block = &model.blocks[0];
    assert_eq!(
        block.capabilities[0].class,
        TypeRef::Class {
            package: "demo".to_string(),
            name: "Wrapper".to_string(),
        }
    );
    assert_eq!(
        block.capabilities[1].class,
        TypeRef::Class {
            package: "demo".to_string(),
            name: "TodoItem".to_string(),
        }
    );
    assert_eq!(
        block.capabilities[2].class,
        TypeRef::Class {
            package: "oracle.basic".to_string(),
            name: "Trade".to_string(),
        }
    );
}

#[test]
fn sigil_content_errors_surface_tagged_with_the_rosetta_path() {
    // A rosetta attribute typed by an unknown name fails resolution; the
    // diagnostic is keyed by the rosetta path and blocks the artifact.
    let broken = concat!(
        "namespace oracle.basic\n\n",
        "type Trade:\n",
        "    id bogus (1..1)\n"
    );
    let compilation = rex_driver::compile_ddd_str(
        "design.ddd",
        concat!(
            "import \"demo.mox\"\n",
            "\n",
            "application Trading {\n",
            "    module trade {\n",
            "        entity Trade\n",
            "    }\n",
            "}\n",
        ),
        &[(MOX.to_string(), mox_with_sigil())],
        &rex_driver::DomainImports {
            schemas: rex_driver::SchemaImports::new(),
            sigil: rex_driver::SigilImports::new().provide(MOX, TRADE, broken),
        },
    );
    assert!(
        compilation
            .diagnostics
            .iter()
            .any(|(path, _)| path == TRADE),
        "sigil diagnostics are keyed by the rosetta path: {:?}",
        compilation.diagnostics
    );
    assert!(compilation.model.is_none());
    assert!(compilation.domains_model.is_none());
}

// --- per-namespace blocking -------------------------------------------------------

/// The two-domain actor harness for the blocking tests: `a.mox` and
/// `b.mox` are compiled through the `.actor` union path, and the actor
/// file's `capability ... on <target>` must resolve against whatever
/// domains survived. `target` names the capability's class (a plain alpha
/// class, or a rosetta class lowered through `a.mox`'s import).
fn two_domain_actor(
    target: &str,
    sigil: &rex_driver::SigilImports,
) -> rex_driver::ActorCompilation {
    let domain_a = concat!(
        "package alpha\n\n",
        "import sigil \"good.rosetta\"\n\n",
        "class Wrapper { String x }\n"
    );
    let domain_b = concat!(
        "package beta\n\n",
        "import sigil \"bad.rosetta\"\n\n",
        "class Other { String x }\n"
    );
    let actor = format!(
        concat!(
            "import \"a.mox\"\n",
            "import \"b.mox\"\n\n",
            "actors Ops {{\n",
            "    actor Agent\n",
            "    capability Touch on {target}\n",
            "    grant Agent {{ permit Touch }}\n",
            "}}\n"
        ),
        target = target
    );
    rex_driver::compile_actors_str(
        "ops.actor",
        &actor,
        &[
            ("a.mox".to_string(), domain_a.to_string()),
            ("b.mox".to_string(), domain_b.to_string()),
        ],
        &rex_driver::DomainImports {
            schemas: rex_driver::SchemaImports::new(),
            sigil: sigil.clone(),
        },
    )
}

fn good_rosetta() -> &'static str {
    "namespace oracle.good\n\ntype Fine:\n    x int (1..1)\n"
}

/// The only error must be the failing namespace's, keyed by its rosetta
/// path: no fallout in the surviving domain (`a.mox`) or the actor file
/// (`capability on <target>` resolved, so the model survived), and the
/// declaring domain's own model dropped (the domain union is incomplete).
fn assert_only_the_rosetta_error_remains(
    compilation: &rex_driver::ActorCompilation,
    key: &str,
    needle: &str,
) {
    assert!(
        compilation.model.is_some(),
        "the actor artifact depends on nothing that failed: {:?}",
        compilation.diagnostics
    );
    assert!(
        compilation.domains_model.is_none(),
        "the declaring domain's model must have dropped"
    );
    let errors: Vec<&(String, rex_driver::Diagnostic)> = compilation
        .diagnostics
        .iter()
        .filter(|(_, diagnostic)| diagnostic.is_error())
        .collect();
    assert_eq!(errors.len(), 1, "{:?}", compilation.diagnostics);
    assert_eq!(errors[0].0, key, "keyed by the rosetta path");
    assert!(
        errors[0].1.message.contains(needle),
        "{:?}",
        errors[0].1.message
    );
    assert!(
        compilation
            .diagnostics
            .iter()
            .all(|(path, _)| path != "a.mox" && path != "ops.actor"),
        "no fallout in the surviving domain or the actor file: {:?}",
        compilation.diagnostics
    );
}

#[test]
fn a_failing_rosetta_namespace_blocks_only_its_declaring_file() {
    // beta's rosetta fails resolution; alpha's namespace lowers cleanly.
    // The capability targets alpha's own class: alpha's model must have
    // survived (under the old coarse blocking it dropped too, and the
    // capability resolution would add a second error).
    let compilation = two_domain_actor(
        "Wrapper",
        &rex_driver::SigilImports::new()
            .provide("a.mox", "good.rosetta", good_rosetta())
            .provide(
                "b.mox",
                "bad.rosetta",
                "namespace oracle.bad\n\ntype Broken:\n    who NoSuchType (1..1)\n",
            ),
    );
    assert_only_the_rosetta_error_remains(&compilation, "bad.rosetta", "NoSuchType");
}

#[test]
fn the_surviving_domains_synthetic_packages_stay_resolvable() {
    // As above, but the capability targets the rosetta class itself: the
    // successful namespace's synthetic package must have joined the union
    // namespace through the surviving domain's model.
    let compilation = two_domain_actor(
        "Fine",
        &rex_driver::SigilImports::new()
            .provide("a.mox", "good.rosetta", good_rosetta())
            .provide(
                "b.mox",
                "bad.rosetta",
                "namespace oracle.bad\n\ntype Broken:\n    who NoSuchType (1..1)\n",
            ),
    );
    assert_only_the_rosetta_error_remains(&compilation, "bad.rosetta", "NoSuchType");
}

#[test]
fn an_unparseable_rosetta_file_blocks_only_its_declaring_file() {
    // A parse failure names no namespace: only beta's model drops.
    let compilation = two_domain_actor(
        "Wrapper",
        &rex_driver::SigilImports::new()
            .provide("a.mox", "good.rosetta", good_rosetta())
            .provide(
                "b.mox",
                "bad.rosetta",
                "namespace oracle.bad\n\ntype Broken:\n    who (1..1)\n",
            ),
    );
    assert_only_the_rosetta_error_remains(&compilation, "bad.rosetta", "expected");
}

#[test]
fn a_domain_without_sigil_imports_survives_another_domains_sigil_failure() {
    // alpha declares no sigil import at all; beta's rosetta fails. alpha's
    // model must survive (the old coarse blocking dropped every domain).
    let domain_a = "package alpha\n\nclass Wrapper { String x }\n".to_string();
    let domain_b = concat!(
        "package beta\n\n",
        "import sigil \"bad.rosetta\"\n\n",
        "class Other { String x }\n"
    )
    .to_string();
    let actor = concat!(
        "import \"a.mox\"\n",
        "import \"b.mox\"\n\n",
        "actors Ops {\n",
        "    actor Agent\n",
        "    capability Touch on Wrapper\n",
        "    grant Agent { permit Touch }\n",
        "}\n"
    );
    let compilation = rex_driver::compile_actors_str(
        "ops.actor",
        actor,
        &[
            ("a.mox".to_string(), domain_a),
            ("b.mox".to_string(), domain_b),
        ],
        &rex_driver::DomainImports {
            schemas: rex_driver::SchemaImports::new(),
            sigil: rex_driver::SigilImports::new().provide(
                "b.mox",
                "bad.rosetta",
                "namespace oracle.bad\n\ntype Broken:\n    who NoSuchType (1..1)\n",
            ),
        },
    );
    assert_only_the_rosetta_error_remains(&compilation, "bad.rosetta", "NoSuchType");
}

#[test]
fn references_into_a_failed_namespace_error_on_the_referencing_file() {
    // The failed namespace's names stay out of the union resolution
    // namespace: beta's reference to its own (failed) rosetta type errors
    // on beta, while alpha and the actor file stay clean.
    let domain_a = concat!(
        "package alpha\n\n",
        "import sigil \"good.rosetta\"\n\n",
        "class Wrapper { String x }\n"
    )
    .to_string();
    let domain_b = concat!(
        "package beta\n\n",
        "import sigil \"bad.rosetta\"\n\n",
        "class Other { refers oracle.bad.Broken x }\n"
    )
    .to_string();
    let actor = concat!(
        "import \"a.mox\"\n",
        "import \"b.mox\"\n\n",
        "actors Ops {\n",
        "    actor Agent\n",
        "    capability Touch on Wrapper\n",
        "    grant Agent { permit Touch }\n",
        "}\n"
    );
    let compilation = rex_driver::compile_actors_str(
        "ops.actor",
        actor,
        &[
            ("a.mox".to_string(), domain_a),
            ("b.mox".to_string(), domain_b),
        ],
        &rex_driver::DomainImports {
            schemas: rex_driver::SchemaImports::new(),
            sigil: rex_driver::SigilImports::new()
                .provide("a.mox", "good.rosetta", good_rosetta())
                .provide(
                    "b.mox",
                    "bad.rosetta",
                    "namespace oracle.bad\n\ntype Broken:\n    who NoSuchType (1..1)\n",
                ),
        },
    );
    assert!(compilation.model.is_none());
    // beta's own reference error, tagged with its file; the sigil content
    // error, keyed by the rosetta path; nothing else.
    assert!(compilation.diagnostics.iter().any(|(path, diagnostic)| {
        path == "b.mox"
            && diagnostic.is_error()
            && diagnostic
                .message
                .contains("unknown type 'oracle.bad.Broken'")
    }));
    assert!(compilation
        .diagnostics
        .iter()
        .any(|(path, _)| path == "bad.rosetta"));
    assert!(compilation
        .diagnostics
        .iter()
        .all(|(path, _)| path != "a.mox" && path != "ops.actor"));
}
