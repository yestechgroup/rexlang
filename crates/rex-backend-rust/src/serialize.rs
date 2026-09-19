//! Canonical instance JSON code generation.
//!
//! Emits `Resource::to_instance_json` / `Resource::from_instance_json` per
//! generated model, implementing the cross-language contract:
//!
//! - Document root: `{ "$type": "rex.instance", "formatVersion": 1, "model":
//!   "<package>", "objects": [...] }`.
//! - Object ids: `"<singular>/<n>"`, n = per-class insertion order —
//!   deterministic for a given resource. The document groups objects by owner
//!   (containment is inlined), so the loader registers and fills in
//!   id-ordinal order, reconstructing the source per-class insertion order
//!   even when objects were created in an order the document could not
//!   express positionally.
//! - Containment features serialize contained objects INLINE; a contained
//!   object that was already emitted as a document object (its class is
//!   declared before its owner's) serializes as a `{"$ref": ...}` link
//!   instead, which the loader resolves during fill. Container features are
//!   NEVER serialized (they are derived from the opposite and reconstructed
//!   by the loader — serializing them would recurse forever).
//! - Cross references serialize as `{"$ref": "<id>"}`.
//! - Enums serialize the declared literal name; datatypes their inner String;
//!   vocabulary-typed attributes the entry key string (loading resolves the
//!   key back to the variant, unknown keys are `InvalidInstance`); optional
//!   attributes are omitted when `None`; derived features are never
//!   serialized.
//! - Loading is strict: unknown feature keys, unknown `$type`, duplicate
//!   `$id`, unresolved `$ref`, a serialized container key, and version/model
//!   mismatches are all `RexError::InvalidInstance`.
//! - Round-trip identity: `from_instance_json(&x.to_instance_json()) == x`.

use rex_ir::{Feature, FeatureKind, PrimitiveType, TypeRef};

use crate::codegen::{ClassCtx, Unit};
use crate::naming::rust_ident;

/// The JSON key a feature serializes under: the declared `.mox` feature name,
/// verbatim (the cross-language contract keys on declared names).
fn json_key(feature: &Feature) -> &str {
    &feature.name
}

/// The Rust expression serializing a value of the feature's type.
///
/// `v` is a place expression of type `T` when `deref` is `false` (required
/// attribute: `obj.field`) and of type `&T` when `deref` is `true` (optional
/// attribute binding or iterator item). Reference-like types
/// (String/enum/datatype/char) clone the same way either way; numerics and
/// bool need a `*` only behind a reference.
fn save_expr(type_: &TypeRef, v: &str, deref: bool) -> String {
    let place = |expr: &str| {
        if deref {
            format!("*{expr}")
        } else {
            expr.to_string()
        }
    };
    match type_ {
        TypeRef::Primitive(primitive) => match primitive {
            PrimitiveType::String => format!("serde_json::Value::String({v}.clone())"),
            PrimitiveType::Int
            | PrimitiveType::Long
            | PrimitiveType::Short
            | PrimitiveType::Byte => format!("serde_json::Value::from({})", place(v)),
            PrimitiveType::Float | PrimitiveType::Double => {
                format!("serde_json::Value::from({})", place(v))
            }
            PrimitiveType::Boolean => format!("serde_json::Value::Bool({})", place(v)),
            PrimitiveType::Char => format!("serde_json::Value::String({v}.to_string())"),
        },
        TypeRef::Enum { .. } => format!("serde_json::Value::String({v}.name().to_string())"),
        TypeRef::Datatype { .. } => format!("serde_json::Value::String({v}.0.clone())"),
        TypeRef::Vocabulary { .. } => format!("serde_json::Value::String({v}.key().to_string())"),
        TypeRef::Class { .. } | TypeRef::Interface { .. } => {
            unreachable!("class/interface values are references, not attributes")
        }
    }
}

/// The Rust expression parsing a `serde_json::Value` named `value` into the
/// feature's scalar type.
fn load_expr(type_: &TypeRef, value: &str, where_: &str) -> String {
    let w = where_.to_string();
    match type_ {
        TypeRef::Primitive(primitive) => match primitive {
            PrimitiveType::String => {
                format!("expect_string({value}, {w:?})?.to_string()")
            }
            PrimitiveType::Int => format!("expect_i32({value}, {w:?})?"),
            PrimitiveType::Long => format!("expect_i64({value}, {w:?})?"),
            PrimitiveType::Short => format!("expect_i16({value}, {w:?})?"),
            PrimitiveType::Byte => format!("expect_i8({value}, {w:?})?"),
            PrimitiveType::Float => format!("expect_f32({value}, {w:?})?"),
            PrimitiveType::Double => format!("expect_f64({value}, {w:?})?"),
            PrimitiveType::Boolean => format!("expect_bool({value}, {w:?})?"),
            PrimitiveType::Char => format!("expect_char({value}, {w:?})?"),
        },
        TypeRef::Enum { name, .. } => format!(
            "{}::from_instance_name(expect_string({value}, {w:?})?)?",
            rust_ident(name)
        ),
        TypeRef::Datatype { name, .. } => format!(
            "{}(expect_string({value}, {w:?})?.to_string())",
            rust_ident(name)
        ),
        TypeRef::Vocabulary { name, .. } => format!(
            "{}::try_from(expect_string({value}, {w:?})?)?",
            rust_ident(name)
        ),
        TypeRef::Class { .. } | TypeRef::Interface { .. } => {
            unreachable!("class/interface values are references, not attributes")
        }
    }
}

