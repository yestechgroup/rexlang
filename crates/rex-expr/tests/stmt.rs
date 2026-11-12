//! The procedural statement IR ([`rex_expr::stmt`]): canonical wire shape and
//! the statement typing rules (issue #178).
//!
//! The canonical JSON literal below is the byte-exact wire contract for a
//! `.ddd` operation's statement body: key order follows declaration order,
//! empty `else_` branches, empty raise argument lists, and valueless returns
//! are omitted, and `if` renames its `else_` field to `else`.

use rex_expr::body::BodyOutput;
use rex_expr::stmt::{check_stmt_body, BodyRaise, Stmt, StmtBody, StmtReceiver};
use rex_expr::DomainTypes;
use rex_ir::{
    Feature, FeatureKind, Multiplicity, Operation, OperationParam, PrimitiveType, TypeRef,
};

fn class_ref(name: &str) -> TypeRef {
    TypeRef::Class {
        package: "pricing".to_string(),
        name: name.to_string(),
    }
}

/// The statement fixture's domain shape: a `Trade` aggregate (a `price`
/// double, a `note` string, a `counterparty` containment, a many `tags`
/// attribute), its `Party`, and a synthetic repository class whose
/// `findById` operation returns a `Trade` (the caller-registered shape the
/// `.ddd` driver will build for an injected repository).
fn fixture_context() -> DomainTypes {
    let mut context = DomainTypes::default();
    context.insert_class(
        "pricing",
        "Trade",
        vec![],
        vec![
            Feature::new(
                "price",
                FeatureKind::Attribute,
                TypeRef::Primitive(PrimitiveType::Double),
                Multiplicity::REQUIRED,
            ),
            Feature::new(
                "note",
                FeatureKind::Attribute,
                TypeRef::Primitive(PrimitiveType::String),
                Multiplicity::REQUIRED,
            ),
            Feature::new(
                "counterparty",
                FeatureKind::Containment,
                class_ref("Party"),
                Multiplicity::REQUIRED,
            ),
            Feature::new(
                "tags",
                FeatureKind::Attribute,
                TypeRef::Primitive(PrimitiveType::String),
                Multiplicity::MANY,
            ),
        ],
        vec![],
    );
    context.insert_class(
        "pricing",
        "Party",
        vec![],
        vec![Feature::new(
            "name",
            FeatureKind::Attribute,
            TypeRef::Primitive(PrimitiveType::String),
            Multiplicity::REQUIRED,
        )],
        vec![],
    );
    context.insert_class(
        "design",
        "TradeRepository",
        vec![],
        vec![],
        vec![Operation::new(
            "findById",
            class_ref("Trade"),
            vec![OperationParam {
                name: "id".to_string(),
                type_: TypeRef::Primitive(PrimitiveType::String),
                multiplicity: None,
            }],
        )],
    );
    context
}

fn param(name: &str, type_: TypeRef, multiplicity: Option<Multiplicity>) -> OperationParam {
    OperationParam {
        name: name.to_string(),
        type_,
        multiplicity,
    }
}

fn id_param() -> OperationParam {
    param("id", TypeRef::Primitive(PrimitiveType::String), None)
}

fn trade_param() -> OperationParam {
    param("trade", class_ref("Trade"), None)
}

/// A `(0..1) string` parameter: types as `Option<string>` in the scope.
fn maybe_note_param() -> OperationParam {
    param(
        "maybeNote",
        TypeRef::Primitive(PrimitiveType::String),
        Some(Multiplicity::OPTIONAL),
    )
}

/// A `(0..*) string` parameter: types as `List<string>` in the scope.
fn tags_param() -> OperationParam {
    param(
        "tags",
        TypeRef::Primitive(PrimitiveType::String),
        Some(Multiplicity::MANY),
    )
}

fn double_output() -> BodyOutput<'static> {
    BodyOutput {
        name: "rate",
        type_: &TypeRef::Primitive(PrimitiveType::Double),
        multiplicity: None,
    }
}

