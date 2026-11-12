//! Lowering-rule tests for the Rust backend's `expr` bodies: the typed
//! expression tree (`rex_expr`) is lowered to exact Rust source text, and
//! every test here pins one rule of `docs/EXPRESSIONS.md` (R1–R4, R9, L1/L2/U1,
//! A1–A6, feature access). The scratch-crate end-to-end test then proves the
//! pinned text compiles and evaluates.

use rex_backend_rust::expr_lower::{lower_expr, LowerCtx};
use rex_expr::{parse, Ty, TypeChecker, TypeContext};
use rex_ir::{
    ClassDef, Feature, FeatureKind, Model, Multiplicity, Operation, OperationParam, Package,
    PrimitiveType, TypeRef,
};

const PKG: &str = "nz.example.library";

fn class_ref(name: &str) -> TypeRef {
    TypeRef::Class {
        package: PKG.to_string(),
        name: name.to_string(),
    }
}

/// The IR equivalent of the library example, extended with the operations and
/// attributes the lowering tests reference: `Library.findBook(String) -> Book`,
/// `Writer.penName() -> String`, and `Book.downloads: long`.
fn library_model() -> Model {
    let mut model = Model::new();
    let mut package = Package::new(PKG);
    package
        .datatypes
        .push(rex_ir::DatatypeDef::new("Date", None).bind("rust", "chrono::NaiveDate"));
    // The L3 enum-literal namespace's registry input: the enum and one
    // enum-typed feature to compare against.
    package.enums.push(rex_ir::EnumDef::new(
        "BookCategory",
        vec![
            rex_ir::EnumLiteral::new("Mystery", None, 0),
            rex_ir::EnumLiteral::new("ScienceFiction", None, 1),
        ],
    ));
    package.classes.push(ClassDef::new(
        "Library",
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
                FeatureKind::Containment,
                class_ref("Book"),
                Multiplicity::MANY,
            )
            .with_opposite("Book", "library"),
        ],
    ));
    package.classes[0].operations.push(Operation::new(
        "findBook",
        class_ref("Book"),
        vec![OperationParam {
            name: "title".to_string(),
            type_: TypeRef::Primitive(PrimitiveType::String),
            multiplicity: None,
        }],
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
                "synopsis",
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
                "downloads",
                FeatureKind::Attribute,
                TypeRef::Primitive(PrimitiveType::Long),
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
            .with_body("expr", "title"),
        ],
    ));
    package.classes.push(ClassDef::new(
        "Writer",
        vec![],
        vec![Feature::new(
            "books",
            FeatureKind::CrossReference,
            class_ref("Book"),
            Multiplicity::MANY,
        )
        .with_opposite("Book", "authors")],
    ));
    package.classes[2].operations.push(Operation::new(
        "penName",
        TypeRef::Primitive(PrimitiveType::String),
        vec![],
    ));
    // A sigil-lowered package function (the `rex_ir::Package::functions`
    // shape wire contract rule 15 lowers Rune declarations to): long return,
    // a required long param, and a many string param.
    package.functions.push(Operation::new(
        "priceQuote",
        TypeRef::Primitive(PrimitiveType::Long),
        vec![
            OperationParam {
                name: "notional".to_string(),
                type_: TypeRef::Primitive(PrimitiveType::Long),
                multiplicity: None,
            },
            OperationParam {
                name: "legIds".to_string(),
                type_: TypeRef::Primitive(PrimitiveType::String),
                multiplicity: Some(Multiplicity::MANY),
            },
        ],
    ));
    model.packages.push(package);
    model
}