/// Emits the whole serialization section: helpers, per-class save/load fns,
/// registry, and the two `Resource` entry points.
pub fn emit(e: &mut String, unit: &Unit<'_>) -> anyhow::Result<()> {
    emit_value_helpers(e);
    emit_ids_and_registry(e, unit);
    emit_enum_loaders(e, unit)?;
    e.push_str("impl Resource {\n");
    emit_assign_ids(e, unit);
    for class in &unit.classes {
        emit_class_save(e, unit, class)?;
    }
    for class in &unit.classes {
        emit_class_register(e, class)?;
        emit_class_fill(e, unit, class)?;
    }
    emit_entry_points(e, unit)?;
    e.push_str("}\n");
    Ok(())
}

fn emit_value_helpers(e: &mut String) {
    e.push_str(
        "#[allow(dead_code)]\n\
         fn instance_error(message: String) -> rex_runtime::RexError {\n\
         \x20   rex_runtime::RexError::InvalidInstance { message }\n\
         }\n\n",
    );
    for (signature, body) in [
        (
            "fn expect_string<'a>(value: &'a serde_json::Value, where_: &str) -> Result<&'a str, rex_runtime::RexError>",
            "value.as_str().ok_or_else(|| instance_error(format!(\"{where_} must be a string\")))",
        ),
        (
            "fn expect_bool(value: &serde_json::Value, where_: &str) -> Result<bool, rex_runtime::RexError>",
            "value.as_bool().ok_or_else(|| instance_error(format!(\"{where_} must be a boolean\")))",
        ),
        (
            "fn expect_i64(value: &serde_json::Value, where_: &str) -> Result<i64, rex_runtime::RexError>",
            "value.as_i64().ok_or_else(|| instance_error(format!(\"{where_} must be an integer\")))",
        ),
        (
            "fn expect_i32(value: &serde_json::Value, where_: &str) -> Result<i32, rex_runtime::RexError>",
            "expect_i64(value, where_)?.try_into().map_err(|_| instance_error(format!(\"{where_} must be a 32-bit integer\")))",
        ),
        (
            "fn expect_i16(value: &serde_json::Value, where_: &str) -> Result<i16, rex_runtime::RexError>",
            "expect_i64(value, where_)?.try_into().map_err(|_| instance_error(format!(\"{where_} must be a 16-bit integer\")))",
        ),
        (
            "fn expect_i8(value: &serde_json::Value, where_: &str) -> Result<i8, rex_runtime::RexError>",
            "expect_i64(value, where_)?.try_into().map_err(|_| instance_error(format!(\"{where_} must be an 8-bit integer\")))",
        ),
        (
            "fn expect_f64(value: &serde_json::Value, where_: &str) -> Result<f64, rex_runtime::RexError>",
            "value.as_f64().ok_or_else(|| instance_error(format!(\"{where_} must be a number\")))",
        ),
        (
            "fn expect_f32(value: &serde_json::Value, where_: &str) -> Result<f32, rex_runtime::RexError>",
            "Ok(expect_f64(value, where_)? as f32)",
        ),
    ] {
        e.push_str(&format!(
            "#[allow(dead_code)]\n{signature} {{\n\
             \x20   {body}\n\
             }}\n\n"
        ));
    }
    e.push_str(
        "#[allow(dead_code)]\n\
         fn expect_char(value: &serde_json::Value, where_: &str) -> Result<char, rex_runtime::RexError> {\n\
         \x20   let s = expect_string(value, where_)?;\n\
         \x20   let mut chars = s.chars();\n\
         \x20   match (chars.next(), chars.next()) {\n\
         \x20       (Some(c), None) => Ok(c),\n\
         \x20       _ => Err(instance_error(format!(\"{where_} must be a single character\"))),\n\
         \x20   }\n\
         }\n\n",
    );
    e.push_str(
        "#[allow(dead_code)]\n\
         fn instance_id_sort_key(object: &serde_json::Value) -> Option<(String, u64)> {\n\
         \x20   let id = object.get(rex_runtime::KEY_ID)?.as_str()?;\n\
         \x20   let (prefix, ordinal) = id.rsplit_once('/')?;\n\
         \x20   Some((prefix.to_string(), ordinal.parse().ok()?))\n\
         }\n\n",
    );
    e.push_str(
        "#[allow(dead_code)]\n\
         fn ref_object(id: &str) -> serde_json::Value {\n\
         \x20   let mut object = serde_json::Map::new();\n\
         \x20   object.insert(rex_runtime::KEY_REF.to_string(), serde_json::Value::String(id.to_string()));\n\
         \x20   serde_json::Value::Object(object)\n\
         }\n\n",
    );
    e.push_str(
        "fn id_of_object<'a>(object: &'a serde_json::Value) -> Result<&'a str, rex_runtime::RexError> {\n\
         \x20   object\n\
         \x20       .get(rex_runtime::KEY_ID)\n\
         \x20       .and_then(|v| v.as_str())\n\
         \x20       .ok_or_else(|| instance_error(\"object is missing its $id\".to_string()))\n\
         }\n\n",
    );
}

