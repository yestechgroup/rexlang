//! Semantic lowering and validation for `.deploy` deployment sources: the
//! `.deploy` counterpart of [`crate::events`] and [`crate::ddd`].
//!
//! This module is salsa-free; [`compile_deploy_str`](crate::compile_deploy_str)
//! parses via the tracked [`parse_deploy_query`](crate::parse_deploy_query)
//! and calls into [`compile_deploy_file`] directly (there is no domain
//! lowering to memoize — a deployment references pre-compiled design/flow
//! artifacts, not domain texts). The normative contract — entry points,
//! resolution semantics, and the validation rules — is documented on
//! [`compile_deploy_str`](crate::compile_deploy_str).
//!
//! The engine mirrors [`crate::events::compile_evt_file`]: parse
//! diagnostics propagate under the deploy file's path, its own diagnostics
//! are tagged with its path in declaration order, and any error-severity
//! diagnostic drops the artifact.
//!
//! Targets are implementation-provided: the capability, engine, and
//! setting tables below are the closed semantic core the issue-specified
//! "validate before generating" rule runs against. Everything
//! platform-specific (capability names beyond the closed vocabulary,
//! mapping resources, policy expressions) is carried as authored.

use std::collections::BTreeMap;

use rex_ir::ddd::DddModel;
use rex_ir::deploy::{
    ComponentKind, DeployApplication, DeployComponent, DeployConnection, DeployDeployment,
    DeployMapping, DeployModel, DeployPolicy, DeployProfile, DeploySetting, DeployTarget,
    DeployValue,
};
use rex_ir::ifml::IfmlModel;
use rex_syntax::ast as dsl;
use rex_syntax::Span;

use crate::diagnostic::Diagnostic;
use crate::import_matches;

/// The closed capability vocabulary a component's `requires` clause may
/// name (extensible per target implementation, but validated so a typo is
/// an error, not a silently-vacuous requirement).
const CAPABILITIES: [&str; 8] = [
    "longRunningProcess",
    "persistentFilesystem",
    "sqlDatabase",
    "container",
    "backgroundWorker",
    "horizontalScaling",
    "edgeExecution",
    "statefulProcess",
];

/// The capabilities each target provides. These tables — not the DSL — are
/// the authority: a target's capabilities are defined by its
/// implementation, never authored in a `.deploy` source.
fn target_capabilities(target: DeployTarget) -> &'static [&'static str] {
    match target {
        DeployTarget::Standalone => &[
            "longRunningProcess",
            "persistentFilesystem",
            "sqlDatabase",
            "backgroundWorker",
            "statefulProcess",
        ],
        DeployTarget::DockerCompose | DeployTarget::Kubernetes => &[
            "longRunningProcess",
            "persistentFilesystem",
            "sqlDatabase",
            "backgroundWorker",
            "statefulProcess",
            "container",
            "horizontalScaling",
        ],
        DeployTarget::CloudflareWorkers => &["sqlDatabase", "horizontalScaling", "edgeExecution"],
    }
}

/// The capabilities a component kind intrinsically requires, whatever its
/// settings say.
fn kind_required_capabilities(kind: ComponentKind) -> &'static [&'static str] {
    match kind {
        ComponentKind::Worker => &["longRunningProcess"],
        ComponentKind::Database => &["sqlDatabase"],
        _ => &[],
    }
}

/// The database engines each target can host. A managed/external engine
/// (postgres on the cluster, D1 at the edge) counts as the target hosting
/// it — the check rejects *incompatible* engines rather than enumerating
/// hosting topologies.
fn target_engines(target: DeployTarget) -> &'static [&'static str] {
    match target {
        DeployTarget::Standalone => &["sqlite"],
        DeployTarget::DockerCompose => &["sqlite", "postgres"],
        DeployTarget::Kubernetes => &["postgres"],
        DeployTarget::CloudflareWorkers => &["d1"],
    }
}

/// The value shape of one setting in a kind's vocabulary.
enum SettingSpec {
    /// An integer setting (`replicas`).
    Int,
    /// A free-text setting (`entrypoint`).
    Text,
    /// A closed word setting (`runtime`, `engine`, ...) with its allowed
    /// values.
    Word(&'static [&'static str]),
}

const CONTAINER_OR_EMBEDDED_OR_WORKER: &[&str] = &["embedded", "container", "worker", "external"];
const CONTAINER_OR_EMBEDDED_OR_EXTERNAL: &[&str] = &["embedded", "container", "external"];
const ENGINES: &[&str] = &["sqlite", "postgres", "d1"];
const STORAGES: &[&str] = &["local", "volume", "external", "managed"];
const SCALINGS: &[&str] = &["singleInstance", "replicated", "autoscaled"];
const FRONTEND_RUNTIMES: &[&str] = &["static", "worker", "container"];

/// The setting vocabulary of one component kind: the closed semantic core.
/// A setting path not listed here (for the component's kind) is an error,
/// and a word value outside its list is an error.
fn component_settings(kind: ComponentKind) -> Vec<(&'static str, SettingSpec)> {
    match kind {
        ComponentKind::Api => vec![
            (
                "runtime",
                SettingSpec::Word(CONTAINER_OR_EMBEDDED_OR_WORKER),
            ),
            ("replicas", SettingSpec::Int),
            ("scaling", SettingSpec::Word(SCALINGS)),
        ],
        ComponentKind::Worker => vec![
            (
                "runtime",
                SettingSpec::Word(CONTAINER_OR_EMBEDDED_OR_EXTERNAL),
            ),
            ("replicas", SettingSpec::Int),
        ],
        ComponentKind::Database => vec![
            ("engine", SettingSpec::Word(ENGINES)),
            ("storage", SettingSpec::Word(STORAGES)),
            (
                "runtime",
                SettingSpec::Word(CONTAINER_OR_EMBEDDED_OR_EXTERNAL),
            ),
        ],
        ComponentKind::Queue => vec![
            (
                "runtime",
                SettingSpec::Word(CONTAINER_OR_EMBEDDED_OR_EXTERNAL),
            ),
            ("replicas", SettingSpec::Int),
        ],
        ComponentKind::ObjectStore => vec![("storage", SettingSpec::Word(STORAGES))],
        ComponentKind::Frontend => vec![
            ("runtime", SettingSpec::Word(FRONTEND_RUNTIMES)),
            ("entrypoint", SettingSpec::Text),
        ],
    }
}

