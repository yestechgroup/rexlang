//! Executable service/repository bodies (issue #178): a `.ddd` operation may
//! carry a closed procedural statement body — `let`, `let … else error …`,
//! assignment, `guard … else error …`, `if`/`else`, and `return` — typed
//! against the operation's parameters, the designed aggregate, and the
//! synthetic repository classes the driver registers.
//!
//! The body lowers to the operation's `bodies["stmt"]` target (the canonical
//! JSON of [`rex_expr::stmt::StmtBody`]); a service body also records the
//! `=> <Repository>` receiver as `bodyReceiver` (the primary repository
//! bound as `self`). Every type/flow failure is a hard compile error
//! (E0360); an unresolved `=>` receiver is E0359.
//!
//! The fixture domain is a small PayRun aggregate: a root with a contained
//! `Line[]` collection, a `Status` lifecycle, an `Audit` aggregate with a
//! declared `record` operation, and a `DraftLine` parameter type.

use rex_driver::{
    compile_ddd_str, diagnostic::Severity, DddCompilation, Diagnostic, DomainImports,
};
use rex_expr::stmt::{Stmt, StmtBody};
use rex_ir::ddd::{DddModel, RepositoryOperation, ServiceOperation};

const DOMAIN: &str = r#"
package test.payroll

enum Status {
    Draft as "D" = 0
    Approved as "A" = 1
}

class Run {
    String id
    Status status
    int total
    contains Line[] lines opposite run
}

class Line {
    container Run run opposite lines
    int quantity
    int unitCost
}

class DraftLine {
    int quantity
    int unitCost
}

class Audit {
    String note
}

class AuditRecord {
    boolean accepted
}
"#;

const DOMAIN_PATH: &str = "tests/conformance/ddd/bodies.mox";

/// Indents every line of `text` by `spaces` (the tests author unindented
/// operation snippets and the helper places them in the module body).
fn indent(text: &str, spaces: usize) -> String {
    let pad = " ".repeat(spaces);
    text.lines()
        .map(|line| format!("{pad}{line}"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// The default repository operations: the four admitted builtins (plus the
/// two deliberately-unregistered ones), and a declared operation carrying a
/// statement body and a `when` guard.
const RUN_REPOSITORY: &str = r#"findById;
findAll;
save;
delete;
findByExample;
findByKeys;
int totalQuantity(String runId) when (status == Draft) {
    let run = self.findById(runId) else error NotFound("Run", runId);
    return run.lines.map(line => line.quantity).sum();
}"#;

const AUDIT_REPOSITORY: &str = r#"findById;
save;
AuditRecord record(String note);"#;

/// The default service operations: a void body exercising every statement
/// form and reduction, a value-returning body with `if`/`else`, an injected
/// bare-name repository call, and a returning single-expression body.
const SERVICE_OPERATIONS: &str = r#"submit(DraftLine draft) capability SubmitRun => RunRepository {
    let run = self.findById("current") else error NotFound("Run", "current");
    guard run.total >= 0 else error Conflict("total");
    let units = run.lines.map(line => line.quantity).sum();
    let anyPositive = run.lines.any(line => line.quantity > 0);
    let allCosted = run.lines.all(line => line.unitCost >= 0);
    let lineCount = run.lines.size();
    run.total = units;
    let saved = self.save(run);
    let audit = AuditRepository.record("submit");
}
boolean approve() capability ApproveRun when (status == Draft) => RunRepository {
    let run = self.findById("current") else error NotFound("Run", "current");
    if (run.total > 0) {
        return true;
    } else {
        return false;
    }
}
boolean note(String text) => AuditRepository {
    let record = self.record(text);
    return record.accepted;
}
int totalFor(String runId) => RunRepository {
    let run = self.findById(runId) else error NotFound("Run", runId);
    return run.lines.map(line => line.quantity * line.unitCost).sum();
}"#;

/// A design with the given service operations and `RunRepository` body, or
/// the defaults.
fn design_with(service_ops: &str, run_repository: &str) -> String {
    format!(
        "import \"bodies.mox\";\n\
         \n\
         application Test {{\n\
         \x20   base test.payroll\n\
         \x20   module payroll {{\n\
         \x20       service RunService {{\n\
         \x20           inject RunRepository;\n\
         \x20           inject AuditRepository;\n\
         {}\n\
         \x20       }}\n\
         \x20       entity Run repository RunRepository {{\n\
         {}\n\
         \x20       }} machine status: Status {{\n\
         \x20           initial Draft;\n\
         \x20           Draft -> Approved via Approve;\n\
         \x20           terminal Approved;\n\
         \x20       }}\n\
         \x20       entity Audit repository AuditRepository {{\n\
         {}\n\
         \x20       }}\n\
         \x20   }}\n\
         }}\n",
        indent(service_ops, 12),
        indent(run_repository, 12),
        indent(AUDIT_REPOSITORY, 12),
    )
}

fn design() -> String {
    design_with(SERVICE_OPERATIONS, RUN_REPOSITORY)
}

/// A design carrying `op` as the only service operation, with the default
/// repositories.
fn design_with_op(op: &str) -> String {
    design_with(op, RUN_REPOSITORY)
}

fn compile(design: &str) -> DddCompilation {
    compile_ddd_str(
        "tests/conformance/ddd/test.ddd",
        design,
        &[(DOMAIN_PATH.to_string(), DOMAIN.to_string())],
        &DomainImports::default(),
    )
}

fn diagnose(design: &str) -> Vec<Diagnostic> {
    compile(design)
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

fn model(design: &str) -> DddModel {
    let compilation = compile(design);
    assert!(
        compilation
            .diagnostics
            .iter()
            .all(|(_, diagnostic)| diagnostic.severity != Severity::Error),
        "expected a clean compile, got: {:?}",
        compilation.diagnostics
    );
    compilation.model.expect("the design lowered")
}

fn service_op<'a>(model: &'a DddModel, name: &str) -> &'a ServiceOperation {
    model.modules[0].services[0]
        .operations
        .iter()
        .find(|operation| operation.name == name)
        .unwrap_or_else(|| panic!("expected service operation `{name}`"))
}

fn repository_op<'a>(model: &'a DddModel, class: &str, name: &str) -> &'a RepositoryOperation {
    model.modules[0]
        .designs
        .iter()
        .find(|design| design.class == class)
        .and_then(|design| design.repository.as_ref())
        .and_then(|repository| {
            repository
                .operations
                .iter()
                .find(|operation| operation.name == name)
        })
        .unwrap_or_else(|| panic!("expected repository operation `{name}` of `{class}`"))
}

