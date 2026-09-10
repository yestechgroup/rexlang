//! Rust code generation from the Core IR.
//!
//! The generator consumes a fully resolved [`rex_ir::Model`] and emits a
//! single `models.rs` implementing the **arena + typed ids** representation
//! required by the project design: bidirectional references and containment
//! cycles cannot be expressed with Rust references, so every object lives in
//! a per-class [`rex_runtime::slotmap::SlotMap`] inside a generated
//! `Resource`, cross references are generated `XxxId` keys, navigation takes
//! `&Resource`, and every mutator takes `&mut Resource` and maintains
//! `opposite` pairs atomically.
//!
//! Mutator shapes per relation pair (cardinalities of the two ends):
//! - many ↔ many: `add`/`remove` on both ends, pushing/removing on both sides.
//! - single ↔ many: `set` on the single end, `add`/`remove` on the many end;
//!   both keep the pair consistent, including detaching from a previous
//!   holder.
//! - single ↔ single: `set` on both ends with full detach/reattach logic.
//! - one-way containment (no opposite): plain owner-side `add`/`remove`.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use anyhow::bail;
use rex_ir::{
    ClassDef, DatatypeDef, DefaultValue, EnumDef, Feature, FeatureKind, Model, PrimitiveType,
    TypeRef,
};

use crate::naming::{rust_ident, slotmap_field, snake_case};

/// Generates the Rust code for `model`.
///
/// Returns a map of relative file path to file contents. Currently emits a
/// single `models.rs`.
pub fn generate(model: &Model) -> anyhow::Result<BTreeMap<String, String>> {
    let code = generate_unit(model)?;
    let mut files = BTreeMap::new();
    files.insert("models.rs".to_string(), code);
    Ok(files)
}

/// Generates the Rust code for `model` and writes it into `out_dir`.
pub fn generate_to_dir(model: &Model, out_dir: &Path) -> anyhow::Result<()> {
    let files = generate(model)?;
    for (relative, contents) in &files {
        let path = out_dir.join(relative);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, contents)?;
    }
    Ok(())
}

/// A generated class with everything the emitter needs precomputed.
struct ClassCtx<'a> {
    class: &'a ClassDef,
    id_type: String,
    slot_field: String,
    single: String,
}

/// All resolved inputs for one generation unit.
struct Unit<'a> {
    type_name: String,
    classes: Vec<ClassCtx<'a>>,
    enums: Vec<&'a EnumDef>,
    datatypes: Vec<&'a DatatypeDef>,
}

impl<'a> Unit<'a> {
    fn class(&self, name: &str) -> anyhow::Result<&ClassCtx<'a>> {
        self.classes
            .iter()
            .find(|c| c.class.name == name)
            .ok_or_else(|| anyhow::anyhow!("class '{name}' not found in model"))
    }

    fn first_enum_literal(&self, enum_name: &str) -> anyhow::Result<&'a rex_ir::EnumLiteral> {
        let enum_def = self
            .enums
            .iter()
            .find(|e| e.name == enum_name)
            .ok_or_else(|| anyhow::anyhow!("enum '{enum_name}' not found in model"))?;
        enum_def
            .literals
            .first()
            .ok_or_else(|| anyhow::anyhow!("enum '{enum_name}' has no literals"))
    }
}

fn collect_unit(model: &Model) -> anyhow::Result<Unit<'_>> {
    let mut classes = Vec::new();
    let mut seen = BTreeSet::new();
    let mut enums = Vec::new();
    let mut datatypes = Vec::new();
    for package in &model.packages {
        enums.extend(package.enums.iter());
        datatypes.extend(package.datatypes.iter());
        for class in &package.classes {
            if !seen.insert(class.name.clone()) {
                bail!(
                    "duplicate class name '{}' across packages; the Rust backend flattens \
                     packages into one module and requires unique class names",
                    class.name
                );
            }
            classes.push(ClassCtx {
                class,
                id_type: format!("{}Id", rust_ident(&class.name)),
                slot_field: slotmap_field(&class.name),
                single: snake_case(&class.name),
            });
        }
    }
    let type_name = model
        .packages
        .first()
        .map(|p| p.name.clone())
        .unwrap_or_else(|| "model".to_string());
    Ok(Unit {
        type_name,
        classes,
        enums,
        datatypes,
    })
}

