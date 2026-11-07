//! Integration tests for `.dmn` files: standalone decision model sources
//! made of `import` declarations followed by the `decision_model`
//! declaration (imports must come first). The import and type-reference
//! productions are reused from the `.mox`/`.ddd` surfaces; every `decision
//! model` word is contextual (the `dmn` table), while `import` and
//! `version` are real lexer keywords.

use rex_syntax::ast::*;
use rex_syntax::{format_dmn, parse_dmn, Span};

fn span_text(source: &str, span: Span) -> &str {
    &source[span.start..span.end]
}

fn parse_ok(source: &str) -> DmnFile {
    let result = parse_dmn(source);
    assert!(
        result.errors.is_empty(),
        "{source:?}: unexpected errors: {:?}",
        result.errors
    );
    result.ast.expect("expected an AST")
}

fn logic_span(logic: &DmnLogic) -> Span {
    match logic {
        DmnLogic::Table { span, .. }
        | DmnLogic::Expression { span, .. }
        | DmnLogic::Invoke { span, .. } => *span,
    }
}

fn expect_model(file: &DmnFile) -> &DmnDecisionModel {
    file.model.as_ref().expect("expected a decision model")
}

// --- parsing -----------------------------------------------------------------

#[test]
fn metadata_lines_and_namespace_parse() {
    let source = concat!(
        "decision_model Loan {\n",
        "    namespace banking.loan;\n",
        "    id \"loan-1\";\n",
        "    version \"1.0\";\n",
        "    effective \"2026-01-01\";\n",
        "    jurisdiction \"NZ\";\n",
        "}\n",
    );
    let file = parse_ok(source);
    let model = expect_model(&file);
    assert_eq!(model.name.text, "Loan");
    assert_eq!(
        model.namespace.as_ref().unwrap().full_name(),
        "banking.loan"
    );
    assert_eq!(model.metadata.len(), 4);
    assert_eq!(model.metadata[0].kind, DmnMetadataKind::Id);
    assert_eq!(model.metadata[0].value, "loan-1");
    assert_eq!(model.metadata[1].kind, DmnMetadataKind::Version);
    assert_eq!(model.metadata[1].value, "1.0");
    assert_eq!(model.metadata[2].kind, DmnMetadataKind::Effective);
    assert_eq!(model.metadata[2].value, "2026-01-01");
    assert_eq!(model.metadata[3].kind, DmnMetadataKind::Jurisdiction);
    assert_eq!(model.metadata[3].value, "NZ");
    assert!(model.inputs.is_empty());
}

#[test]
fn inputs_parse_as_typed_entries() {
    let source = "decision_model M { inputs { age: Number; tier: banking.Tier } }";
    let file = parse_ok(source);
    let model = expect_model(&file);
    assert_eq!(model.inputs.len(), 2);
    assert_eq!(model.inputs[0].name.text, "age");
    assert_eq!(model.inputs[0].type_ref.name.full_name(), "Number");
    assert_eq!(model.inputs[1].name.text, "tier");
    assert_eq!(model.inputs[1].type_ref.name.full_name(), "banking.Tier");
}

#[test]
fn knowledge_sources_parse_with_properties_and_docs() {
    let source = concat!(
        "decision_model M {\n",
        "    knowledge {\n",
        "        /// The policy manual.\n",
        "        source Policy {\n",
        "            name \"Underwriting Policy\";\n",
        "            reference \"https://example.com/policy\"\n",
        "        }\n",
        "    }\n",
        "}\n",
    );
    let file = parse_ok(source);
    let model = expect_model(&file);
    let source_decl = &model.knowledge_sources[0];
    assert_eq!(source_decl.name.text, "Policy");
    assert_eq!(
        source_decl.description.as_deref(),
        Some("The policy manual.")
    );
    assert_eq!(
        source_decl.display_name.as_deref(),
        Some("Underwriting Policy")
    );
    assert_eq!(
        source_decl.reference.as_deref(),
        Some("https://example.com/policy")
    );
    assert!(model.models.is_empty());
}

