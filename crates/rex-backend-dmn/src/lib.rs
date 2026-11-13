//! The `.dmn` execution backend: lowers a compiled [`DmnModel`] artifact
//! into the dsntk model the decision engine evaluates — rexlang authors
//! and validates the decision model, dsntk executes it.
//!
//! This backend consumes **only** the [`rex_ir::dmn::DmnModel`] artifact
//! plus the markdown decision-table contents the artifact references (the
//! driver's discipline: the host provides the table contents keyed by the
//! path as written; the backend stays filesystem-free). No domain model
//! is consumed: every type the artifact carries is already resolved, and
//! the FEEL mapping below is decided from the artifact alone.
//!
//! # The seam
//!
//! * [`lower`] builds one dsntk [`Definitions`]: inputs, knowledge
//!   sources, business knowledge models, decisions (with their three
//!   requirement kinds), and decision services — each element identified
//!   by its model-unique name, so provenance traces attribute tables to
//!   decisions and service invocations resolve by name.
//! * [`DecisionEngine`] wraps the lowered model in the dsntk
//!   [`ModelEvaluator`] and evaluates one decision service against a JSON
//!   input context, returning the JSON result and the [`Provenance`]
//!   record: model identity, service, inputs, outputs, the matched
//!   decision-table rules (from the evaluator's trace), and the
//!   evaluation timestamp — the "why did the system make this decision"
//!   audit trail for regulated applications.
//! * [`DecisionEngine::evaluate`] takes the clock as a parameter
//!   (`now`), so the provenance record is deterministic and can be
//!   pinned byte-for-byte in tests.
//!
//! # Types: the FEEL mapping
//!
//! The artifact's type references map to FEEL as follows:
//!
//! | rex-ir type | FEEL |
//! |---|---|
//! | `String` | `string` |
//! | `Int` / `Long` / `Short` / `Byte` / `Float` / `Double` | `number` |
//! | `Boolean` | `boolean` |
//! | `Date` | `date` |
//! | a class / enum / datatype / interface / vocabulary name | opaque — the name is carried, no FEEL type is declared |
//!
//! Class-typed inputs are **pass-through contexts**: the caller supplies
//! a JSON object, the engine lowers it to a FEEL context, and decision
//! logic navigates it (`ticket.needsRefund`) without any deep validation.
//! Enum-typed inputs are strings. This is the documented v1 contract —
//! deep typing of domain classes in FEEL is out of scope.
//!
//! # Multi-output decisions
//!
//! A decision with named outputs declares a component item definition
//! named by the decision (the named outputs as its components); a
//! decision-table logic evaluates to a FEEL context keyed by the output
//! names and the JSON result carries them as an object. Where the logic
//! produces a single value (an invoked model, an expression), the
//! decision's value is what its logic produces.
//!
//! # Rejections
//!
//! The backend rejects what it cannot execute, naming the element (the
//! Cedar-backend convention): a referenced markdown table that was not
//! provided ([`GenerateError::TableNotProvided`]), a table that does not
//! parse ([`GenerateError::TableError`]), a decision service referencing
//! an unknown element or exposing a logic-less output decision
//! ([`GenerateError::LogiclessDecision`] — logic-less decisions are
//! architecture-first declarations, but the engine executes logic), an
//! evaluation naming an unknown service, a service invocation missing a
//! declared input, and a service outputting a non-data value.
//!
//! # Determinism
//!
//! Lowering is a pure function of the artifact and the table contents.
//! The provenance record is deterministic given the injected clock.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use dsntk_common::HRef;
use dsntk_feel::context::FeelContext;
use dsntk_feel::values::Value;
use dsntk_feel::{FeelType, Name};
use dsntk_model::builder::DefinitionsBuilder;
use dsntk_model::{
    Binding, BusinessKnowledgeModel, Decision, DecisionService, Definitions, ExpressionInstance,
    FunctionKind, InformationItem, InformationRequirement, InputData, Invocation, ItemDefinition,
    KnowledgeRequirement, KnowledgeSource, LiteralExpression,
};
use dsntk_model_evaluator::trace::TableTrace;
use dsntk_model_evaluator::ModelEvaluator;
use rex_ir::dmn::{DecisionLogic, DecisionService as IrDecisionService, DmnModel, MetadataKind};
use rex_backend_common::canonical_key_order;
use rex_ir::{PrimitiveType, TypeRef};
use serde::Serialize;