/// Lowers and validates one parsed `.deploy` file against the pre-compiled
/// design and flow artifacts its imports resolve to. Returns the
/// deployment artifact (or `None` when any error-severity diagnostic was
/// produced) plus the diagnostics, each tagged with the deploy file's
/// path.
///
/// See [`compile_deploy_str`](crate::compile_deploy_str) for the normative
/// contract.
pub(crate) fn compile_deploy_file(
    path: &str,
    source: &str,
    ast: Option<&dsl::DeployFile>,
    parse_diagnostics: &[Diagnostic],
    designs: &[(String, DddModel)],
    flows: &[(String, IfmlModel)],
) -> (Option<DeployModel>, Vec<(String, Diagnostic)>) {
    let mut diags: Vec<(String, Diagnostic)> = parse_diagnostics
        .iter()
        .map(|diagnostic| (path.to_string(), diagnostic.clone()))
        .collect();

    let Some(ast) = ast else {
        return (None, diags);
    };

    let mut local: Vec<Diagnostic> = Vec::new();
    let mut model = DeployModel::new();

    // Rule 2 — every import must name a provided `.ddd` design or `.ifml`
    // flow (exact string or lexical resolution, the `.actor` rule).
    for import in &ast.imports {
        let provided_design = designs
            .iter()
            .any(|(provided, _)| import_matches(&import.path, path, provided));
        let provided_flow = flows
            .iter()
            .any(|(provided, _)| import_matches(&import.path, path, provided));
        if !provided_design && !provided_flow {
            local.push(
                Diagnostic::error(
                    format!("imported file \"{}\" was not provided", import.path),
                    Some(import.span),
                )
                .with_help("imports must name a .ddd design or an .ifml flow the host provides"),
            );
        } else if !import.path.ends_with(".ddd") && !import.path.ends_with(".ifml") {
            local.push(
                Diagnostic::error(
                    format!(
                        "unsupported import \"{}\" (expected a .ddd design or an .ifml flow)",
                        import.path
                    ),
                    Some(import.span),
                )
                .with_help("a deployment composes .ddd designs and .ifml flows"),
            );
        }
    }

    // Applications: unique names; per application, unique component names,
    // validated kinds/capabilities/settings/bindings, and known connection
    // endpoints.
    let mut application_names: Vec<(String, Span)> = Vec::new();
    for application in &ast.applications {
        if application_names
            .iter()
            .any(|(name, _)| *name == application.name.text)
        {
            local.push(
                Diagnostic::error(
                    format!("duplicate application '{}'", application.name.text),
                    Some(application.name.span),
                )
                .with_help(format!(
                    "'{}' is already declared in this file",
                    application.name.text
                )),
            );
        } else {
            application_names.push((application.name.text.clone(), application.name.span));
        }

        let mut out = DeployApplication::new(application.name.text.clone());
        // Components keyed by name for connection and setting resolution,
        // with the component-name spans for component-level diagnostics
        // assembled after the AST is gone.
        let mut components: Vec<(String, ComponentKind, Span)> = Vec::new();
        for component in &application.components {
            if components
                .iter()
                .any(|(name, _, _)| *name == component.name.text)
            {
                local.push(
                    Diagnostic::error(
                        format!(
                            "duplicate component '{}' in application '{}'",
                            component.name.text, application.name.text
                        ),
                        Some(component.name.span),
                    )
                    .with_help("a component is declared once per application"),
                );
                continue;
            }

            // Rule 3 — the kind must be in the closed vocabulary.
            let kind = match ComponentKind::from_keyword(&component.kind.text) {
                Some(kind) => kind,
                None => {
                    local.push(
                        Diagnostic::error(
                            format!("unknown component kind '{}'", component.kind.text),
                            Some(component.kind.span),
                        )
                        .with_help(format!(
                            "known kinds: {}",
                            [
                                "api",
                                "worker",
                                "database",
                                "queue",
                                "objectStore",
                                "frontend"
                            ]
                            .join(", ")
                        )),
                    );
                    continue;
                }
            };

            // Rule 4 — capability names must be in the closed vocabulary.
            for capability in &component.requires {
                if !CAPABILITIES.contains(&capability.text.as_str()) {
                    local.push(
                        Diagnostic::error(
                            format!("unknown capability '{}'", capability.text),
                            Some(capability.span),
                        )
                        .with_help(format!("known capabilities: {}", CAPABILITIES.join(", "))),
                    );
                }
            }

            // Rule 5 — baseline settings use single-segment paths from the
            // kind's vocabulary.
            let mut seen_settings: Vec<&str> = Vec::new();
            let mut settings: Vec<DeploySetting> = Vec::new();
            for setting in &component.settings {
                validate_setting_path_len(&setting.path, 1, &mut local);
                let name = setting.path.segments.last().map(|name| name.text.as_str());
                if let Some(name) = name {
                    if seen_settings.contains(&name) {
                        local.push(
                            Diagnostic::error(
                                format!(
                                    "duplicate setting '{}' on component '{}'",
                                    name, component.name.text
                                ),
                                Some(setting.path.span),
                            )
                            .with_help("a setting is declared once per level"),
                        );
                        continue;
                    }
                    seen_settings.push(name);
                }
                let Some(value) =
                    check_setting_value(&setting.path, &setting.value, kind, &mut local)
                else {
                    continue;
                };
                settings.push(DeploySetting {
                    path: setting.path.full_name(),
                    value,
                });
            }

            // Rule 6 — bindings must resolve against the provided
            // artifacts: the path (as written) must be an import of this
            // file, and a `#<Module>` suffix must name a module there.
            let mut designs_out = Vec::new();
            let mut flows_out = Vec::new();
            for binding in &component.designs {
                let resolved = rex_ir::deploy::DesignBinding::parse(&binding.reference);
                if let Some(diagnostic) = check_design_binding(&resolved, path, component, designs)
                {
                    local.push(diagnostic);
                }
                designs_out.push(resolved);
            }
            for binding in &component.flows {
                let resolved = rex_ir::deploy::FlowBinding::parse(&binding.reference);
                if let Some(diagnostic) = check_flow_binding(&resolved, path, component, flows) {
                    local.push(diagnostic);
                }
                flows_out.push(resolved);
            }

            components.push((component.name.text.clone(), kind, component.name.span));
            let mut component_out = DeployComponent::new(component.name.text.clone(), kind);
            component_out.requires = component
                .requires
                .iter()
                .map(|capability| capability.text.clone())
                .collect();
            component_out.settings = settings;
            component_out.designs = designs_out;
            component_out.flows = flows_out;
            out.components.push(component_out);
        }

        for connection in &application.connections {
            for endpoint in [&connection.from, &connection.to] {
                if endpoint.text.is_empty() {
                    continue; // parser recovery; the syntax error fired
                }
                if !components.iter().any(|(name, _, _)| *name == endpoint.text) {
                    local.push(
                        Diagnostic::error(
                            format!(
                                "unknown component '{}' in connection from '{}' to '{}'",
                                endpoint.text, connection.from.text, connection.to.text
                            ),
                            Some(endpoint.span),
                        )
                        .with_help(format!(
                            "connections must reference components of application '{}'",
                            application.name.text
                        )),
                    );
                }
            }
            out.connections.push(DeployConnection::new(
                connection.from.text.clone(),
                connection.to.text.clone(),
            ));
        }

        model.applications.push(out);
    }

    // Profiles: unique names, a known target, shape-valid defaults,
    // validated mapping kinds.
    let mut profile_names: Vec<(String, Span)> = Vec::new();
    for profile in &ast.profiles {
        if profile_names
            .iter()
            .any(|(name, _)| *name == profile.name.text)
        {
            local.push(
                Diagnostic::error(
                    format!("duplicate profile '{}'", profile.name.text),
                    Some(profile.name.span),
                )
                .with_help(format!(
                    "'{}' is already declared in this file",
                    profile.name.text
                )),
            );
        } else {
            profile_names.push((profile.name.text.clone(), profile.name.span));
        }

        // Rule 7 — a profile selects an implementation-provided target.
        let target = match profile.target.as_ref() {
            Some(target) => match DeployTarget::from_keyword(&target.text) {
                Some(target) => target,
                None => {
                    local.push(
                        Diagnostic::error(
                            format!("unknown deployment target '{}'", target.text),
                            Some(target.span),
                        )
                        .with_help(format!(
                            "known targets: {}",
                            [
                                "standalone",
                                "dockerCompose",
                                "kubernetes",
                                "cloudflareWorkers"
                            ]
                            .join(", ")
                        )),
                    );
                    continue;
                }
            },
            None => {
                local.push(
                    Diagnostic::error(
                        format!("profile '{}' declares no `target:` line", profile.name.text),
                        Some(profile.name.span),
                    )
                    .with_help(
                        "add `target: standalone | dockerCompose | kubernetes | cloudflareWorkers`",
                    ),
                );
                continue;
            }
        };

        let mut profile_out = DeployProfile::new(profile.name.text.clone(), target);

        // Defaults are shape-validated here (two-segment paths, no
        // duplicates); the vocabulary/type validation runs per deployment,
        // against the application the profile is being applied to.
        let mut seen_defaults: Vec<String> = Vec::new();
        for setting in &profile.defaults {
            validate_setting_path_len(&setting.path, 2, &mut local);
            if let Some(full) = Some(setting.path.full_name()) {
                if seen_defaults.contains(&full) {
                    local.push(
                        Diagnostic::error(
                            format!(
                                "duplicate default '{}' in profile '{}'",
                                full, profile.name.text
                            ),
                            Some(setting.path.span),
                        )
                        .with_help("a setting is declared once per level"),
                    );
                    continue;
                }
                seen_defaults.push(full);
            }
            profile_out.defaults.push(DeploySetting {
                path: setting.path.full_name(),
                value: setting_value_as_authored(&setting.value),
            });
        }

        for policy in &profile.policies {
            let expression = policy_text(source, &policy.expr);
            let expression = match expression {
                Some(expression) => expression,
                None => continue, // recovery placeholder; the syntax error fired
            };
            profile_out.policies.push(if policy.prohibit {
                DeployPolicy::prohibit(expression)
            } else {
                DeployPolicy::require(expression)
            });
        }

        for mapping in &profile.mappings {
            let kind = match ComponentKind::from_keyword(&mapping.kind.text) {
                Some(kind) => kind,
                None => {
                    local.push(
                        Diagnostic::error(
                            format!("unknown component kind '{}' in mapping", mapping.kind.text),
                            Some(mapping.kind.span),
                        )
                        .with_help(format!(
                            "known kinds: {}",
                            [
                                "api",
                                "worker",
                                "database",
                                "queue",
                                "objectStore",
                                "frontend"
                            ]
                            .join(", ")
                        )),
                    );
                    continue;
                }
            };
            if mapping.resource.segments.is_empty() {
                continue; // parser recovery; the syntax error fired
            }
            profile_out
                .mappings
                .push(DeployMapping::new(kind, mapping.resource.full_name()));
        }

        model.profiles.push(profile_out);
    }

    // Deployments: unique names, known application and profile, then the
    // per-deployment rules — effective settings, capabilities, engines,
    // and policy enforcement.
    let mut deployment_names: Vec<(String, Span)> = Vec::new();
    // Component-name spans per application, for component-level diagnostics
    // assembled after the AST application is out of scope.
    let mut component_spans: BTreeMap<String, BTreeMap<String, Span>> = BTreeMap::new();
    for application in &ast.applications {
        let spans = component_spans
            .entry(application.name.text.clone())
            .or_default();
        for component in &application.components {
            spans.insert(component.name.text.clone(), component.name.span);
        }
    }
    for deployment in &ast.deployments {
        if deployment_names
            .iter()
            .any(|(name, _)| *name == deployment.name.text)
        {
            local.push(
                Diagnostic::error(
                    format!("duplicate deployment '{}'", deployment.name.text),
                    Some(deployment.name.span),
                )
                .with_help(format!(
                    "'{}' is already declared in this file",
                    deployment.name.text
                )),
            );
        } else {
            deployment_names.push((deployment.name.text.clone(), deployment.name.span));
        }

        let application = match model
            .applications
            .iter()
            .find(|candidate| candidate.name == deployment.application.text)
        {
            Some(application) => application.clone(),
            None => {
                if !deployment.application.text.is_empty() {
                    local.push(
                        Diagnostic::error(
                            format!(
                                "unknown application '{}' in deployment '{}'",
                                deployment.application.text, deployment.name.text
                            ),
                            Some(deployment.application.span),
                        )
                        .with_help("deployments must reference applications of this file"),
                    );
                }
                continue;
            }
        };
        let profile_name = match &deployment.profile {
            Some(profile) if !profile.text.is_empty() => profile.text.clone(),
            Some(_) | None => {
                // The parser reported the missing `use` line; recover with
                // no profile so the remaining rules still see the file.
                String::new()
            }
        };
        let profile = match model
            .profiles
            .iter()
            .find(|candidate| candidate.name == profile_name)
        {
            Some(profile) => Some(profile.clone()),
            None => {
                if !profile_name.is_empty() {
                    local.push(
                        Diagnostic::error(
                            format!(
                                "unknown profile '{}' in deployment '{}'",
                                profile_name, deployment.name.text
                            ),
                            Some(
                                deployment
                                    .profile
                                    .as_ref()
                                    .map(|profile| profile.span)
                                    .unwrap_or(deployment.span),
                            ),
                        )
                        .with_help("deployments must reference profiles of this file"),
                    );
                }
                None
            }
        };

        // Rule 8 — effective settings: component baselines, then the
        // profile's defaults, then the deployment's overrides. Every
        // default/override must name a component of the application and a
        // setting from that component kind's vocabulary. Both levels are
        // validated from the AST settings (which carry spans).
        let mut effective: BTreeMap<String, BTreeMap<String, DeployValue>> = BTreeMap::new();
        for component in &application.components {
            let mut settings = BTreeMap::new();
            for setting in &component.settings {
                settings.insert(setting.path.clone(), setting.value.clone());
            }
            effective.insert(component.name.clone(), settings);
        }
        if let Some(profile) = &profile {
            if let Some(ast_profile) = ast
                .profiles
                .iter()
                .find(|candidate| candidate.name.text == profile.name)
            {
                apply_settings(
                    &ast_profile.defaults,
                    &application,
                    &mut effective,
                    &mut local,
                    SettingsLevel::Default(&profile.name),
                );
            }
        }
        let mut seen_configure: Vec<String> = Vec::new();
        let mut configure_out = Vec::new();
        for setting in &deployment.configure {
            validate_setting_path_len(&setting.path, 2, &mut local);
            let full = setting.path.full_name();
            if seen_configure.contains(&full) {
                local.push(
                    Diagnostic::error(
                        format!(
                            "duplicate override '{}' in deployment '{}'",
                            full, deployment.name.text
                        ),
                        Some(setting.path.span),
                    )
                    .with_help("a setting is declared once per level"),
                );
                continue;
            }
            seen_configure.push(full.clone());
            if let Some(value) = check_setting_against_application(
                &setting.path,
                &setting.value,
                &application,
                &mut local,
                &SettingsLevel::Override(&deployment.name.text),
            ) {
                if let Some((component, name)) = split_setting_path(&full) {
                    effective
                        .entry(component.to_string())
                        .or_default()
                        .insert(name.to_string(), value.clone());
                }
                configure_out.push(DeploySetting { path: full, value });
            }
        }

        let Some(profile) = &profile else {
            continue; // unknown profile: the remaining rules need a target
        };

        // Rule 9 — capability validation: each component's intrinsic and
        // declared requirements must be covered by the target.
        for component in &application.components {
            let settings = &effective[&component.name];
            let span = component_spans
                .get(&deployment.application.text)
                .and_then(|spans| spans.get(&component.name))
                .copied()
                .unwrap_or(deployment.span);
            let mut required: Vec<&'static str> =
                kind_required_capabilities(component.kind).to_vec();
            if effective_word(settings, "engine") == Some("sqlite") {
                required.push("persistentFilesystem");
            }
            if effective_word(settings, "runtime") == Some("container") {
                required.push("container");
            }
            for capability in &component.requires {
                // The closed vocabulary was already validated at the
                // application, so this is only a lookup convenience.
                if let Some(capability) = CAPABILITIES.iter().find(|known| *known == capability) {
                    if !required.contains(capability) {
                        required.push(capability);
                    }
                }
            }
            for capability in required {
                if !target_capabilities(profile.target).contains(&capability) {
                    local.push(
                        Diagnostic::error(
                            format!(
                                "component '{}' requires capability '{}', which target '{}' \
                                 does not provide",
                                component.name,
                                capability,
                                profile.target.keyword()
                            ),
                            Some(span),
                        )
                        .with_help(format!(
                            "target '{}' provides: {}",
                            profile.target.keyword(),
                            target_capabilities(profile.target).join(", ")
                        )),
                    );
                }
            }

            // Rule 10 — engine/target compatibility: the issue's D1 rule —
            // a postgres database is not silently mapped onto a target
            // that cannot host it.
            if component.kind == ComponentKind::Database {
                if let Some(engine) = effective_word(settings, "engine") {
                    if !target_engines(profile.target).contains(&engine) {
                        local.push(
                            Diagnostic::error(
                                format!(
                                    "database '{}' engine '{}' is not available on target '{}'",
                                    component.name,
                                    engine,
                                    profile.target.keyword()
                                ),
                                Some(span),
                            )
                            .with_help(format!(
                                "target '{}' hosts: {}",
                                profile.target.keyword(),
                                target_engines(profile.target).join(", ")
                            )),
                        );
                    }
                }
            }
        }

        // Rule 11 — policies: parsed and typed with rex-expr (the
        // `when`-condition pattern), then evaluated against the effective
        // settings. `require` conditions must hold; `prohibit` conditions
        // must not.
        for policy in &profile.policies {
            check_policy(policy, &application, deployment, &effective, &mut local);
        }

        let mut deployment_out = DeployDeployment::new(
            deployment.name.text.clone(),
            deployment.application.text.clone(),
            profile_name,
        );
        deployment_out.configure = configure_out;
        model.deployments.push(deployment_out);
    }

    diags.extend(local.into_iter().map(|d| (path.to_string(), d)));

    let blocked = diags.iter().any(|(_, diagnostic)| diagnostic.is_error());
    ((!blocked).then_some(model), diags)
}

