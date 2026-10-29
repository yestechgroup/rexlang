//! The `.cmmn` → [cmmn-rs](https://github.com/yestechgroup/cmmn-rs) backend:
//! lowers a compiled [`CmmnModel`] artifact into the JSON `CaseDefinition`
//! the cmmn-rs engine (`cmmn-json` → `cmmn-core`) consumes — rexlang
//! authors and validates the case, cmmn-rs executes it.
//!
//! This backend consumes **only** the [`rex_ir::cmmn::CmmnModel`] artifact
//! plus the domain [`rex_ir::Model`] its case-file types resolve against
//! (the [`rex_backend_cedar`](https://docs.rs/rex-backend-cedar) pair
//! precedent: the artifact is the contract, the domain model supplies the
//! class features the case file's items carry as engine-visible
//! properties).
//!
//! # The seam
//!
//! * A case lowers to one engine `CaseDefinition` (pretty JSON): the case
//!   file items become `caseFileModel` entries (properties = the features
//!   of the domain class the item is typed by, attributes only), the plan
//!   lowers to the engine's flat `planModel` graph with a synthetic root
//!   stage (`<case>_plan`, auto-complete), roles lower keyed by name, and
//!   each criterion becomes its own engine sentry (`sentry_<item>_<n>`)
//!   referenced by a generated criterion (`crit_<item>_<n>`).
//! * A human task lowers to `taskType: "User"` with `requiredRole`; a
//!   service task lowers to `taskType: "Process"` whose `processRef` is
//!   the resolved `alias.Service.operation` — the key the engine's task
//!   executor registry resolves (the engine executes composition tasks
//!   through a registered [`TaskExecutor`](cmmn) seam at activation).
//! * A discretionary task is **excluded from its stage's children** (the
//!   engine instantiates only the containment tree at case start) and
//!   becomes a `discretionaryItem` plus a `planningTable` attached to the
//!   stage it was declared in (or the root stage for top-level items).
//! * `required;` lowers to `requiredRule {condition: null}`, `repeats;` to
//!   `repetitionRule {condition: null}`, and `auto;` to
//!   `manualActivationRule {condition: Feel("false")}` — an explicit FALSE
//!   rule is auto-start (the engine's absent-rule default is manual
//!   activation, per CMMN 1.1 Table 5.51).
//!
//! # Expressions: rex-expr authored, FEEL executed
//!
//! The engine evaluates sentry if-parts as FEEL against the case file
//! scope. A `when` condition is therefore **translated** from rex-expr to
//! FEEL at generate time, within a documented subset:
//!
//! | rex-expr | FEEL |
//! |---|---|
//! | `a && b`, `a \|\| b`, `!a` | `a and b`, `a or b`, `not (a)` |
//! | `=`, `!=`, `<`, `<=`, `>`, `>=` | identical (`==` becomes `=`) |
//! | `+`, `-`, `*`, `/` | identical (R1/R4 already enforced at compile) |
//! | `x.size()` | `count(x)` (spec A5) |
//! | `x.sum()` | `sum(x)` (spec A6) |
//! | `case_file.x`, bare `x` | `x` (the engine's scope keys are the case-file items) |
//! | `a.b.c` navigation | `a.b.c` |
//! | literals, `date("...")` | identical |
//! | `if c { t } else { e }` | `if c then t else e` |
//!
//! Everything else — `?.` safe navigation, `?:` coalescing, `let`, lambdas,
//! list literals, model operations, and the remaining collection algebra —
//! is a **named generate-time error** ([`GenerateError`]), the
//! Cedar-backend convention: the backend rejects what it cannot map,
//! naming the case and plan item.
//!
//! Two more rejections document today's engine gaps honestly: a **bare
//! event on-part** (`.evt` events — the engine's `OnPart` is plan-item and
//! case-file transitions only), and a **plan-item predicate inside a
//! `when`** (the engine evaluates if-parts against the case-file scope
//! alone; plan-item states are on-parts, not scope lookups).
//!
//! # Determinism
//!
//! The emitted JSON is byte-stable: key sections serialize in declaration
//! order (typed structs), the engine's keyed sections serialize sorted
//! (`BTreeMap`), and sentry/criterion ids derive from plan item names
//! (`sentry_<item>_entry_<n>` / `crit_<item>_entry_<n>`, `exit_<n>`
//! likewise). Regenerating the same artifact always yields the same bytes.

