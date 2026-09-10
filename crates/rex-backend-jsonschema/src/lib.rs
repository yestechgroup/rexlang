//! rex-backend-jsonschema — JSON Schema generation from the Core IR.
//!
//! Consumes a resolved [`rex_ir::Model`] and emits JSON Schema draft
//! 2020-12 documents in one of two flavors ([`Profile`]):
//!
//! - [`Profile::Wire`] validates the **whole canonical instance document**
//!   produced by the Rust backend's `to_instance_json`: the
//!   `{"$type": "rex.instance", ...}` envelope, `"<singular>/<n>"` object
//!   ids, inlined containment, `{"$ref": ...}` cross references, and the
//!   strict omission of container and derived features. It is the machine
//!   counterpart of the cross-language instance contract.
//! - [`Profile::Api`] is a **flattened DTO projection** for OpenAPI-style
//!   consumers: per-class object schemas, cross references as plain id
//!   strings, derived features kept as `readOnly`, no document envelope.
//!
//! Both flavors encode `extends` as schema structure (an `allOf` on the
//! subclass, a `$type` enum on the base) and place the object closure
//! (`unevaluatedProperties`/`additionalProperties: false`) on the outermost
//! class only, so subclass instances carrying inherited properties validate.
//!
//! Output is deterministic: schemas are built with `serde_json::Map`
//! (alphabetical key order) and serialized with `to_string_pretty`, so the
//! golden fixtures are byte-stable.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use rex_ir::{
    ClassDef, DatatypeDef, EnumDef, Feature, FeatureKind, Model, Multiplicity, PrimitiveType,
    TypeRef, Upper, VocabularyDef,
};

/// Which flavor of schema to generate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Profile {
    /// Validates the canonical instance document (envelope, ids, `$ref`s).
    Wire,
    /// Flattened DTO projection for OpenAPI-style consumers.
    Api,
}

/// Generates the JSON Schema for `model`.
///
/// Returns a map of relative file path to file contents. Currently emits a
/// single `schema.json`.
pub fn generate(model: &Model, profile: Profile) -> anyhow::Result<BTreeMap<String, String>> {
    let context = Context::collect(model)?;
    let schema = match profile {
        Profile::Wire => wire_schema(&context)?,
        Profile::Api => api_schema(&context)?,
    };
    let json = serde_json::to_string_pretty(&schema)?;
    let mut files = BTreeMap::new();
    files.insert("schema.json".to_string(), json);
    Ok(files)
}

/// Generates the JSON Schema for `model` and writes it into `out_dir`.
pub fn generate_to_dir(model: &Model, profile: Profile, out_dir: &Path) -> anyhow::Result<()> {
    let files = generate(model, profile)?;
    for (relative, contents) in &files {
        let path = out_dir.join(relative);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, contents)?;
    }
    Ok(())
}

/// The draft every generated schema declares.
const DRAFT: &str = "https://json-schema.org/draft/2020-12/schema";

/// The `$schema`-style projection of one model: every definition the emitter
/// needs, pre-resolved by name (backends flatten packages and require unique
/// class names, mirroring the Rust backend).
struct Context<'a> {
    /// The first package name; used for `$id` and the document `model` const.
    package_name: String,
    /// All classes in declaration order.
    classes: Vec<&'a ClassDef>,
    /// All enums, for literal lookups.
    enums: Vec<&'a EnumDef>,
    /// All datatypes, for `$comment` lookups.
    datatypes: Vec<&'a DatatypeDef>,
    /// All vocabularies, for entry-key lookups.
    vocabularies: Vec<&'a VocabularyDef>,
}

