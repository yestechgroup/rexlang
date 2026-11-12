//! Condition-scoping tests for `.ddd` service-operation `when` guards
//! (issue rexlang#180): an op-level guard is no longer single-entity. Own
//! -record fields stay unqualified (resolved on the bound entity — the
//! module's unique machine-bearing class, or the class a transition pin
//! names); qualified paths (`timecardRepository.status`) resolve against
//! the module's injected repositories, and the resolution lowers into the
//! artifact as `conditionPaths` so consumers never re-parse the condition
//! string. Multi-machine modules admit op-level guards when every
//! referenced name resolves: ambiguous state names and ambiguous
//! state-typed field names are the `E0354` error, with the enum-type
//! spellings (`<Enum>.<State>`, lowerCamel(`<Enum>`) for a field) as the
//! qualified forms.
//!
//! The fixture domain (`timesheet.mox`) carries two aggregates — `PayRun`
//! (machine `status: PayRunStatus`) and `Timecard` (machine
//! `status: TimecardStatus`) — so one module exercises the single-machine
//! and multi-machine shapes by which designs it declares.

use rex_driver::{compile_ddd_str, diagnostic::Severity, Diagnostic, DomainImports};
use rex_ir::ddd::{ConditionPath, ConditionScope};

/// The shared fixture domain: `PayRun.status: PayRunStatus`, `int total`;
/// `Timecard.status: TimecardStatus`, `int hours`.
const TIMESHEET_DOMAIN: &str = include_str!("../../../tests/conformance/ddd/timesheet.mox");