fn value_type(type_: &TypeRef) -> anyhow::Result<String> {
    Ok(match type_ {
        TypeRef::Primitive(primitive) => match primitive {
            PrimitiveType::String => "String".to_string(),
            PrimitiveType::Int => "i32".to_string(),
            PrimitiveType::Long => "i64".to_string(),
            PrimitiveType::Short => "i16".to_string(),
            PrimitiveType::Float => "f32".to_string(),
            PrimitiveType::Double => "f64".to_string(),
            PrimitiveType::Boolean => "bool".to_string(),
            PrimitiveType::Byte => "i8".to_string(),
            PrimitiveType::Char => "char".to_string(),
        },
        TypeRef::Class { name, .. }
        | TypeRef::Enum { name, .. }
        | TypeRef::Datatype { name, .. } => rust_ident(name),
        TypeRef::Interface { name, .. } => {
            bail!("interfaces are not materialized by the Rust backend yet ('{name}')")
        }
    })
}

fn is_reference(kind: FeatureKind) -> bool {
    matches!(
        kind,
        FeatureKind::Containment | FeatureKind::CrossReference | FeatureKind::Container
    )
}

fn feature_doc(feature: &Feature) -> &'static str {
    match feature.kind {
        FeatureKind::Attribute => "attribute",
        FeatureKind::Containment => "contains",
        FeatureKind::CrossReference => "refers",
        FeatureKind::Container => "container",
    }
}

/// The stored field type for a feature.
///
/// Many features store `Vec<T>`; references store ids (`Option<T>` when
/// optional — ids cannot be defaulted); scalar attributes are `T` when
/// required (`lower >= 1`) and `Option<T>` when optional.
fn field_type(_unit: &Unit<'_>, feature: &Feature) -> anyhow::Result<String> {
    if feature.is_derived {
        bail!("derived feature '{}' should not be materialized", feature.name);
    }
    let inner = match feature.kind {
        FeatureKind::Attribute => value_type(&feature.type_)?,
        FeatureKind::Containment | FeatureKind::CrossReference | FeatureKind::Container => {
            match &feature.type_ {
                TypeRef::Class { name, .. } => format!("{}Id", rust_ident(name)),
                other => bail!(
                    "feature '{}' must have a class type, found {}",
                    feature.name,
                    other.qualified_name().unwrap_or_else(|| "primitive".into())
                ),
            }
        }
    };
    Ok(if feature.multiplicity.is_many() {
        format!("Vec<{inner}>")
    } else if is_reference(feature.kind) || feature.multiplicity.lower == 0 {
        format!("Option<{inner}>")
    } else {
        inner
    })
}

/// The initial value expression for a required scalar attribute field.
fn default_expr(unit: &Unit<'_>, feature: &Feature) -> anyhow::Result<String> {
    if let Some(default) = &feature.default {
        return Ok(match default {
            DefaultValue::String(value) => format!("{value:?}.to_string()"),
            DefaultValue::Int(value) => value.to_string(),
            DefaultValue::Bool(value) => value.to_string(),
            DefaultValue::EnumLiteral(literal) => {
                let enum_name = match &feature.type_ {
                    TypeRef::Enum { name, .. } => rust_ident(name),
                    other => bail!(
                        "default EnumLiteral on non-enum feature '{}' ({})",
                        feature.name,
                        other.qualified_name().unwrap_or_default()
                    ),
                };
                format!("{enum_name}::{}", rust_ident(literal))
            }
        });
    }
    Ok(match &feature.type_ {
        TypeRef::Primitive(primitive) => match primitive {
            PrimitiveType::String => "String::new()".to_string(),
            PrimitiveType::Int | PrimitiveType::Long | PrimitiveType::Short
            | PrimitiveType::Byte => "0".to_string(),
            PrimitiveType::Float | PrimitiveType::Double => "0.0".to_string(),
            PrimitiveType::Boolean => "false".to_string(),
            PrimitiveType::Char => "'\\0'".to_string(),
        },
        TypeRef::Enum { name, .. } => {
            let first = unit.first_enum_literal(name)?;
            format!("{}::{}", rust_ident(name), rust_ident(&first.name))
        }
        TypeRef::Datatype { name, .. } => format!("{}::default()", rust_ident(name)),
        TypeRef::Class { name, .. } => bail!(
            "required attribute '{}' has class type '{name}' with no default",
            feature.name
        ),
        TypeRef::Interface { name, .. } => bail!(
            "required attribute '{}' has interface type '{name}'",
            feature.name
        ),
    })
}

fn generate_unit(model: &Model) -> anyhow::Result<String> {
    let unit = collect_unit(model)?;
    let mut e = String::new();
    emit_header(&mut e, model, &unit);
    emit_new_key_types(&mut e, &unit);
    e.push('\n');
    emit_datatypes(&mut e, &unit);
    e.push('\n');
    emit_enums(&mut e, &unit)?;
    for class in &unit.classes {
        e.push('\n');
        emit_struct(&mut e, &unit, class)?;
    }
    e.push('\n');
    emit_resource(&mut e, &unit)?;
    Ok(e)
}