/// The two-segment split of a `component.setting` path.
fn split_setting_path(path: &str) -> Option<(&str, &str)> {
    let (component, setting) = path.split_once('.')?;
    Some((component, setting))
}

/// A word-valued setting from an effective-settings map.
fn effective_word<'a>(settings: &'a BTreeMap<String, DeployValue>, name: &str) -> Option<&'a str> {
    match settings.get(name)? {
        DeployValue::Word(word) => Some(word.as_str()),
        DeployValue::Text(text) => Some(text.as_str()),
        DeployValue::Int(_) => None,
    }
}

/// The source text of a policy condition, strictly inside the parens (the
/// `when`-condition convention). `None` for the recovery placeholder.
fn policy_text(source: &str, expr: &Span) -> Option<String> {
    if expr.end >= expr.start + 2 {
        Some(source[expr.start + 1..expr.end - 1].trim().to_string())
    } else {
        None
    }
}

/// Rejects a setting path with the wrong segment count for its level
/// (component baselines are single-segment; defaults/overrides are
/// `component.setting` pairs).
fn validate_setting_path_len(
    path: &dsl::QualifiedName,
    expected: usize,
    local: &mut Vec<Diagnostic>,
) {
    if path.segments.len() != expected {
        local.push(
            Diagnostic::error(
                format!(
                    "setting path '{}' must have exactly {} segment{}",
                    path.full_name(),
                    expected,
                    if expected == 1 { "" } else { "s" }
                ),
                Some(path.span),
            )
            .with_help(if expected == 1 {
                "component settings are `name: value` lines"
            } else {
                "defaults and overrides are `component.setting: value` lines"
            }),
        );
    }
}

