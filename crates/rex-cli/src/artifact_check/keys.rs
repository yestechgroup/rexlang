//! The structural-key scan: the sorted camelCase vocabulary and the
//! snake_case shadow detector.

use serde_json::Value;

/// `packages`/`blocks` sanity: an array of objects each carrying a string
/// `name`.
pub(super) fn check_named_array(value: &Value, key: &str, errors: &mut Vec<String>) {
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

/// Every structural (non model-defined) key of the IR, actor, DDD design,
/// event, deployment, and use-case wire formats, from the `rex-ir` serde
/// shapes — the camelCase spellings a snake_case key is compared against.
/// Sorted for binary search.
const STRUCTURAL_KEYS: &[&str] = &[
    "actors",
    "anchor",
    "annotations",
    "applicabilityRules",
    "application",
    "authorizedRoles",
    "autoComplete",
    "base",
    "blocks",
    "bodies",
    "builtin",
    "cache",
    "capabilities",
    "capability",
    "caseFile",
    "cases",
    "cedar",
    "channels",
    "children",
    "class",
    "classes",
    "componentType",
    "composition",
    "constraints",
    "consumer",
    "context",
    "convert",
    "create",
    "datatypes",
    "default",
    "definitionRef",
    "delegations",
    "dependencies",
    "description",
    "designs",
    "detailFields",
    "details",
    "effect",
    "end",
    "entries",
    "enums",
    "eventType",
    "events",
    "extends",
    "facets",
    "features",
    "finite",
    "first",
    "flags",
    "format",
    "formatVersion",
    "from",
    "functions",
    "grants",
    "hierarchyField",
    "id",
    "ifPart",
    "inputParams",
    "interfaces",
    "isDefault",
    "isDerived",
    "isId",
    "isLandmark",
    "isModal",
    "isReadOnly",
    "isXor",
    "key",
    "kind",
    "label",
    "labelField",
    "labelFields",
    "literals",
    "lower",
    "manualActivation",
    "manualActivationRule",
    "maxDepth",
    "maxLength",
    "maximum",
    "message",
    "messages",
    "minLength",
    "minimum",
    "moduleUses",
    "modules",
    "multiplicity",
    "name",
    "neverBoth",
    "nonPersistent",
    "objects",
    "obligations",
    "occurrences",
    "onParts",
    "operation",
    "operations",
    "opposite",
    "optimisticLocking",
    "outputParams",
    "packages",
    "params",
    "participant",
    "pattern",
    "payload",
    "planItems",
    "planningMode",
    "planningTables",
    "platform",
    "processRef",
    "publishes",
    "purposes",
    "realizes",
    "repetitionRule",
    "repository",
    "requiredRole",
    "requiredRule",
    "requires",
    "returnMultiplicity",
    "returnType",
    "rexVersion",
    "schemaName",
    "sentryRef",
    "services",
    "source",
    "steps",
    "stereotype",
    "subcommands",
    "subject",
    "subscriptions",
    "successions",
    "target",
    "targetBindings",
    "taskType",
    "then",
    "to",
    "type",
    "typeName",
    "unique",
    "upper",
    "useCases",
    "value",
    "valueFields",
    "version",
    "vocabularies",
    "when",
];

/// Reports every key whose spelling is snake_case but whose camelCase form
/// is a known structural key — the readers ignore unknown fields (forward
/// compatibility), so such artifacts load but silently drop the value.
///
/// Runs on IR, actor, DDD design, event, deployment, and use-case artifacts
/// only: their keys are entirely structural. Instance objects' feature keys
/// are model-defined names, so instance documents are exempt.
pub(super) fn scan_snake_case_keys(value: &Value, errors: &mut Vec<String>) {
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
