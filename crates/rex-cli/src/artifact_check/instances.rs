//! Instance-artifact integrity: `$id`/`$type`/`$ref` registration and
//! resolution over the instance's object graph.

use std::collections::BTreeSet;

use serde_json::Value;

use super::kinds::finish;
use super::ArtifactKind;
use super::SUPPORTED_INSTANCE_FORMAT_VERSION;

pub(super) fn check_instance(
    root: &serde_json::Map<String, Value>,
) -> Result<ArtifactKind, Vec<String>> {
    let mut errors = Vec::new();
    if let Some(error) = format_version_error(root, SUPPORTED_INSTANCE_FORMAT_VERSION, "instance") {
        errors.push(error);
    }
    match root.get("model") {
        None => errors.push("instance is missing \"model\"".to_string()),
        Some(model) if model.as_str().is_none_or(str::is_empty) => errors.push(format!(
            "instance \"model\" must be a non-empty string, found {model}"
        )),
        _ => {}
    }
    let Some(objects) = root.get("objects") else {
        errors.push("instance is missing \"objects\"".to_string());
        return finish(ArtifactKind::Instance, errors);
    };
    let Some(objects) = objects.as_array() else {
        errors.push(format!(
            "instance \"objects\" must be an array, found {objects}"
        ));
        return finish(ArtifactKind::Instance, errors);
    };
    for (index, object) in objects.iter().enumerate() {
        if !object.is_object() {
            errors.push(format!(
                "objects[{index}] must be an object, found {object}"
            ));
        }
    }
    let mut ids = BTreeSet::new();
    register_objects(objects, &mut ids, &mut errors);
    resolve_refs(objects, &ids, &mut errors);
    finish(ArtifactKind::Instance, errors)
}

/// The readers' version gate: only the exact supported version passes; a
/// different version (including a higher one) is rejected with an error
/// naming both versions, like [`rex_ir::IrError::UnsupportedFormatVersion`].
pub(super) fn format_version_error(
    root: &serde_json::Map<String, Value>,
    supported: u32,
    what: &str,
) -> Option<String> {
    match root.get("formatVersion") {
        None => Some(format!(
            "missing {what} formatVersion: this tool reads formatVersion {supported}"
        )),
        Some(found) => match found.as_u64() {
            Some(found) if found == u64::from(supported) => None,
            Some(found) => Some(format!(
                "unsupported {what} formatVersion {found}: this tool reads formatVersion \
                 {supported} — regenerate the artifact with a matching rexlang version"
            )),
            None => Some(format!(
                "{what} formatVersion must be an unsigned integer, found {found}"
            )),
        },
    }
}

/// Pass one over the instance object tree: register every `$id` (checking
/// well-formedness and uniqueness) and flag malformed objects.
fn register_objects(items: &[Value], ids: &mut BTreeSet<String>, errors: &mut Vec<String>) {
    for item in items {
        register_value(item, ids, errors);
    }
}

fn register_value(value: &Value, ids: &mut BTreeSet<String>, errors: &mut Vec<String>) {
    match value {
        Value::Array(items) => register_objects(items, ids, errors),
        Value::Object(map) => {
            let id = map.get("$id");
            if let Some(id) = id {
                match id.as_str() {
                    Some(id) => {
                        if let Some(problem) = malformed_id(id) {
                            errors.push(problem);
                        } else if !ids.insert(id.to_string()) {
                            errors.push(format!(
                                "duplicate $id {id:?}: object ids must be unique within \
                                 the document"
                            ));
                        }
                    }
                    None => errors.push(format!("object $id must be a string, found {id}")),
                }
                match map.get("$type") {
                    None => errors.push(format!("object with $id {id:?} is missing $type")),
                    Some(type_) if type_.as_str().is_none() => {
                        errors.push(format!("object $type must be a string, found {type_}"))
                    }
                    _ => {}
                }
            } else if let Some(type_) = map.get("$type") {
                errors.push(format!(
                    "object with $type {type_:?} has neither $id nor $ref: instance \
                     objects are either registered ($id) or links ($ref)"
                ));
            }
            if let Some(reference) = map.get("$ref") {
                let extra: Vec<&String> = map.keys().filter(|key| **key != "$ref").collect();
                if !extra.is_empty() {
                    errors.push(format!(
                        "a $ref object must contain only \"$ref\" (found {reference}), \
                         with keys {}",
                        quoted_list(&extra),
                    ));
                }
            }
            for child in map.values() {
                register_value(child, ids, errors);
            }
        }
        _ => {}
    }
}

/// Pass two: every `$ref` must resolve to an `$id` collected anywhere in the
/// document (refs may point forward or up through containment).
fn resolve_refs(items: &[Value], ids: &BTreeSet<String>, errors: &mut Vec<String>) {
    for item in items {
        resolve_value(item, ids, errors);
    }
}

fn resolve_value(value: &Value, ids: &BTreeSet<String>, errors: &mut Vec<String>) {
    match value {
        Value::Array(items) => resolve_refs(items, ids, errors),
        Value::Object(map) => {
            if let Some(target) = map
                .get("$ref")
                .and_then(Value::as_str)
                .filter(|target| !ids.contains(*target))
            {
                errors.push(format!(
                    "unresolved $ref {target:?}: no object in this document has that $id"
                ));
            }
            for child in map.values() {
                resolve_value(child, ids, errors);
            }
        }
        _ => {}
    }
}

/// Canonical object ids are `<class>/<ordinal>` — the shape the generated
/// loaders and sort keys rely on (`rsplit_once('/')` + numeric ordinal).
fn malformed_id(id: &str) -> Option<String> {
    let Some((prefix, ordinal)) = id.rsplit_once('/') else {
        return Some(format!(
            "object $id {id:?} is malformed: canonical ids are \"<class>/<ordinal>\""
        ));
    };
    if prefix.is_empty() {
        return Some(format!(
            "object $id {id:?} is malformed: the class prefix before '/' is empty"
        ));
    }
    if ordinal.is_empty() || ordinal.parse::<u64>().is_err() {
        return Some(format!(
            "object $id {id:?} is malformed: the ordinal after '/' must be a \
             non-negative integer"
        ));
    }
    None
}

fn quoted_list(keys: &[&String]) -> String {
    let mut out = String::new();
    for (index, key) in keys.iter().enumerate() {
        if index > 0 {
            out.push_str(", ");
        }
        out.push('"');
        out.push_str(key);
        out.push('"');
    }
    out
}
