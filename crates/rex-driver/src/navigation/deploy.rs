//! Span index for `.deploy` deployment sources (issue rexlang#244).
//!
//! Definitions are the graph's deployment nodes — the deployment application,
//! its components, the reusable profiles, and the deployments selecting them
//! — named through [`crate::graph::ids`], so the span table and the node table
//! cannot drift. Every one of them lives in the file itself, so the builder
//! takes the source alone (no display path).
//!
//! References are the deployment model's cross-surface wiring. A `design` or
//! `flow` binding names an *import path* rather than a symbol: the index
//! records the module suffix when one is written (`"support.ddd#desk"` →
//! `desk`, the graph's `bindsDesign` attribute) and the path as written
//! otherwise (a binding to the imported file's whole application/flow, which
//! only a path match can bind). Everything else is a name mention inside the
//! file: `requires` names name capabilities, `connects` endpoints name
//! components, and a deployment's `for`/`use` clauses name an application and
//! a profile. Binding stays the graph index's job.

use rex_ir::graph::kinds;

use super::surface::{ReferenceContext, SurfaceIndex};
use crate::graph::ids;

/// Builds the span index for one `.deploy` source. A source that does not
/// parse yields an empty index; this never panics.
pub fn deploy_index(source: &str) -> SurfaceIndex {
    let mut index = SurfaceIndex::default();
    let Some(ast) = rex_syntax::parse_deploy(source).ast else {
        return index;
    };

    for application in &ast.applications {
        index.push_definition(
            kinds::DEPLOY_APPLICATION,
            application.name.text.clone(),
            application.name.span,
            application.span,
        );
        for component in &application.components {
            index.push_definition(
                kinds::DEPLOY_COMPONENT,
                ids::deploy_component_name(&application.name.text, &component.name.text),
                component.name.span,
                component.span,
            );
            for capability in &component.requires {
                index.push_reference(
                    &[kinds::CAPABILITY],
                    capability.text.clone(),
                    capability.span,
                    ReferenceContext::DeployRequires,
                );
            }
            for binding in &component.designs {
                binding_reference(&mut index, binding, ReferenceContext::DesignBinding);
            }
            for binding in &component.flows {
                binding_reference(&mut index, binding, ReferenceContext::FlowBinding);
            }
        }
        for connection in &application.connections {
            for end in [&connection.from, &connection.to] {
                index.push_reference(
                    &[kinds::DEPLOY_COMPONENT],
                    end.text.clone(),
                    end.span,
                    ReferenceContext::Connection,
                );
            }
        }
    }

    for profile in &ast.profiles {
        index.push_definition(
            kinds::DEPLOY_PROFILE,
            profile.name.text.clone(),
            profile.name.span,
            profile.span,
        );
    }

    for deployment in &ast.deployments {
        index.push_definition(
            kinds::DEPLOYMENT,
            ids::deployment_name(&deployment.application.text, &deployment.name.text),
            deployment.name.span,
            deployment.span,
        );
        index.push_reference(
            &[kinds::DEPLOY_APPLICATION],
            deployment.application.text.clone(),
            deployment.application.span,
            ReferenceContext::DeployFor,
        );
        // `use` is recovered as `None` (a reported syntax error) when the
        // source omitted it: nothing to index, not a nameless mention.
        if let Some(profile) = &deployment.profile {
            index.push_reference(
                &[kinds::DEPLOY_PROFILE],
                profile.text.clone(),
                profile.span,
                ReferenceContext::ProfileUse,
            );
        }
    }

    index.sort();
    index
}