#[test]
fn models_parse_with_all_three_logic_kinds() {
    let source = concat!(
        "decision_model M {\n",
        "    knowledge {\n",
        "        model TableRules {\n",
        "            parameters { age: Number; }\n",
        "            output Boolean;\n",
        "            logic table \"rules.dmn\"\n",
        "        }\n",
        "        model ExprRules {\n",
        "            parameters {}\n",
        "            output Money;\n",
        "            logic expression \"fee = base * factor\"\n",
        "        }\n",
        "        model Invoked {\n",
        "            parameters { tier: String; }\n",
        "            output Rate;\n",
        "            logic invoke Pricing {\n",
        "                tier: \"applicant.tier\";\n",
        "                amount: \"request.amount\";\n",
        "            }\n",
        "        }\n",
        "    }\n",
        "}\n",
    );
    let file = parse_ok(source);
    let model = expect_model(&file);
    assert_eq!(model.knowledge_sources.len(), 0);
    assert_eq!(model.models.len(), 3);

    let table_rules = &model.models[0];
    assert_eq!(table_rules.name.text, "TableRules");
    assert_eq!(table_rules.parameters.len(), 1);
    assert_eq!(table_rules.parameters[0].name.text, "age");
    assert_eq!(table_rules.output.name.full_name(), "Boolean");
    assert_eq!(
        table_rules.logic,
        DmnLogic::Table {
            source: "rules.dmn".to_string(),
            span: logic_span(&table_rules.logic),
        }
    );

    let expr_rules = &model.models[1];
    assert!(expr_rules.parameters.is_empty());
    match &expr_rules.logic {
        DmnLogic::Expression { text, .. } => assert_eq!(text, "fee = base * factor"),
        other => panic!("expected expression logic, found {other:?}"),
    }

    let invoked = &model.models[2];
    match &invoked.logic {
        DmnLogic::Invoke {
            model, bindings, ..
        } => {
            assert_eq!(model.text, "Pricing");
            assert_eq!(bindings.len(), 2);
            assert_eq!(bindings[0].name.text, "tier");
            assert_eq!(bindings[0].expression, "applicant.tier");
            assert_eq!(bindings[1].name.text, "amount");
            assert_eq!(bindings[1].expression, "request.amount");
        }
        other => panic!("expected invoke logic, found {other:?}"),
    }
}

#[test]
fn decisions_parse_with_anonymous_and_named_outputs() {
    let source = concat!(
        "decision_model M {\n",
        "    decisions {\n",
        "        /// Whether the application passes.\n",
        "        decision Approve {\n",
        "            output Boolean;\n",
        "            logic invoke Rules { age: \"applicant.age\"; }\n",
        "        }\n",
        "        decision Fee {\n",
        "            outputs {\n",
        "                fee: Money;\n",
        "                waiver: Boolean;\n",
        "            }\n",
        "        }\n",
        "        decision Contract {\n",
        "            outputs { flag: Boolean }\n",
        "            logic table \"flags.dmn\"\n",
        "        }\n",
        "    }\n",
        "}\n",
    );
    let file = parse_ok(source);
    let model = expect_model(&file);
    assert_eq!(model.decisions.len(), 3);

    let approve = &model.decisions[0];
    assert_eq!(approve.name.text, "Approve");
    assert_eq!(
        approve.description.as_deref(),
        Some("Whether the application passes.")
    );
    // The anonymous `output Boolean;` line: an output with an empty name.
    assert_eq!(approve.outputs.len(), 1);
    assert_eq!(approve.outputs[0].name.text, "");
    assert_eq!(approve.outputs[0].type_ref.name.full_name(), "Boolean");
    assert!(matches!(approve.logic, Some(DmnLogic::Invoke { .. })));

    let fee = &model.decisions[1];
    assert_eq!(fee.outputs.len(), 2);
    assert_eq!(fee.outputs[0].name.text, "fee");
    assert_eq!(fee.outputs[1].name.text, "waiver");
    assert!(
        fee.logic.is_none(),
        "a decision may declare only its contract"
    );

    let contract = &model.decisions[2];
    assert!(matches!(contract.logic, Some(DmnLogic::Table { .. })));
}

#[test]
fn requirement_groups_parse_and_fold() {
    let source = concat!(
        "decision_model M {\n",
        "    requirements {\n",
        "        knowledge {\n",
        "            Approve requires Rules;\n",
        "            Fee invokes Pricing;\n",
        "        }\n",
        "        information {\n",
        "            Approve requires request;\n",
        "        }\n",
        "        authority {\n",
        "            Approve governed_by Policy;\n",
        "        }\n",
        "    }\n",
        "}\n",
    );
    let file = parse_ok(source);
    let model = expect_model(&file);
    assert_eq!(model.information_requirements.len(), 1);
    assert_eq!(model.knowledge_requirements.len(), 2);
    assert_eq!(model.authority_requirements.len(), 1);

    let knowledge = &model.knowledge_requirements;
    assert_eq!(knowledge[0].requiring.text, "Approve");
    assert_eq!(knowledge[0].required.text, "Rules");
    assert_eq!(knowledge[1].requiring.text, "Fee");
    assert_eq!(knowledge[1].required.text, "Pricing");

    let authority = &model.authority_requirements[0];
    assert_eq!(authority.requiring.text, "Approve");
    assert_eq!(authority.required.text, "Policy");
}

