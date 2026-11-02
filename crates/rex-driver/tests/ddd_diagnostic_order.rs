//! Characterization test for the diagnostic emission ORDER of
//! `compile_ddd_file`: one design engineered to trip diagnostics in every
//! stage — the prologue (import buckets), rule 1, the per-module machine
//! scopes, the per-declaration lowering stages (services, designs with
//! rules 2/3/4/5/12/14/15, searches with rule 11), and the deferred
//! validations (rules 11-binding, 6, 13, 9, 8, 10) — with the exact
//! sequence of severities and messages pinned.
//!
//! The stage decomposition of `compile_ddd_file` must keep this sequence
//! byte-identical: the goldens pin artifacts, this pins the interleaving.
//! The statement order inside `compile_ddd_file` (and now its stage
//! functions) is the spec — `docs/DDD.md`'s diagnostics chapter depends on
//! it.

use rex_driver::{compile_ddd_str, compile_ddd_str_with_actors, DddCompilation, Severity};
use rex_ir::{ActorModel, ActorsDef, CapabilityDef, TypeRef};

const DOMAIN: &str = r#"
package nz.example.order

enum Status {
    Draft as "D" = 0
}

class Cart {
    String label
    contains Item[] items opposite cart
}

class Item {
    container Cart cart opposite items
    String sku
    Status state
    contains Chapter[] chapters opposite item
}

class Chapter {
    container Item item opposite chapters
    String heading
}

class PhysicalMedia {
    String status
    refers Chapter media
}

class Money {
    int cents
}

class Report {
    String title
    op String render() {
        expr { title }
    }
}
"#;

/// One design tripping every stage: the prologue (`missing.mox`), rule 1
/// (`base nz.example.nowhere`), module `alpha` (machine-scope ambiguity,
/// service-condition scope, rule 7 signatures, rules 2/3/4/5/12/14/15 on
/// the designs, rule 11 on the searches), module `beta` (duplicate
/// service/design, delegations, the rule-11 cross-module target), and a
/// duplicate `alpha` (rule 2 on module names). The deferred rules 6/13/9/8
/// fire after every module.
const DESIGN: &str = r#"
import "order.mox";
import "missing.mox";

application Order {
    base nz.example.nowhere

    module alpha {
        service Checkout {
            inject UnknownThing;
            inject ItemRepository;
            String grab(Mystery thing);
            protected save => ItemRepository.save;
            String twice();
            String twice();
            String zap() when (state == Open);
        }

        entity Ghost

        value Money
        value Money nonPersistent
        dto Money nonPersistent
        value Money scaffold !cache
        value Money repository MoneyRepository {
            findById;
        }
        entity Cart nonPersistent scaffold
        machine status: Status {
            initial Open;
            Open -> Held;
            terminal Held;
        }
        entity Item scaffold repository ItemRepository {
            findById;
            save;
        }
        machine state: Status {
            initial Open;
            Open -> Held;
            terminal Held;
        }
        entity Money

        search AlphaSearch {
            entity Item
            text {
                ghost
                sku boost 0 analyzer ""
            }
            filters {
                cart
            }
            document {
                dup = sku;
                dup = sku;
                bad = sku +;
            }
            pagination {
                limit 100
                max 10
            }
        }
        search BetaSearch {
            entity Money
        }
        search GammaSearch {
            entity Report
        }
    }

    module beta {
        entity Report
        service Checkout {
            String go() capability UnknownCapability;
            finish => Checkout.nope;
            hop => GhostService.save;
        }
        entity Money
    }

    module alpha {
    }
}
"#;

fn compile() -> DddCompilation {
    compile_ddd_str(
        "design.ddd",
        DESIGN,
        &[("order.mox".to_string(), DOMAIN.to_string())],
        &rex_driver::DomainImports::default(),
    )
}

fn compile_with_actors(actors: &ActorModel) -> DddCompilation {
    compile_ddd_str_with_actors(
        "design.ddd",
        DESIGN,
        &[("order.mox".to_string(), DOMAIN.to_string())],
        &rex_driver::DomainImports::default(),
        actors,
    )
}

fn actors_with(capabilities: &[&str]) -> ActorModel {
    let mut block = ActorsDef::new("Ops");
    for name in capabilities {
        block = block.capability(CapabilityDef::new(
            *name,
            TypeRef::Class {
                package: "nz.example.order".to_string(),
                name: "Item".to_string(),
            },
        ));
    }
    ActorModel::new().block(block)
}

/// The pinned emission order as `(severity, message)` pairs, ignoring the
/// per-file path tags (every diagnostic here belongs to `design.ddd`).
fn sequence(compilation: &DddCompilation) -> Vec<(Severity, String)> {
    compilation
        .diagnostics
        .iter()
        .map(|(path, diagnostic)| {
            assert_eq!(path, "design.ddd", "unexpected source file");
            (diagnostic.severity, diagnostic.message.clone())
        })
        .collect()
}

