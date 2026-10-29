//! Validation-rule tests for `.ddd` state machines: every rule the driver
//! enforces on a `machine` block, one failing design per rule, plus the
//! guard/condition binding contract (states bind as enum values — a guard
//! referencing an undeclared name or a foreign machine's state is a type
//! error, and the string-literal "magic string" form is rejected by the
//! shared expression language's same-type equality rule).

use rex_driver::{compile_ddd_str, Diagnostic, DomainImports};

/// Compiles a design against the payrun domain (the canonical fixture's
/// domain: `PayRun.status: PayRunStatus`, `PayRun.total: int`) and returns
/// the design-file diagnostics (domains are expected to compile clean).
fn diagnose(design: &str) -> Vec<Diagnostic> {
    const DOMAIN: &str = include_str!("../../../tests/conformance/ddd/payrun.mox");
    let compilation = compile_ddd_str(
        "tests/conformance/ddd/test.ddd",
        design,
        &[(
            "tests/conformance/ddd/payrun.mox".to_string(),
            DOMAIN.to_string(),
        )],
        &DomainImports::default(),
    );
    compilation
        .diagnostics
        .into_iter()
        .filter(|(path, _)| path.ends_with(".ddd"))
        .map(|(_, diagnostic)| diagnostic)
        .collect()
}

fn assert_error(design: &str, expected: &str) {
    let diagnostics = diagnose(design);
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains(expected)),
        "expected a diagnostic containing {expected:?}, got: {diagnostics:?}"
    );
}

fn assert_clean(design: &str) {
    let diagnostics = diagnose(design);
    assert!(
        diagnostics
            .iter()
            .all(|diagnostic| diagnostic.severity != rex_driver::diagnostic::Severity::Error),
        "expected no errors, got: {diagnostics:?}"
    );
}

const HEAD: &str = r#"
import "payrun.mox";

application Payroll {
    base nz.example.payroll
    module payroll {
"#;

const TAIL: &str = r#"
    }
}
"#;

fn design(body: &str) -> String {
    format!("{HEAD}{body}{TAIL}")
}

#[test]
fn the_canonical_machine_compiles_clean() {
    assert_clean(&design(
        r#"
        entity PayRun repository PayRunRepository {
            findById;
            save;
        }
        machine status: PayRunStatus {
            initial Draft;
            Draft -> Calculating via Calculate;
            Calculating -> AwaitingApproval via Submit;
            AwaitingApproval -> Approved via Approve when (total > 0);
            Approved -> Processing via Schedule;
            Processing -> Posted via Post;
            Draft -> Reversed;
            terminal Reversed;
            terminal Posted;
        }
"#,
    ));
}

#[test]
fn a_machine_requires_an_entity_design() {
    assert_error(
        &design(
            r#"
        value PayRun
        machine status: PayRunStatus {
            initial Draft;
            Draft -> Reversed;
            terminal Reversed;
        }
"#,
        ),
        "requires an entity design",
    );
}

#[test]
fn a_machine_type_must_be_an_enum() {
    assert_error(
        &design(
            r#"
        entity PayRun
        machine status: int {
            initial Draft;
            Draft -> Reversed;
            terminal Reversed;
        }
"#,
        ),
        "must bind an enum",
    );
}

#[test]
fn a_machine_binds_an_existing_enum_typed_feature() {
    assert_error(
        &design(
            r#"
        entity PayRun
        machine billingStatus: PayRunStatus {
            initial Draft;
            Draft -> Reversed;
            terminal Reversed;
        }
"#,
        ),
        "binds feature 'billingStatus' of 'PayRun', which does not exist",
    );
    assert_error(
        &design(
            r#"
        entity PayRun
        machine total: PayRunStatus {
            initial Draft;
            Draft -> Reversed;
            terminal Reversed;
        }
"#,
        ),
        "carries a different type",
    );
}

#[test]
fn a_machine_declares_exactly_one_initial_state() {
    assert_error(
        &design(
            r#"
        entity PayRun
        machine status: PayRunStatus {
            Draft -> Reversed;
            terminal Reversed;
        }
"#,
        ),
        "declares no initial state",
    );
}

#[test]
fn terminal_states_are_declared_once_and_take_no_outgoing_edges() {
    assert_error(
        &design(
            r#"
        entity PayRun
        machine status: PayRunStatus {
            initial Draft;
            Draft -> Reversed;
            terminal Reversed;
            terminal Reversed;
        }
"#,
        ),
        "duplicate terminal state 'Reversed'",
    );
    assert_error(
        &design(
            r#"
        entity PayRun
        machine status: PayRunStatus {
            initial Draft;
            Draft -> Reversed;
            Reversed -> Draft;
            terminal Reversed;
        }
"#,
        ),
        "terminal state 'Reversed' takes a transition",
    );
}