/// Records one `design`/`flow` binding's mention.
///
/// The name is the `#<Module>` suffix when written (the graph's module
/// lookup is a `.ends_with(".<module>")` match, and a bare module name
/// last-segment-matches the module node), else the path as written — the
/// whole-application binding only a path match can resolve.
fn binding_reference(
    index: &mut SurfaceIndex,
    binding: &rex_syntax::ast::DeployBindingDecl,
    context: ReferenceContext,
) {
    let (path, module) = match binding.reference.rsplit_once('#') {
        Some((path, module)) => (path, Some(module)),
        None => (binding.reference.as_str(), None),
    };
    match (binding.is_design, module) {
        (true, Some(module)) => index.push_reference(
            &[kinds::MODULE, kinds::DESIGN_APPLICATION],
            module,
            binding.reference_span,
            context,
        ),
        (true, None) => index.push_reference(
            &[kinds::DESIGN_APPLICATION, kinds::MODULE],
            path,
            binding.reference_span,
            context,
        ),
        (false, Some(module)) => {
            index.push_reference(&[kinds::FLOW], module, binding.reference_span, context)
        }
        (false, None) => {
            index.push_reference(&[kinds::FLOW], path, binding.reference_span, context)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::navigation::test_support;

    const SUPPORT: &str = "examples/support.deploy";
    const FIXTURE: &str = "tests/conformance/graph/app.deploy";

    #[test]
    fn deploy_definitions_are_the_graph_nodes() {
        let index = deploy_index(&test_support::fixture(FIXTURE));
        test_support::assert_definitions_match_graph(
            &index,
            test_support::fixture_graph(),
            FIXTURE,
        );
    }

    #[test]
    fn design_binding_points_at_the_ddd_module() {
        let source = test_support::fixture(FIXTURE);
        let index = deploy_index(&source);
        let design = index
            .references
            .iter()
            .find(|reference| reference.context == ReferenceContext::DesignBinding)
            .expect("the api component's design binding");
        assert_eq!(
            design.target_kinds,
            &[kinds::MODULE, kinds::DESIGN_APPLICATION]
        );
        assert_eq!(design.name, "desk", "the `#Module` suffix is the mention");
        assert_eq!(&source[design.span.start..design.span.end], "app.ddd#desk");
        assert_eq!(
            index.references_to(kinds::MODULE, "GraphDesk.desk").len(),
            1,
            "the module suffix binds the design's module node"
        );
    }

    #[test]
    fn flow_binding_points_at_the_ifml_flow() {
        let source = test_support::fixture(FIXTURE);
        let index = deploy_index(&source);
        let flow = index
            .references
            .iter()
            .find(|reference| reference.context == ReferenceContext::FlowBinding)
            .expect("the web component's flow binding");
        assert_eq!(flow.target_kinds, &[kinds::FLOW]);
        assert_eq!(
            flow.name, "app.ifml",
            "no `#Module`, so the path as written"
        );
        assert_eq!(&source[flow.span.start..flow.span.end], "app.ifml");

        // The conformance deploy model binds a module of the flow too.
        let source = test_support::fixture("tests/conformance/deploy/svc.deploy");
        let index = deploy_index(&source);
        let module = index
            .references
            .iter()
            .find(|reference| {
                reference.context == ReferenceContext::FlowBinding && reference.name == "Pagination"
            })
            .expect("the web component's flow module binding");
        assert_eq!(
            &source[module.span.start..module.span.end],
            "../ifml/app.ifml#Pagination"
        );
    }

    #[test]
    fn component_requires_names_point_at_capabilities() {
        let source = test_support::fixture("tests/conformance/deploy/svc.deploy");
        let index = deploy_index(&source);
        let requires: Vec<_> = index
            .references
            .iter()
            .filter(|reference| reference.context == ReferenceContext::DeployRequires)
            .collect();
        assert_eq!(requires.len(), 1);
        assert_eq!(requires[0].name, "longRunningProcess");
        assert_eq!(requires[0].target_kinds, &[kinds::CAPABILITY]);
        assert_eq!(
            &source[requires[0].span.start..requires[0].span.end],
            "longRunningProcess"
        );
        assert_eq!(
            index
                .references_to(kinds::CAPABILITY, "Support.RaiseRefund")
                .len(),
            0,
            "a target capability does not bind a policy capability"
        );
    }

    #[test]
    fn connections_point_at_components() {
        let source = test_support::fixture(FIXTURE);
        let index = deploy_index(&source);
        let ends: Vec<_> = index
            .references
            .iter()
            .filter(|reference| reference.context == ReferenceContext::Connection)
            .collect();
        assert_eq!(ends.len(), 4, "two `connects` entries, two ends each");
        assert!(ends
            .iter()
            .all(|reference| reference.target_kinds == [kinds::DEPLOY_COMPONENT]));
        assert_eq!(ends[0].name, "web");
        assert_eq!(&source[ends[0].span.start..ends[0].span.end], "web");
        assert_eq!(
            index
                .references_to(kinds::DEPLOY_COMPONENT, "GraphDesk.web")
                .len(),
            1
        );
    }

    /// The reference application's deployment file: two profiles, two
    /// deployments, one application.
    #[test]
    fn the_support_deployment_indexes_its_profiles_and_deployments() {
        let source = test_support::fixture(SUPPORT);
        let index = deploy_index(&source);
        let profiles: Vec<_> = index
            .definitions
            .iter()
            .filter(|definition| definition.kind == kinds::DEPLOY_PROFILE)
            .collect();
        assert_eq!(profiles.len(), 2, "production_k8s and edge");
        let deployments: Vec<_> = index
            .definitions
            .iter()
            .filter(|definition| definition.kind == kinds::DEPLOYMENT)
            .collect();
        assert_eq!(deployments.len(), 2);
        assert!(deployments
            .iter()
            .any(|definition| definition.name == "SupportDesk.edgeLive"));
        assert_eq!(
            index
                .references_to(kinds::DEPLOY_PROFILE, "SupportDesk.edge")
                .len(),
            1,
            "the edgeLive deployment's `use edge`"
        );
    }

    #[test]
    fn deployment_clauses_point_at_the_application_and_profile() {
        let source = test_support::fixture(FIXTURE);
        let index = deploy_index(&source);
        let application = index
            .references
            .iter()
            .find(|reference| reference.context == ReferenceContext::DeployFor)
            .expect("the `for` clause");
        assert_eq!(application.name, "GraphDesk");
        assert_eq!(application.target_kinds, &[kinds::DEPLOY_APPLICATION]);
        assert_eq!(
            index
                .references_to(kinds::DEPLOY_APPLICATION, "GraphDesk")
                .len(),
            1
        );
        let profile = index
            .references
            .iter()
            .find(|reference| reference.context == ReferenceContext::ProfileUse)
            .expect("the `use` clause");
        assert_eq!(profile.name, "production_k8s");
        assert_eq!(profile.target_kinds, &[kinds::DEPLOY_PROFILE]);
        assert_eq!(
            &source[profile.span.start..profile.span.end],
            "production_k8s"
        );
    }

    #[test]
    fn broken_source_yields_an_empty_index_not_a_panic() {
        assert!(deploy_index("%%% not a deployment %%%").is_empty());
        assert!(deploy_index("").is_empty());
        // A deployment without its `use` line parses (a reported error): the
        // definition is still indexed, no nameless mention is.
        let index = deploy_index("deployment production for Desk {\n}\n");
        assert_eq!(
            index
                .definitions_of(kinds::DEPLOYMENT, "Desk.production")
                .len(),
            1
        );
        assert!(!index
            .references
            .iter()
            .any(|reference| reference.context == ReferenceContext::ProfileUse));
    }
}