fn string_output() -> BodyOutput<'static> {
    BodyOutput {
        name: "note",
        type_: &TypeRef::Primitive(PrimitiveType::String),
        multiplicity: None,
    }
}

fn many_string_output() -> BodyOutput<'static> {
    BodyOutput {
        name: "tags",
        type_: &TypeRef::Primitive(PrimitiveType::String),
        multiplicity: Some(&Multiplicity::MANY),
    }
}

fn optional_string_output() -> BodyOutput<'static> {
    BodyOutput {
        name: "note",
        type_: &TypeRef::Primitive(PrimitiveType::String),
        multiplicity: Some(&Multiplicity::OPTIONAL),
    }
}

fn trade_repository() -> StmtReceiver<'static> {
    StmtReceiver {
        name: "trades",
        package: "design",
        class: "TradeRepository",
    }
}

fn conflict(arg: &str) -> BodyRaise {
    BodyRaise {
        variant: "Conflict".to_string(),
        args: vec![arg.to_string()],
    }
}

fn not_found(args: &[&str]) -> BodyRaise {
    BodyRaise {
        variant: "NotFound".to_string(),
        args: args.iter().map(|arg| arg.to_string()).collect(),
    }
}

fn return_of(expr: Option<&str>) -> Stmt {
    Stmt::Return {
        expr: expr.map(|text| text.to_string()),
    }
}

/// Every variant in one body: the pinned wire literal. `letElse` tags as one
/// word, raise args ride in declaration order, `assign` always carries its
/// path, the `if` first branch is present while the empty `else` is omitted,
/// and a valueless `return` is just its tag.
#[test]
fn stmt_body_serializes_every_variant_to_the_pinned_canonical_json() {
    let body = StmtBody {
        stmts: vec![
            Stmt::Let {
                name: "trade".to_string(),
                expr: "trades.findById(id)".to_string(),
            },
            Stmt::LetElse {
                name: "note".to_string(),
                expr: "maybeNote".to_string(),
                error: not_found(&["id", "note"]),
            },
            Stmt::Assign {
                path: vec!["trade".to_string(), "price".to_string()],
                expr: "1.0".to_string(),
            },
            Stmt::Guard {
                expr: "trade.price > 0.0".to_string(),
                error: conflict("trade.note"),
            },
            Stmt::If {
                cond: "trade.price > 0.0".to_string(),
                then: vec![return_of(Some("trade.price"))],
                else_: vec![],
            },
            Stmt::Return { expr: None },
        ],
    };
    let json = serde_json::to_string(&body).expect("serialize");
    assert_eq!(
        json,
        "{\"stmts\":[{\"kind\":\"let\",\"name\":\"trade\",\"expr\":\"trades.findById(id)\"},{\"kind\":\"letElse\",\"name\":\"note\",\"expr\":\"maybeNote\",\"error\":{\"variant\":\"NotFound\",\"args\":[\"id\",\"note\"]}},{\"kind\":\"assign\",\"path\":[\"trade\",\"price\"],\"expr\":\"1.0\"},{\"kind\":\"guard\",\"expr\":\"trade.price > 0.0\",\"error\":{\"variant\":\"Conflict\",\"args\":[\"trade.note\"]}},{\"kind\":\"if\",\"cond\":\"trade.price > 0.0\",\"then\":[{\"kind\":\"return\",\"expr\":\"trade.price\"}]},{\"kind\":\"return\"}]}"
    );
}

#[test]
fn stmt_body_round_trips() {
    let body = StmtBody {
        stmts: vec![
            Stmt::Let {
                name: "trade".to_string(),
                expr: "trades.findById(id)".to_string(),
            },
            Stmt::If {
                cond: "trade.price > 0.0".to_string(),
                then: vec![return_of(Some("trade.price"))],
                else_: vec![return_of(Some("0.0"))],
            },
        ],
    };
    let json = serde_json::to_string(&body).expect("serialize");
    let round: StmtBody = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(round, body);
    // The non-empty `else` serializes under its Rust-keyword-safe name.
    assert!(
        json.contains("\"else\":[{\"kind\":\"return\",\"expr\":\"0.0\"}]"),
        "{json}"
    );
}