fn body(operation: &ServiceOperation) -> StmtBody {
    serde_json::from_str(
        operation
            .bodies
            .get("stmt")
            .expect("the operation carries a `stmt` body target"),
    )
    .expect("the body target is canonical StmtBody JSON")
}

// --- lowering ---------------------------------------------------------------

/// The void body lowers every statement form in order, the service records
/// its `=>` receiver, and the op has no delegation.
#[test]
fn void_service_body_lowers_every_statement_form() {
    let model = model(&design());
    let submit = service_op(&model, "submit");
    assert!(
        submit.delegation.is_none(),
        "a body op declares no delegation"
    );
    assert_eq!(submit.body_receiver.as_deref(), Some("RunRepository"));
    let body = body(submit);
    assert_eq!(body.stmts.len(), 9, "the fixture's nine statements lower");

    assert!(matches!(
        &body.stmts[0],
        Stmt::LetElse { name, .. } if name == "run"
    ));
    assert!(matches!(
        &body.stmts[1],
        Stmt::Guard { expr, .. } if expr == "run.total >= 0"
    ));
    assert!(matches!(
        &body.stmts[2],
        Stmt::Let { name, expr } if name == "units"
            && expr == "run.lines.map(line => line.quantity).sum()"
    ));
    assert!(matches!(
        &body.stmts[3],
        Stmt::Let { name, expr } if name == "anyPositive" && expr.contains(".any(")
    ));
    assert!(matches!(
        &body.stmts[4],
        Stmt::Let { name, expr } if name == "allCosted" && expr.contains(".all(")
    ));
    assert!(matches!(
        &body.stmts[5],
        Stmt::Let { name, expr } if name == "lineCount" && expr == "run.lines.size()"
    ));
    assert!(matches!(
        &body.stmts[6],
        Stmt::Assign { path, expr } if path == &["run".to_string(), "total".to_string()]
            && expr == "units"
    ));
    assert!(matches!(
        &body.stmts[7],
        Stmt::Let { name, expr } if name == "saved" && expr == "self.save(run)"
    ));
    assert!(matches!(
        &body.stmts[8],
        Stmt::Let { name, expr } if name == "audit" && expr == "AuditRepository.record(\"submit\")"
    ));
}

