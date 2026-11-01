//! Inheritance/actor cycle detection through the verified `rex-verify`
//! kernel: `link(super, sub)` refuses exactly the edges that would close a
//! cycle, so a rejected link *is* a cycle. These tests pin the diagnostics
//! (message, file, span presence) the kernel-backed detectors emit.

use rex_driver::{compile_str, render, Severity};

fn diagnose(source: &str) -> Vec<String> {
    let compilation = compile_str("cycles.mox", source, &rex_driver::DomainImports::default());
    compilation
        .diagnostics
        .iter()
        .map(|d| d.message.clone())
        .collect()
}

#[test]
fn self_extending_class_is_a_cycle() {
    let source = r#"
package nz.example.cycles

class Loop extends Loop {
    int x
}
"#;
    let messages = diagnose(source);
    assert_eq!(messages.len(), 1, "{messages:?}");
    assert_eq!(messages[0], "inheritance cycle detected: Loop extends Loop");
}

#[test]
fn mutual_extends_reports_the_ring() {
    let source = r#"
package nz.example.cycles

class A extends B {
    int x
}

class B extends A {
    int y
}
"#;
    let messages = diagnose(source);
    assert_eq!(messages.len(), 1, "{messages:?}");
    assert_eq!(
        messages[0],
        "inheritance cycle detected: A extends B extends A"
    );
}

#[test]
fn transitive_extends_cycle_names_the_ring_from_its_first_declared_member() {
    let source = r#"
package nz.example.cycles

class A extends B {
    int x
}

class B extends C {
    int y
}

class C extends B {
    int z
}
"#;
    let messages = diagnose(source);
    assert_eq!(messages.len(), 1, "{messages:?}");
    // A and B are a chain into the B/C ring; the reported cycle is the ring.
    assert_eq!(
        messages[0],
        "inheritance cycle detected: B extends C extends B"
    );
}

#[test]
fn cross_package_extends_cycle_is_detected() {
    let source = r#"
package cycles

class A extends other.B {
    int x
}
"#;
    let other = r#"
package other

class B extends A {
    int y
}
"#;
    let compilation = rex_driver::compile_files(
        &[
            ("cycles.mox".to_string(), source.to_string()),
            ("other.mox".to_string(), other.to_string()),
        ],
        &rex_driver::DomainImports::default(),
    );
    let messages: Vec<String> = compilation
        .diagnostics
        .iter()
        .map(|(_, d)| d.message.clone())
        .collect();
    assert_eq!(messages.len(), 1, "{messages:?}");
    assert!(
        messages[0].starts_with("inheritance cycle detected: "),
        "{messages:?}"
    );
}

#[test]
fn diamond_extends_is_not_a_cycle() {
    // Multiple `extends` is legal; only cycles are refused.
    let source = r#"
package nz.example.cycles

class Base {
    int x
}

class Left extends Base {
    int y
}

class Right extends Base {
    int z
}

class Join extends Left, Right {
    int w
}
"#;
    let compilation = compile_str("cycles.mox", source, &rex_driver::DomainImports::default());
    assert!(
        compilation.diagnostics.is_empty(),
        "{}",
        render("cycles.mox", source, &compilation.diagnostics)
    );
}

#[test]
fn unknown_extends_target_is_not_reported_by_the_cycle_pass() {
    // A missing super is a resolution diagnostic elsewhere; the cycle pass
    // must not crash on it or report a phantom cycle.
    let source = r#"
package nz.example.cycles

class A extends Missing {
    int x
}
"#;
    let messages = diagnose(source);
    assert!(
        messages
            .iter()
            .all(|m| !m.starts_with("inheritance cycle detected:")),
        "{messages:?}"
    );
}

#[test]
fn actor_extends_forward_reference_is_an_unknown_actor_diagnostic() {
    // Actor parents must be declared before their children
    // (`lower_actors_decl` checks `actor_names` so far), so an extends ring
    // is unrepresentable in one block and the kernel-backed cycle pass is
    // a safety net for the union passes. A forward reference is the
    // unknown-actor error, characterised here.
    let source = r#"
actors Support {
    actor Customer extends Agent
    actor Sub extends Customer
    actor Agent extends Sub
}
"#;
    let compilation = rex_driver::compile_actors_str(
        "actors.actor",
        source,
        &[],
        &rex_driver::DomainImports::default(),
    );
    let messages: Vec<String> = compilation
        .diagnostics
        .iter()
        .map(|(_, d)| d.message.clone())
        .collect();
    assert!(
        messages
            .iter()
            .any(|m| m == "actor `Customer` extends unknown actor `Agent`"),
        "{messages:?}"
    );
}

#[test]
fn actor_chain_without_cycle_compiles() {
    let source = r#"
actors Support {
    actor Base
    actor Sub extends Base
}
"#;
    let compilation = rex_driver::compile_actors_str(
        "actors.actor",
        source,
        &[],
        &rex_driver::DomainImports::default(),
    );
    assert!(
        compilation.diagnostics.is_empty(),
        "{:?}",
        compilation.diagnostics
    );
}

#[test]
fn cycle_diagnostics_are_errors_that_block_the_model() {
    let source = r#"
package nz.example.cycles

class A extends B {
    int x
}

class B extends A {
    int y
}
"#;
    let compilation = compile_str("cycles.mox", source, &rex_driver::DomainImports::default());
    assert!(
        compilation.model.is_none(),
        "a cyclic model must not publish"
    );
    assert!(
        compilation
            .diagnostics
            .iter()
            .any(|d| d.severity == Severity::Error),
        "{:?}",
        compilation.diagnostics
    );
}