use std::collections::BTreeMap;

use serde::Serialize;

use rex_ir::cmmn::{
    CaseDef, CaseFileItem, CaseFileTransition, CmmnModel, Criterion, OnPart, PlanItem,
    PlanItemKind, PlanItemTransition, PlanningMode,
};
use rex_ir::{Model, PrimitiveType, TypeRef};

/// A generate-time failure: the artifact is valid rexlang but names
/// something the engine cannot express today (the Cedar-backend
/// convention — rejections name the case and plan item).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum GenerateError {
    /// A criterion's bare on-part names a domain event. The engine's
    /// `OnPart` is plan-item and case-file transitions only; event
    /// listeners are an engine milestone away.
    #[error("case '{case}': plan item '{item}': on-part '{event}' is a domain event; the case engine reacts to plan-item and case-file transitions only")]
    EventOnPart {
        /// The declaring case.
        case: String,
        /// The declaring plan item.
        item: String,
        /// The event name.
        event: String,
    },
    /// A `when` condition references a plan item. The engine evaluates
    /// if-parts against the case-file scope; plan-item states are on-parts.
    #[error("case '{case}': plan item '{item}': the when condition references plan item '{reference}'; only case-file items can appear in a when condition")]
    PlanItemPredicate {
        /// The declaring case.
        case: String,
        /// The declaring plan item.
        item: String,
        /// The referenced plan item.
        reference: String,
    },
    /// A `when` condition uses a construct outside the FEEL-translatable
    /// subset.
    #[error("case '{case}': plan item '{item}': {reason} in the when condition; only case-file navigation, comparisons, boolean logic, arithmetic, size()/sum(), literals, and if/else translate to FEEL")]
    Untranslatable {
        /// The declaring case.
        case: String,
        /// The declaring plan item.
        item: String,
        /// What cannot be translated.
        reason: String,
    },
    /// A milestone carries `auto;` — the manual activation rule is not
    /// applicable to milestones (CMMN 1.1 Table 5.55; the engine validates
    /// it as `ForbiddenRule`).
    #[error("case '{case}': milestone '{item}' cannot carry `auto;`; the manual activation rule is not applicable to milestones")]
    MilestoneAutoActivation {
        /// The declaring case.
        case: String,
        /// The milestone.
        item: String,
    },
    /// A case-file item's multiplicity is outside the engine's vocabulary.
    #[error("case '{case}': case-file item '{item}' has multiplicity [{lower}..{upper}]; the engine supports [0..1], [1..1], [0..*], and [1..*]")]
    UnsupportedMultiplicity {
        /// The declaring case.
        case: String,
        /// The case-file item.
        item: String,
        /// The lower bound.
        lower: u32,
        /// The upper bound (`*` for unbounded).
        upper: String,
    },
}

/// One generated case definition: the case's name and its engine-ready
/// JSON (`cmmn-json::deserialize_case_definition` consumes it).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaseDefinitionJson {
    /// The case's name.
    pub case: String,
    /// The pretty-printed `CaseDefinition` JSON.
    pub json: String,
}

/// Lowers every case of the artifact into engine-ready `CaseDefinition`
/// JSON, one entry per case, in declaration order.
pub fn generate(
    model: &CmmnModel,
    domains: &Model,
) -> Result<Vec<CaseDefinitionJson>, GenerateError> {
    model
        .cases
        .iter()
        .map(|case| {
            Ok(CaseDefinitionJson {
                case: case.name.clone(),
                json: serde_json::to_string_pretty(
                    &CaseDefinitionEmitter::new(case, domains).emit()?,
                )
                .map_err(|_| GenerateError::Untranslatable {
                    case: case.name.clone(),
                    item: String::new(),
                    reason: "the lowered definition does not serialize".to_string(),
                })?,
            })
        })
        .collect()
}

// --- engine-shaped emission types ---------------------------------------------

/// The engine `CaseDefinition` (snake_case, its wire shape).
#[derive(Serialize)]
#[serde(rename_all = "snake_case")]
struct EngineCase {
    id: String,
    name: String,
    case_file_model: EngineCaseFileModel,
    plan_model: EnginePlanModel,
    roles: BTreeMap<String, EngineRole>,
}

