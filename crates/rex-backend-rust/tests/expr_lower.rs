//! Lowering-rule tests for the Rust backend's `expr` bodies: the typed
//! expression tree (`rex_expr`) is lowered to exact Rust source text, and
//! every test here pins one rule of `docs/EXPRESSIONS.md` (R1–R4, L1/L2/U1,
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