/// Lowers `source` in the given self class and asserts the exact Rust text.
/// The `title: String` operation parameter is in scope only on `Library`
/// (the `op Book findBook(String title)` context the tests model).
fn lower_in(class: &str, source: &str, expected: &str) {
    let model = library_model();
    let params: &[OperationParam] = if class == "Library" {
        &[OperationParam {
            name: "title".to_string(),
            type_: TypeRef::Primitive(PrimitiveType::String),
            multiplicity: None,
        }]
    } else {
        &[]
    };
    let ctx = LowerCtx::new(&model, PKG, class, params);
    let parsed = parse(source);
    assert!(parsed.errors.is_empty(), "{source:?}: {:?}", parsed.errors);
    let mut checker = TypeChecker::new(TypeContext::from_model(&model)).with_self(PKG, class);
    for param in params {
        checker = checker.with_binding(&param.name, Ty::string());
    }
    let ty = checker
        .type_of(parsed.ast.as_ref().expect("well-formed ast"))
        .unwrap_or_else(|errors| panic!("{source:?}: {errors:?}"));
    let code = lower_expr(parsed.ast.as_ref().expect("ast"), &ty, &ctx)
        .unwrap_or_else(|error| panic!("{source:?}: {error}"));
    assert_eq!(code, expected, "lowering of {source:?}");
}

// ---------------------------------------------------------------------------
// Literals (with L1 literal width)
// ---------------------------------------------------------------------------

#[test]
fn int_literal_carries_the_i32_suffix() {
    lower_in("Library", "41", "41i32");
}

#[test]
fn long_literal_in_a_long_context_carries_i64() {
    // L1: `1` is typed long next to the `long` feature, so the suffix follows.
    lower_in("Book", "downloads + 1", "(self.downloads + 1i64)");
}

#[test]
fn string_literal_is_the_escaped_rust_str_to_string() {
    lower_in(
        "Library",
        "\"a \\\"quoted\\\" \\\\ backslash\"",
        "\"a \\\"quoted\\\" \\\\ backslash\".to_string()",
    );
}

#[test]
fn l3_enum_member_literal_lowers_to_the_variant_path() {
    // The dotted namespace member lowers to the same `Enum::Member` shape
    // the codegen emits for enum defaults; the comparison is the plain
    // same-type `==` of R2.
    lower_in(
        "Book",
        "category == BookCategory.Mystery",
        "(self.category == BookCategory::Mystery)",
    );
    lower_in(
        "Book",
        "BookCategory.ScienceFiction != category",
        "(BookCategory::ScienceFiction != self.category)",
    );
    lower_in("Book", "BookCategory.Mystery", "BookCategory::Mystery");
}

#[test]
fn boolean_literals_lower_to_rust_bools() {
    lower_in("Library", "true", "true");
    lower_in("Library", "false", "false");
}

// ---------------------------------------------------------------------------
// Names: self features, parameters, lambda parameters (the `b.` sugar)
// ---------------------------------------------------------------------------

#[test]
fn self_feature_reads_the_struct_field_and_clones_values() {
    lower_in("Book", "title", "self.title.clone()");
    // Copy-shaped fields (ids, ints, enums) need no clone.
    lower_in("Book", "pages", "self.pages");
    lower_in("Book", "library", "self.library");
    // Datatypes are Clone-only newtypes.
    lower_in("Book", "copyright", "self.copyright.clone()");
}

#[test]
fn operation_parameter_is_the_rust_parameter_cloned_when_owned() {
    lower_in("Library", "title", "title.clone()");
}

// ---------------------------------------------------------------------------
// Feature access through `res` (ids are the runtime representation)
// ---------------------------------------------------------------------------

#[test]
fn feature_access_shapes_are_pinned_through_map() {
    // Title: String (owned) — `b.title` reads through `res` and clones.
    lower_in(
        "Library",
        "books.map(b => b.title)",
        "self.books.clone().iter().map(|b| res.book(*b).map(|o| o.title.clone()).expect(\"dangling `Book` id\")).collect::<Vec<_>>()",
    );
}

#[test]
fn copy_valued_feature_access_needs_no_clone() {
    lower_in(
        "Library",
        "books.any(b => b.pages == 0)",
        "self.books.clone().iter().any(|b| (res.book(*b).map(|o| o.pages).expect(\"dangling `Book` id\") == 0i32))",
    );
}