/// The engine `CaseFileModel`.
#[derive(Serialize)]
#[serde(rename_all = "snake_case")]
struct EngineCaseFileModel {
    items: BTreeMap<String, EngineCaseFileItem>,
    children: Vec<String>,
}

/// The engine `CaseFileItemDefinition`.
#[derive(Serialize)]
#[serde(rename_all = "snake_case")]
struct EngineCaseFileItem {
    id: String,
    name: String,
    multiplicity: &'static str,
    children: Vec<String>,
    properties: Vec<EngineProperty>,
}

/// The engine `PropertyDefinition`.
#[derive(Clone, Serialize)]
#[serde(rename_all = "snake_case")]
struct EngineProperty {
    name: String,
    type_name: String,
}

/// The engine `CasePlanModel` (the root stage).
#[derive(Serialize)]
#[serde(rename_all = "snake_case")]
struct EnginePlanModel {
    id: String,
    name: String,
    plan_items: BTreeMap<String, EnginePlanItem>,
    sentries: BTreeMap<String, EngineSentry>,
    planning_tables: BTreeMap<String, EnginePlanningTable>,
    discretionary_items: BTreeMap<String, EngineDiscretionaryItem>,
    applicability_rules: BTreeMap<String, EngineApplicabilityRule>,
    auto_complete: bool,
}

/// The engine `PlanItemDefinition`.
#[derive(Serialize)]
#[serde(rename_all = "snake_case")]
struct EnginePlanItem {
    id: String,
    name: String,
    kind: EngineKind,
    entry_criteria: Vec<EngineCriterion>,
    exit_criteria: Vec<EngineCriterion>,
    control: Option<EngineControl>,
}

/// The engine `PlanItemKind` (externally tagged, PascalCase variants).
#[derive(Serialize)]
enum EngineKind {
    /// A task; the payload is the engine `TaskDefinition`.
    Task(EngineTask),
    /// A stage; the payload is the engine `StageDefinition`.
    Stage(EngineStage),
    /// A milestone (the engine `MilestoneDefinition` is empty).
    Milestone {},
}

/// The engine `TaskDefinition`.
#[derive(Serialize)]
#[serde(rename_all = "snake_case")]
struct EngineTask {
    task_type: &'static str,
    required_role: Option<String>,
    planning_mode: &'static str,
    composition: Option<EngineComposition>,
}

/// The engine `TaskComposition` (externally tagged).
#[derive(Serialize)]
enum EngineComposition {
    /// The engine `ProcessTaskDefinition`.
    Process(EngineProcess),
}

/// The engine `ProcessTaskDefinition`.
#[derive(Serialize)]
#[serde(rename_all = "snake_case")]
struct EngineProcess {
    process_ref: String,
    is_blocking: bool,
    mappings: Vec<EngineMapping>,
}

/// The engine `ParameterMapping` (never emitted by v1: the execute binding
/// is the process reference; parameter mappings are an engine-side
/// integration concern).
#[derive(Serialize)]
#[serde(rename_all = "snake_case")]
struct EngineMapping {}

/// The engine `StageDefinition`.
#[derive(Serialize)]
#[serde(rename_all = "snake_case")]
struct EngineStage {
    children: Vec<String>,
    auto_complete: bool,
}

/// The engine `PlanItemControl`.
#[derive(Serialize)]
#[serde(rename_all = "snake_case")]
struct EngineControl {
    manual_activation_rule: Option<EngineRule>,
    required_rule: Option<EngineRule>,
    repetition_rule: Option<EngineRule>,
}

/// The engine `Rule` (a `null` condition is always TRUE).
#[derive(Serialize)]
#[serde(rename_all = "snake_case")]
struct EngineRule {
    condition: Option<EngineExpression>,
}

impl EngineRule {
    /// The always-TRUE rule (`condition: null`).
    fn no_condition() -> Self {
        Self { condition: None }
    }
}

/// The engine `Expression` (`{"language": "Feel", "source": "..."}`).
#[derive(Serialize)]
#[serde(rename_all = "snake_case")]
struct EngineExpression {
    language: &'static str,
    source: String,
}

/// The engine `Sentry`.
#[derive(Serialize)]
#[serde(rename_all = "snake_case")]
struct EngineSentry {
    id: String,
    on_parts: Vec<EngineOnPart>,
    if_part: Option<EngineExpression>,
}