/// A value-returning body lowers its `if`/`else` branches and returns; the
/// body alongside `capability` and `when` clauses keeps both.
#[test]
fn returning_body_lowers_if_else_and_keeps_clauses() {
    let model = model(&design());
    let approve = service_op(&model, "approve");
    assert_eq!(approve.capabilities, ["ApproveRun"]);
    assert_eq!(approve.condition.as_deref(), Some("status == Draft"));
    assert_eq!(approve.body_receiver.as_deref(), Some("RunRepository"));
    let approve_body = body(approve);
    assert_eq!(approve_body.stmts.len(), 2);
    assert!(matches!(
        &approve_body.stmts[0],
        Stmt::LetElse { name, .. } if name == "run"
    ));
    let Stmt::If { cond, then, else_ } = &approve_body.stmts[1] else {
        panic!("expected an if statement, got {:?}", approve_body.stmts[1]);
    };
    assert_eq!(cond, "(run.total > 0)");
    assert!(matches!(&then[0], Stmt::Return { expr: Some(text) } if text == "true"));
    assert!(matches!(&else_[0], Stmt::Return { expr: Some(text) } if text == "false"));

    let total_for = service_op(&model, "totalFor");
    assert_eq!(total_for.body_receiver.as_deref(), Some("RunRepository"));
    assert!(matches!(
        &body(total_for).stmts[1],
        Stmt::Return { expr: Some(text) }
            if text == "run.lines.map(line => line.quantity * line.unitCost).sum()"
    ));
}

/// An injected non-primary repository binds by its bare name (`AuditRepository`)
/// and `self` stays the primary receiver.
#[test]
fn injected_repository_receiver_is_a_bare_name() {
    let model = model(&design());
    let note = service_op(&model, "note");
    assert_eq!(note.body_receiver.as_deref(), Some("AuditRepository"));
    let body = body(note);
    assert!(matches!(
        &body.stmts[0],
        Stmt::Let { expr, .. } if expr == "self.record(text)"
    ));
    assert!(matches!(
        &body.stmts[1],
        Stmt::Return { expr: Some(text) } if text == "record.accepted"
    ));
}

/// A repository operation body lowers onto the repository op with no
/// `bodyReceiver` (`self` is the repository's own design).
#[test]
fn repository_body_lowers_without_a_body_receiver() {
    let model = model(&design());
    let total_quantity = repository_op(&model, "Run", "totalQuantity");
    let body: StmtBody = serde_json::from_str(
        total_quantity
            .bodies
            .get("stmt")
            .expect("the repository operation carries a `stmt` target"),
    )
    .expect("canonical StmtBody JSON");
    assert_eq!(body.stmts.len(), 2);
    assert!(matches!(
        &body.stmts[0],
        Stmt::LetElse { name, .. } if name == "run"
    ));
    assert!(matches!(
        &body.stmts[1],
        Stmt::Return { expr: Some(text) } if text == "run.lines.map(line => line.quantity).sum()"
    ));
    assert_eq!(total_quantity.condition.as_deref(), Some("status == Draft"));
}

// --- diagnostics ------------------------------------------------------------

/// An unresolvable `=>` receiver is E0359.
#[test]
fn unknown_body_receiver_emits_e0359() {
    let op = r#"boolean submit() => NoSuchRepository {
    return true;
}"#;
    assert_error(&design_with_op(op), "E0359");
    assert_error(
        &design_with_op(op),
        "unknown body receiver 'NoSuchRepository'",
    );
}

/// A receiver that is designed but not injected by the service is E0359.
#[test]
fn uninjected_body_receiver_emits_e0359() {
    let design = design_with_op(
        r#"boolean submit() => AuditRepository {
    return true;
}"#,
    );
    // Drop the AuditRepository injection: the receiver is no longer part of
    // the service's dependency set.
    let design = design.replace("            inject AuditRepository;\n", "");
    assert_error(&design, "E0359");
    assert_error(&design, "is not injected");
}