impl<'a> Context<'a> {
    fn collect(model: &'a Model) -> anyhow::Result<Self> {
        let mut classes = Vec::new();
        let mut seen = BTreeSet::new();
        let mut enums = Vec::new();
        let mut datatypes = Vec::new();
        let mut vocabularies = Vec::new();
        for package in &model.packages {
            enums.extend(package.enums.iter());
            datatypes.extend(package.datatypes.iter());
            vocabularies.extend(package.vocabularies.iter());
            for class in &package.classes {
                if !seen.insert(class.name.clone()) {
                    anyhow::bail!(
                        "duplicate class name '{}' across packages; the JSON Schema backend \
                         flattens packages into one $defs namespace and requires unique class names",
                        class.name
                    );
                }
                classes.push(class);
            }
        }
        let package_name = model
            .packages
            .first()
            .map(|p| p.name.clone())
            .unwrap_or_else(|| "model".to_string());
        Ok(Self {
            package_name,
            classes,
            enums,
            datatypes,
            vocabularies,
        })
    }

    fn vocabulary_def(&self, name: &str) -> anyhow::Result<&'a VocabularyDef> {
        self.vocabularies
            .iter()
            .copied()
            .find(|v| v.name == name)
            .ok_or_else(|| anyhow::anyhow!("vocabulary '{name}' not found in model"))
    }

    fn enum_def(&self, name: &str) -> anyhow::Result<&'a EnumDef> {
        self.enums
            .iter()
            .copied()
            .find(|e| e.name == name)
            .ok_or_else(|| anyhow::anyhow!("enum '{name}' not found in model"))
    }

    fn datatype_def(&self, name: &str) -> anyhow::Result<&'a DatatypeDef> {
        self.datatypes
            .iter()
            .copied()
            .find(|d| d.name == name)
            .ok_or_else(|| anyhow::anyhow!("datatype '{name}' not found in model"))
    }

    /// `true` if any class directly extends `name` (i.e. `name` is a base).
    fn is_extended(&self, name: &str) -> bool {
        self.classes
            .iter()
            .any(|c| c.name != name && self.extends_class(c, name))
    }

    fn extends_class(&self, class: &ClassDef, base: &str) -> bool {
        class
            .extends
            .iter()
            .any(|t| matches!(t, TypeRef::Class { name, .. } if name == base))
    }

    /// Direct subclass names of `name`, in declaration order.
    fn subclass_names(&self, name: &str) -> Vec<String> {
        self.classes
            .iter()
            .filter(|c| c.name != name && self.extends_class(c, name))
            .map(|c| c.name.clone())
            .collect()
    }

    /// All transitive subclass names of `name`, breadth-first in declaration
    /// order, excluding `name` itself.
    fn transitive_subclass_names(&self, name: &str) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        let mut queue = self.subclass_names(name);
        let mut index = 0;
        while index < queue.len() {
            let current = queue[index].clone();
            index += 1;
            if out.contains(&current) {
                continue;
            }
            out.push(current.clone());
            for sub in self.subclass_names(&current) {
                if !queue.contains(&sub) {
                    queue.push(sub);
                }
            }
        }
        out
    }

    /// The class plus all transitive subclass names: the "type family" a
    /// base-class schema must accept (ids and `$type` discriminators).
    fn type_family(&self, class_name: &str) -> Vec<String> {
        let mut names = vec![class_name.to_string()];
        names.extend(self.transitive_subclass_names(class_name));
        names
    }

    /// The id-schema for a class's `$id` property: a single
    /// `"<singular>/<n>"` pattern, or — for extended bases, whose defs apply
    /// to subclass instances through `allOf` — an `anyOf` over the whole
    /// subclass family's patterns.
    fn id_schema(&self, class_name: &str) -> serde_json::Value {
        let family = self.type_family(class_name);
        if family.len() == 1 {
            serde_json::json!({
                "type": "string",
                "pattern": format!("^{}/[0-9]+$", snake_case(class_name)),
            })
        } else {
            let arms = family
                .iter()
                .map(|name| {
                    serde_json::json!({
                        "type": "string",
                        "pattern": format!("^{}/[0-9]+$", snake_case(name)),
                    })
                })
                .collect::<Vec<_>>();
            serde_json::json!({"anyOf": arms})
        }
    }

    /// Names of every class referenced by a cross reference feature, sorted.
    fn cross_reference_targets(&self) -> BTreeSet<String> {
        let mut targets = BTreeSet::new();
        for class in &self.classes {
            for feature in &class.features {
                if feature.kind == FeatureKind::CrossReference {
                    if let TypeRef::Class { name, .. } = &feature.type_ {
                        targets.insert(name.clone());
                    }
                }
            }
        }
        targets
    }
}

