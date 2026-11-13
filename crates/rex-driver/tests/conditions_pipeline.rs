//! Characterization pins for the `when`-condition pipeline shared by six
//! surfaces (issue rexlang#161): each surface's parse-error diagnostics,
//! non-boolean typing errors, ok-boolean paths, and guard string lints are
//! pinned by exact message, exact byte span, severity, and help text, so
//! the shared skeleton cannot drift one surface's wording, spans, or
//! emission order.
//!
//! The surfaces and their entry points:
//!
//! | Surface | Entry point | Condition site |
//! |---|---|---|
//! | `.ddd` designs | [`rex_driver::compile_ddd_str`] | service/repo/transition/child-op guards |
//! | `.cmmn` criteria | [`rex_driver::compile_cmmn_str`] | plan-item `when` if-parts |
//! | `.uc` step guards | [`rex_driver::compile_usecase_str`] | step `when` guards |
//! | `.deploy` policies | [`rex_driver::compile_deploy_str`] | `require`/`prohibit` typing half |
//! | `.evt` subscription filters | [`rex_driver::compile_evt_str`] | subscription `when` filters |
//! | capability `when` | [`rex_driver::compile_str`] | the deferred pending conditions |
//!
//! Span conventions differ by surface and are pinned as-is: the whole
//! `when (...)` paren span for `.cmmn`/`.uc`/`.evt`, the deployment
//! declaration span for `.deploy`, error spans rebased into the paren
//! interior (capped at the closing paren) for `.ddd`, and error spans
//! rebased onto the trimmed interior for the pending capability
//! conditions.

use rex_driver::diagnostic::Severity;
use rex_driver::{
    compile_cmmn_str, compile_ddd_str, compile_deploy_str, compile_evt_str, compile_str,
    compile_usecase_str, Compilation, DeployCompilation, Diagnostic, DomainImports, Span,
};

/// Byte span of the `nth` (0-based) occurrence of `needle` in `source`.
fn span_of(source: &str, needle: &str, nth: usize) -> Span {
    let (start, _) = source
        .match_indices(needle)
        .nth(nth)
        .unwrap_or_else(|| panic!("occurrence {nth} of '{needle}' not found"));
    (start..start + needle.len()).into()
}

/// The span of the raw `(...)` parens following the `nth` (0-based)
/// `when ` keyword: from the open paren through the close paren,
/// inclusive — the span the parsers attach to a `when` clause.
fn when_parens(source: &str, nth: usize) -> Span {
    let open = source.match_indices("when (").nth(nth).expect("when (").0 + "when ".len();
    let close = source[open..].find(')').expect("close paren") + open;
    (open..close + 1).into()
}

// --- site: `.ddd` designs ----------------------------------------------------
//
// The fixture domain: `Order.state: Status`, `Order.label: String`,
// `Order.total: int`.

const DDD_DOMAIN: &str = r#"
package demo

enum Status {
    Draft as "D" = 0
    Open as "O" = 1
}

class Order {
    String label
    Status state
    int total
}
"#;

