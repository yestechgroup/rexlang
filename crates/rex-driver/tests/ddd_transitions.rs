//! Transition-binding tests for delegated `.ddd` service operations
//! (issue rexlang#176): `name => Repository.save transition <Trigger>`
//! declares, instead of infers, the machine-bound transition the
//! delegation fires. The driver validates the binding against the machine
//! bound to the delegation target's repository design: the trigger must be
//! one the machine declares (an explicit `via` name or a bare arrow's
//! derived name, never both), the binding is legal only on a `save`
//! delegation, and a multi-machine module must qualify the machine by its
//! **enum type name** — rexlang machine declarations are field-named
//! (`machine status: PayRunStatus`), so a machine-name qualifier does not
//! exist as authored. A delegation rule 8 already rejects stays silent
//! here (no cascade).
//!
//! The joint shape with issue rexlang#180 closes the suite: a resolved
//! binding pins the machine's class as the op's own-record condition
//! scope, which is what admits a scoped guard in a multi-machine module.

use rex_driver::{compile_ddd_str, diagnostic::Severity, Diagnostic, DomainImports};
use rex_ir::ddd::ConditionScope;

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

/// One machine-bearing design: `PayRun` with the canonical lifecycle
/// (explicit `via` triggers `Calculate`, `Submit`, `Approve` and the bare
/// arrow `Draft -> Reversed`, whose trigger is derived as `reversed`).
const PAYRUN_MACHINE: &str = r#"
        machine status: PayRunStatus {
            initial Draft;
            Draft -> Calculating via Calculate;
            Calculating -> AwaitingApproval via Submit;
            AwaitingApproval -> Posted via Approve;
            Draft -> Reversed via Reverse;
            Calculating -> Reversed;
            terminal Posted;
            terminal Reversed;
        }
"#;

fn payrun_module(service_body: &str) -> String {
    design(&format!(
        r#"
        service PayRunService {{
            inject PayRunRepository;
{service_body}
        }}
        entity PayRun repository PayRunRepository {{
            findById;
            save;
        }}
{PAYRUN_MACHINE}
        entity Timecard repository TimecardRepository {{
            findById;
            save;
        }}
"#
    ))
}

// ---------------------------------------------------------------------------
// Explicit bindings: the trigger exists on the machine bound to the
// delegation target's repository design, and the artifact carries the
// binding as authored.
// ---------------------------------------------------------------------------

#[test]
fn an_explicit_binding_lowers_the_transition() {
    let design = payrun_module("            submit => PayRunRepository.save transition Submit;\n");
    assert_clean(&design);
    let operations = operations_of(&design);
    let transition = operations[0].transition.as_ref().expect("the binding");
    assert_eq!(transition.trigger, "Submit");
    assert_eq!(transition.machine, None, "unqualified: no machine");
}

#[test]
fn a_qualified_binding_lowers_the_machine() {
    let design = payrun_module(
        "            reverse => PayRunRepository.save transition PayRunStatus.Reverse;\n",
    );
    assert_clean(&design);
    let operations = operations_of(&design);
    let transition = operations[0].transition.as_ref().expect("the binding");
    assert_eq!(transition.trigger, "Reverse");
    assert_eq!(
        transition.machine.as_deref(),
        Some("PayRunStatus"),
        "the qualifier is the machine's enum type name"
    );
}

#[test]
fn a_derived_trigger_is_a_legal_target() {
    // `Calculating -> Reversed` is a bare arrow: its trigger is derived as
    // the lowerCamel of the target state, and binding it is legal.
    let design = payrun_module("            renew => PayRunRepository.save transition reversed;\n");
    assert_clean(&design);
    let operations = operations_of(&design);
    assert_eq!(
        operations[0]
            .transition
            .as_ref()
            .expect("the binding")
            .trigger,
        "reversed"
    );
}

// ---------------------------------------------------------------------------
// Unknown triggers: the E0350 error names the trigger and the context, with
// the did-you-mean closest declared trigger when one is a plausible typo,
// and the exhaustive list otherwise.
// ---------------------------------------------------------------------------