#[test]
fn container_feature_access_yields_the_optional_id_field() {
    lower_in(
        "Library",
        "books.map(b => b.library)",
        "self.books.clone().iter().map(|b| res.book(*b).map(|o| o.library).expect(\"dangling `Book` id\")).collect::<Vec<_>>()",
    );
}

#[test]
fn safe_navigation_chains_and_then_map() {
    // `a?.b` propagates None: and_then over the optional receiver, then map
    // (or and_then when the feature itself is optional-valued).
    lower_in(
        "Library",
        "books.map(b => b.library?.name)",
        "self.books.clone().iter().map(|b| res.book(*b).map(|o| o.library).expect(\"dangling `Book` id\").and_then(|id| res.library(id)).map(|o| o.name.clone())).collect::<Vec<_>>()",
    );
}

// ---------------------------------------------------------------------------
// A1 first — Option of the element
// ---------------------------------------------------------------------------

#[test]
fn a1_first_with_predicate_uses_find_copied_for_copy_elements() {
    lower_in(
        "Library",
        "books.first(b => b.title == title)",
        "self.books.clone().iter().find(|b| (res.book(**b).map(|o| o.title.clone()).expect(\"dangling `Book` id\") == title.clone())).copied()",
    );
}

#[test]
fn a1_first_without_predicate_is_first_copied() {
    lower_in(
        "Library",
        "books.first()",
        "self.books.clone().first().copied()",
    );
}

#[test]
fn a1_first_over_non_copy_elements_clones_the_match_only() {
    lower_in(
        "Book",
        "[title].first(t => t == \"x\")",
        "vec![self.title.clone()].iter().find(|t| ((**t).clone() == \"x\")).cloned()",
    );
}

// ---------------------------------------------------------------------------
// A2 filter / A3 map / A4 any / A5 size / A6 sum
// ---------------------------------------------------------------------------

#[test]
fn a2_filter_keeps_order_and_collects() {
    lower_in(
        "Library",
        "books.filter(b => b.pages > 300)",
        "self.books.clone().iter().filter(|b| (res.book(**b).map(|o| o.pages).expect(\"dangling `Book` id\") > 300i32)).copied().collect::<Vec<_>>()",
    );
}

#[test]
fn a3_map_projects_through_res() {
    lower_in(
        "Library",
        "books.map(b => b.title)",
        "self.books.clone().iter().map(|b| res.book(*b).map(|o| o.title.clone()).expect(\"dangling `Book` id\")).collect::<Vec<_>>()",
    );
}

#[test]
fn a4_any_folds_to_a_bool() {
    lower_in(
        "Library",
        "books.any(b => b.pages > 300)",
        "self.books.clone().iter().any(|b| (res.book(*b).map(|o| o.pages).expect(\"dangling `Book` id\") > 300i32))",
    );
}

#[test]
fn a4_all_folds_to_a_bool() {
    lower_in(
        "Library",
        "books.all(b => b.pages > 300)",
        "self.books.clone().iter().all(|b| (res.book(*b).map(|o| o.pages).expect(\"dangling `Book` id\") > 300i32))",
    );
}

#[test]
fn a5_size_is_the_len_cast_to_i32() {
    lower_in("Library", "books.size()", "self.books.clone().len() as i32");
}

#[test]
fn a6_sum_borrows_and_annotates_the_element_type() {
    // A6 over a projection: map to ints, then sum with the width annotation.
    lower_in(
        "Library",
        "books.map(b => b.pages).sum()",
        "self.books.clone().iter().map(|b| res.book(*b).map(|o| o.pages).expect(\"dangling `Book` id\")).collect::<Vec<_>>().iter().sum::<i32>()",
    );
}

#[test]
fn a6_sum_over_long_elements_annotates_i64() {
    // downloads is `long`; L1 types the projection long.
    lower_in(
        "Book",
        "[downloads].sum()",
        "vec![self.downloads].iter().sum::<i64>()",
    );
}

