//! Capability-class mapping: the class backing a capability, its
//! primitive-typed attributes, and the Cedar primitive spellings.
use rex_ir::{ActorsDef, ClassDef, Feature, FeatureKind, Model, PrimitiveType, TypeRef};

/// The class definition a capability of `actors` is granted on, resolved
/// through the capability's package-qualified `TypeRef`.
pub(super) fn capability_class<'a>(
    actors: &ActorsDef,
    model: &'a Model,
    capability: &str,
) -> anyhow::Result<&'a ClassDef> {
    let TypeRef::Class {
        package,
        name: class,
    } = actors
        .capabilities
        .iter()
        .find(|candidate| candidate.name == capability)
        .map(|capability| &capability.class)
        .ok_or_else(|| anyhow::anyhow!("unknown capability `{capability}`"))?
    else {
        anyhow::bail!(
            "capability `{capability}` must be granted on a class, which the \
             driver validates"
        );
    };
    model
        .packages
        .iter()
        .find(|candidate| candidate.name == *package)
        .and_then(|package| {
            package
                .classes
                .iter()
                .find(|candidate| candidate.name == *class)
        })
        .ok_or_else(|| {
            anyhow::anyhow!(
                "capability `{capability}` is granted on class `{class}`, which is \
                 not in the model"
            )
        })
}

/// The Cedar type of a primitive-typed stored attribute, or `None` for any
/// feature that is not a primitive attribute (refs, containments, enums,
/// datatypes, derived features).
pub(super) fn primitive_cedar_type(feature: &Feature) -> Option<&'static str> {
    if feature.is_derived || feature.kind != FeatureKind::Attribute {
        return None;
    }
    let TypeRef::Primitive(primitive) = feature.type_ else {
        return None;
    };
    Some(match primitive {
        // Dates have no Cedar attribute representation: Cedar has no date
        // type, so a date attribute is not a schema attribute and a `when`
        // condition over it is refused by name (`is_primitive_attribute`).
        PrimitiveType::Date => return None,
        PrimitiveType::String | PrimitiveType::Char => "String",
        PrimitiveType::Int | PrimitiveType::Long | PrimitiveType::Short | PrimitiveType::Byte => {
            "Long"
        }
        PrimitiveType::Float | PrimitiveType::Double => "Double",
        PrimitiveType::Boolean => "Boolean",
    })
}

/// `true` if `name` is a primitive-typed stored attribute of `class`.
pub(super) fn is_primitive_attribute(class: &ClassDef, name: &str) -> bool {
    class
        .features
        .iter()
        .any(|feature| feature.name == name && primitive_cedar_type(feature).is_some())
}
