//! Parser tests for the expression sublanguage (`docs/EXPRESSIONS.md`):
//! shapes, spans, precedence, associativity, lambdas, `?.` vs `.`, `?:`,
//! if/let, collection literals, algebra shapes, recovery, and no-panics.

use rex_expr::{parse, AlgebraKind, BinOp, Expr, ExprKind, Spanned, UnOp};

/// Parses `source`, expecting no errors; returns the expression.
fn ok(source: &str) -> Expr {
    let result = parse(source);
    assert!(result.errors.is_empty(), "{source:?}: {:?}", result.errors);
    result.ast.unwrap_or_else(|| panic!("{source:?}: no ast"))
}

/// The kind of an expression (cloned; comparisons in these tests ignore
/// spans).
fn kind(expr: &Expr) -> ExprKind {
    expr.kind.clone()
}

/// A span-free copy of an expression, for structural comparison: every node
/// gets span `0..0`.
fn plain(expr: &Expr) -> Expr {
    let span = (0..0).into();
    let zero = span;
    let kind = match &expr.kind {
        ExprKind::Int(value) => ExprKind::Int(*value),
        ExprKind::String(value) => ExprKind::String(value.clone()),
        ExprKind::Bool(value) => ExprKind::Bool(*value),
        ExprKind::Null => ExprKind::Null,
        ExprKind::Date { text, .. } => ExprKind::Date {
            text: text.clone(),
            literal_span: zero,
        },
        ExprKind::Name(text) => ExprKind::Name(text.clone()),
        ExprKind::FeatureAccess {
            receiver,
            name,
            optional_safe,
        } => ExprKind::FeatureAccess {
            receiver: Box::new(plain(receiver)),
            name: Spanned {
                value: name.value.clone(),
                span: zero,
            },
            optional_safe: *optional_safe,
        },
        ExprKind::Coalesce { value, default } => ExprKind::Coalesce {
            value: Box::new(plain(value)),
            default: Box::new(plain(default)),
        },
        ExprKind::Call {
            receiver,
            name,
            args,
            optional_safe,
        } => ExprKind::Call {
            receiver: Box::new(plain(receiver)),
            name: Spanned {
                value: name.value.clone(),
                span: zero,
            },
            args: args.iter().map(plain).collect(),
            optional_safe: *optional_safe,
        },
        ExprKind::Algebra {
            receiver,
            kind,
            lambda,
            optional_safe,
        } => ExprKind::Algebra {
            receiver: Box::new(plain(receiver)),
            kind: *kind,
            lambda: lambda
                .as_ref()
                .map(|(param, body)| (param.clone(), Box::new(plain(body)))),
            optional_safe: *optional_safe,
        },
        ExprKind::Binary { op, lhs, rhs } => ExprKind::Binary {
            op: *op,
            lhs: Box::new(plain(lhs)),
            rhs: Box::new(plain(rhs)),
        },
        ExprKind::Unary { op, expr } => ExprKind::Unary {
            op: *op,
            expr: Box::new(plain(expr)),
        },
        ExprKind::If { cond, then, else_ } => ExprKind::If {
            cond: Box::new(plain(cond)),
            then: Box::new(plain(then)),
            else_: Box::new(plain(else_)),
        },
        ExprKind::Let { name, init, body } => ExprKind::Let {
            name: Spanned {
                value: name.value.clone(),
                span: zero,
            },
            init: Box::new(plain(init)),
            body: Box::new(plain(body)),
        },
        ExprKind::Lambda { param, body } => ExprKind::Lambda {
            param: param.clone(),
            body: Box::new(plain(body)),
        },
        ExprKind::ListLiteral(items) => ExprKind::ListLiteral(items.iter().map(plain).collect()),
    };
    Expr::new(kind, span)
}

/// Parses `source` (expecting no errors) and returns its span-free shape.
fn shape(source: &str) -> Expr {
    plain(&ok(source))
}

fn int(value: i64) -> ExprKind {
    ExprKind::Int(value)
}

fn nm(text: &str) -> ExprKind {
    ExprKind::Name(text.to_string())
}

fn bin(op: BinOp, lhs: ExprKind, rhs: ExprKind) -> ExprKind {
    ExprKind::Binary {
        op,
        lhs: Box::new(Expr::new(lhs, (0..0).into())),
        rhs: Box::new(Expr::new(rhs, (0..0).into())),
    }
}