/// Builds the WIRE profile schema: a root document schema validating the
/// whole canonical instance document, with per-class and per-ref defs.
fn wire_schema(context: &Context<'_>) -> anyhow::Result<serde_json::Value> {
    let mut root = serde_json::Map::new();
    root.insert("$schema".to_string(), serde_json::json!(DRAFT));
    root.insert(
        "$id".to_string(),
        serde_json::json!(format!("urn:rex:model:{}:wire", context.package_name)),
    );
    root.insert("type".to_string(), serde_json::json!("object"));
    root.insert(
        "properties".to_string(),
        wire_root_properties(context)?,
    );
    let mut required = vec![
        "$type".to_string(),
        "formatVersion".to_string(),
        "model".to_string(),
        "objects".to_string(),
    ];
    required.sort();
    root.insert("required".to_string(), serde_json::json!(required));
    root.insert(
        "unevaluatedProperties".to_string(),
        serde_json::json!(false),
    );
    root.insert("$defs".to_string(), wire_defs(context)?);
    Ok(serde_json::Value::Object(root))
}

/// The document envelope: the instance `$type`/`formatVersion`/`model` consts
/// and the `objects` array discriminated by class `$type`.
fn wire_root_properties(context: &Context<'_>) -> anyhow::Result<serde_json::Value> {
    let mut properties = serde_json::Map::new();
    properties.insert(
        "$type".to_string(),
        serde_json::json!({"const": "rex.instance"}),
    );
    properties.insert("formatVersion".to_string(), serde_json::json!({"const": 1}));
    properties.insert(
        "model".to_string(),
        serde_json::json!({"const": context.package_name}),
    );
    let mut objects = serde_json::Map::new();
    objects.insert("type".to_string(), serde_json::json!("array"));
    if !context.classes.is_empty() {
        let arms: Vec<serde_json::Value> = context
            .classes
            .iter()
            .map(|class| serde_json::json!({"$ref": format!("#/$defs/{}", class.name)}))
            .collect();
        objects.insert("items".to_string(), serde_json::json!({"anyOf": arms}));
    }
    properties.insert("objects".to_string(), serde_json::Value::Object(objects));
    Ok(serde_json::Value::Object(properties))
}

/// All `$defs` of the wire schema: one def per class plus one shared
/// `<Target>Ref` def per cross reference target class.
fn wire_defs(context: &Context<'_>) -> anyhow::Result<serde_json::Value> {
    let mut defs = serde_json::Map::new();
    for class in &context.classes {
        defs.insert(class.name.clone(), wire_class_def(context, class)?);
    }
    for target in context.cross_reference_targets() {
        defs.insert(format!("{target}Ref"), ref_def(context, &target));
    }
    Ok(serde_json::Value::Object(defs))
}

/// The shared `{"$ref": "<id>"}` link-object def for one target class.
fn ref_def(context: &Context<'_>, class_name: &str) -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "properties": {
            "$ref": context.id_schema(class_name),
        },
        "required": ["$ref"],
        "unevaluatedProperties": false,
    })
}

