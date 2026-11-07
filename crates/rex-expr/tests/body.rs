//! The Rune statement IR ([`rex_expr::body`]): canonical wire shape and the
//! body typing rules (RUNE-BODIES stage 1).
//!
//! The canonical JSON literal below is the byte-exact wire contract for the
//! `"rune"` body target: key order follows declaration order, empty vecs and
//! unnamed guards are omitted, and step `path`s are omitted when empty.

use rex_expr::body::{
    check_body, BodyBinding, BodyDispatch, BodyGuard, BodyOutput, BodyStep, GuardPhase, RuneBody,
};
use rex_expr::{DomainTypes, NamedKind};
use rex_ir::{Feature, FeatureKind, Multiplicity, PrimitiveType, TypeRef};

/// The functions fixture's domain shape: a `notional double` input, a
/// `rate double` output, a class with a `price` feature, and an enum.
fn fixture_context() -> DomainTypes {
    let mut context = DomainTypes::default();
    context.insert_class(
        "pricing",
        "Instrument",
        vec![],
        vec![Feature::new(
            "price",
            FeatureKind::Attribute,
            TypeRef::Primitive(PrimitiveType::Double),
            Multiplicity::REQUIRED,
        )],
        vec![],
    );
    context.insert_named("pricing", "RateType", NamedKind::Enum);
    context
}

fn string_param(name: &str, multiplicity: Option<Multiplicity>) -> rex_ir::OperationParam {
    rex_ir::OperationParam {
        name: name.to_string(),
        type_: TypeRef::Primitive(PrimitiveType::String),
        multiplicity,
    }
}

fn double_param(name: &str) -> rex_ir::OperationParam {
    rex_ir::OperationParam {
        name: name.to_string(),
        type_: TypeRef::Primitive(PrimitiveType::Double),
        multiplicity: None,
    }
}

fn rate_output() -> BodyOutput<'static> {
    BodyOutput {
        name: "rate",
        type_: &TypeRef::Primitive(PrimitiveType::Double),
        multiplicity: None,
    }
}

/// The funcs-fixture shape: `condition PositiveNotional: notional > 0.0` and
/// `set rate: notional`. The serialized form is the pinned wire literal.
#[test]
fn rune_body_serializes_to_the_pinned_canonical_json() {
    let body = RuneBody {
        guards: vec![BodyGuard {
            expr: "notional > 0.0".to_string(),
            name: Some("PositiveNotional".to_string()),
            phase: GuardPhase::Pre,
        }],
        bindings: Vec::new(),
        steps: vec![BodyStep::Set {
            root: "rate".to_string(),
            path: Vec::new(),
            expr: "notional".to_string(),
        }],
    };
    let json = serde_json::to_string(&body).expect("serialize");
    assert_eq!(
        json,
        "{\"guards\":[{\"expr\":\"notional > 0.0\",\"name\":\"PositiveNotional\",\"phase\":\"pre\"}],\"steps\":[{\"kind\":\"set\",\"root\":\"rate\",\"expr\":\"notional\"}]}"
    );
    let round: RuneBody = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(round, body);
}

/// Full shape: a post guard without a name serializes without the `name`
/// key, a binding emits `expr` then `name`, an `add` step tags `"kind":
/// "add"` and carries its `->` path.
#[test]
fn rune_body_serializes_unnamed_guards_bindings_and_add_paths() {
    let body = RuneBody {
        guards: vec![BodyGuard {
            expr: "base > 0.0".to_string(),
            name: None,
            phase: GuardPhase::Post,
        }],
        bindings: vec![BodyBinding {
            expr: "notional".to_string(),
            name: "base".to_string(),
        }],
        steps: vec![BodyStep::Add {
            root: "ids".to_string(),
            path: vec!["leg".to_string()],
            expr: "legId".to_string(),
        }],
    };
    let json = serde_json::to_string(&body).expect("serialize");
    assert_eq!(
        json,
        "{\"guards\":[{\"expr\":\"base > 0.0\",\"phase\":\"post\"}],\"bindings\":[{\"expr\":\"notional\",\"name\":\"base\"}],\"steps\":[{\"kind\":\"add\",\"root\":\"ids\",\"path\":[\"leg\"],\"expr\":\"legId\"}]}"
    );
}

/// The happy path mirrors the funcs fixture: a boolean pre guard over the
/// inputs and a `set` of the output slot to an input.
#[test]
fn check_body_accepts_the_funcs_fixture_shape() {
    let body = RuneBody {
        guards: vec![BodyGuard {
            expr: "notional > 0.0".to_string(),
            name: Some("PositiveNotional".to_string()),
            phase: GuardPhase::Pre,
        }],
        bindings: Vec::new(),
        steps: vec![BodyStep::Set {
            root: "rate".to_string(),
            path: Vec::new(),
            expr: "notional".to_string(),
        }],
    };
    let checked = check_body(
        &fixture_context(),
        &[double_param("notional")],
        Some(rate_output()),
        None,
        &body,
    );
    assert_eq!(checked, Ok(()));
}

