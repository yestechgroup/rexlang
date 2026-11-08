//! Shared value types of the IR: [`TypeRef`], [`PrimitiveType`],
//! [`Multiplicity`], and [`DefaultValue`].

use std::fmt;

use serde::{Deserialize, Serialize};

/// Cardinality bounds of a [`Feature`](crate::Feature), e.g. `0..*` or `1..1`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Multiplicity {
    /// Inclusive lower bound (`0`, `1`, ...).
    pub lower: u32,
    /// Inclusive upper bound.
    pub upper: Upper,
}

impl Multiplicity {
    /// `0..1`
    pub const OPTIONAL: Self = Self {
        lower: 0,
        upper: Upper::Finite(1),
    };
    /// `1..1`
    pub const REQUIRED: Self = Self {
        lower: 1,
        upper: Upper::Finite(1),
    };
    /// `0..*`
    pub const MANY: Self = Self {
        lower: 0,
        upper: Upper::Unbounded,
    };

    /// Creates a multiplicity from explicit bounds.
    pub const fn new(lower: u32, upper: Upper) -> Self {
        Self { lower, upper }
    }

    /// `true` if the feature may hold more than one value.
    pub const fn is_many(&self) -> bool {
        match self.upper {
            Upper::Finite(upper) => upper > 1,
            Upper::Unbounded => true,
        }
    }

    /// `true` if a collection/value of exactly `n` elements satisfies these
    /// bounds.
    pub const fn contains(&self, n: u32) -> bool {
        if n < self.lower {
            return false;
        }
        match self.upper {
            Upper::Finite(upper) => n <= upper,
            Upper::Unbounded => true,
        }
    }
}

/// The default multiplicity is [`Multiplicity::OPTIONAL`] (`0..1`), matching
/// the Ecore convention for unannotated features.
impl Default for Multiplicity {
    fn default() -> Self {
        Self::OPTIONAL
    }
}

/// The upper bound of a [`Multiplicity`].
///
/// Serializes as `"unbounded"` or `{"finite": <n>}`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Upper {
    /// An explicit inclusive upper bound.
    Finite(u32),
    /// `*` — unbounded.
    Unbounded,
}

/// The opposite end of a bidirectional relation.
///
/// Names are resolved within the same package as the owning class.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OppositeRef {
    /// Name of the class holding the opposite feature.
    pub class: String,
    /// Name of the opposite feature within that class.
    pub feature: String,
}

/// A default value for a [`Feature`](crate::Feature).
///
/// Adjacently tagged: `{"type": "int", "value": 42}`. Tags are camelCase:
/// `"string"`, `"int"`, `"bool"`, `"enumLiteral"`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "camelCase")]
pub enum DefaultValue {
    /// A string default.
    String(String),
    /// An integer default.
    Int(i64),
    /// A boolean default.
    Bool(bool),
    /// An enum literal default, referenced by literal name within the
    /// feature's enum type.
    EnumLiteral(String),
}

/// A fully **resolved** type reference. Never an unresolved name.
///
/// Adjacently tagged: `{"type": "class", "value": {"package": "p",
/// "name": "Book"}}`; primitives are `{"type": "primitive", "value":
/// "string"}`. Tags are camelCase: `"primitive"`, `"class"`, `"enum"`,
/// `"datatype"`, `"interface"`, `"vocabulary"`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "camelCase")]
pub enum TypeRef {
    /// A built-in primitive.
    Primitive(PrimitiveType),
    /// A class, by qualified name.
    Class {
        /// Owning package name.
        package: String,
        /// Class name within that package.
        name: String,
    },
    /// An enum, by qualified name.
    Enum {
        /// Owning package name.
        package: String,
        /// Enum name within that package.
        name: String,
    },
    /// A datatype, by qualified name.
    Datatype {
        /// Owning package name.
        package: String,
        /// Datatype name within that package.
        name: String,
    },
    /// An interface, by qualified name.
    Interface {
        /// Owning package name.
        package: String,
        /// Interface name within that package.
        name: String,
    },
    /// A vocabulary of enumerated key values (e.g. the ISO 4217 currencies),
    /// by qualified name. Attribute features may use vocabulary types; their
    /// values are entry keys of the referenced [`VocabularyDef`](crate::VocabularyDef).
    Vocabulary {
        /// Owning package name.
        package: String,
        /// Vocabulary name within that package.
        name: String,
    },
}

impl TypeRef {
    /// Returns the `package::name` qualified name of a named reference, or
    /// `None` for primitives.
    pub fn qualified_name(&self) -> Option<String> {
        match self {
            TypeRef::Primitive(_) => None,
            TypeRef::Class { package, name }
            | TypeRef::Enum { package, name }
            | TypeRef::Datatype { package, name }
            | TypeRef::Interface { package, name }
            | TypeRef::Vocabulary { package, name } => Some(format!("{package}::{name}")),
        }
    }
}

/// The built-in primitives (Xcore's Java-style primitives, plus the
/// calendar `date`).
///
/// Serializes as a bare lowercase string: `"string"`, `"int"`, `"long"`,
/// `"short"`, `"float"`, `"double"`, `"boolean"`, `"byte"`, `"char"`,
/// `"date"`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PrimitiveType {
    /// UTF-16-ish text (`String`).
    String,
    /// 32-bit signed integer (`int`).
    Int,
    /// 64-bit signed integer (`long`).
    Long,
    /// 16-bit signed integer (`short`).
    Short,
    /// 32-bit float (`float`).
    Float,
    /// 64-bit float (`double`).
    Double,
    /// Truth value (`boolean`).
    Boolean,
    /// 8-bit signed integer (`byte`).
    Byte,
    /// A single character (`char`).
    Char,
    /// A calendar date without time or timezone (`date`), serialized as an
    /// ISO-8601 `YYYY-MM-DD` string on the wire. Additive (wire contract
    /// rule 13).
    Date,
}

impl PrimitiveType {
    /// `true` for `Int`, `Long`, `Short`, `Float`, `Double`, and `Byte`.
    pub const fn is_numeric(self) -> bool {
        matches!(
            self,
            Self::Int | Self::Long | Self::Short | Self::Float | Self::Double | Self::Byte
        )
    }
}

impl fmt::Display for PrimitiveType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            Self::String => "string",
            Self::Int => "int",
            Self::Long => "long",
            Self::Short => "short",
            Self::Float => "float",
            Self::Double => "double",
            Self::Boolean => "boolean",
            Self::Byte => "byte",
            Self::Char => "char",
            Self::Date => "date",
        };
        f.write_str(name)
    }
}