/// The engine `OnPart` (externally tagged, PascalCase variants).
#[derive(Serialize)]
enum EngineOnPart {
    /// A plan item standard event.
    PlanItem {
        /// The watched plan item.
        source: String,
        /// The watched standard event (PascalCase).
        event: &'static str,
    },
    /// A case-file item transition.
    CaseFileItem {
        /// The watched case-file item.
        source: String,
        /// The watched transition (PascalCase).
        transition: &'static str,
    },
}

/// The engine `Criterion`.
#[derive(Serialize)]
#[serde(rename_all = "snake_case")]
struct EngineCriterion {
    id: String,
    sentry_ref: String,
}

/// The engine `Role`.
#[derive(Serialize)]
struct EngineRole {
    id: String,
    name: String,
}

/// The engine `PlanningTable`.
#[derive(Serialize)]
#[serde(rename_all = "snake_case")]
struct EnginePlanningTable {
    id: String,
    name: String,
    context: EnginePlanningContext,
    items: Vec<String>,
    applicability_rules: Vec<String>,
    authorized_roles: Vec<String>,
}

/// The engine `PlanningTableContext` (externally tagged).
#[derive(Serialize)]
enum EnginePlanningContext {
    /// The table of a stage (the root stage included).
    Stage(String),
}

/// The engine `DiscretionaryItem`.
#[derive(Serialize)]
#[serde(rename_all = "snake_case")]
struct EngineDiscretionaryItem {
    id: String,
    name: String,
    definition_ref: String,
    authorized_roles: Vec<String>,
    applicability_rules: Vec<String>,
}

/// The engine `ApplicabilityRule` (never emitted by v1).
#[derive(Serialize)]
struct EngineApplicabilityRule {}

// --- emission -------------------------------------------------------------------

/// The per-case emitter: walks the artifact once, building the engine
/// shape.
struct CaseDefinitionEmitter<'a> {
    case: &'a CaseDef,
    /// Feature lookups for case-file item properties: class qualified name
    /// → its attribute features.
    classes: BTreeMap<String, Vec<EngineProperty>>,
    plan_items: BTreeMap<String, EnginePlanItem>,
    sentries: BTreeMap<String, EngineSentry>,
    planning_tables: BTreeMap<String, EnginePlanningTable>,
    discretionary_items: BTreeMap<String, EngineDiscretionaryItem>,
}

impl<'a> CaseDefinitionEmitter<'a> {
    fn new(case: &'a CaseDef, domains: &'a Model) -> Self {
        let mut classes = BTreeMap::new();
        for package in &domains.packages {
            for class in &package.classes {
                let properties = class
                    .features
                    .iter()
                    .filter(|feature| matches!(feature.kind, rex_ir::FeatureKind::Attribute))
                    .map(|feature| EngineProperty {
                        name: feature.name.clone(),
                        type_name: type_name(&feature.type_),
                    })
                    .collect();
                classes.insert(format!("{}::{}", package.name, class.name), properties);
            }
        }
        Self {
            case,
            classes,
            plan_items: BTreeMap::new(),
            sentries: BTreeMap::new(),
            planning_tables: BTreeMap::new(),
            discretionary_items: BTreeMap::new(),
        }
    }