fn emit_header(e: &mut String, model: &Model, unit: &Unit<'_>) {
    e.push_str("//! Generated by rexlang — DO NOT EDIT.\n//!\n");
    e.push_str(&format!(
        "//! Source model: formatVersion {}, rexVersion {}.\n//!\n",
        model.format_version,
        model.rex_version.as_deref().unwrap_or("unknown")
    ));
    e.push_str("//! Tier 0 notes:\n");
    e.push_str(
        "//! - operations are abstract hooks: implement them as hand-written methods\n\
         //!   in your own modules (generated code never contains operation bodies);\n",
    );
    if unit.classes.iter().any(|c| c.class.features.iter().any(|f| f.is_derived)) {
        e.push_str(
            "//! - derived features are declared in the IR but not materialized as fields;\n",
        );
    }
    e.push_str(
        "//! - datatype target bindings are recorded as doc comments on the newtypes,\n\
         //!   never as dependencies; interface conformance is not materialized yet.\n",
    );
    if unit.classes.iter().any(|c| !c.class.extends.is_empty()) {
        e.push_str(
            "//! - inheritance declared via `extends` is not materialized in this milestone.\n",
        );
    }
    e.push_str("\nuse rex_runtime::slotmap::SlotMap;\n");
}

fn emit_new_key_types(e: &mut String, unit: &Unit<'_>) {
    if unit.classes.is_empty() {
        return;
    }
    e.push_str("rex_runtime::slotmap::new_key_type! {\n");
    for class in &unit.classes {
        e.push_str(&format!("    pub struct {};\n", class.id_type));
    }
    e.push_str("}\n");
}

fn emit_datatypes(e: &mut String, unit: &Unit<'_>) {
    for datatype in &unit.datatypes {
        e.push_str("/// Opaque datatype newtype.\n");
        if let Some(platform) = &datatype.platform {
            e.push_str(&format!("/// Declared `wraps {platform}`.\n"));
        }
        if datatype.target_bindings.is_empty() {
            e.push_str("/// No target bindings declared.\n");
        } else {
            e.push_str("/// Target bindings:\n");
            for (target, type_name) in &datatype.target_bindings {
                e.push_str(&format!("/// - {target}: `{type_name}`\n"));
            }
        }
        e.push_str("#[derive(Debug, Clone, PartialEq, Eq, Default, Hash)]\n");
        e.push_str(&format!(
            "pub struct {}(pub String);\n",
            rust_ident(&datatype.name)
        ));
    }
}

fn emit_enums(e: &mut String, unit: &Unit<'_>) -> anyhow::Result<()> {
    for enum_def in &unit.enums {
        if enum_def.literals.is_empty() {
            bail!("enum '{}' has no literals", enum_def.name);
        }
        let name = rust_ident(&enum_def.name);
        e.push_str("/// Generated enum.\n");
        e.push_str("#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]\n");
        e.push_str("#[repr(i64)]\n");
        e.push_str(&format!("pub enum {name} {{\n"));
        for literal in &enum_def.literals {
            if let Some(label) = &literal.label {
                e.push_str(&format!(
                    "    /// Literal value {}, label {:?}.\n",
                    literal.value, label
                ));
            }
            e.push_str(&format!(
                "    {} = {},\n",
                rust_ident(&literal.name),
                literal.value
            ));
        }
        e.push_str("}\n\n");

        e.push_str(&format!("impl {name} {{\n"));
        e.push_str("    /// The declared literal name.\n");
        e.push_str("    pub fn name(self) -> &'static str {\n        match self {\n");
        for literal in &enum_def.literals {
            e.push_str(&format!(
                "            Self::{} => {:?},\n",
                rust_ident(&literal.name),
                literal.name
            ));
        }
        e.push_str("        }\n    }\n\n");
        e.push_str("    /// The human-readable label if declared, else the literal name.\n");
        e.push_str("    pub fn label(self) -> &'static str {\n        match self {\n");
        for literal in &enum_def.literals {
            let label = literal.label.clone().unwrap_or_else(|| literal.name.clone());
            e.push_str(&format!(
                "            Self::{} => {:?},\n",
                rust_ident(&literal.name),
                label
            ));
        }
        e.push_str("        }\n    }\n\n");
        e.push_str("    /// The declared integer value.\n");
        e.push_str("    pub fn value(self) -> i64 {\n        self as i64\n    }\n}\n\n");

        // TryFrom<i64>: deduplicate repeated values (first literal wins).
        let mut seen_values: BTreeSet<i64> = BTreeSet::new();
        e.push_str(&format!("impl TryFrom<i64> for {name} {{\n"));
        e.push_str("    type Error = rex_runtime::RexError;\n\n");
        e.push_str("    fn try_from(value: i64) -> Result<Self, Self::Error> {\n");
        e.push_str("        match value {\n");
        for literal in &enum_def.literals {
            if seen_values.insert(literal.value) {
                e.push_str(&format!(
                    "            {} => Ok(Self::{}),\n",
                    literal.value,
                    rust_ident(&literal.name)
                ));
            }
        }
        e.push_str(&format!(
            "            other => Err(rex_runtime::RexError::InvalidInstance {{\n                message: format!(\"unknown {name} value {{other}}\"),\n            }}),\n"
        ));
        e.push_str("        }\n    }\n}\n\n");

        e.push_str(&format!("impl Default for {name} {{\n"));
        e.push_str("    fn default() -> Self {\n");
        e.push_str(&format!(
            "        Self::{}\n",
            rust_ident(&enum_def.literals[0].name)
        ));
        e.push_str("    }\n}\n\n");

        e.push_str(&format!("impl std::fmt::Display for {name} {{\n"));
        e.push_str(
            "    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {\n\
             \x20       f.write_str(self.label())\n    }\n}\n",
        );
    }
    Ok(())
}