// ---------------------------------------------------------------------------
// R2 equality — value equality on strings, null-safe, element-wise on Option
// ---------------------------------------------------------------------------

#[test]
fn r2_string_equality_is_plain_rust_eq_with_borrowed_literals() {
    lower_in(
        "Library",
        "books.first(b => b.title == \"Dune\")",
        "self.books.clone().iter().find(|b| (res.book(**b).map(|o| o.title.clone()).expect(\"dangling `Book` id\") == \"Dune\")).copied()",
    );
}

#[test]
fn r2_option_equality_is_element_wise_via_partialeq() {
    lower_in(
        "Library",
        "books.first(b => b.library == b.library)",
        "self.books.clone().iter().find(|b| (res.book(**b).map(|o| o.library).expect(\"dangling `Book` id\") == res.book(**b).map(|o| o.library).expect(\"dangling `Book` id\"))).copied()",
    );
}

#[test]
fn r2_null_equality_is_is_none() {
    lower_in(
        "Library",
        "books.first(b => b.library == null)",
        "self.books.clone().iter().find(|b| res.book(**b).map(|o| o.library).expect(\"dangling `Book` id\").is_none()).copied()",
    );
    lower_in(
        "Library",
        "books.first(b => b.library != null)",
        "self.books.clone().iter().find(|b| !(res.book(**b).map(|o| o.library).expect(\"dangling `Book` id\").is_none())).copied()",
    );
}

#[test]
fn r2_null_against_a_plain_value_is_the_constant_answer() {
    // A required String can never be null: `title == null` is false, `!=` true.
    lower_in("Book", "title == null", "false");
    lower_in("Book", "title != null", "true");
    lower_in("Book", "null == null", "true");
}

// ---------------------------------------------------------------------------
// R1 arithmetic / R4 division — plain Rust operators (panic = the contract)
// ---------------------------------------------------------------------------

#[test]
fn r1_arithmetic_is_plain_and_width_typed() {
    lower_in("Book", "pages * 2", "(self.pages * 2i32)");
    lower_in("Book", "downloads * 2", "(self.downloads * 2i64)");
    lower_in("Book", "pages - 1", "(self.pages - 1i32)");
}

#[test]
fn r4_division_is_plain_truncating_slash() {
    lower_in("Book", "pages / 2", "(self.pages / 2i32)");
}

// ---------------------------------------------------------------------------
// R9 string concatenation — `format!` on the owned `String` operands
// ---------------------------------------------------------------------------

#[test]
fn r9_string_concatenation_lowers_to_format() {
    // Left-associative chain: every binary node lowers to one `format!`, so
    // nesting is safe and no operand ordering constraint from Rust's
    // consuming `Add` (which `String + String` does not even implement)
    // can surface.
    lower_in(
        "Book",
        r#"title + " - " + synopsis"#,
        "format!(\"{}{}\", format!(\"{}{}\", self.title.clone(), \" - \".to_string()), \
         self.synopsis.clone())",
    );
}

#[test]
fn r9_string_literal_operand_concatenates_through_format() {
    lower_in(
        "Library",
        r#""Book: " + name"#,
        "format!(\"{}{}\", \"Book: \".to_string(), self.name.clone())",
    );
}

#[test]
fn r9_optional_string_concatenation_propagates_absence_via_zip() {
    // R3: an absent operand yields an absent result. `citation` is an
    // optional derived String; the present side rides along in `Some`.
    lower_in(
        "Book",
        "citation + title",
        "self.citation(res).zip(Some(self.title.clone())).map(|(concat_lhs, concat_rhs)| \
         format!(\"{concat_lhs}{concat_rhs}\"))",
    );
}