#[test]
fn an_unknown_trigger_errors_with_did_you_mean() {
    assert_error(
        &payrun_module("            submit => PayRunRepository.save transition Submt;\n"),
        "E0350 unknown transition trigger 'Submt' on operation 'submit' of service \
         'PayRunService'",
    );
    let diagnostics = diagnose(&payrun_module(
        "            submit => PayRunRepository.save transition Submt;\n",
    ));
    assert!(
        diagnostics.iter().any(|diagnostic| diagnostic
            .help
            .as_deref()
            .is_some_and(|help| help.contains("did you mean 'Submit'?"))),
        "expected the did-you-mean help line, got: {diagnostics:?}"
    );
}

#[test]
fn an_unknown_trigger_lists_the_declared_triggers() {
    assert_error(
        &payrun_module("            submit => PayRunRepository.save transition Vanish;\n"),
        "E0350 unknown transition trigger 'Vanish'",
    );
    let diagnostics = diagnose(&payrun_module(
        "            submit => PayRunRepository.save transition Vanish;\n",
    ));
    assert!(
        diagnostics.iter().any(
            |diagnostic| diagnostic.help.as_deref().is_some_and(|help| help
                .contains("the machine declares: Calculate, Submit, Approve, Reverse, reversed"))
        ),
        "expected the exhaustive trigger list (derived names included), got: {diagnostics:?}"
    );
}

#[test]
fn a_derived_and_explicit_collision_is_ambiguous() {
    // `calculating` is both an explicit `via` trigger and the derived name
    // of the bare arrow — the binding cannot say which edge it fires.
    assert_error(
        &design(
            r#"
        service PayRunService {
            inject PayRunRepository;
            renew => PayRunRepository.save transition calculating;
        }
        entity PayRun repository PayRunRepository {
            findById;
            save;
        }
        machine status: PayRunStatus {
            initial Draft;
            Draft -> Calculating via calculating;
            Calculating -> Posted via Post;
            Posted -> Calculating;
        }
"#,
        ),
        "E0350 transition trigger 'calculating' on operation 'renew' of service \
         'PayRunService' is ambiguous",
    );
}

// ---------------------------------------------------------------------------
// The binding is legal only on a delegation to a repository `save`.
// ---------------------------------------------------------------------------

#[test]
fn a_transition_on_a_declared_op_is_an_error() {
    assert_error(
        &payrun_module("            boolean approve() transition Submit;\n"),
        "E0351 transition on operation 'approve' of service 'PayRunService', which \
         declares its own signature",
    );
}

#[test]
fn a_transition_on_a_non_save_delegation_is_an_error() {
    assert_error(
        &payrun_module("            find => PayRunRepository.findById transition Submit;\n"),
        "E0351 transition on operation 'find' of service 'PayRunService', which \
         delegates to repository 'PayRunRepository.findById'; only a `save` \
         delegation carries a transition",
    );
}

// ---------------------------------------------------------------------------
// Multi-machine modules: the binding must qualify the machine by its enum
// type name.
// ---------------------------------------------------------------------------

/// A module with two machine-bearing designs — both machines bind the
/// field name `status`, so the qualified spelling is the only
/// disambiguation.
fn two_machine_module(service_body: &str) -> String {
    design(&format!(
        r#"
        service PayRunService {{
            inject PayRunRepository;
            inject TimecardRepository;
{service_body}
        }}
        entity PayRun repository PayRunRepository {{
            findById;
            save;
        }}
        machine status: PayRunStatus {{
            initial Draft;
            Draft -> Calculating via Calculate;
            Calculating -> AwaitingApproval via Submit;
            AwaitingApproval -> Posted via Approve;
            terminal Posted;
        }}
        entity Timecard repository TimecardRepository {{
            findById;
            save;
        }}
        machine status: TimecardStatus {{
            initial Open;
            Open -> Submitted via SendTimecard;
            Submitted -> Approved via ApproveTimecard;
            terminal Approved;
        }}
"#
    ))
}

#[test]
fn a_multi_machine_module_requires_the_qualifier() {
    assert_error(
        &two_machine_module("            submit => PayRunRepository.save transition Submit;\n"),
        "E0352 transition on operation 'submit' of service 'PayRunService' has no \
         machine qualifier; module 'payroll' carries 2 machine-bearing designs",
    );
    let diagnostics = diagnose(&two_machine_module(
        "            submit => PayRunRepository.save transition Submit;\n",
    ));
    assert!(
        diagnostics.iter().any(|diagnostic| diagnostic
            .help
            .as_deref()
            .is_some_and(|help| help.contains("qualify as `<EnumType>.<Trigger>`")
                && help.contains("status: PayRunStatus"))),
        "expected the machine-naming help line, got: {diagnostics:?}"
    );
}

