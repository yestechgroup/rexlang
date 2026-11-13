//! The decision pass: `data:` names resolve against the decision
//! services of the decision models the host provides — bare-name
//! union-wide across all models (the `requires:` discipline), after the
//! domain classes. A service with no outputs is `E0110`; a service name
//! two models declare is `E0111`; a name that resolves to a service is
//! silent here and suppresses the unknown-data-entity error in the
//! expression walk.

use std::collections::BTreeMap;

use rex_ir::dmn::{DecisionService, DmnModel};

use crate::resolve::{IfmlCompilation, IfmlDiagnostic};

use super::reachability::{instantiated_modules, site_is_instantiated};
use super::{E_AMBIGUOUS_DECISION_SERVICE, E_EMPTY_DECISION_SERVICE};

/// What a bare `data:` name resolves to in the provided decision models.
pub(super) enum DecisionData<'a> {
    /// The name matches no declared decision service.
    None,
    /// The name matches exactly one declared service; whether it
    /// declares outputs decides `E0110`.
    Service {
        /// Whether the service exposes at least one output decision.
        has_outputs: bool,
    },
    /// The name matches services declared by several models — the
    /// declaring models' display names, for the diagnostic.
    Ambiguous(Vec<&'a str>),
}

/// One service-table entry: the declared service plus the model that
/// declares it (the model names the ambiguity diagnostic).
struct ServiceEntry<'a> {
    service: &'a DecisionService,
    model: &'a DmnModel,
}

/// The decision side of the checker: bare-name lookup over the decision
/// services of every provided model, plus the did-you-mean universe the
/// unknown-data-entity hint draws from. Built once per check from the
/// host's slice; an empty slice builds no index and skips the pass
/// entirely.
pub(super) struct DecisionIndex<'a> {
    services: BTreeMap<&'a str, Vec<ServiceEntry<'a>>>,
    names: Vec<&'a String>,
}

impl<'a> DecisionIndex<'a> {
    pub(super) fn new(decisions: &'a [DmnModel]) -> Self {
        let mut services: BTreeMap<&'a str, Vec<ServiceEntry<'a>>> = BTreeMap::new();
        let mut names: Vec<&'a String> = Vec::new();
        for model in decisions {
            for service in &model.services {
                names.push(&service.name);
                services
                    .entry(service.name.as_str())
                    .or_default()
                    .push(ServiceEntry { service, model });
            }
        }
        names.sort();
        names.dedup();
        DecisionIndex { services, names }
    }

    /// Resolves a bare service name across every provided model.
    pub(super) fn resolve(&self, bare: &str) -> DecisionData<'a> {
        match self.services.get(bare).map(Vec::as_slice) {
            Some([entry]) => DecisionData::Service {
                has_outputs: !entry.service.outputs.is_empty(),
            },
            Some(entries) => DecisionData::Ambiguous(
                entries
                    .iter()
                    .map(|entry| model_display_name(entry.model))
                    .collect(),
            ),
            None => DecisionData::None,
        }
    }

    /// The unique declared service name within edit distance 2 of
    /// `requested`, for the unknown-data-entity did-you-mean hint — the
    /// unique-candidate policy (ambiguity defeats the hint).
    pub(super) fn suggestion(&self, requested: &str) -> Option<String> {
        rex_diagnostics::did_you_mean_unique(requested, self.names.iter().copied())
    }
}

/// The name a diagnostic cites for a decision model: its declared
/// `decision_model` name, else its namespace, else a placeholder — the
/// checker receives artifacts, not paths, so this is the identity the
/// wire carries.
fn model_display_name(model: &DmnModel) -> &str {
    model
        .name
        .as_deref()
        .or(model.namespace.as_deref())
        .unwrap_or("<unnamed>")
}

/// The decision pass: every `data:` binding on an **instantiated** site
/// resolves bare-name union-wide against the provided decision models'
/// service tables — a service that declares no outputs is `E0110`
/// (the component has no data contract), a name two models declare is
/// `E0111` (the declaring models are listed). A name that resolves to a
/// healthy service is silent: the service's output decisions form the
/// component's data contract, a check-only fact (the IR and the manifest
/// are unchanged), and a name that matches no service stays to the
/// expression walk's unknown-data-entity diagnostic. Spans come from the
/// index's [`DataSite`](crate::index::DataSite)s (the name token itself).
/// A module an app instantiates via `use` contributes its sites to the
/// app's surface (the reachability rule of [`instantiated_modules`]); an
/// uninstantiated module keeps its sites to its own compilation's report.
pub(super) fn check_decisions(
    compilation: &IfmlCompilation,
    decisions: &[DmnModel],
) -> Vec<IfmlDiagnostic> {
    if decisions.is_empty() {
        return Vec::new();
    }
    let table = DecisionIndex::new(decisions);
    let index = &compilation.index;
    let reached = instantiated_modules(index);
    let mut diags = Vec::new();
    for site in &index.data_sites {
        if !site_is_instantiated(&reached, site.file, site.name_span) {
            continue;
        }
        let file = index.files[site.file].path.clone();
        match table.resolve(&site.name) {
            DecisionData::None => {}
            DecisionData::Service { has_outputs } => {
                if !has_outputs {
                    diags.push(IfmlDiagnostic::error(
                        E_EMPTY_DECISION_SERVICE,
                        format!("decision service '{}' declares no outputs", site.name),
                        site.name_span,
                        file,
                    ));
                }
            }
            DecisionData::Ambiguous(models) => {
                diags.push(IfmlDiagnostic::error(
                    E_AMBIGUOUS_DECISION_SERVICE,
                    format!(
                        "decision service '{}' is declared by {} — {}",
                        site.name,
                        plural_models(models.len()),
                        model_list(&models),
                    ),
                    site.name_span,
                    file,
                ));
            }
        }
    }
    diags
}

/// The count phrase for the ambiguity diagnostic: `1 model` reads wrong,
/// so two is spelled; everything else is numeric.
fn plural_models(count: usize) -> &'static str {
    match count {
        2 => "two models",
        _ => "multiple models",
    }
}

/// The declaring-model list of the ambiguity diagnostic: `'A' and 'B'`
/// for two, `'A', 'B' and 'C'` beyond.
fn model_list(models: &[&str]) -> String {
    match models {
        [one] => format!("'{one}'"),
        [first, second] => format!("'{first}' and '{second}'"),
        [rest @ .., last] => format!(
            "{} and '{last}'",
            rest.iter()
                .map(|name| format!("'{name}'"))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        [] => String::new(),
    }
}
