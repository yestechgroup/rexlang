//! The expression type system, mirroring the resolved Core IR.

use rex_ir::{PrimitiveType, TypeRef};
use std::fmt;

/// The kind of a named type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NamedKind {
    /// A class (`rex_ir::ClassDef`).
    Class,
    /// An enum (`rex_ir::EnumDef`).
    Enum,
    /// A datatype (`rex_ir::DatatypeDef`).
    Datatype,
    /// An interface (`rex_ir::InterfaceDef`).
    Interface,
    /// A vocabulary (`rex_ir::VocabularyDef`).
    Vocabulary,
}

impl fmt::Display for NamedKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            NamedKind::Class => "class",
            NamedKind::Enum => "enum",
            NamedKind::Datatype => "datatype",
            NamedKind::Interface => "interface",
            NamedKind::Vocabulary => "vocabulary",
        })
    }
}

/// The type of a well-typed expression (spec: "Type system").
///
/// Booleans are [`Ty::Primitive`]`(`[`PrimitiveType::Boolean`]`)` — there is
/// no separate boolean variant. [`Ty::Null`] types the `null` literal only
/// (spec R2/R3): it participates in equality and `?:`, and nowhere else.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ty {
    /// A built-in primitive.
    Primitive(PrimitiveType),
    /// A class, enum, datatype, interface, or vocabulary, package-qualified.
    Named {
        /// The kind of named type.
        kind: NamedKind,
        /// Owning package name.
        package: String,
        /// Type name within the package.
        name: String,
    },
    /// A to-many collection of `T`.
    List(Box<Ty>),
    /// An optional (0..1) `T`.
    Option(Box<Ty>),
    /// The type of the `null` literal; see spec R2 and R3.
    Null,
}

impl Ty {
    /// The primitive `int` type.
    pub fn int() -> Self {
        Ty::Primitive(PrimitiveType::Int)
    }

    /// The primitive `long` type.
    pub fn long() -> Self {
        Ty::Primitive(PrimitiveType::Long)
    }

    /// The primitive `boolean` type.
    pub fn boolean() -> Self {
        Ty::Primitive(PrimitiveType::Boolean)
    }

    /// The primitive `string` type.
    pub fn string() -> Self {
        Ty::Primitive(PrimitiveType::String)
    }

    /// A named type of the given kind, package, and name.
    pub fn named(kind: NamedKind, package: impl Into<String>, name: impl Into<String>) -> Self {
        Ty::Named {
            kind,
            package: package.into(),
            name: name.into(),
        }
    }

    /// A class type in the given package.
    pub fn class(package: impl Into<String>, name: impl Into<String>) -> Self {
        Ty::named(NamedKind::Class, package, name)
    }

    /// Wraps this type in `List`.
    pub fn list(self) -> Self {
        Ty::List(Box::new(self))
    }

    /// Wraps this type in `Option`.
    pub fn optional(self) -> Self {
        Ty::Option(Box::new(self))
    }

    /// The element type of a `List`, or `None` for any other type.
    pub fn element(&self) -> Option<&Ty> {
        match self {
            Ty::List(element) => Some(element),
            _ => None,
        }
    }

    /// The inner type of an `Option`, or `None` for any other type.
    pub fn inner(&self) -> Option<&Ty> {
        match self {
            Ty::Option(inner) => Some(inner),
            _ => None,
        }
    }

    /// Converts a resolved Core IR type reference.
    pub fn from_type_ref(type_ref: &TypeRef) -> Self {
        match type_ref {
            TypeRef::Primitive(primitive) => Ty::Primitive(*primitive),
            TypeRef::Class { package, name } => Ty::named(NamedKind::Class, package, name),
            TypeRef::Enum { package, name } => Ty::named(NamedKind::Enum, package, name),
            TypeRef::Datatype { package, name } => Ty::named(NamedKind::Datatype, package, name),
            TypeRef::Interface { package, name } => Ty::named(NamedKind::Interface, package, name),
            TypeRef::Vocabulary { package, name } => {
                Ty::named(NamedKind::Vocabulary, package, name)
            }
        }
    }

    /// Whether this is a numeric primitive (`int long short float double byte`).
    pub fn is_numeric(&self) -> bool {
        matches!(self, Ty::Primitive(p) if p.is_numeric())
    }

    /// Whether this is an integer primitive (`int long short byte`).
    pub fn is_integer(&self) -> bool {
        matches!(
            self,
            Ty::Primitive(
                PrimitiveType::Int
                    | PrimitiveType::Long
                    | PrimitiveType::Short
                    | PrimitiveType::Byte
            )
        )
    }
}

impl fmt::Display for Ty {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Ty::Primitive(primitive) => write!(f, "{primitive}"),
            Ty::Named { kind, name, .. } => write!(f, "{kind} `{name}`"),
            Ty::List(element) => write!(f, "List<{element}>"),
            Ty::Option(inner) => write!(f, "Option<{inner}>"),
            Ty::Null => f.write_str("null"),
        }
    }
}