/// Validates one baseline component setting against its kind's vocabulary
/// and lowers the value. `None` when a diagnostic was emitted.
fn check_setting_value(
    path: &dsl::QualifiedName,
    value: &dsl::DeploySettingValue,
    kind: ComponentKind,
    local: &mut Vec<Diagnostic>,
) -> Option<DeployValue> {
    let name = path.segments.last()?.text.clone();
    check_setting(&name, path.span, value, kind, local)
}

/// Validates one default/override setting against the application the
/// profile/deployment applies to: the path's first segment must name a
/// component, the second a setting from that component kind's vocabulary.
fn check_setting_against_application(
    path: &dsl::QualifiedName,
    value: &dsl::DeploySettingValue,
    application: &DeployApplication,
    local: &mut Vec<Diagnostic>,
    level: &SettingsLevel<'_>,
) -> Option<DeployValue> {
    let segments: Vec<&str> = path
        .segments
        .iter()
        .map(|segment| segment.text.as_str())
        .collect();
    let [component_name, setting_name] = segments.as_slice() else {
        return None; // the segment-count diagnostic already fired
    };
    let Some(component) = application
        .components
        .iter()
        .find(|component| component.name == *component_name)
    else {
        local.push(
            Diagnostic::error(
                format!(
                    "{} names unknown component '{}' — application '{}' has no such component",
                    level.label(),
                    component_name,
                    application.name
                ),
                Some(path.span),
            )
            .with_help("defaults and overrides must name components of the deployed application"),
        );
        return None;
    };
    check_setting(setting_name, path.span, value, component.kind, local)
}