/// An empty body is an empty object: no `stmts` key, so a body-less design
/// artifact stays byte-identical.
#[test]
fn stmt_body_serializes_an_empty_body_as_an_empty_object() {
    let json = serde_json::to_string(&StmtBody::default()).expect("serialize");
    assert_eq!(json, "{}");
}

/// `let` infers its initializer's type and only later statements see the
/// name (the sequential-scope rule).
#[test]
fn check_stmt_let_infers_and_threads_scope() {
    let body = StmtBody {
        stmts: vec![
            Stmt::Let {
                name: "price".to_string(),
                expr: "trade.price".to_string(),
            },
            return_of(Some("price")),
        ],
    };
    assert_eq!(
        check_stmt_body(
            &fixture_context(),
            &[trade_param()],
            Some(double_output()),
            &[],
            &body,
        ),
        Ok(())
    );

    // A guard before the `let` cannot see the name yet.
    let early = StmtBody {
        stmts: vec![
            Stmt::Guard {
                expr: "price > 0.0".to_string(),
                error: conflict("id"),
            },
            Stmt::Let {
                name: "price".to_string(),
                expr: "trade.price".to_string(),
            },
            return_of(Some("price")),
        ],
    };
    let errors = check_stmt_body(
        &fixture_context(),
        &[trade_param(), id_param()],
        Some(double_output()),
        &[],
        &early,
    )
    .expect_err("forward reference");
    assert_eq!(errors.len(), 1, "{errors:?}");
    assert_eq!(errors[0].slot, "guard 'price > 0.0'");
    assert!(
        errors[0].message.contains("unknown name `price`"),
        "{:?}",
        errors[0].message
    );
}

/// `let … else error …` unwraps an `Option<T>` initializer and binds the
/// plain `T` (returning it as the `string` output proves the unwrap).
#[test]
fn check_stmt_let_else_unwraps_option() {
    let body = StmtBody {
        stmts: vec![
            Stmt::LetElse {
                name: "note".to_string(),
                expr: "maybeNote".to_string(),
                error: conflict("id"),
            },
            return_of(Some("note")),
        ],
    };
    assert_eq!(
        check_stmt_body(
            &fixture_context(),
            &[maybe_note_param(), id_param()],
            Some(string_output()),
            &[],
            &body,
        ),
        Ok(())
    );
}

#[test]
fn check_stmt_let_else_requires_an_option_initializer() {
    let body = StmtBody {
        stmts: vec![
            Stmt::LetElse {
                name: "price".to_string(),
                expr: "trade.price".to_string(),
                error: conflict("id"),
            },
            return_of(Some("trade.price")),
        ],
    };
    let errors = check_stmt_body(
        &fixture_context(),
        &[trade_param(), id_param()],
        Some(double_output()),
        &[],
        &body,
    )
    .expect_err("non-option initializer");
    assert_eq!(errors.len(), 1, "{errors:?}");
    assert_eq!(errors[0].slot, "let-else 'price'");
    assert!(
        errors[0].message.contains("Option"),
        "{:?}",
        errors[0].message
    );
}

/// An assignment path walks class features from the local; an unknown
/// segment is an error naming the whole path.
#[test]
fn check_stmt_assign_rejects_unknown_path_feature() {
    let body = StmtBody {
        stmts: vec![
            Stmt::Let {
                name: "draft".to_string(),
                expr: "trade".to_string(),
            },
            Stmt::Assign {
                path: vec!["draft".to_string(), "nope".to_string()],
                expr: "1.0".to_string(),
            },
            return_of(Some("trade.price")),
        ],
    };
    let errors = check_stmt_body(
        &fixture_context(),
        &[trade_param()],
        Some(double_output()),
        &[],
        &body,
    )
    .expect_err("unknown feature");
    assert_eq!(errors.len(), 1, "{errors:?}");
    assert_eq!(errors[0].slot, "assign 'draft.nope'");
    assert!(
        errors[0].message.contains("unknown feature 'nope'"),
        "{:?}",
        errors[0].message
    );
}