#[test]
fn the_diagnostic_emission_order_is_stable() {
    let compilation = compile();
    let actual = sequence(&compilation);
    // Filled from the current behavior — this test characterizes the
    // emission order so the stage decomposition of `compile_ddd_file`
    // cannot perturb it. The stages in order: the prologue (import
    // buckets), rule 1, module `alpha`'s machine scopes / service-condition
    // scope / services / designs / searches, modules `beta` and the
    // duplicate `alpha`, then the deferred validations (rule 11 binding,
    // rule 6, rule 13, rule 9, rule 8).
    let expected: Vec<(Severity, String)> = vec![
                    (Severity::Error, "imported file \"missing.mox\" was not provided".to_string()),
                    (Severity::Error, "unknown base package 'nz.example.nowhere'".to_string()),
                    (Severity::Error, "ambiguous state name 'Open' in module 'alpha': it is declared by more than one machine, so conditions cannot bind it".to_string()),
                    (Severity::Error, "ambiguous state name 'Held' in module 'alpha': it is declared by more than one machine, so conditions cannot bind it".to_string()),
                    (Severity::Error, "operation 'zap' of service 'Checkout' carries a `when` condition, but module 'alpha' does not have exactly one machine-bearing design to scope it".to_string()),
                    (Severity::Error, "unknown type 'Mystery'".to_string()),
                    (Severity::Error, "duplicate operation 'twice' in service 'Checkout'".to_string()),
                    (Severity::Error, "invalid condition in operation 'zap' of service 'Checkout': unknown name `state`".to_string()),
                    (Severity::Error, "unknown type 'Ghost'".to_string()),
                    (Severity::Error, "value design 'Money': feature 'cents' is not readonly; a value object is immutable".to_string()),
                    (Severity::Error, "duplicate design for class 'Money'".to_string()),
                    (Severity::Error, "value design 'Money': feature 'cents' is not readonly; a value object is immutable".to_string()),
                    (Severity::Error, "duplicate design for class 'Money'".to_string()),
                    (Severity::Error, "duplicate design for class 'Money'".to_string()),
                    (Severity::Error, "'!cache' is meaningless; 'cache' defaults to off".to_string()),
                    (Severity::Error, "flag 'scaffold' requires an entity design; 'Money' is designed as a value".to_string()),
                    (Severity::Error, "value design 'Money': feature 'cents' is not readonly; a value object is immutable".to_string()),
                    (Severity::Error, "duplicate design for class 'Money'".to_string()),
                    (Severity::Error, "value design 'Money': feature 'cents' is not readonly; a value object is immutable".to_string()),
                    (Severity::Error, "repository 'MoneyRepository' requires an entity design; 'Money' is designed as a value".to_string()),
                    (Severity::Error, "flag 'nonPersistent' requires a value or dto design; 'Cart' is designed as an entity".to_string()),
                    (Severity::Error, "scaffolded entity 'Cart' declares no repository".to_string()),
                    (Severity::Error, "machine 'status' binds feature 'status' of 'Cart', which does not exist".to_string()),
                    (Severity::Error, "duplicate design for class 'Money'".to_string()),
                    (Severity::Error, "unknown text field 'ghost' on 'Item'".to_string()),
                    (Severity::Error, "boost of text field 'sku' must be at least 1".to_string()),
                    (Severity::Error, "analyzer of text field 'sku' must not be empty".to_string()),
                    (Severity::Error, "filter 'cart' is a class reference; only value features may filter".to_string()),
                    (Severity::Error, "duplicate document field 'dup' in search 'AlphaSearch'".to_string()),
                    (Severity::Error, "invalid document expression in field 'bad': unexpected trailing input".to_string()),
                    (Severity::Error, "pagination limit 100 exceeds the max 10".to_string()),
                    (Severity::Error, "duplicate service 'Checkout'".to_string()),
                    (Severity::Error, "duplicate design for class 'Money'".to_string()),
                    (Severity::Error, "duplicate module 'alpha'".to_string()),
                    (Severity::Error, "search 'GammaSearch' targets 'Report', which has no entity design in module 'alpha'".to_string()),
                    (Severity::Error, "entity 'Item' is contained by 'Cart'; only aggregate roots may declare a repository".to_string()),
                    (Severity::Error, "cross reference 'PhysicalMedia.media' to 'Chapter' escapes its aggregate rooted at 'Cart'".to_string()),
                    (Severity::Error, "unknown dependency 'UnknownThing'".to_string()),
                    (Severity::Error, "service 'Checkout' has no operation 'nope'".to_string()),
                    (Severity::Error, "unknown delegation target 'GhostService'".to_string()),
    ];
    assert_eq!(
        actual,
        expected,
        "diagnostic sequence drifted:\nactual:\n{}",
        actual
            .iter()
            .map(|(severity, message)| format!("    ({severity:?}, {message:?}),"))
            .collect::<Vec<_>>()
            .join("\n")
    );
}

#[test]
fn rule_10_runs_last_and_only_with_actors() {
    let plain = compile();
    let with_actors = compile_with_actors(&actors_with(&["KnownCapability"]));

    // Without an actor model the capability names are recorded unvalidated:
    // the plain sequence is the actors sequence minus the final capability
    // diagnostics.
    let mut expected = sequence(&with_actors);
    let tail = expected.split_off(expected.len() - 1);
    assert_eq!(sequence(&plain), expected);
    assert!(
        tail.iter()
            .all(|(_, message)| message.contains("unknown capability")),
        "the split-off tail must be the rule-10 diagnostics: {tail:?}"
    );
}
