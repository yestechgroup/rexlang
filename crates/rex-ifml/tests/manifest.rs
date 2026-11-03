//! The control/route manifest emitter: compiling canonical sources must
//! produce the declaration-backed manifest — routes, controls (expanded,
//! with resolved data entities), edges, actions, and commands.
//!
//! These tests pin the derivation contract of `rex_ir::ifml_manifest`;
//! `docs/IFML.md` documents it for consumers.

use rex_ir::ifml::{ComponentType, EventType};
use rex_ir::ifml_manifest::{IfmlManifest, ManifestEdgeKind, ManifestEdgeSource};

use rex_driver::{compile_str, DomainImports};
use rex_ifml::{compile_ifml_str, manifest_ifml, IfmlCompilation, IfmlImports};

const SHOP_MOX: &str = r#"
package shop

class Customer {
    String id
    String name
    String status
}

class Order {
    String id
    int total
}
"#;

/// Compiles `source` as `app.ifml` and emits its manifest. `domains`
/// mirrors the CLI: an imported `.mox` union, or `None`.
fn manifest_of(source: &str, domains: bool) -> IfmlManifest {
    let compilation =
        compile_ifml_str("app.ifml", source, &IfmlImports::default()).expect("source must compile");
    let model = if domains { Some(shop_union()) } else { None };
    manifest_ifml(&compilation, &IfmlImports::default(), model.as_ref())
}

/// The domain union a `.mox` import lowers to.
fn shop_union() -> rex_ir::Model {
    let compilation = compile_str("shop.mox", SHOP_MOX, &DomainImports::default());
    assert!(
        compilation.diagnostics.is_empty(),
        "the shop domain must compile: {:?}",
        compilation.diagnostics
    );
    compilation.model.expect("a clean compile lowers a model")
}

fn edges<'a>(
    manifest: &'a IfmlManifest,
    view: &str,
    component: Option<&str>,
) -> Vec<&'a rex_ir::ifml_manifest::ManifestEdge> {
    manifest
        .edges
        .iter()
        .filter(|edge| match &edge.source {
            ManifestEdgeSource::View {
                view: v,
                component: c,
            } => v == view && c.as_deref() == component,
            ManifestEdgeSource::Command { .. } => false,
        })
        .collect()
}

#[test]
fn empty_source_yields_the_minimal_manifest() {
    let manifest = manifest_of("", false);
    assert_eq!(manifest, IfmlManifest::default());
    assert_eq!(
        manifest.to_json().expect("serialize empty"),
        r#"{"formatVersion":1}"#
    );
}

#[test]
fn routes_carry_flags_params_and_gates() {
    let manifest = manifest_of(
        r#"
view "CustomerList" {
    params { slug: String = "home" };
    label "Customers";
    landmark: true;
    modal: true;
    roles: [admin];
    requires: [ViewCustomers];
    if status == "active";
}

view "Detail" {
    xor: true;
}
"#,
        false,
    );
    assert_eq!(manifest.views.len(), 2);
    let list = &manifest.views[0];
    assert_eq!(list.name, "CustomerList");
    assert_eq!(list.label.as_deref(), Some("Customers"));
    assert!(list.is_landmark);
    assert!(list.is_modal);
    assert!(!list.is_xor);
    assert_eq!(list.roles, vec!["admin".to_string()]);
    assert_eq!(list.requires, vec!["ViewCustomers".to_string()]);
    assert_eq!(list.params.len(), 1);
    assert_eq!(list.params[0].name, "slug");
    assert_eq!(list.params[0].type_ref, "String");
    assert!(list.params[0].default.is_some());
    assert!(list.condition.is_some());
    assert!(manifest.views[1].is_xor);
    assert!(!manifest.views[1].is_landmark);
}

#[test]
fn controls_flatten_the_expanded_tree_with_container_paths() {
    let manifest = manifest_of(
        r#"
module "Pager" {
    input { page: Int = 1 }
    output { total: Int }

    component "pager" {
        type: list;
        data: Item;
    }
}

view "Report" {
    component "grid" {
        type: list;
        data: Customer;
    }

    container "Summary" {
        default: true;

        component "revenue" {
            type: chart;
        }

        container "Deep" {
            component "rows" {
                type: table;
                paginated: true;
            }
        }
    }

    use "Pager" as pager { page: 2; };
}
"#,
        false,
    );
    let names: Vec<(&str, Vec<&str>)> = manifest
        .controls
        .iter()
        .map(|control| {
            (
                control.name.as_str(),
                control.containers.iter().map(String::as_str).collect(),
            )
        })
        .collect();
    assert_eq!(
        names,
        vec![
            ("grid", vec![]),
            ("pager", vec![]),
            ("revenue", vec!["Summary"]),
            ("rows", vec!["Summary", "Deep"]),
        ],
        "level components first, then containers depth-first; module uses spliced"
    );
    let pager = &manifest.controls[1];
    assert_eq!(
        pager.component_type.as_ref().map(ComponentType::as_str),
        Some("list"),
        "the spliced module control carries its type"
    );
    assert_eq!(pager.data.as_deref(), Some("Item"));
}

