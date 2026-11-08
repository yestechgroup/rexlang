//! The Cedar JSON schema emitter: one entity type per actor and per
//! capability class, one action per capability with its reachable
//! principals.
use std::collections::BTreeSet;

use super::mapping::{capability_class, primitive_cedar_type};
use super::reach::reaches_capability;
use rex_backend_common::canonical_key_order;
use rex_ir::{ActorsDef, Model};

/// Builds the Cedar JSON schema for one actors block.
pub(super) fn schema_json(actors: &ActorsDef, model: &Model) -> anyhow::Result<String> {
    let namespace = &actors.name;
    let mut entity_types = serde_json::Map::new();
    for actor in &actors.actors {
        let mut definition = serde_json::Map::new();
        if let Some(parent) = &actor.extends {
            definition.insert(
                "memberOfTypes".to_string(),
                serde_json::json!([format!("{namespace}::{parent}")]),
            );
        }
        entity_types.insert(actor.name.clone(), serde_json::Value::Object(definition));
    }
    let mut seen_classes = BTreeSet::new();
    for capability in &actors.capabilities {
        let class = capability_class(actors, model, &capability.name)?;
        if !seen_classes.insert(class.name.clone()) {
            continue;
        }
        let mut attributes = serde_json::Map::new();
        for feature in &class.features {
            if let Some(cedar_type) = primitive_cedar_type(feature) {
                attributes.insert(
                    feature.name.clone(),
                    serde_json::json!({"type": cedar_type}),
                );
            }
        }
        entity_types.insert(
            class.name.clone(),
            serde_json::json!({
                "shape": {
                    "type": "Record",
                    "attributes": attributes,
                }
            }),
        );
    }
    let mut actions = serde_json::Map::new();
    for capability in &actors.capabilities {
        let class = capability_class(actors, model, &capability.name)?;
        let principals: Vec<String> = actors
            .actors
            .iter()
            .filter(|actor| reaches_capability(actors, &actor.name, &capability.name))
            .map(|actor| format!("{namespace}::{}", actor.name))
            .collect();
        actions.insert(
            capability.name.clone(),
            serde_json::json!({
                "appliesTo": {
                    "principalTypes": principals,
                    "resourceTypes": [format!("{namespace}::{}", class.name)],
                    "context": {
                        "type": "Record",
                        "attributes": {},
                    },
                }
            }),
        );
    }
    let mut namespace_document = serde_json::Map::new();
    namespace_document.insert(
        "entityTypes".to_string(),
        serde_json::Value::Object(entity_types),
    );
    namespace_document.insert("actions".to_string(), serde_json::Value::Object(actions));
    let mut root = serde_json::Map::new();
    root.insert(
        namespace.clone(),
        serde_json::Value::Object(namespace_document),
    );
    Ok(serde_json::to_string_pretty(&canonical_key_order(
        serde_json::Value::Object(root),
    ))?)
}