#[test]
fn services_and_views_parse() {
    let source = concat!(
        "decision_model M {\n",
        "    services {\n",
        "        /// Rates and fees.\n",
        "        service Pricing {\n",
        "            description \"Rates and fees.\";\n",
        "            inputs { tier amount }\n",
        "            outputs { rate }\n",
        "            encapsulates { Rules Pricing }\n",
        "        }\n",
        "    }\n",
        "    views {\n",
        "        view Letter {\n",
        "            show { Approve Fee }\n",
        "            direction outgoing;\n",
        "        }\n",
        "        view Brief {\n",
        "            description \"The committee brief.\";\n",
        "        }\n",
        "    }\n",
        "}\n",
    );
    let file = parse_ok(source);
    let model = expect_model(&file);
    let service = &model.services[0];
    assert_eq!(service.name.text, "Pricing");
    assert_eq!(service.description.as_deref(), Some("Rates and fees."));
    assert_eq!(
        service
            .inputs
            .iter()
            .map(|n| n.text.as_str())
            .collect::<Vec<_>>(),
        ["tier", "amount"]
    );
    assert_eq!(service.outputs.len(), 1);
    assert_eq!(service.encapsulates.len(), 2);

    let letter = &model.views[0];
    assert_eq!(letter.name.text, "Letter");
    assert!(letter.description.is_none());
    assert_eq!(
        letter
            .show
            .iter()
            .map(|n| n.text.as_str())
            .collect::<Vec<_>>(),
        ["Approve", "Fee"]
    );
    assert_eq!(letter.direction.as_ref().unwrap().text, "outgoing");

    let brief = &model.views[1];
    assert_eq!(brief.description.as_deref(), Some("The committee brief."));
    assert!(brief.show.is_empty());
    assert!(brief.direction.is_none());
}

#[test]
fn imports_parse_with_and_without_semicolons() {
    let file = parse_ok("import \"library.mox\"; import \"jurisdiction.mox\" decision_model M { }");
    assert_eq!(file.imports.len(), 2);
    assert_eq!(file.imports[0].path, "library.mox");
    assert_eq!(file.imports[1].path, "jurisdiction.mox");
    let source = "import \"library.mox\";\ndecision_model M { }";
    let file = parse_ok(source);
    assert_eq!(
        span_text(source, file.imports[0].span),
        "import \"library.mox\";"
    );
}

#[test]
fn interleaved_members_fold_into_per_kind_lists() {
    let source = concat!(
        "decision_model M {\n",
        "    jurisdiction \"NZ\";\n",
        "    inputs { age: Number; }\n",
        "    knowledge { source Policy { } }\n",
        "    id \"m-1\";\n",
        "    decisions { decision D { } }\n",
        "    services { service S { } }\n",
        "    views { view V { } }\n",
        "    knowledge { model Rules { output Boolean; logic table \"t\" } }\n",
        "    requirements { authority { D governed_by Policy; } }\n",
        "    version \"2.0\";\n",
        "}\n",
    );
    let file = parse_ok(source);
    let model = expect_model(&file);
    assert_eq!(model.metadata.len(), 3);
    assert_eq!(model.metadata[0].kind, DmnMetadataKind::Jurisdiction);
    assert_eq!(model.metadata[1].kind, DmnMetadataKind::Id);
    assert_eq!(model.metadata[2].kind, DmnMetadataKind::Version);
    assert_eq!(model.inputs.len(), 1);
    assert_eq!(model.knowledge_sources.len(), 1);
    assert_eq!(model.models.len(), 1);
    assert_eq!(model.decisions.len(), 1);
    assert_eq!(model.authority_requirements.len(), 1);
    assert_eq!(model.services.len(), 1);
    assert_eq!(model.views.len(), 1);
}

#[test]
fn escaped_dmn_words_remain_usable_as_names() {
    let file = parse_ok(concat!(
        "decision_model ^decision_model {\n",
        "    inputs { ^inputs: ^logic; }\n",
        "    knowledge { source ^source { name \"x\"; } }\n",
        "    decisions { decision ^decision { output ^output; } }\n",
        "    requirements { information { ^decision requires ^knowledge; } }\n",
        "    services { service ^service { inputs { ^view } } }\n",
        "    views { view ^view { show { ^service } } }\n",
        "}\n",
    ));
    let model = expect_model(&file);
    assert_eq!(model.name.text, "decision_model");
    assert_eq!(model.inputs[0].name.text, "inputs");
    assert_eq!(model.inputs[0].type_ref.name.full_name(), "logic");
    assert_eq!(model.knowledge_sources[0].name.text, "source");
    assert_eq!(model.decisions[0].name.text, "decision");
    assert_eq!(
        model.decisions[0].outputs[0].type_ref.name.full_name(),
        "output"
    );
    assert_eq!(model.information_requirements[0].requiring.text, "decision");
    assert_eq!(model.information_requirements[0].required.text, "knowledge");
    assert_eq!(model.services[0].name.text, "service");
    assert_eq!(model.services[0].inputs[0].text, "view");
    assert_eq!(model.views[0].name.text, "view");
    assert_eq!(model.views[0].show[0].text, "service");
}

