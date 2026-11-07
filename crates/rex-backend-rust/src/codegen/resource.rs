//! The `Resource` arena runtime glue: per-class singleton lookups and
//! the containment/xref pair mutators (once per pair, one-sided
//! fallback, cardinality-dispatched bodies).

use std::collections::BTreeSet;

use rex_ir::{Feature, FeatureKind, TypeRef};

use crate::naming::{rust_ident, snake_case};

use super::*;
pub(crate) fn emit_resource(e: &mut String, unit: &Unit<'_>) -> anyhow::Result<()> {
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

    e.push_str("impl PartialEq for Resource {\n");
    e.push_str("    fn eq(&self, other: &Self) -> bool {\n");
    for class in &unit.classes {
        e.push_str(&format!(
            "        self.{slot}.len() == other.{slot}.len()\n\
             \x20           && self.{slot}.iter().all(|(key, value)| other.{slot}.get(key) == Some(value))\n\
             \x20           &&\n",
            slot = class.slot_field,
        ));
    }
    e.push_str("        true\n");
    e.push_str("    }\n}\n\n");

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
pub(crate) struct End<'a> {
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

/// The value parameter of an opposite-maintaining mutator (issue #3).
///
/// Distinct-class pairs keep the target class's snake name. When both ends
/// share one class that name collides with the subject parameter, so the
/// near-end role names the value instead — and when the role *is* the class
/// name, a numbered fallback keeps the signature deterministic and valid.
fn value_param_name(a_single: &str, role: &str, b_single: &str) -> String {
    if b_single != a_single {
        b_single.to_string()
    } else if role != a_single {
        role.to_string()
    } else {
        format!("{a_single}_2")
    }
}

/// Emits all opposite-maintaining mutators.
///
/// Containment/container pairs are emitted once per feature of the pair (each
/// pair has exactly one `contains` end and one `container` end). Cross
/// reference pairs are emitted once, from the canonical
/// lexicographically-first end, to avoid duplicates.
pub(crate) fn emit_pair_mutators(e: &mut String, unit: &Unit<'_>) -> anyhow::Result<()> {
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
pub(crate) fn emit_pair(e: &mut String, a: &End<'_>, b: &End<'_>) -> anyhow::Result<()> {
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
    let (b_id, b_slot) = (&b.class.id_type, &b.class.slot_field);
    let field = a.field();
    let opposite = b.field();
    let b_single = value_param_name(&a_single, &field, &b.class.single);
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
    let (b_id, b_slot) = (&b.class.id_type, &b.class.slot_field);
    let field = a.field();
    let opposite = b.field();
    let b_single = value_param_name(&a_single, &field, &b.class.single);
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
    let (b_id, b_slot) = (&b.class.id_type, &b.class.slot_field);
    let field = a.field();
    let opposite = b.field();
    let b_single = value_param_name(&a_single, &field, &b.class.single);
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
    let (b_id, b_slot) = (&b.class.id_type, &b.class.slot_field);
    let field = a.field();
    let opposite = b.field();
    let b_single = value_param_name(&a_single, &field, &b.class.single);
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
    let (b_id, b_slot) = (&child_class.id_type, &child_class.slot_field);
    let field = owner.field();
    let b_single = value_param_name(&a_single, &field, &child_class.single);
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