/// Unknown operations on a receiver are E0360 — including the builtins the
/// closed subset deliberately does not register.
#[test]
fn unknown_operation_emits_e0360() {
    for call in ["self.findByExample(run)", "self.findByKeys(run)"] {
        let op = format!(
            "boolean submit() => RunRepository {{\n    \
             let run = self.findById(\"x\") else error NotFound(\"Run\", \"x\");\n    \
             let found = {call};\n    \
             return true;\n}}"
        );
        assert_error(&design_with_op(&op), "E0360");
        assert_error(&design_with_op(&op), "unknown operation");
    }
}

/// An unknown feature on a navigated class is E0360.
#[test]
fn unknown_feature_emits_e0360() {
    let op = r#"boolean submit() => RunRepository {
    let run = self.findById("x") else error NotFound("Run", "x");
    let nope = run.nope;
    return true;
}"#;
    assert_error(&design_with_op(op), "E0360");
    assert_error(&design_with_op(op), "unknown feature `nope`");
}

/// A type mismatch in an assignment is E0360.
#[test]
fn type_mismatch_emits_e0360() {
    let op = r#"boolean submit() => RunRepository {
    let run = self.findById("x") else error NotFound("Run", "x");
    run.total = "oops";
    return true;
}"#;
    assert_error(&design_with_op(op), "E0360");
    assert_error(&design_with_op(op), "assign types string");
}

/// A non-boolean guard is E0360.
#[test]
fn non_boolean_guard_emits_e0360() {
    let op = r#"boolean submit() => RunRepository {
    guard 1 else error Conflict("nope");
    return true;
}"#;
    assert_error(&design_with_op(op), "E0360");
    assert_error(&design_with_op(op), "guard condition must be boolean");
}

/// The closed raise vocabulary and its arity are enforced (E0360).
#[test]
fn unknown_error_variant_and_arity_emit_e0360() {
    let unknown = r#"boolean submit() => RunRepository {
    let run = self.findById("x") else error Maybe("x");
    return true;
}"#;
    assert_error(&design_with_op(unknown), "E0360");
    assert_error(&design_with_op(unknown), "unknown error variant `Maybe`");

    let wrong_arity = r#"boolean submit() => RunRepository {
    guard true else error NotFound("only-one");
    return true;
}"#;
    assert_error(&design_with_op(wrong_arity), "E0360");
    assert_error(&design_with_op(wrong_arity), "expects 2 arguments");
}

/// A value-returning body that can fall off its end is E0360.
#[test]
fn missing_return_emits_e0360() {
    let op = r#"int submit() => RunRepository {
    let run = self.findById("x") else error NotFound("Run", "x");
}"#;
    assert_error(&design_with_op(op), "E0360");
    assert_error(&design_with_op(op), "missing return");
}

/// A body on a builtin repository operation is a parse refusal naming the
/// builtin.
#[test]
fn builtin_with_body_is_a_parse_refusal() {
    let design = design_with(SERVICE_OPERATIONS, "findById { let run = 1; }");
    let diagnostics = diagnose(&design);
    assert!(
        diagnostics.iter().any(|diagnostic| diagnostic
            .message
            .contains("built-in repository operation 'findById' takes no body")),
        "expected the builtin-body refusal, got: {diagnostics:?}"
    );
}

/// The closed statement subset names the construct it refused.
#[test]
fn unsupported_statement_is_named() {
    let op = r#"boolean submit() => RunRepository {
    while (true) {
        return true;
    }
}"#;
    let diagnostics = diagnose(&design_with_op(op));
    assert!(
        diagnostics.iter().any(|diagnostic| diagnostic
            .message
            .contains("unsupported statement 'while' in a service body")),
        "expected the named refusal, got: {diagnostics:?}"
    );
}

/// A `transition` binding on a body operation is refused with the existing
/// E0351 semantics (the body form counts as declaring its own signature).
#[test]
fn transition_on_a_body_op_is_e0351() {
    let op = r#"boolean submit() transition Approve => RunRepository {
    return true;
}"#;
    assert_error(&design_with_op(op), "E0351");
}