#[test]
fn declarations_carry_whole_decl_spans() {
    let source = "decision_model M {\n    decisions { decision D { output Boolean; } }\n}\n";
    let file = parse_ok(source);
    let model = file.model.as_ref().unwrap();
    assert!(span_text(source, model.span).starts_with("decision_model M"));
    let decision = &model.decisions[0];
    assert!(span_text(source, decision.span).starts_with("decision D"));
    assert_eq!(
        span_text(source, decision.outputs[0].span),
        "output Boolean;"
    );
}

// --- error cases -------------------------------------------------------------

#[test]
fn import_after_model_is_an_error() {
    let source = "decision_model M { }\nimport \"late.mox\";";
    let result = parse_dmn(source);
    let error = result
        .errors
        .iter()
        .find(|error| error.message == "`import` after the `decision_model` declaration")
        .expect("expected the pinned late-import error");
    assert_eq!(span_text(source, error.span), "import \"late.mox\";");
    // Recovery keeps the model and drops the misplaced import.
    let file = result.ast.expect("expected a recovered AST");
    assert_eq!(file.imports.len(), 0);
    assert!(file.model.is_some());
}

#[test]
fn duplicate_decision_model_is_an_error_and_first_wins() {
    let result = parse_dmn("decision_model A { } decision_model B { }");
    assert!(
        result
            .errors
            .iter()
            .any(|error| error.message == "duplicate `decision_model` declaration"),
        "expected the duplicate-model error, got {:?}",
        result.errors
    );
    let file = result.ast.expect("expected a recovered AST");
    assert_eq!(file.model.as_ref().unwrap().name.text, "A");
}

#[test]
fn duplicate_namespace_is_an_error_and_first_wins() {
    let result = parse_dmn("decision_model M { namespace a.b namespace c.d }");
    assert!(
        result
            .errors
            .iter()
            .any(|error| error.message == "duplicate `namespace` line"),
        "expected the duplicate-namespace error, got {:?}",
        result.errors
    );
    let file = result.ast.expect("expected a recovered AST");
    assert_eq!(
        file.model
            .as_ref()
            .unwrap()
            .namespace
            .as_ref()
            .unwrap()
            .full_name(),
        "a.b"
    );
}

#[test]
fn a_file_without_a_decision_model_is_an_error() {
    let result = parse_dmn("import \"a.mox\";");
    assert!(
        result
            .errors
            .iter()
            .any(|error| error.message == "expected a `decision_model` declaration"),
        "expected the missing-model error, got {:?}",
        result.errors
    );
    assert!(result.ast.unwrap().model.is_none());
}

#[test]
fn missing_semicolons_are_tolerated_and_canonicalized() {
    // The EBNF's `;?` is the `.ddd` import rule: a missing statement `;`
    // is never an error; the formatter emits the canonical one.
    let source = concat!(
        "decision_model M {\n",
        "    jurisdiction \"NZ\"\n",
        "    inputs {\n",
        "        age: Number\n",
        "        tier: String;\n",
        "    }\n",
        "}\n",
    );
    let file = parse_ok(source);
    let model = expect_model(&file);
    assert_eq!(model.metadata.len(), 1);
    assert_eq!(model.inputs.len(), 2);
    let canonical = concat!(
        "decision_model M {\n",
        "    jurisdiction \"NZ\";\n",
        "    inputs {\n",
        "        age: Number;\n",
        "        tier: String;\n",
        "    }\n",
        "}\n",
    );
    assert_eq!(fmt_dmn(source), canonical);
}

#[test]
fn missing_metadata_value_is_reported_and_recovered() {
    let result = parse_dmn("decision_model M { id }");
    assert!(
        result
            .errors
            .iter()
            .any(|error| error.message == "expected a string after `id`"),
        "expected the missing-value error, got {:?}",
        result.errors
    );
    let file = result.ast.expect("expected a recovered AST");
    let metadata = &file.model.as_ref().unwrap().metadata;
    assert_eq!(metadata.len(), 1);
    assert_eq!(metadata[0].kind, DmnMetadataKind::Id);
    assert_eq!(metadata[0].value, "");
}