/// One class def of the wire schema.
///
/// Subclasses get `allOf: [{"$ref": base}]`; the `$type` property is a const
/// on leaf classes and an enum (over the transitive subclass closure) on
/// extended bases; `unevaluatedProperties: false` is only placed on leaves so
/// inherited properties evaluate through the `allOf` chain.
fn wire_class_def(context: &Context<'_>, class: &ClassDef) -> anyhow::Result<serde_json::Value> {
    let mut def = serde_json::Map::new();
    def.insert("type".to_string(), serde_json::json!("object"));
    if let Some(comment) = class_comment(class, true) {
        def.insert("$comment".to_string(), serde_json::json!(comment));
    }
    insert_base_all_of(class, &mut def);
    def.insert(
        "properties".to_string(),
        wire_class_properties(context, class)?,
    );
    def.insert(
        "required".to_string(),
        serde_json::json!(class_required(class, true)),
    );
    if !context.is_extended(&class.name) {
        def.insert(
            "unevaluatedProperties".to_string(),
            serde_json::json!(false),
        );
    }
    Ok(serde_json::Value::Object(def))
}

fn wire_class_properties(
    context: &Context<'_>,
    class: &ClassDef,
) -> anyhow::Result<serde_json::Value> {
    let mut properties = serde_json::Map::new();
    properties.insert(
        "$type".to_string(),
        type_discriminator(context, class, true),
    );
    properties.insert("$id".to_string(), context.id_schema(&class.name));
    for feature in &class.features {
        if feature.is_derived || feature.kind == FeatureKind::Container {
            continue;
        }
        properties.insert(
            feature.name.clone(),
            feature_schema(context, feature, Profile::Wire)?,
        );
    }
    Ok(serde_json::Value::Object(properties))
}

/// The `$type` property schema: a const for leaf classes, an enum over the
/// class plus its transitive subclasses for extended bases. In the API
/// profile the property is emitted only for extended bases.
fn type_discriminator(
    context: &Context<'_>,
    class: &ClassDef,
    include_leaf_const: bool,
) -> serde_json::Value {
    let mut names = vec![class.name.clone()];
    names.extend(context.transitive_subclass_names(&class.name));
    if names.len() == 1 {
        if include_leaf_const {
            serde_json::json!({"const": class.name})
        } else {
            serde_json::Value::Null
        }
    } else {
        serde_json::json!({"enum": names})
    }
}

/// Builds the API profile schema: a components-style document holding only
/// `$schema`, `$id`, and `$defs`.
fn api_schema(context: &Context<'_>) -> anyhow::Result<serde_json::Value> {
    let mut root = serde_json::Map::new();
    root.insert("$schema".to_string(), serde_json::json!(DRAFT));
    root.insert(
        "$id".to_string(),
        serde_json::json!(format!("urn:rex:model:{}:api", context.package_name)),
    );
    let mut defs = serde_json::Map::new();
    for class in &context.classes {
        defs.insert(class.name.clone(), api_class_def(context, class)?);
    }
    root.insert("$defs".to_string(), serde_json::Value::Object(defs));
    Ok(serde_json::Value::Object(root))
}

/// One class def of the API profile: no envelope keys, cross references as
/// plain id strings, derived features kept as `readOnly`.
fn api_class_def(context: &Context<'_>, class: &ClassDef) -> anyhow::Result<serde_json::Value> {
    let mut def = serde_json::Map::new();
    def.insert("type".to_string(), serde_json::json!("object"));
    if let Some(comment) = class_comment(class, false) {
        def.insert("$comment".to_string(), serde_json::json!(comment));
    }
    let extended = context.is_extended(&class.name);
    let descends = class
        .extends
        .iter()
        .any(|t| matches!(t, TypeRef::Class { .. }));
    if !extended && !descends {
        def.insert("additionalProperties".to_string(), serde_json::json!(false));
    }
    insert_base_all_of(class, &mut def);

    let mut properties = serde_json::Map::new();
    let discriminator = type_discriminator(context, class, false);
    if !discriminator.is_null() {
        properties.insert("$type".to_string(), discriminator);
    }
    for feature in &class.features {
        if feature.kind == FeatureKind::Container {
            continue;
        }
        let mut schema = feature_schema(context, feature, Profile::Api)?;
        if feature.is_derived || feature.is_read_only {
            schema
                .as_object_mut()
                .expect("feature schema is an object")
                .insert("readOnly".to_string(), serde_json::json!(true));
        }
        properties.insert(feature.name.clone(), schema);
    }
    def.insert("properties".to_string(), serde_json::Value::Object(properties));

    let mut required = class_required(class, false);
    if extended {
        required.push("$type".to_string());
    }
    required.sort();
    required.dedup();
    def.insert("required".to_string(), serde_json::json!(required));

    if descends && !extended {
        def.insert(
            "unevaluatedProperties".to_string(),
            serde_json::json!(false),
        );
    }
    Ok(serde_json::Value::Object(def))
}

