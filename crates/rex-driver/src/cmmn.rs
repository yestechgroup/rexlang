//! Semantic lowering and validation for `.cmmn` case-management sources:
//! the `.cmmn` counterpart of [`crate::usecases`] and [`crate::events`].
//!
//! This module is salsa-free; the driver's tracked queries call into it via
//! [`compile_cmmn_file`]. The normative contract — entry points, resolution
//! semantics, and the validation rules — is documented on
//! [`compile_cmmn_str`](crate::compile_cmmn_str).
//!
//! The engine mirrors [`crate::usecases::compile_usecase_file`]: parse
//! diagnostics and domain diagnostics propagate under their own file paths,
//! the `.cmmn` file's own diagnostics are tagged with its path, and any
//! error-severity diagnostic anywhere drops the artifact. Case-file item
//! types resolve through the shared domain namespace helper
//! ([`crate::lower::resolve_ddd_type`]) — never re-implemented here — while
//! role, plan-item, design, and event lookups run against the pre-compiled
//! artifacts the caller provides (the `.deploy` pattern), and `when`
//! conditions type-check against a synthetic case universe (the
//! [`crate::deploy`] policy pattern): the case file as a root class, plan
//! items as classes of boolean transition predicates.

use rex_expr::Ty;
use rex_ir::cmmn::{
    CaseDef, CaseFileItem, CaseFileTransition, CmmnModel, ControlRules, Criterion, OnPart,
    PlanItem, PlanItemKind, PlanItemTransition, PlanningMode, Role, ServiceDef, StageBody, TaskDef,
};
use rex_ir::ddd::DddModel;
use rex_ir::events::EventModel;
use rex_ir::{ActorModel, Multiplicity, TypeRef};
use rex_syntax::ast as dsl;
use rex_syntax::Span;

use crate::diagnostic::Diagnostic;
use crate::import_matches;
use crate::lower::{self, DomainUnit};

/// The provided-artifact bucket an import hits when it names a `.ddd`
/// design (the designs entry of the four-bucket hit array).
const BUCKET_DESIGNS: usize = 1;

/// The plan-item standard events an [`OnPart::Item`] may watch, in
/// vocabulary order.
const PLAN_ITEM_TRANSITIONS: [(&str, PlanItemTransition); 7] = [
    ("create", PlanItemTransition::Create),
    ("enable", PlanItemTransition::Enable),
    ("start", PlanItemTransition::Start),
    ("complete", PlanItemTransition::Complete),
    ("terminate", PlanItemTransition::Terminate),
    ("fail", PlanItemTransition::Fail),
    ("occur", PlanItemTransition::Occur),
];

/// The case-file transitions an [`OnPart::CaseFile`] may watch, in
/// vocabulary order.
const CASE_FILE_TRANSITIONS: [(&str, CaseFileTransition); 4] = [
    ("create", CaseFileTransition::Create),
    ("update", CaseFileTransition::Update),
    ("delete", CaseFileTransition::Delete),
    ("childUpdate", CaseFileTransition::ChildUpdate),
];

/// The name universes of one case: plan items and case-file items share one
/// namespace (rule 5) so every criterion on-part resolves unambiguously;
/// roles are separate.
struct CaseNames {
    /// Case-file item names, in declaration order.
    file_items: Vec<String>,
    /// Plan item names (the whole containment tree, stages included), in
    /// depth-first declaration order.
    plan_items: Vec<String>,
}

impl CaseNames {
    /// `true` when the name is declared, as either kind.
    fn known(&self, name: &str) -> bool {
        self.file_items.iter().any(|item| item == name)
            || self.plan_items.iter().any(|item| item == name)
    }

    /// The kind the name was declared as — `true` for a case-file item.
    fn is_file_item(&self, name: &str) -> bool {
        self.file_items.iter().any(|item| item == name)
    }
}

