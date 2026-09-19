//! Wire-format validation for serialized rexlang artifacts — the portable
//! test kit behind `rexlang artifact check`.
//!
//! Third-party backends consume [`rex_ir::Model`] / [`rex_ir::ActorModel`]
//! (or read the serialized artifact directly) and emit canonical instance
//! JSON. This module checks the *format* invariants of those documents
//! without needing the originating model:
//!
//! * the root shape matches a known artifact kind (IR artifact, standalone
//!   actor-policy artifact, or canonical instance),
//! * `formatVersion` is present and supported — a higher version is rejected
//!   with an error naming it, mirroring the readers' version gate
//!   ([`rex_ir::IrError::UnsupportedFormatVersion`]),
//! * keys on known objects are camelCase (snake_case spellings of known
//!   structural keys are called out with the correct spelling),
//! * instance documents are sane: `$id`s are well-formed (`<class>/<n>`)
//!   and unique, `$type` is present, and every `$ref` resolves within the
//!   document.
//!
//! The wire-format contract itself is normative in the `rex-ir` crate docs;
//! `docs/BACKENDS.md` is the backend-author guide.

use std::collections::BTreeSet;

use serde_json::Value;

/// The instance `formatVersion` this tool reads
/// ([`rex_runtime::INSTANCE_FORMAT_VERSION`]).
pub const SUPPORTED_INSTANCE_FORMAT_VERSION: u32 = rex_runtime::INSTANCE_FORMAT_VERSION;

/// The artifact kinds a document can be classified as.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArtifactKind {
    /// A domain IR artifact (`rex_ir::Model`): root carries `packages`.
    IrModel,
    /// A standalone actor-policy artifact (`rex_ir::ActorModel`): root
    /// carries `blocks` (omitted when empty).
    ActorModel,
    /// A canonical instance document: root `$type` is `rex.instance`.
    Instance,
}

/// Validates one artifact document, returning its kind or every violation
/// found. `path` prefixes each error line (the file the document came from).
pub fn check_str(path: &str, text: &str) -> Result<ArtifactKind, Vec<String>> {
    match check(text) {
        Ok(kind) => Ok(kind),
        Err(errors) => Err(errors
            .into_iter()
            .map(|error| format!("{path}: {error}"))
            .collect()),
    }
}

/// Validates one artifact document without path prefixing.
pub fn check(text: &str) -> Result<ArtifactKind, Vec<String>> {
    let value: Value = match serde_json::from_str(text) {
        Ok(value) => value,
        Err(error) => return Err(vec![format!("not valid JSON: {error}")]),
    };
    let Some(root) = value.as_object() else {
        return Err(vec!["artifact root must be a JSON object".to_string()]);
    };
    if let Some(tag) = root.get("$type") {
        return match tag.as_str() {
            Some(t) if t == rex_runtime::INSTANCE_TYPE => check_instance(root),
            Some(other) => Err(vec![format!(
                "unknown root $type {other:?}: instance documents declare $type {:?}; \
                 IR and actor artifacts carry no root $type",
                rex_runtime::INSTANCE_TYPE
            )]),
            None => Err(vec![format!("root $type must be a string, found {tag}")]),
        };
    }
    if root.contains_key("packages") || root.contains_key("rexVersion") {
        return check_ir_model(root);
    }
    // No `packages`/`rexVersion`: the canonical block-less ActorModel is
    // exactly `{"formatVersion": N}` — a Model always emits `packages`.
    check_actor_model(root)
}

fn check_ir_model(root: &serde_json::Map<String, Value>) -> Result<ArtifactKind, Vec<String>> {
    let mut errors = Vec::new();
    if let Some(error) = format_version_error(root, rex_ir::FORMAT_VERSION, "IR artifact") {
        errors.push(error);
    }
    if let Some(packages) = root.get("packages") {
        check_named_array(packages, "packages", &mut errors);
    }
    scan_snake_case_keys(&Value::Object(root.clone()), &mut errors);
    finish(ArtifactKind::IrModel, errors)
}