fn emit_struct(e: &mut String, unit: &Unit<'_>, class: &ClassCtx<'_>) -> anyhow::Result<()> {
    let name = rust_ident(&class.class.name);
    if !class.class.extends.is_empty() {
        let extends: Vec<String> = class
            .class
            .extends
            .iter()
            .map(|t| t.qualified_name().unwrap_or_else(|| "primitive".into()))
            .collect();
        e.push_str(&format!("/// Declared `extends {}`.\n", extends.join(", ")));
    }
    e.push_str("/// Generated class struct.\n");
    e.push_str("#[derive(Debug, Clone, PartialEq)]\n");
    e.push_str(&format!("pub struct {name} {{\n"));
    for feature in &class.class.features {
        if feature.is_derived {
            continue;
        }
        e.push_str(&format!(
            "    /// feature id {}: {}\n",
            feature.id,
            feature_doc(feature)
        ));
        e.push_str(&format!(
            "    pub {}: {},\n",
            rust_ident(&snake_case(&feature.name)),
            field_type(unit, feature)?
        ));
    }
    e.push_str("}\n\n");

    e.push_str(&format!("impl Default for {name} {{\n"));
    e.push_str("    fn default() -> Self {\n        Self {\n");
    for feature in &class.class.features {
        if feature.is_derived {
            continue;
        }
        let field = rust_ident(&snake_case(&feature.name));
        let value = if feature.multiplicity.is_many() {
            "Vec::new()".to_string()
        } else if is_reference(feature.kind) || feature.multiplicity.lower == 0 {
            "None".to_string()
        } else {
            default_expr(unit, feature)?
        };
        e.push_str(&format!("            {field}: {value},\n"));
    }
    e.push_str("        }\n    }\n}\n\n");

    // Struct impl: attribute setters + reference navigation.
    let mut methods = String::new();
    for feature in &class.class.features {
        if feature.is_derived {
            continue;
        }
        let field = rust_ident(&snake_case(&feature.name));
        match feature.kind {
            FeatureKind::Attribute => {
                if feature.is_read_only {
                    continue;
                }
                let value_type = if feature.multiplicity.is_many() {
                    format!("Vec<{}>", value_type(&feature.type_)?)
                } else {
                    value_type(&feature.type_)?
                };
                methods.push_str(&format!(
                    "    /// Sets `{field}` (feature id {}). Safety: plain stored value, no opposites.\n    pub fn set_{field}(&mut self, {field}: {value_type}) {{\n        self.{field} = {field};\n    }}\n\n",
                    feature.id
                ));
            }
            FeatureKind::Containment | FeatureKind::CrossReference | FeatureKind::Container => {
                let target = match &feature.type_ {
                    TypeRef::Class { name, .. } => name,
                    _ => continue,
                };
                let target_ctx = unit.class(target)?;
                let target_struct = rust_ident(&target_ctx.class.name);
                let (signature, resolve) = if feature.multiplicity.is_many() {
                    (
                        format!("pub fn {field}<'a>(&self, res: &'a Resource) -> Vec<&'a {target_struct}>"),
                        format!(
                            "self.{field}.iter().filter_map(|id| res.{}(*id)).collect()",
                            target_ctx.single
                        ),
                    )
                } else {
                    (
                        format!(
                            "pub fn {field}<'a>(&self, res: &'a Resource) -> Option<&'a {target_struct}>"
                        ),
                        format!(
                            "self.{field}.as_ref().and_then(|id| res.{}(*id))",
                            target_ctx.single
                        ),
                    )
                };
                methods.push_str(&format!(
                    "    /// Resolves `{field}` against the resource (feature id {}).\n    {signature} {{\n        {resolve}\n    }}\n\n",
                    feature.id
                ));
            }
        }
    }
    if !methods.is_empty() {
        e.push_str(&format!("impl {name} {{\n"));
        e.push_str(&methods);
        e.push_str("}\n");
    }
    Ok(())
}