/// The assignment root must be a local: parameters, receivers, and unknown
/// names are all rejected.
#[test]
fn check_stmt_assign_requires_a_local_root() {
    let body = StmtBody {
        stmts: vec![
            Stmt::Assign {
                path: vec!["trade".to_string(), "price".to_string()],
                expr: "1.0".to_string(),
            },
            Stmt::Assign {
                path: vec!["trades".to_string(), "price".to_string()],
                expr: "1.0".to_string(),
            },
            Stmt::Assign {
                path: vec!["mystery".to_string(), "price".to_string()],
                expr: "1.0".to_string(),
            },
            return_of(Some("trade.price")),
        ],
    };
    let errors = check_stmt_body(
        &fixture_context(),
        &[trade_param(), id_param()],
        Some(double_output()),
        &[trade_repository()],
        &body,
    )
    .expect_err("non-local roots");
    assert_eq!(errors.len(), 3, "{errors:?}");
    assert_eq!(errors[0].slot, "assign 'trade.price'");
    assert!(
        errors[0].message.contains("parameter 'trade'"),
        "{:?}",
        errors[0].message
    );
    assert_eq!(errors[1].slot, "assign 'trades.price'");
    assert!(
        errors[1].message.contains("receiver 'trades'"),
        "{:?}",
        errors[1].message
    );
    assert_eq!(errors[2].slot, "assign 'mystery.price'");
    assert!(
        errors[2].message.contains("unknown local 'mystery'"),
        "{:?}",
        errors[2].message
    );
}

/// The navigated target slot unifies exactly with the assigned expression:
/// scalars, nested class features, mismatches, and non-class segments.
#[test]
fn check_stmt_assign_types_the_target_slot() {
    let draft = || Stmt::Let {
        name: "draft".to_string(),
        expr: "trade".to_string(),
    };
    let good = StmtBody {
        stmts: vec![
            draft(),
            Stmt::Assign {
                path: vec!["draft".to_string(), "price".to_string()],
                expr: "trade.price * 2.0".to_string(),
            },
            Stmt::Assign {
                path: vec![
                    "draft".to_string(),
                    "counterparty".to_string(),
                    "name".to_string(),
                ],
                expr: "trade.note".to_string(),
            },
            return_of(Some("trade.price")),
        ],
    };
    assert_eq!(
        check_stmt_body(
            &fixture_context(),
            &[trade_param()],
            Some(double_output()),
            &[],
            &good,
        ),
        Ok(())
    );

    let mismatch = StmtBody {
        stmts: vec![
            draft(),
            Stmt::Assign {
                path: vec!["draft".to_string(), "price".to_string()],
                expr: "trade.note".to_string(),
            },
            return_of(Some("trade.price")),
        ],
    };
    let errors = check_stmt_body(
        &fixture_context(),
        &[trade_param()],
        Some(double_output()),
        &[],
        &mismatch,
    )
    .expect_err("assign mismatch");
    assert_eq!(errors.len(), 1, "{errors:?}");
    assert_eq!(errors[0].slot, "assign 'draft.price'");
    assert!(
        errors[0].message.contains("target slot is"),
        "{:?}",
        errors[0].message
    );

    let scalar_path = StmtBody {
        stmts: vec![
            draft(),
            Stmt::Assign {
                path: vec!["draft".to_string(), "price".to_string(), "x".to_string()],
                expr: "1.0".to_string(),
            },
            return_of(Some("trade.price")),
        ],
    };
    let errors = check_stmt_body(
        &fixture_context(),
        &[trade_param()],
        Some(double_output()),
        &[],
        &scalar_path,
    )
    .expect_err("navigate a scalar");
    assert_eq!(errors.len(), 1, "{errors:?}");
    assert_eq!(errors[0].slot, "assign 'draft.price.x'");
    assert!(
        errors[0].message.contains("not a class"),
        "{:?}",
        errors[0].message
    );
}