/// The receiver of a member access/call/algebra node, for chain checks.
fn receiver_of(expr: &Expr) -> &Expr {
    match &expr.kind {
        ExprKind::FeatureAccess { receiver, .. }
        | ExprKind::Call { receiver, .. }
        | ExprKind::Algebra { receiver, .. } => receiver,
        other => panic!("not a member node: {other:?}"),
    }
}

#[test]
fn literals_parse_with_spans() {
    assert_eq!(kind(&ok("42")), int(42));
    assert_eq!(ok("42").span, (0..2).into());

    assert_eq!(kind(&ok(r#""hi""#)), ExprKind::String("hi".to_string()));
    assert_eq!(
        kind(&ok(r#""a\"b\\c""#)),
        ExprKind::String("a\"b\\c".to_string())
    );
    assert_eq!(kind(&ok("true")), ExprKind::Bool(true));
    assert_eq!(kind(&ok("false")), ExprKind::Bool(false));
    assert_eq!(kind(&ok("null")), ExprKind::Null);
    assert_eq!(ok("null").span, (0..4).into());

    assert_eq!(kind(&ok("pages")), nm("pages"));
    assert_eq!(ok("pages").span, (0..5).into());
}

#[test]
fn integer_literals_are_non_negative_and_minus_is_unary() {
    // `-42` is Unary(Neg, Int(42)), not a literal: the token grammar has no
    // negative literals (docs/EXPRESSIONS.md lexical rules).
    let expr = ok("-42");
    assert_eq!(
        kind(&expr),
        ExprKind::Unary {
            op: UnOp::Neg,
            expr: Box::new(Expr::new(int(42), (1..3).into())),
        }
    );
    assert_eq!(expr.span, (0..3).into());
}

#[test]
fn list_literals_parse_empty_and_nested() {
    assert_eq!(kind(&ok("[]")), ExprKind::ListLiteral(vec![]));
    assert_eq!(ok("[]").span, (0..2).into());

    let list = ok("[1, 2, 3]");
    match kind(&list) {
        ExprKind::ListLiteral(items) => {
            assert_eq!(items.len(), 3);
            assert_eq!(items[0].span, (1..2).into());
            assert_eq!(items[2].span, (7..8).into());
        }
        other => panic!("expected list, got {other:?}"),
    }
    assert_eq!(list.span, (0..9).into());

    let nested = ok("[[1], 2]");
    match kind(&nested) {
        ExprKind::ListLiteral(items) => {
            assert_eq!(items.len(), 2);
            assert!(matches!(kind(&items[0]), ExprKind::ListLiteral(_)));
        }
        other => panic!("expected list, got {other:?}"),
    }
}

#[test]
fn unary_operators_parse_and_nest() {
    let not = ok("!flag");
    assert_eq!(
        kind(&not),
        ExprKind::Unary {
            op: UnOp::Not,
            expr: Box::new(Expr::new(nm("flag"), (1..5).into())),
        }
    );
    assert_eq!(not.span, (0..5).into());

    // `-(-x)` and `!-x`: unary is right-recursive over unary.
    let double = ok("--x");
    assert!(matches!(
        kind(&double),
        ExprKind::Unary { op: UnOp::Neg, .. }
    ));
    let mixed = ok("!-x");
    assert!(matches!(
        kind(&mixed),
        ExprKind::Unary { op: UnOp::Not, .. }
    ));
}

#[test]
fn precedence_multiplicative_over_additive() {
    // 1 + 2 * 3  ==  1 + (2 * 3)
    assert_eq!(
        shape("1 + 2 * 3").kind,
        bin(BinOp::Add, int(1), bin(BinOp::Mul, int(2), int(3)))
    );
    // 10 - 4 / 2  ==  10 - (4 / 2)
    assert_eq!(
        shape("10 - 4 / 2").kind,
        bin(BinOp::Sub, int(10), bin(BinOp::Div, int(4), int(2)))
    );
}

#[test]
fn binary_operators_are_left_associative() {
    // 10 - 4 - 3  ==  (10 - 4) - 3
    assert_eq!(
        shape("10 - 4 - 3").kind,
        bin(BinOp::Sub, bin(BinOp::Sub, int(10), int(4)), int(3))
    );
    // a - b + c  ==  (a - b) + c  (same-tier, left fold)
    assert_eq!(
        shape("a - b + c").kind,
        bin(BinOp::Add, bin(BinOp::Sub, nm("a"), nm("b")), nm("c"))
    );
}

#[test]
fn parentheses_override_precedence() {
    assert_eq!(
        shape("(1 + 2) * 3").kind,
        bin(BinOp::Mul, bin(BinOp::Add, int(1), int(2)), int(3))
    );
}

#[test]
fn precedence_comparisons_over_logic() {
    // a < b && c < d  ==  (a < b) && (c < d)
    assert_eq!(
        shape("a < b && c < d").kind,
        bin(
            BinOp::And,
            bin(BinOp::Lt, nm("a"), nm("b")),
            bin(BinOp::Lt, nm("c"), nm("d"))
        )
    );
    // a || b && c  ==  a || (b && c)  (&& binds tighter than ||)
    assert_eq!(
        shape("a || b && c").kind,
        bin(BinOp::Or, nm("a"), bin(BinOp::And, nm("b"), nm("c")))
    );
    // x == y || p <= q  ==  (x == y) || (p <= q)
    assert_eq!(
        shape("x == y || p <= q").kind,
        bin(
            BinOp::Or,
            bin(BinOp::Eq, nm("x"), nm("y")),
            bin(BinOp::Le, nm("p"), nm("q"))
        )
    );
}

#[test]
fn single_equals_is_an_equality_alias() {
    // Spec R2: `==` is canonical; `=` is accepted where an operator is
    // expected and means the same thing.
    assert_eq!(shape("a = b").kind, bin(BinOp::Eq, nm("a"), nm("b")));
    assert_eq!(shape("a == b").kind, bin(BinOp::Eq, nm("a"), nm("b")));
}

#[test]
fn coalesce_parses_and_binds_looser_than_logic_or() {
    let elvis = ok(r#"book.citation ?: "none""#);
    match kind(&elvis) {
        ExprKind::Coalesce { value, default } => {
            assert!(matches!(kind(&value), ExprKind::FeatureAccess { .. }));
            assert_eq!(kind(&default), ExprKind::String("none".to_string()));
        }
        other => panic!("expected coalesce, got {other:?}"),
    }
    assert_eq!(elvis.span, (0..23).into());

    // left-associative chain: a ?: b ?: c == (a ?: b) ?: c
    let chain = ok("a ?: b ?: c");
    let outer = match kind(&chain) {
        ExprKind::Coalesce { value, .. } => value,
        other => panic!("expected coalesce, got {other:?}"),
    };
    assert!(matches!(kind(&outer), ExprKind::Coalesce { .. }));

    // x == y ?: z  ==  (x == y) ?: z — coalesce is the loosest binary level.
    let mixed = ok("x == y ?: z");
    let value = match kind(&mixed) {
        ExprKind::Coalesce { value, .. } => value,
        other => panic!("expected coalesce, got {other:?}"),
    };
    assert!(matches!(
        kind(&value),
        ExprKind::Binary { op: BinOp::Eq, .. }
    ));
}

#[test]
fn if_expr_parses_with_spans() {
    let source = "if flag { 1 } else { 2 }";
    let expr = ok(source);
    assert_eq!(expr.span, (0..source.len()).into());
    match kind(&expr) {
        ExprKind::If { cond, then, else_ } => {
            assert_eq!(kind(&cond), nm("flag"));
            assert_eq!(kind(&then), int(1));
            assert_eq!(kind(&else_), int(2));
        }
        other => panic!("expected if, got {other:?}"),
    }

    // else { if ... } nests; the condition is a full expression.
    let nested = ok(r#"if a.title == "x" { 1 } else { if b { 2 } else { 3 } }"#);
    assert!(matches!(kind(&nested), ExprKind::If { .. }));
}

#[test]
fn let_expr_binds_over_a_maximal_body() {
    let source = "let x = 1; x + 2";
    let expr = ok(source);
    assert_eq!(expr.span, (0..source.len()).into());
    match kind(&expr) {
        ExprKind::Let { name, init, body } => {
            assert_eq!(name.value, "x");
            assert_eq!(name.span, (4..5).into());
            assert_eq!(plain(&init).kind, int(1));
            assert_eq!(
                plain(&body).kind,
                bin(BinOp::Add, nm("x"), int(2)),
                "let body extends maximally"
            );
        }
        other => panic!("expected let, got {other:?}"),
    }

    // nested let in the body
    let nested = ok("let a = 1; let b = a; b");
    match kind(&nested) {
        ExprKind::Let { name, .. } => assert_eq!(name.value, "a"),
        other => panic!("expected let, got {other:?}"),
    }
}

#[test]
fn lambda_parses_with_maximal_body() {
    let expr = ok("b => b + 1");
    match kind(&expr) {
        ExprKind::Lambda { param, body } => {
            assert_eq!(param, "b");
            assert_eq!(plain(&body).kind, bin(BinOp::Add, nm("b"), int(1)));
        }
        other => panic!("expected lambda, got {other:?}"),
    }
    assert_eq!(ok("b => b").span, (0..6).into());
}

#[test]
fn feature_access_and_safe_navigation() {
    let access = ok("book.title");
    match kind(&access) {
        ExprKind::FeatureAccess {
            receiver,
            name,
            optional_safe,
        } => {
            assert_eq!(kind(&receiver), nm("book"));
            assert_eq!(name.value, "title");
            assert_eq!(name.span, (5..10).into());
            assert!(!optional_safe);
        }
        other => panic!("expected feature access, got {other:?}"),
    }
    assert_eq!(access.span, (0..10).into());

    let safe = ok("book?.title");
    match kind(&safe) {
        ExprKind::FeatureAccess { optional_safe, .. } => assert!(optional_safe),
        other => panic!("expected feature access, got {other:?}"),
    }

    // chains: a.b.c folds left
    let chain = ok("a.b.c");
    assert!(matches!(
        kind(receiver_of(&chain)),
        ExprKind::FeatureAccess { .. }
    ));
}

#[test]
fn operation_calls_capture_arguments() {
    let call = ok(r#"library.findBook("ulysses")"#);
    match kind(&call) {
        ExprKind::Call {
            receiver,
            name,
            args,
            optional_safe,
        } => {
            assert_eq!(kind(&receiver), nm("library"));
            assert_eq!(name.value, "findBook");
            assert_eq!(args.len(), 1);
            assert_eq!(kind(&args[0]), ExprKind::String("ulysses".to_string()));
            assert!(!optional_safe);
        }
        other => panic!("expected call, got {other:?}"),
    }
    assert_eq!(call.span, (0..27).into());

    let multi = ok("a.op(1, 2 + 3)");
    match kind(&multi) {
        ExprKind::Call { args, .. } => {
            assert_eq!(args.len(), 2);
            assert_eq!(plain(&args[1]).kind, bin(BinOp::Add, int(2), int(3)));
        }
        other => panic!("expected call, got {other:?}"),
    }

    let nested = ok("a.op(b.other(1))");
    match kind(&nested) {
        ExprKind::Call { args, .. } => {
            assert!(matches!(kind(&args[0]), ExprKind::Call { .. }));
        }
        other => panic!("expected call, got {other:?}"),
    }

    // no-arg call, and ?. on a call
    match kind(&ok("a.op()")) {
        ExprKind::Call {
            args,
            optional_safe,
            ..
        } => {
            assert!(args.is_empty());
            assert!(!optional_safe);
        }
        other => panic!("expected call, got {other:?}"),
    }
    match kind(&ok("a?.op()")) {
        ExprKind::Call { optional_safe, .. } => assert!(optional_safe),
        other => panic!("expected call, got {other:?}"),
    }
}

#[test]
fn collection_algebra_shapes() {
    let size = ok("books.size()");
    match kind(&size) {
        ExprKind::Algebra {
            receiver,
            kind: algebra,
            lambda,
            optional_safe,
        } => {
            assert_eq!(kind(&receiver), nm("books"));
            assert_eq!(algebra, AlgebraKind::Size);
            assert!(lambda.is_none());
            assert!(!optional_safe);
        }
        other => panic!("expected algebra, got {other:?}"),
    }

    // every algebra kind parses in call position
    for (source, expected) in [
        ("books.first()", AlgebraKind::First),
        ("books.filter(f => f)", AlgebraKind::Filter),
        ("books.map(f => f)", AlgebraKind::Map),
        ("books.any(f => f)", AlgebraKind::Any),
        ("books.sum()", AlgebraKind::Sum),
    ] {
        match kind(&ok(source)) {
            ExprKind::Algebra { kind: algebra, .. } => {
                assert_eq!(algebra, expected, "{source}")
            }
            other => panic!("{source}: expected algebra, got {other:?}"),
        }
    }

    let filter = ok("books.filter(b => b.pages > 100)");
    match kind(&filter) {
        ExprKind::Algebra {
            lambda,
            kind: algebra,
            ..
        } => {
            assert_eq!(algebra, AlgebraKind::Filter);
            let (param, body) = lambda.as_ref().expect("lambda");
            assert_eq!(param, "b");
            match kind(body) {
                ExprKind::Binary { op, lhs, .. } => {
                    assert_eq!(op, BinOp::Gt);
                    match kind(&lhs) {
                        ExprKind::FeatureAccess { name, .. } => {
                            assert_eq!(name.value, "pages")
                        }
                        other => panic!("expected feature access, got {other:?}"),
                    }
                }
                other => panic!("expected binary, got {other:?}"),
            }
        }
        other => panic!("expected algebra, got {other:?}"),
    }

    // first takes an optional lambda; algebra names are contextual: a plain
    // `book.first` without parentheses is a feature access, not algebra.
    match kind(&ok("books.first(b => b.pages > 10)")) {
        ExprKind::Algebra {
            kind: algebra,
            lambda,
            ..
        } => {
            assert_eq!(algebra, AlgebraKind::First);
            assert!(lambda.is_some());
        }
        other => panic!("expected algebra, got {other:?}"),
    }
    assert!(matches!(
        kind(&ok("book.first")),
        ExprKind::FeatureAccess { .. }
    ));
}

#[test]
fn algebra_argument_shapes_are_enforced() {
    // filter/map/any require exactly one lambda
    for source in ["books.filter(1)", "books.map()", "books.any(1, 2)"] {
        let result = parse(source);
        assert!(
            result.errors.iter().any(|e| e.message.contains("lambda")),
            "{source}: expected a lambda-shape error, got {:?}",
            result.errors
        );
    }
    // first takes at most one lambda
    let result = parse("books.first(1)");
    assert!(result.errors.iter().any(|e| e.message.contains("lambda")));
    // size/sum take no arguments
    for source in ["books.size(b => b)", "books.sum(1)"] {
        let result = parse(source);
        assert!(
            result
                .errors
                .iter()
                .any(|e| e.message.contains("no arguments")),
            "{source}: expected a no-arguments error, got {:?}",
            result.errors
        );
    }
}

#[test]
fn spans_cover_the_whole_expression() {
    assert_eq!(ok("1 + 2").span, (0..5).into());
    assert_eq!(ok("book?.title").span, (0..11).into());
    assert_eq!(ok("books.filter(b => b.pages > 1)").span, (0..30).into());
    assert_eq!(ok("let x = 1; x").span, (0..12).into());
    assert_eq!(ok("if a { 1 } else { 2 }").span, (0..21).into());
    assert_eq!(ok(r#"book.citation ?: "none""#).span, (0..23).into());
}

#[test]
fn trailing_input_is_recovered_with_the_parsed_expression() {
    // Recovery (like rex-syntax): the valid prefix expression is returned and
    // the junk tail becomes a single error.
    let result = parse("1 + 2 ) (");
    assert!(result.ast.is_some(), "valid prefix should be recovered");
    assert_eq!(
        plain(result.ast.as_ref().unwrap()).kind,
        bin(BinOp::Add, int(1), int(2))
    );
    assert!(
        result.errors.iter().any(|e| e.message.contains("trailing")),
        "expected a trailing-input error, got {:?}",
        result.errors
    );

    let result = parse("1 2");
    assert!(result.ast.is_some());
    assert!(!result.errors.is_empty());
}

#[test]
fn incomplete_expressions_report_errors_at_the_offending_span() {
    // A missing operand after `+`: the valid prefix `1` is recovered and the
    // junk (`+`) becomes a trailing-input error.
    let result = parse("1 +");
    assert!(result.ast.is_some(), "the prefix expression is recovered");
    assert_eq!(result.errors.len(), 1);
    assert_eq!(result.errors[0].message, "unexpected trailing input");

    // An unclosed delimiter cannot be recovered: no expression is produced,
    // and the error points at end of input.
    let result = parse("(1 + 2");
    assert!(result.ast.is_none());
    assert!(!result.errors.is_empty());
    assert_eq!(
        result.errors[0].span,
        (6..6).into(),
        "error points at end of input"
    );
}

#[test]
fn empty_input_yields_an_error_not_a_panic() {
    let result = parse("");
    assert!(result.ast.is_none());
    assert_eq!(result.errors.len(), 1);
    assert_eq!(result.errors[0].span, (0..0).into());
}

#[test]
fn garbage_never_panics() {
    let garbage = [
        "(((((",
        ")))))",
        "@#$%^&",
        "\"unterminated",
        "let",
        "let =",
        "if x",
        "if x { 1 }",
        "books.filter()",
        "books.filter(1",
        "a ? b",
        "a ?:",
        "?.x",
        "= =",
        "=>",
        "9223372036854775808",
        "a b c",
        "1..2",
        "x;",
        ";x",
        "[1,",
        "[1,,2]",
        "a.,b",
        "{ 1 }",
        "f(1;2)",
    ];
    for source in garbage {
        let result = parse(source);
        // Whatever happens, parsing terminates: either an expression was
        // recovered or the input is rejected with at least one error.
        if result.ast.is_none() {
            assert!(
                !result.errors.is_empty(),
                "{source:?}: no ast and no errors"
            );
        }
    }
}

#[test]
fn algebra_names_are_contextual_and_keywords_are_reserved() {
    // `first` etc. are ordinary identifiers outside call position; a bare call
    // has no receiver and the grammar has no free functions.
    let result = parse("first(1)");
    assert!(result.ast.is_none() || !result.errors.is_empty());
    // Keywords cannot be identifiers: this must not parse as `if + 1`.
    assert!(!parse("if + 1").errors.is_empty());
}

// ---------------------------------------------------------------------------
// Date literals (issue #9, spec R6)
// ---------------------------------------------------------------------------

#[test]
fn date_literals_are_their_own_primary_form() {
    let expr = ok(r#"date("2026-09-17")"#);
    let ExprKind::Date { text, literal_span } = &expr.kind else {
        panic!("expected a date literal, got {:?}", expr.kind)
    };
    assert_eq!(text, "2026-09-17");
    assert_eq!(
        *literal_span,
        (5..17).into(),
        "span points at the string literal"
    );
    assert_eq!(expr.span, (0..18).into(), "the node spans the whole form");
}

#[test]
fn date_literals_compose_like_any_primary() {
    // Comparison chains, arguments, and let bindings all accept them.
    let comparison = ok(r#"date("2026-09-17") < date("2027-01-01")"#);
    match kind(&comparison) {
        ExprKind::Binary { op: BinOp::Lt, .. } => {}
        other => panic!("expected a comparison, got {other:?}"),
    }
    match kind(&ok(r#"let d = date("2026-01-31"); d.plus_months(1)"#)) {
        ExprKind::Let { init, body, .. } => {
            assert!(matches!(&init.kind, ExprKind::Date { .. }));
            match &body.kind {
                ExprKind::Call { name, args, .. } => {
                    assert_eq!(name.value, "plus_months");
                    assert!(matches!(&args[0].kind, ExprKind::Int(1)));
                }
                other => panic!("expected a method call, got {other:?}"),
            }
        }
        other => panic!("expected a let, got {other:?}"),
    }
}

#[test]
fn date_constructor_requires_a_string_literal() {
    // A non-literal argument that parses gets the R6 shape diagnostic...
    for source in ["date(20260917)", "date(day)"] {
        let result = parse(source);
        assert!(
            result
                .errors
                .iter()
                .any(|error| error.message.contains("string literal")),
            "{source:?}: expected the R6 string-literal diagnostic, got {:?}",
            result.errors
        );
    }
    // ...and one that does not parse at all is still rejected.
    let result = parse(r#"date("a" ++ "b")"#);
    assert!(
        result.ast.is_none() || !result.errors.is_empty(),
        "a non-literal argument must not silently produce a date"
    );
}

#[test]
fn date_stays_a_contextual_word() {
    // A bare `date` is still an ordinary name...
    assert_eq!(kind(&ok("date")), nm("date"));
    // ...and member access on a feature named `date` keeps working.
    let access = ok("book.date");
    match kind(&access) {
        ExprKind::FeatureAccess { name, .. } => assert_eq!(name.value, "date"),
        other => panic!("expected feature access, got {other:?}"),
    }
    // `date` alone never becomes the constructor without a string argument.
    let result = parse(r#"date ("2026-09-17")"#);
    assert!(
        matches!(&result.ast.expect("parses").kind, ExprKind::Date { .. }),
        "whitespace before the parenthesis still forms the constructor"
    );
}