/// A generate-time failure: the artifact is valid rexlang but names
/// something the engine cannot execute (the Cedar-backend convention —
/// rejections name the element).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum GenerateError {
    /// A decision logic references a markdown table that was not
    /// provided. The host provides table contents keyed by the path as
    /// written (the driver's discipline).
    #[error("decision table \"{path}\" was not provided")]
    TableNotProvided {
        /// The table path as written in the artifact.
        path: String,
    },
    /// A provided markdown table does not parse in the dsntk shape.
    #[error("decision table \"{path}\": {reason}")]
    TableError {
        /// The table path as written in the artifact.
        path: String,
        /// The recognizer's parse failure reason.
        reason: String,
    },
    /// A decision service references an element that does not exist in
    /// the model (the driver validates service endpoints; a hand-built
    /// or stale artifact can still disagree).
    #[error("decision service '{service}' references unknown element '{name}'")]
    UnknownServiceEndpoint {
        /// The declaring service.
        service: String,
        /// The referenced element name.
        name: String,
    },
    /// A decision service exposes a logic-less output decision: the
    /// declaration is architecture-first, but the engine executes logic.
    #[error("decision service '{service}' outputs logic-less decision '{decision}'")]
    LogiclessDecision {
        /// The declaring service.
        service: String,
        /// The logic-less decision.
        decision: String,
    },
    /// The engine was asked to evaluate a service the model does not
    /// declare.
    #[error("unknown decision service '{service}'; the model declares: {declared}")]
    UnknownService {
        /// The requested service.
        service: String,
        /// The declared services, comma-separated.
        declared: String,
    },
    /// The evaluation input is not a JSON object.
    #[error("decision service '{service}' input must be a JSON object")]
    InvalidInput {
        /// The invoked service.
        service: String,
    },
    /// The model evaluator rejected the lowered model.
    #[error("the lowered model does not evaluate: {reason}")]
    Evaluator {
        /// The evaluator's failure reason.
        reason: String,
    },
    /// The service produced a value that is not a data value (a function
    /// definition or another engine-internal value leaked to the output).
    #[error("decision service '{service}' produced a non-data output value: {kind}")]
    NonDataOutput {
        /// The invoked service.
        service: String,
        /// The value's engine type description.
        kind: String,
    },
}

/// The type name an artifact type reference carries into the engine:
/// primitives map to their FEEL names, domain names to themselves.
fn type_name(type_ref: &TypeRef) -> String {
    match type_ref {
        TypeRef::Primitive(PrimitiveType::String) => "string".to_string(),
        TypeRef::Primitive(PrimitiveType::Boolean) => "boolean".to_string(),
        TypeRef::Primitive(PrimitiveType::Date) => "date".to_string(),
        TypeRef::Primitive(primitive) if primitive.is_numeric() => "number".to_string(),
        // Domain types (classes, enums, datatypes, interfaces,
        // vocabularies) and the unlisted primitives lower as `Any`: the
        // engine types them opaquely and evaluates them as pass-through
        // contexts / strings (the documented v1 contract).
        _ => "Any".to_string(),
    }
}

/// The FEEL type an artifact type reference declares: primitives map to
/// FEEL scalars; domain types stay opaque — the engine types them `Any`
/// and evaluates them as pass-through contexts (the documented v1
/// contract: class-typed inputs arrive as JSON objects and decision
/// logic navigates them without deep validation).
fn feel_type(type_ref: &TypeRef) -> FeelType {
    match type_ref {
        TypeRef::Primitive(PrimitiveType::String) => FeelType::String,
        TypeRef::Primitive(PrimitiveType::Boolean) => FeelType::Boolean,
        TypeRef::Primitive(PrimitiveType::Date) => FeelType::Date,
        TypeRef::Primitive(primitive) if primitive.is_numeric() => FeelType::Number,
        _ => FeelType::Any,
    }
}

/// Lowers one named, typed entry (a model input, an output, or a
/// parameter) into the information item the engine carries for it.
fn information_item(name: &str, type_ref: &TypeRef) -> InformationItem {
    InformationItem::build(
        name.to_string(),
        type_name(type_ref),
        Some(feel_type(type_ref)),
    )
}