/// Inserts `allOf: [{"$ref": "#/$defs/Base"}, ...]` for class-typed extends.
fn insert_base_all_of(class: &ClassDef, def: &mut serde_json::Map<String, serde_json::Value>) {
    let bases: Vec<&String> = class
        .extends
        .iter()
        .filter_map(|t| match t {
            TypeRef::Class { name, .. } => Some(name),
            _ => None,
        })
        .collect();
    if !bases.is_empty() {
        let arms = bases
            .iter()
            .map(|base| serde_json::json!({"$ref": format!("#/$defs/{base}")}))
            .collect::<Vec<_>>();
        def.insert("allOf".to_string(), serde_json::Value::Array(arms));
    }
}

/// The required-member list for a class: `$type`/`$id` plus every
/// non-derived feature that must be present.
///
/// Attributes and cross references are required when single-valued with
/// `lower >= 1`; containment is required when many (the array is always
/// serialized) or when single with `lower >= 1`.
fn class_required(class: &ClassDef, include_envelope_keys: bool) -> Vec<String> {
    let mut required = Vec::new();
    if include_envelope_keys {
        required.push("$id".to_string());
        required.push("$type".to_string());
    }
    for feature in &class.features {
        if feature.is_derived {
            continue;
        }
        let is_required = match feature.kind {
            FeatureKind::Attribute | FeatureKind::CrossReference => {
                !feature.multiplicity.is_many() && feature.multiplicity.lower >= 1
            }
            FeatureKind::Containment => {
                feature.multiplicity.is_many() || feature.multiplicity.lower >= 1
            }
            FeatureKind::Container => false,
        };
        if is_required {
            required.push(feature.name.clone());
        }
    }
    required.sort();
    required.dedup();
    required
}

/// The value schema for one feature: its element schema, wrapped in an array
/// schema when the multiplicity is many.
fn feature_schema(
    context: &Context<'_>,
    feature: &Feature,
    profile: Profile,
) -> anyhow::Result<serde_json::Value> {
    let element = match feature.kind {
        FeatureKind::Attribute => value_schema(context, &feature.type_, profile)?,
        FeatureKind::Containment => {
            let TypeRef::Class { name, .. } = &feature.type_ else {
                anyhow::bail!(
                    "containment feature '{}' must have a class type",
                    feature.name
                );
            };
            serde_json::json!({"$ref": format!("#/$defs/{name}")})
        }
        FeatureKind::CrossReference => {
            let TypeRef::Class { name, .. } = &feature.type_ else {
                anyhow::bail!(
                    "cross reference feature '{}' must have a class type",
                    feature.name
                );
            };
            match profile {
                Profile::Wire => serde_json::json!({"$ref": format!("#/$defs/{name}Ref")}),
                Profile::Api => context.id_schema(name),
            }
        }
        FeatureKind::Container => anyhow::bail!(
            "container feature '{}' must not be materialized as a schema property",
            feature.name
        ),
    };
    Ok(if feature.multiplicity.is_many() {
        array_schema(element, &feature.multiplicity)
    } else {
        element
    })
}