/// Lowers and validates one parsed `.cmmn` file against its imported
/// domains and the pre-compiled design, actor, and event artifacts its
/// imports resolve to. Returns the case artifact (or `None` when any
/// error-severity diagnostic was produced anywhere) plus the diagnostics
/// from every file, each tagged with its path.
///
/// See [`compile_cmmn_str`](crate::compile_cmmn_str) for the normative
/// contract.
#[allow(clippy::too_many_arguments)]
pub(crate) fn compile_cmmn_file(
    path: &str,
    source: &str,
    ast: Option<&dsl::CmmnFile>,
    parse_diagnostics: &[Diagnostic],
    domains: &[DomainUnit],
    designs: &[(String, DddModel)],
    actors: &[(String, ActorModel)],
    events: &[(String, EventModel)],
) -> (Option<CmmnModel>, Vec<(String, Diagnostic)>) {
    let mut diags: Vec<(String, Diagnostic)> = parse_diagnostics
        .iter()
        .map(|diagnostic| (path.to_string(), diagnostic.clone()))
        .collect();

    let imports = ast.map(|file| file.imports.as_slice()).unwrap_or_default();

    // Which provided bucket each import resolves against (exact path or
    // lexical resolution, the `.deploy` rule): domains, designs, actors,
    // events.
    let hits: Vec<[bool; 4]> = imports
        .iter()
        .map(|import| {
            [
                domains
                    .iter()
                    .any(|unit| import_matches(&import.path, path, &unit.path)),
                designs
                    .iter()
                    .any(|(provided, _)| import_matches(&import.path, path, provided)),
                actors
                    .iter()
                    .any(|(provided, _)| import_matches(&import.path, path, provided)),
                events
                    .iter()
                    .any(|(provided, _)| import_matches(&import.path, path, provided)),
            ]
        })
        .collect();

    for (import, hit) in imports.iter().zip(&hits) {
        // Rule 1 — every import must name a provided artifact of any
        // surface.
        if !hit.iter().any(|hit| *hit) {
            diags.push((
                path.to_string(),
                Diagnostic::error(
                    format!("imported file \"{}\" was not provided", import.path),
                    Some(import.span),
                ),
            ));
            continue;
        }
        // Rule 1 — aliases exist for `execute`, which names designs; every
        // other surface rejects them.
        if import.alias.is_some() && !hit[BUCKET_DESIGNS] {
            diags.push((
                path.to_string(),
                Diagnostic::error(
                    format!(
                        "import \"{}\" is not a design; only design imports take an alias",
                        import.path
                    ),
                    Some(import.span),
                )
                .with_help("alias the `.ddd` design an `execute` binding names"),
            ));
        }
    }

    // Domain diagnostics propagate under their own file, in import order.
    for unit in domains {
        for diagnostic in &unit.diagnostics {
            diags.push((unit.path.clone(), diagnostic.clone()));
        }
    }

    let Some(ast) = ast else {
        return (None, diags);
    };

    // The `.cmmn` file's own diagnostics collect here (tagged with `path`
    // at the end), per case in source order; within a case: uniqueness
    // first, then roles, case-file items, and plan items (kind checks,
    // execute resolution, criteria, conditions) — each in declaration
    // order.
    let mut local: Vec<Diagnostic> = Vec::new();
    let packages = lower::domain_namespaces(domains);

    // Design imports by alias (rule 7): the first aliased design match per
    // alias wins.
    let mut designs_by_alias: Vec<(&str, &DddModel)> = Vec::new();
    for (import, hit) in imports.iter().zip(&hits) {
        let Some(alias) = &import.alias else {
            continue;
        };
        if !hit[BUCKET_DESIGNS] || designs_by_alias.iter().any(|(seen, _)| seen == alias) {
            continue;
        }
        if let Some((_, design)) = designs
            .iter()
            .find(|(provided, _)| import_matches(&import.path, path, provided))
        {
            designs_by_alias.push((alias.as_str(), design));
        }
    }

    let mut model = CmmnModel::new();
    let mut case_names: Vec<(String, Span)> = Vec::new();

    for decl in &ast.cases {
        // Rule 2 — case names are unique within the file.
        if case_names.iter().any(|(name, _)| *name == decl.name) {
            local.push(
                Diagnostic::error(format!("duplicate case '{}'", decl.name), Some(decl.span))
                    .with_help(format!("'{}' is already declared in this file", decl.name)),
            );
        } else {
            case_names.push((decl.name.clone(), decl.span));
        }

        // Rule 3 — roles: unique names, `.actor` bindings resolve.
        let mut roles: Vec<Role> = Vec::new();
        for role in &decl.roles {
            if roles.iter().any(|seen| seen.name == role.name) {
                local.push(Diagnostic::error(
                    format!("duplicate role '{}' in case '{}'", role.name, decl.name),
                    Some(role.span),
                ));
                continue;
            }
            let mut lowered = Role {
                name: role.name.clone(),
                actor: None,
            };
            if let Some(actor) = &role.actor {
                lowered.actor = Some(actor.clone());
                if !actor_known(actor, actors) {
                    local.push(
                        Diagnostic::error(
                            format!(
                                "unknown actor '{}' on role '{}' in case '{}'",
                                actor, role.name, decl.name
                            ),
                            Some(role.span),
                        )
                        .with_help(if actors.is_empty() {
                            "no `.actor` models were provided"
                        } else {
                            "role bindings must name an actor declared in an imported `.actor` file"
                        }),
                    );
                }
            }
            roles.push(lowered);
        }

        // The shared namespace (rule 5), collected before anything
        // resolves so criteria and conditions see the whole case.
        let names = collect_names(decl);

        // Rule 4 — case-file items: shared-namespace uniqueness, then type
        // resolution in the domain union.
        let mut case_file: Vec<CaseFileItem> = Vec::new();
        for item in &decl.case_file {
            if names.plan_items.contains(&item.name) || case_file.iter().any(|seen| seen.name == item.name)
            {
                local.push(Diagnostic::error(
                    format!("duplicate name '{}' in case '{}'", item.name, decl.name),
                    Some(item.span),
                ));
            }
            let ty = lower::resolve_ddd_type(&item.type_ref, &packages, None, &mut local);
            case_file.push(CaseFileItem {
                name: item.name.clone(),
                description: item.description.clone(),
                ty,
                multiplicity: ir_multiplicity(item.multiplicity.as_ref()),
            });
        }

        // Rule 10 — the synthetic case universe for `when` conditions (the
        // `.deploy` pattern): the case file as a class whose features are
        // the case-file items (typed by the imported domains), each plan
        // item as a class of boolean transition predicates, and the case
        // itself as the root class whose features reach both — so
        // `case_file.financials.size() > 0` and `VerifyBorrower.completed`
        // are plain feature navigation from `self`. No fake `Model` is
        // assembled.
        let mut types = rex_expr::DomainTypes::default();
        for unit in domains {
            if let Some(domains_model) = &unit.model {
                types.merge(rex_expr::DomainTypes::from_model(domains_model));
            }
        }
        let case_file_features: Vec<rex_ir::Feature> = case_file
            .iter()
            .map(|item| {
                rex_ir::Feature::new(
                    item.name.clone(),
                    rex_ir::FeatureKind::Attribute,
                    item.ty.clone(),
                    item.multiplicity,
                )
            })
            .collect();
        types.insert_class(
            "",
            "case_file",
            Vec::new(),
            case_file_features.clone(),
            Vec::new(),
        );
        for name in &names.plan_items {
            let predicates = PLAN_ITEM_TRANSITIONS
                .iter()
                .map(|(word, _)| {
                    rex_ir::Feature::new(
                        (*word).to_string(),
                        rex_ir::FeatureKind::Attribute,
                        TypeRef::Primitive(rex_ir::PrimitiveType::Boolean),
                        Multiplicity::REQUIRED,
                    )
                })
                .collect();
            types.insert_class("", name, Vec::new(), predicates, Vec::new());
        }
        {
            let mut root_features: Vec<rex_ir::Feature> = case_file_features;
            root_features.push(rex_ir::Feature::new(
                "case_file".to_string(),
                rex_ir::FeatureKind::Attribute,
                TypeRef::Class {
                    package: String::new(),
                    name: "case_file".to_string(),
                },
                Multiplicity::REQUIRED,
            ));
            for name in &names.plan_items {
                root_features.push(rex_ir::Feature::new(
                    name.clone(),
                    rex_ir::FeatureKind::Attribute,
                    TypeRef::Class {
                        package: String::new(),
                        name: name.clone(),
                    },
                    Multiplicity::REQUIRED,
                ));
            }
            types.insert_class("", &decl.name, Vec::new(), root_features, Vec::new());
        }

        // Plan items, depth-first so children follow their parent stage in
        // the flat artifact graph. `seen` carries the plan-item names
        // already lowered, for the shared-namespace rule.
        let mut plan_items: Vec<PlanItem> = Vec::new();
        let mut seen: Vec<String> = Vec::new();
        for item in &decl.plan_items {
            lower_plan_item(
                item,
                &decl.name,
                source,
                &roles,
                &names,
                &mut seen,
                &designs_by_alias,
                events,
                &types,
                &mut plan_items,
                &mut local,
            );
        }

        model.cases.push(CaseDef {
            name: decl.name.clone(),
            description: decl.description.clone(),
            roles,
            case_file,
            plan_items,
        });
    }

    diags.extend(local.into_iter().map(|d| (path.to_string(), d)));

    let blocked = diags.iter().any(|(_, diagnostic)| diagnostic.is_error());
    ((!blocked).then_some(model), diags)
}

