//! The Tier-1 type mapping contract: `PrimitiveType` → Rust, feature
//! shapes (`Vec<T>`/`Option<T>`/`T`), operation signatures, and declared
//! + implicit defaults.

use anyhow::bail;
use rex_ir::{DefaultValue, Feature, FeatureKind, PrimitiveType, TypeRef};

use crate::naming::rust_ident;

use super::values::variant_ident;
use super::*;
pub(crate) fn value_type(type_: &TypeRef) -> anyhow::Result<String> {
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
            // The calendar date primitive maps to the runtime support type:
            // a newtype over days since the epoch with ISO-8601
            // serialization (issue #9).
            PrimitiveType::Date => "rex_runtime::Date".to_string(),
        },
        TypeRef::Class { name, .. }
        | TypeRef::Enum { name, .. }
        | TypeRef::Datatype { name, .. }
        | TypeRef::Vocabulary { name, .. } => rust_ident(name),
        TypeRef::Interface { name, .. } => {
            bail!("interfaces are not materialized by the Rust backend yet ('{name}')")
        }
    })
}

pub(crate) fn is_reference(kind: FeatureKind) -> bool {
    matches!(
        kind,
        FeatureKind::Containment | FeatureKind::CrossReference | FeatureKind::Container
    )
}

/// The Rust type of an operation signature slot (param or return), per the
/// Tier 1 convention: classes map to their typed id (`BookId`), enums,
/// datatypes, and vocabularies to their Rust type, primitives as for fields.
pub(crate) fn op_type(type_: &TypeRef) -> anyhow::Result<String> {
    match type_ {
        TypeRef::Class { name, .. } => Ok(format!("{}Id", rust_ident(name))),
        TypeRef::Interface { name, .. } => {
            bail!("interfaces are not materialized by the Rust backend yet ('{name}')")
        }
        other => value_type(other),
    }
}

pub(crate) fn feature_doc(feature: &Feature) -> &'static str {
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
pub(crate) fn field_type(_unit: &Unit<'_>, feature: &Feature) -> anyhow::Result<String> {
    if feature.is_derived {
        bail!(
            "derived feature '{}' should not be materialized",
            feature.name
        );
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
pub(crate) fn default_expr(unit: &Unit<'_>, feature: &Feature) -> anyhow::Result<String> {
    // A declared default on a vocabulary-typed attribute names an entry key.
    if let (Some(DefaultValue::String(key)), TypeRef::Vocabulary { name, .. }) =
        (&feature.default, &feature.type_)
    {
        let vocabulary = unit
            .vocabularies
            .iter()
            .find(|v| v.name == *name)
            .ok_or_else(|| anyhow::anyhow!("vocabulary '{name}' not found in model"))?;
        if !vocabulary.entries.iter().any(|entry| entry.key == *key) {
            bail!("default '{key}' is not an entry of vocabulary '{name}'");
        }
        return Ok(format!("{}::{}", rust_ident(name), variant_ident(key)?));
    }
    if let Some(default) = &feature.default {
        return Ok(match default {
            DefaultValue::String(value) => match &feature.type_ {
                // A datatype-typed attribute stores the wrapped String: the
                // default must construct the newtype, not the bare inner
                // value.
                TypeRef::Datatype { name, .. } => {
                    format!("{}({value:?}.to_string())", rust_ident(name))
                }
                _ => format!("{value:?}.to_string()"),
            },
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
            PrimitiveType::Int
            | PrimitiveType::Long
            | PrimitiveType::Short
            | PrimitiveType::Byte => "0".to_string(),
            PrimitiveType::Float | PrimitiveType::Double => "0.0".to_string(),
            PrimitiveType::Boolean => "false".to_string(),
            PrimitiveType::Char => "'\\0'".to_string(),
            // Dates take no declared defaults (the .mox surface rejects
            // them); the implicit default of a required date attribute is
            // the epoch 1970-01-01.
            PrimitiveType::Date => "rex_runtime::Date::EPOCH".to_string(),
        },
        TypeRef::Enum { name, .. } => {
            let first = unit.first_enum_literal(name)?;
            format!("{}::{}", rust_ident(name), rust_ident(&first.name))
        }
        TypeRef::Datatype { name, .. } => format!("{}::default()", rust_ident(name)),
        TypeRef::Vocabulary { name, .. } => {
            let first = unit.first_vocabulary_entry(name)?;
            format!("{}::{}", rust_ident(name), variant_ident(&first.key)?)
        }
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