#[test]
fn check_stmt_guard_requires_boolean() {
    let body = StmtBody {
        stmts: vec![
            Stmt::Guard {
                expr: "trade.price".to_string(),
                error: conflict("trade.note"),
            },
            return_of(Some("trade.price")),
        ],
    };
    let errors = check_stmt_body(
        &fixture_context(),
        &[trade_param()],
        Some(double_output()),
        &[],
        &body,
    )
    .expect_err("non-boolean guard");
    assert_eq!(errors.len(), 1, "{errors:?}");
    assert_eq!(errors[0].slot, "guard 'trade.price'");
    assert!(
        errors[0].message.contains("must be boolean"),
        "{:?}",
        errors[0].message
    );
}

#[test]
fn check_stmt_guard_rejects_unknown_error_variant() {
    let body = StmtBody {
        stmts: vec![
            Stmt::Guard {
                expr: "trade.price > 0.0".to_string(),
                error: BodyRaise {
                    variant: "Exploded".to_string(),
                    args: Vec::new(),
                },
            },
            return_of(Some("trade.price")),
        ],
    };
    let errors = check_stmt_body(
        &fixture_context(),
        &[trade_param()],
        Some(double_output()),
        &[],
        &body,
    )
    .expect_err("unknown variant");
    assert_eq!(errors.len(), 1, "{errors:?}");
    assert_eq!(errors[0].slot, "guard 'trade.price > 0.0'");
    assert!(
        errors[0]
            .message
            .contains("unknown error variant `Exploded`"),
        "{:?}",
        errors[0].message
    );
}

#[test]
fn check_stmt_guard_rejects_error_arity() {
    let body = StmtBody {
        stmts: vec![
            Stmt::Guard {
                expr: "trade.price > 0.0".to_string(),
                error: not_found(&["id"]),
            },
            Stmt::Guard {
                expr: "trade.price > 0.0".to_string(),
                error: BodyRaise {
                    variant: "Conflict".to_string(),
                    args: vec!["id".to_string(), "id".to_string()],
                },
            },
            return_of(Some("trade.price")),
        ],
    };
    let errors = check_stmt_body(
        &fixture_context(),
        &[trade_param(), id_param()],
        Some(double_output()),
        &[],
        &body,
    )
    .expect_err("arity");
    assert_eq!(errors.len(), 2, "{errors:?}");
    assert!(
        errors[0].message.contains("`NotFound` expects 2 arguments"),
        "{:?}",
        errors[0].message
    );
    assert!(
        errors[1].message.contains("`Conflict` expects 1 argument"),
        "{:?}",
        errors[1].message
    );
}

#[test]
fn check_stmt_guard_rejects_non_scalar_error_arguments() {
    let body = StmtBody {
        stmts: vec![
            Stmt::Guard {
                expr: "trade.price > 0.0".to_string(),
                error: conflict("trade"),
            },
            return_of(Some("trade.price")),
        ],
    };
    let errors = check_stmt_body(
        &fixture_context(),
        &[trade_param()],
        Some(double_output()),
        &[],
        &body,
    )
    .expect_err("class argument");
    assert_eq!(errors.len(), 1, "{errors:?}");
    assert!(
        errors[0].message.contains("must be a scalar"),
        "{:?}",
        errors[0].message
    );
    assert!(
        errors[0].message.contains("class `Trade`"),
        "{:?}",
        errors[0].message
    );
}