/// Lowers one decision logic's table: the markdown table is recognized
/// with the dsntk recognizer and rebuilt with the owning element's name
/// as its identifier, so provenance traces attribute matched rules to
/// the element.
fn lower_table(
    source: &str,
    owner_name: &str,
    tables: &[(String, String)],
) -> Result<dsntk_model::DecisionTable, GenerateError> {
    let content = tables
        .iter()
        .find(|(path, _)| path == source)
        .map(|(_, content)| content)
        .ok_or_else(|| GenerateError::TableNotProvided {
            path: source.to_string(),
        })?;
    let recognized: dsntk_model::DecisionTable = dsntk_recognizer::from_markdown(content, false)
        .map_err(|reason| GenerateError::TableError {
            path: source.to_string(),
            reason: reason.to_string(),
        })?
        .into();
    Ok(dsntk_model::DecisionTable::build(
        Some(owner_name),
        recognized.information_item_name().clone(),
        recognized.input_clauses().cloned().collect(),
        recognized.output_clauses().cloned().collect(),
        recognized.annotations().cloned().collect(),
        recognized.rules().cloned().collect(),
        recognized.hit_policy(),
        *recognized.aggregation(),
        recognized.preferred_orientation().clone(),
        recognized.output_label().clone(),
    ))
}

/// Lowers one decision logic (a decision's or a business knowledge
/// model's) into the engine's expression instance.
///
/// `output_sources` maps the named outputs of the decision's
/// information-required decisions to the decisions declaring them (the
/// `.dmn` binding-resolution universe: "a binding name resolves to a
/// declared input, a required decision, or a named output of a required
/// decision"). The engine scopes a required decision's results under the
/// decision's name, so a binding naming an output lowers to the
/// qualified navigation the engine reads (`RefundAuthority.
/// requires_approval`).
fn lower_logic(
    logic: &DecisionLogic,
    owner_name: &str,
    tables: &[(String, String)],
    output_sources: &BTreeMap<String, String>,
) -> Result<ExpressionInstance, GenerateError> {
    let _ = owner_name;
    match logic {
        DecisionLogic::Expression { text } => Ok(ExpressionInstance::LiteralExpression(Box::new(
            LiteralExpression::build(text.clone()),
        ))),
        DecisionLogic::Table { source } => Ok(ExpressionInstance::DecisionTable(Box::new(
            lower_table(source, owner_name, tables)?,
        ))),
        DecisionLogic::Invoke { model, bindings } => {
            Ok(ExpressionInstance::Invocation(Box::new(Invocation::build(
                ExpressionInstance::LiteralExpression(Box::new(LiteralExpression::build(
                    model.clone(),
                ))),
                bindings
                    .iter()
                    .map(|binding| {
                        // A binding naming an output of an information-required
                        // decision lowers to the qualified navigation.
                        let expression = match output_sources.get(binding.expression.trim()) {
                            Some(decision_name)
                                if is_plain_identifier(binding.expression.trim()) =>
                            {
                                format!("{decision_name}.{}", binding.expression.trim())
                            }
                            _ => binding.expression.clone(),
                        };
                        Binding::build(
                            InformationItem::build(binding.name.clone(), "Any", None),
                            Some(ExpressionInstance::LiteralExpression(Box::new(
                                LiteralExpression::build(expression),
                            ))),
                        )
                    })
                    .collect(),
            ))))
        }
    }
}

/// Lowers one business knowledge model: the model's logic becomes the
/// encapsulated function; the model's parameters are the function's
/// formal parameters (the invocation binds them by name).
fn lower_bkm(
    bkm: &rex_ir::dmn::Bkm,
    tables: &[(String, String)],
) -> Result<BusinessKnowledgeModel, GenerateError> {
    let formal_parameters: Vec<InformationItem> = bkm
        .parameters
        .iter()
        .map(|parameter| information_item(&parameter.name, &parameter.type_ref))
        .collect();
    let body = lower_logic(&bkm.logic, &bkm.name, tables, &BTreeMap::new())?;
    Ok(BusinessKnowledgeModel::build(
        Some(&bkm.name),
        bkm.name.clone(),
        bkm.description.as_deref(),
        type_name(&bkm.output),
        Some(feel_type(&bkm.output)),
        formal_parameters,
        dsntk_model::FunctionDefinition::build(vec![], Some(body), FunctionKind::Feel),
    ))
}