fn emit_ids_and_registry(e: &mut String, unit: &Unit<'_>) {
    e.push_str(
        "/// Per-class object ids in insertion order: `\"<singular>/<n>\"`.\n\
         #[derive(Debug, Default)]\n\
         struct InstanceIds {\n",
    );
    for class in &unit.classes {
        e.push_str(&format!(
            "    {}: std::collections::HashMap<{}, String>,\n",
            class.slot_field, class.id_type
        ));
    }
    e.push_str("}\n\n");

    e.push_str(
        "/// Registered objects by canonical id, filled during the first load pass.\n\
         #[derive(Debug, Default)]\n\
         struct InstanceRegistry {\n",
    );
    for class in &unit.classes {
        e.push_str(&format!(
            "    {}: std::collections::HashMap<String, {}>,\n",
            class.slot_field, class.id_type
        ));
    }
    e.push_str("}\n\n");
}

fn emit_assign_ids(e: &mut String, unit: &Unit<'_>) {
    e.push_str("    fn assign_instance_ids(&self, ids: &mut InstanceIds) {\n");
    for class in &unit.classes {
        e.push_str(&format!(
            "        for (index, (key, _)) in self.{}.iter().enumerate() {{\n\
             \x20           ids.{}.insert(key, format!(\"{}/{{index}}\"));\n\
             \x20       }}\n",
            class.slot_field, class.slot_field, class.single
        ));
    }
    e.push_str("    }\n\n");
}

/// Saves one object: `$type`/`$id`, attributes, embedded containment,
/// `$ref` cross references. Container and derived features are skipped.
fn emit_class_save(e: &mut String, unit: &Unit<'_>, class: &ClassCtx<'_>) -> anyhow::Result<()> {
    let struct_name = rust_ident(&class.class.name);
    e.push_str(&format!(
        "    fn {}_to_json(\n\
         \x20       &self,\n\
         \x20       id: {},\n\
         \x20       ids: &InstanceIds,\n\
         \x20       visited: &mut std::collections::HashSet<String>,\n\
         \x20   ) -> serde_json::Value {{\n\
         \x20       let mut object = serde_json::Map::new();\n\
         \x20       let id_string = ids.{}[&id].clone();\n\
         \x20       object.insert(\n\
         \x20           rex_runtime::KEY_TYPE.to_string(),\n\
         \x20           serde_json::Value::String({struct_name:?}.to_string()),\n\
         \x20       );\n\
         \x20       object.insert(rex_runtime::KEY_ID.to_string(), serde_json::Value::String(id_string.clone()));\n\
         \x20       visited.insert(id_string);\n",
        class.single, class.id_type, class.slot_field
    ));
    e.push_str(&format!(
        "        let Some(obj) = self.{}.get(id) else {{\n\
         \x20           return serde_json::Value::Object(object);\n\
         \x20       }};\n",
        class.slot_field
    ));
    for feature in &class.class.features {
        if feature.is_derived {
            continue;
        }
        let field = rust_ident(&crate::naming::snake_case(&feature.name));
        let key = json_key(feature);
        match feature.kind {
            FeatureKind::Attribute => {
                if feature.multiplicity.is_many() {
                    e.push_str(&format!(
                        "        object.insert(\n\
                         \x20           {key:?}.to_string(),\n\
                         \x20           serde_json::Value::Array(obj.{field}.iter().map(|v| {accessor}).collect()),\n\
                         \x20       );\n",
                        accessor = save_expr(&feature.type_, "v", true),
                    ));
                } else if feature.multiplicity.lower == 0 {
                    e.push_str(&format!(
                        "        if let Some(v) = &obj.{field} {{\n\
                         \x20           object.insert({key:?}.to_string(), {});\n\
                         \x20       }}\n",
                        save_expr(&feature.type_, "v", true),
                    ));
                } else {
                    e.push_str(&format!(
                        "        object.insert({key:?}.to_string(), {});\n",
                        save_expr(&feature.type_, &format!("obj.{field}"), false),
                    ));
                }
            }
            FeatureKind::Containment => {
                let target = match &feature.type_ {
                    TypeRef::Class { name, .. } => name,
                    _ => continue,
                };
                let target_ctx = unit.class(target)?;
                if feature.multiplicity.is_many() {
                    e.push_str(&format!(
                        "        object.insert(\n\
                         \x20           {key:?}.to_string(),\n\
                         \x20           serde_json::Value::Array(\n\
                         \x20               obj.{field}\n\
                         \x20                   .iter()\n\
                         \x20                   .map(|child| {{\n\
                         \x20                       let child_id = &ids.{slot}[child];\n\
                         \x20                       if visited.contains(child_id) {{\n\
                         \x20                           ref_object(child_id)\n\
                         \x20                       }} else {{\n\
                         \x20                           self.{single}_to_json(*child, ids, visited)\n\
                         \x20                       }}\n\
                         \x20                   }})\n\
                         \x20                   .collect(),\n\
                         \x20           ),\n\
                         \x20       );\n",
                        slot = target_ctx.slot_field,
                        single = target_ctx.single
                    ));
                } else {
                    e.push_str(&format!(
                        "        if let Some(child) = obj.{field} {{\n\
                         \x20           let child_id = &ids.{slot}[&child];\n\
                         \x20           let value = if visited.contains(child_id) {{\n\
                         \x20               ref_object(child_id)\n\
                         \x20           }} else {{\n\
                         \x20               self.{single}_to_json(child, ids, visited)\n\
                         \x20           }};\n\
                         \x20           object.insert({key:?}.to_string(), value);\n\
                         \x20       }}\n",
                        slot = target_ctx.slot_field,
                        single = target_ctx.single
                    ));
                }
            }
            FeatureKind::CrossReference => {
                let target = match &feature.type_ {
                    TypeRef::Class { name, .. } => name,
                    _ => continue,
                };
                let target_ctx = unit.class(target)?;
                if feature.multiplicity.is_many() {
                    e.push_str(&format!(
                        "        object.insert(\n\
                         \x20           {key:?}.to_string(),\n\
                         \x20           serde_json::Value::Array(\n\
                         \x20               obj.{field}\n\
                         \x20                   .iter()\n\
                         \x20                   .map(|child| ref_object(&ids.{}[child]))\n\
                         \x20                   .collect(),\n\
                         \x20           ),\n\
                         \x20       );\n",
                        target_ctx.slot_field
                    ));
                } else {
                    e.push_str(&format!(
                        "        if let Some(child) = &obj.{field} {{\n\
                         \x20           object.insert({key:?}.to_string(), ref_object(&ids.{}[child]));\n\
                         \x20       }}\n",
                        target_ctx.slot_field
                    ));
                }
            }
            FeatureKind::Container => {
                // Never serialized: reconstructed from nesting on load.
            }
        }
    }
    e.push_str("        serde_json::Value::Object(object)\n    }\n\n");
    Ok(())
}