/// Which settings list a diagnostic is about (for the message's wording).
enum SettingsLevel<'a> {
    Default(&'a str),
    Override(&'a str),
}

impl SettingsLevel<'_> {
    /// The diagnostic prefix: `default of profile 'p'` /
    /// `override of deployment 'd'`.
    fn label(&self) -> String {
        match self {
            SettingsLevel::Default(name) => format!("default of profile '{name}'"),
            SettingsLevel::Override(name) => format!("override of deployment '{name}'"),
        }
    }
}

/// Validates a setting name/value pair against one kind's vocabulary.
fn check_setting(
    name: &str,
    span: Span,
    value: &dsl::DeploySettingValue,
    kind: ComponentKind,
    local: &mut Vec<Diagnostic>,
) -> Option<DeployValue> {
    let Some((_, spec)) = component_settings(kind)
        .into_iter()
        .find(|(known, _)| *known == name)
    else {
        let known = component_settings(kind)
            .into_iter()
            .map(|(known, _)| known)
            .collect::<Vec<_>>()
            .join(", ");
        local.push(
            Diagnostic::error(
                format!("unknown setting '{}' for kind '{}'", name, kind.keyword()),
                Some(span),
            )
            .with_help(format!("{} settings: {}", kind.keyword(), known)),
        );
        return None;
    };
    match (&spec, value) {
        (SettingSpec::Int, dsl::DeploySettingValue::Int(value)) => Some(DeployValue::Int(*value)),
        (SettingSpec::Int, _) => {
            local.push(
                Diagnostic::error(format!("setting '{name}' expects an integer"), Some(span))
                    .with_help("integer settings take a plain integer literal"),
            );
            None
        }
        (SettingSpec::Text, dsl::DeploySettingValue::Str(text)) => {
            Some(DeployValue::Text(text.clone()))
        }
        (SettingSpec::Text, _) => {
            local.push(
                Diagnostic::error(format!("setting '{name}' expects a string"), Some(span))
                    .with_help("text settings take a double-quoted string"),
            );
            None
        }
        (SettingSpec::Word(allowed), dsl::DeploySettingValue::Word(word)) => {
            if allowed.contains(&word.text.as_str()) {
                Some(DeployValue::Word(word.text.clone()))
            } else {
                local.push(
                    Diagnostic::error(format!("invalid {} '{}'", name, word.text), Some(word.span))
                        .with_help(format!("expected one of: {}", allowed.join(", "))),
                );
                None
            }
        }
        (SettingSpec::Word(_), _) => {
            local.push(
                Diagnostic::error(format!("setting '{name}' expects a word"), Some(span))
                    .with_help("word settings take a bare name (the enum-value spelling)"),
            );
            None
        }
    }
}