#[test]
fn missing_typed_entry_type_is_reported_and_recovered() {
    let result = parse_dmn("decision_model M { inputs { age } }");
    assert!(
        result
            .errors
            .iter()
            .any(|error| error.message == "expected `: <type>` after the name"),
        "expected the missing-type error, got {:?}",
        result.errors
    );
    let file = result.ast.expect("expected a recovered AST");
    let inputs = &file.model.as_ref().unwrap().inputs;
    assert_eq!(inputs.len(), 1);
    assert_eq!(inputs[0].name.text, "age");
}

#[test]
fn missing_bkm_logic_is_reported_and_recovered() {
    let source = "decision_model M { knowledge { model Rules { output Boolean; } } }";
    let result = parse_dmn(source);
    assert!(
        result
            .errors
            .iter()
            .any(|error| error.message == "expected `logic` in the model body"),
        "expected the missing-logic error, got {:?}",
        result.errors
    );
    let file = result.ast.expect("expected a recovered AST");
    let models = &file.model.as_ref().unwrap().models;
    assert_eq!(models.len(), 1);
    assert_eq!(models[0].output.name.full_name(), "Boolean");
    // The recovered model carries empty expression logic.
    assert_eq!(
        models[0].logic,
        DmnLogic::Expression {
            text: String::new(),
            span: logic_span(&models[0].logic),
        }
    );
}

#[test]
fn duplicate_bkm_clauses_are_reported_and_first_wins() {
    let source = concat!(
        "decision_model M { knowledge { model R {\n",
        "    output Boolean; output Money;\n",
        "    logic table \"a\" logic table \"b\"\n",
        "} } }\n",
    );
    let result = parse_dmn(source);
    assert!(
        result
            .errors
            .iter()
            .any(|error| error.message == "duplicate `output` clause"),
        "expected the duplicate-output error, got {:?}",
        result.errors
    );
    assert!(
        result
            .errors
            .iter()
            .any(|error| error.message == "duplicate `logic` clause"),
        "expected the duplicate-logic error, got {:?}",
        result.errors
    );
    let file = result.ast.expect("expected a recovered AST");
    let model = &file.model.as_ref().unwrap().models[0];
    assert_eq!(model.output.name.full_name(), "Boolean");
    match &model.logic {
        DmnLogic::Table { source, .. } => assert_eq!(source, "a"),
        other => panic!("expected table logic, found {other:?}"),
    }
}

#[test]
fn missing_requirement_connective_is_reported_and_recovered() {
    let source = "decision_model M { requirements { information { Approve; } } }";
    let result = parse_dmn(source);
    assert!(
        result.errors.iter().any(|error| {
            error.message == "expected `requires`, `invokes`, or `governed_by` after the name"
        }),
        "expected the missing-connective error, got {:?}",
        result.errors
    );
    let file = result.ast.expect("expected a recovered AST");
    let information = &file.model.as_ref().unwrap().information_requirements;
    assert_eq!(information.len(), 1);
    assert_eq!(information[0].requiring.text, "Approve");
    assert_eq!(information[0].required.text, "");
}

#[test]
fn missing_logic_kind_is_reported_and_recovered() {
    let source = "decision_model M { decisions { decision D { logic } } }";
    let result = parse_dmn(source);
    assert!(
        result.errors.iter().any(|error| {
            error.message == "expected `table`, `expression`, or `invoke` after `logic`"
        }),
        "expected the missing-kind error, got {:?}",
        result.errors
    );
    let file = result.ast.expect("expected a recovered AST");
    let decision = &file.model.as_ref().unwrap().decisions[0];
    assert!(matches!(decision.logic, Some(DmnLogic::Expression { .. })));
}

#[test]
fn junk_regions_recover_to_the_next_declaration() {
    // Garbage before the imports.
    let result = parse_dmn("bogus garbage 42 import \"x.mox\" decision_model M { }");
    assert!(!result.errors.is_empty(), "expected a junk-region error");
    let file = result.ast.expect("expected a recovered AST");
    assert_eq!(file.imports.len(), 1);
    assert!(file.model.is_some());

    // A `.mox`-style declaration is junk in a `.dmn` file.
    let result = parse_dmn("class Bogus { int pages } decision_model M { }");
    assert!(!result.errors.is_empty(), "expected a junk-region error");
    let file = result.ast.expect("expected a recovered AST");
    assert!(file.imports.is_empty());
    assert!(file.model.is_some());
}

#[test]
fn junk_members_recover_to_the_next_member() {
    let source = "decision_model M { @@@ inputs { age: Number; } decisions { decision D { } } }";
    let result = parse_dmn(source);
    assert!(
        result
            .errors
            .iter()
            .any(|error| error.message == "expected a decision model member"),
        "expected the junk-member error, got {:?}",
        result.errors
    );
    let file = result.ast.expect("expected a recovered AST");
    let model = file.model.as_ref().unwrap();
    assert_eq!(model.inputs.len(), 1);
    assert_eq!(model.decisions.len(), 1);
}