    fn emit(mut self) -> Result<EngineCase, GenerateError> {
        let case_name = &self.case.name;
        let root_id = format!("{case_name}_plan");

        // Case file: items keyed by name, all roots (v1 case files are flat).
        let mut items = BTreeMap::new();
        let mut children = Vec::new();
        for item in &self.case.case_file {
            children.push(item.name.clone());
            items.insert(item.name.clone(), self.case_file_item(item)?);
        }

        // Plan items, depth-first (the artifact's flat graph): discretionary
        // tasks leave their stage's children and become planning entries.
        for item in &self.case.plan_items {
            self.plan_item(case_name, &root_id, item)?;
        }

        // The root stage itself (the engine models the case plan as one).
        // Its children are the case's root plan items — the ones no stage
        // lists — minus discretionary tasks, which plan at runtime.
        let mut staged: Vec<&str> = Vec::new();
        for item in &self.case.plan_items {
            if let PlanItemKind::Stage(stage) = &item.kind {
                staged.extend(stage.children.iter().map(String::as_str));
            }
        }
        let root_children: Vec<String> = self
            .case
            .plan_items
            .iter()
            .filter(|item| !item.discretionary_planning() && !staged.contains(&item.name.as_str()))
            .map(|item| item.name.clone())
            .collect();
        self.plan_items.insert(
            root_id.clone(),
            EnginePlanItem {
                id: root_id.clone(),
                name: case_name.clone(),
                kind: EngineKind::Stage(EngineStage {
                    children: root_children,
                    auto_complete: true,
                }),
                entry_criteria: Vec::new(),
                exit_criteria: Vec::new(),
                control: None,
            },
        );

        let roles = self
            .case
            .roles
            .iter()
            .map(|role| {
                (
                    role.name.clone(),
                    EngineRole {
                        id: role.name.clone(),
                        name: role.name.clone(),
                    },
                )
            })
            .collect();

        Ok(EngineCase {
            id: case_name.clone(),
            name: case_name.clone(),
            case_file_model: EngineCaseFileModel { items, children },
            plan_model: EnginePlanModel {
                id: root_id,
                name: case_name.clone(),
                plan_items: self.plan_items,
                sentries: self.sentries,
                planning_tables: self.planning_tables,
                discretionary_items: self.discretionary_items,
                applicability_rules: BTreeMap::new(),
                auto_complete: true,
            },
            roles,
        })
    }

    /// One case-file item: multiplicity mapped to the engine's vocabulary,
    /// properties = the domain class's attribute features (unknown class →
    /// none — the compile-time check is the authority).
    fn case_file_item(&self, item: &CaseFileItem) -> Result<EngineCaseFileItem, GenerateError> {
        let multiplicity = match (item.multiplicity.lower, &item.multiplicity.upper) {
            (0, rex_ir::Upper::Finite(1)) => "ZeroOrOne",
            (1, rex_ir::Upper::Finite(1)) => "One",
            (0, rex_ir::Upper::Unbounded) => "ZeroOrMore",
            (1, rex_ir::Upper::Unbounded) => "OneOrMore",
            (lower, upper) => {
                return Err(GenerateError::UnsupportedMultiplicity {
                    case: self.case.name.clone(),
                    item: item.name.clone(),
                    lower,
                    upper: match upper {
                        rex_ir::Upper::Finite(n) => n.to_string(),
                        rex_ir::Upper::Unbounded => "*".to_string(),
                    },
                })
            }
        };
        let properties = match item.ty.qualified_name() {
            Some(qualified) => self.classes.get(&qualified).cloned().unwrap_or_default(),
            None => Vec::new(),
        };
        Ok(EngineCaseFileItem {
            id: item.name.clone(),
            name: item.name.clone(),
            multiplicity,
            children: Vec::new(),
            properties,
        })
    }