/// The authored value of a profile default, before any per-deployment
/// validation (profiles are application-independent at compile time).
fn setting_value_as_authored(value: &dsl::DeploySettingValue) -> DeployValue {
    match value {
        dsl::DeploySettingValue::Int(value) => DeployValue::Int(*value),
        dsl::DeploySettingValue::Str(text) => DeployValue::Text(text.clone()),
        dsl::DeploySettingValue::Word(word) => DeployValue::Word(word.text.clone()),
    }
}

/// Applies a profile's defaults (or a deployment's overrides) onto the
/// effective-settings map, validating each AST setting against the
/// application: two-segment paths, no duplicates, and the component kind's
/// vocabulary.
fn apply_settings(
    settings: &[dsl::DeploySettingDecl],
    application: &DeployApplication,
    effective: &mut BTreeMap<String, BTreeMap<String, DeployValue>>,
    local: &mut Vec<Diagnostic>,
    level: SettingsLevel<'_>,
) {
    let mut seen: Vec<String> = Vec::new();
    for setting in settings {
        validate_setting_path_len(&setting.path, 2, local);
        let full = setting.path.full_name();
        if seen.contains(&full) {
            local.push(Diagnostic::error(
                format!(
                    "duplicate {} '{}' — a setting is declared once per level",
                    level.label(),
                    full
                ),
                Some(setting.path.span),
            ));
            continue;
        }
        seen.push(full.clone());
        let Some(value) = check_setting_against_application(
            &setting.path,
            &setting.value,
            application,
            local,
            &level,
        ) else {
            continue;
        };
        if let Some((component_name, setting_name)) = split_setting_path(&full) {
            effective
                .entry(component_name.to_string())
                .or_default()
                .insert(setting_name.to_string(), value);
        }
    }
}