/// Builds the component item definition of a multi-output decision: the
/// decision's named outputs as its components, under the decision's name
/// — the engine evaluates such a decision to a FEEL context keyed by the
/// output names.
fn multi_output_components(decision: &rex_ir::dmn::Decision) -> ItemDefinition {
    let components = decision
        .outputs
        .iter()
        .map(|output| {
            ItemDefinition::build(
                None,
                output.name.clone(),
                Some(&type_name(&output.type_ref)),
                Some(feel_type(&output.type_ref)),
                None,
                vec![],
                false,
            )
        })
        .collect();
    ItemDefinition::build(
        Some(&decision.name),
        decision.name.clone(),
        None,
        None,
        None,
        components,
        false,
    )
}

/// Lowers one decision with its three requirement kinds. Requirements
/// reference their targets by `#name` fragments — the driver validated
/// every reference, so each target exists in the lowered model.
#[allow(clippy::too_many_arguments)]
fn lower_decision(
    decision: &rex_ir::dmn::Decision,
    information_requirements: &[rex_ir::dmn::Requirement],
    knowledge_requirements: &[rex_ir::dmn::Requirement],
    authority_requirements: &[rex_ir::dmn::Requirement],
    decision_index: &BTreeMap<String, &rex_ir::dmn::Decision>,
    decision_names: &BTreeSet<&str>,
    tables: &[(String, String)],
) -> Result<(Decision, Option<ItemDefinition>), GenerateError> {
    // A single (anonymous) output names the variable by the decision with
    // the output's type; a multi-output decision references the component
    // item definition the decision's name identifies.
    let (variable, item_definition) = match decision.outputs.len() {
        1 => (
            information_item(&decision.name, &decision.outputs[0].type_ref),
            None,
        ),
        _ => (
            InformationItem::build(&decision.name, &decision.name, None),
            Some(multi_output_components(decision)),
        ),
    };
    // The named outputs of this decision's information-required
    // decisions — the binding universe the surface documents.
    let mut output_sources = BTreeMap::new();
    for requirement in information_requirements
        .iter()
        .filter(|requirement| requirement.requiring == decision.name)
    {
        if let Some(required_decision) = decision_index.get(&requirement.required) {
            for output in &required_decision.outputs {
                output_sources
                    .entry(output.name.clone())
                    .or_insert_with(|| required_decision.name.clone());
            }
        }
    }
    let logic = decision
        .logic
        .as_ref()
        .map(|logic| lower_logic(logic, &decision.name, tables, &output_sources))
        .transpose()?;
    let built = Decision::build(
        Some(&decision.name),
        decision.name.clone(),
        decision.description.as_deref(),
        variable,
        logic,
        information_requirements
            .iter()
            .filter(|requirement| requirement.requiring == decision.name)
            .map(|requirement| {
                if decision_names.contains(requirement.required.as_str()) {
                    InformationRequirement::build(
                        Some(
                            HRef::try_from(format!("#{}", requirement.required).as_str()).unwrap(),
                        ),
                        None,
                    )
                } else {
                    InformationRequirement::build(
                        None,
                        Some(
                            HRef::try_from(format!("#{}", requirement.required).as_str()).unwrap(),
                        ),
                    )
                }
            })
            .collect(),
        knowledge_requirements
            .iter()
            .filter(|requirement| requirement.requiring == decision.name)
            .map(|requirement| {
                KnowledgeRequirement::build(
                    HRef::try_from(format!("#{}", requirement.required).as_str()).unwrap(),
                )
            })
            .collect(),
        authority_requirements
            .iter()
            .filter(|requirement| requirement.requiring == decision.name)
            .map(|requirement| {
                dsntk_model::AuthorityRequirement::build(
                    Some(HRef::try_from(format!("#{}", requirement.required).as_str()).unwrap()),
                    None,
                    None,
                )
            })
            .collect(),
    );
    Ok((built, item_definition))
}