/// Wraps an element schema in an array schema, bounding it by the feature's
/// declared multiplicity.
fn array_schema(items: serde_json::Value, multiplicity: &Multiplicity) -> serde_json::Value {
    let mut wrapper = serde_json::Map::new();
    wrapper.insert("type".to_string(), serde_json::json!("array"));
    wrapper.insert("items".to_string(), items);
    if multiplicity.lower > 0 {
        wrapper.insert("minItems".to_string(), serde_json::json!(multiplicity.lower));
    }
    if let Upper::Finite(upper) = multiplicity.upper {
        wrapper.insert("maxItems".to_string(), serde_json::json!(upper));
    }
    serde_json::Value::Object(wrapper)
}

/// The value schema of an attribute's element type.
fn value_schema(
    context: &Context<'_>,
    type_: &TypeRef,
    profile: Profile,
) -> anyhow::Result<serde_json::Value> {
    match type_ {
        TypeRef::Primitive(primitive) => Ok(primitive_schema(*primitive, profile)),
        TypeRef::Enum { name, .. } => {
            let enum_def = context.enum_def(name)?;
            let literals: Vec<&String> = enum_def.literals.iter().map(|l| &l.name).collect();
            Ok(serde_json::json!({"enum": literals}))
        }
        TypeRef::Datatype { name, .. } => {
            let datatype = context.datatype_def(name)?;
            Ok(serde_json::json!({
                "type": "string",
                "$comment": datatype_comment(datatype),
            }))
        }
        TypeRef::Vocabulary { name, .. } => {
            let vocabulary = context.vocabulary_def(name)?;
            let keys: Vec<&String> = vocabulary.entries.iter().map(|entry| &entry.key).collect();
            Ok(serde_json::json!({
                "$comment": vocabulary_comment(vocabulary),
                "enum": keys,
            }))
        },
        TypeRef::Class { name, .. } | TypeRef::Interface { name, .. } => anyhow::bail!(
            "attribute values must be primitive, enum, or datatype; found class '{name}'"
        ),
    }
}

/// The JSON Schema mapping of one primitive. The API profile additionally
/// annotates the platform-width `format` keywords.
fn primitive_schema(primitive: PrimitiveType, profile: Profile) -> serde_json::Value {
    let mut schema = serde_json::Map::new();
    let numeric = |schema: &mut serde_json::Map<String, serde_json::Value>,
                   kind: &str,
                   format: Option<&str>| {
        schema.insert("type".to_string(), serde_json::json!(kind));
        if profile == Profile::Api {
            if let Some(format) = format {
                schema.insert("format".to_string(), serde_json::json!(format));
            }
        }
    };
    match primitive {
        PrimitiveType::String => {
            schema.insert("type".to_string(), serde_json::json!("string"));
        }
        PrimitiveType::Int => numeric(&mut schema, "integer", Some("int32")),
        PrimitiveType::Long => numeric(&mut schema, "integer", Some("int64")),
        PrimitiveType::Short | PrimitiveType::Byte => numeric(&mut schema, "integer", None),
        PrimitiveType::Float => numeric(&mut schema, "number", Some("float")),
        PrimitiveType::Double => numeric(&mut schema, "number", Some("double")),
        PrimitiveType::Boolean => {
            schema.insert("type".to_string(), serde_json::json!("boolean"));
        }
        PrimitiveType::Char => {
            schema.insert("type".to_string(), serde_json::json!("string"));
            schema.insert("minLength".to_string(), serde_json::json!(1));
            schema.insert("maxLength".to_string(), serde_json::json!(1));
        }
    }
    serde_json::Value::Object(schema)
}

/// The `$comment` documenting a datatype attribute's target mapping.
fn datatype_comment(datatype: &DatatypeDef) -> String {    let mut comment = format!(
        "datatype {} ({});",
        datatype.name,
        match &datatype.platform {
            Some(platform) => format!("wraps {platform}"),
            None => "opaque".to_string(),
        }
    );
    if !datatype.target_bindings.is_empty() {
        comment.push_str(" target bindings: ");
        let bindings: Vec<String> = datatype
            .target_bindings
            .iter()
            .map(|(target, type_name)| format!("{target} → {type_name}"))
            .collect();
        comment.push_str(&bindings.join("; "));
    }
    comment
}

