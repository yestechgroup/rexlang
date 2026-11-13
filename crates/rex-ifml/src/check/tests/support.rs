//! Shared fixtures for the check test suite: the shop model, the
//! actor policy, and the `diags` entry helpers.

use super::*;

pub(crate) use rex_ir::dmn::DmnModel;
pub(crate) use rex_ir::{
    ActorDef, ActorModel, ActorsDef, CapabilityDef, ClassDef, Feature, FeatureKind, Multiplicity,
    Operation, OperationParam, Package, PrimitiveType, TypeRef,
};

pub(crate) const PKG: &str = "nz.example.shop";

pub(crate) fn class_ref(name: &str) -> TypeRef {
    TypeRef::Class {
        package: PKG.to_string(),
        name: name.to_string(),
    }
}

pub(crate) fn attr(name: &str, primitive: PrimitiveType) -> Feature {
    Feature::new(
        name,
        FeatureKind::Attribute,
        TypeRef::Primitive(primitive),
        Multiplicity::REQUIRED,
    )
}

/// A commerce-ish domain: Product → Category, Order → Customer.
pub(crate) fn shop_model() -> Model {
    let mut model = Model::new();
    let mut package = Package::new(PKG);
    package.classes.push(ClassDef::new(
        "Product",
        vec![],
        vec![
            attr("name", PrimitiveType::String),
            attr("price", PrimitiveType::Int),
            attr("status", PrimitiveType::String),
            Feature::new(
                "category",
                FeatureKind::CrossReference,
                class_ref("Category"),
                Multiplicity::REQUIRED,
            ),
        ],
    ));
    package.classes.push(ClassDef::new(
        "Category",
        vec![],
        vec![attr("name", PrimitiveType::String)],
    ));
    package.classes.push(ClassDef::new(
        "Order",
        vec![],
        vec![
            attr("total", PrimitiveType::Int),
            Feature::new(
                "customer",
                FeatureKind::CrossReference,
                class_ref("Customer"),
                Multiplicity::REQUIRED,
            ),
        ],
    ));
    package.classes.push(ClassDef::new(
        "Customer",
        vec![],
        vec![attr("name", PrimitiveType::String)],
    ));
    model.packages.push(package);
    model
}

/// A `score(int) -> boolean` package function — the shape a lowered
/// sigil import contributes to `Package::functions`.
pub(crate) fn score_function() -> Operation {
    Operation::new(
        "score",
        TypeRef::Primitive(PrimitiveType::Boolean),
        vec![OperationParam {
            name: "value".to_string(),
            type_: TypeRef::Primitive(PrimitiveType::Int),
            multiplicity: None,
        }],
    )
}

/// The shop domain plus the `score(int) -> boolean` package function.
pub(crate) fn scored_shop_model() -> Model {
    let mut model = shop_model();
    model.packages[0].functions.push(score_function());
    model
}

/// Compiles `source` as `app.ifml` with no imports, checks against
/// `model`, and keeps the **error-severity** diagnostics: these fixtures
/// pin the E-band passes; the lint band's warnings have their own tests
/// ([`lint_diags`] keeps them).
pub(crate) fn diags(source: &str, model: &Model) -> Vec<IfmlDiagnostic> {
    let compilation = crate::compile_ifml_str("app.ifml", source, &crate::IfmlImports::default())
        .expect("source must parse and resolve");
    check_ifml(&compilation, Some(model))
        .into_iter()
        .filter(|diagnostic| diagnostic.severity.is_error())
        .collect()
}

/// An actors layer declaring what app-style flows require: two
/// capabilities on `Customer` and two roles.
pub(crate) fn auth_model() -> ActorModel {
    ActorModel::new().block(
        ActorsDef::new("Auth")
            .actor(ActorDef::new("admin"))
            .actor(ActorDef::new("manager"))
            .capability(CapabilityDef::new("ManageCustomer", class_ref("Customer")))
            .capability(CapabilityDef::new("ApproveCustomer", class_ref("Customer"))),
    )
}

/// [`diags`] with an actor layer — error-severity only, like [`diags`].
pub(crate) fn diags_with_actors(
    source: &str,
    model: &Model,
    actors: &[ActorModel],
) -> Vec<IfmlDiagnostic> {
    let compilation = crate::compile_ifml_str("app.ifml", source, &crate::IfmlImports::default())
        .expect("source must parse and resolve");
    check_ifml_with_actors(&compilation, Some(model), actors)
        .into_iter()
        .filter(|diagnostic| diagnostic.severity.is_error())
        .collect()
}

/// A decision layer over the shop domain: the `AssessRefund` service
/// exposing the `RefundVerdict` decision's typed outputs.
pub(crate) fn shop_decision() -> DmnModel {
    let mut model = DmnModel::new();
    model.name = Some("ShopDecisions".to_string());
    model.decisions.push(rex_ir::dmn::Decision {
        name: "RefundVerdict".to_string(),
        description: None,
        outputs: vec![
            rex_ir::dmn::Output {
                name: "eligible".to_string(),
                type_ref: TypeRef::Primitive(PrimitiveType::Boolean),
            },
            rex_ir::dmn::Output {
                name: "reason".to_string(),
                type_ref: TypeRef::Primitive(PrimitiveType::String),
            },
        ],
        logic: None,
        emits: Vec::new(),
    });
    model.services.push(rex_ir::dmn::DecisionService {
        name: "AssessRefund".to_string(),
        description: None,
        inputs: vec!["Order".to_string()],
        outputs: vec!["RefundVerdict".to_string()],
        encapsulates: Vec::new(),
    });
    model
}

/// [`diags`] with a decision layer — error-severity only, like [`diags`].
pub(crate) fn diags_with_decisions(
    source: &str,
    model: &Model,
    decisions: &[DmnModel],
) -> Vec<IfmlDiagnostic> {
    let compilation = crate::compile_ifml_str("app.ifml", source, &crate::IfmlImports::default())
        .expect("source must parse and resolve");
    check_ifml_with_decisions(&compilation, Some(model), &[], decisions)
        .into_iter()
        .filter(|diagnostic| diagnostic.severity.is_error())
        .collect()
}