/// Lowers one decision service. The exposed outputs are the service's
/// output decisions; the remaining encapsulated elements are internals
/// the engine pre-evaluates (a decision never appears in both lists —
/// the engine evaluates the two collections separately). A decision
/// listed as an output must carry logic — the engine executes it.
fn lower_service(
    service: &IrDecisionService,
    decision_index: &BTreeMap<String, &rex_ir::dmn::Decision>,
    known_names: &BTreeSet<&str>,
) -> Result<DecisionService, GenerateError> {
    for output in &service.outputs {
        let decision =
            decision_index
                .get(output)
                .ok_or_else(|| GenerateError::UnknownServiceEndpoint {
                    service: service.name.clone(),
                    name: output.clone(),
                })?;
        if decision.logic.is_none() {
            return Err(GenerateError::LogiclessDecision {
                service: service.name.clone(),
                decision: output.clone(),
            });
        }
    }
    for name in service
        .inputs
        .iter()
        .chain(service.outputs.iter())
        .chain(service.encapsulates.iter())
    {
        if !known_names.contains(name.as_str()) {
            return Err(GenerateError::UnknownServiceEndpoint {
                service: service.name.clone(),
                name: name.clone(),
            });
        }
    }
    let input_data = service
        .inputs
        .iter()
        .map(|name| HRef::try_from(format!("#{name}").as_str()).unwrap())
        .collect();
    let output_decisions = service
        .outputs
        .iter()
        .map(|name| HRef::try_from(format!("#{name}").as_str()).unwrap())
        .collect();
    let encapsulated_decisions = service
        .encapsulates
        .iter()
        .filter(|name| !service.outputs.contains(name))
        .filter(|name| decision_index.contains_key(name.as_str()))
        .map(|name| HRef::try_from(format!("#{name}").as_str()).unwrap())
        .collect();
    Ok(DecisionService::build(
        Some(&service.name),
        service.name.clone(),
        service.description.as_deref(),
        InformationItem::build(&service.name, "Any", None),
        input_data,
        vec![],
        output_decisions,
        encapsulated_decisions,
    ))
}

/// Lowers a compiled decision-model artifact into the dsntk model the
/// engine evaluates.
///
/// Every element is identified by its model-unique name (the driver
/// validated name uniqueness across all sections), the three requirement
/// kinds wire the graph by `#name` fragments, and each markdown decision
/// table is recognized with the dsntk recognizer and rebuilt under its
/// owning element's name.
pub fn lower(model: &DmnModel, tables: &[(String, String)]) -> Result<Definitions, GenerateError> {
    let model_name = model
        .name
        .clone()
        .unwrap_or_else(|| "DecisionModel".to_string());
    let namespace = model
        .namespace
        .clone()
        .unwrap_or_else(|| format!("urn:rexlang:{model_name}"));

    let decision_index: BTreeMap<String, &rex_ir::dmn::Decision> = model
        .decisions
        .iter()
        .map(|decision| (decision.name.clone(), decision))
        .collect();
    let decision_names: BTreeSet<&str> = model
        .decisions
        .iter()
        .map(|decision| decision.name.as_str())
        .collect();
    let input_names: BTreeSet<&str> = model
        .inputs
        .iter()
        .map(|input| input.name.as_str())
        .collect();

    let mut builder = DefinitionsBuilder::new(model_name.clone(), namespace.clone());

    // Opaque (domain) types carry no FEEL type: the engine evaluates the
    // class-typed inputs as pass-through contexts and navigates them
    // dynamically. Primitives carry their FEEL types on the information
    // items directly — no item definitions at all.

    for input in &model.inputs {
        builder = builder.input_data(InputData::build(
            Some(&input.name),
            input.name.clone(),
            input.description.as_deref(),
            type_name(&input.type_ref),
            Some(feel_type(&input.type_ref)),
        ));
    }
    for source in &model.knowledge_sources {
        builder = builder.knowledge_source(KnowledgeSource::build(
            Some(&source.name),
            source.name.clone(),
            source.description.as_deref(),
        ));
    }
    for bkm in &model.business_knowledge_models {
        builder = builder.business_knowledge_model(lower_bkm(bkm, tables)?);
    }
    for decision in &model.decisions {
        let (decision, item_definition) = lower_decision(
            decision,
            &model.information_requirements,
            &model.knowledge_requirements,
            &model.authority_requirements,
            &decision_index,
            &decision_names,
            tables,
        )?;
        if let Some(item_definition) = item_definition {
            builder = builder.item_definition(item_definition);
        }
        builder = builder.decision(decision);
    }
    let mut known_names = decision_names.clone();
    known_names.extend(input_names);
    known_names.extend(
        model
            .knowledge_sources
            .iter()
            .map(|source| source.name.as_str()),
    );
    known_names.extend(
        model
            .business_knowledge_models
            .iter()
            .map(|bkm| bkm.name.as_str()),
    );
    for service in &model.services {
        builder = builder.decision_service(lower_service(service, &decision_index, &known_names)?);
    }
    Ok(builder.build())
}

/// One evaluated decision service: the JSON result value and the
/// provenance record.
#[derive(Debug, Clone)]
pub struct Evaluation {
    /// The service's JSON result value.
    pub value: serde_json::Value,
    /// The evaluation's provenance record.
    pub provenance: Provenance,
}

