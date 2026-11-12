//! Typed-lifecycle literal checks through real surface compiles (issue
//! #142, the rex-expr L3 rule): the enum-literal namespace
//! (`EnumName.Member`) resolves inside `.ddd` guards, a string literal
//! against an enum-typed field that names no declared member is the V13-
//! style membership ERROR, and a `String`-typed field compared to a string
//! literal in a `when`/filter guard is the use-based WARNING suggesting the
//! enum declaration.

use rex_driver::{
    compile_ddd_str, compile_evt_str, diagnostic::Severity, Diagnostic, DomainImports,
};

/// The canonical fixture's domain: `PayRun.status: PayRunStatus`,
/// `PayRun.total: int`.
const PAYRUN_DOMAIN: &str = include_str!("../../../tests/conformance/ddd/payrun.mox");

/// Compiles a design against a caller-provided domain source under the
/// same path key (the design's `import "payrun.mox"` resolves lexically to
/// it) and returns the design-file diagnostics (domains are expected to
/// compile clean).
fn diagnose_design(design: &str, domain: &str) -> Vec<Diagnostic> {
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

fn design(body: &str) -> String {
    format!(
        r#"import "payrun.mox";

application Payroll {{
    base nz.example.payroll
    module payroll {{
{body}
    }}
}}
"#
    )
}

fn assert_error(design_source: &str, expected: &str) {
    let diagnostics = diagnose_design(design_source, PAYRUN_DOMAIN);
    assert!(
        diagnostics
            .iter()
            .any(|d| d.severity == Severity::Error && d.message.contains(expected)),
        "expected an error containing {expected:?}, got: {diagnostics:?}"
    );
}

fn assert_warning(design_source: &str, domain: &str, expected: &str) {
    let diagnostics = diagnose_design(design_source, domain);
    assert!(
        diagnostics
            .iter()
            .any(|d| d.severity == Severity::Warning && d.message.contains(expected)),
        "expected a warning containing {expected:?}, got: {diagnostics:?}"
    );
}

/// A guard may name an enum member as a typed value through the dotted
/// namespace — the spelling the machine-state binding generalizes.
#[test]
fn a_guard_may_name_enum_members_as_typed_values() {
    assert!(diagnose_design(
        &design(
            r#"
        entity PayRun repository PayRunRepository {
            findById;
        }
        machine status: PayRunStatus {
            initial Draft;
            Draft -> Posted via Post when (status == PayRunStatus.Draft);
            terminal Posted;
        }
"#,
        ),
        PAYRUN_DOMAIN,
    )
    .is_empty());
    // Both operand orders, and `!=`.
    assert!(diagnose_design(
        &design(
            r#"
        entity PayRun repository PayRunRepository {
            findById;
        }
        machine status: PayRunStatus {
            initial Draft;
            Draft -> Posted via Post when (PayRunStatus.Posted != status);
            terminal Posted;
        }
"#,
        ),
        PAYRUN_DOMAIN,
    )
    .is_empty());
}

/// A string literal compared against the machine's enum-typed field where
/// the literal names no declared member is the closed-set membership error,
/// not the generic same-type mismatch.
#[test]
fn a_non_member_literal_against_an_enum_field_is_a_membership_error() {
    assert_error(
        &design(
            r#"
        entity PayRun repository PayRunRepository {
            findById;
        }
        machine status: PayRunStatus {
            initial Draft;
            Draft -> Posted via Post when (status == "Wibble");
            terminal Posted;
        }
"#,
        ),
        "`\"Wibble\"` is not a literal of enum `PayRunStatus`",
    );
}

/// A declared member is still rejected by the same-type rule (R2): the
/// typed spelling is the dotted form, never a bare string (the
/// anti-magic-strings stance of docs/DDD.md).
#[test]
fn a_member_literal_against_an_enum_field_stays_the_same_type_error() {
    assert_error(
        &design(
            r#"
        entity PayRun repository PayRunRepository {
            findById;
        }
        machine status: PayRunStatus {
            initial Draft;
            Draft -> Posted via Post when (status == "Draft");
            terminal Posted;
        }
"#,
        ),
        "cannot compare",
    );
}

/// A domain with a plain `String` lifecycle-agnostic field compared in a
/// machine guard: the use-based warning fires where the comparison happens,
/// naming the field and suggesting the enum declaration. Not name-based —
/// the field is `tier`, not `status`/`state`.
const TIER_DOMAIN: &str = r#"
package nz.example.payroll

enum PayRunStatus {
    Draft as "D" = 0
    Posted as "O" = 1
}

class PayRun {
    PayRunStatus status
    String tier
    int total
}
"#;

#[test]
fn a_string_field_guard_comparison_is_a_use_based_warning() {
    let source = design(
        r#"
        entity PayRun repository PayRunRepository {
            findById;
        }
        machine status: PayRunStatus {
            initial Draft;
            Draft -> Posted via Post when (tier == "gold");
            terminal Posted;
        }
"#,
    );
    assert_warning(&source, TIER_DOMAIN, "`tier`");
    // And nothing else: the guard still type-checks (a warning, not an
    // error).
    let diagnostics = diagnose_design(&source, TIER_DOMAIN);
    assert!(
        diagnostics.iter().all(|d| d.severity != Severity::Error),
        "expected no errors, got: {diagnostics:?}"
    );
}

/// The empty string can never be a declared enum member, so an emptiness
/// check like `tier != ""` is not a lifecycle comparison: the sentinel
/// comparison stays silent, while a non-empty literal (`tier == "Draft"`)
/// over the same field still warns (rexlang#142, the lint's sentinel skip).
#[test]
fn an_empty_string_sentinel_comparison_stays_silent() {
    let sentinel = design(
        r#"
        entity PayRun repository PayRunRepository {
            findById;
        }
        machine status: PayRunStatus {
            initial Draft;
            Draft -> Posted via Post when (tier != "");
            terminal Posted;
        }
"#,
    );
    assert!(diagnose_design(&sentinel, TIER_DOMAIN).is_empty());
    // The `==` direction of the sentinel is silent too.
    let sentinel_eq = design(
        r#"
        entity PayRun repository PayRunRepository {
            findById;
        }
        machine status: PayRunStatus {
            initial Draft;
            Draft -> Posted via Post when (tier == "");
            terminal Posted;
        }
"#,
    );
    assert!(diagnose_design(&sentinel_eq, TIER_DOMAIN).is_empty());
    // And a non-empty literal over the same field still warns.
    assert_warning(
        &design(
            r#"
        entity PayRun repository PayRunRepository {
            findById;
        }
        machine status: PayRunStatus {
            initial Draft;
            Draft -> Posted via Post when (tier == "Draft");
            terminal Posted;
        }
"#,
        ),
        TIER_DOMAIN,
        "`tier`",
    );
}

/// The canonical `.evt` fixture's billing filter (`when (orderId != "")`)
/// compiles clean — the empty literal is a sentinel, not a lifecycle
/// comparison, so the lint stays silent (rexlang#142).
#[test]
fn the_canonical_evt_filter_sentinel_compiles_clean() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../");
    let contract_path = root.join("tests/conformance/events/orders.evt");
    let domain_path = root.join("tests/conformance/events/orders.mox");
    let contract = std::fs::read_to_string(&contract_path).expect("read orders.evt");
    let domain = std::fs::read_to_string(&domain_path).expect("read orders.mox");
    let compilation = compile_evt_str(
        contract_path.to_str().expect("utf-8 path"),
        &contract,
        &[(
            domain_path.to_str().expect("utf-8 path").to_string(),
            domain,
        )],
        &DomainImports::default(),
    );
    assert!(
        compilation.diagnostics.is_empty(),
        "the sentinel filter compiles clean: {:?}",
        compilation.diagnostics
    );
}