#[test]
fn r9_concatenation_lowering_compiles_inside_filter_lambdas() {
    // The `format!` operands are owned locals, so the lowering is
    // capture-agnostic inside collection-algebra closures.
    lower_in(
        "Library",
        "books.filter(b => b.title + b.title == name)",
        r#"self.books.clone().iter().filter(|b| (format!("{}{}", res.book(**b).map(|o| o.title.clone()).expect("dangling `Book` id"), res.book(**b).map(|o| o.title.clone()).expect("dangling `Book` id")) == self.name.clone())).copied().collect::<Vec<_>>()"#,
    );
}

// ---------------------------------------------------------------------------
// Logic, unary, U1 if, let, list literals
// ---------------------------------------------------------------------------

#[test]
fn logic_and_unary_operators_map_one_to_one() {
    // Every binary node is parenthesized, so precedence is safe by
    // construction.
    lower_in(
        "Book",
        "pages > 1 && pages < 10 || !(title == null)",
        "(((self.pages > 1i32) && (self.pages < 10i32)) || !(false))",
    );
    lower_in("Book", "-pages", "-(self.pages)");
}

#[test]
fn u1_if_lowers_to_a_rust_if_expression() {
    lower_in(
        "Book",
        "if pages > 300 { title } else { \"short\" }",
        "if (self.pages > 300i32) { self.title.clone() } else { \"short\".to_string() }",
    );
}

#[test]
fn let_lowers_to_a_scoped_rust_block() {
    lower_in(
        "Library",
        "let long = books.filter(b => b.pages > 300); long.size()",
        "{ let long = self.books.clone().iter().filter(|b| (res.book(**b).map(|o| o.pages).expect(\"dangling `Book` id\") > 300i32)).copied().collect::<Vec<_>>(); long.clone().len() as i32 }",
    );
}

#[test]
fn list_literals_lower_to_vec() {
    lower_in("Book", "[1, 2]", "vec![1i32, 2i32]");
    lower_in("Book", "[title]", "vec![self.title.clone()]");
}

// ---------------------------------------------------------------------------
// Operation calls
// ---------------------------------------------------------------------------

#[test]
fn operation_call_on_a_safe_receiver_maps_over_the_lookup() {
    lower_in(
        "Library",
        "books.map(b => b.library?.findBook(title))",
        "self.books.clone().iter().map(|b| res.book(*b).map(|o| o.library).expect(\"dangling `Book` id\").and_then(|id| res.library(id)).map(|o| o.find_book(title.clone()))).collect::<Vec<_>>()",
    );
}

#[test]
fn operation_call_on_a_plain_receiver_expects_the_lookup() {
    lower_in(
        "Book",
        "authors.first(w => w.penName() == \"x\")",
        "self.authors.clone().iter().find(|w| (res.writer(**w).expect(\"dangling `Writer` id\").pen_name() == \"x\")).copied()",
    );
}

// ---------------------------------------------------------------------------
// Derived features: access through the generated accessor
// ---------------------------------------------------------------------------

#[test]
fn derived_feature_access_calls_the_generated_accessor() {
    // Through `res` (citation types as Option(String): map, not and_then —
    // the accessor result is already the feature's optional shape).
    lower_in(
        "Library",
        "books.map(b => b.citation)",
        "self.books.clone().iter().map(|b| res.book(*b).map(|o| o.citation(res)).expect(\"dangling `Book` id\")).collect::<Vec<_>>()",
    );
    // On the implicit self: a direct accessor call.
    lower_in("Book", "citation", "self.citation(res)");
}

// ---------------------------------------------------------------------------
// R3 coalesce
// ---------------------------------------------------------------------------

#[test]
fn r3_coalesce_is_unwrap_or_with_an_eager_default() {
    lower_in(
        "Library",
        "books.map(b => b.library?.name ?: \"anon\")",
        "self.books.clone().iter().map(|b| res.book(*b).map(|o| o.library).expect(\"dangling `Book` id\").and_then(|id| res.library(id)).map(|o| o.name.clone()).unwrap_or(\"anon\".to_string())).collect::<Vec<_>>()",
    );
}

// ---------------------------------------------------------------------------
// Defensive edges
// ---------------------------------------------------------------------------

