//! Struct, `Default`, and method emission per class: setters, reference
//! navigation, derived accessors, and operations (verbatim `rust` bodies
//! or lowered `expr` bodies).

use anyhow::bail;
use rex_ir::{FeatureKind, TypeRef};

use crate::naming::{rust_ident, snake_case};

use super::ops::expr_return_type;
use super::types::{feature_doc, field_type, is_reference, op_type, value_type};
use super::validate::emit_validate_method;
use super::*;
pub(crate) fn emit_struct(
    e: &mut String,
    unit: &Unit<'_>,
    class: &ClassCtx<'_>,
) -> anyhow::Result<()> {
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

    // Struct impl: attribute setters + reference navigation + derived
    // accessors (Tier 2: an `expr` body becomes an accessor) + operation
    // bodies (Tier 1: a `rust` body becomes a method; body-less operations
    // are hand-written hooks and emit nothing).
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
                // Optional scalar fields store `Option<T>`; their setter takes
                // the plain value and wraps it.
                let assign = if feature.multiplicity.is_many() || feature.multiplicity.lower >= 1 {
                    format!("self.{field} = {field};")
                } else {
                    format!("self.{field} = Some({field});")
                };
                methods.push_str(&format!(
                    "    /// Sets `{field}` (feature id {}). Safety: plain stored value, no opposites.\n    pub fn set_{field}(&mut self, {field}: {value_type}) {{\n        {assign}\n    }}\n\n",
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
    // Derived-feature accessors (Tier 2): `expr` bodies lower like
    // operations, with no parameters and the feature's declared typing as
    // the return contract.
    for feature in &class.class.features {
        if !feature.is_derived {
            continue;
        }
        let expr_body = feature.bodies.get("expr");
        let rust_body = feature.bodies.get("rust");
        if let (Some(_), Some(_)) = (expr_body, rust_body) {
            bail!(
                "derived feature '{}' has conflicting 'rust' and 'expr' bodies; \
                 derived bodies must use the neutral expression language",
                feature.name
            );
        }
        let Some(expr_body) = expr_body else {
            continue;
        };
        let (lowered, ret) =
            lower_derived_expr_body(unit.model, &class.package, class.class, feature, expr_body)?;
        methods.push_str(&format!(
            "    /// Derived feature `{name}`, lowered from the neutral `expr` body.\n    pub fn {snake}(&self, res: &Resource) -> {ret} {{\n        {lowered}\n    }}\n\n",
            name = feature.name,
            snake = rust_ident(&snake_case(&feature.name)),
        ));
    }
    for operation in &class.class.operations {
        let expr_body = operation.bodies.get("expr");
        let rust_body = operation.bodies.get("rust");
        if let (Some(_), Some(_)) = (expr_body, rust_body) {
            bail!(
                "operation '{}' has conflicting 'rust' and 'expr' bodies; \
                 declare exactly one body per operation",
                operation.name
            );
        }
        let params = operation
            .params
            .iter()
            .map(|param| {
                Ok(format!(
                    "{}: {}",
                    rust_ident(&snake_case(&param.name)),
                    op_type(&param.type_)?
                ))
            })
            .collect::<anyhow::Result<Vec<_>>>()?
            .join(", ");
        let head = if params.is_empty() {
            String::new()
        } else {
            format!(", {params}")
        };
        if let Some(expr_body) = expr_body {
            let (lowered, expr_ty) = lower_op_expr_body(
                unit.model,
                &class.package,
                class.class,
                operation,
                expr_body,
            )?;
            let ret = expr_return_type(&expr_ty)?;
            methods.push_str(&format!(
                "    /// Declared operation `{name}` (lowered from the neutral `expr` body).\n    pub fn {snake}(&self, res: &Resource{head}) -> {ret} {{\n        {lowered}\n    }}\n\n",
                name = operation.name,
                snake = rust_ident(&snake_case(&operation.name)),
            ));
            continue;
        }
        let Some(body) = rust_body else {
            continue;
        };
        methods.push_str(&format!(
            "    /// Declared operation `{name}` (rust body embedded verbatim).\n    pub fn {snake}(&self, res: &Resource{head}) -> {ret} {{\n{body}\n    }}\n\n",
            name = operation.name,
            snake = rust_ident(&snake_case(&operation.name)),
            ret = op_type(&operation.return_type)?,
        ));
    }
    // Constraint validation: only classes with at least one checkable
    // constraint carry a `validate` method, so unconstrained models keep
    // byte-identical generated code.
    emit_validate_method(&mut methods, unit, class)?;
    if !methods.is_empty() {
        e.push_str(&format!("impl {name} {{\n"));
        e.push_str(&methods);
        e.push_str("}\n");
    }
    Ok(())
}