#[test]
fn a_missing_block_brace_is_reported_without_swallowing_the_close() {
    // `inputs decisions { ... }` — the inputs block's `{` is missing; the
    // decisions block after it still parses and the model body's `}` is
    // never swallowed.
    let source = "decision_model M { inputs decisions { decision D { } } }";
    let result = parse_dmn(source);
    assert!(
        result
            .errors
            .iter()
            .any(|error| error.message == "expected `{` after `inputs`"),
        "expected the missing-brace error, got {:?}",
        result.errors
    );
    let file = result.ast.expect("expected a recovered AST");
    let model = file.model.as_ref().unwrap();
    assert!(model.inputs.is_empty());
    assert_eq!(model.decisions.len(), 1);
}

#[test]
fn adversarial_inputs_do_not_panic() {
    let nasty = [
        "",
        "   \n\t",
        "import",
        "import \"",
        "import \"unterminated",
        "decision_model",
        "decision_model M",
        "decision_model M {",
        "decision_model M { namespace",
        "decision_model M { namespace a.",
        "decision_model M { id",
        "decision_model M { version",
        "decision_model M { version \"1\"",
        "decision_model M { inputs",
        "decision_model M { inputs {",
        "decision_model M { inputs { age",
        "decision_model M { inputs { age:",
        "decision_model M { inputs { age: }",
        "decision_model M { knowledge",
        "decision_model M { knowledge {",
        "decision_model M { knowledge { source",
        "decision_model M { knowledge { source S",
        "decision_model M { knowledge { source S {",
        "decision_model M { knowledge { source S { name",
        "decision_model M { knowledge { model",
        "decision_model M { knowledge { model R",
        "decision_model M { knowledge { model R {",
        "decision_model M { knowledge { model R { parameters",
        "decision_model M { knowledge { model R { output",
        "decision_model M { knowledge { model R { output T",
        "decision_model M { knowledge { model R { logic",
        "decision_model M { knowledge { model R { logic table",
        "decision_model M { knowledge { model R { logic invoke",
        "decision_model M { knowledge { model R { logic invoke P",
        "decision_model M { knowledge { model R { logic invoke P {",
        "decision_model M { knowledge { model R { logic invoke P { tier } } } }",
        "decision_model M { decisions",
        "decision_model M { decisions {",
        "decision_model M { decisions { decision",
        "decision_model M { decisions { decision D {",
        "decision_model M { decisions { decision D { outputs",
        "decision_model M { decisions { decision D { outputs {",
        "decision_model M { decisions { decision D { outputs { f } } } }",
        "decision_model M { requirements",
        "decision_model M { requirements {",
        "decision_model M { requirements { information",
        "decision_model M { requirements { information {",
        "decision_model M { requirements { information { A requires",
        "decision_model M { requirements { information { A requires ; } } } }",
        "decision_model M { services { service",
        "decision_model M { services { service S {",
        "decision_model M { services { service S { inputs {",
        "decision_model M { services { service S { description",
        "decision_model M { views { view",
        "decision_model M { views { view V { direction",
        "decision_model M { views { view V { show { A B } } } }",
        "decision_model M { } } }",
        "@@@ ### :",
        "^decision_model ^M { } ^decision_model ^N { }",
        "decision_model M { ^inputs { ^age: ^logic; } }",
    ];
    for input in nasty {
        let result = parse_dmn(input);
        let _ = format!("{result:?}"); // must be debuggable and must not panic
        let _ = format_dmn(input); // must not panic either
    }
}

// --- formatting --------------------------------------------------------------

fn fmt_dmn(source: &str) -> String {
    format_dmn(source).expect("format succeeds")
}

/// Formats a positive source and pins the round-trip: the output is a
/// fixpoint and still parses cleanly.
fn assert_round_trip(source: &str) {
    let once = fmt_dmn(source);
    let twice = fmt_dmn(&once);
    assert_eq!(once, twice, "formatting must be a fixpoint for:\n{source}");
    let result = parse_dmn(&once);
    assert!(
        result.errors.is_empty(),
        "formatted output must parse cleanly, got {:?}\n{once}",
        result.errors
    );
}

#[test]
fn empty_dmn_file_formats_to_empty_output() {
    assert_eq!(fmt_dmn(""), "");
    assert_eq!(fmt_dmn("  \n\n "), "");
}

