//! Integration tests for the per-agent tool manifests projected from an
//! [`rex_ir::ActorModel`] (`rex_driver::manifest`).

use rex_driver::manifest::{agent_tool_manifests, AgentToolManifest, ToolManifestDocument};
use rex_ir::{
    ActorDef, ActorKind, ActorModel, ActorsDef, CapabilityDef, DelegationDef, GrantDef, GrantEntry,
    TypeRef,
};

fn ticket_ref(package: &str) -> TypeRef {
    TypeRef::Class {
        package: package.to_string(),
        name: "Ticket".to_string(),
    }
}

fn block(name: &str) -> ActorsDef {
    ActorsDef::new(name)
}

fn tools_of(manifests: &[AgentToolManifest], agent: &str) -> Vec<String> {
    manifests
        .iter()
        .find(|manifest| manifest.agent == agent)
        .unwrap_or_else(|| panic!("no manifest for agent '{agent}'"))
        .tools
        .clone()
}

#[test]
fn agent_inherits_ancestor_permits_nearest_first_with_dedup() {
    let model = ActorModel::new().block(
        block("Support")
            .actor(ActorDef::new("Customer"))
            .actor(
                ActorDef::new("Agent")
                    .extends("Customer")
                    .kind(ActorKind::Agent),
            )
            .capability(CapabilityDef::new("ReadTicket", ticket_ref("s")))
            .capability(CapabilityDef::new("RaiseRefund", ticket_ref("s")))
            .grant(GrantDef::new("Agent").entry(GrantEntry::permit("RaiseRefund")))
            .grant(GrantDef::new("Customer").entry(GrantEntry::permit("ReadTicket"))),
    );
    let manifests = agent_tool_manifests(&model);
    assert_eq!(manifests.len(), 1);
    assert_eq!(
        tools_of(&manifests, "Agent"),
        ["RaiseRefund", "ReadTicket"],
        "self grants come before inherited ones (nearest first)"
    );
}

#[test]
fn only_agent_actors_get_manifests() {
    let model = ActorModel::new().block(
        block("Support")
            .actor(ActorDef::new("Customer"))
            .actor(ActorDef::new("Agent").kind(ActorKind::Agent))
            .actor(ActorDef::new("Auditor").kind(ActorKind::Human)),
    );
    let manifests = agent_tool_manifests(&model);
    assert_eq!(
        manifests
            .iter()
            .map(|m| m.agent.as_str())
            .collect::<Vec<_>>(),
        ["Agent"],
        "human and undeclared-kind actors are omitted"
    );
}

#[test]
fn kind_is_inherited_through_the_ancestor_chain() {
    let model = ActorModel::new().block(
        block("Support")
            .actor(ActorDef::new("Root").kind(ActorKind::Agent))
            .actor(ActorDef::new("Middle").extends("Root"))
            .actor(ActorDef::new("Leaf").extends("Middle")),
    );
    let manifests = agent_tool_manifests(&model);
    assert_eq!(
        manifests
            .iter()
            .map(|m| m.agent.as_str())
            .collect::<Vec<_>>(),
        ["Root", "Middle", "Leaf"],
        "a kind declared on the nearest ancestor flows down the whole chain"
    );
}

#[test]
fn cross_block_same_named_actors_pool_grants_and_delegations() {
    let model = ActorModel::new()
        .block(
            block("First")
                .actor(ActorDef::new("Agent").kind(ActorKind::Agent))
                .capability(CapabilityDef::new("ReadTicket", ticket_ref("f")))
                .grant(GrantDef::new("Agent").entry(GrantEntry::permit("ReadTicket"))),
        )
        .block(
            block("Second")
                .actor(ActorDef::new("Agent"))
                .capability(CapabilityDef::new("RaiseRefund", ticket_ref("s")))
                .grant(GrantDef::new("Agent").entry(GrantEntry::permit("RaiseRefund")))
                .delegation(
                    DelegationDef::new("FromSecond", "Agent", "Agent")
                        .purpose("Triage")
                        .entry(GrantEntry::permit("ReadTicket")),
                ),
        );
    let manifests = agent_tool_manifests(&model);
    assert_eq!(manifests.len(), 1, "one manifest per pooled actor name");
    assert_eq!(
        tools_of(&manifests, "Agent"),
        ["ReadTicket", "RaiseRefund"],
        "permits pool across all blocks, first block first"
    );
    let delegations = &manifests[0].delegations;
    assert_eq!(delegations.len(), 1);
    assert_eq!(delegations[0].name, "FromSecond");
    assert_eq!(delegations[0].purpose.as_deref(), Some("Triage"));
    assert_eq!(
        delegations[0]
            .entries
            .iter()
            .map(|entry| entry.capability.as_str())
            .collect::<Vec<_>>(),
        ["ReadTicket"]
    );
}

#[test]
fn forbid_and_cedar_entries_never_become_tools() {
    let model = ActorModel::new().block(
        block("Support")
            .actor(ActorDef::new("Agent").kind(ActorKind::Agent))
            .capability(CapabilityDef::new("ReadTicket", ticket_ref("s")))
            .capability(CapabilityDef::new("RaiseRefund", ticket_ref("s")))
            .capability(CapabilityDef::new("ArchiveTicket", ticket_ref("s")))
            .grant(
                GrantDef::new("Agent")
                    .entry(GrantEntry::permit("ReadTicket"))
                    .entry(GrantEntry::forbid("ArchiveTicket"))
                    .entry(
                        GrantEntry::permit("RaiseRefund")
                            .when("amount <= 1000")
                            .cedar("permit(principal, action);"),
                    ),
            ),
    );
    let manifests = agent_tool_manifests(&model);
    assert_eq!(
        tools_of(&manifests, "Agent"),
        ["ReadTicket"],
        "forbid entries and cedar-backed entries are excluded, even when they carry a permit effect"
    );
}