/// The `$comment` documenting a vocabulary attribute's vendored source:
/// source id, pinned version, and entry count.
fn vocabulary_comment(vocabulary: &VocabularyDef) -> String {
    format!(
        "vocabulary {source}@{version} with {count} entries",
        source = vocabulary.source,
        version = vocabulary.version.as_deref().unwrap_or("unversioned"),
        count = vocabulary.entries.len(),
    )
}

/// The `$comment` documenting a class's intentionally omitted features and
/// its `id`/`readonly` modifiers.
///
/// The wire profile omits both container back-pointers and derived features;
/// the API profile keeps derived features (`readOnly`), so only containers
/// are listed there. Every feature carrying the `id` or `readonly` modifier
/// contributes an `identity feature '<f>'` / `readonly feature '<f>'` note;
/// these only appear when modifiers do, so unmodified models (e.g. the
/// conformance goldens) are unaffected.
fn class_comment(class: &ClassDef, include_derived: bool) -> Option<String> {
    let mut parts: Vec<String> = Vec::new();
    for feature in &class.features {
        if feature.kind == FeatureKind::Container {
            parts.push(format!(
                "omitted: container '{}' (reconstructed from nesting on load)",
                feature.name
            ));
        }
    }
    if include_derived {
        for feature in &class.features {
            if feature.is_derived {
                parts.push(format!(
                    "omitted: derived '{}' (computed, not stored)",
                    feature.name
                ));
            }
        }
    }
    for feature in &class.features {
        if feature.is_id {
            parts.push(format!("identity feature '{}'", feature.name));
        }
        if feature.is_read_only {
            parts.push(format!("readonly feature '{}'", feature.name));
        }
    }
    (!parts.is_empty()).then(|| parts.join("; "))
}

