//! Check tests: the decision dimension — `data:` names resolve against
//! the decision services of the provided decision models, after the
//! domain classes (`E0110`/`E0111`; the did-you-mean hint on the
//! unknown-data-entity error; the empty-slice and reachability
//! contracts).

use super::support::*;
use super::*;

#[test]
fn service_data_resolves_silently_and_keeps_the_ir_untouched() {
    let source = r#"
view "Refunds" {
    component "assessment" {
        type: details;
        data: AssessRefund;
    }
}
"#;
    // The service's output decisions form the component's data contract —
    // a check-only fact: no diagnostic, and (the shape above carries no
    // expression) nothing else changes.
    assert!(diags_with_decisions(source, &shop_model(), &[shop_decision()]).is_empty());
}

#[test]
fn domain_classes_resolve_first_with_unchanged_diagnostics() {
    // `Order` is a domain class: the decision dimension must not
    // interfere — the filter still binds the class's features exactly as
    // without decisions.
    let source = r#"
view "Refunds" {
    component "grid" {
        type: list;
        data: Order;
        filter: total > 0;
    }
}
"#;
    assert!(diags_with_decisions(source, &shop_model(), &[shop_decision()]).is_empty());
}

#[test]
fn unknown_data_name_gains_the_service_did_you_mean_hint() {
    let source = r#"
view "Refunds" {
    component "assessment" {
        type: details;
        data: AssessRefrund;
    }
}
"#;
    let diags = diags_with_decisions(source, &shop_model(), &[shop_decision()]);
    assert_eq!(diags.len(), 1, "{diags:?}");
    assert_eq!(diags[0].code, E_UNKNOWN_ENTITY);
    assert_eq!(
        diags[0].message,
        "view 'Refunds' component 'assessment': unknown data entity 'AssessRefrund'; did you mean 'AssessRefund'?"
    );
    // The expression walk's data diagnostic keeps its existing span (the
    // whole property assignment — the E0100 convention).
    let (start, end) = diags[0].span;
    assert_eq!(&source[start..end], "data: AssessRefrund;");
}

#[test]
fn unknown_data_name_stays_unchanged_when_nothing_is_close() {
    let source = r#"
view "Refunds" {
    component "assessment" {
        type: details;
        data: Ghost;
    }
}
"#;
    let diags = diags_with_decisions(source, &shop_model(), &[shop_decision()]);
    assert_eq!(diags.len(), 1, "{diags:?}");
    assert_eq!(
        diags[0].message,
        "view 'Refunds' component 'assessment': unknown data entity 'Ghost'"
    );
}

#[test]
fn unknown_data_name_is_byte_identical_without_a_decisions_slice() {
    let source = r#"
view "Refunds" {
    component "assessment" {
        type: details;
        data: AssessRefrund;
    }
}
"#;
    // The empty slice skips the pass entirely: the diagnostic keeps its
    // exact pre-decision shape (no did-you-mean over nothing).
    let diags = diags(source, &shop_model());
    assert_eq!(diags.len(), 1, "{diags:?}");
    assert_eq!(
        diags[0].message,
        "view 'Refunds' component 'assessment': unknown data entity 'AssessRefrund'"
    );
}

#[test]
fn output_less_service_data_reports_e0110_naming_the_service() {
    let mut outputless = shop_decision();
    outputless.services[0].outputs.clear();
    let source = r#"
view "Refunds" {
    component "assessment" {
        type: details;
        data: AssessRefund;
    }
}
"#;
    let diags = diags_with_decisions(source, &shop_model(), &[outputless]);
    assert_eq!(diags.len(), 1, "{diags:?}");
    assert_eq!(diags[0].code, E_EMPTY_DECISION_SERVICE);
    assert_eq!(
        diags[0].message,
        "decision service 'AssessRefund' declares no outputs"
    );
    let (start, end) = diags[0].span;
    assert_eq!(&source[start..end], "AssessRefund");
}

