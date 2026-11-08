//! `ddd` tests: rule 10 capabilities.

use crate::support::*;

// --- rule 10: capabilities ---------------------------------------------------

#[test]
fn capabilities_are_recorded_unvalidated_without_actors() {
    let source = concat!(
        "application A {\n",
        "    module m {\n",
        "        service S {\n",
        "            boolean a() capability NobodyDeclaredThis;\n",
        "        }\n",
        "    }\n",
        "}\n",
    );
    let compilation = compile_domain(source);
    assert_clean(&compilation);
    let model = compilation.model.expect("artifact lowered");
    assert_eq!(
        model.modules[0].services[0].operations[0].capabilities,
        ["NobodyDeclaredThis"]
    );
}

#[test]
fn capabilities_must_exist_in_the_actor_model() {
    let source = concat!(
        "application A {\n",
        "    module m {\n",
        "        service LoanService {\n",
        "            boolean borrow() capability BorrowBooks, RenewLoans;\n",
        "        }\n",
        "    }\n",
        "}\n",
    );
    let domains = [("library.mox".to_string(), DOMAIN.to_string())];
    let compilation = compile_ddd_str_with_actors(
        "design.ddd",
        source,
        &domains,
        &rex_driver::DomainImports::default(),
        &[actors_with(&["BorrowBooks"])],
    );
    let diagnostic = single(
        &compilation,
        "unknown capability 'RenewLoans' on operation 'borrow' of service 'LoanService'",
    );
    assert_eq!(diagnostic.span, Some(span_of(source, "RenewLoans", 0)));
    assert!(
        diagnostic
            .help
            .as_deref()
            .unwrap_or_default()
            .contains("the actor model declares: BorrowBooks"),
        "help lists the declared capabilities: {:?}",
        diagnostic.help
    );
    assert!(compilation.model.is_none());
}

#[test]
fn declared_capabilities_are_accepted_by_the_actors_variant() {
    let source = concat!(
        "application A {\n",
        "    module m {\n",
        "        service S {\n",
        "            boolean a() capability Declared;\n",
        "        }\n",
        "    }\n",
        "}\n",
    );
    let domains = [("library.mox".to_string(), DOMAIN.to_string())];
    let compilation = compile_ddd_str_with_actors(
        "design.ddd",
        source,
        &domains,
        &rex_driver::DomainImports::default(),
        &[actors_with(&["Declared"])],
    );
    assert_clean(&compilation);
}

#[test]
fn capabilities_against_an_empty_actor_model_list_nothing() {
    let source = concat!(
        "application A {\n",
        "    module m {\n",
        "        service S {\n",
        "            boolean a() capability Anything;\n",
        "        }\n",
        "    }\n",
        "}\n",
    );
    let domains = [("library.mox".to_string(), DOMAIN.to_string())];
    let compilation = compile_ddd_str_with_actors(
        "design.ddd",
        source,
        &domains,
        &rex_driver::DomainImports::default(),
        &[ActorModel::new()],
    );
    let diagnostic = single(&compilation, "unknown capability 'Anything'");
    assert!(
        diagnostic
            .help
            .as_deref()
            .unwrap_or_default()
            .contains("the actor model declares no capabilities"),
        "help explains the empty union: {:?}",
        diagnostic.help
    );
}

#[test]
fn rule_10_suggests_the_nearest_declared_name() {
    let source = concat!(
        "application A {\n",
        "    module m {\n",
        "        service LoanService {\n",
        "            boolean borrow() capability BorrowBookss;\n",
        "        }\n",
        "    }\n",
        "}\n",
    );
    let domains = [("library.mox".to_string(), DOMAIN.to_string())];
    let compilation = compile_ddd_str_with_actors(
        "design.ddd",
        source,
        &domains,
        &rex_driver::DomainImports::default(),
        &[actors_with(&["BorrowBooks"])],
    );
    let diagnostic = single(&compilation, "unknown capability 'BorrowBookss'");
    assert_eq!(diagnostic.span, Some(span_of(source, "BorrowBookss", 0)));
    assert_eq!(
        diagnostic.help.as_deref(),
        Some("did you mean 'BorrowBooks'?"),
        "a near-miss names its intended capability instead of listing everything"
    );
    let code = diagnostic
        .code
        .as_ref()
        .expect("the unknown-capability diagnostic carries a code");
    assert_eq!(code.name(), "unknownCapability");
    let span = span_of(source, "BorrowBookss", 0);
    let json = serde_json::to_value(code).expect("the code serializes");
    assert_eq!(
        json,
        serde_json::json!({
            "unknownCapability": {
                "capability": "BorrowBookss",
                "suggestion": "BorrowBooks",
                "span": [span.start, span.end]
            }
        }),
        "the payload carries the suggestion and the edit span for the LSP quick fix"
    );
}

#[test]
fn rule_10_suggests_the_case_variant() {
    let source = concat!(
        "application A {\n",
        "    module m {\n",
        "        service LoanService {\n",
        "            boolean borrow() capability borrowbooks;\n",
        "        }\n",
        "    }\n",
        "}\n",
    );
    let domains = [("library.mox".to_string(), DOMAIN.to_string())];
    let compilation = compile_ddd_str_with_actors(
        "design.ddd",
        source,
        &domains,
        &rex_driver::DomainImports::default(),
        &[actors_with(&["BorrowBooks"])],
    );
    let diagnostic = single(&compilation, "unknown capability 'borrowbooks'");
    assert_eq!(
        diagnostic.help.as_deref(),
        Some("did you mean 'BorrowBooks'?"),
        "a case-variant match wins outright"
    );
}

#[test]
fn rule_10_without_a_suggestion_lists_the_declared_names() {
    let source = concat!(
        "application A {\n",
        "    module m {\n",
        "        service LoanService {\n",
        "            boolean borrow() capability WhollyUnrelated;\n",
        "        }\n",
        "    }\n",
        "}\n",
    );
    let domains = [("library.mox".to_string(), DOMAIN.to_string())];
    let compilation = compile_ddd_str_with_actors(
        "design.ddd",
        source,
        &domains,
        &rex_driver::DomainImports::default(),
        &[actors_with(&["BorrowBooks"])],
    );
    let diagnostic = single(&compilation, "unknown capability 'WhollyUnrelated'");
    assert!(
        diagnostic
            .help
            .as_deref()
            .unwrap_or_default()
            .contains("the actor model declares: BorrowBooks"),
        "no near-miss keeps the exhaustive list: {:?}",
        diagnostic.help
    );
    let code = diagnostic.code.as_ref().expect("carries a code");
    let json = serde_json::to_value(code).expect("the code serializes");
    assert_eq!(
        json["unknownCapability"]["suggestion"],
        serde_json::Value::Null,
        "no suggestion is omitted from the payload"
    );
}

#[test]
fn rule_10_pools_capabilities_across_actor_models() {
    let source = concat!(
        "application A {\n",
        "    module m {\n",
        "        service LoanService {\n",
        "            boolean borrow() capability BorrowBooks, RenewLoans;\n",
        "        }\n",
        "    }\n",
        "}\n",
    );
    let domains = [("library.mox".to_string(), DOMAIN.to_string())];
    let compilation = compile_ddd_str_with_actors(
        "design.ddd",
        source,
        &domains,
        &rex_driver::DomainImports::default(),
        &[actors_with(&["BorrowBooks"]), actors_with(&["RenewLoans"])],
    );
    assert_clean(&compilation);
}