fn check_actor_model(root: &serde_json::Map<String, Value>) -> Result<ArtifactKind, Vec<String>> {
    let mut errors = Vec::new();
    if let Some(error) =
        format_version_error(root, rex_ir::ACTOR_MODEL_FORMAT_VERSION, "actor artifact")
    {
        errors.push(error);
    }
    if let Some(blocks) = root.get("blocks") {
        check_named_array(blocks, "blocks", &mut errors);
    }
    scan_snake_case_keys(&Value::Object(root.clone()), &mut errors);
    finish(ArtifactKind::ActorModel, errors)
}

fn check_instance(root: &serde_json::Map<String, Value>) -> Result<ArtifactKind, Vec<String>> {
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

fn finish(kind: ArtifactKind, errors: Vec<String>) -> Result<ArtifactKind, Vec<String>> {
    if errors.is_empty() {
        Ok(kind)
    } else {
        Err(errors)
    }
}

/// The readers' version gate: only the exact supported version passes; a
/// different version (including a higher one) is rejected with an error
/// naming both versions, like [`rex_ir::IrError::UnsupportedFormatVersion`].
fn format_version_error(
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

/// `packages`/`blocks` sanity: an array of objects each carrying a string
/// `name`.
fn check_named_array(value: &Value, key: &str, errors: &mut Vec<String>) {
    let Some(items) = value.as_array() else {
        errors.push(format!("{key:?} must be an array, found {value}"));
        return;
    };
    for (index, item) in items.iter().enumerate() {
        match item.as_object() {
            None => errors.push(format!("{key}[{index}] must be an object, found {item}")),
            Some(object) => match object.get("name") {
                None => errors.push(format!("{key}[{index}] is missing \"name\"")),
                Some(name) if name.as_str().is_none_or(str::is_empty) => errors.push(format!(
                    "{key}[{index}] \"name\" must be a non-empty string, found {name}"
                )),
                _ => {}
            },
        }
    }
}

/// Every structural (non model-defined) key of the IR and actor wire
/// formats, from the `rex-ir` serde shapes — the camelCase spellings a
/// snake_case key is compared against. Sorted for binary search.
const STRUCTURAL_KEYS: &[&str] = &[
    "actors",
    "annotations",
    "blocks",
    "bodies",
    "capabilities",
    "capability",
    "cedar",
    "class",
    "classes",
    "constraints",
    "convert",
    "create",
    "datatypes",
    "default",
    "delegations",
    "description",
    "details",
    "effect",
    "entries",
    "enums",
    "extends",
    "facets",
    "features",
    "finite",
    "format",
    "formatVersion",
    "from",
    "grants",
    "id",
    "interfaces",
    "isDerived",
    "isId",
    "isReadOnly",
    "key",
    "kind",
    "label",
    "literals",
    "lower",
    "maxLength",
    "maximum",
    "minLength",
    "minimum",
    "multiplicity",
    "name",
    "neverBoth",
    "objects",
    "obligations",
    "operations",
    "opposite",
    "packages",
    "params",
    "pattern",
    "platform",
    "purposes",
    "rexVersion",
    "returnType",
    "source",
    "targetBindings",
    "to",
    "type",
    "unique",
    "upper",
    "value",
    "version",
    "vocabularies",
    "when",
];

/// Reports every key whose spelling is snake_case but whose camelCase form
/// is a known structural key — the readers ignore unknown fields (forward
/// compatibility), so such artifacts load but silently drop the value.
///
/// Runs on IR and actor artifacts only: their keys are entirely structural.
/// Instance objects' feature keys are model-defined names, so instance
/// documents are exempt.
fn scan_snake_case_keys(value: &Value, errors: &mut Vec<String>) {
    match value {
        Value::Object(map) => {
            for (key, child) in map {
                if key.contains('_') {
                    let camel_case = camel_case(key);
                    if STRUCTURAL_KEYS.binary_search(&camel_case.as_str()).is_ok() {
                        errors.push(format!(
                            "key {key:?} is not wire format: the camelCase key is \
                             {camel_case:?}"
                        ));
                    }
                }
                scan_snake_case_keys(child, errors);
            }
        }
        Value::Array(items) => {
            for item in items {
                scan_snake_case_keys(item, errors);
            }
        }
        _ => {}
    }
}

/// `format_version` -> `formatVersion` (first segment lowercase, the rest
/// capitalized).
fn camel_case(key: &str) -> String {
    let mut out = String::with_capacity(key.len());
    for (index, segment) in key
        .split('_')
        .filter(|segment| !segment.is_empty())
        .enumerate()
    {
        let mut chars = segment.chars();
        if let Some(first) = chars.next() {
            if index == 0 {
                out.extend(first.to_lowercase());
            } else {
                out.extend(first.to_uppercase());
            }
            out.push_str(chars.as_str());
        }
    }
    out
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bare_format_version_classifies_as_block_less_actor_model() {
        // A Model always emits `packages`; the only artifact whose canonical
        // form is just the version marker is the empty ActorModel.
        assert_eq!(
            check(r#"{"formatVersion": 1}"#),
            Ok(ArtifactKind::ActorModel)
        );
    }

    #[test]
    fn packages_key_classifies_as_ir_model() {
        assert_eq!(
            check(r#"{"formatVersion": 1, "rexVersion": "0.1.0", "packages": []}"#),
            Ok(ArtifactKind::IrModel)
        );
    }

    #[test]
    fn format_version_gate_names_both_versions() {
        let errors = check(r#"{"formatVersion": 2, "packages": []}"#).unwrap_err();
        assert!(
            errors[0].contains("formatVersion 2") && errors[0].contains("formatVersion 1"),
            "errors: {errors:?}"
        );
    }

    #[test]
    fn snake_case_keys_are_reported_with_the_camel_case_spelling() {
        let errors =
            check(r#"{"formatVersion": 1, "format_version": 1, "packages": []}"#).unwrap_err();
        assert!(
            errors
                .iter()
                .any(|error| error.contains("format_version") && error.contains("formatVersion")),
            "errors: {errors:?}"
        );
    }

    #[test]
    fn instance_ids_must_be_well_formed_unique_and_resolved() {
        let errors = check(
            r#"{
                "$type": "rex.instance", "formatVersion": 1, "model": "m",
                "objects": [
                    {"$id": "book/0", "$type": "Book", "other": {"$ref": "writer/0"}}
                ]
            }"#,
        )
        .unwrap_err();
        assert!(
            errors
                .iter()
                .any(|error| error.contains("unresolved $ref \"writer/0\"")),
            "errors: {errors:?}"
        );

        let errors = check(
            r#"{
                "$type": "rex.instance", "formatVersion": 1, "model": "m",
                "objects": [
                    {"$id": "book/0", "$type": "Book"},
                    {"$id": "book/0", "$type": "Book"}
                ]
            }"#,
        )
        .unwrap_err();
        assert!(
            errors
                .iter()
                .any(|error| error.contains("duplicate $id \"book/0\"")),
            "errors: {errors:?}"
        );

        let errors = check(
            r#"{
                "$type": "rex.instance", "formatVersion": 1, "model": "m",
                "objects": [{"$id": "book", "$type": "Book"}]
            }"#,
        )
        .unwrap_err();
        assert!(
            errors.iter().any(|error| error.contains("malformed")),
            "errors: {errors:?}"
        );
    }

    #[test]
    fn unknown_root_type_is_rejected() {
        let errors = check(r#"{"$type": "something.else", "formatVersion": 1}"#).unwrap_err();
        assert!(
            errors[0].contains("something.else") && errors[0].contains("rex.instance"),
            "errors: {errors:?}"
        );
    }
}