#[test]
fn controls_resolve_data_against_the_domain_union() {
    let source = r#"
view "List" {
    component "grid" {
        type: list;
        data: Customer;
    }
    component "generic" {
        type: list;
        data: Item;
    }
}
"#;
    let with_domains = manifest_of(source, true);
    assert_eq!(with_domains.controls.len(), 2);
    let resolved = with_domains.controls[0]
        .resolved_data
        .as_ref()
        .expect("Customer resolves uniquely");
    assert_eq!(
        (resolved.package.as_str(), resolved.name.as_str()),
        ("shop", "Customer")
    );
    assert!(
        with_domains.controls[1].resolved_data.is_none(),
        "the generic pattern name resolves to no class"
    );

    let without_domains = manifest_of(source, false);
    for control in &without_domains.controls {
        assert!(control.resolved_data.is_none());
        assert!(control.data.is_some(), "the bare spelling is always kept");
    }
}

#[test]
fn edges_carry_trigger_guard_bindings_and_target() {
    let manifest = manifest_of(
        r#"
view "List" {
    component "grid" {
        type: list;
        data: Customer;

        on select(row) requires: [OpenCustomer] if row.active -> navigate("Detail", {
            customerId: row.id,
            slug: "detail"
        });
        on reload -> refresh("grid");
        on create -> navigate("Detail");
    }
}

view "Detail" {
    component "form" {
        type: form;

        on save(values) -> action("UpdateCustomer", {
            body: values;
            on success -> navigate("List");
            on error -> stay;
        });
        on cancel -> stay;
    }
}

view "Home" {
    on load -> refresh("nothingHere");
}

action "UpdateCustomer" {
    target: Customer;
}
"#,
        false,
    );

    let select = &edges(&manifest, "List", Some("grid"))[0];
    assert_eq!(select.trigger, EventType::Select);
    assert_eq!(select.requires, vec!["OpenCustomer".to_string()]);
    assert!(select.condition.is_some());
    assert_eq!(select.kind, ManifestEdgeKind::Navigate);
    assert_eq!(select.target, "Detail");
    assert_eq!(select.bindings.len(), 2);
    assert_eq!(select.bindings[0].0, "customerId");

    let grid_edges = edges(&manifest, "List", Some("grid"));
    assert_eq!(grid_edges.len(), 3, "select, reload, create");
    assert_eq!(grid_edges[1].kind, ManifestEdgeKind::Refresh);
    assert_eq!(grid_edges[1].target, "grid");
    assert!(grid_edges[1].bindings.is_empty());
    assert_eq!(grid_edges[2].kind, ManifestEdgeKind::Navigate);
    assert_eq!(
        grid_edges[2].bindings.len(),
        0,
        "no binding block, no pairs"
    );

    // The invocation edge plus its nested success handler (the error
    // branch is `stay` — no edge); cancel is `stay` too.
    let form_edges = edges(&manifest, "Detail", Some("form"));
    assert_eq!(form_edges.len(), 2, "{form_edges:?}");
    assert_eq!(form_edges[0].kind, ManifestEdgeKind::Action);
    assert_eq!(form_edges[0].target, "UpdateCustomer");
    assert_eq!(form_edges[1].kind, ManifestEdgeKind::Navigate);
    assert_eq!(
        form_edges[1].trigger,
        EventType::Custom("success".to_string())
    );
    assert_eq!(
        form_edges[1].source,
        ManifestEdgeSource::View {
            view: "Detail".to_string(),
            component: Some("form".to_string())
        },
        "nested handlers stay on the invocation's source"
    );

    // View-level handlers carry no component.
    let home = edges(&manifest, "Home", None);
    assert_eq!(home.len(), 1);
    assert_eq!(home[0].kind, ManifestEdgeKind::Refresh);
}

#[test]
fn module_contributed_edges_join_the_manifest() {
    // The pattern module's handler is spliced into the view by the
    // expansion; its navigate target is declared by the app, so the edge
    // is declaration-backed like any app-authored one.
    let manifest = manifest_of(
        r#"
module "Inbox" {
    input { page: Int = 1 }
    output { total: Int }

    component "list" {
        type: list;

        on select(row) -> navigate("Folder", { id: row.id });
    }
}

view "Queue" {
    use "Inbox" as inbox { page: 1; };
}

view "Folder" {
    component "rows" { type: list; }
}
"#,
        false,
    );
    let queue = edges(&manifest, "Queue", Some("list"));
    assert_eq!(queue.len(), 1, "{queue:?}");
    assert_eq!(queue[0].kind, ManifestEdgeKind::Navigate);
    assert_eq!(queue[0].target, "Folder");
    assert_eq!(queue[0].bindings.len(), 1);
}