    /// One plan item: the kind mapping, its criteria as generated sentries,
    /// and — for discretionary tasks — the planning entries.
    fn plan_item(
        &mut self,
        case: &str,
        root_id: &str,
        item: &PlanItem,
    ) -> Result<(), GenerateError> {
        let kind = match &item.kind {
            PlanItemKind::Task(task) => {
                let discretionary = item.planning == PlanningMode::Manual;
                EngineKind::Task(EngineTask {
                    task_type: "User",
                    required_role: task.performer.clone(),
                    planning_mode: if discretionary { "Automatic" } else { "None" },
                    composition: None,
                })
            }
            PlanItemKind::Service(service) => EngineKind::Task(EngineTask {
                task_type: "Process",
                required_role: None,
                planning_mode: "None",
                composition: Some(EngineComposition::Process(EngineProcess {
                    process_ref: service.execute.clone(),
                    is_blocking: true,
                    mappings: Vec::new(),
                })),
            }),
            PlanItemKind::Milestone => EngineKind::Milestone {},
            PlanItemKind::Stage(stage) => EngineKind::Stage(EngineStage {
                // Discretionary children plan at runtime; they are not part
                // of the containment tree.
                children: stage
                    .children
                    .iter()
                    .filter(|name| {
                        !self.case.plan_items.iter().any(|candidate| {
                            candidate.name == **name && candidate.planning == PlanningMode::Manual
                        })
                    })
                    .cloned()
                    .collect(),
                auto_complete: false,
            }),
        };

        // Control rules. `auto;` is an explicit FALSE manual activation
        // rule (auto-start); it is not applicable to milestones.
        // `auto;` on a milestone is rejected outright: the manual
        // activation rule is not applicable to milestones (CMMN 1.1 Table
        // 5.55; the engine validates it as `ForbiddenRule`).
        if matches!(item.kind, PlanItemKind::Milestone)
            && item
                .control
                .as_ref()
                .is_some_and(|control| control.manual_activation.is_some())
        {
            return Err(GenerateError::MilestoneAutoActivation {
                case: case.to_string(),
                item: item.name.clone(),
            });
        }
        let control = item.control.as_ref().map(|control| {
            let manual_activation_rule = control.manual_activation.map(|auto_start| EngineRule {
                condition: Some(EngineExpression {
                    language: "Feel",
                    source: if auto_start { "false" } else { "true" }.to_string(),
                }),
            });
            let required_rule = control.required.then(EngineRule::no_condition);
            let repetition_rule = control.repetition.then(EngineRule::no_condition);
            EngineControl {
                manual_activation_rule,
                required_rule,
                repetition_rule,
            }
        });

        let mut entry_criteria = Vec::new();
        for (index, criterion) in item.entry.iter().enumerate() {
            entry_criteria.push(self.criterion(case, &item.name, "entry", index, criterion)?);
        }
        let mut exit_criteria = Vec::new();
        for (index, criterion) in item.exit.iter().enumerate() {
            exit_criteria.push(self.criterion(case, &item.name, "exit", index, criterion)?);
        }

        // Discretionary tasks join a planning table attached to their
        // context stage (the root stage for top-level items).
        if item.planning == PlanningMode::Manual {
            let context = self
                .case
                .plan_items
                .iter()
                .find(|candidate| {
                    matches!(&candidate.kind, PlanItemKind::Stage(stage) if stage.children.contains(&item.name))
                })
                .map(|stage| stage.name.clone())
                .unwrap_or_else(|| root_id.to_string());
            let disc_id = format!("disc_{}", item.name);
            let table_id = format!("planning_{context}");
            self.discretionary_items.insert(
                disc_id.clone(),
                EngineDiscretionaryItem {
                    id: disc_id.clone(),
                    name: item.name.clone(),
                    definition_ref: item.name.clone(),
                    authorized_roles: Vec::new(),
                    applicability_rules: Vec::new(),
                },
            );
            self.planning_tables
                .entry(table_id.clone())
                .or_insert_with(|| EnginePlanningTable {
                    id: table_id.clone(),
                    name: format!("{context} planning"),
                    context: EnginePlanningContext::Stage(context),
                    items: Vec::new(),
                    applicability_rules: Vec::new(),
                    authorized_roles: Vec::new(),
                })
                .items
                .push(disc_id);
        }

        self.plan_items.insert(
            item.name.clone(),
            EnginePlanItem {
                id: item.name.clone(),
                name: item.name.clone(),
                kind,
                entry_criteria,
                exit_criteria,
                control,
            },
        );
        Ok(())
    }

    /// One criterion: a generated sentry (id `sentry_<item>_<side>_<n>`)
    /// plus the criterion referencing it.
    fn criterion(
        &mut self,
        case: &str,
        item: &str,
        side: &str,
        index: usize,
        criterion: &Criterion,
    ) -> Result<EngineCriterion, GenerateError> {
        let mut on_parts = Vec::new();
        for part in &criterion.on_parts {
            on_parts.push(match part {
                OnPart::Item {
                    item: source,
                    event,
                } => EngineOnPart::PlanItem {
                    source: source.clone(),
                    event: plan_item_transition(*event),
                },
                OnPart::CaseFile {
                    item: source,
                    transition,
                } => EngineOnPart::CaseFileItem {
                    source: source.clone(),
                    transition: case_file_transition(*transition),
                },
                OnPart::Event(event) => {
                    return Err(GenerateError::EventOnPart {
                        case: case.to_string(),
                        item: item.to_string(),
                        event: event.clone(),
                    })
                }
            });
        }
        let if_part = match &criterion.when {
            Some(text) => Some(EngineExpression {
                language: "Feel",
                source: self.translate_when(case, item, text)?,
            }),
            None => None,
        };
        let sentry_id = format!("sentry_{item}_{side}_{index}");
        self.sentries.insert(
            sentry_id.clone(),
            EngineSentry {
                id: sentry_id.clone(),
                on_parts,
                if_part,
            },
        );
        Ok(EngineCriterion {
            id: format!("crit_{item}_{side}_{index}"),
            sentry_ref: sentry_id,
        })
    }