/// Compiles a design against [`DDD_DOMAIN`] and returns the design-file
/// diagnostics (the domain is expected to compile clean).
fn diagnose_design(design: &str) -> Vec<Diagnostic> {
    let compilation = compile_ddd_str(
        "tests/conformance/ddd/test.ddd",
        design,
        &[(
            "tests/conformance/ddd/domain.mox".to_string(),
            DDD_DOMAIN.to_string(),
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

#[test]
fn ddd_parse_error_is_labeled_and_rebased_into_the_interior() {
    let design = r#"
import "domain.mox";

application Shop {
    base demo
    module m {
        entity Order
        machine state: Status {
            initial Draft;
            Draft -> Open;
            terminal Open;
        }
        service Checkout {
            String f() when (state ==);
        }
    }
}
"#;
    let diagnostics = diagnose_design(design);
    assert_eq!(
        diagnostics.len(),
        2,
        "the parse error plus the recovered expression's typing: {diagnostics:?}"
    );
    // First: the parse error, rebased into the interior.
    let diagnostic = &diagnostics[0];
    assert_eq!(diagnostic.severity, Severity::Error);
    assert_eq!(
        diagnostic.message,
        "invalid condition in operation 'f' of service 'Checkout': unexpected trailing input",
    );
    assert_eq!(
        diagnostic.help, None,
        "ddd condition diagnostics carry no help"
    );
    // The parser's offsets are relative to the sliced text; they are
    // rebased from the open paren and capped at the close paren.
    let parens = when_parens(design, 0);
    let text_start = parens.start + 1;
    assert_eq!(
        diagnostic.span,
        Some((text_start + 6..text_start + 8).into())
    );
    // Second: the recovered `state` expression types, but is not boolean.
    let diagnostic = &diagnostics[1];
    assert_eq!(diagnostic.severity, Severity::Error);
    assert_eq!(
        diagnostic.message,
        "invalid condition in operation 'f' of service 'Checkout': \
         condition must be boolean, found enum `Status`",
    );
    assert_eq!(diagnostic.help, None);
    assert_eq!(diagnostic.span, Some((parens.start + 1..parens.end).into()));
}

#[test]
fn ddd_non_boolean_condition_names_the_context_and_spans_the_interior() {
    let design = r#"
import "domain.mox";

application Shop {
    base demo
    module m {
        entity Order
        machine state: Status {
            initial Draft;
            Draft -> Open;
            terminal Open;
        }
        service Checkout {
            String f() when (total);
        }
    }
}
"#;
    let diagnostics = diagnose_design(design);
    assert_eq!(
        diagnostics.len(),
        1,
        "exactly one non-boolean diagnostic: {diagnostics:?}"
    );
    let diagnostic = &diagnostics[0];
    assert_eq!(diagnostic.severity, Severity::Error);
    assert_eq!(
        diagnostic.message,
        "invalid condition in operation 'f' of service 'Checkout': \
         condition must be boolean, found int",
    );
    assert_eq!(diagnostic.help, None);
    // The non-boolean diagnostic spans the interior exactly: from just
    // after the open paren through the close paren (the ddd cap).
    let parens = when_parens(design, 0);
    assert_eq!(diagnostic.span, Some((parens.start + 1..parens.end).into()));
}

#[test]
fn ddd_guard_string_lint_is_labeled_and_spanned_on_the_literal() {
    let design = r#"
import "domain.mox";

application Shop {
    base demo
    module m {
        entity Order
        machine state: Status {
            initial Draft;
            Draft -> Open;
            terminal Open;
        }
        service Checkout {
            String f() when (label == "draft");
        }
    }
}
"#;
    let diagnostics = diagnose_design(design);
    assert_eq!(diagnostics.len(), 1, "one lint: {diagnostics:?}");
    let diagnostic = &diagnostics[0];
    assert_eq!(diagnostic.severity, Severity::Warning);
    assert_eq!(
        diagnostic.message,
        "operation 'f' of service 'Checkout': guard compares string literal \"draft\" \
         against string-typed `label` — declare the field as an enum so guard \
         literals are checked against its members (docs/EXPRESSIONS.md rule L3)",
    );
    assert_eq!(diagnostic.help, None);
    // The lint's span is the string literal's, rebased into the interior.
    assert_eq!(diagnostic.span, Some(span_of(design, "\"draft\"", 0)));
}

#[test]
fn ddd_transition_guard_non_boolean_names_the_edge() {
    let design = r#"
import "domain.mox";

application Shop {
    base demo
    module m {
        entity Order
        machine state: Status {
            initial Draft;
            Draft -> Open when (total);
            terminal Open;
        }
    }
}
"#;
    let diagnostics = diagnose_design(design);
    assert_eq!(
        diagnostics.len(),
        1,
        "exactly one non-boolean diagnostic: {diagnostics:?}"
    );
    let diagnostic = &diagnostics[0];
    assert_eq!(diagnostic.severity, Severity::Error);
    assert_eq!(
        diagnostic.message,
        "invalid condition in transition 'Draft -> Open' of machine 'state': \
         condition must be boolean, found int",
    );
    assert_eq!(diagnostic.help, None);
    let parens = when_parens(design, 0);
    assert_eq!(diagnostic.span, Some((parens.start + 1..parens.end).into()));
}

#[test]
fn ddd_ok_boolean_condition_is_clean_and_lowers_verbatim() {
    let design = r#"
import "domain.mox";

application Shop {
    base demo
    module m {
        entity Order
        machine state: Status {
            initial Draft;
            Draft -> Open;
            terminal Open;
        }
        service Checkout {
            String f() when (state == Draft);
        }
    }
}
"#;
    let compilation = compile_ddd_str(
        "tests/conformance/ddd/test.ddd",
        design,
        &[(
            "tests/conformance/ddd/domain.mox".to_string(),
            DDD_DOMAIN.to_string(),
        )],
        &DomainImports::default(),
    );
    assert!(
        compilation.diagnostics.is_empty(),
        "expected a clean compile: {:?}",
        compilation.diagnostics
    );
    let model = compilation.model.expect("artifact");
    assert_eq!(
        model.modules[0].services[0].operations[0]
            .condition
            .as_deref(),
        Some("state == Draft"),
        "the condition lowers as the verbatim trimmed interior"
    );
}

// --- site: `.cmmn` criteria ---------------------------------------------------

const CMMN_FIXTURE_DIR: &str = "tests/conformance/cmmn";

/// Compiles a case source against the lending conformance imports (the
/// shape of `cmmn_golden`'s harness, reduced to the domain bucket). The
/// case is compiled under the fixture directory so its `import
/// "lending.mox"` resolves the way the golden harness resolves it.
fn compile_case(source: &str) -> rex_driver::CmmnCompilation {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(CMMN_FIXTURE_DIR);
    let domain_path = dir.join("lending.mox");
    let domain_source = std::fs::read_to_string(&domain_path).expect("read lending.mox");
    compile_cmmn_str(
        dir.join("case.cmmn").display().to_string().as_str(),
        source,
        &[(domain_path.display().to_string(), domain_source)],
        &DomainImports::default(),
        &[],
        &[],
        &[],
        &[],
    )
}

#[test]
fn cmmn_parse_error_names_the_criterion_and_spans_the_parens() {
    let source = r#"import "lending.mox"
case C {
    case_file {
        Borrower borrower;
    }
    milestone M {
        entry on borrower.update when (borrower.name ==);
    }
}
"#;
    let compilation = compile_case(source);
    let diagnostics: Vec<&Diagnostic> = compilation
        .diagnostics
        .iter()
        .filter(|(path, _)| path.ends_with(".cmmn"))
        .map(|(_, diagnostic)| diagnostic)
        .collect();
    assert_eq!(diagnostics.len(), 2, "one parse error: {diagnostics:?}");
    let diagnostic = diagnostics[0];
    assert_eq!(diagnostic.severity, Severity::Error);
    assert_eq!(
        diagnostic.message,
        "invalid criterion condition: unexpected trailing input",
    );
    assert_eq!(
        diagnostic.help.as_deref(),
        Some("in the `when` of plan item 'M' in case 'C'"),
    );
    // The whole paren span, verbatim.
    assert_eq!(diagnostic.span, Some(when_parens(source, 0)));
    // The recovered `borrower.name` expression types too (R3: the case-file
    // item receiver is optional), after the parse error.
    let diagnostic = diagnostics[1];
    assert_eq!(diagnostic.severity, Severity::Error);
    assert_eq!(
        diagnostic.message,
        "R3: receiver is optional (Option<class `Borrower`>); use `?.` to navigate it (see docs/EXPRESSIONS.md rule R3)",
    );
    assert_eq!(
        diagnostic.help.as_deref(),
        Some("in the `when` of plan item 'M' in case 'C'"),
    );
    assert_eq!(diagnostic.span, Some(when_parens(source, 0)));
}

#[test]
fn cmmn_non_boolean_condition_names_the_found_type() {
    let source = r#"import "lending.mox"
case C {
    case_file {
        Borrower[1..1] borrower;
    }
    milestone M {
        entry on borrower.update when (borrower.name);
    }
}
"#;
    let compilation = compile_case(source);
    let diagnostics: Vec<&Diagnostic> = compilation
        .diagnostics
        .iter()
        .filter(|(path, _)| path.ends_with(".cmmn"))
        .map(|(_, diagnostic)| diagnostic)
        .collect();
    assert_eq!(diagnostics.len(), 1, "one type error: {diagnostics:?}");
    let diagnostic = diagnostics[0];
    assert_eq!(diagnostic.severity, Severity::Error);
    assert_eq!(
        diagnostic.message,
        "criterion condition must be boolean, found string",
    );
    assert_eq!(
        diagnostic.help.as_deref(),
        Some("in the `when` of plan item 'M' in case 'C'"),
    );
    assert_eq!(diagnostic.span, Some(when_parens(source, 0)));
}

#[test]
fn cmmn_guard_string_lint_warns_on_the_paren_span() {
    let source = r#"import "lending.mox"
case C {
    case_file {
        Borrower[1..1] borrower;
    }
    milestone M {
        entry on borrower.update when (borrower.name == "draft");
    }
}
"#;
    let compilation = compile_case(source);
    let warnings: Vec<&Diagnostic> = compilation
        .diagnostics
        .iter()
        .filter(|(path, _)| path.ends_with(".cmmn"))
        .map(|(_, diagnostic)| diagnostic)
        .collect();
    assert_eq!(warnings.len(), 1, "one lint: {warnings:?}");
    let diagnostic = warnings[0];
    assert_eq!(diagnostic.severity, Severity::Warning);
    assert_eq!(
        diagnostic.message,
        "guard compares string literal \"draft\" against string-typed `borrower.name` \
         — declare the field as an enum so guard literals are checked against its \
         members (docs/EXPRESSIONS.md rule L3)",
    );
    assert_eq!(
        diagnostic.help.as_deref(),
        Some("in the `when` of plan item 'M' in case 'C'"),
    );
    assert_eq!(diagnostic.span, Some(when_parens(source, 0)));
}

// --- site: `.uc` step guards --------------------------------------------------

const UC_DOMAIN: &str = concat!(
    "package nz.example.delivery\n",
    "\n",
    "class Drone {\n",
    "    boolean armed\n",
    "    String label\n",
    "    op boolean signal()\n",
    "}\n",
);

fn compile_uc(source: &str) -> rex_driver::UseCaseCompilation {
    compile_usecase_str(
        "bad.uc",
        source,
        &[("domain.mox".to_string(), UC_DOMAIN.to_string())],
        &DomainImports::default(),
        &[],
        &[],
        &[],
    )
}

fn uc_diagnostics(compilation: &rex_driver::UseCaseCompilation) -> Vec<&Diagnostic> {
    compilation
        .diagnostics
        .iter()
        .filter(|(path, _)| path.ends_with(".uc"))
        .map(|(_, diagnostic)| diagnostic)
        .collect()
}

#[test]
fn uc_parse_error_names_the_step_and_spans_the_parens() {
    let source = concat!(
        "import \"domain.mox\"\n",
        "\n",
        "usecase U {\n",
        "    subject D: Drone {\n",
        "        perform x: signal when (armed &&);\n",
        "    }\n",
        "}\n",
    );
    let compilation = compile_uc(source);
    assert!(
        compilation.model.is_none(),
        "a parse error must drop the artifact"
    );
    let diagnostics = uc_diagnostics(&compilation);
    assert_eq!(diagnostics.len(), 1, "one parse error: {diagnostics:?}");
    let diagnostic = diagnostics[0];
    assert_eq!(diagnostic.severity, Severity::Error);
    assert_eq!(
        diagnostic.message,
        "invalid step guard condition: unexpected trailing input",
    );
    assert_eq!(
        diagnostic.help.as_deref(),
        Some("in the `when` of step 'x' in use case 'U'"),
    );
    assert_eq!(diagnostic.span, Some(when_parens(source, 0)));
}

#[test]
fn uc_non_boolean_guard_names_the_found_type() {
    let source = concat!(
        "import \"domain.mox\"\n",
        "\n",
        "usecase U {\n",
        "    subject D: Drone {\n",
        "        perform x: signal when (label);\n",
        "    }\n",
        "}\n",
    );
    let compilation = compile_uc(source);
    assert!(compilation.model.is_none());
    let diagnostics = uc_diagnostics(&compilation);
    assert_eq!(diagnostics.len(), 1, "one type error: {diagnostics:?}");
    let diagnostic = diagnostics[0];
    assert_eq!(diagnostic.severity, Severity::Error);
    assert_eq!(
        diagnostic.message,
        "step guard condition must be boolean, found string",
    );
    assert_eq!(
        diagnostic.help.as_deref(),
        Some("in the `when` of step 'x' in use case 'U'"),
    );
    assert_eq!(diagnostic.span, Some(when_parens(source, 0)));
}

#[test]
fn uc_guard_string_lint_warns_on_the_paren_span() {
    let source = concat!(
        "import \"domain.mox\"\n",
        "\n",
        "usecase U {\n",
        "    subject D: Drone {\n",
        "        perform x: signal when (label == \"draft\");\n",
        "    }\n",
        "}\n",
    );
    let compilation = compile_uc(source);
    assert!(
        compilation.model.is_some(),
        "a warning must not drop the artifact: {:?}",
        compilation.diagnostics
    );
    let diagnostics = uc_diagnostics(&compilation);
    assert_eq!(diagnostics.len(), 1, "one lint: {diagnostics:?}");
    let diagnostic = diagnostics[0];
    assert_eq!(diagnostic.severity, Severity::Warning);
    assert_eq!(
        diagnostic.message,
        "guard compares string literal \"draft\" against string-typed `label` \
         — declare the field as an enum so guard literals are checked against its \
         members (docs/EXPRESSIONS.md rule L3)",
    );
    assert_eq!(
        diagnostic.help.as_deref(),
        Some("in the `when` of step 'x' in use case 'U'"),
    );
    assert_eq!(diagnostic.span, Some(when_parens(source, 0)));
}

// --- site: `.deploy` policies -------------------------------------------------

fn compile_deploy(source: &str) -> DeployCompilation {
    compile_deploy_str("bad.deploy", source, &[], &[])
}

const DEPLOY_HEAD: &str = concat!(
    "application Svc {\n",
    "    component api: api {}\n",
    "}\n",
    "\n",
    "profile prod {\n",
    "    target: standalone\n",
    "    defaults {\n",
    "        api.replicas: 2\n",
    "    }\n",
);

const DEPLOY_TAIL: &str = concat!(
    "}\n",
    "\n",
    "deployment dev for Svc {\n",
    "    use prod\n",
    "}\n",
);

/// The deployment declaration's span: from the `deployment` keyword
/// through the closing brace — the span every policy diagnostic carries.
fn deployment_span(source: &str) -> Span {
    let start = source.find("deployment dev").expect("deployment decl");
    let end = source[start..].rfind('}').expect("closing brace") + start + 1;
    (start..end).into()
}

#[test]
fn deploy_parse_error_names_the_keyword_and_spans_the_deployment() {
    let source = format!("{DEPLOY_HEAD}    require (api.replicas >=)\n{DEPLOY_TAIL}");
    let compilation = compile_deploy(&source);
    assert!(compilation.model.is_none());
    let diagnostics: Vec<&Diagnostic> = compilation
        .diagnostics
        .iter()
        .filter(|(path, _)| path.ends_with(".deploy"))
        .map(|(_, diagnostic)| diagnostic)
        .collect();
    assert_eq!(
        diagnostics.len(),
        2,
        "the parse error plus the recovered expression's typing: {diagnostics:?}"
    );
    let diagnostic = diagnostics[0];
    assert_eq!(diagnostic.severity, Severity::Error);
    assert_eq!(
        diagnostic.message,
        "invalid policy condition: unexpected trailing input",
    );
    assert_eq!(
        diagnostic.help.as_deref(),
        Some("in `require (api.replicas >=)`"),
    );
    // The policy's span is the deployment declaration's span.
    assert_eq!(diagnostic.span, Some(deployment_span(&source)));
    let diagnostic = diagnostics[1];
    assert_eq!(diagnostic.severity, Severity::Error);
    assert_eq!(
        diagnostic.message,
        "policy condition must be boolean, found int"
    );
    assert_eq!(
        diagnostic.help.as_deref(),
        Some("in `require (api.replicas >=)`"),
    );
    assert_eq!(diagnostic.span, Some(deployment_span(&source)));
}

#[test]
fn deploy_non_boolean_condition_names_the_found_type() {
    let source = format!("{DEPLOY_HEAD}    require (api.replicas + 1)\n{DEPLOY_TAIL}");
    let compilation = compile_deploy(&source);
    assert!(compilation.model.is_none());
    let diagnostics: Vec<&Diagnostic> = compilation
        .diagnostics
        .iter()
        .filter(|(path, _)| path.ends_with(".deploy"))
        .map(|(_, diagnostic)| diagnostic)
        .collect();
    assert_eq!(diagnostics.len(), 1, "one type error: {diagnostics:?}");
    let diagnostic = diagnostics[0];
    assert_eq!(diagnostic.severity, Severity::Error);
    assert_eq!(
        diagnostic.message,
        "policy condition must be boolean, found int"
    );
    assert_eq!(
        diagnostic.help.as_deref(),
        Some("in `require (api.replicas + 1)`"),
    );
    assert_eq!(diagnostic.span, Some(deployment_span(&source)));
}

#[test]
fn deploy_prohibit_help_names_the_prohibit_keyword() {
    let source = format!("{DEPLOY_HEAD}    prohibit (api.replicas)\n{DEPLOY_TAIL}");
    let compilation = compile_deploy(&source);
    assert!(compilation.model.is_none());
    let diagnostics: Vec<&Diagnostic> = compilation
        .diagnostics
        .iter()
        .filter(|(path, _)| path.ends_with(".deploy"))
        .map(|(_, diagnostic)| diagnostic)
        .collect();
    assert_eq!(diagnostics.len(), 1, "one type error: {diagnostics:?}");
    let diagnostic = diagnostics[0];
    assert_eq!(diagnostic.severity, Severity::Error);
    assert_eq!(
        diagnostic.message,
        "policy condition must be boolean, found int"
    );
    assert_eq!(
        diagnostic.help.as_deref(),
        Some("in `prohibit (api.replicas)`"),
    );
}

#[test]
fn deploy_runs_no_guard_string_lints() {
    // `api.health` is a Text setting: the ok-boolean path types cleanly and
    // the policy evaluates to a satisfied `require` — with no guard lint,
    // unlike every other surface (the deploy contract has no enum-vocabulary
    // advice to give).
    let source = concat!(
        "application Svc {\n",
        "    component api: api {}\n",
        "}\n",
        "\n",
        "profile prod {\n",
        "    target: standalone\n",
        "    defaults {\n",
        "        api.health: \"green\"\n",
        "    }\n",
        "    require (api.health == \"green\")\n",
        "}\n",
        "\n",
        "deployment dev for Svc {\n",
        "    use prod\n",
        "}\n",
    );
    let compilation = compile_deploy(source);
    assert!(
        compilation.diagnostics.is_empty(),
        "a satisfied text-equality policy compiles clean, no lint: {:?}",
        compilation.diagnostics
    );
    assert!(compilation.model.is_some());
}

// --- site: `.evt` subscription filters ----------------------------------------

fn compile_events(source: &str) -> rex_driver::EventCompilation {
    compile_evt_str("bad.evt", source, &[], &DomainImports::default())
}

fn evt_diagnostic(compilation: &rex_driver::EventCompilation) -> &Diagnostic {
    let matches: Vec<&Diagnostic> = compilation
        .diagnostics
        .iter()
        .filter(|(path, _)| path.ends_with(".evt"))
        .map(|(_, diagnostic)| diagnostic)
        .collect();
    assert_eq!(
        matches.len(),
        1,
        "exactly one diagnostic: {:?}",
        compilation.diagnostics
    );
    matches[0]
}

#[test]
fn evt_parse_error_names_the_filter_and_spans_the_parens() {
    let source = concat!(
        "event E { orderId: String; }\n",
        "subscription s { events [ E ] consumer c when (orderId !) }",
    );
    let compilation = compile_events(source);
    assert!(
        compilation.model.is_none(),
        "a parse error must drop the artifact"
    );
    let matches: Vec<&Diagnostic> = compilation
        .diagnostics
        .iter()
        .filter(|(path, _)| path.ends_with(".evt"))
        .map(|(_, diagnostic)| diagnostic)
        .collect();
    assert_eq!(
        matches.len(),
        2,
        "the parse error plus the recovered expression's typing: {:?}",
        compilation.diagnostics
    );
    // First: the parse error, help naming the subscription only.
    let diagnostic = matches[0];
    assert_eq!(diagnostic.severity, Severity::Error);
    assert_eq!(
        diagnostic.message,
        "invalid when filter: unexpected trailing input"
    );
    assert_eq!(
        diagnostic.help.as_deref(),
        Some("in the `when` of subscription 's'"),
    );
    assert_eq!(diagnostic.span, Some(when_parens(source, 0)));
    // Second: the recovered `orderId` expression types against every
    // subscribed event; the typed help names the event.
    let diagnostic = matches[1];
    assert_eq!(diagnostic.severity, Severity::Error);
    assert_eq!(
        diagnostic.message,
        "when filter must be boolean, found string"
    );
    assert_eq!(
        diagnostic.help.as_deref(),
        Some("in the `when` of subscription 's' against event 'E'"),
    );
    assert_eq!(diagnostic.span, Some(when_parens(source, 0)));
}

#[test]
fn evt_non_boolean_filter_names_the_found_type_per_event() {
    let source = concat!(
        "event E { orderId: String; }\n",
        "subscription s { events [ E ] consumer c when (orderId) }",
    );
    let compilation = compile_events(source);
    assert!(compilation.model.is_none());
    let diagnostic = evt_diagnostic(&compilation);
    assert_eq!(diagnostic.severity, Severity::Error);
    assert_eq!(
        diagnostic.message,
        "when filter must be boolean, found string"
    );
    // The typed help names the event the filter failed against; the parse
    // help names only the subscription.
    assert_eq!(
        diagnostic.help.as_deref(),
        Some("in the `when` of subscription 's' against event 'E'"),
    );
    assert_eq!(diagnostic.span, Some(when_parens(source, 0)));
}

#[test]
fn evt_guard_string_lint_dedupes_across_events() {
    // The same comparison against every listed event is ONE warning, with
    // the first listed event's help; the filter still lowers (warnings do
    // not drop the artifact).
    let source = concat!(
        "event OrderPlaced { status: String; }\n",
        "event OrderCancelled { status: String; }\n",
        "subscription s { events [ OrderPlaced OrderCancelled ] consumer c when (status == \"draft\") }",
    );
    let compilation = compile_events(source);
    assert_eq!(
        compilation
            .model
            .as_ref()
            .expect("a warning must not drop the artifact")
            .subscriptions[0]
            .when
            .as_deref(),
        Some("status == \"draft\""),
        "the verbatim interior lowers"
    );
    let diagnostic = evt_diagnostic(&compilation);
    assert_eq!(diagnostic.severity, Severity::Warning);
    assert_eq!(
        diagnostic.message,
        "guard compares string literal \"draft\" against string-typed `status` \
         — declare the field as an enum so guard literals are checked against its \
         members (docs/EXPRESSIONS.md rule L3) (in the `when` of subscription 's')",
    );
    assert_eq!(
        diagnostic.help.as_deref(),
        Some("in the `when` of subscription 's' against event 'OrderPlaced'"),
    );
    assert_eq!(diagnostic.span, Some(when_parens(source, 0)));
}

#[test]
fn evt_ok_boolean_filter_lowers_the_verbatim_interior() {
    let source = concat!(
        "event E { orderId: String; }\n",
        "subscription s { events [ E ] consumer c when (orderId != \"\") }",
    );
    let compilation = compile_events(source);
    assert!(
        compilation.diagnostics.is_empty(),
        "expected a clean compile: {:?}",
        compilation.diagnostics
    );
    let model = compilation.model.expect("artifact");
    assert_eq!(
        model.subscriptions[0].when.as_deref(),
        Some("orderId != \"\"")
    );
}

// --- site: capability `when` (deferred pending conditions) ---------------------

fn compile_mox(source: &str) -> Compilation {
    compile_str("inline.mox", source, &DomainImports::default())
}

#[test]
fn capability_when_parse_error_is_rebased_onto_the_trimmed_interior() {
    let source = r#"
package demo

class Ticket { String title int amount }

actors Support {
    actor Customer

    capability ReadTicket on Ticket

    grant Customer {
        permit ReadTicket when (   amount <=   )
    }
}
"#;
    let compilation = compile_mox(source);
    assert!(
        compilation.model.is_none(),
        "a parse error must drop the artifact"
    );
    let matches: Vec<&Diagnostic> = compilation
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.message.starts_with("invalid `when` condition:"))
        .collect();
    assert_eq!(
        matches.len(),
        1,
        "one parse error: {:?}",
        compilation.diagnostics
    );
    let diagnostic = matches[0];
    assert_eq!(diagnostic.severity, Severity::Error);
    assert_eq!(
        diagnostic.message,
        "invalid `when` condition: unexpected trailing input",
    );
    assert_eq!(diagnostic.help, None);
    // The parser's offsets are rebased onto the trimmed text's first byte
    // and clamped into the paren interior: here onto the `<=`.
    assert_eq!(
        diagnostic.span,
        Some(span_of(source, "<=", 0)),
        "the span lands on the trailing operator"
    );
    // No typing diagnostic follows: the deferred pending conditions skip
    // typing for any condition that still carries parse errors (they were
    // reported during lowering).
}

#[test]
fn capability_when_non_boolean_spans_the_trimmed_interior_including_padding() {
    let source = r#"
package demo

class Ticket { String title int amount }

actors Support {
    actor Customer

    capability ReadTicket on Ticket

    grant Customer {
        permit ReadTicket when ( title )
    }
}
"#;
    let compilation = compile_mox(source);
    assert!(compilation.model.is_none());
    let matches: Vec<&Diagnostic> = compilation
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.message.contains("must be boolean"))
        .collect();
    assert_eq!(
        matches.len(),
        1,
        "one type error: {:?}",
        compilation.diagnostics
    );
    let diagnostic = matches[0];
    assert_eq!(diagnostic.severity, Severity::Error);
    assert_eq!(
        diagnostic.message,
        "invalid `when` condition: condition must be boolean, found string",
    );
    assert_eq!(diagnostic.help, None);
    // The non-boolean span is the interior exactly — inner region between
    // the parens, padding included.
    let parens = when_parens(source, 0);
    assert_eq!(
        diagnostic.span,
        Some((parens.start + 1..parens.end - 1).into())
    );
}

#[test]
fn capability_when_guard_lint_is_prefixed_and_spanned_on_the_literal() {
    let source = r#"
package demo

class Ticket { String title int amount }

actors Support {
    actor Customer

    capability ReadTicket on Ticket

    grant Customer {
        permit ReadTicket when (title == "draft")
    }
}
"#;
    let compilation = compile_mox(source);
    let matches: Vec<&Diagnostic> = compilation
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.severity == Severity::Warning)
        .collect();
    assert_eq!(matches.len(), 1, "one lint: {:?}", compilation.diagnostics);
    let diagnostic = matches[0];
    assert_eq!(
        diagnostic.message,
        "invalid `when` condition: guard compares string literal \"draft\" against \
         string-typed `title` — declare the field as an enum so guard literals are \
         checked against its members (docs/EXPRESSIONS.md rule L3)",
    );
    assert_eq!(diagnostic.help, None);
    assert_eq!(diagnostic.span, Some(span_of(source, "\"draft\"", 0)));
}

#[test]
fn capability_when_ok_boolean_lowers_the_verbatim_interior() {
    let source = r#"
package demo

class Ticket { String title int amount }

actors Support {
    actor Customer

    capability ReadTicket on Ticket

    grant Customer {
        permit ReadTicket when (  amount >= 10  )
    }
}
"#;
    let compilation = compile_mox(source);
    assert!(
        compilation.diagnostics.is_empty(),
        "expected a clean compile: {:?}",
        compilation.diagnostics
    );
    let model = compilation.model.expect("model lowered");
    assert_eq!(
        model.packages[0].actors[0].grants[0].entries[0]
            .when
            .as_deref(),
        Some("amount >= 10"),
        "the interior lowers ends-trimmed"
    );
}