fn emit_resource(e: &mut String, unit: &Unit<'_>) -> anyhow::Result<()> {
    e.push_str("/// Arena root: owns every object of the model.\n");
    e.push_str("#[derive(Debug, Default, Clone)]\n");
    e.push_str("pub struct Resource {\n");
    for class in &unit.classes {
        e.push_str(&format!(
            "    pub {}: SlotMap<{}, {}>,\n",
            class.slot_field,
            class.id_type,
            rust_ident(&class.class.name)
        ));
    }
    e.push_str("}\n\n");

    e.push_str("impl rex_runtime::Resource for Resource {\n");
    e.push_str("    fn type_name(&self) -> &'static str {\n");
    e.push_str(&format!("        {:?}\n", unit.type_name));
    e.push_str("    }\n}\n\n");

    e.push_str("impl Resource {\n");
    for class in &unit.classes {
        let struct_name = rust_ident(&class.class.name);
        e.push_str(&format!(
            "    /// Looks up a {single} by id.\n    pub fn {single}(&self, id: {id_type}) -> Option<&{struct_name}> {{\n        self.{slot}.get(id)\n    }}\n\n",
            single = class.single,
            id_type = class.id_type,
            slot = class.slot_field,
        ));
        e.push_str(&format!(
            "    /// Mutable lookup for a {single} by id.\n    pub fn {single}_mut(&mut self, id: {id_type}) -> Option<&mut {struct_name}> {{\n        self.{slot}.get_mut(id)\n    }}\n\n",
            single = class.single,
            id_type = class.id_type,
            slot = class.slot_field,
        ));
        e.push_str(&format!(
            "    /// Creates a default-initialized {struct_name}.\n    pub fn new_{single}(&mut self) -> {id_type} {{\n        self.{slot}.insert({struct_name}::default())\n    }}\n\n",
            single = class.single,
            id_type = class.id_type,
            slot = class.slot_field,
        ));
    }
    emit_pair_mutators(e, unit)?;
    e.push_str("}\n");
    Ok(())
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Cardinality {
    Many,
    Single,
}

/// One end of a paired relation: a feature plus its owning class context.
struct End<'a> {
    class: &'a ClassCtx<'a>,
    feature: &'a Feature,
}

impl End<'_> {
    fn cardinality(&self) -> Cardinality {
        if self.feature.multiplicity.is_many() {
            Cardinality::Many
        } else {
            Cardinality::Single
        }
    }

    fn field(&self) -> String {
        rust_ident(&snake_case(&self.feature.name))
    }
}

/// Emits all opposite-maintaining mutators.
///
/// Containment/container pairs are emitted once per feature of the pair (each
/// pair has exactly one `contains` end and one `container` end). Cross
/// reference pairs are emitted once, from the canonical
/// lexicographically-first end, to avoid duplicates.
fn emit_pair_mutators(e: &mut String, unit: &Unit<'_>) -> anyhow::Result<()> {
    for class in &unit.classes {
        for feature in &class.class.features {
            if feature.kind != FeatureKind::Containment {
                continue;
            }
            let target = match &feature.type_ {
                TypeRef::Class { name, .. } => name,
                _ => continue,
            };
            let owner = End { class, feature };
            let child_class = unit.class(target)?;
            let child_feature = feature.opposite.as_ref().and_then(|opposite| {
                child_class
                    .class
                    .features
                    .iter()
                    .find(|f| f.name == opposite.feature)
            });
            match child_feature {
                Some(child_feature) => {
                    let child = End {
                        class: child_class,
                        feature: child_feature,
                    };
                    emit_pair(e, &owner, &child)?;
                }
                None => emit_one_sided_containment(e, &owner, child_class)?,
            }
        }
    }

    let mut emitted: BTreeSet<(String, String)> = BTreeSet::new();
    for class in &unit.classes {
        for feature in &class.class.features {
            if feature.kind != FeatureKind::CrossReference {
                continue;
            }
            let target = match &feature.type_ {
                TypeRef::Class { name, .. } => name,
                _ => continue,
            };
            let other_class = unit.class(target)?;
            let Some(other_feature) = feature.opposite.as_ref().and_then(|opposite| {
                other_class
                    .class
                    .features
                    .iter()
                    .find(|f| f.name == opposite.feature)
            }) else {
                continue;
            };
            let key = (
                format!("{}/{}", class.class.name, feature.name),
                format!("{}/{}", other_class.class.name, other_feature.name),
            );
            let canonical = if key.0 <= key.1 {
                key.clone()
            } else {
                (key.1, key.0)
            };
            if !emitted.insert(canonical) {
                continue;
            }
            emit_pair(
                e,
                &End { class, feature },
                &End {
                    class: other_class,
                    feature: other_feature,
                },
            )?;
        }
    }
    Ok(())
}

