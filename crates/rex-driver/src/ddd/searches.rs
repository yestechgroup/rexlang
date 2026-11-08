//! Search lowering and the rule-11 binding subchecks: text fields,
//! filter/sort, and document fields.

use rex_ir::ddd::{
    Boost, DocumentField, Pagination, RankingStrategy, SearchDef, SearchField, SearchFilter,
    SearchSort,
};
use rex_syntax::ast as dsl;

use crate::diagnostic::Diagnostic;
use crate::lower::{self, DomainPackage};

use super::*;

/// Rule 11 — the search→entity binding: the entity design must exist in
/// the search's own module. Deferred so a search may reference an entity
/// designed later in the file.
pub(crate) fn rule_11_bind_searches(
    search_bindings: &[SearchBindingCheck<'_>],
    entity_designs: &[EntityDesign],
) -> Vec<Diagnostic> {
    let mut local: Vec<Diagnostic> = Vec::new();
    for check in search_bindings {
        let designed_here = entity_designs.iter().any(|design| {
            design.package == check.package
                && design.class == check.class
                && design.module == check.module
        });
        if designed_here {
            continue;
        }
        let help = match entity_designs
            .iter()
            .find(|design| design.package == check.package && design.class == check.class)
        {
            Some(design) => format!(
                "'{0}' is designed as an entity in module '{1}'; move the search \
                 there, or drop that design",
                check.class, design.module
            ),
            None => format!(
                "add an `entity {0}` design to module '{1}' first",
                check.class, check.module
            ),
        };
        local.push(
            Diagnostic::error(
                format!(
                    "search '{}' targets '{}', which has no entity design in module '{}'",
                    check.search, check.class, check.module
                ),
                Some(check.entity.span),
            )
            .with_help(help),
        );
    }
    local
}
/// Stage 6 — one module's search projections (rule 11): the name is unique
/// application-wide, the entity line resolves like a design target and
/// queues the same-module binding check, members resolve against the
/// entity class's features, document expressions type-check with the
/// entity as `self`, and the capability queue feeds rule 10. Returns the
/// lowered searches and the stage's diagnostics.
#[allow(clippy::too_many_arguments)]
pub(crate) fn lower_module_searches<'a>(
    module: &'a dsl::DddModule,
    design_source: &str,
    packages: &[DomainPackage],
    base: Option<&str>,
    union: Option<&rex_ir::Model>,
    registries: &mut Registries<'a>,
) -> (Vec<SearchDef>, Vec<Diagnostic>) {
    let mut local: Vec<Diagnostic> = Vec::new();
    let mut searches_out = Vec::new();
    for search in &module.searches {
        // Rule 2 — search names are unique application-wide.
        if registries
            .searches
            .iter()
            .any(|(name, _)| *name == search.name.text)
        {
            local.push(
                Diagnostic::error(
                    format!("duplicate search '{}'", search.name.text),
                    Some(search.name.span),
                )
                .with_help(format!(
                    "'{}' is already declared in this application",
                    search.name.text
                )),
            );
        }

        // The entity line: required, resolving like a design target, and
        // bound to an entity design of the same module.
        let mut resolved_entity: Option<(String, String)> = None;
        match &search.entity {
            None => local.push(
                Diagnostic::error(
                    format!("search '{}' does not declare an entity", search.name.text),
                    Some(search.span),
                )
                .with_help("add `entity <Class>` to the search body"),
            ),
            Some(entity) => {
                let help = format!(
                    "the search '{}' in module '{}' must target a class declared \
                     in the imported domains",
                    search.name.text, module.name.text
                );
                let resolved = lower::resolve_ddd_class(
                    &dsl::TypeRef {
                        name: entity.clone(),
                        span: entity.span,
                    },
                    packages,
                    base,
                    &help,
                    &mut local,
                );
                if let Some((package, class)) = resolved {
                    registries.search_bindings.push(SearchBindingCheck {
                        search: &search.name.text,
                        module: &module.name.text,
                        package: package.clone(),
                        class: class.clone(),
                        entity,
                    });
                    resolved_entity = Some((package, class));
                }
            }
        }

        // Feature-level checks run only against a fully lowered union (a
        // failed domain may hide features; its diagnostics already block
        // the artifact). Inherited features resolve too.
        let entity_features = resolved_entity.as_ref().and_then(|(package, class)| {
            union
                .as_ref()
                .and_then(|union| class_features(union, package, class))
        });

        let mut search_out = SearchDef::new(
            search.name.text.clone(),
            search
                .entity
                .as_ref()
                .map(|entity| entity.full_name())
                .unwrap_or_default(),
        );
        search_out.description = search.doc.clone();
        for capability in &search.capabilities {
            search_out.capabilities.push(capability.text.clone());
            registries.capability_checks.push(CapabilityCheck {
                context: format!("search '{}'", search.name.text),
                capability,
            });
        }

        if let Some(features) = entity_features {
            let union = union.as_ref().expect("checked above");
            let (text, stage_diags) =
                rule_11_search_text_fields(search, &features, union, &search_out.entity);
            local.extend(stage_diags);
            search_out.text = text;
            let (filters, sort, stage_diags) =
                rule_11_search_filter_sort(search, &features, &search_out.entity);
            local.extend(stage_diags);
            search_out.filters = filters;
            search_out.sort = sort;
            let (document, stage_diags) = rule_11_search_document_fields(
                search,
                resolved_entity.as_ref(),
                union,
                design_source,
            );
            local.extend(stage_diags);
            search_out.document = document;
        }

        // Ranking, analyzer, and pagination lower as authored; the
        // value-range rules are validation, not rewriting.
        search_out.ranking = search.ranking.as_ref().map(|ranking| match ranking {
            dsl::DddRanking::Bm25 => RankingStrategy::Bm25,
            dsl::DddRanking::TfIdf => RankingStrategy::TfIdf,
            dsl::DddRanking::Exact => RankingStrategy::Exact,
            dsl::DddRanking::Custom(name) => RankingStrategy::Custom(name.clone()),
        });
        search_out.analyzer = search.analyzer.clone();
        if let Some(pagination) = &search.pagination {
            for (value, label) in [(pagination.limit, "limit"), (pagination.max, "max")] {
                if value.is_some_and(|value| value < 1) {
                    local.push(Diagnostic::error(
                        format!("pagination {label} must be at least 1"),
                        Some(pagination.span),
                    ));
                }
            }
            if let (Some(limit), Some(max)) = (pagination.limit, pagination.max) {
                if limit > max {
                    local.push(Diagnostic::error(
                        format!("pagination limit {limit} exceeds the max {max}"),
                        Some(pagination.span),
                    ));
                }
            }
            search_out.pagination = Some(Pagination {
                limit: pagination.limit.map(|value| value.max(0) as u32),
                max_limit: pagination.max.map(|value| value.max(0) as u32),
                cursor: pagination.cursor,
            });
        }

        if !registries
            .searches
            .iter()
            .any(|(name, _)| *name == search.name.text)
        {
            registries.searches.push((
                search.name.text.clone(),
                SearchEntry {
                    module: module.name.text.clone(),
                },
            ));
        }
        searches_out.push(search_out);
    }
    (searches_out, local)
}
/// Rule 11 — text fields: a direct, text-like feature of the entity, with
/// a boost of at least 1 and a non-empty analyzer. `union` is the fully
/// lowered union the features were read from (guaranteed by the caller).
fn rule_11_search_text_fields(
    search: &dsl::DddSearch,
    features: &[&rex_ir::Feature],
    union: &rex_ir::Model,
    entity: &str,
) -> (Vec<SearchField>, Vec<Diagnostic>) {
    let mut local: Vec<Diagnostic> = Vec::new();
    let mut fields = Vec::new();
    for field in &search.text {
        let Some(name) = single_segment(&field.property) else {
            local.push(
                Diagnostic::error(
                    format!(
                        "text field '{}' must name a feature of '{}' directly",
                        field.property.full_name(),
                        entity
                    ),
                    Some(field.property.span),
                )
                .with_help("path projections are not carried by the artifact"),
            );
            continue;
        };
        let Some(feature) = features.iter().find(|f| f.name == name) else {
            local.push(
                Diagnostic::error(
                    format!("unknown text field '{name}' on '{entity}'"),
                    Some(field.property.span),
                )
                .with_help(format!("'{entity}' declares: {}", feature_list(features))),
            );
            continue;
        };
        if !text_indexable(&feature.type_, union) {
            local.push(Diagnostic::error(
                format!(
                    "text field '{name}' is not text-like; only string, \
                     enum, datatype, and text-keyed vocabulary features may \
                     be indexed"
                ),
                Some(field.property.span),
            ));
        }
        if let Some(boost) = field.boost {
            if boost < 1 {
                local.push(Diagnostic::error(
                    format!("boost of text field '{name}' must be at least 1"),
                    Some(field.property.span),
                ));
            }
        }
        if field
            .analyzer
            .as_ref()
            .is_some_and(|analyzer| analyzer.is_empty())
        {
            local.push(Diagnostic::error(
                format!("analyzer of text field '{name}' must not be empty"),
                Some(field.property.span),
            ));
        }
        let mut field_out = SearchField::new(field.property.full_name());
        field_out.boost = field.boost.map(|boost| Boost::new(boost as f32));
        field_out.analyzer = field.analyzer.clone();
        fields.push(field_out);
    }
    (fields, local)
}
/// Rule 11 — filters and sorts: direct, non-class features of the entity.
fn rule_11_search_filter_sort(
    search: &dsl::DddSearch,
    features: &[&rex_ir::Feature],
    entity: &str,
) -> (Vec<SearchFilter>, Vec<SearchSort>, Vec<Diagnostic>) {
    let mut local: Vec<Diagnostic> = Vec::new();
    let mut filters = Vec::new();
    let mut sort = Vec::new();
    for (names, label, kind) in [
        (&search.filters, "filter", "filter"),
        (&search.sort, "sort", "sort"),
    ] {
        for property in names.iter() {
            let Some(name) = single_segment(property) else {
                local.push(
                    Diagnostic::error(
                        format!(
                            "{label} '{0}' must name a feature of '{1}' directly",
                            property.full_name(),
                            entity
                        ),
                        Some(property.span),
                    )
                    .with_help("path projections are not carried by the artifact"),
                );
                continue;
            };
            let Some(feature) = features.iter().find(|f| f.name == name) else {
                local.push(Diagnostic::error(
                    format!("unknown {label} '{name}' on '{entity}'"),
                    Some(property.span),
                ));
                continue;
            };
            if matches!(
                feature.type_,
                rex_ir::TypeRef::Class { .. } | rex_ir::TypeRef::Interface { .. }
            ) {
                local.push(Diagnostic::error(
                    format!(
                        "{kind} '{name}' is a class reference; only value \
                         features may {kind}"
                    ),
                    Some(property.span),
                ));
                continue;
            }
            match kind {
                "filter" => filters.push(SearchFilter::new(property.full_name())),
                _ => sort.push(SearchSort::new(property.full_name())),
            }
        }
    }
    (filters, sort, local)
}
/// Rule 11 — document entries: unique names, expressions parsed and
/// type-checked with the entity as `self` (any value type is legal; the
/// consumer decides the rendered form).
fn rule_11_search_document_fields(
    search: &dsl::DddSearch,
    resolved_entity: Option<&(String, String)>,
    union: &rex_ir::Model,
    design_source: &str,
) -> (Vec<DocumentField>, Vec<Diagnostic>) {
    let mut local: Vec<Diagnostic> = Vec::new();
    let mut document = Vec::new();
    let mut document_names: Vec<&str> = Vec::new();
    for entry in &search.document {
        if document_names.contains(&entry.name.text.as_str()) {
            local.push(
                Diagnostic::error(
                    format!(
                        "duplicate document field '{}' in search '{}'",
                        entry.name.text, search.name.text
                    ),
                    Some(entry.name.span),
                )
                .with_help("a document field is declared once per search"),
            );
        } else {
            document_names.push(&entry.name.text);
        }
        let text = design_source[entry.expr.start..entry.expr.end].trim();
        let parsed = rex_expr::parse(text);
        for error in &parsed.errors {
            let start = entry
                .expr
                .start
                .saturating_add(error.span.start)
                .min(entry.expr.end);
            let end = entry
                .expr
                .start
                .saturating_add(error.span.end)
                .clamp(start, entry.expr.end);
            local.push(Diagnostic::error(
                format!(
                    "invalid document expression in field '{}': {}",
                    entry.name.text, error.message
                ),
                Some((start..end).into()),
            ));
        }
        if let (Some(ast), Some((package, class))) = (&parsed.ast, resolved_entity) {
            let context = rex_expr::TypeContext::from_model(union);
            let checker = rex_expr::TypeChecker::new(context).with_self(package, class);
            if let Err(errors) = checker.type_of(ast) {
                for error in errors {
                    let start = entry
                        .expr
                        .start
                        .saturating_add(error.span.start)
                        .min(entry.expr.end);
                    let end = entry
                        .expr
                        .start
                        .saturating_add(error.span.end)
                        .clamp(start, entry.expr.end);
                    local.push(Diagnostic::error(
                        format!(
                            "invalid document expression in field '{}': {}",
                            entry.name.text, error.message
                        ),
                        Some((start..end).into()),
                    ));
                }
            }
        }
        document.push(DocumentField::new(
            entry.name.text.clone(),
            text.to_string(),
        ));
    }
    (document, local)
}
/// The single segment of a name that must reference a feature directly;
/// `None` for a multi-segment (path) reference.
fn single_segment(name: &dsl::QualifiedName) -> Option<&str> {
    match name.segments.as_slice() {
        [segment] => Some(segment.text.as_str()),
        _ => None,
    }
}
/// The comma-joined feature names of a class, for "did you mean" help.
fn feature_list(features: &[&rex_ir::Feature]) -> String {
    features
        .iter()
        .map(|feature| feature.name.as_str())
        .collect::<Vec<_>>()
        .join(", ")
}
/// Whether a feature's type is text-like and may be indexed: the string
/// primitives, enums (literal names are text), datatypes (the string-family
/// default the constraint rules use), and vocabularies whose key facet is a
/// string primitive. Class references, numerics, booleans, and dates are
/// filter/sort facets, not indexed prose.
fn text_indexable(type_: &rex_ir::TypeRef, union: &rex_ir::Model) -> bool {
    match type_ {
        rex_ir::TypeRef::Primitive(primitive) => matches!(
            primitive,
            rex_ir::PrimitiveType::String | rex_ir::PrimitiveType::Char
        ),
        rex_ir::TypeRef::Enum { .. } => true,
        rex_ir::TypeRef::Datatype { .. } => true,
        rex_ir::TypeRef::Vocabulary { package, name } => union
            .packages
            .iter()
            .find(|candidate| candidate.name == *package)
            .and_then(|candidate| candidate.vocabularies.iter().find(|v| v.name == *name))
            .and_then(|vocabulary| {
                vocabulary
                    .facets
                    .iter()
                    .find(|facet| facet.name == vocabulary.key)
            })
            .is_some_and(|key| {
                matches!(
                    key.type_,
                    rex_ir::PrimitiveType::String | rex_ir::PrimitiveType::Char
                )
            }),
        rex_ir::TypeRef::Class { .. } | rex_ir::TypeRef::Interface { .. } => false,
    }
}