#[test]
fn canonical_layout_sections_and_members() {
    let messy = concat!(
        "import   \"a.mox\" ;\n",
        "\n",
        "\n",
        "decision_model  M {\n",
        "   jurisdiction  \"NZ\"\n",
        "   effective \"2026-01-01\"\n",
        "   version \"1.0\";\n",
        "   id \"m-1\"\n",
        "   namespace   banking.loan ;\n",
        "    /// The facts.\n",
        "    inputs {  age :  Number tier : banking.Tier }\n",
        "    knowledge { model R { output Boolean logic table \"r.dmn\" }\n",
        "        source P { reference \"ops\" } }\n",
        "    requirements { authority { age governed_by P ; } }\n",
        "    decisions { decision D { output Boolean; logic table \"d.dmn\" } }\n",
        "}\n",
    );
    let canonical = concat!(
        "import \"a.mox\";\n",
        "\n",
        "decision_model M {\n",
        "    namespace banking.loan;\n",
        "    id \"m-1\";\n",
        "    version \"1.0\";\n",
        "    effective \"2026-01-01\";\n",
        "    jurisdiction \"NZ\";\n",
        "    /// The facts.\n",
        "    inputs {\n",
        "        age: Number;\n",
        "        tier: banking.Tier;\n",
        "    }\n",
        "    knowledge {\n",
        "        source P {\n",
        "            reference \"ops\";\n",
        "        }\n",
        "        model R {\n",
        "            output Boolean;\n",
        "            logic table \"r.dmn\"\n",
        "        }\n",
        "    }\n",
        "    decisions {\n",
        "        decision D {\n",
        "            output Boolean;\n",
        "            logic table \"d.dmn\"\n",
        "        }\n",
        "    }\n",
        "    requirements {\n",
        "        authority {\n",
        "            age governed_by P;\n",
        "        }\n",
        "    }\n",
        "}\n",
    );
    assert_eq!(fmt_dmn(messy), canonical);
    assert_eq!(fmt_dmn(canonical), canonical);
}

#[test]
fn requirement_groups_reorder_information_knowledge_authority() {
    let messy = concat!(
        "decision_model M {\n",
        "    requirements {\n",
        "        knowledge { D requires R; }\n",
        "        authority { D governed_by P; }\n",
        "        information { D requires x; }\n",
        "    }\n",
        "}\n",
    );
    let canonical = concat!(
        "decision_model M {\n",
        "    requirements {\n",
        "        information {\n",
        "            D requires x;\n",
        "        }\n",
        "        knowledge {\n",
        "            D requires R;\n",
        "        }\n",
        "        authority {\n",
        "            D governed_by P;\n",
        "        }\n",
        "    }\n",
        "}\n",
    );
    assert_eq!(fmt_dmn(messy), canonical);
    assert_eq!(fmt_dmn(canonical), canonical);
}

#[test]
fn name_lists_stay_inline_while_they_fit() {
    let canonical = concat!(
        "decision_model M {\n",
        "    services {\n",
        "        service S {\n",
        "            inputs { tier amount }\n",
        "            outputs { rate }\n",
        "            encapsulates { Rules Pricing }\n",
        "        }\n",
        "    }\n",
        "    views {\n",
        "        view V {\n",
        "            show { Approve Fee }\n",
        "            direction outgoing;\n",
        "        }\n",
        "    }\n",
        "}\n",
    );
    assert_eq!(fmt_dmn(canonical), canonical);
}

#[test]
fn a_long_name_list_renders_one_name_per_line() {
    let messy = concat!(
        "decision_model M {\n",
        "    services {\n",
        "        service InternationalClaimsPricingAndEligibilityOrchestration {\n",
        "            inputs { tierOfTheApplicantUnderwritingCategoryWithAdjustment secondLayerReinsuranceRiskClassBand }\n",
        "        }\n",
        "    }\n",
        "}\n",
    );
    let canonical = concat!(
        "decision_model M {\n",
        "    services {\n",
        "        service InternationalClaimsPricingAndEligibilityOrchestration {\n",
        "            inputs {\n",
        "                tierOfTheApplicantUnderwritingCategoryWithAdjustment\n",
        "                secondLayerReinsuranceRiskClassBand\n",
        "            }\n",
        "        }\n",
        "    }\n",
        "}\n",
    );
    assert_eq!(fmt_dmn(messy), canonical);
    assert_eq!(fmt_dmn(canonical), canonical);
    // A comment inside the braces also forces the multiline shape, and
    // travels with the list.
    let commented = concat!(
        "decision_model M {\n",
        "    services {\n",
        "        service S {\n",
        "            inputs { // trailing\n",
        "                tier\n",
        "            }\n",
        "        }\n",
        "    }\n",
        "}\n",
    );
    assert_eq!(fmt_dmn(commented), commented);
}