/// Collects the shared namespace of one case (rule 5): the case-file items
/// and the whole plan-item containment tree, depth-first.
fn collect_names(decl: &dsl::CmmnCase) -> CaseNames {
    fn walk(items: &[dsl::CmmnItemDecl], plan_items: &mut Vec<String>) {
        for item in items {
            plan_items.push(item.name.clone());
            if let dsl::CmmnItemKind::Stage { items } = &item.kind {
                walk(items, plan_items);
            }
        }
    }
    let mut plan_items = Vec::new();
    walk(&decl.plan_items, &mut plan_items);
    CaseNames {
        file_items: decl.case_file.iter().map(|item| item.name.clone()).collect(),
        plan_items,
    }
}

/// Lowers one plan item (and, for stages, its children) into the artifact,
/// running the kind checks (rule 6), the execute resolution (rule 7), and
/// the criterion resolution + `when` typing (rules 9 and 10).
#[allow(clippy::too_many_arguments)]
fn lower_plan_item(
    item: &dsl::CmmnItemDecl,
    case: &str,
    source: &str,
    roles: &[Role],
    names: &CaseNames,
    seen: &mut Vec<String>,
    designs_by_alias: &[(&str, &DddModel)],
    events: &[(String, EventModel)],
    types: &rex_expr::DomainTypes,
    out: &mut Vec<PlanItem>,
    local: &mut Vec<Diagnostic>,
) {
    // Rule 5 — plan-item names join the shared namespace exactly once.
    if seen.iter().any(|name| name == &item.name) {
        local.push(Diagnostic::error(
            format!("duplicate name '{}' in case '{}'", item.name, case),
            Some(item.span),
        ));
    } else {
        seen.push(item.name.clone());
    }

    // Rule 8 — discretionary planning is a task/service concept.
    if item.discretionary && matches!(item.kind, dsl::CmmnItemKind::Milestone) {
        local.push(Diagnostic::error(
            format!(
                "plan item '{}' in case '{}' cannot be discretionary",
                item.name, case
            ),
            Some(item.span),
        ));
    }

    // Body clauses, validated per kind (rule 6: only human tasks carry a
    // performer; rule 7: only service tasks execute an operation). The
    // clauses parse wherever they were written; placement is validated
    // here so a stray clause is a diagnosed error, never a silent drop.
    let performer = item.performer.clone();
    let execute = item.execute.clone();
    if let Some((role, span)) = &performer {
        if !matches!(item.kind, dsl::CmmnItemKind::Task) {
            local.push(Diagnostic::error(
                format!(
                    "plan item '{}' in case '{}' cannot have a performer",
                    item.name, case
                ),
                Some(*span),
            ));
        } else if !roles.iter().any(|known| &known.name == role) {
            local.push(
                Diagnostic::error(
                    format!(
                        "unknown role '{}' on task '{}' in case '{}'",
                        role, item.name, case
                    ),
                    Some(*span),
                )
                .with_help("declare the role in the case's `roles` block"),
            );
        }
    }
    if execute.is_some() && !matches!(item.kind, dsl::CmmnItemKind::Service) {
        local.push(Diagnostic::error(
            format!(
                "plan item '{}' in case '{}' cannot execute an operation",
                item.name, case
            ),
            Some(item.span),
        ));
    }

    // Rule 7 — the execute binding resolves through the aliased design
    // (the use-case `realizes` rule), and every service must carry one.
    let mut lowered_execute: Option<String> = None;
    if matches!(item.kind, dsl::CmmnItemKind::Service) {
        if let Some(clause) = &execute {
            let reference = format!("{}.{}.{}", clause.alias, clause.service, clause.operation);
            match designs_by_alias
                .iter()
                .find(|(alias, _)| *alias == clause.alias)
            {
                None => {
                    local.push(
                        Diagnostic::error(
                            format!("unknown design alias '{}' in case '{}'", clause.alias, case),
                            Some(clause.span),
                        )
                        .with_help(format!(
                            "add `import \"<design>.ddd\" as {}` to the imports",
                            clause.alias
                        )),
                    );
                }
                Some((_, design)) => {
                    let services: Vec<&rex_ir::ddd::Service> = design
                        .modules
                        .iter()
                        .flat_map(|module| &module.services)
                        .filter(|service| service.name == clause.service)
                        .collect();
                    match services.len() {
                        0 => {
                            local.push(Diagnostic::error(
                                format!(
                                    "unknown service '{}' in design '{}' (case '{}')",
                                    clause.service, clause.alias, case
                                ),
                                Some(clause.span),
                            ));
                        }
                        1 => {
                            if !services[0]
                                .operations
                                .iter()
                                .any(|operation| operation.name == clause.operation)
                            {
                                local.push(Diagnostic::error(
                                    format!(
                                        "unknown operation '{}' on service '{}' in design '{}' (case '{}')",
                                        clause.operation, clause.service, clause.alias, case
                                    ),
                                    Some(clause.span),
                                ));
                            }
                        }
                        _ => {
                            local.push(Diagnostic::error(
                                format!(
                                    "ambiguous service '{}' in design '{}' (case '{}')",
                                    clause.service, clause.alias, case
                                ),
                                Some(clause.span),
                            ));
                        }
                    }
                }
            }
            lowered_execute = Some(reference);
        } else {
            local.push(Diagnostic::error(
                format!(
                    "service '{}' in case '{}' does not declare an execute binding",
                    item.name, case
                ),
                Some(item.span),
            ));
            lowered_execute = Some(String::new());
        }
    }

    // Rule 9 — criteria: milestones carry no exit criteria, every on-part
    // resolves, and `when` conditions type-check boolean against the
    // synthetic case universe.
    let mut entry: Vec<Criterion> = Vec::new();
    let mut exit: Vec<Criterion> = Vec::new();
    if matches!(item.kind, dsl::CmmnItemKind::Milestone) && !item.exit.is_empty() {
        local.push(Diagnostic::error(
            format!(
                "milestone '{}' in case '{}' cannot have exit criteria",
                item.name, case
            ),
            Some(item.span),
        ));
    }
    for (criterion, is_exit) in item
        .entry
        .iter()
        .map(|c| (c, false))
        .chain(item.exit.iter().map(|c| (c, true)))
    {
        let mut on_parts: Vec<OnPart> = Vec::new();
        for part in &criterion.on_parts {
            match &part.event {
                Some(word) => {
                    // Dotted: a plan item's standard event or a case-file
                    // item's transition.
                    if names.known(&part.source) {
                        if names.is_file_item(&part.source) {
                            match CASE_FILE_TRANSITIONS
                                .iter()
                                .find(|(known, _)| known == word)
                            {
                                Some((_, transition)) => {
                                    on_parts.push(OnPart::CaseFile {
                                        item: part.source.clone(),
                                        transition: *transition,
                                    });
                                }
                                None => local.push(Diagnostic::error(
                                    format!(
                                        "unknown transition '{}' on case-file item '{}' in case '{}'",
                                        word, part.source, case
                                    ),
                                    Some(part.span),
                                )),
                            }
                        } else {
                            match PLAN_ITEM_TRANSITIONS
                                .iter()
                                .find(|(known, _)| known == word)
                            {
                                Some((_, event)) => {
                                    on_parts.push(OnPart::Item {
                                        item: part.source.clone(),
                                        event: *event,
                                    });
                                }
                                None => local.push(Diagnostic::error(
                                    format!(
                                        "unknown transition '{}' on plan item '{}' in case '{}'",
                                        word, part.source, case
                                    ),
                                    Some(part.span),
                                )),
                            }
                        }
                    } else {
                        local.push(Diagnostic::error(
                            format!(
                                "unknown plan item or case-file item '{}' in case '{}'",
                                part.source, case
                            ),
                            Some(part.span),
                        ));
                    }
                }
                None => {
                    // Bare: an event of an imported `.evt` contract.
                    let matches: Vec<&EventModel> = events
                        .iter()
                        .filter(|(_, model)| {
                            model
                                .events
                                .iter()
                                .any(|event| event.name == part.source)
                        })
                        .map(|(_, model)| model)
                        .collect();
                    match matches.len() {
                        0 => {
                            local.push(
                                Diagnostic::error(
                                    format!("unknown event '{}' in case '{}'", part.source, case),
                                    Some(part.span),
                                )
                                .with_help(if events.is_empty() {
                                    "no `.evt` models were provided"
                                } else {
                                    "bare on-parts must name an event declared in an imported `.evt` file"
                                }),
                            );
                        }
                        1 => on_parts.push(OnPart::Event(part.source.clone())),
                        _ => {
                            local.push(Diagnostic::error(
                                format!("ambiguous event '{}' in case '{}'", part.source, case),
                                Some(part.span),
                            ));
                        }
                    }
                }
            }
        }
        let when = criterion
            .when
            .and_then(|span| check_when(span, source, case, &item.name, types, local));
        let lowered = Criterion { on_parts, when };
        if is_exit {
            exit.push(lowered);
        } else {
            entry.push(lowered);
        }
    }

    let kind = match &item.kind {
        dsl::CmmnItemKind::Task => PlanItemKind::Task(TaskDef {
            performer: performer.map(|(role, _)| role),
        }),
        dsl::CmmnItemKind::Service => PlanItemKind::Service(ServiceDef {
            execute: lowered_execute.unwrap_or_default(),
        }),
        dsl::CmmnItemKind::Milestone => PlanItemKind::Milestone,
        dsl::CmmnItemKind::Stage { items } => PlanItemKind::Stage(StageBody {
            children: items.iter().map(|child| child.name.clone()).collect(),
        }),
    };

    let control = (item.required || item.auto || item.repeats).then(|| ControlRules {
        required: item.required,
        manual_activation: item.auto.then_some(false),
        repetition: item.repeats,
    });

    // Depth-first order: a stage lands in the flat graph before its
    // children, so consumers read the containment tree the way the source
    // declares it.
    out.push(PlanItem {
        name: item.name.clone(),
        description: item.description.clone(),
        kind,
        entry,
        exit,
        control,
        planning: if item.discretionary {
            PlanningMode::Manual
        } else {
            PlanningMode::None
        },
    });

    if let dsl::CmmnItemKind::Stage { items } = &item.kind {
        for child in items {
            lower_plan_item(
                child,
                case,
                source,
                roles,
                names,
                seen,
                designs_by_alias,
                events,
                types,
                out,
                local,
            );
        }
    }
}