/// Compiles a design against the fixture domain and returns the
/// design-file diagnostics (the domain is expected to compile clean).
fn diagnose(design: &str) -> Vec<Diagnostic> {
    let compilation = compile_ddd_str(
        "tests/conformance/ddd/test.ddd",
        design,
        &[(
            "tests/conformance/ddd/timesheet.mox".to_string(),
            TIMESHEET_DOMAIN.to_string(),
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
            .any(|diagnostic| diagnostic.severity == Severity::Error
                && diagnostic.message.contains(expected)),
        "expected an error containing {expected:?}, got: {diagnostics:?}"
    );
}

fn assert_clean(design: &str) {
    let diagnostics = diagnose(design);
    assert!(
        diagnostics
            .iter()
            .all(|diagnostic| diagnostic.severity != Severity::Error),
        "expected no errors, got: {diagnostics:?}"
    );
}

/// The lowered service operations of a compiling design (first service).
fn operations_of(design: &str) -> Vec<rex_ir::ddd::ServiceOperation> {
    let compilation = compile_ddd_str(
        "tests/conformance/ddd/test.ddd",
        design,
        &[(
            "tests/conformance/ddd/timesheet.mox".to_string(),
            TIMESHEET_DOMAIN.to_string(),
        )],
        &DomainImports::default(),
    );
    let model = compilation.model.expect("expected an artifact");
    model.modules[0].services[0].operations.clone()
}

const HEAD: &str = r#"
import "timesheet.mox";

application Timesheet {
    base nz.example.timesheet
    module payroll {
"#;

const TAIL: &str = r#"
    }
}
"#;

fn design(body: &str) -> String {
    format!("{HEAD}{body}{TAIL}")
}

/// A module whose only machine-bearing design is `PayRun`.
const SINGLE_MACHINE: &str = r#"
        service PayRunService {
            inject TimecardRepository;
            boolean approve() when (status == AwaitingApproval && TimecardRepository.status == TimecardStatus.Approved);
        }
        entity PayRun repository PayRunRepository {
            findById;
            save;
        }
        machine status: PayRunStatus {
            initial Draft;
            Draft -> AwaitingApproval via Submit;
            AwaitingApproval -> Posted via Approve;
            terminal Posted;
        }
        entity Timecard repository TimecardRepository {
            findById;
            save;
        }
"#;

// ---------------------------------------------------------------------------
// Own-record guards are unchanged: a single-machine module keeps the
// machine-bound shape, and a guard with no injected path lowers no
// `conditionPaths` (the existing artifacts stay byte-identical).
// ---------------------------------------------------------------------------

#[test]
fn own_record_guard_compiles_without_condition_paths() {
    let design = design(
        r#"
        service PayRunService {
            boolean approve() when (status == AwaitingApproval);
        }
        entity PayRun repository PayRunRepository {
            findById;
            save;
        }
        machine status: PayRunStatus {
            initial Draft;
            Draft -> AwaitingApproval via Submit;
            AwaitingApproval -> Posted via Approve;
            terminal Posted;
        }
"#,
    );
    assert_clean(&design);
    let operations = operations_of(&design);
    assert!(
        operations[0].condition_paths.is_empty(),
        "an own-record guard carries no resolved paths: {:?}",
        operations[0].condition_paths
    );
}

// ---------------------------------------------------------------------------
// A qualified path resolves against the module's injected repositories and
// lowers as an `injected` ConditionPath.
// ---------------------------------------------------------------------------

#[test]
fn a_qualified_injection_guard_lowers_an_injected_path() {
    let design = design(SINGLE_MACHINE);
    assert_clean(&design);
    let operations = operations_of(&design);
    let paths = &operations[0].condition_paths;
    assert_eq!(paths.len(), 1, "one injected path: {paths:?}");
    assert_eq!(
        paths[0],
        ConditionPath {
            scope: ConditionScope::Injected {
                repository: "TimecardRepository".to_string(),
                class: "Timecard".to_string(),
            },
            field: "status".to_string(),
            field_type: rex_ir::TypeRef::Enum {
                package: "nz.example.timesheet".to_string(),
                name: "TimecardStatus".to_string(),
            },
        },
        "the injected path resolves the repository's designed class and the field's type"
    );
}

// ---------------------------------------------------------------------------
// Unresolvable paths are the E0353 error, with the did-you-mean closest
// name when one is a plausible typo.
// ---------------------------------------------------------------------------

#[test]
fn an_unknown_injection_is_a_condition_path_error() {
    assert_error(
        &design(
            r#"
        service PayRunService {
            inject TimecardRepository;
            boolean approve() when (timecardRepository.status == TimecardStatus.Approved);
        }
        entity PayRun repository PayRunRepository {
            findById;
        }
        machine status: PayRunStatus {
            initial Draft;
            Draft -> Posted via Approve;
            terminal Posted;
        }
        entity Timecard repository TimecardRepository {
            findById;
        }
"#,
        ),
        "E0353 unknown condition path 'timecardRepository'",
    );
    // The did-you-mean suggestion rides on the help line.
    let diagnostics = diagnose(&design(
        r#"
        service PayRunService {
            inject TimecardRepository;
            boolean approve() when (timecardRepository.status == TimecardStatus.Approved);
        }
        entity PayRun repository PayRunRepository {
            findById;
        }
        machine status: PayRunStatus {
            initial Draft;
            Draft -> Posted via Approve;
            terminal Posted;
        }
        entity Timecard repository TimecardRepository {
            findById;
        }
"#,
    ));
    assert!(
        diagnostics.iter().any(|diagnostic| diagnostic
            .help
            .as_deref()
            .is_some_and(|help| help.contains("did you mean 'TimecardRepository'?"))),
        "expected a did-you-mean help line, got: {diagnostics:?}"
    );
}

#[test]
fn an_unknown_field_on_an_injected_class_is_a_condition_path_error() {
    assert_error(
        &design(
            r#"
        service PayRunService {
            inject TimecardRepository;
            boolean approve() when (TimecardRepository.stauts == TimecardStatus.Approved);
        }
        entity PayRun repository PayRunRepository {
            findById;
        }
        machine status: PayRunStatus {
            initial Draft;
            Draft -> Posted via Approve;
            terminal Posted;
        }
        entity Timecard repository TimecardRepository {
            findById;
        }
"#,
        ),
        "E0353 unknown field 'stauts' on 'Timecard'",
    );
}

// ---------------------------------------------------------------------------
// A path that resolves but mistypes is not E0353: the shared expression
// language's same-type equality rule fires, spanned on the condition.
// ---------------------------------------------------------------------------

#[test]
fn a_type_mismatch_is_the_same_type_equality_error() {
    assert_error(
        &design(
            r#"
        service PayRunService {
            inject TimecardRepository;
            boolean approve() when (TimecardRepository.hours == Draft);
        }
        entity PayRun repository PayRunRepository {
            findById;
        }
        machine status: PayRunStatus {
            initial Draft;
            Draft -> Posted via Approve;
            terminal Posted;
        }
        entity Timecard repository TimecardRepository {
            findById;
        }
"#,
        ),
        "invalid condition",
    );
    // ... and it is not the condition-path error (the path resolves).
    let diagnostics = diagnose(&design(
        r#"
        service PayRunService {
            inject TimecardRepository;
            boolean approve() when (TimecardRepository.hours == Draft);
        }
        entity PayRun repository PayRunRepository {
            findById;
        }
        machine status: PayRunStatus {
            initial Draft;
            Draft -> Posted via Approve;
            terminal Posted;
        }
        entity Timecard repository TimecardRepository {
            findById;
        }
"#,
    ));
    assert!(
        !diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("E0353")),
        "a resolved path never reports E0353: {diagnostics:?}"
    );
}

// ---------------------------------------------------------------------------
// Multi-machine modules: the "exactly one machine-bearing design"
// restriction is relaxed — a guard is admitted when every referenced name
// resolves. Ambiguous state names and ambiguous state-typed field names
// are E0354 at the reference site; the enum-type spellings qualify.
// ---------------------------------------------------------------------------

#[test]
fn a_multi_machine_module_admits_a_scoped_guard() {
    // Both machines bind the field name `status`, so the own-record side
    // uses the enum-derived spelling (`payRunStatus`) and the state names
    // stay unqualified where they are unique (`Draft` is PayRun's alone).
    let design = design(
        r#"
        service PayRunService {
            inject PayRunRepository;
            inject TimecardRepository;
            boolean submit() when (payRunStatus == Draft && TimecardRepository.status == TimecardStatus.Approved);
        }
        entity PayRun repository PayRunRepository {
            findById;
            save;
        }
        machine status: PayRunStatus {
            initial Draft;
            Draft -> Posted via Submit;
            terminal Posted;
        }
        entity Timecard repository TimecardRepository {
            findById;
            save;
        }
        machine status: TimecardStatus {
            initial Open;
            Open -> Submitted via Send;
            Submitted -> Approved via MarkApproved;
            terminal Approved;
        }
"#,
    );
    assert_clean(&design);
    let operations = operations_of(&design);
    let paths = &operations[0].condition_paths;
    assert_eq!(
        paths.len(),
        2,
        "the own-record path and the injected path: {paths:?}"
    );
    assert_eq!(
        paths[0],
        ConditionPath {
            scope: ConditionScope::OwnRecord,
            field: "status".to_string(),
            field_type: rex_ir::TypeRef::Enum {
                package: "nz.example.timesheet".to_string(),
                name: "PayRunStatus".to_string(),
            },
        },
        "the qualified own-record path names the machine's field and enum"
    );
    assert_eq!(
        paths[1],
        ConditionPath {
            scope: ConditionScope::Injected {
                repository: "TimecardRepository".to_string(),
                class: "Timecard".to_string(),
            },
            field: "status".to_string(),
            field_type: rex_ir::TypeRef::Enum {
                package: "nz.example.timesheet".to_string(),
                name: "TimecardStatus".to_string(),
            },
        },
        "the injected path resolves across the multi-machine module"
    );
}

#[test]
fn an_ambiguous_state_name_is_the_machine_scope_error() {
    // Both machines declare `Approved`; the unqualified spelling is the
    // E0354 error with the enum-type qualification as help.
    assert_error(
        &design(
            r#"
        service PayRunService {
            inject PayRunRepository;
            inject TimecardRepository;
            boolean submit() when (payRunStatus == Approved);
        }
        entity PayRun repository PayRunRepository {
            findById;
            save;
        }
        machine status: PayRunStatus {
            initial Draft;
            Draft -> Approved via Submit;
            terminal Approved;
        }
        entity Timecard repository TimecardRepository {
            findById;
            save;
        }
        machine status: TimecardStatus {
            initial Open;
            Open -> Approved via MarkApproved;
            terminal Approved;
        }
"#,
        ),
        "E0354 ambiguous machine scope",
    );
    let diagnostics = diagnose(&design(
        r#"
        service PayRunService {
            inject PayRunRepository;
            inject TimecardRepository;
            boolean submit() when (payRunStatus == Approved);
        }
        entity PayRun repository PayRunRepository {
            findById;
            save;
        }
        machine status: PayRunStatus {
            initial Draft;
            Draft -> Approved via Submit;
            terminal Approved;
        }
        entity Timecard repository TimecardRepository {
            findById;
            save;
        }
        machine status: TimecardStatus {
            initial Open;
            Open -> Approved via MarkApproved;
            terminal Approved;
        }
"#,
    ));
    assert!(
        diagnostics.iter().any(|diagnostic| diagnostic
            .help
            .as_deref()
            .is_some_and(|help| help.contains("qualify as `<EnumType>.<State>`"))),
        "expected the qualify-as help line, got: {diagnostics:?}"
    );
}

#[test]
fn the_qualified_state_spelling_resolves_the_ambiguity() {
    // `TimecardStatus.Approved` (the enum-literal namespace, rule L3 of
    // docs/EXPRESSIONS.md) pins the machine.
    assert_clean(&design(
        r#"
        service PayRunService {
            inject TimecardRepository;
            boolean submit() when (TimecardRepository.status == TimecardStatus.Approved);
        }
        entity PayRun repository PayRunRepository {
            findById;
            save;
        }
        machine status: PayRunStatus {
            initial Draft;
            Draft -> Posted via Submit;
            terminal Posted;
        }
        entity Timecard repository TimecardRepository {
            findById;
            save;
        }
        machine status: TimecardStatus {
            initial Open;
            Open -> Approved via MarkApproved;
            terminal Approved;
        }
"#,
    ));
}

#[test]
fn an_ambiguous_machine_field_is_the_machine_scope_error() {
    // Both machines bind the field name `status`; the bare spelling is
    // ambiguous (never guessed), the enum-derived spelling resolves.
    assert_error(
        &design(
            r#"
        service PayRunService {
            inject PayRunRepository;
            boolean submit() when (status == Draft);
        }
        entity PayRun repository PayRunRepository {
            findById;
            save;
        }
        machine status: PayRunStatus {
            initial Draft;
            Draft -> Posted via Submit;
            terminal Posted;
        }
        entity Timecard repository TimecardRepository {
            findById;
            save;
        }
        machine status: TimecardStatus {
            initial Open;
            Open -> Approved via MarkApproved;
            terminal Approved;
        }
"#,
        ),
        "E0354 ambiguous machine scope",
    );
    let diagnostics = diagnose(&design(
        r#"
        service PayRunService {
            inject PayRunRepository;
            boolean submit() when (status == Draft);
        }
        entity PayRun repository PayRunRepository {
            findById;
            save;
        }
        machine status: PayRunStatus {
            initial Draft;
            Draft -> Posted via Submit;
            terminal Posted;
        }
        entity Timecard repository TimecardRepository {
            findById;
            save;
        }
        machine status: TimecardStatus {
            initial Open;
            Open -> Approved via MarkApproved;
            terminal Approved;
        }
"#,
    ));
    assert!(
        diagnostics.iter().any(|diagnostic| diagnostic
            .help
            .as_deref()
            .is_some_and(|help| help.contains("`payRunStatus`"))),
        "expected the enum-derived field spelling in the help line, got: {diagnostics:?}"
    );
}

#[test]
fn a_unique_machine_field_stays_bare_in_a_multi_machine_module() {
    // `Timecard` binds its machine as `state` — unique across the module's
    // machines — so the bare spelling resolves without qualification. The
    // domain names the feature `state` accordingly.
    const DOMAIN: &str = r#"
package nz.example.timesheet

enum PayRunStatus {
    Draft as "D" = 0
    Posted as "P" = 1
}

enum TimecardStatus {
    Open as "O" = 0
    Approved as "R" = 1
}

class PayRun {
    PayRunStatus status
}

class Timecard {
    TimecardStatus state
}
"#;
    let source = r#"import "timesheet.mox";

application Timesheet {
    base nz.example.timesheet
    module payroll {
        service PayRunService {
            inject TimecardRepository;
            boolean submit() when (state == Open);
        }
        entity PayRun repository PayRunRepository {
            findById;
            save;
        }
        machine status: PayRunStatus {
            initial Draft;
            Draft -> Posted via Submit;
            terminal Posted;
        }
        entity Timecard repository TimecardRepository {
            findById;
            save;
        }
        machine state: TimecardStatus {
            initial Open;
            Open -> Approved via MarkApproved;
            terminal Approved;
        }
    }
}
"#;
    let compilation = compile_ddd_str(
        "tests/conformance/ddd/test.ddd",
        source,
        &[(
            "tests/conformance/ddd/timesheet.mox".to_string(),
            DOMAIN.to_string(),
        )],
        &DomainImports::default(),
    );
    let diagnostics: Vec<Diagnostic> = compilation
        .diagnostics
        .into_iter()
        .filter(|(path, _)| path.ends_with(".ddd"))
        .map(|(_, diagnostic)| diagnostic)
        .collect();
    assert!(
        diagnostics
            .iter()
            .all(|diagnostic| diagnostic.severity != Severity::Error),
        "expected no errors, got: {diagnostics:?}"
    );
}

// ---------------------------------------------------------------------------
// A module with no machine-bearing design still refuses an op-level guard
// whose own-record references have no scope (the machine-bound rule's
// remnant; the transition pin of issue rexlang#176 lifts it per-op).
// ---------------------------------------------------------------------------

#[test]
fn a_guard_without_any_machine_scope_still_errors() {
    assert_error(
        &design(
            r#"
        service PayRunService {
            boolean approve() when (status == Draft);
        }
        entity PayRun repository PayRunRepository {
            findById;
        }
"#,
        ),
        "does not have exactly one machine-bearing design",
    );
}