#[test]
fn duplicate_edges_and_clashing_triggers_are_rejected() {
    assert_error(
        &design(
            r#"
        entity PayRun
        machine status: PayRunStatus {
            initial Draft;
            Draft -> Reversed;
            Draft -> Reversed;
            terminal Reversed;
        }
"#,
        ),
        "duplicate transition 'Draft -> Reversed'",
    );
    assert_error(
        &design(
            r#"
        entity PayRun
        machine status: PayRunStatus {
            initial Draft;
            Draft -> Calculating via Calculate;
            Draft -> Reversed via Calculate;
            Calculating -> Reversed;
            terminal Reversed;
        }
"#,
        ),
        "trigger 'Calculate' leaves state 'Draft' more than once",
    );
}

#[test]
fn every_state_is_reachable_and_non_terminal_states_take_an_edge() {
    assert_error(
        &design(
            r#"
        entity PayRun
        machine status: PayRunStatus {
            initial Draft;
            Draft -> Reversed;
            terminal Reversed;
            terminal Posted;
        }
"#,
        ),
        "state 'Posted' is unreachable",
    );
    assert_error(
        &design(
            r#"
        entity PayRun
        machine status: PayRunStatus {
            initial Draft;
            Draft -> Calculating;
            Draft -> Reversed;
            terminal Reversed;
        }
"#,
        ),
        "state 'Calculating' takes no transition and is not terminal",
    );
}

#[test]
fn guards_bind_declared_states_and_type_check_against_the_class() {
    // A state reference is an enum-typed binding: comparing the bound
    // feature works, an unknown name is a type error.
    assert_clean(&design(
        r#"
        entity PayRun repository PayRunRepository {
            PayRun[] findByState(String state) when (status == Posted);
        }
        machine status: PayRunStatus {
            initial Draft;
            Processing -> Posted via Post;
            Draft -> Processing;
            terminal Posted;
        }
"#,
    ));
    assert_error(
        &design(
            r#"
        entity PayRun repository PayRunRepository {
            PayRun[] findByState(String state) when (status == Shelved);
        }
        machine status: PayRunStatus {
            initial Draft;
            Processing -> Posted via Post;
            Draft -> Processing;
            terminal Posted;
        }
"#,
        ),
        "invalid condition",
    );
    // The magic-string form is not typed either: an enum compares with an
    // enum value, not with a string literal (the shared expression
    // language's same-type equality rule).
    assert_error(
        &design(
            r#"
        entity PayRun repository PayRunRepository {
            PayRun[] findByState(String state) when (status == "Posted");
        }
        machine status: PayRunStatus {
            initial Draft;
            Processing -> Posted via Post;
            Draft -> Processing;
            terminal Posted;
        }
"#,
        ),
        "invalid condition",
    );
    // A guard comparing a feature of the wrong type to a state binding is a
    // type error: `total` is `int`, the states are `PayRunStatus` values.
    assert_error(
        &design(
            r#"
        entity PayRun
        machine status: PayRunStatus {
            initial Draft;
            Draft -> Reversed when (total == Draft);
            terminal Reversed;
        }
"#,
        ),
        "cannot compare",
    );
}

#[test]
fn service_conditions_scope_to_the_module_machine() {
    assert_clean(&design(
        r#"
        service PayRunService {
            boolean approve() capability ApprovePayRun when (status == AwaitingApproval);
        }
        entity PayRun
        machine status: PayRunStatus {
            initial Draft;
            Calculating -> AwaitingApproval via Submit;
            AwaitingApproval -> Approved via Approve;
            Approved -> Processing via Schedule;
            Processing -> Posted via Post;
            Draft -> Calculating;
            Draft -> Reversed;
            terminal Reversed;
            terminal Posted;
        }
"#,
    ));
    // A service condition without a machine-bearing design in the module is
    // undecidable.
    assert_error(
        &design(
            r#"
        service PayRunService {
            boolean approve() when (status == AwaitingApproval);
        }
        entity PayRun
"#,
        ),
        "does not have exactly one machine-bearing design",
    );
    // A service condition referencing a feature the machine's class does
    // not carry is a type error.
    assert_error(
        &design(
            r#"
        service PayRunService {
            boolean approve() when (billingStatus == AwaitingApproval);
        }
        entity PayRun
        machine status: PayRunStatus {
            initial Draft;
            Draft -> Reversed;
            terminal Reversed;
        }
"#,
        ),
        "invalid condition",
    );
}