    /// Translates one `when` condition's rex-expr text into FEEL, within
    /// the documented subset.
    fn translate_when(&self, case: &str, item: &str, text: &str) -> Result<String, GenerateError> {
        let Some(ast) = rex_expr_parse(text) else {
            return Err(GenerateError::Untranslatable {
                case: case.to_string(),
                item: item.to_string(),
                reason: format!("`{text}` does not parse"),
            });
        };
        let plan_items: Vec<String> = self
            .case
            .plan_items
            .iter()
            .map(|candidate| candidate.name.clone())
            .collect();
        let mut out = String::new();
        translate(&ast, case, item, &plan_items, &mut out).map(|_| out)
    }
}

/// `true` when the plan item plans at runtime (the discretionary flag).
trait DiscretionaryPlanning {
    fn discretionary_planning(&self) -> bool;
}

impl DiscretionaryPlanning for PlanItem {
    fn discretionary_planning(&self) -> bool {
        self.planning == PlanningMode::Manual
    }
}

/// Maps a rexlang plan-item transition onto the engine's PascalCase
/// spelling. `fail` is the engine's `Fault` (Table 5.31).
fn plan_item_transition(transition: PlanItemTransition) -> &'static str {
    match transition {
        PlanItemTransition::Create => "Create",
        PlanItemTransition::Enable => "Enable",
        PlanItemTransition::Start => "Start",
        PlanItemTransition::Complete => "Complete",
        PlanItemTransition::Terminate => "Terminate",
        PlanItemTransition::Fail => "Fault",
        PlanItemTransition::Occur => "Occur",
    }
}

/// Maps a rexlang case-file transition onto the engine's PascalCase
/// spelling.
fn case_file_transition(transition: CaseFileTransition) -> &'static str {
    match transition {
        CaseFileTransition::Create => "Create",
        CaseFileTransition::Update => "Update",
        CaseFileTransition::Delete => "Delete",
        CaseFileTransition::AddChild => "AddChild",
        CaseFileTransition::RemoveChild => "RemoveChild",
    }
}

/// The engine-visible property type name of a domain type: FEEL builtin
/// names for primitives, the qualified name for named types.
fn type_name(ty: &TypeRef) -> String {
    match ty {
        TypeRef::Primitive(primitive) => match primitive {
            PrimitiveType::String => "string".to_string(),
            PrimitiveType::Boolean => "boolean".to_string(),
            PrimitiveType::Int
            | PrimitiveType::Long
            | PrimitiveType::Short
            | PrimitiveType::Float
            | PrimitiveType::Double
            | PrimitiveType::Byte
            | PrimitiveType::Char => "number".to_string(),
            PrimitiveType::Date => "date".to_string(),
        },
        other => other.qualified_name().unwrap_or_default(),
    }
}

use rex_expr::ast::{AlgebraKind, BinOp, ExprKind, UnOp};
use rex_expr::Expr;

/// Parses the when text with rex-expr; `None` when the text does not parse
/// (a compile-time guarantee — the backend reports it as untranslatable).
fn rex_expr_parse(text: &str) -> Option<Expr> {
    rex_expr::parse(text).ast
}

