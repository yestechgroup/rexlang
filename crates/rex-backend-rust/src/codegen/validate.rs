//! The `validate()` emission: per-type attribute checks (string length,
//! integer width, datatype newtype wrap, enum/vocabulary value) emitted
//! only when checkable constraints exist — unconstrained models stay
//! byte-identical.

use anyhow::bail;
use rex_ir::{ClassDef, Feature, FeatureKind, PrimitiveType, TypeRef};

use crate::naming::{rust_ident, snake_case};

use super::*;
/// The attributes of `class` carrying declared constraints: the candidates
/// for a generated `validate` method. Derived features and non-attribute
/// features never carry constraints.
fn constrained_attributes(class: &ClassDef) -> impl Iterator<Item = &Feature> {
    class.features.iter().filter(|feature| {
        feature.kind == FeatureKind::Attribute
            && !feature.is_derived
            && !feature.constraints.is_empty()
    })
}

/// The runtime-checkable quantity of one constrained attribute: the string
/// family checks a length expression, the numeric family an `i64` value
/// expression (the IR stores bounds as `i64`). The expression reads the
/// element value bound to `value` by the multiplicity shape.
enum Checked {
    Length(String),
    Number(String),
}

/// One `if … { violations.push(…) }` block for a single bound.
pub(crate) fn bound_check(
    feature_name: &str,
    kind: &str,
    comparison: &str,
    detail: &str,
) -> String {
    format!(
        "            if {comparison} {{\n                \
         violations.push(rex_runtime::validation::Violation {{\n                    \
         feature: {feature_name:?}.to_string(),\n                    \
         kind: rex_runtime::validation::ConstraintKind::{kind},\n                    \
         detail: format!(\"{detail}\"),\n                \
         }});\n            }}\n"
    )
}

/// Emits the `validate` method for `class` into the accumulated `methods`
/// buffer. Nothing is emitted when no constraint of the class is checkable
/// by the Rust backend.
pub(crate) fn emit_validate_method(
    methods: &mut String,
    unit: &Unit<'_>,
    class: &ClassCtx<'_>,
) -> anyhow::Result<()> {
    let mut body = String::new();
    for feature in constrained_attributes(class.class) {
        emit_attribute_checks(unit, feature, &mut body)?;
    }
    if body.is_empty() {
        return Ok(());
    }
    methods.push_str(
        "    /// Reports one violation per offended attribute constraint.\n    \
         pub fn validate(&self) -> Vec<rex_runtime::validation::Violation> {\n        \
         let mut violations = Vec::new();\n",
    );
    methods.push_str(&body);
    methods.push_str("        violations\n    }\n\n");
    Ok(())
}

/// Emits the check statements for one constrained attribute: one
/// `violations.push` per offended bound, per element for many-valued
/// attributes (the element index is prefixed to the detail). Absent values
/// (optional attributes) are not violations — multiplicity is not a
/// constraint.
pub(crate) fn emit_attribute_checks(
    unit: &Unit<'_>,
    feature: &Feature,
    out: &mut String,
) -> anyhow::Result<()> {
    let field = rust_ident(&snake_case(&feature.name));
    let constraints = &feature.constraints;
    // How the checked quantity is computed from the element value. Bounds
    // the backend checks descriptively in this milestone (enum literal
    // *name* bounds, short/byte/float/double bounds) map to `None` and emit
    // nothing. Class/interface attributes cannot carry constraints (the
    // driver rejects them), so their presence means a corrupt IR.
    let checked = match &feature.type_ {
        TypeRef::Primitive(PrimitiveType::String) => {
            Some(Checked::Length("value.chars().count()".to_string()))
        }
        TypeRef::Primitive(PrimitiveType::Int) => {
            Some(Checked::Number("i64::from(*value)".to_string()))
        }
        TypeRef::Primitive(PrimitiveType::Long) => Some(Checked::Number("*value".to_string())),
        TypeRef::Datatype { .. } => Some(Checked::Length("value.0.chars().count()".to_string())),
        TypeRef::Enum { .. } => Some(Checked::Number("value.value()".to_string())),
        TypeRef::Vocabulary { name, .. } => {
            let vocabulary = unit
                .vocabularies
                .iter()
                .find(|v| v.name == *name)
                .ok_or_else(|| anyhow::anyhow!("vocabulary '{name}' not found in model"))?;
            let key_facet = vocabulary
                .facets
                .iter()
                .find(|facet| facet.name == vocabulary.key)
                .ok_or_else(|| {
                    anyhow::anyhow!("vocabulary '{name}' has no key facet '{}'", vocabulary.key)
                })?;
            match key_facet.type_ {
                PrimitiveType::String => {
                    Some(Checked::Length("value.key().chars().count()".to_string()))
                }
                PrimitiveType::Int | PrimitiveType::Long => Some(Checked::Number(format!(
                    "value.{}()",
                    rust_ident(&snake_case(&key_facet.name))
                ))),
                _ => None,
            }
        }
        TypeRef::Class { name, .. } | TypeRef::Interface { name, .. } => bail!(
            "feature '{}' has constraints, but class and interface types take none ('{name}')",
            feature.name
        ),
        _ => None,
    };
    let Some(checked) = checked else {
        return Ok(());
    };
    let many = feature.multiplicity.is_many();
    let indexed = |detail: String| {
        if many {
            format!("[{{index}}] {detail}")
        } else {
            detail
        }
    };
    let mut checks = String::new();
    match checked {
        Checked::Length(length_expr) => {
            if constraints.min_length.is_some() || constraints.max_length.is_some() {
                checks.push_str(&format!("            let len = {length_expr};\n"));
                if let Some(min) = constraints.min_length {
                    checks.push_str(&bound_check(
                        &feature.name,
                        "MinLength",
                        &format!("len < {min}"),
                        &indexed(format!("length {{len}} is below minLength {min}")),
                    ));
                }
                if let Some(max) = constraints.max_length {
                    checks.push_str(&bound_check(
                        &feature.name,
                        "MaxLength",
                        &format!("len > {max}"),
                        &indexed(format!("length {{len}} exceeds maxLength {max}")),
                    ));
                }
            }
        }
        Checked::Number(number_expr) => {
            if constraints.minimum.is_some() || constraints.maximum.is_some() {
                checks.push_str(&format!("            let number = {number_expr};\n"));
                if let Some(min) = constraints.minimum {
                    checks.push_str(&bound_check(
                        &feature.name,
                        "Minimum",
                        &format!("number < {min}"),
                        &indexed(format!("value {{number}} is below minimum {min}")),
                    ));
                }
                if let Some(max) = constraints.maximum {
                    checks.push_str(&bound_check(
                        &feature.name,
                        "Maximum",
                        &format!("number > {max}"),
                        &indexed(format!("value {{number}} exceeds maximum {max}")),
                    ));
                }
            }
        }
    }
    if checks.is_empty() {
        return Ok(());
    }
    if many {
        out.push_str(&format!(
            "        for (index, value) in self.{field}.iter().enumerate() {{\n{checks}        }}\n"
        ));
    } else if feature.multiplicity.lower == 0 {
        out.push_str(&format!(
            "        if let Some(value) = &self.{field} {{\n{checks}        }}\n"
        ));
    } else {
        out.push_str(&format!(
            "        {{\n            let value = &self.{field};\n{checks}        }}\n"
        ));
    }
    Ok(())
}