/// The dispatch head's value binds as an enum-typed name in the pre-guard
/// scope, so the synthetic `attr == VALUE` guard types (the machine-state
/// precedent: literals are bare enum-typed names).
#[test]
fn check_body_binds_the_dispatch_value_in_pre_guards() {
    let body = RuneBody {
        guards: vec![BodyGuard {
            expr: "t == FIRST".to_string(),
            name: Some("FIRST".to_string()),
            phase: GuardPhase::Pre,
        }],
        bindings: Vec::new(),
        steps: vec![BodyStep::Set {
            root: "r".to_string(),
            path: Vec::new(),
            expr: "1".to_string(),
        }],
    };
    let params = vec![rex_ir::OperationParam {
        name: "t".to_string(),
        type_: TypeRef::Enum {
            package: "pricing".to_string(),
            name: "RateType".to_string(),
        },
        multiplicity: None,
    }];
    let checked = check_body(
        &fixture_context(),
        &params,
        Some(BodyOutput {
            name: "r",
            type_: &TypeRef::Primitive(PrimitiveType::Int),
            multiplicity: None,
        }),
        Some(BodyDispatch {
            input: "t",
            enum_type: &TypeRef::Enum {
                package: "pricing".to_string(),
                name: "RateType".to_string(),
            },
            value: "FIRST",
        }),
        &body,
    );
    assert_eq!(checked, Ok(()));
}

/// A guard that does not type boolean is a slot error naming the guard.
#[test]
fn check_body_rejects_a_non_boolean_guard() {
    let body = RuneBody {
        guards: vec![BodyGuard {
            expr: "notional + 1.0".to_string(),
            name: Some("Positive".to_string()),
            phase: GuardPhase::Pre,
        }],
        bindings: Vec::new(),
        steps: Vec::new(),
    };
    let errors = check_body(
        &fixture_context(),
        &[double_param("notional")],
        Some(rate_output()),
        None,
        &body,
    )
    .expect_err("non-boolean guard");
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].slot, "guard 'Positive'");
    assert!(
        errors[0].message.contains("must be boolean"),
        "{:?}",
        errors[0].message
    );
}

/// An unknown name inside any slot is a slot error (did the typo stick to
/// the failing slot).
#[test]
fn check_body_rejects_unknown_names_in_guards_and_steps() {
    let body = RuneBody {
        guards: vec![BodyGuard {
            expr: "notionaL > 0.0".to_string(),
            name: Some("Positive".to_string()),
            phase: GuardPhase::Pre,
        }],
        bindings: Vec::new(),
        steps: vec![BodyStep::Set {
            root: "rate".to_string(),
            path: Vec::new(),
            expr: "nope".to_string(),
        }],
    };
    let errors = check_body(
        &fixture_context(),
        &[double_param("notional")],
        Some(rate_output()),
        None,
        &body,
    )
    .expect_err("unknown names");
    assert_eq!(errors.len(), 2);
    assert_eq!(errors[0].slot, "guard 'Positive'");
    assert!(
        errors[0].message.contains("unknown name"),
        "{:?}",
        errors[0]
    );
    assert_eq!(errors[1].slot, "set 'rate'");
}

/// `add` requires a list-valued target slot.
#[test]
fn check_body_rejects_add_to_a_scalar_output() {
    let body = RuneBody {
        guards: Vec::new(),
        bindings: Vec::new(),
        steps: vec![BodyStep::Add {
            root: "rate".to_string(),
            path: Vec::new(),
            expr: "notional".to_string(),
        }],
    };
    let errors = check_body(
        &fixture_context(),
        &[double_param("notional")],
        Some(rate_output()),
        None,
        &body,
    )
    .expect_err("add to scalar");
    assert_eq!(errors[0].slot, "add 'rate'");
    assert!(
        errors[0].message.contains("list"),
        "{:?}",
        errors[0].message
    );
}