#[test]
fn service_declared_by_two_models_reports_e0111_listing_both() {
    let mut billing = shop_decision();
    billing.name = Some("BillingDecisions".to_string());
    let source = r#"
view "Refunds" {
    component "assessment" {
        type: details;
        data: AssessRefund;
    }
}
"#;
    let diags = diags_with_decisions(source, &shop_model(), &[shop_decision(), billing]);
    assert_eq!(diags.len(), 1, "{diags:?}");
    assert_eq!(diags[0].code, E_AMBIGUOUS_DECISION_SERVICE);
    assert_eq!(
        diags[0].message,
        "decision service 'AssessRefund' is declared by two models — 'ShopDecisions' and 'BillingDecisions'"
    );
}

#[test]
fn ambiguous_domain_data_stays_ambiguous_even_with_services() {
    // A name two *domain* packages declare resolved ambiguously — the
    // decision dimension never rescues domain ambiguity.
    let mut model = shop_model();
    let mut second = Package::new("other.shop");
    second.classes.push(ClassDef::new(
        "Order",
        vec![],
        vec![attr("total", PrimitiveType::Int)],
    ));
    model.packages.push(second);
    let source = r#"
view "Refunds" {
    component "grid" {
        type: list;
        data: Order;
    }
}
"#;
    let diags = diags_with_decisions(source, &model, &[shop_decision()]);
    assert_eq!(diags.len(), 1, "{diags:?}");
    assert_eq!(
        diags[0].message,
        "view 'Refunds' component 'grid': ambiguous data entity 'Order'"
    );
}

#[test]
fn instantiated_module_data_sites_are_diagnosed_and_unused_siblings_stay_inert() {
    let imports = {
        let mut imports = crate::IfmlImports::default();
        imports.provide(
            "app.ifml",
            "panels.ifml",
            r#"
module "Verdict" {
    input { rows: Int }
    output { total: Int }

    component "verdict" {
        type: details;
        data: AssessRefund;
    }
}

module "Unused" {
    input { rows: Int }
    output { total: Int }

    component "ghost" {
        type: details;
        data: AssessRefund;
    }
}
"#,
        );
        imports
    };
    let source = r#"
import "panels.ifml";

view "Refunds" {
    use "Verdict" as verdict { rows: 1; };
}
"#;
    let compile =
        || crate::compile_ifml_str("app.ifml", source, &imports).expect("compiles and resolves");
    // A healthy service: the used module's site resolves silently.
    let diags: Vec<_> =
        check_ifml_with_decisions(&compile(), Some(&shop_model()), &[], &[shop_decision()])
            .into_iter()
            .filter(|diagnostic| diagnostic.severity.is_error())
            .collect();
    assert!(diags.is_empty(), "{diags:?}");

    // The output-less shape diagnoses the used module's site only — the
    // unused sibling's identical site stays inert.
    let mut outputless = shop_decision();
    outputless.services[0].outputs.clear();
    let compilation = compile();
    let diags: Vec<_> =
        check_ifml_with_decisions(&compilation, Some(&shop_model()), &[], &[outputless])
            .into_iter()
            .filter(|diagnostic| diagnostic.severity.is_error())
            .collect();
    assert_eq!(diags.len(), 1, "{diags:?}");
    assert_eq!(diags[0].code, E_EMPTY_DECISION_SERVICE);
    assert_eq!(
        diags[0].message,
        "decision service 'AssessRefund' declares no outputs"
    );
    assert_eq!(
        diags[0].file, "panels.ifml",
        "the span lives in the module file"
    );
    let (start, end) = diags[0].span;
    assert_eq!(
        &source_text(&compilation, "panels.ifml")[start..end],
        "AssessRefund"
    );
}

#[test]
fn empty_decisions_slice_skips_the_pass_entirely() {
    // A service-shaped name with no decisions slice behaves exactly as
    // before the feature: the plain unknown-data-entity error.
    let source = r#"
view "Refunds" {
    component "assessment" {
        type: details;
        data: AssessRefund;
    }
}
"#;
    let diags = diags(source, &shop_model());
    assert_eq!(diags.len(), 1, "{diags:?}");
    assert_eq!(diags[0].code, E_UNKNOWN_ENTITY);
    assert_eq!(
        diags[0].message,
        "view 'Refunds' component 'assessment': unknown data entity 'AssessRefund'"
    );
}

/// The text of one compilation file — for span slicing in imported files.
fn source_text(compilation: &IfmlCompilation, path: &str) -> String {
    compilation
        .index
        .files
        .iter()
        .find(|file| file.path == path)
        .map(|file| file.text.clone())
        .expect("imported file present")
}