/// Branch locals stay inside their branch; using one after the `if` is an
/// unknown name (and unreachable code is not diagnosed).
#[test]
fn check_stmt_if_branches_have_local_scope() {
    let body = StmtBody {
        stmts: vec![
            Stmt::If {
                cond: "trade.price > 0.0".to_string(),
                then: vec![
                    Stmt::Let {
                        name: "local".to_string(),
                        expr: "trade.price".to_string(),
                    },
                    return_of(Some("local")),
                ],
                else_: vec![return_of(Some("trade.price"))],
            },
            return_of(Some("local")),
        ],
    };
    let errors = check_stmt_body(
        &fixture_context(),
        &[trade_param()],
        Some(double_output()),
        &[],
        &body,
    )
    .expect_err("branch local escapes");
    assert_eq!(errors.len(), 1, "{errors:?}");
    assert_eq!(errors[0].slot, "return");
    assert!(
        errors[0].message.contains("unknown name `local`"),
        "{:?}",
        errors[0].message
    );
}

#[test]
fn check_stmt_if_requires_boolean_condition() {
    let body = StmtBody {
        stmts: vec![Stmt::If {
            cond: "trade.price".to_string(),
            then: vec![return_of(Some("trade.price"))],
            else_: vec![return_of(Some("0.0"))],
        }],
    };
    let errors = check_stmt_body(
        &fixture_context(),
        &[trade_param()],
        Some(double_output()),
        &[],
        &body,
    )
    .expect_err("non-boolean condition");
    assert_eq!(errors.len(), 1, "{errors:?}");
    assert_eq!(errors[0].slot, "if 'trade.price'");
    assert!(
        errors[0].message.contains("must be boolean"),
        "{:?}",
        errors[0].message
    );
}

/// `return` unifies with the output type lifted by its multiplicity: a
/// `(0..*)` output wants the list, a `(0..1)` output the option.
#[test]
fn check_stmt_return_types_against_output_multiplicity() {
    let list_return = StmtBody {
        stmts: vec![return_of(Some("tags"))],
    };
    assert_eq!(
        check_stmt_body(
            &fixture_context(),
            &[tags_param()],
            Some(many_string_output()),
            &[],
            &list_return,
        ),
        Ok(())
    );

    let element_return = StmtBody {
        stmts: vec![return_of(Some("id"))],
    };
    let errors = check_stmt_body(
        &fixture_context(),
        &[id_param()],
        Some(many_string_output()),
        &[],
        &element_return,
    )
    .expect_err("element into a many output");
    assert_eq!(errors.len(), 1, "{errors:?}");
    assert_eq!(errors[0].slot, "return");
    assert!(
        errors[0].message.contains("List<string>"),
        "{:?}",
        errors[0].message
    );

    let option_return = StmtBody {
        stmts: vec![return_of(Some("maybeNote"))],
    };
    assert_eq!(
        check_stmt_body(
            &fixture_context(),
            &[maybe_note_param()],
            Some(optional_string_output()),
            &[],
            &option_return,
        ),
        Ok(())
    );
}