/// Type-checks one `when (...)` if-part against the synthetic case universe
/// (rule 10), returning the condition's verbatim interior text (wire rule
/// 9). Parse or type errors are diagnostics; the condition still lowers
/// with its text when only typing failed — the artifact is dropped on any
/// error anyway.
fn check_when(
    span: Span,
    source: &str,
    case: &str,
    item: &str,
    types: &rex_expr::DomainTypes,
    local: &mut Vec<Diagnostic>,
) -> Option<String> {
    // The span covers the raw parens; slice strictly inside (the
    // `when`-condition convention).
    let interior = source
        .get(span.start + 1..span.end.saturating_sub(1))
        .unwrap_or_default()
        .trim()
        .to_string();
    let parsed = rex_expr::parse(&interior);
    for error in &parsed.errors {
        local.push(
            Diagnostic::error(format!("invalid criterion condition: {}", error.message), Some(span))
                .with_help(format!("in the `when` of plan item '{item}' in case '{case}'")),
        );
    }
    let ast = parsed.ast?;
    let checker = rex_expr::TypeChecker::new(types.clone()).with_self("", case);
    match checker.type_of(&ast) {
        Ok(ty) if ty == Ty::boolean() => Some(interior),
        Ok(ty) => {
            local.push(
                Diagnostic::error(
                    format!("criterion condition must be boolean, found {ty}"),
                    Some(span),
                )
                .with_help(format!("in the `when` of plan item '{item}' in case '{case}'")),
            );
            Some(interior)
        }
        Err(errors) => {
            for error in errors {
                local.push(
                    Diagnostic::error(error.message, Some(span)).with_help(format!(
                        "in the `when` of plan item '{item}' in case '{case}'"
                    )),
                );
            }
            Some(interior)
        }
    }
}