/// Emits mutators for both ends of a relation pair.
fn emit_pair(e: &mut String, a: &End<'_>, b: &End<'_>) -> anyhow::Result<()> {
    match (a.cardinality(), b.cardinality()) {
        (Cardinality::Many, Cardinality::Many) => {
            emit_add_remove_many(e, a, b)?;
            emit_add_remove_many(e, b, a)?;
        }
        (Cardinality::Single, Cardinality::Many) => {
            emit_set_single(e, a, b)?;
            emit_add_remove_to_single(e, b, a)?;
        }
        (Cardinality::Many, Cardinality::Single) => {
            emit_add_remove_to_single(e, a, b)?;
            emit_set_single(e, b, a)?;
        }
        (Cardinality::Single, Cardinality::Single) => {
            emit_set_single_single(e, a, b)?;
            emit_set_single_single(e, b, a)?;
        }
    }
    Ok(())
}

/// `add`/`remove` for a many end whose opposite is also many.
fn emit_add_remove_many(e: &mut String, a: &End<'_>, b: &End<'_>) -> anyhow::Result<()> {
    let (a_single, a_id, a_slot) = (
        a.class.single.clone(),
        &a.class.id_type,
        &a.class.slot_field,
    );
    let (b_single, b_id, b_slot) = (
        b.class.single.clone(),
        &b.class.id_type,
        &b.class.slot_field,
    );
    let field = a.field();
    let opposite = b.field();
    e.push_str(&format!(
        "    /// Adds `{b_single}` to `{a_single}.{field}`, maintaining `{b_single}.{opposite}`.\n\
         \x20   pub fn {a_single}_add_{field}(&mut self, {a_single}: {a_id}, {b_single}: {b_id}) {{\n\
         \x20       if !self.{a_slot}.contains_key({a_single}) || !self.{b_slot}.contains_key({b_single}) {{\n\
         \x20           return;\n\
         \x20       }}\n\
         \x20       if let Some(a) = self.{a_slot}.get_mut({a_single}) {{\n\
         \x20           if !a.{field}.contains(&{b_single}) {{\n\
         \x20               a.{field}.push({b_single});\n\
         \x20           }}\n\
         \x20       }}\n\
         \x20       if let Some(b) = self.{b_slot}.get_mut({b_single}) {{\n\
         \x20           if !b.{opposite}.contains(&{a_single}) {{\n\
         \x20               b.{opposite}.push({a_single});\n\
         \x20           }}\n\
         \x20       }}\n\
         \x20   }}\n\n\
         \x20   /// Removes `{b_single}` from `{a_single}.{field}`, maintaining `{b_single}.{opposite}`.\n\
         \x20   pub fn {a_single}_remove_{field}(&mut self, {a_single}: {a_id}, {b_single}: {b_id}) {{\n\
         \x20       if let Some(a) = self.{a_slot}.get_mut({a_single}) {{\n\
         \x20           a.{field}.retain(|id| *id != {b_single});\n\
         \x20       }}\n\
         \x20       if let Some(b) = self.{b_slot}.get_mut({b_single}) {{\n\
         \x20           b.{opposite}.retain(|id| *id != {a_single});\n\
         \x20       }}\n\
         \x20   }}\n"
    ));
    Ok(())
}

