//! Type-checker tests for the expression sublanguage, against an inline
//! `nz.example.library` Core IR (the `examples/library.mox` shape used by
//! `rex-backend-rust`'s end-to-end tests, extended with a `long` attribute and
//! a second operation). Test names cite the rule numbers of
//! `docs/EXPRESSIONS.md` (R1–R4, L1–L2, U1, A1–A6).

use rex_expr::{parse, NamedKind, Ty, TypeChecker, TypeContext};
use rex_ir::{
    ClassDef, DatatypeDef, EnumDef, EnumLiteral, Feature, FeatureKind, Model, Multiplicity,
    Operation, OperationParam, Package, PrimitiveType, TypeRef,
};

const PKG: &str = "nz.example.library";

fn class_ref(name: &str) -> TypeRef {
    TypeRef::Class {
        package: PKG.to_string(),
        name: name.to_string(),
    }
}

/// The IR equivalent of `examples/library.mox` after driver lowering, plus the
/// expression-test extensions: `Book.downloads` (a REQUIRED `long` attribute)
/// and `Library.totalInCirculation(int) -> long`.
fn library_model() -> Model {
    let mut model = Model::new();
    let mut package = Package::new(PKG);
    package.enums.push(EnumDef::new(
        "BookCategory",
        vec![
            EnumLiteral::new("Mystery", None, 0),
            EnumLiteral::new("ScienceFiction", None, 1),
        ],
    ));
    package
        .datatypes
        .push(DatatypeDef::new("Date", None).bind("rust", "chrono::NaiveDate"));
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
    package.classes[0].operations.push(Operation::new(
        "totalInCirculation",
        TypeRef::Primitive(PrimitiveType::Long),
        vec![OperationParam {
            name: "year".to_string(),
            type_: TypeRef::Primitive(PrimitiveType::Int),
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

/// A checker with `book: Book` and `library: Library` bound.
fn checker() -> TypeChecker {
    TypeChecker::new(TypeContext::from_model(&library_model()))
        .with_binding("book", Ty::class(PKG, "Book"))
        .with_binding("library", Ty::class(PKG, "Library"))
        .with_binding("writer", Ty::class(PKG, "Writer"))
}

/// Parses and type-checks `source`, expecting the given type.
fn assert_ty(source: &str, expected: Ty) {
    let parsed = parse(source);
    assert!(parsed.errors.is_empty(), "{source:?}: {:?}", parsed.errors);
    let ty = checker()
        .type_of(parsed.ast.as_ref().expect("ast"))
        .unwrap_or_else(|errors| panic!("{source:?}: {errors:?}"));
    assert_eq!(ty, expected, "{source:?}");
}

/// Parses and type-checks `source`, expecting failure; every message must
/// mention `rule`.
fn assert_err(source: &str, rule: &str) -> Vec<rex_expr::ExprError> {
    let parsed = parse(source);
    assert!(parsed.errors.is_empty(), "{source:?}: {:?}", parsed.errors);
    let errors = checker()
        .type_of(parsed.ast.as_ref().expect("ast"))
        .expect_err("expected type errors");
    assert!(
        errors.iter().any(|e| e.message.contains(rule)),
        "{source:?}: expected a `{rule}` error, got {errors:?}"
    );
    errors
}

fn book() -> Ty {
    Ty::class(PKG, "Book")
}

// ---------------------------------------------------------------------------
// Feature typing mirrors the IR
// ---------------------------------------------------------------------------

#[test]
fn feature_access_types_mirror_the_ir() {
    assert_ty("book.pages", Ty::int()); // 1..1 attribute
    assert_ty("book.title", Ty::string()); // 1..1 attribute
    assert_ty("book.citation", Ty::string().optional()); // derived, 0..1
    assert_ty("book.downloads", Ty::long()); // 1..1 long attribute
    assert_ty(
        "book.copyright",
        Ty::named(NamedKind::Datatype, PKG, "Date"),
    );
    assert_ty(
        "book.category",
        Ty::named(NamedKind::Enum, PKG, "BookCategory"),
    );
    assert_ty("book.authors", Ty::class(PKG, "Writer").list()); // to-many refers
    assert_ty("library.books", book().list()); // to-many contains, empty ok
    assert_ty("book.library", Ty::class(PKG, "Library").optional()); // container
}

#[test]
fn unknown_feature_reports_the_name_span() {
    let parsed = parse("book.titulo");
    let errors = checker()
        .type_of(parsed.ast.as_ref().expect("ast"))
        .expect_err("unknown feature");
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].span, (5..11).into(), "span points at `titulo`");
}

#[test]
fn bare_names_must_be_bound_and_shadowing_scopes() {
    assert_ty("let x = book.pages; x", Ty::int());
    // unbound name
    let parsed = parse("book.pages + unbound");
    let errors = checker()
        .type_of(parsed.ast.as_ref().expect("ast"))
        .expect_err("unbound");
    assert_eq!(errors.len(), 1);
}

// ---------------------------------------------------------------------------
// R1 — INTEGER OVERFLOW
// ---------------------------------------------------------------------------

#[test]
fn r1_constant_overflow_is_a_type_error() {
    // 2147483647 + 1 overflows int at compile time -> type-check error.
    let errors = assert_err("2147483647 + 1", "R1");
    assert_eq!(errors.len(), 1);
    // ...and so do folded multiplication and subtraction chains.
    assert_err("2147483647 * 2", "R1");
    assert_err("-2147483647 - 2", "R1");
}

#[test]
fn r1_in_range_constants_type_check() {
    assert_ty("2147483646 + 1", Ty::int());
    assert_ty("10 / 2", Ty::int());
    assert_ty("-(0 - 2147483647)", Ty::int());
}

#[test]
fn r1_integer_literals_default_to_int() {
    // A lone literal is `int`; one that does not fit 32 bits is rejected.
    assert_ty("42", Ty::int());
    assert_err("2147483648", "R1");
    // ...unless L1 adapts it into a long context.
    assert_ty("book.downloads + 2147483648", Ty::long());
}

#[test]
fn r1_non_constant_overflow_is_a_runtime_panic_contract() {
    // The compile-time checker only folds constants; feature-typed operands
    // type-check normally (the panic contract is a backend obligation).
    assert_ty("book.pages + book.pages", Ty::int());
    assert_ty("book.downloads + book.downloads", Ty::long());
    // Mixed int/long non-literals are an error: no implicit conversion (L2).
    assert_err("book.pages + book.downloads", "L2");
}

// ---------------------------------------------------------------------------
// R2 — STRING EQUALITY
// ---------------------------------------------------------------------------

#[test]
fn r2_string_equality_is_value_equality() {
    assert_ty(r#"book.title == "ulysses""#, Ty::boolean());
    assert_ty(r#"book.title = "ulysses""#, Ty::boolean()); // `=` alias
    assert_ty(r#"book.title != "ulysses""#, Ty::boolean());
    // null-safe: null compares with anything
    assert_ty(r#"book.title == null"#, Ty::boolean());
    assert_ty("book.library == null", Ty::boolean());
    assert_ty("null == null", Ty::boolean());
    assert_ty("book == null", Ty::boolean());
}

#[test]
fn r2_option_equality_and_type_mismatches() {
    // Option<T> == element-wise; typing-wise Option compares with null.
    assert_ty("book.authors == null", Ty::boolean());
    // Same-type named comparisons are fine; distinct types are not.
    assert_ty("book.library == book.library", Ty::boolean());
    assert_err("book == writer", "R2");
    // Equality across primitive types is a mismatch too.
    assert_err(r#"book.pages == "many""#, "R2");
}

// ---------------------------------------------------------------------------
// R3 — OPTION/NULL PROPAGATION
// ---------------------------------------------------------------------------

#[test]
fn r3_unsafe_access_on_optional_receiver_is_a_type_error() {
    // book.library is Option(Library); `.name` on it is a type error.
    let errors = assert_err("book.library.name", "R3");
    assert_eq!(errors.len(), 1);
}

#[test]
fn r3_safe_navigation_types_as_option() {
    // `?.` propagates: the result is Option of what the inner access yields.
    // `book.library` is Option(Library); `name` is a REQUIRED string, so the
    // `?.` wraps it -> Option(string).
    assert_ty("book.library?.name", Ty::string().optional());
    // `books` is List(Book), not Option, so `?.` still wraps the whole access.
    assert_ty("book.library?.books", book().list().optional());
    // `?.` on a non-optional receiver simply wraps.
    assert_ty("book?.pages", Ty::int().optional());
    // chained: getBook returns Book; the `?.` hop keeps Option(Book)
    assert_ty(r#"book.library?.findBook("x")"#, book().optional());
}

#[test]
fn r3_coalesce_yields_the_inner_type() {
    assert_ty(r#"book.citation ?: "untitled""#, Ty::string());
    assert_ty("book.library?.name ?: \"main\"", Ty::string());
    assert_ty(r#"book.library ?: library"#, Ty::class(PKG, "Library"));
    // `?: null` stays the inner type (null acts as None).
    assert_ty("book.citation ?: null", Ty::string());
    // `?:` requires an optional (or null) left side.
    assert_err(r#"book.title ?: "x""#, "R3");
}

#[test]
fn r3_to_many_yields_a_list_not_an_option() {
    assert_ty("library.books", book().list());
    assert_ty("book.authors", Ty::class(PKG, "Writer").list());
}

// ---------------------------------------------------------------------------
// R4 — DIVISION BY ZERO
// ---------------------------------------------------------------------------

#[test]
fn r4_constant_zero_divisor_is_a_type_error() {
    let errors = assert_err("book.pages / 0", "R4");
    assert_eq!(errors.len(), 1);
    // unary-minus-folded zero counts as a constant zero too
    assert_err("book.pages / -0", "R4");
    assert_err("2147483647 / (2 - 2)", "R4");
}

#[test]
fn r4_integer_division_is_truncating_and_types_as_the_operand() {
    assert_ty("book.pages / 2", Ty::int());
    assert_ty("book.downloads / 2", Ty::long()); // L1 widens the literal
    assert_ty("10 / 3", Ty::int());
}

// ---------------------------------------------------------------------------
// Operation calls
// ---------------------------------------------------------------------------

#[test]
fn op_calls_check_arity_and_types() {
    assert_ty(r#"library.findBook("x")"#, book());
    assert_ty("library.totalInCirculation(2026)", Ty::long());

    // arity errors
    assert_err("library.findBook()", "arity");
    assert_err(r#"library.findBook("a", "b")"#, "arity");
    // argument type errors (no implicit int -> long, L1/L2)
    assert_err("library.findBook(1)", "argument");
    assert_err("library.totalInCirculation(book.downloads)", "argument");
    // unknown operation
    assert_err(r#"library.findCat("x")"#, "unknown");
    // calling an operation on a non-class receiver
    assert_err("book.pages.trim()", "unknown");
}

// ---------------------------------------------------------------------------
// Collection algebra (A1-A6)
// ---------------------------------------------------------------------------

#[test]
fn algebra_types_follow_a1_to_a6() {
    assert_ty("library.books.size()", Ty::int()); // A5
    assert_ty("library.books.filter(b => b.pages > 100)", book().list()); // A2
    assert_ty("library.books.map(b => b.title)", Ty::string().list()); // A3
                                                                       // A3 with a boolean lambda: List(boolean)
    assert_ty("library.books.map(b => b.pages > 5)", Ty::boolean().list());
    assert_ty("library.books.any(b => b.pages > 500)", Ty::boolean()); // A4
    assert_ty("library.books.first()", book().optional()); // A1
    assert_ty("library.books.first(b => b.pages > 10)", book().optional());
    assert_ty("library.books.map(b => b.pages).sum()", Ty::int()); // A6
}

#[test]
fn algebra_requires_collections_and_matching_lambdas() {
    // algebra on a non-list
    assert_err("book.pages.size()", "collection");
    // filter/any/first lambdas must produce boolean
    assert_err("library.books.filter(b => b.pages)", "boolean");
    assert_err("library.books.any(b => b.pages)", "boolean");
    // sum requires a numeric element type (A6)
    assert_err("library.books.sum()", "numeric");
    assert_err("library.books.map(b => b.title).sum()", "numeric");
    // lambda body errors are reported with the lambda parameter bound
    assert_err("library.books.filter(b => b.nope > 1)", "nope");
}

// ---------------------------------------------------------------------------
// if / unary / lists
// ---------------------------------------------------------------------------

#[test]
fn u1_if_requires_boolean_condition_and_exact_branch_unification() {
    assert_ty("if true { 1 } else { 2 }", Ty::int());
    assert_ty("if book.pages > 0 { book.pages } else { 0 }", Ty::int());
    assert_ty(
        "let d = book.downloads; if true { d } else { d }",
        Ty::long(),
    );

    // non-boolean condition
    assert_err("if book.pages { 1 } else { 2 }", "boolean");
    // U1: no implicit int -> long in either direction, literals included
    assert_err("if true { 1 } else { book.downloads }", "U1");
    assert_err("let d = book.downloads; if true { d } else { 1 }", "U1");
}

#[test]
fn unary_and_list_literals_type_check() {
    assert_ty("!true", Ty::boolean());
    assert_ty("-book.pages", Ty::int());
    assert_err(r#"-"x""#, "numeric");
    assert_err("!1", "boolean");

    assert_ty("[1, 2, 3]", Ty::int().list());
    assert_ty("[book.title, \"x\"]", Ty::string().list());
    assert_ty("[book.downloads, 1]", Ty::long().list()); // L1 element adaptation
    assert_err("[1, book.title]", "type");
    // an empty list cannot have its element type inferred
    assert_err("[]", "infer");
}

#[test]
fn multiple_errors_are_collected_without_cascades() {
    // Both operands are broken, so both errors are reported, but the `+`
    // itself does not add a third one.
    let parsed = parse("unknown_a + unknown_b");
    let errors = checker()
        .type_of(parsed.ast.as_ref().expect("ast"))
        .expect_err("expected errors");
    assert_eq!(errors.len(), 2);
}