/// Maps an AST multiplicity annotation onto the IR's bounds form. `None`
/// (no annotation) is `0..1` — the `.mox` feature convention.
fn ir_multiplicity(multiplicity: Option<&dsl::Multiplicity>) -> Multiplicity {
    use dsl::{MultBound, MultiplicityKind};
    let Some(multiplicity) = multiplicity else {
        return Multiplicity::OPTIONAL;
    };
    let (lower, upper) = match &multiplicity.kind {
        MultiplicityKind::Unbounded => (0u32, rex_ir::Upper::Unbounded),
        MultiplicityKind::Exact(n) => (*n as u32, rex_ir::Upper::Finite(*n as u32)),
        MultiplicityKind::Range(lower, upper) => (
            *lower as u32,
            match upper {
                MultBound::Int(n) => rex_ir::Upper::Finite(*n as u32),
                MultBound::Star => rex_ir::Upper::Unbounded,
            },
        ),
    };
    Multiplicity { lower, upper }
}

/// `true` when some provided actor model declares the actor name.
fn actor_known(name: &str, actors: &[(String, ActorModel)]) -> bool {
    actors.iter().any(|(_, model)| {
        model
            .blocks
            .iter()
            .any(|block| block.actors.iter().any(|actor| actor.name == name))
    })
}