/// One literal-name loader per enum referenced by any attribute, each in its
/// own `impl` block.
fn emit_enum_loaders(e: &mut String, unit: &Unit<'_>) -> anyhow::Result<()> {
    let mut emitted: std::collections::BTreeSet<String> = Default::default();
    for class in &unit.classes {
        for feature in &class.class.features {
            let TypeRef::Enum { name, .. } = &feature.type_ else {
                continue;
            };
            if !emitted.insert(name.clone()) {
                continue;
            }
            let Some(enum_def) = unit.enums.iter().find(|e_| e_.name == *name) else {
                anyhow::bail!("enum '{name}' not found in model");
            };
            let enum_name = rust_ident(name);
            e.push_str(&format!(
                "impl {enum_name} {{\n\
                 \x20   #[allow(dead_code)]\n\
                 \x20   /// Resolves a canonical instance literal name.\n\
                 \x20   fn from_instance_name(value: &str) -> Result<Self, rex_runtime::RexError> {{\n\
                 \x20       match value {{\n"
            ));
            for literal in &enum_def.literals {
                e.push_str(&format!(
                    "            {:?} => Ok(Self::{}),\n",
                    literal.name,
                    rust_ident(&literal.name)
                ));
            }
            e.push_str(&format!(
                "            other => Err(instance_error(format!(\"unknown {enum_name} literal {{other:?}}\"))),\n\
                 \x20       }}\n\
                 \x20   }}\n\
                 }}\n\n"
            ));
        }
    }
    Ok(())
}

/// First load pass, per object: validate and map the canonical id to a fresh
/// key. Registration is flat (no containment recursion): the loader collects
/// every document object first and registers per class in id-ordinal order.
fn emit_class_register(e: &mut String, class: &ClassCtx<'_>) -> anyhow::Result<()> {
    let struct_name = rust_ident(&class.class.name);
    e.push_str(&format!(
        "    fn register_{single}(\n\
         \x20       object: &serde_json::Value,\n\
         \x20       registry: &mut InstanceRegistry,\n\
         \x20       resource: &mut Resource,\n\
         \x20   ) -> Result<(), rex_runtime::RexError> {{\n\
         \x20       let map = object\n\
         \x20           .as_object()\n\
         \x20           .ok_or_else(|| instance_error(\"objects must be JSON objects\".to_string()))?;\n\
         \x20       let type_name = map\n\
         \x20           .get(rex_runtime::KEY_TYPE)\n\
         \x20           .and_then(|v| v.as_str())\n\
         \x20           .ok_or_else(|| instance_error(\"object is missing $type\".to_string()))?;\n\
         \x20       if type_name != {struct_name:?} {{\n\
         \x20           return Err(instance_error(format!(\n\
         \x20               \"expected $type \\\"{struct_name}\\\", found {{type_name:?}}\"\n\
         \x20           )));\n\
         \x20       }}\n\
         \x20       let id = id_of_object(object)?;\n",
        single = class.single,
    ));
    e.push_str(&format!(
        "        if registry.{}.contains_key(id) {{\n\
         \x20           return Err(instance_error(format!(\"duplicate $id {{id:?}}\")));\n\
         \x20       }}\n\
         \x20       let key = resource.{}.insert({struct_name}::default());\n\
         \x20       registry.{}.insert(id.to_string(), key);\n\
         \x20       Ok(())\n\
         \x20   }}\n\n",
        class.slot_field, class.slot_field, class.slot_field
    ));
    Ok(())
}

