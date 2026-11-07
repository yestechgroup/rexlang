//! The domain side of the expression checker: the [`rex_expr::DomainTypes`]
//! index built from the domain model, bare-name lookup over the same
//! model, and the builtin alias table shared with the resolver.

use std::collections::BTreeMap;

use rex_expr::{DomainTypes, NamedKind, Ty};
use rex_ir::Model;
use rex_ir::PrimitiveType;

use crate::index::LiteralKind;

/// Why a bare entity name does not resolve to exactly one class.
pub(super) enum EntityError {
    Unknown,
    Ambiguous,
}

/// The domain side of the checker: the [`DomainTypes`] the expression
/// checker consumes, plus bare-name lookup over the same model (the
/// `.ifml` names are unqualified, so a unique class or named type across
/// all packages is required).
pub(super) struct DomainIndex<'a> {
    pub(super) types: DomainTypes,
    classes: BTreeMap<&'a str, Vec<(&'a str, &'a str)>>,
    named: BTreeMap<&'a str, Vec<(&'a str, &'a str, NamedKind)>>,
}

impl<'a> DomainIndex<'a> {
    pub(super) fn new(model: &'a Model) -> Self {
        let mut index = DomainIndex {
            types: DomainTypes::from_model(model),
            classes: BTreeMap::new(),
            named: BTreeMap::new(),
        };
        for package in &model.packages {
            for class in &package.classes {
                index
                    .classes
                    .entry(class.name.as_str())
                    .or_default()
                    .push((package.name.as_str(), class.name.as_str()));
            }
            for enum_ in &package.enums {
                index.named.entry(enum_.name.as_str()).or_default().push((
                    package.name.as_str(),
                    enum_.name.as_str(),
                    NamedKind::Enum,
                ));
            }
            for datatype in &package.datatypes {
                index
                    .named
                    .entry(datatype.name.as_str())
                    .or_default()
                    .push((
                        package.name.as_str(),
                        datatype.name.as_str(),
                        NamedKind::Datatype,
                    ));
            }
            for interface in &package.interfaces {
                index
                    .named
                    .entry(interface.name.as_str())
                    .or_default()
                    .push((
                        package.name.as_str(),
                        interface.name.as_str(),
                        NamedKind::Interface,
                    ));
            }
            for vocabulary in &package.vocabularies {
                index
                    .named
                    .entry(vocabulary.name.as_str())
                    .or_default()
                    .push((
                        package.name.as_str(),
                        vocabulary.name.as_str(),
                        NamedKind::Vocabulary,
                    ));
            }
        }
        index
    }

    /// Resolves a bare entity name to a unique `(package, class)`.
    pub(super) fn resolve_class(&self, bare: &str) -> Result<(&'a str, &'a str), EntityError> {
        match self.classes.get(bare).map(Vec::as_slice) {
            Some([class]) => Ok(*class),
            Some(_) => Err(EntityError::Ambiguous),
            None => Err(EntityError::Unknown),
        }
    }

    /// Maps a declared `type_ref` onto an expression type: builtins through
    /// the [alias table](builtin_type), any other name as a domain type
    /// resolved by bare name.
    pub(super) fn map_type_ref(&self, type_ref: &str) -> Option<Ty> {
        if let Some(builtin) = builtin_type(type_ref) {
            return builtin.ty();
        }
        if let Ok((package, class)) = self.resolve_class(type_ref) {
            return Some(Ty::class(package, class));
        }
        match self.named.get(type_ref).map(Vec::as_slice) {
            Some([(package, name, kind)]) => Some(Ty::named(*kind, *package, *name)),
            _ => None,
        }
    }
}

/// The `.ifml` surface's builtin parameter types, with what each means to
/// the expression checker — the **single alias table** shared by the
/// expression checker's `type_ref` mapping ([`DomainIndex::map_type_ref`])
/// and the resolver's structural override check
/// ([`crate::resolve::literal_suits_type`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BuiltinType {
    /// `String` — text.
    Str,
    /// `Uuid` — checked as a string: identifiers are opaque text to
    /// expressions.
    Uuid,
    /// `Int` — whole numbers.
    Int,
    /// `Float` — the expression language's float literal type.
    Float,
    /// `Boolean`.
    Boolean,
    /// `DateTime` — checked as the expression language's calendar `date`.
    /// Checking alias only: there is no time-of-day reasoning, so
    /// `order.createdAt > date("2026-01-01")` type-checks on the date
    /// value's terms (R7 ordering). No literal override kind suits a
    /// `DateTime` input — string/number literals are rejected, so such
    /// inputs are expression-bound or defaulted at the declaration.
    DateTime,
}

/// Classifies a declared `type_ref` as a surface builtin, or `None` when it
/// is a domain type name.
pub(crate) fn builtin_type(type_ref: &str) -> Option<BuiltinType> {
    match type_ref {
        "String" => Some(BuiltinType::Str),
        "Uuid" => Some(BuiltinType::Uuid),
        "Int" => Some(BuiltinType::Int),
        "Float" => Some(BuiltinType::Float),
        "Boolean" => Some(BuiltinType::Boolean),
        "DateTime" => Some(BuiltinType::DateTime),
        _ => None,
    }
}

impl BuiltinType {
    /// The expression type the checker binds for parameters of this
    /// builtin.
    pub(crate) fn ty(self) -> Option<Ty> {
        match self {
            BuiltinType::Str | BuiltinType::Uuid => Some(Ty::string()),
            BuiltinType::Int => Some(Ty::int()),
            BuiltinType::Float => Some(Ty::float()),
            BuiltinType::Boolean => Some(Ty::boolean()),
            BuiltinType::DateTime => Some(Ty::Primitive(PrimitiveType::Date)),
        }
    }

    /// Whether a literal override of this kind structurally suits an input
    /// of this builtin.
    pub(crate) fn suits(self, literal: LiteralKind) -> bool {
        matches!(
            (self, literal),
            (BuiltinType::Str | BuiltinType::Uuid, LiteralKind::Str)
                | (BuiltinType::Int | BuiltinType::Float, LiteralKind::Num)
                | (BuiltinType::Boolean, LiteralKind::Bool)
        )
    }
}