/// Every successful path must return when the operation declares an output:
/// falling off the end is an error labeled `body`.
#[test]
fn check_stmt_missing_return_is_an_error() {
    let fell_through = StmtBody {
        stmts: vec![Stmt::Let {
            name: "price".to_string(),
            expr: "trade.price".to_string(),
        }],
    };
    let errors = check_stmt_body(
        &fixture_context(),
        &[trade_param()],
        Some(double_output()),
        &[],
        &fell_through,
    )
    .expect_err("missing return");
    assert_eq!(errors.len(), 1, "{errors:?}");
    assert_eq!(errors[0].slot, "body");
    assert!(
        errors[0].message.contains("missing return"),
        "{:?}",
        errors[0].message
    );

    // A guard's error exit does not make the success path exit.
    let guarded = StmtBody {
        stmts: vec![Stmt::Guard {
            expr: "trade.price > 0.0".to_string(),
            error: conflict("trade.note"),
        }],
    };
    let errors = check_stmt_body(
        &fixture_context(),
        &[trade_param()],
        Some(double_output()),
        &[],
        &guarded,
    )
    .expect_err("guard is not an exit");
    assert_eq!(errors.len(), 1, "{errors:?}");
    assert_eq!(errors[0].slot, "body");

    // An `if` exits only when both branches do; an empty `else` falls
    // through.
    let partial_if = StmtBody {
        stmts: vec![Stmt::If {
            cond: "trade.price > 0.0".to_string(),
            then: vec![return_of(Some("trade.price"))],
            else_: vec![],
        }],
    };
    let errors = check_stmt_body(
        &fixture_context(),
        &[trade_param()],
        Some(double_output()),
        &[],
        &partial_if,
    )
    .expect_err("partial if");
    assert_eq!(errors.len(), 1, "{errors:?}");
    assert_eq!(errors[0].slot, "body");

    // Both branches returning satisfies the flow rule.
    let both_return = StmtBody {
        stmts: vec![Stmt::If {
            cond: "trade.price > 0.0".to_string(),
            then: vec![return_of(Some("trade.price"))],
            else_: vec![return_of(Some("0.0"))],
        }],
    };
    assert_eq!(
        check_stmt_body(
            &fixture_context(),
            &[trade_param()],
            Some(double_output()),
            &[],
            &both_return,
        ),
        Ok(())
    );
}

/// A void operation needs no return and may fall off the end; returning a
/// value from it is an error.
#[test]
fn check_stmt_void_body_needs_no_return() {
    let body = StmtBody {
        stmts: vec![
            Stmt::Let {
                name: "draft".to_string(),
                expr: "trade".to_string(),
            },
            Stmt::Guard {
                expr: "trade.price > 0.0".to_string(),
                error: conflict("trade.note"),
            },
            Stmt::Assign {
                path: vec!["draft".to_string(), "price".to_string()],
                expr: "trade.price * 2.0".to_string(),
            },
        ],
    };
    assert_eq!(
        check_stmt_body(&fixture_context(), &[trade_param()], None, &[], &body),
        Ok(())
    );

    let returns_a_value = StmtBody {
        stmts: vec![return_of(Some("trade.price"))],
    };
    let errors = check_stmt_body(
        &fixture_context(),
        &[trade_param()],
        None,
        &[],
        &returns_a_value,
    )
    .expect_err("value from void");
    assert_eq!(errors.len(), 1, "{errors:?}");
    assert_eq!(errors[0].slot, "return");
    assert!(
        errors[0].message.contains("void"),
        "{:?}",
        errors[0].message
    );
}

/// A receiver is a class-typed root, so `<receiver>.<operation>(...)`
/// type-checks through the ordinary call machinery against the synthetic
/// repository class.
#[test]
fn check_stmt_binds_receivers_as_callable_classes() {
    let body = StmtBody {
        stmts: vec![
            Stmt::Let {
                name: "trade".to_string(),
                expr: "trades.findById(id)".to_string(),
            },
            return_of(Some("trade.price")),
        ],
    };
    assert_eq!(
        check_stmt_body(
            &fixture_context(),
            &[id_param()],
            Some(double_output()),
            &[trade_repository()],
            &body,
        ),
        Ok(())
    );

    // A misspelled operation is the ordinary call error, wrapped in the
    // statement's slot.
    let bad_call = StmtBody {
        stmts: vec![
            Stmt::Let {
                name: "trade".to_string(),
                expr: "trades.lookup(id)".to_string(),
            },
            return_of(Some("0.0")),
        ],
    };
    let errors = check_stmt_body(
        &fixture_context(),
        &[id_param()],
        Some(double_output()),
        &[trade_repository()],
        &bad_call,
    )
    .expect_err("unknown operation");
    assert_eq!(errors.len(), 1, "{errors:?}");
    assert_eq!(errors[0].slot, "let 'trade'");
    assert!(
        errors[0].message.contains("unknown operation"),
        "{:?}",
        errors[0].message
    );
}