/// `set` for a single end `a` whose opposite end `b` is many.
fn emit_set_single(e: &mut String, a: &End<'_>, b: &End<'_>) -> anyhow::Result<()> {
    let (a_single, a_id, a_slot) = (
        a.class.single.clone(),
        &a.class.id_type,
        &a.class.slot_field,
    );
    let (b_single, b_id, b_slot) = (
        b.class.single.clone(),
        &b.class.id_type,
        &b.class.slot_field,
    );
    let field = a.field();
    let opposite = b.field();
    e.push_str(&format!(
        "    /// Sets `{a_single}.{field}` (or clears it), maintaining `{b_single}.{opposite}`.\n\
         \x20   pub fn {a_single}_set_{field}(&mut self, {a_single}: {a_id}, {b_single}: Option<{b_id}>) {{\n\
         \x20       if !self.{a_slot}.contains_key({a_single}) {{\n\
         \x20           return;\n\
         \x20       }}\n\
         \x20       if let Some(target) = {b_single} {{\n\
         \x20           if !self.{b_slot}.contains_key(target) {{\n\
         \x20               return;\n\
         \x20           }}\n\
         \x20       }}\n\
         \x20       let current = self.{a_slot}.get({a_single}).and_then(|s| s.{field});\n\
         \x20       if current == {b_single} {{\n\
         \x20           return;\n\
         \x20       }}\n\
         \x20       if let Some(old) = current {{\n\
         \x20           if let Some(old_b) = self.{b_slot}.get_mut(old) {{\n\
         \x20               old_b.{opposite}.retain(|id| *id != {a_single});\n\
         \x20           }}\n\
         \x20       }}\n\
         \x20       if let Some(new_b) = {b_single} {{\n\
         \x20           if let Some(b) = self.{b_slot}.get_mut(new_b) {{\n\
         \x20               if !b.{opposite}.contains(&{a_single}) {{\n\
         \x20                   b.{opposite}.push({a_single});\n\
         \x20               }}\n\
         \x20           }}\n\
         \x20       }}\n\
         \x20       if let Some(a) = self.{a_slot}.get_mut({a_single}) {{\n\
         \x20           a.{field} = {b_single};\n\
         \x20       }}\n\
         \x20   }}\n"
    ));
    Ok(())
}

/// `add`/`remove` for a many end `a` whose opposite end `b` is single.
fn emit_add_remove_to_single(e: &mut String, a: &End<'_>, b: &End<'_>) -> anyhow::Result<()> {
    let (a_single, a_id, a_slot) = (
        a.class.single.clone(),
        &a.class.id_type,
        &a.class.slot_field,
    );
    let (b_single, b_id, b_slot) = (
        b.class.single.clone(),
        &b.class.id_type,
        &b.class.slot_field,
    );
    let field = a.field();
    let opposite = b.field();
    e.push_str(&format!(
        "    /// Adds `{b_single}` to `{a_single}.{field}`, detaching it from any previous `{opposite}` holder.\n\
         \x20   pub fn {a_single}_add_{field}(&mut self, {a_single}: {a_id}, {b_single}: {b_id}) {{\n\
         \x20       if !self.{a_slot}.contains_key({a_single}) || !self.{b_slot}.contains_key({b_single}) {{\n\
         \x20           return;\n\
         \x20       }}\n\
         \x20       let current = self.{b_slot}.get({b_single}).and_then(|b| b.{opposite});\n\
         \x20       if current == Some({a_single}) {{\n\
         \x20           return;\n\
         \x20       }}\n\
         \x20       if let Some(old) = current {{\n\
         \x20           if let Some(old_a) = self.{a_slot}.get_mut(old) {{\n\
         \x20               old_a.{field}.retain(|id| *id != {b_single});\n\
         \x20           }}\n\
         \x20       }}\n\
         \x20       if let Some(a) = self.{a_slot}.get_mut({a_single}) {{\n\
         \x20           if !a.{field}.contains(&{b_single}) {{\n\
         \x20               a.{field}.push({b_single});\n\
         \x20           }}\n\
         \x20       }}\n\
         \x20       if let Some(b) = self.{b_slot}.get_mut({b_single}) {{\n\
         \x20           b.{opposite} = Some({a_single});\n\
         \x20       }}\n\
         \x20   }}\n\n\
         \x20   /// Removes `{b_single}` from `{a_single}.{field}`, clearing `{b_single}.{opposite}` if it pointed here.\n\
         \x20   pub fn {a_single}_remove_{field}(&mut self, {a_single}: {a_id}, {b_single}: {b_id}) {{\n\
         \x20       if let Some(a) = self.{a_slot}.get_mut({a_single}) {{\n\
         \x20           a.{field}.retain(|id| *id != {b_single});\n\
         \x20       }}\n\
         \x20       if let Some(b) = self.{b_slot}.get_mut({b_single}) {{\n\
         \x20           if b.{opposite} == Some({a_single}) {{\n\
         \x20               b.{opposite} = None;\n\
         \x20           }}\n\
         \x20       }}\n\
         \x20   }}\n"
    ));
    Ok(())
}

