//! Child-scoped aggregate operations (issue rexlang#179): a module-level
//! `extend <Class> { ... }` block declares root-side operations that act on
//! the aggregate's contained children — `boolean addLine(PayRunLineDraft
//! line) => lines.add;` meaning "reach the `lines` child collection through
//! the root and `add` to it".
//!
//! The block lowers onto the target class's design as
//! `childOperations` (an ordinary `ServiceOperation` list, each carrying a
//! `childScope` naming the child feature and its resolved class), and the
//! driver validates:
//!
//! - the extend target resolves to a class with an entity design in the
//!   same module that is an aggregate root (a repository-bearing entity,
//!   rule 6) — `E0358`;
//! - the child feature exists on the target class and is a containment or
//!   cross-reference — `E0355`;
//! - the action is one of `add`, `remove`, `save` — `E0357`, with
//!   `add`/`remove` requiring a many-valued feature — `E0356`;
//! - child-operation names are unique per extend target, across every
//!   block (blocks merge in source order);
//! - a `when` guard types against the op's declared parameters **and** the
//!   child feature's target class as `self`.
//!
//! The fixture domain (`childops.mox`) is a PayRun aggregate: a root with a
//! contained `PayRunLine[] lines` collection, a contained scalar
//! `PayRunNote note`, and a `PayRunLineDraft` DTO for parameters.

use rex_driver::{
    compile_ddd_str, diagnostic::Severity, DddCompilation, Diagnostic, DomainImports,
};
use rex_ir::ddd::{ChildRef, ServiceOperation};

const CHILD_DOMAIN: &str = include_str!("../../../tests/conformance/ddd/childops.mox");
const DOMAIN_PATH: &str = "tests/conformance/ddd/childops.mox";

/// Compiles a design against the child-ops fixture domain.
fn compile(design: &str) -> DddCompilation {
    compile_ddd_str(
        "tests/conformance/ddd/test.ddd",
        design,
        &[(DOMAIN_PATH.to_string(), CHILD_DOMAIN.to_string())],
        &DomainImports::default(),
    )
}

/// The design-file diagnostics (the fixture domain compiles clean).
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

fn assert_clean(design: &str) {
    let diagnostics = diagnose(design);
    assert!(
        diagnostics
            .iter()
            .all(|diagnostic| diagnostic.severity != Severity::Error),
        "expected no errors, got: {diagnostics:?}"
    );
}