/// Converts a class name to the snake_case singular used in canonical
/// instance ids (`"Book"` → `"book"`, `"Library"` → `"library"`), mirroring
/// the instance serializer.
fn snake_case(name: &str) -> String {
    let chars: Vec<char> = name.chars().collect();
    let mut out = String::with_capacity(name.len() + 4);
    for (index, ch) in chars.iter().copied().enumerate() {
        if ch.is_ascii_uppercase() {
            let prev_lower_or_digit = index
                .checked_sub(1)
                .map(|p| chars[p].is_ascii_lowercase() || chars[p].is_ascii_digit())
                .unwrap_or(false);
            let next_lower = chars
                .get(index + 1)
                .map(|n| n.is_ascii_lowercase())
                .unwrap_or(false);
            if prev_lower_or_digit || (index > 0 && next_lower) {
                out.push('_');
            }
            out.extend(ch.to_lowercase());
        } else {
            out.push(ch);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wire_primitive_mapping() {
        assert_eq!(
            primitive_schema(PrimitiveType::String, Profile::Wire),
            serde_json::json!({"type": "string"})
        );
        assert_eq!(
            primitive_schema(PrimitiveType::Int, Profile::Wire),
            serde_json::json!({"type": "integer"})
        );
        assert_eq!(
            primitive_schema(PrimitiveType::Double, Profile::Wire),
            serde_json::json!({"type": "number"})
        );
        assert_eq!(
            primitive_schema(PrimitiveType::Boolean, Profile::Wire),
            serde_json::json!({"type": "boolean"})
        );
        assert_eq!(
            primitive_schema(PrimitiveType::Char, Profile::Wire),
            serde_json::json!({"maxLength": 1, "minLength": 1, "type": "string"})
        );
    }

    #[test]
    fn api_primitive_mapping_adds_formats() {
        assert_eq!(
            primitive_schema(PrimitiveType::Long, Profile::Api),
            serde_json::json!({"format": "int64", "type": "integer"})
        );
        assert_eq!(
            primitive_schema(PrimitiveType::Float, Profile::Api),
            serde_json::json!({"format": "float", "type": "number"})
        );
        assert_eq!(
            primitive_schema(PrimitiveType::Short, Profile::Api),
            serde_json::json!({"type": "integer"}),
            "short has no platform format"
        );
    }

    #[test]
    fn id_patterns_use_snake_case_singular() {
        assert_eq!(snake_case("Library"), "library");
        assert_eq!(snake_case("Book"), "book");
        assert_eq!(snake_case("Writer"), "writer");
        assert_eq!(snake_case("HTTPServer"), "http_server");
        let mut properties = serde_json::Map::new();
        properties.insert(
            "$id".to_string(),
            serde_json::json!({"pattern": format!("^{}/[0-9]+$", snake_case("Library"))}),
        );
        assert_eq!(
            properties["$id"]["pattern"],
            "^library/[0-9]+$",
            "matches canonical ids like \"library/0\""
        );
    }

    #[test]
    fn multiplicity_bounds_wrap_arrays() {
        let bounded = Multiplicity {
            lower: 2,
            upper: Upper::Finite(5),
        };
        assert_eq!(
            array_schema(serde_json::json!({"type": "string"}), &bounded),
            serde_json::json!({"items": {"type": "string"}, "maxItems": 5, "minItems": 2, "type": "array"})
        );
        let unbounded = Multiplicity {
            lower: 0,
            upper: Upper::Unbounded,
        };
        assert_eq!(
            array_schema(serde_json::json!({"type": "string"}), &unbounded),
            serde_json::json!({"items": {"type": "string"}, "type": "array"})
        );
    }

    #[test]
    fn class_comment_lists_omissions_in_order() {
        let class = ClassDef::new(
            "Book",
            vec![],
            vec![
                Feature::new(
                    "library",
                    FeatureKind::Container,
                    TypeRef::Class {
                        package: "p".to_string(),
                        name: "Library".to_string(),
                    },
                    Multiplicity::REQUIRED,
                ),
                Feature::new(
                    "citation",
                    FeatureKind::Attribute,
                    TypeRef::Primitive(PrimitiveType::String),
                    Multiplicity::OPTIONAL,
                )
                .derived(),
            ],
        );
        let wire = class_comment(&class, true).expect("wire comment");
        assert_eq!(
            wire,
            "omitted: container 'library' (reconstructed from nesting on load); \
             omitted: derived 'citation' (computed, not stored)"
        );
        let api = class_comment(&class, false).expect("api comment");
        assert_eq!(
            api,
            "omitted: container 'library' (reconstructed from nesting on load)"
        );
    }

    #[test]
    fn inheritance_helpers_resolve_transitive_closure() {
        let class = |name: &str, extends: Vec<TypeRef>| {
            ClassDef::new(name, extends, vec![])
        };
        let base = class("Animal", vec![]);
        let middle = class(
            "Dog",
            vec![TypeRef::Class {
                package: "zoo".to_string(),
                name: "Animal".to_string(),
            }],
        );
        let leaf = class(
            "Puppy",
            vec![TypeRef::Class {
                package: "zoo".to_string(),
                name: "Dog".to_string(),
            }],
        );
        let mut model = Model::new();
        let mut package = rex_ir::Package::new("zoo");
        package.classes = vec![base, middle, leaf];
        model.packages.push(package);
        let context = Context::collect(&model).expect("context");

        assert!(context.is_extended("Animal"));
        assert!(context.is_extended("Dog"));
        assert!(!context.is_extended("Puppy"));
        assert_eq!(
            context.transitive_subclass_names("Animal"),
            vec!["Dog".to_string(), "Puppy".to_string()]
        );
        assert_eq!(
            context.transitive_subclass_names("Puppy"),
            Vec::<String>::new()
        );
        assert_eq!(context.cross_reference_targets(), BTreeSet::new());
    }
}