/// Second load pass: assign attribute fields, resolve `$ref`s, embed
/// containment children into their slots, and reconstruct container
/// opposites from nesting.
fn emit_class_fill(e: &mut String, unit: &Unit<'_>, class: &ClassCtx<'_>) -> anyhow::Result<()> {
    let struct_name = rust_ident(&class.class.name);
    e.push_str(&format!(
        "    fn fill_{single}(\n\
         \x20       object: &serde_json::Value,\n\
         \x20       registry: &InstanceRegistry,\n\
         \x20       resource: &mut Resource,\n\
         \x20   ) -> Result<(), rex_runtime::RexError> {{\n\
         \x20       let map = object\n\
         \x20           .as_object()\n\
         \x20           .ok_or_else(|| instance_error(\"objects must be JSON objects\".to_string()))?;\n\
         \x20       let id = id_of_object(object)?;\n",
        single = class.single,
    ));
    e.push_str(&format!(
        "        let key = *registry\n\
         \x20           .{}\n\
         \x20           .get(id)\n\
         \x20           .ok_or_else(|| instance_error(format!(\"object {{id:?}} was never registered\")))?;\n",
        class.slot_field
    ));
    e.push_str("        for (json_key, value) in map {\n");
    e.push_str("            match json_key.as_str() {\n");
    e.push_str("                rex_runtime::KEY_TYPE | rex_runtime::KEY_ID => {}\n");
    for feature in &class.class.features {
        if feature.is_derived {
            continue;
        }
        let field = rust_ident(&crate::naming::snake_case(&feature.name));
        let key = json_key(feature);
        match feature.kind {
            FeatureKind::Attribute => {
                let load = |v: &str| load_expr(&feature.type_, v, &format!("{struct_name}.{key}"));
                if feature.multiplicity.is_many() {
                    e.push_str(&format!(
                        "                {key:?} => {{\n\
                         \x20                   let array = value\n\
                         \x20                       .as_array()\n\
                         \x20                       .ok_or_else(|| instance_error(format!(\"{struct_name}.{key} must be an array\")))?;\n\
                         \x20                   let mut parsed = Vec::with_capacity(array.len());\n\
                         \x20                   for element in array {{\n\
                         \x20                       parsed.push({});\n\
                         \x20                   }}\n\
                         \x20                   if let Some(obj) = resource.{}.get_mut(key) {{\n\
                         \x20                       obj.{field} = parsed;\n\
                         \x20                   }}\n\
                         \x20               }}\n",
                        load("element"),
                        class.slot_field,
                    ));
                } else {
                    // Optional scalars skip JSON nulls (absence is the
                    // default) and always wrap the loaded value in `Some`;
                    // `value` is a `&serde_json::Value`, never an `Option`.
                    let (assign, presence) = if feature.multiplicity.lower == 0 {
                        (
                            format!("Some({})", load("value")),
                            "                if !value.is_null() {\n",
                        )
                    } else {
                        (load("value"), "                if !value.is_null() {\n")
                    };
                    e.push_str(&format!(
                        "                {key:?} => {{\n\
                         \x20{presence}\
                         \x20                   if let Some(obj) = resource.{}.get_mut(key) {{\n\
                         \x20                       obj.{field} = {assign};\n\
                         \x20                   }}\n\
                         \x20               }}\n\
                         \x20           }}\n",
                        class.slot_field,
                    ));
                }
            }
            FeatureKind::Containment => {
                let target = match &feature.type_ {
                    TypeRef::Class { name, .. } => name,
                    _ => continue,
                };
                let target_ctx = unit.class(target)?;
                let target_single = &target_ctx.single;
                let target_slot = &target_ctx.slot_field;
                let container_field =
                    container_field_of(target_ctx, class, feature.opposite.as_ref())?;
                if feature.multiplicity.is_many() {
                    let mut block = format!(
                        "                {key:?} => {{\n\
                         \x20                   let array = value\n\
                         \x20                       .as_array()\n\
                         \x20                       .ok_or_else(|| instance_error(format!(\"{struct_name}.{key} must be an array\")))?;\n\
                         \x20                   for element in array {{\n\
                         \x20                       let child = Self::{target_single}_id_of(element, registry)?;\n\
                         \x20                       if let Some(obj) = resource.{slot}.get_mut(key) {{\n\
                         \x20                           if !obj.{field}.contains(&child) {{\n\
                         \x20                               obj.{field}.push(child);\n\
                         \x20                           }}\n\
                         \x20                       }}\n",
                        slot = class.slot_field,
                    );
                    if !container_field.is_empty() {
                        block.push_str(&format!(
                            "                        if let Some(child_obj) = resource.{target_slot}.get_mut(child) {{\n\
                             \x20                           child_obj.{container_field} = Some(key);\n\
                             \x20                       }}\n"
                        ));
                    }
                    block.push_str("                    }\n                }\n");
                    e.push_str(&block);
                } else {
                    let mut block = format!(
                        "                {key:?} => {{\n\
                         \x20                   let child = Self::{target_single}_id_of(value, registry)?;\n\
                         \x20                   if let Some(obj) = resource.{slot}.get_mut(key) {{\n\
                         \x20                       obj.{field} = Some(child);\n\
                         \x20                   }}\n",
                        slot = class.slot_field,
                    );
                    if !container_field.is_empty() {
                        block.push_str(&format!(
                            "                    if let Some(child_obj) = resource.{target_slot}.get_mut(child) {{\n\
                             \x20                       child_obj.{container_field} = Some(key);\n\
                             \x20                   }}\n"
                        ));
                    }
                    block.push_str("                }\n");
                    e.push_str(&block);
                }
            }
            FeatureKind::CrossReference => {
                let target = match &feature.type_ {
                    TypeRef::Class { name, .. } => name,
                    _ => continue,
                };
                let target_ctx = unit.class(target)?;
                let target_single = &target_ctx.single;
                // Both ends of a cross reference are serialized; each side is
                // filled from its own JSON (documented contract).
                if feature.multiplicity.is_many() {
                    e.push_str(&format!(
                        "                {key:?} => {{\n\
                         \x20                   let array = value\n\
                         \x20                       .as_array()\n\
                         \x20                       .ok_or_else(|| instance_error(format!(\"{struct_name}.{key} must be an array\")))?;\n\
                         \x20                   for element in array {{\n\
                         \x20                       let target = Self::{target_single}_id_of(element, registry)?;\n\
                         \x20                       if let Some(obj) = resource.{slot}.get_mut(key) {{\n\
                         \x20                           if !obj.{field}.contains(&target) {{\n\
                         \x20                               obj.{field}.push(target);\n\
                         \x20                           }}\n\
                         \x20                       }}\n\
                         \x20                   }}\n\
                         \x20               }}\n",
                        slot = class.slot_field,
                    ));
                } else {
                    e.push_str(&format!(
                        "                {key:?} => {{\n\
                         \x20                   let target = Self::{target_single}_id_of(value, registry)?;\n\
                         \x20                   if let Some(obj) = resource.{slot}.get_mut(key) {{\n\
                         \x20                       obj.{field} = Some(target);\n\
                         \x20                   }}\n\
                         \x20               }}\n",
                        slot = class.slot_field,
                    ));
                }
            }
            FeatureKind::Container => {
                let key = json_key(feature);
                e.push_str(&format!(
                    "                {key:?} => {{\n\
                     \x20                   return Err(instance_error(format!(\n\
                     \x20                       \"container feature {struct_name}.{key} must not be serialized; it is reconstructed from nesting\"\n\
                     \x20                   )));\n\
                     \x20               }}\n",
                ));
            }
        }
    }
    e.push_str(&format!(
        "                other => {{\n\
         \x20                   return Err(instance_error(format!(\n\
         \x20                       \"unknown feature {{other}} on {struct_name}\"\n\
         \x20                   )));\n\
         \x20               }}\n\
         \x20           }}\n\
         \x20       }}\n\
         \x20       Ok(())\n\
         \x20   }}\n\n"
    ));
    Ok(())
}

