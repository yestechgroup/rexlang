//! Validation-rule tests for `.ddd` state machines: every rule the driver
//! enforces on a `machine` block, one failing design per rule, plus the
//! guard/condition binding contract (states bind as enum values — a guard
//! referencing an undeclared name or a foreign machine's state is a type
//! error, and the string-literal "magic string" form is rejected by the
//! shared expression language's same-type equality rule). The
//! typed-lifecycle rules close the no-machine cases: a bare-`String`
//! status-like field on a designed entity is an error, and guard literals
//! against the machine field must be machine states.

use rex_driver::{compile_ddd_str, Diagnostic, DomainImports};

/// The canonical fixture's domain: `PayRun.status: PayRunStatus`,
/// `PayRun.total: int`.
const PAYRUN_DOMAIN: &str = include_str!("../../../tests/conformance/ddd/payrun.mox");

/// Compiles a design against a caller-provided domain source under the
/// same path key (the design's `import "payrun.mox"` resolves lexically to
/// it) and returns the design-file diagnostics (domains are expected to
/// compile clean).
fn diagnose_with(design: &str, domain: &str) -> Vec<Diagnostic> {
    let compilation = compile_ddd_str(
        "tests/conformance/ddd/test.ddd",
        design,
        &[(
            "tests/conformance/ddd/payrun.mox".to_string(),
            domain.to_string(),
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
    assert_error_with(design, PAYRUN_DOMAIN, expected);
}

fn assert_error_with(design: &str, domain: &str, expected: &str) {
    let diagnostics = diagnose_with(design, domain);
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains(expected)),
        "expected a diagnostic containing {expected:?}, got: {diagnostics:?}"
    );
}

fn assert_clean(design: &str) {
    assert_clean_with(design, PAYRUN_DOMAIN);
}

fn assert_clean_with(design: &str, domain: &str) {
    let diagnostics = diagnose_with(design, domain);
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

/// A domain exercising the typed-lifecycle rule's scope: the same
/// status-like field across the stereotype and type matrix — a bare
/// `String` on entity-designed classes is the defect; the integer-typed,
/// enum-typed, value-object (every feature `readonly`, per the value
/// contract), and dto shapes are the legal neighbours.
const BARE_STATUS_DOMAIN: &str = r#"
package nz.example.payroll

enum PayRunStatus {
    Draft as "D" = 0
    Posted as "O" = 1
}

class PayRun {
    String status
    int total
}

class PayrollRun {
    String state
    int total
}

class DoubleRun {
    String status
    String state
}

class LedgerRun {
    int status
}

class TransferRun {
    PayRunStatus status
}

class SnapshotRun {
    readonly String status
}

class TransferRecord {
    String status
}
"#;

#[test]
fn a_bare_string_status_field_on_an_entity_is_rejected() {
    assert_error_with(
        &design("        entity PayRun\n"),
        BARE_STATUS_DOMAIN,
        "field 'status' is a bare String",
    );
    // The `state` spelling is the same defect, and a class carrying both
    // spellings gets one diagnostic per offending field.
    assert_error_with(
        &design("        entity PayrollRun\n"),
        BARE_STATUS_DOMAIN,
        "field 'state' is a bare String",
    );
    let diagnostics = diagnose_with(&design("        entity DoubleRun\n"), BARE_STATUS_DOMAIN);
    for field in ["status", "state"] {
        assert!(
            diagnostics.iter().any(|diagnostic| diagnostic
                .message
                .contains(&format!("field '{field}' is a bare String"))),
            "expected a diagnostic for field '{field}', got: {diagnostics:?}"
        );
    }
}

#[test]
fn value_object_and_dto_string_status_fields_are_exempt() {
    // The check is entity semantics (the machine rule's scope): a value
    // object is immutable and a dto is a transfer record — neither moves
    // through states, so neither carries the machine obligation.
    assert_clean_with(&design("        value SnapshotRun\n"), BARE_STATUS_DOMAIN);
    assert_clean_with(&design("        dto TransferRecord\n"), BARE_STATUS_DOMAIN);
}

#[test]
fn enum_typed_status_without_a_machine_is_legal() {
    // An enum-typed status field with no machine is a legitimate modelling
    // choice (the enum documents the vocabulary; a machine is optional).
    assert_clean_with(&design("        entity TransferRun\n"), BARE_STATUS_DOMAIN);
}

#[test]
fn a_non_string_status_field_is_not_flagged() {
    // The rule is exact: only a bare `String` is the magic-string shape.
    assert_clean_with(&design("        entity LedgerRun\n"), BARE_STATUS_DOMAIN);
}

/// A domain whose enum carries a member the machine never declares
/// (`Pended`): guard literals against the machine field must be machine
/// states, not merely members of the bound enum.
const EXTRA_STATE_DOMAIN: &str = r#"
package nz.example.payroll

enum PayRunStatus {
    Draft as "D" = 0
    Pended as "P" = 1
    Posted as "O" = 2
}

class PayRun {
    PayRunStatus status
    PayRunStatus priorStatus
    int total
}
"#;

const PENDED_MACHINE: &str = r#"
        entity PayRun
        machine status: PayRunStatus {
            initial Draft;
            Draft -> Posted;
            terminal Posted;
        }
"#;

#[test]
fn a_guard_literal_that_is_not_a_machine_state_is_rejected() {
    // `Pended` is a member of the bound enum but not a state of this
    // machine: it can never hold on it, so it must not pass as a guard
    // literal — on a service operation's condition …
    assert_error_with(
        &design(&format!(
            r#"
        service PayRunService {{
            boolean hold() when (status == Pended);
        }}
{PENDED_MACHINE}
"#
        )),
        EXTRA_STATE_DOMAIN,
        "unknown name `Pended`",
    );
    // … the `!=` spelling, and a machine edge guard, are the same contract.
    assert_error_with(
        &design(
            r#"
        entity PayRun
        machine status: PayRunStatus {
            initial Draft;
            Draft -> Posted when (status != Pended);
            terminal Posted;
        }
"#,
        ),
        EXTRA_STATE_DOMAIN,
        "unknown name `Pended`",
    );
}

#[test]
fn a_guard_literal_naming_a_machine_state_types_clean() {
    assert_clean_with(
        &design(&format!(
            r#"
        service PayRunService {{
            boolean post() when (status == Posted);
        }}
{PENDED_MACHINE}
"#
        )),
        EXTRA_STATE_DOMAIN,
    );
}

#[test]
fn guard_literals_from_other_features_are_not_checked() {
    // The machine-state contract scopes to comparisons against the
    // machine's field: another enum-typed feature compares as a plain
    // expression — its name operand resolves as a feature, and the
    // machine never gates it.
    assert_clean_with(
        &design(&format!(
            r#"
        service PayRunService {{
            boolean audit() when (priorStatus == status);
        }}
{PENDED_MACHINE}
"#
        )),
        EXTRA_STATE_DOMAIN,
    );
}