/// The lowered artifact of a design expected to compile clean.
fn model(design: &str) -> rex_ir::ddd::DddModel {
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

/// The `PayRun` design's lowered child operations.
fn child_operations(design: &str) -> Vec<ServiceOperation> {
    let model = model(design);
    model.modules[0]
        .designs
        .iter()
        .find(|design| design.class == "PayRun")
        .expect("the PayRun design")
        .child_operations
        .clone()
}

/// A design whose `payroll` module carries `body` (the root entity design
/// is prepended by the caller).
fn payroll(body: &str) -> String {
    format!(
        "import \"childops.mox\";\n\
         \n\
         application Payroll {{\n\
         \x20   base nz.example.payroll\n\
         \x20   module payroll {{\n\
         {body}\
         \x20   }}\n\
         }}\n"
    )
}

/// The root entity design every child-op test extends: the PayRun aggregate
/// root, with the repository that makes it a root (rule 6).
const ROOT: &str = "\
        entity PayRun repository PayRunRepository {\n\
            findById;\n\
            save;\n\
        }\n";

/// A `lines` collection child op block.
const LINES_BLOCK: &str = "\
        extend PayRun {\n\
            boolean addLine(PayRunLineDraft line) => lines.add;\n\
            boolean removeLine(String lineId) => lines.remove;\n\
            boolean saveLine(PayRunLineDraft line) => lines.save;\n\
        }\n";

// --- lowering ---------------------------------------------------------------

#[test]
fn extend_lowers_child_scope_signatures_and_condition() {
    let design = payroll(&format!(
        "{ROOT}\
         \x20       extend PayRun {{\n\
         \x20           boolean addLine(PayRunLineDraft line) => lines.add;\n\
         \x20           boolean removeLine(String lineId) => lines.remove;\n\
         \x20           boolean saveLine(PayRunLineDraft line) => lines.save when (line.quantity > 0);\n\
         \x20           boolean updateNote(String text) => note.save;\n\
         \x20       }}\n"
    ));
    let operations = child_operations(&design);
    assert_eq!(
        operations
            .iter()
            .map(|operation| operation.name.as_str())
            .collect::<Vec<_>>(),
        ["addLine", "removeLine", "saveLine", "updateNote"],
    );

    // The collection child resolves to its feature and full class name.
    let add = &operations[0];
    assert_eq!(
        add.child_scope,
        Some(ChildRef::new(
            "lines",
            "nz.example.payroll.PayRunLine",
            "add",
        )),
    );
    assert_eq!(
        add.return_type,
        Some(rex_ir::TypeRef::Primitive(rex_ir::PrimitiveType::Boolean)),
    );
    assert_eq!(add.params.len(), 1);
    assert_eq!(add.params[0].name, "line");
    assert_eq!(
        add.params[0].type_,
        rex_ir::TypeRef::Class {
            package: "nz.example.payroll".to_string(),
            name: "PayRunLineDraft".to_string(),
        },
    );
    assert_eq!(add.delegation, None);
    assert!(add.condition.is_none());

    // The guard is carried verbatim (the service-op convention) and lowers
    // no resolved paths (child guards are not the injected-path surface).
    let save = &operations[2];
    assert_eq!(save.condition.as_deref(), Some("line.quantity > 0"));
    assert!(save.condition_paths.is_empty());
    assert_eq!(
        save.child_scope,
        Some(ChildRef::new(
            "lines",
            "nz.example.payroll.PayRunLine",
            "save",
        )),
    );
    assert_eq!(
        operations[1]
            .child_scope
            .as_ref()
            .map(|scope| scope.action.as_str()),
        Some("remove"),
    );

    // `save` is legal on a scalar containment too.
    assert_eq!(
        operations[3].child_scope,
        Some(ChildRef::new(
            "note",
            "nz.example.payroll.PayRunNote",
            "save"
        )),
    );
}

#[test]
fn multiple_extend_blocks_for_one_class_merge_in_source_order() {
    let design = payroll(&format!(
        "{ROOT}\
         \x20       extend PayRun {{\n\
         \x20           boolean addLine(PayRunLineDraft line) => lines.add;\n\
         \x20       }}\n\
         \x20       extend PayRun {{\n\
         \x20           boolean removeLine(String lineId) => lines.remove;\n\
         \x20       }}\n"
    ));
    let operations = child_operations(&design);
    assert_eq!(
        operations
            .iter()
            .map(|operation| operation.name.as_str())
            .collect::<Vec<_>>(),
        ["addLine", "removeLine"],
    );
}

#[test]
fn a_protected_child_operation_lowers_protected() {
    let design = payroll(&format!(
        "{ROOT}\
         \x20       extend PayRun {{\n\
         \x20           protected boolean addLine(PayRunLineDraft line) => lines.add;\n\
         \x20       }}\n"
    ));
    let operations = child_operations(&design);
    assert!(operations[0].is_protected);
}

#[test]
fn a_child_operation_signature_typechecks_against_the_domain() {
    assert_error(
        &payroll(&format!(
            "{ROOT}\
             \x20       extend PayRun {{\n\
             \x20           boolean addLine(Mystery line) => lines.add;\n\
             \x20       }}\n"
        )),
        "unknown type 'Mystery'",
    );
}

// --- `when` scoping ---------------------------------------------------------

#[test]
fn child_when_binds_operation_params_and_the_child_as_self() {
    // `line.quantity` reads the declared parameter; the bare `unitCost`
    // resolves on the child class (the op's `self`), not the root.
    assert_clean(&payroll(&format!(
        "{ROOT}\
         \x20       extend PayRun {{\n\
         \x20           boolean saveLine(PayRunLineDraft line) => lines.save when (line.quantity > 0 && unitCost > 0);\n\
         \x20       }}\n"
    )));
}

#[test]
fn child_when_rejects_a_non_boolean_guard() {
    assert_error(
        &payroll(&format!(
            "{ROOT}\
             \x20       extend PayRun {{\n\
             \x20           boolean saveLine(PayRunLineDraft line) => lines.save when (line.quantity);\n\
             \x20       }}\n"
        )),
        "condition must be boolean",
    );
}

#[test]
fn child_when_scopes_the_root_out_of_self() {
    // `total` is a root feature; the guard's `self` is the child class, so
    // the bare name does not resolve.
    assert_error(
        &payroll(&format!(
            "{ROOT}\
             \x20       extend PayRun {{\n\
             \x20           boolean addLine(PayRunLineDraft line) => lines.add when (total > 0);\n\
             \x20       }}\n"
        )),
        "unknown name `total`",
    );
}

// --- child feature / action validation --------------------------------------

#[test]
fn unknown_child_feature_is_e0355() {
    assert_error(
        &payroll(&format!(
            "{ROOT}\
             \x20       extend PayRun {{\n\
             \x20           boolean addWidget(String id) => widgets.add;\n\
             \x20       }}\n"
        )),
        "E0355 unknown child feature 'widgets' on 'PayRun'",
    );
}

#[test]
fn a_non_child_feature_is_e0355() {
    // `total` is an attribute — not a containment or reference, so it has
    // no child aggregate to extend.
    assert_error(
        &payroll(&format!(
            "{ROOT}\
             \x20       extend PayRun {{\n\
             \x20           boolean bump(String id) => total.add;\n\
             \x20       }}\n"
        )),
        "E0355 child feature 'total' of 'PayRun' is an attribute",
    );
}

#[test]
fn add_and_remove_require_a_collection_e0356() {
    assert_error(
        &payroll(&format!(
            "{ROOT}\
             \x20       extend PayRun {{\n\
             \x20           boolean addNote(String text) => note.add;\n\
             \x20       }}\n"
        )),
        "E0356 child operation 'addNote' in extend 'PayRun': action 'add' requires a collection",
    );
    assert_error(
        &payroll(&format!(
            "{ROOT}\
             \x20       extend PayRun {{\n\
             \x20           boolean removeNote(String id) => note.remove;\n\
             \x20       }}\n"
        )),
        "E0356 child operation 'removeNote' in extend 'PayRun': action 'remove' requires a collection",
    );
}

#[test]
fn save_is_legal_for_a_scalar_child() {
    assert_clean(&payroll(&format!(
        "{ROOT}\
         \x20       extend PayRun {{\n\
         \x20           boolean updateNote(String text) => note.save;\n\
         \x20       }}\n"
    )));
}

#[test]
fn unknown_child_action_is_e0357() {
    assert_error(
        &payroll(&format!(
            "{ROOT}\
             \x20       extend PayRun {{\n\
             \x20           boolean clearLines() => lines.clear;\n\
             \x20       }}\n"
        )),
        "E0357 unknown child action 'clear' in child operation 'clearLines'",
    );
}

#[test]
fn duplicate_child_operation_names_are_refused() {
    assert_error(
        &payroll(&format!(
            "{ROOT}\
             \x20       extend PayRun {{\n\
             \x20           boolean addLine(PayRunLineDraft line) => lines.add;\n\
             \x20       }}\n\
             \x20       extend PayRun {{\n\
             \x20           boolean addLine(PayRunLineDraft line) => lines.save;\n\
             \x20       }}\n"
        )),
        "duplicate child operation 'addLine' in extend 'PayRun'",
    );
}

// --- extend target validation ----------------------------------------------

#[test]
fn unknown_extend_target_is_e0358() {
    assert_error(
        &payroll(&format!(
            "{ROOT}\
             \x20       extend Mystery {{\n\
             \x20           boolean addLine(PayRunLineDraft line) => lines.add;\n\
             \x20       }}\n"
        )),
        "E0358 unknown extend target 'Mystery'",
    );
}

#[test]
fn extend_target_without_a_design_is_e0358() {
    assert_error(
        &payroll(&format!(
            "{ROOT}\
             \x20       extend PayRunLine {{\n\
             \x20           boolean rename(String name) => payRun.save;\n\
             \x20       }}\n"
        )),
        "E0358 extend target 'PayRunLine' has no design in module 'payroll'",
    );
}

#[test]
fn extend_target_that_is_not_an_aggregate_root_is_e0358() {
    // The child is designed as an entity but carries no repository, so it
    // is not an aggregate root.
    assert_error(
        &payroll(&format!(
            "{ROOT}\
             \x20       entity PayRunLine\n\
             \x20       extend PayRunLine {{\n\
             \x20           boolean rename(String name) => payRun.save;\n\
             \x20       }}\n"
        )),
        "E0358 extend target 'PayRunLine' is not an aggregate root",
    );
    // A value design is never a root either.
    assert_error(
        &payroll(&format!(
            "{ROOT}\
             \x20       value PayRunNote\n\
             \x20       extend PayRunNote {{\n\
             \x20           boolean rewrite(String text) => payRun.save;\n\
             \x20       }}\n"
        )),
        "E0358 extend target 'PayRunNote' is not an aggregate root",
    );
}

#[test]
fn extend_target_in_another_module_is_e0358() {
    let design = "import \"childops.mox\";\n\n\
         application Payroll {\n\
         \x20   base nz.example.payroll\n\
         \x20   module payroll {\n\
         \x20       entity PayRun repository PayRunRepository {\n\
         \x20           findById;\n\
         \x20       }\n\
         \x20   }\n\
         \x20   module other {\n\
         \x20       extend PayRun {\n\
         \x20           boolean addLine(PayRunLineDraft line) => lines.add;\n\
         \x20       }\n\
         \x20   }\n\
         }\n";
    assert_error(
        design,
        "E0358 extend target 'PayRun' has no design in module 'other'",
    );
}

// --- the aggregate rules keep their verdicts --------------------------------

#[test]
fn a_standalone_repository_on_a_contained_child_is_still_refused() {
    // Rule 6 is untouched: designing the contained child with its own
    // repository stays an error even though `extend` is in play.
    assert_error(
        &payroll(&format!(
            "{ROOT}\
             \x20       entity PayRunLine repository PayRunLineRepository {{\n\
             \x20           findById;\n\
             \x20       }}\n"
        )),
        "entity 'PayRunLine' is contained by 'PayRun'; only aggregate roots may declare a repository",
    );
}

#[test]
fn a_reference_escaping_its_aggregate_is_still_refused() {
    // Rule 13 is untouched: an outside class referring to a contained child
    // stays an error. The domain adds the escaping class; the design only
    // extends the root.
    let domain = format!(
        "{CHILD_DOMAIN}\n\
         class AuditTrail {{\n\
         \x20   refers PayRunLine line\n\
         }}\n"
    );
    let design = payroll(LINES_BLOCK);
    let compilation = compile_ddd_str(
        "tests/conformance/ddd/test.ddd",
        &design,
        &[(DOMAIN_PATH.to_string(), domain)],
        &DomainImports::default(),
    );
    assert!(
        compilation
            .diagnostics
            .iter()
            .any(|(_, diagnostic)| diagnostic.message.contains(
                "cross reference 'AuditTrail.line' to 'PayRunLine' escapes its aggregate"
            )),
        "expected the rule-13 finding, got: {:?}",
        compilation.diagnostics
    );
}