#[test]
fn actions_carry_target_and_resolution() {
    let source = r#"
action "UpdateCustomer" {
    target: Customer;
}

action "Ping" {}
"#;
    let with_domains = manifest_of(source, true);
    assert_eq!(with_domains.actions.len(), 2);
    let update = &with_domains.actions[0];
    assert_eq!(update.target.as_deref(), Some("Customer"));
    let resolved = update.resolved_target.as_ref().expect("Customer resolves");
    assert_eq!(
        (resolved.package.as_str(), resolved.name.as_str()),
        ("shop", "Customer")
    );
    assert!(with_domains.actions[1].target.is_none());
    assert!(with_domains.actions[1].resolved_target.is_none());
}

#[test]
fn commands_flatten_invocation_paths_and_edge_sources() {
    let manifest = manifest_of(
        r#"
command "tickets" {
    description: "Work the queue";
    requires: [Agent];
    option state: String = "open";

    on run -> action("ListTickets");

    command "show" {
        argument ticketNo: String;
        prompt confirm: confirm = false;

        on run -> action("GetTicket");
    }
}

action "ListTickets" {
    target: Customer;
}

action "GetTicket" {
    target: Customer;
}
"#,
        false,
    );
    assert_eq!(manifest.commands.len(), 2, "subcommands flatten");
    let root = &manifest.commands[0];
    assert_eq!(root.path, vec!["tickets".to_string()]);
    assert_eq!(root.description.as_deref(), Some("Work the queue"));
    assert_eq!(root.requires, vec!["Agent".to_string()]);
    assert_eq!(root.options.len(), 1);
    assert!(root.arguments.is_empty());
    assert!(root.prompts.is_empty());

    let show = &manifest.commands[1];
    assert_eq!(show.path, vec!["tickets".to_string(), "show".to_string()]);
    assert_eq!(show.arguments.len(), 1);
    assert_eq!(show.prompts.len(), 1);

    // Command handlers are edges sourced by the invocation path.
    let command_edges: Vec<_> = manifest
        .edges
        .iter()
        .filter(|edge| matches!(edge.source, ManifestEdgeSource::Command { .. }))
        .collect();
    assert_eq!(command_edges.len(), 2);
    assert_eq!(command_edges[0].kind, ManifestEdgeKind::Action);
    assert_eq!(command_edges[0].target, "ListTickets");
    assert_eq!(
        command_edges[0].source,
        ManifestEdgeSource::Command {
            path: vec!["tickets".to_string()]
        }
    );
    assert_eq!(command_edges[1].target, "GetTicket");
    assert_eq!(
        command_edges[1].source,
        ManifestEdgeSource::Command {
            path: vec!["tickets".to_string(), "show".to_string()]
        }
    );
}

#[test]
fn emission_is_deterministic() {
    let source = r#"
view "List" {
    component "grid" { type: list; data: Customer; }
    on load -> refresh("grid");
}
view "Detail" {
    params { id: Uuid };
}
action "UpdateCustomer" { target: Customer; }
command "tickets" { on run -> action("UpdateCustomer"); }
"#;
    let first = manifest_of(source, true)
        .to_json_pretty()
        .expect("serialize");
    let second = manifest_of(source, true)
        .to_json_pretty()
        .expect("serialize");
    assert_eq!(first, second, "same input, byte-identical manifest");
}

#[test]
fn the_manifest_never_exists_for_a_broken_model() {
    // The post-resolution contract: compilation reports the dangling
    // reference (E0104) before any manifest can be derived, so emission
    // is unreachable for the missing-button class.
    let source = r#"
view "List" {
    component "grid" {
        type: list;

        on select(row) -> action("Missing");
    }
}
"#;
    let compilation = compile_ifml_str("app.ifml", source, &IfmlImports::default())
        .expect("resolution succeeds; the checker reports the dangling action");
    let diags = rex_ifml::check_ifml(&compilation, None);
    assert_eq!(diags.len(), 1);
    assert_eq!(diags[0].code, "E0104");
}

#[test]
fn the_canonical_fixture_manifests() {
    // The canonical conformance models emit non-empty manifests end to
    // end (byte goldens live in tests/golden.rs).
    for (path, must_have_commands) in [
        ("../../tests/conformance/ifml/app.ifml", false),
        ("../../tests/conformance/ifml/cli.ifml", true),
    ] {
        let source =
            std::fs::read_to_string(path).unwrap_or_else(|error| panic!("read {path}: {error}"));
        let compilation = compile_ifml_str(path, &source, &IfmlImports::default())
            .unwrap_or_else(|diagnostics| panic!("{path} must compile: {diagnostics:?}"));
        let manifest = manifest_ifml(&compilation, &IfmlImports::default(), None);
        assert!(
            !manifest.views.is_empty() || !manifest.commands.is_empty(),
            "{path} must lower to something"
        );
        if must_have_commands {
            assert!(!manifest.commands.is_empty(), "{path} carries commands");
        }
    }
}

#[test]
fn a_compilation_helper_smoke() {
    let _: IfmlCompilation = compile_ifml_str("app.ifml", "", &IfmlImports::default()).unwrap();
}