/// Rule 6 — a design binding must reference one of the file's `.ddd`
/// imports (exact string or lexical resolution) and, when it names a
/// module, that module must exist in the imported design.
fn check_design_binding(
    binding: &rex_ir::deploy::DesignBinding,
    deploy_path: &str,
    component: &dsl::DeployComponentDecl,
    designs: &[(String, DddModel)],
) -> Option<Diagnostic> {
    let resolved = designs
        .iter()
        .find(|(provided, _)| import_matches(&binding.source, deploy_path, provided));
    let Some((_, design)) = resolved else {
        return Some(
            Diagnostic::error(
                format!(
                    "component '{}' binds unknown design \"{}\"",
                    component.name.text, binding.source
                ),
                Some(component.span),
            )
            .with_help("a design binding's path must match one of this file's imports"),
        );
    };
    if let Some(module) = &binding.module {
        if !design
            .modules
            .iter()
            .any(|candidate| &candidate.name == module)
        {
            return Some(
                Diagnostic::error(
                    format!(
                        "design \"{}\" declares no module '{}' (component '{}')",
                        binding.source, module, component.name.text
                    ),
                    Some(component.span),
                )
                .with_help(format!(
                    "modules of {}: {}",
                    binding.source,
                    design
                        .modules
                        .iter()
                        .map(|module| module.name.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                )),
            );
        }
    }
    None
}

/// Rule 6 — the flow-binding twin of [`check_design_binding`].
fn check_flow_binding(
    binding: &rex_ir::deploy::FlowBinding,
    deploy_path: &str,
    component: &dsl::DeployComponentDecl,
    flows: &[(String, IfmlModel)],
) -> Option<Diagnostic> {
    let resolved = flows
        .iter()
        .find(|(provided, _)| import_matches(&binding.source, deploy_path, provided));
    let Some((_, flow)) = resolved else {
        return Some(
            Diagnostic::error(
                format!(
                    "component '{}' binds unknown flow \"{}\"",
                    component.name.text, binding.source
                ),
                Some(component.span),
            )
            .with_help("a flow binding's path must match one of this file's imports"),
        );
    };
    if let Some(module) = &binding.module {
        if !flow
            .modules
            .iter()
            .any(|candidate| &candidate.name == module)
        {
            return Some(
                Diagnostic::error(
                    format!(
                        "flow \"{}\" declares no module '{}' (component '{}')",
                        binding.source, module, component.name.text
                    ),
                    Some(component.span),
                )
                .with_help(format!(
                    "modules of {}: {}",
                    binding.source,
                    flow.modules
                        .iter()
                        .map(|module| module.name.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                )),
            );
        }
    }
    None
}

/// Rule 11 — one policy condition, end to end: parse (rex-expr), type-check
/// against the application's components (the synthetic-settings-class
/// pattern), then evaluate against the deployment's effective settings.
/// A violated `require` or satisfied `prohibit` is an error.
fn check_policy(
    policy: &DeployPolicy,
    application: &DeployApplication,
    deployment: &dsl::DeployDeploymentDecl,
    effective: &BTreeMap<String, BTreeMap<String, DeployValue>>,
    local: &mut Vec<Diagnostic>,
) {
    let parsed = rex_expr::parse(&policy.expression);
    for error in &parsed.errors {
        local.push(
            Diagnostic::error(
                format!("invalid policy condition: {}", error.message),
                Some(deployment.span),
            )
            .with_help(format!(
                "in `{} ({})`",
                if policy.prohibit {
                    "prohibit"
                } else {
                    "require"
                },
                policy.expression
            )),
        );
    }
    let Some(ast) = parsed.ast else {
        return;
    };

    // The synthetic settings universe: one class per component (features =
    // the kind's setting vocabulary), plus the application as a root class
    // whose features are the components. `api.replicas` is then plain
    // feature navigation from `self` — no fake `Model` assembled.
    let mut types = rex_expr::DomainTypes::default();
    for component in &application.components {
        let features = component_settings(component.kind)
            .into_iter()
            .map(|(name, spec)| {
                rex_ir::Feature::new(
                    name,
                    rex_ir::FeatureKind::Attribute,
                    match spec {
                        SettingSpec::Int => rex_ir::TypeRef::Primitive(rex_ir::PrimitiveType::Int),
                        SettingSpec::Text | SettingSpec::Word(_) => {
                            rex_ir::TypeRef::Primitive(rex_ir::PrimitiveType::String)
                        }
                    },
                    rex_ir::Multiplicity::REQUIRED,
                )
            })
            .collect();
        types.insert_class("", &component.name, Vec::new(), features, Vec::new());
    }
    let root_features = application
        .components
        .iter()
        .map(|component| {
            rex_ir::Feature::new(
                component.name.clone(),
                rex_ir::FeatureKind::Attribute,
                rex_ir::TypeRef::Class {
                    package: String::new(),
                    name: component.name.clone(),
                },
                rex_ir::Multiplicity::REQUIRED,
            )
        })
        .collect();
    types.insert_class("", &application.name, Vec::new(), root_features, Vec::new());

    let checker = rex_expr::TypeChecker::new(types).with_self("", &application.name);
    match checker.type_of(&ast) {
        Ok(ty) if ty == rex_expr::Ty::boolean() => {}
        Ok(ty) => {
            local.push(
                Diagnostic::error(
                    format!("policy condition must be boolean, found {ty}"),
                    Some(deployment.span),
                )
                .with_help(format!(
                    "in `{} ({})`",
                    if policy.prohibit {
                        "prohibit"
                    } else {
                        "require"
                    },
                    policy.expression
                )),
            );
            return;
        }
        Err(errors) => {
            for error in errors {
                local.push(
                    Diagnostic::error(
                        format!("invalid policy condition: {}", error.message),
                        Some(deployment.span),
                    )
                    .with_help(format!(
                        "in `{} ({})`",
                        if policy.prohibit {
                            "prohibit"
                        } else {
                            "require"
                        },
                        policy.expression
                    )),
                );
            }
            return;
        }
    }

    let value = match evaluate(&ast, effective) {
        Ok(value) => value,
        Err(message) => {
            local.push(
                Diagnostic::error(
                    format!("cannot evaluate policy condition: {message}"),
                    Some(deployment.span),
                )
                .with_help(format!(
                    "in `{} ({})`",
                    if policy.prohibit {
                        "prohibit"
                    } else {
                        "require"
                    },
                    policy.expression
                )),
            );
            return;
        }
    };
    let EvalValue::Bool(holds) = value else {
        local.push(
            Diagnostic::error(
                "policy condition did not evaluate to a boolean",
                Some(deployment.span),
            )
            .with_help("policies are boolean conditions over the effective settings"),
        );
        return;
    };
    let violation = match (policy.prohibit, holds) {
        (false, false) => Some("require"),
        (true, true) => Some("prohibit"),
        _ => None,
    };
    if let Some(keyword) = violation {
        local.push(
            Diagnostic::error(
                format!(
                    "policy violated in deployment '{}': {} ({})",
                    deployment.name.text, keyword, policy.expression
                ),
                Some(deployment.span),
            )
            .with_help(match keyword {
                "require" => "overrides and defaults must satisfy every `require` of the profile",
                _ => "overrides and defaults must not satisfy any `prohibit` of the profile",
            }),
        );
    }
}

/// A policy evaluation value (settings carry no floats and no options).
enum EvalValue {
    Int(i64),
    Text(String),
    Bool(bool),
}

/// The constant evaluator for policy conditions: literals, `component.setting`
/// paths, comparisons, boolean logic, and integer arithmetic with the
/// expression rules R1 (checked arithmetic) and R4 (no zero divisor). The
/// evaluated settings come from the deployment's effective settings map.
fn evaluate(
    expr: &rex_expr::Expr,
    effective: &BTreeMap<String, BTreeMap<String, DeployValue>>,
) -> Result<EvalValue, String> {
    match &expr.kind {
        rex_expr::ExprKind::Int(value) => Ok(EvalValue::Int(*value)),
        rex_expr::ExprKind::String(text) => Ok(EvalValue::Text(text.clone())),
        rex_expr::ExprKind::Bool(value) => Ok(EvalValue::Bool(*value)),
        rex_expr::ExprKind::Name(name) => Err(format!(
            "component '{name}' is used bare — reference its settings as '{name}.<setting>'"
        )),
        rex_expr::ExprKind::FeatureAccess {
            receiver,
            name,
            optional_safe: false,
        } => {
            let rex_expr::ExprKind::Name(component) = &receiver.kind else {
                return Err(
                    "only `component.setting` paths are supported, found a nested receiver"
                        .to_string(),
                );
            };
            let settings = effective.get(component).ok_or_else(|| {
                format!("unknown component '{component}' in application settings")
            })?;
            let value = settings.get(&name.value).ok_or_else(|| {
                format!(
                    "component '{component}' has no effective setting '{}'",
                    name.value
                )
            })?;
            Ok(match value {
                DeployValue::Int(value) => EvalValue::Int(*value),
                DeployValue::Text(text) => EvalValue::Text(text.clone()),
                DeployValue::Word(word) => EvalValue::Text(word.clone()),
            })
        }
        rex_expr::ExprKind::Binary { op, lhs, rhs } => {
            let lhs = evaluate(lhs, effective)?;
            let rhs = evaluate(rhs, effective)?;
            evaluate_binary(*op, lhs, rhs)
        }
        rex_expr::ExprKind::Unary { op, expr: inner } => {
            let inner = evaluate(inner, effective)?;
            match (*op, inner) {
                (rex_expr::UnOp::Not, EvalValue::Bool(value)) => Ok(EvalValue::Bool(!value)),
                (rex_expr::UnOp::Neg, EvalValue::Int(value)) => {
                    let negated = value.checked_neg();
                    negated
                        .map(EvalValue::Int)
                        .ok_or_else(|| "integer overflow in policy condition (rule R1)".to_string())
                }
                (op, _) => Err(format!("operator `{op}` applied to a non-matching value")),
            }
        }
        _ => Err(
            "only literals, `component.setting` paths, comparisons, boolean logic, and \
             integer arithmetic are supported in policy conditions"
                .to_string(),
        ),
    }
}

/// Evaluates one binary operation with the expression rules R1/R4.
fn evaluate_binary(
    op: rex_expr::BinOp,
    lhs: EvalValue,
    rhs: EvalValue,
) -> Result<EvalValue, String> {
    use rex_expr::BinOp;
    match op {
        BinOp::And | BinOp::Or => match (lhs, rhs) {
            (EvalValue::Bool(lhs), EvalValue::Bool(rhs)) => {
                Ok(EvalValue::Bool(if op == BinOp::And {
                    lhs && rhs
                } else {
                    lhs || rhs
                }))
            }
            _ => Err(format!("operator `{op}` expects boolean operands")),
        },
        BinOp::Eq | BinOp::Ne => {
            let equal = match (&lhs, &rhs) {
                (EvalValue::Int(lhs), EvalValue::Int(rhs)) => lhs == rhs,
                (EvalValue::Text(lhs), EvalValue::Text(rhs)) => lhs == rhs,
                (EvalValue::Bool(lhs), EvalValue::Bool(rhs)) => lhs == rhs,
                _ => return Err(format!("operator `{op}` compared mismatched value kinds")),
            };
            Ok(EvalValue::Bool(if op == BinOp::Eq {
                equal
            } else {
                !equal
            }))
        }
        BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge => match (lhs, rhs) {
            (EvalValue::Int(lhs), EvalValue::Int(rhs)) => Ok(EvalValue::Bool(match op {
                BinOp::Lt => lhs < rhs,
                BinOp::Le => lhs <= rhs,
                BinOp::Gt => lhs > rhs,
                _ => lhs >= rhs,
            })),
            _ => Err(format!("operator `{op}` expects integer operands")),
        },
        BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div => match (lhs, rhs) {
            (EvalValue::Int(lhs), EvalValue::Int(rhs)) => {
                let value = match op {
                    BinOp::Add => lhs.checked_add(rhs),
                    BinOp::Sub => lhs.checked_sub(rhs),
                    BinOp::Mul => lhs.checked_mul(rhs),
                    _ => {
                        if rhs == 0 {
                            return Err("division by zero (rule R4)".to_string());
                        }
                        lhs.checked_div(rhs)
                    }
                };
                value
                    .map(EvalValue::Int)
                    .ok_or_else(|| "integer overflow in policy condition (rule R1)".to_string())
            }
            _ => Err(format!("operator `{op}` expects integer operands")),
        },
    }
}