#[test]
fn delegation_entries_keep_permit_only_with_when_and_obligations() {
    let model = ActorModel::new().block(
        block("Support")
            .actor(ActorDef::new("Agent").kind(ActorKind::Agent))
            .capability(CapabilityDef::new("ReadTicket", ticket_ref("s")))
            .capability(CapabilityDef::new("ArchiveTicket", ticket_ref("s")))
            .delegation(
                DelegationDef::new("Intake", "Customer", "Agent")
                    .purpose("RefundTriage")
                    .entry(
                        GrantEntry::permit("ReadTicket")
                            .when("!internal")
                            .obligation("ack"),
                    )
                    .entry(GrantEntry::forbid("ArchiveTicket").obligation("log")),
            ),
    );
    let manifests = agent_tool_manifests(&model);
    let delegations = &manifests[0].delegations;
    assert_eq!(delegations.len(), 1);
    assert_eq!(delegations[0].name, "Intake");
    assert_eq!(delegations[0].purpose.as_deref(), Some("RefundTriage"));
    assert_eq!(delegations[0].entries.len(), 1, "only permits surface");
    let entry = &delegations[0].entries[0];
    assert_eq!(entry.capability, "ReadTicket");
    assert_eq!(entry.when.as_deref(), Some("!internal"));
    assert_eq!(entry.obligations, ["ack"]);
}

#[test]
fn delegation_without_permit_entries_still_appears_empty() {
    let model = ActorModel::new().block(
        block("Support")
            .actor(ActorDef::new("Agent").kind(ActorKind::Agent))
            .delegation(
                DelegationDef::new("EmptyIntake", "Customer", "Agent")
                    .entry(GrantEntry::forbid("ArchiveTicket")),
            ),
    );
    let manifests = agent_tool_manifests(&model);
    assert_eq!(manifests.len(), 1);
    assert_eq!(manifests[0].tools, Vec::<String>::new());
    assert_eq!(manifests[0].delegations.len(), 1);
    assert_eq!(manifests[0].delegations[0].name, "EmptyIntake");
    assert!(manifests[0].delegations[0].entries.is_empty());
}

#[test]
fn manifests_follow_actor_declaration_order() {
    let model = ActorModel::new().block(
        block("Support")
            .actor(ActorDef::new("Human"))
            .actor(ActorDef::new("Beta").kind(ActorKind::Agent))
            .actor(ActorDef::new("Alpha").kind(ActorKind::Agent))
            .actor(ActorDef::new("Human2")),
    );
    let manifests = agent_tool_manifests(&model);
    assert_eq!(
        manifests
            .iter()
            .map(|m| m.agent.as_str())
            .collect::<Vec<_>>(),
        ["Beta", "Alpha"],
        "declaration order, first occurrence, humans skipped"
    );
}

#[test]
fn empty_model_yields_no_manifests() {
    assert!(agent_tool_manifests(&ActorModel::new()).is_empty());
    let only_humans = ActorModel::new().block(block("Support").actor(ActorDef::new("Customer")));
    assert!(agent_tool_manifests(&only_humans).is_empty());
}

#[test]
fn cyclic_extends_graph_does_not_loop_or_panic() {
    let model = ActorModel::new().block(
        block("Broken")
            .actor(ActorDef::new("A").extends("B").kind(ActorKind::Agent))
            .actor(ActorDef::new("B").extends("A"))
            .capability(CapabilityDef::new("ReadTicket", ticket_ref("s")))
            .grant(GrantDef::new("A").entry(GrantEntry::permit("ReadTicket")))
            .grant(GrantDef::new("B").entry(GrantEntry::forbid("ReadTicket"))),
    );
    let manifests = agent_tool_manifests(&model);
    assert_eq!(
        manifests.len(),
        2,
        "B inherits A's agent kind even in a cycle"
    );
    assert_eq!(
        tools_of(&manifests, "A"),
        ["ReadTicket"],
        "the lineage walk visits each member once, then stops"
    );
    assert_eq!(
        tools_of(&manifests, "B"),
        ["ReadTicket"],
        "B's walk reaches A across the cycle edge, then stops"
    );
}

#[test]
fn json_document_is_pinned_field_for_field() {
    let model = ActorModel::new().block(
        block("Support")
            .actor(ActorDef::new("Customer"))
            .actor(
                ActorDef::new("Agent")
                    .extends("Customer")
                    .kind(ActorKind::Agent),
            )
            .capability(CapabilityDef::new("ReadTicket", ticket_ref("s")))
            .grant(GrantDef::new("Customer").entry(GrantEntry::permit("ReadTicket")))
            .delegation(
                DelegationDef::new("RefundIntake", "Customer", "Agent")
                    .purpose("RefundTriage")
                    .entry(
                        GrantEntry::permit("ReadTicket")
                            .when("!internal")
                            .obligation("ack"),
                    )
                    .entry(GrantEntry::forbid("ArchiveTicket")),
            ),
    );
    let document = ToolManifestDocument {
        agents: agent_tool_manifests(&model),
    };
    let json = document.to_json_pretty().expect("serialize manifests");
    assert_eq!(
        json,
        r#"{
  "agents": [
    {
      "agent": "Agent",
      "tools": [
        "ReadTicket"
      ],
      "delegations": [
        {
          "name": "RefundIntake",
          "purpose": "RefundTriage",
          "entries": [
            {
              "capability": "ReadTicket",
              "when": "!internal",
              "obligations": [
                "ack"
              ]
            }
          ]
        }
      ]
    }
  ]
}"#
    );
}