/// The "why did the system make this decision" audit record: model
/// identity, service, inputs, outputs, the matched decision-table rules,
/// and the evaluation timestamp.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Provenance {
    /// The model's declared `id` metadata, when present.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_id: Option<String>,
    /// The model's declared `version` metadata, when present.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_version: Option<String>,
    /// The model's namespace.
    pub namespace: String,
    /// The model's name.
    pub model_name: String,
    /// The invoked decision service.
    pub service: String,
    /// The input context the service was evaluated against.
    pub inputs: serde_json::Value,
    /// The service's JSON output value.
    pub outputs: serde_json::Value,
    /// The matched decision-table rules, one entry per evaluated table.
    pub rule_matches: Vec<RuleMatch>,
    /// The evaluation timestamp, as produced by the injected clock.
    pub evaluated_at: String,
}

/// The matched rules of one evaluated decision table: the owning
/// element's name (the identifier the lowering gave the table) and the
/// zero-based numbers of the rules that matched, in table order.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RuleMatch {
    /// The evaluated table's identifier (the owning element's name).
    pub table_id: String,
    /// The numbers of the rules that matched.
    pub matched_rules: Vec<usize>,
}

/// The decision engine: the lowered model wrapped in the dsntk model
/// evaluator, plus the model identity the provenance record carries.
pub struct DecisionEngine {
    evaluator: Arc<ModelEvaluator>,
    namespace: String,
    model_name: String,
    model_id: Option<String>,
    model_version: Option<String>,
    services: BTreeSet<String>,
}

impl DecisionEngine {
    /// Lowers the artifact and builds the engine: parse-time validation
    /// already ran; a failing evaluator build means the lowered model
    /// itself is inconsistent.
    pub fn new(model: &DmnModel, tables: &[(String, String)]) -> Result<Self, GenerateError> {
        let model_name = model
            .name
            .clone()
            .unwrap_or_else(|| "DecisionModel".to_string());
        let namespace = model
            .namespace
            .clone()
            .unwrap_or_else(|| format!("urn:rexlang:{model_name}"));
        let mut model_id = None;
        let mut model_version = None;
        for entry in &model.metadata {
            match entry.kind {
                MetadataKind::Id => model_id = Some(entry.value.clone()),
                MetadataKind::Version => model_version = Some(entry.value.clone()),
                _ => {}
            }
        }
        let services: BTreeSet<String> = model
            .services
            .iter()
            .map(|service| service.name.clone())
            .collect();
        let definitions = lower(model, tables)?;
        let evaluator =
            ModelEvaluator::new(&[definitions]).map_err(|reason| GenerateError::Evaluator {
                reason: reason.to_string(),
            })?;
        Ok(Self {
            evaluator,
            namespace,
            model_name,
            model_id,
            model_version,
            services,
        })
    }

    /// Evaluates one decision service of the model against a JSON input
    /// context, returning the JSON result and the provenance record.
    ///
    /// The clock is injected (`now`): the provenance record is
    /// deterministic given it, and can be pinned byte-for-byte in tests.
    pub fn evaluate(
        &self,
        service: &str,
        inputs: &serde_json::Value,
        now: impl Fn() -> String,
    ) -> Result<Evaluation, GenerateError> {
        if !self.services.contains(service) {
            let declared = self.services.iter().cloned().collect::<Vec<_>>().join(", ");
            return Err(GenerateError::UnknownService {
                service: service.to_string(),
                declared,
            });
        }
        let input_context = input_context(service, inputs)?;
        let (value, traces) = self.evaluator.evaluate_invocable_traced(
            &self.namespace,
            &self.model_name,
            service,
            &input_context,
        );
        let value = value_to_json(service, value)?;
        // Key order is canonicalized (the shared backend discipline): the
        // JSON echoes into the provenance record must not drift with the
        // workspace's serde_json feature unification.
        let value = canonical_key_order(value);
        let inputs = canonical_key_order(inputs.clone());
        let provenance = Provenance {
            model_id: self.model_id.clone(),
            model_version: self.model_version.clone(),
            namespace: self.namespace.clone(),
            model_name: self.model_name.clone(),
            service: service.to_string(),
            inputs,
            outputs: value.clone(),
            // One decision may evaluate more than once within a service
            // invocation (as an encapsulated pre-evaluation and again as
            // a downstream requirement) — the same table evaluating to
            // the same matched rules reports once.
            rule_matches: {
                let mut seen = BTreeSet::new();
                traces
                    .into_iter()
                    .map(
                        |TableTrace {
                             table_id,
                             matched_rules,
                         }| RuleMatch {
                            table_id,
                            matched_rules,
                        },
                    )
                    .filter(|rule_match| {
                        seen.insert((
                            rule_match.table_id.clone(),
                            rule_match.matched_rules.clone(),
                        ))
                    })
                    .collect()
            },
            evaluated_at: now(),
        };
        Ok(Evaluation { value, provenance })
    }
}