#[test]
fn a_multi_machine_module_with_the_qualifier_compiles() {
    assert_clean(&two_machine_module(
        "            submit => PayRunRepository.save transition PayRunStatus.Submit;\n",
    ));
}

#[test]
fn a_qualifier_naming_a_foreign_machine_is_the_unknown_trigger_error() {
    assert_error(
        &two_machine_module(
            "            submit => PayRunRepository.save transition TimecardStatus.Submit;\n",
        ),
        "E0350 unknown transition trigger 'TimecardStatus.Submit' on operation 'submit' \
         of service 'PayRunService'",
    );
    let diagnostics = diagnose(&two_machine_module(
        "            submit => PayRunRepository.save transition TimecardStatus.Submit;\n",
    ));
    assert!(
        diagnostics.iter().any(|diagnostic| diagnostic
            .message
            .contains("'status: PayRunStatus', not 'TimecardStatus'")),
        "expected the machine-naming message, got: {diagnostics:?}"
    );
    assert!(
        diagnostics.iter().any(|diagnostic| diagnostic
            .help
            .as_deref()
            .is_some_and(|help| help.contains("qualify as `PayRunStatus.Submit`"))),
        "expected the qualify-as help line, got: {diagnostics:?}"
    );
}

// ---------------------------------------------------------------------------
// A delegation rule 8 already rejects stays silent here: the transition
// check never cascades onto an unresolved target.
// ---------------------------------------------------------------------------

#[test]
fn an_unresolved_delegation_target_stays_silent() {
    let diagnostics = diagnose(&payrun_module(
        "            submit => GhostRepository.save transition Submit;\n",
    ));
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("unknown delegation target")),
        "expected rule 8's unknown-target error, got: {diagnostics:?}"
    );
    assert!(
        !diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("E035")),
        "the transition check stays silent on an unresolved target: {diagnostics:?}"
    );
}

// ---------------------------------------------------------------------------
// The joint shape with issue rexlang#180: a resolved binding pins the
// machine's class as the op's own-record condition scope, which is what
// admits the scoped guard in a multi-machine module — the old
// "exactly one machine-bearing design" error must not fire.
// ---------------------------------------------------------------------------

#[test]
fn the_joint_transition_and_condition_shape_compiles() {
    let design = two_machine_module(
        "            submit => PayRunRepository.save transition PayRunStatus.Submit\n\
             \x20   when (payRunStatus == Draft && TimecardRepository.status == Approved);\n",
    );
    assert_clean(&design);
    let diagnostics = diagnose(&design);
    assert!(
        !diagnostics.iter().any(|diagnostic| diagnostic
            .message
            .contains("does not have exactly one machine-bearing design")),
        "the relaxed scope must not report the single-machine error: {diagnostics:?}"
    );
    let operations = operations_of(&design);
    let transition = operations[0].transition.as_ref().expect("the binding");
    assert_eq!(transition.trigger, "Submit");
    assert_eq!(transition.machine.as_deref(), Some("PayRunStatus"));
    let paths = &operations[0].condition_paths;
    assert_eq!(
        paths.len(),
        2,
        "the own-record and injected paths: {paths:?}"
    );
    assert_eq!(
        paths[0].scope,
        ConditionScope::OwnRecord,
        "the own-record path is machine-scoped: {paths:?}"
    );
    assert_eq!(paths[0].field, "status");
    assert_eq!(
        paths[1],
        rex_ir::ddd::ConditionPath {
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
        "the injected path resolves across the module"
    );
}

#[test]
fn a_transition_pin_scopes_own_record_names() {
    // With the pin, the op's own-record features resolve on the pinned
    // machine's class: `total` is a PayRun feature.
    let design = two_machine_module(
        "            submit => PayRunRepository.save transition PayRunStatus.Submit\n\
             \x20   when (payRunStatus == Draft && total >= 0);\n",
    );
    assert_clean(&design);
}