/// Renders one rex-expr AST node into FEEL text.
fn translate(
    ast: &Expr,
    case: &str,
    item: &str,
    plan_items: &[String],
    out: &mut String,
) -> Result<(), GenerateError> {
    let fail = |reason: String| GenerateError::Untranslatable {
        case: case.to_string(),
        item: item.to_string(),
        reason,
    };
    match &ast.kind {
        ExprKind::Int(value) => out.push_str(&value.to_string()),
        ExprKind::Float(value) => out.push_str(&format!("{value}")),
        ExprKind::String(text) => {
            out.push('"');
            out.push_str(&text.replace('\\', "\\\\").replace('"', "\\\""));
            out.push('"');
        }
        ExprKind::Bool(value) => out.push_str(if *value { "true" } else { "false" }),
        ExprKind::Null => out.push_str("null"),
        ExprKind::Date { text, .. } => {
            out.push_str("date(\"");
            out.push_str(text);
            out.push_str("\")");
        }
        ExprKind::Name(name) => {
            if plan_items.iter().any(|known| known == name) {
                return Err(GenerateError::PlanItemPredicate {
                    case: case.to_string(),
                    item: item.to_string(),
                    reference: name.clone(),
                });
            }
            out.push_str(name);
        }
        ExprKind::FeatureAccess {
            receiver,
            name,
            optional_safe,
        } => {
            if *optional_safe {
                return Err(fail("`?.` safe navigation".to_string()));
            }
            // `case_file.x` and bare `x` both denote the case-file item; the
            // engine's scope keys are the item names.
            if let ExprKind::Name(root) = &receiver.kind {
                if root == "case_file" {
                    out.push_str(&name.value);
                    return Ok(());
                }
                if plan_items.iter().any(|known| known == root) {
                    return Err(GenerateError::PlanItemPredicate {
                        case: case.to_string(),
                        item: item.to_string(),
                        reference: root.clone(),
                    });
                }
            }
            translate(receiver, case, item, plan_items, out)?;
            out.push('.');
            out.push_str(&name.value);
        }
        ExprKind::Coalesce { .. } => {
            return Err(fail("`?:` coalescing".to_string()));
        }
        ExprKind::Call { .. } => {
            return Err(fail("an operation call".to_string()));
        }
        ExprKind::Algebra {
            receiver,
            kind,
            lambda,
            ..
        } => {
            if lambda.is_some() && !matches!(kind, AlgebraKind::Any) {
                return Err(fail(format!("`{kind}(...)` with a lambda")));
            }
            let mut inner = String::new();
            translate(receiver, case, item, plan_items, &mut inner)?;
            match kind {
                AlgebraKind::Size => {
                    out.push_str("count(");
                    out.push_str(&inner);
                    out.push(')');
                }
                AlgebraKind::Sum => {
                    out.push_str("sum(");
                    out.push_str(&inner);
                    out.push(')');
                }
                _ => return Err(fail(format!("`{kind}(...)`"))),
            }
        }
        ExprKind::Binary { op, lhs, rhs } => {
            let mut left = String::new();
            translate(lhs, case, item, plan_items, &mut left)?;
            let mut right = String::new();
            translate(rhs, case, item, plan_items, &mut right)?;
            let symbol = match op {
                BinOp::And => "and",
                BinOp::Or => "or",
                BinOp::Eq => "=",
                BinOp::Ne => "!=",
                BinOp::Lt => "<",
                BinOp::Le => "<=",
                BinOp::Gt => ">",
                BinOp::Ge => ">=",
                BinOp::Add => "+",
                BinOp::Sub => "-",
                BinOp::Mul => "*",
                BinOp::Div => "/",
            };
            out.push_str(&left);
            out.push(' ');
            out.push_str(symbol);
            out.push(' ');
            out.push_str(&right);
        }
        ExprKind::Unary { op, expr } => {
            let mut inner = String::new();
            translate(expr, case, item, plan_items, &mut inner)?;
            match op {
                UnOp::Not => {
                    out.push_str("not (");
                    out.push_str(&inner);
                    out.push(')');
                }
                UnOp::Neg => {
                    out.push('-');
                    out.push_str(&inner);
                }
            }
        }
        ExprKind::If { cond, then, else_ } => {
            let mut condition = String::new();
            translate(cond, case, item, plan_items, &mut condition)?;
            let mut then_branch = String::new();
            translate(then, case, item, plan_items, &mut then_branch)?;
            let mut else_branch = String::new();
            translate(else_, case, item, plan_items, &mut else_branch)?;
            out.push_str("if ");
            out.push_str(&condition);
            out.push_str(" then ");
            out.push_str(&then_branch);
            out.push_str(" else ");
            out.push_str(&else_branch);
        }
        ExprKind::Lambda { .. } | ExprKind::ListLiteral(_) | ExprKind::Let { .. } => {
            return Err(fail("a lambda, list literal, or let binding".to_string()));
        }
    }
    Ok(())
}