#[test]
fn safe_algebra_wraps_a_non_optional_result() {
    // `books?.size()` — the receiver is never optional, so `?.` just wraps.
    lower_in(
        "Library",
        "books?.size()",
        "Some(self.books.clone().len() as i32)",
    );
}

#[test]
fn unknown_names_are_lower_errors() {
    let model = library_model();
    let ctx = LowerCtx::new(&model, PKG, "Library", &[]);
    let parsed = parse("nope");
    let expr = parsed.ast.expect("ast");
    let error = lower_expr(&expr, &Ty::string(), &ctx).expect_err("unknown name");
    assert!(error.to_string().contains("nope"), "{error}");
}

// ---------------------------------------------------------------------------
// Date primitives and the calendar algebra (issue #9, spec R5–R8)
// ---------------------------------------------------------------------------

/// A `Loan` model with a required date, an optional date, a many-valued
/// date, and a `long` attribute, for the date lowering tests.
fn loan_model() -> Model {
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
                "dueDate",
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
            Feature::new(
                "downloads",
                FeatureKind::Attribute,
                TypeRef::Primitive(PrimitiveType::Long),
                Multiplicity::REQUIRED,
            ),
        ],
    ));
    model.packages.push(package);
    model
}

/// Lowers `source` on `Loan` and asserts the exact Rust text.
fn lower_on_loan(source: &str, expected: &str) {
    let model = loan_model();
    let ctx = LowerCtx::new(&model, PKG, "Loan", &[]);
    let parsed = parse(source);
    assert!(parsed.errors.is_empty(), "{source:?}: {:?}", parsed.errors);
    let checker = TypeChecker::new(TypeContext::from_model(&model)).with_self(PKG, "Loan");
    let ty = checker
        .type_of(parsed.ast.as_ref().expect("well-formed ast"))
        .unwrap_or_else(|errors| panic!("{source:?}: {errors:?}"));
    let code = lower_expr(parsed.ast.as_ref().expect("ast"), &ty, &ctx)
        .unwrap_or_else(|error| panic!("{source:?}: {error}"));
    assert_eq!(code, expected, "lowering of {source:?}");
}

#[test]
fn date_literal_lowers_to_a_checked_runtime_constructor() {
    lower_on_loan(
        r#"date("2026-09-17")"#,
        r#"rex_runtime::Date::from_str("2026-09-17").expect("R6-checked date literal")"#,
    );
}

#[test]
fn date_comparison_lowers_to_the_total_order() {
    // R7: ordering is plain `<` on the day-number newtype.
    lower_on_loan(
        r#"effectiveDate < date("2027-01-01")"#,
        "(self.effective_date < rex_runtime::Date::from_str(\"2027-01-01\").expect(\"R6-checked date literal\"))",
    );
    // R2: equality is plain value equality.
    lower_on_loan(
        "effectiveDate == dueDate",
        "(self.effective_date == self.due_date)",
    );
}

#[test]
fn calendar_algebra_lowers_to_runtime_methods() {
    // R8: plus_months; R5: plus_days; both Copy reads off self.
    lower_on_loan(
        "effectiveDate.plus_months(6)",
        "(self.effective_date.plus_months(6i32))",
    );
    lower_on_loan(
        "effectiveDate.plus_days(30)",
        "(self.effective_date.plus_days(30i32))",
    );
    lower_on_loan(
        r#"effectiveDate.diff_days(date("2027-03-17"))"#,
        "(self.effective_date.diff_days(rex_runtime::Date::from_str(\"2027-03-17\").expect(\"R6-checked date literal\")))",
    );
}

#[test]
fn optional_date_navigation_lowers_with_map() {
    // R3: `?.` over an optional date propagates None through the algebra.
    lower_on_loan(
        "maturityDate?.plus_days(1)",
        "(self.maturity_date.map(|__rex_date| __rex_date.plus_days(1i32)))",
    );
}