/// The container feature field on `target_class` that points back at
/// `owner_class` for a containment feature, for reconstructing the opposite
/// when loading. The containment's `opposite` names the container feature
/// exactly; matching by owner class alone would reconstruct every
/// containment onto the first container feature when a child class carries
/// several container ends back to the same owner (e.g. a many and a single
/// containment of the same child class).
fn container_field_of<'a>(
    target_class: &ClassCtx<'a>,
    owner_class: &ClassCtx<'_>,
    opposite: Option<&rex_ir::OppositeRef>,
) -> anyhow::Result<String> {
    let wanted = opposite.map(|opposite| opposite.feature.as_str());
    for feature in &target_class.class.features {
        if feature.kind == FeatureKind::Container {
            if let Some(opposite) = &feature.opposite {
                if opposite.class == owner_class.class.name
                    && wanted.is_none_or(|name| name == feature.name)
                {
                    return Ok(rust_ident(&crate::naming::snake_case(&feature.name)));
                }
            }
        }
    }
    // No container back-pointer declared; references are one-way.
    Ok(String::new())
}

fn emit_entry_points(e: &mut String, unit: &Unit<'_>) -> anyhow::Result<()> {
    // Per-class typed id resolvers used by fill.
    for class in &unit.classes {
        e.push_str(&format!(
            "    #[allow(dead_code)]\n\
             \x20   fn {single}_id_of(value: &serde_json::Value, registry: &InstanceRegistry) -> Result<{id_type}, rex_runtime::RexError> {{\n\
             \x20       let id = value\n\
             \x20           .get(rex_runtime::KEY_REF)\n\
             \x20           .or_else(|| value.get(rex_runtime::KEY_ID))\n\
             \x20           .and_then(|v| v.as_str())\n\
             \x20           .ok_or_else(|| instance_error(\"expected an object with $id or a {{\\\"$ref\\\": ...}} link\".to_string()))?;\n\
             \x20       registry\n\
             \x20           .{slot}\n\
             \x20           .get(id)\n\
             \x20           .copied()\n\
             \x20           .ok_or_else(|| instance_error(format!(\"unresolved reference {{id:?}}\")))\n\
             \x20   }}\n\n",
            single = class.single,
            id_type = class.id_type,
            slot = class.slot_field,
        ));
    }

    // Per-class collectors: append the object and every embedded containment
    // object, depth-first in document order. Containment entries that are
    // `{"$ref": ...}` links (no `$type`) are skipped here; their embedding is
    // resolved during fill.
    for class in &unit.classes {
        e.push_str(&format!(
            "    fn collect_{single}<'a>(\n\
             \x20       object: &'a serde_json::Value,\n\
             \x20       collected: &mut Vec<&'a serde_json::Value>,\n\
             \x20   ) {{\n\
             \x20       collected.push(object);\n",
            single = class.single,
        ));
        for feature in &class.class.features {
            if feature.kind != FeatureKind::Containment {
                continue;
            }
            let target = match &feature.type_ {
                TypeRef::Class { name, .. } => name,
                _ => continue,
            };
            let target_ctx = unit.class(target)?;
            let key = json_key(feature);
            if feature.multiplicity.is_many() {
                e.push_str(&format!(
                    "        if let Some(children) = object.get({key:?}).and_then(|v| v.as_array()) {{\n\
                     \x20           for child in children {{\n\
                     \x20               if child.get(rex_runtime::KEY_TYPE).is_some() {{\n\
                     \x20                   Self::collect_{}(child, collected);\n\
                     \x20               }}\n\
                     \x20           }}\n\
                     \x20       }}\n",
                    target_ctx.single
                ));
            } else {
                e.push_str(&format!(
                    "        if let Some(child) = object.get({key:?}) {{\n\
                     \x20           if child.get(rex_runtime::KEY_TYPE).is_some() {{\n\
                     \x20               Self::collect_{}(child, collected);\n\
                     \x20           }}\n\
                     \x20       }}\n",
                    target_ctx.single
                ));
            }
        }
        e.push_str("    }\n\n");
    }

    e.push_str(
        "    /// Serializes the resource to the canonical instance JSON format.\n\
         \x20   pub fn to_instance_json(&self) -> String {\n\
         \x20       let mut ids = InstanceIds::default();\n\
         \x20       self.assign_instance_ids(&mut ids);\n\
         \x20       let mut visited = std::collections::HashSet::new();\n\
         \x20       let mut objects = Vec::new();\n",
    );
    for class in &unit.classes {
        e.push_str(&format!(
            "        for (key, _) in self.{}.iter() {{\n\
             \x20           if !visited.contains(&ids.{}[&key]) {{\n\
             \x20               objects.push(self.{}_to_json(key, &ids, &mut visited));\n\
             \x20           }}\n\
             \x20       }}\n",
            class.slot_field, class.slot_field, class.single
        ));
    }
    e.push_str(&format!(
        "        let mut root = serde_json::Map::new();\n\
         \x20       root.insert(\n\
         \x20           rex_runtime::KEY_TYPE.to_string(),\n\
         \x20           serde_json::Value::String(rex_runtime::INSTANCE_TYPE.to_string()),\n\
         \x20       );\n\
         \x20       root.insert(\n\
         \x20           \"formatVersion\".to_string(),\n\
         \x20           serde_json::Value::from(rex_runtime::INSTANCE_FORMAT_VERSION),\n\
         \x20       );\n\
         \x20       root.insert(\"model\".to_string(), serde_json::Value::String({type_name:?}.to_string()));\n\
         \x20       root.insert(\"objects\".to_string(), serde_json::Value::Array(objects));\n\
         \x20       serde_json::to_string_pretty(&serde_json::Value::Object(root)).unwrap_or_default()\n\
         \x20   }}\n\n",
        type_name = unit.type_name,
    ));

    e.push_str(&format!(
        "    /// Loads a resource from canonical instance JSON.\n\
         \x20   pub fn from_instance_json(json: &str) -> Result<Self, rex_runtime::RexError> {{\n\
         \x20       let value: serde_json::Value =\n\
         \x20           serde_json::from_str(json).map_err(|error| instance_error(format!(\"invalid JSON: {{error}}\")))?;\n\
         \x20       let root = value\n\
         \x20           .as_object()\n\
         \x20           .ok_or_else(|| instance_error(\"instance document must be a JSON object\".to_string()))?;\n\
         \x20       let document_type = root\n\
         \x20           .get(rex_runtime::KEY_TYPE)\n\
         \x20           .and_then(|v| v.as_str())\n\
         \x20           .ok_or_else(|| instance_error(\"document is missing $type\".to_string()))?;\n\
         \x20       if document_type != rex_runtime::INSTANCE_TYPE {{\n\
         \x20           return Err(instance_error(format!(\n\
         \x20               \"expected $type {{:?}}, found {{document_type:?}}\",\n\
         \x20               rex_runtime::INSTANCE_TYPE\n\
         \x20           )));\n\
         \x20       }}\n\
         \x20       let format_version = root\n\
         \x20           .get(\"formatVersion\")\n\
         \x20           .and_then(|v| v.as_u64())\n\
         \x20           .ok_or_else(|| instance_error(\"document is missing formatVersion\".to_string()))?;\n\
         \x20       if format_version != rex_runtime::INSTANCE_FORMAT_VERSION as u64 {{\n\
         \x20           return Err(instance_error(format!(\n\
         \x20               \"unsupported instance formatVersion {{format_version}}\"\n\
         \x20           )));\n\
         \x20       }}\n\
         \x20       let model = root\n\
         \x20           .get(\"model\")\n\
         \x20           .and_then(|v| v.as_str())\n\
         \x20           .ok_or_else(|| instance_error(\"document is missing model\".to_string()))?;\n\
         \x20       if model != {type_name:?} {{\n\
         \x20           return Err(instance_error(format!(\n\
         \x20               \"instance is for model {{model:?}}, expected \\\"{type_name}\\\"\"\n\
         \x20           )));\n\
         \x20       }}\n\
         \x20       let objects = root\n\
         \x20           .get(\"objects\")\n\
         \x20           .and_then(|v| v.as_array())\n\
         \x20           .ok_or_else(|| instance_error(\"document is missing objects\".to_string()))?;\n\
         \x20       // Flatten every document object (containment is inlined, so most\n\
         \x20       // objects nest inside their owner), validating $type en route.\n\
         \x20       let mut collected: Vec<&serde_json::Value> = Vec::new();\n\
         \x20       for object in objects {{\n\
         \x20           let type_name = object\n\
         \x20               .get(rex_runtime::KEY_TYPE)\n\
         \x20               .and_then(|v| v.as_str())\n\
         \x20               .ok_or_else(|| instance_error(\"object is missing $type\".to_string()))?;\n\
         \x20           match type_name {{\n",
        type_name = unit.type_name,
    ));
    for class in &unit.classes {
        e.push_str(&format!(
            "                {:?} => Self::collect_{}(object, &mut collected),\n",
            rust_ident(&class.class.name),
            class.single
        ));
    }
    e.push_str(
        "                other => {\n\
         \x20                   return Err(instance_error(format!(\"unknown $type {other:?}\")));\n\
         \x20               }\n\
         \x20           }\n\
         \x20       }\n\
         \x20       // Canonical ids encode per-class insertion order (\"<singular>/<n>\")\n\
         \x20       // while the document groups objects by owner, so register and fill\n\
         \x20       // in id-ordinal order: the loaded arena preserves the source\n\
         \x20       // per-class insertion order — the precondition for round-trip\n\
         \x20       // identity. Objects with non-canonical ids keep their document\n\
         \x20       // order, after the canonical ones (the sort is stable).\n\
         \x20       collected.sort_by(|left, right| {\n\
         \x20           instance_id_sort_key(left).cmp(&instance_id_sort_key(right))\n\
         \x20       });\n\
         \x20       let mut resource = Resource::default();\n\
         \x20       let mut registry = InstanceRegistry::default();\n\
         \x20       for object in &collected {\n\
         \x20           let type_name = object\n\
         \x20               .get(rex_runtime::KEY_TYPE)\n\
         \x20               .and_then(|v| v.as_str())\n\
         \x20               .ok_or_else(|| instance_error(\"object is missing $type\".to_string()))?;\n\
         \x20           match type_name {\n",
    );
    for class in &unit.classes {
        e.push_str(&format!(
            "                {:?} => Self::register_{}(object, &mut registry, &mut resource)?,\n",
            rust_ident(&class.class.name),
            class.single
        ));
    }
    e.push_str(
        "                other => {\n\
         \x20                   return Err(instance_error(format!(\"unknown $type {other:?}\")));\n\
         \x20               }\n\
         \x20           }\n\
         \x20       }\n\
         \x20       for object in &collected {\n\
         \x20           let type_name = object\n\
         \x20               .get(rex_runtime::KEY_TYPE)\n\
         \x20               .and_then(|v| v.as_str())\n\
         \x20               .ok_or_else(|| instance_error(\"object is missing $type\".to_string()))?;\n\
         \x20           match type_name {\n",
    );
    for class in &unit.classes {
        e.push_str(&format!(
            "                {:?} => Self::fill_{}(object, &registry, &mut resource)?,\n",
            rust_ident(&class.class.name),
            class.single
        ));
    }
    e.push_str(
        "                other => {\n\
         \x20                   return Err(instance_error(format!(\"unknown $type {other:?}\")));\n\
         \x20               }\n\
         \x20           }\n\
         \x20       }\n\
         \x20       Ok(resource)\n\
         \x20   }\n",
    );
    Ok(())
}