/// `add` appends the element type of the target list; a `set` must unify
/// exactly with the target slot, including through a `->` path.
#[test]
fn check_body_types_add_elements_and_set_paths_against_the_output_slot() {
    let ids_output = BodyOutput {
        name: "ids",
        type_: &TypeRef::Primitive(PrimitiveType::String),
        multiplicity: Some(&Multiplicity {
            lower: 0,
            upper: rex_ir::Upper::Unbounded,
        }),
    };
    let good = RuneBody {
        guards: Vec::new(),
        bindings: Vec::new(),
        steps: vec![BodyStep::Add {
            root: "ids".to_string(),
            path: Vec::new(),
            expr: "legId".to_string(),
        }],
    };
    assert_eq!(
        check_body(
            &fixture_context(),
            &[string_param("legId", None)],
            Some(ids_output),
            None,
            &good
        ),
        Ok(())
    );

    let params = vec![rex_ir::OperationParam {
        name: "trade".to_string(),
        type_: TypeRef::Class {
            package: "pricing".to_string(),
            name: "Instrument".to_string(),
        },
        multiplicity: None,
    }];
    // A class-typed output slot navigates: `set quote->price: 1.0` writes
    // the double feature.
    let class_output = BodyOutput {
        name: "quote",
        type_: &TypeRef::Class {
            package: "pricing".to_string(),
            name: "Instrument".to_string(),
        },
        multiplicity: None,
    };
    let navigated = RuneBody {
        guards: Vec::new(),
        bindings: Vec::new(),
        steps: vec![BodyStep::Set {
            root: "quote".to_string(),
            path: vec!["price".to_string()],
            expr: "1.0".to_string(),
        }],
    };
    assert_eq!(
        check_body(
            &fixture_context(),
            &params,
            Some(class_output),
            None,
            &navigated
        ),
        Ok(())
    );

    // `set rate: trade` is a class, not the double slot.
    let mismatch = RuneBody {
        guards: Vec::new(),
        bindings: Vec::new(),
        steps: vec![BodyStep::Set {
            root: "rate".to_string(),
            path: Vec::new(),
            expr: "trade".to_string(),
        }],
    };
    let errors = check_body(
        &fixture_context(),
        &params,
        Some(rate_output()),
        None,
        &mismatch,
    )
    .expect_err("class into double");
    assert_eq!(errors[0].slot, "set 'rate'");

    // Navigating a scalar slot is a slot error (`set rate->price` — rate is
    // a double, not a class).
    let bad_path = RuneBody {
        guards: Vec::new(),
        bindings: Vec::new(),
        steps: vec![BodyStep::Set {
            root: "rate".to_string(),
            path: vec!["price".to_string()],
            expr: "1.0".to_string(),
        }],
    };
    let errors = check_body(
        &fixture_context(),
        &params,
        Some(rate_output()),
        None,
        &bad_path,
    )
    .expect_err("navigate a scalar");
    assert_eq!(errors[0].slot, "set 'rate->price'");
}

/// Scope visibility: steps see the bindings; a binding sees only the inputs
/// and its earlier siblings; pre guards see the inputs but not the output
/// slot; post guards see everything.
#[test]
fn check_body_threads_the_scopes_in_declaration_order() {
    let body = RuneBody {
        guards: vec![
            BodyGuard {
                expr: "rate > 0.0".to_string(),
                name: Some("TooEarly".to_string()),
                phase: GuardPhase::Pre,
            },
            BodyGuard {
                expr: "base >= 0.0 && rate > 0.0".to_string(),
                name: Some("SeesAll".to_string()),
                phase: GuardPhase::Post,
            },
        ],
        bindings: vec![
            BodyBinding {
                expr: "notional".to_string(),
                name: "base".to_string(),
            },
            BodyBinding {
                expr: "later".to_string(),
                name: "forward".to_string(),
            },
        ],
        steps: vec![BodyStep::Set {
            root: "rate".to_string(),
            path: Vec::new(),
            expr: "base".to_string(),
        }],
    };
    let errors = check_body(
        &fixture_context(),
        &[double_param("notional")],
        Some(rate_output()),
        None,
        &body,
    )
    .expect_err("scope violations");
    assert_eq!(errors.len(), 2, "{errors:?}");
    assert_eq!(errors[0].slot, "guard 'TooEarly'");
    assert!(
        errors[0].message.contains("unknown name `rate`"),
        "{:?}",
        errors[0].message
    );
    assert_eq!(errors[1].slot, "binding 'forward'");
}

/// A step root must be the output slot or a binding (the sigil AST's
/// assign-root rule); a path must name real features on the way.
#[test]
fn check_body_rejects_unknown_roots_and_unknown_path_features() {
    let params = vec![rex_ir::OperationParam {
        name: "trade".to_string(),
        type_: TypeRef::Class {
            package: "pricing".to_string(),
            name: "Instrument".to_string(),
        },
        multiplicity: None,
    }];
    let body = RuneBody {
        guards: Vec::new(),
        bindings: Vec::new(),
        steps: vec![
            BodyStep::Set {
                root: "mystery".to_string(),
                path: Vec::new(),
                expr: "1.0".to_string(),
            },
            BodyStep::Set {
                root: "rate".to_string(),
                path: vec!["nope".to_string()],
                expr: "1.0".to_string(),
            },
        ],
    };
    let errors = check_body(
        &fixture_context(),
        &params,
        Some(rate_output()),
        None,
        &body,
    )
    .expect_err("bad roots");
    assert_eq!(errors.len(), 2);
    assert_eq!(errors[0].slot, "set 'mystery'");
    assert_eq!(errors[1].slot, "set 'rate->nope'");
}