#[test]
fn logic_clauses_carry_no_semicolon() {
    let canonical = concat!(
        "decision_model M {\n",
        "    knowledge {\n",
        "        model Table {\n",
        "            output Boolean;\n",
        "            logic table \"r.dmn\"\n",
        "        }\n",
        "        model Expr {\n",
        "            output Money;\n",
        "            logic expression \"fee = base * factor\"\n",
        "        }\n",
        "        model Invoke {\n",
        "            output Rate;\n",
        "            logic invoke P {\n",
        "                tier: \"x\";\n",
        "            }\n",
        "        }\n",
        "    }\n",
        "}\n",
    );
    assert_eq!(fmt_dmn(canonical), canonical);
    assert_round_trip(canonical);
}

#[test]
fn doc_comments_travel_with_their_declarations() {
    let messy = concat!(
        "decision_model B { }\n",
        "/// The approvals.\n",
        "decision_model A { decisions { /// Whether.\n",
        "    decision D { } } }\n",
    );
    let canonical = concat!(
        "decision_model B {}\n",
        "\n",
        "/// The approvals.\n",
        "decision_model A {\n",
        "    decisions { /// Whether.\n",
        "        decision D {}\n",
        "    }\n",
        "}\n",
    );
    assert_eq!(fmt_dmn(messy), canonical);
    assert_eq!(fmt_dmn(canonical), canonical);
}

#[test]
fn comments_are_preserved() {
    let source = concat!(
        "// header\n",
        "decision_model M { // inner\n",
        "    // about the inputs\n",
        "    inputs { age: Number; } // trailing\n",
        "}\n",
        "// bye\n",
    );
    let expected = concat!(
        "// header\n",
        "decision_model M { // inner\n",
        "    // about the inputs\n",
        "    inputs {\n",
        "        age: Number;\n",
        "    } // trailing\n",
        "}\n",
        "\n",
        "// bye\n",
    );
    assert_eq!(fmt_dmn(source), expected);
    assert_eq!(fmt_dmn(expected), expected);
}

#[test]
fn parse_errors_still_format() {
    assert_eq!(fmt_dmn("decision_model"), "decision_model\n");
    assert_eq!(
        fmt_dmn("@@@ decision_model M { }"),
        "decision_model M {}\n\n@ @ @\n"
    );
}

#[test]
fn all_positive_shapes_round_trip() {
    assert_round_trip("decision_model Loan {\n    namespace banking.loan;\n    id \"loan-1\";\n    version \"1.0\";\n    effective \"2026-01-01\";\n    jurisdiction \"NZ\";\n    inputs { age: Number; tier: banking.Tier; }\n}\n");
    assert_round_trip("decision_model M {\n    knowledge {\n        source Policy { name \"P\"; reference \"ops\"; }\n        model Rules { parameters { age: Number } output Boolean; logic table \"r.dmn\" }\n        model Pricing { parameters { tier: String } output Rate; logic invoke P { tier: \"x\"; } }\n    }\n}\n");
    assert_round_trip("decision_model M {\n    decisions { decision Approve { outputs { fee: Money; waiver: Boolean } logic invoke Rules { age: \"a.age\"; } } }\n}\n");
    assert_round_trip("import \"library.mox\";\nimport \"jurisdiction.mox\";\n\ndecision_model ^decision_model { inputs { ^inputs: ^logic; } }\n");
    assert_round_trip("decision_model M {\n    services { service S { description \"d\"; inputs { tier } outputs { rate } encapsulates { R } } }\n    views { view V { show { A B } direction outgoing; } }\n}\n");
}

#[test]
fn canonical_fixture_is_a_fixpoint() {
    let source = include_str!("../../../tests/conformance/models/dmn.dmn");
    let once = fmt_dmn(source);
    let twice = fmt_dmn(&once);
    assert_eq!(once, twice, "formatting must be a fixpoint for:\n{source}");
}

#[test]
fn canonical_fixture_parses_cleanly_and_recovers_a_full_model() {
    // The fixture deliberately omits a few statement `;`s — tolerated by
    // the EBNF's `;?` — so a clean parse is the bar; the AST must carry
    // every member kind.
    let source = include_str!("../../../tests/conformance/models/dmn.dmn");
    let result = parse_dmn(source);
    assert!(
        result.errors.is_empty(),
        "unexpected fixture errors: {:?}",
        result.errors
    );
    let model = expect_model(result.ast.as_ref().expect("the fixture recovers to a file"));
    assert_eq!(model.name.text, "LoanApproval");
    assert_eq!(model.decisions.len(), 3);
    assert_eq!(model.knowledge_sources.len(), 2);
    assert_eq!(model.models.len(), 3);
}

#[test]
fn golden_fixture_parses_cleanly_and_matches_the_formatter() {
    let golden = include_str!("../../../tests/conformance/fmt/dmn.fmt.dmn");
    parse_ok(golden);
    let source = include_str!("../../../tests/conformance/models/dmn.dmn");
    assert_eq!(fmt_dmn(source), golden);
}