/// `set` for a single end whose opposite is also single.
fn emit_set_single_single(e: &mut String, a: &End<'_>, b: &End<'_>) -> anyhow::Result<()> {
    let (a_single, a_id, a_slot) = (
        a.class.single.clone(),
        &a.class.id_type,
        &a.class.slot_field,
    );
    let (b_single, b_id, b_slot) = (
        b.class.single.clone(),
        &b.class.id_type,
        &b.class.slot_field,
    );
    let field = a.field();
    let opposite = b.field();
    e.push_str(&format!(
        "    /// Sets `{a_single}.{field}` (or clears it), maintaining `{b_single}.{opposite}`.\n\
         \x20   pub fn {a_single}_set_{field}(&mut self, {a_single}: {a_id}, {b_single}: Option<{b_id}>) {{\n\
         \x20       if !self.{a_slot}.contains_key({a_single}) {{\n\
         \x20           return;\n\
         \x20       }}\n\
         \x20       if let Some(target) = {b_single} {{\n\
         \x20           if !self.{b_slot}.contains_key(target) {{\n\
         \x20               return;\n\
         \x20           }}\n\
         \x20       }}\n\
         \x20       let current = self.{a_slot}.get({a_single}).and_then(|s| s.{field});\n\
         \x20       if current == {b_single} {{\n\
         \x20           return;\n\
         \x20       }}\n\
         \x20       if let Some(old) = current {{\n\
         \x20           if let Some(old_b) = self.{b_slot}.get_mut(old) {{\n\
         \x20               if old_b.{opposite} == Some({a_single}) {{\n\
         \x20                   old_b.{opposite} = None;\n\
         \x20               }}\n\
         \x20           }}\n\
         \x20       }}\n\
         \x20       if let Some(new_b) = {b_single} {{\n\
         \x20           if let Some(prev) = self.{b_slot}.get(new_b).and_then(|b| b.{opposite}) {{\n\
         \x20               if prev != {a_single} {{\n\
         \x20                   if let Some(prev_a) = self.{a_slot}.get_mut(prev) {{\n\
         \x20                       prev_a.{field} = None;\n\
         \x20                   }}\n\
         \x20               }}\n\
         \x20           }}\n\
         \x20           if let Some(b) = self.{b_slot}.get_mut(new_b) {{\n\
         \x20               b.{opposite} = Some({a_single});\n\
         \x20           }}\n\
         \x20       }}\n\
         \x20       if let Some(a) = self.{a_slot}.get_mut({a_single}) {{\n\
         \x20           a.{field} = {b_single};\n\
         \x20       }}\n\
         \x20   }}\n"
    ));
    Ok(())
}

/// Owner-side mutators for a `contains` feature without an opposite: plain
/// id-list maintenance with no back-pointer to keep in sync.
fn emit_one_sided_containment(
    e: &mut String,
    owner: &End<'_>,
    child_class: &ClassCtx<'_>,
) -> anyhow::Result<()> {
    let (a_single, a_id, a_slot) = (
        owner.class.single.clone(),
        &owner.class.id_type,
        &owner.class.slot_field,
    );
    let (b_single, b_id, b_slot) = (
        child_class.single.clone(),
        &child_class.id_type,
        &child_class.slot_field,
    );
    let field = owner.field();
    e.push_str(&format!(
        "    /// Adds `{b_single}` to `{a_single}.{field}` (one-way containment, no opposite).\n\
         \x20   pub fn {a_single}_add_{field}(&mut self, {a_single}: {a_id}, {b_single}: {b_id}) {{\n\
         \x20       if !self.{a_slot}.contains_key({a_single}) || !self.{b_slot}.contains_key({b_single}) {{\n\
         \x20           return;\n\
         \x20       }}\n\
         \x20       if let Some(a) = self.{a_slot}.get_mut({a_single}) {{\n\
         \x20           if !a.{field}.contains(&{b_single}) {{\n\
         \x20               a.{field}.push({b_single});\n\
         \x20           }}\n\
         \x20       }}\n\
         \x20   }}\n\n\
         \x20   /// Removes `{b_single}` from `{a_single}.{field}` (one-way containment, no opposite).\n\
         \x20   pub fn {a_single}_remove_{field}(&mut self, {a_single}: {a_id}, {b_single}: {b_id}) {{\n\
         \x20       if let Some(a) = self.{a_slot}.get_mut({a_single}) {{\n\
         \x20           a.{field}.retain(|id| *id != {b_single});\n\
         \x20       }}\n\
         \x20   }}\n"
    ));
    Ok(())
}