#[test]
fn date_collections_lower_with_the_generic_algebra() {
    // A2 over dates: the Vec receiver clones, the predicate sees `&&T`, and
    // Copy elements adapt with `.copied()`.
    lower_on_loan(
        r#"renewals.filter(d => d > date("2026-01-01"))"#,
        "self.renewals.clone().iter().filter(|d| (**d > rex_runtime::Date::from_str(\"2026-01-01\").expect(\"R6-checked date literal\"))).copied().collect::<Vec<_>>()",
    );
    // A3 to int results (`map` lends `&T`; the deref parenthesizes).
    lower_on_loan(
        "renewals.map(d => d.diff_days(effectiveDate))",
        "self.renewals.clone().iter().map(|d| ((*d).diff_days(self.effective_date))).collect::<Vec<_>>()",
    );
}

// ---------------------------------------------------------------------------
// Bare free-function calls (issue #63): the emitted stub's exact name
// ---------------------------------------------------------------------------

#[test]
fn a_bare_call_lowers_to_the_free_stub_call() {
    // The snake_case stub name `emit_functions` generates, an L1-adapted
    // int-literal argument (`notional: long`), and a many string argument.
    lower_in(
        "Book",
        "priceQuote(2026, [\"L1\"])",
        "price_quote(2026i64, vec![\"L1\".to_string()])",
    );
}

#[test]
fn an_ambiguous_bare_call_bails_naming_both_packages() {
    // The same function name declared by two packages: lowering names every
    // declaring package, mirroring the checker's ambiguity wording.
    let mut model = library_model();
    let mut rates = Package::new("sigil::nz.example.rates");
    rates.functions.push(Operation::new(
        "priceQuote",
        TypeRef::Primitive(PrimitiveType::Int),
        vec![],
    ));
    model.packages.push(rates);
    let parsed = parse(r#"priceQuote("a")"#);
    assert!(parsed.errors.is_empty(), "{:?}", parsed.errors);
    let ctx = LowerCtx::new(&model, PKG, "Book", &[]);
    let error =
        lower_expr(parsed.ast.as_ref().expect("ast"), &Ty::long(), &ctx).expect_err("ambiguous");
    assert_eq!(
        error.message,
        "ambiguous function `priceQuote`: declared in nz.example.library, \
         sigil::nz.example.rates"
    );
}

#[test]
fn an_interface_typed_function_call_bails_naming_the_function() {
    // Interfaces have no struct representation, so an interface-typed
    // function signature cannot be called in generated code.
    let mut model = library_model();
    model.packages[0]
        .interfaces
        .push(rex_ir::InterfaceDef::new("Lendable"));
    model.packages[0].functions.push(Operation::new(
        "latestQuote",
        TypeRef::Interface {
            package: PKG.to_string(),
            name: "Lendable".to_string(),
        },
        vec![],
    ));
    let parsed = parse("latestQuote()");
    assert!(parsed.errors.is_empty(), "{:?}", parsed.errors);
    let ctx = LowerCtx::new(&model, PKG, "Book", &[]);
    let error = lower_expr(parsed.ast.as_ref().expect("ast"), &Ty::Null, &ctx)
        .expect_err("interface-typed");
    assert_eq!(
        error.message,
        "function `latestQuote` has an interface-typed signature; interfaces \
         have no struct representation"
    );
}

#[test]
fn an_unknown_function_call_bails() {
    // The checker rejects the name before lowering; the lowering still
    // fails closed on its own resolution, so the tree is lowered directly.
    let model = library_model();
    let parsed = parse("noSuchFunction()");
    assert!(parsed.errors.is_empty(), "{:?}", parsed.errors);
    let ctx = LowerCtx::new(&model, PKG, "Book", &[]);
    let error = lower_expr(parsed.ast.as_ref().expect("ast"), &Ty::Null, &ctx)
        .expect_err("unknown function");
    assert_eq!(error.message, "unknown function `noSuchFunction`");
}