/// Whether the text is a plain FEEL identifier (letters, digits,
/// underscores — nothing that would make a qualified or literal form).
fn is_plain_identifier(text: &str) -> bool {
    !text.is_empty()
        && text
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
        && !text.chars().next().unwrap().is_ascii_digit()
}

/// The FEEL number zero — the fallback for a JSON number the decimal
/// engine cannot represent (JSON's float infinities).
fn zero_number() -> dsntk_feel::FeelNumber {
    "0".parse().expect("the FEEL number zero parses")
}

/// Lowers the evaluation's JSON input into the FEEL context the engine
/// evaluates against; extra keys pass through (documented v1 contract).
fn input_context(service: &str, inputs: &serde_json::Value) -> Result<FeelContext, GenerateError> {
    let serde_json::Value::Object(entries) = inputs else {
        return Err(GenerateError::InvalidInput {
            service: service.to_string(),
        });
    };
    let mut context = FeelContext::default();
    for (key, value) in entries {
        context.set_entry(&Name::from(key.as_str()), json_to_value(value));
    }
    Ok(context)
}

/// Lowers one JSON value into the FEEL value: numbers, strings,
/// booleans, nulls, arrays, and objects (contexts) map directly.
fn json_to_value(value: &serde_json::Value) -> Value {
    match value {
        serde_json::Value::Null => Value::Null(None),
        serde_json::Value::Bool(flag) => Value::Boolean(*flag),
        serde_json::Value::Number(number) => {
            Value::Number(number.to_string().parse().unwrap_or_else(|_| zero_number()))
        }
        serde_json::Value::String(text) => Value::String(text.clone()),
        serde_json::Value::Array(items) => Value::List(items.iter().map(json_to_value).collect()),
        serde_json::Value::Object(entries) => {
            let mut context = FeelContext::default();
            for (key, value) in entries {
                context.set_entry(&Name::from(key.as_str()), json_to_value(value));
            }
            Value::Context(context)
        }
    }
}

/// Raises the FEEL value the service produced to JSON: data values
/// (numbers, strings, booleans, dates and times, lists, contexts, nulls)
/// map directly; engine-internal values are a named rejection.
fn value_to_json(service: &str, value: Value) -> Result<serde_json::Value, GenerateError> {
    Ok(match value {
        Value::Null(_) => serde_json::Value::Null,
        Value::Boolean(flag) => serde_json::Value::Bool(flag),
        Value::Number(number) => {
            let json_number: serde_json::Number = serde_json::from_str(&number.to_string())
                .map_err(|_| GenerateError::NonDataOutput {
                    service: service.to_string(),
                    kind: format!("unrepresentable number `{number}`"),
                })?;
            serde_json::Value::Number(json_number)
        }
        Value::String(text) => serde_json::Value::String(text),
        Value::Date(date) => serde_json::Value::String(date.to_string()),
        Value::Time(time) => serde_json::Value::String(time.to_string()),
        Value::DateTime(datetime) => serde_json::Value::String(datetime.to_string()),
        Value::DaysAndTimeDuration(duration) => serde_json::Value::String(duration.to_string()),
        Value::YearsAndMonthsDuration(duration) => serde_json::Value::String(duration.to_string()),
        Value::List(items) => serde_json::Value::Array(
            items
                .into_iter()
                .map(|item| value_to_json(service, item))
                .collect::<Result<Vec<_>, _>>()?,
        ),
        Value::Context(context) => {
            let mut map = serde_json::Map::new();
            for (name, value) in context.iter() {
                map.insert(name.to_string(), value_to_json(service, value.clone())?);
            }
            serde_json::Value::Object(map)
        }
        other => {
            return Err(GenerateError::NonDataOutput {
                service: service.to_string(),
                kind: other.type_of().to_string(),
            })
        }
    })
}
