//! rex-expr — the Tier 2 expression sublanguage front half: parser and
//! type-checker for the small, closed expression language described in
//! [docs/EXPRESSIONS.md] (the deliberate Xbase replacement).
//!
//! The language is deliberately small — literals, feature access, operation
//! calls, arithmetic/comparison, `if`/`let`, and a fixed collection algebra —
//! with the four disagreeing-language semantics pinned as rules **R1**–**R4**
//! in the spec document (integer overflow, string equality, option/null
//! propagation, division by zero).
//!
//! Typical use:
//!
//! ```
//! use rex_expr::{TypeChecker, TypeContext, parse};
//! use rex_ir::{ClassDef, Feature, FeatureKind, Multiplicity, Model, Package, PrimitiveType, TypeRef};
//!
//! let mut model = Model::new();
//! let mut package = Package::new("demo");
//! package.classes.push(ClassDef::new(
//!     "Book",
//!     vec![],
//!     vec![Feature::new(
//!         "pages",
//!         FeatureKind::Attribute,
//!         TypeRef::Primitive(PrimitiveType::Int),
//!         Multiplicity::REQUIRED,
//!     )],
//! ));
//! model.packages.push(package);
//!
//! let parsed = parse("book.pages + 1");
//! assert!(parsed.errors.is_empty());
//!
//! let checker = TypeChecker::new(TypeContext::from_model(&model))
//!     .with_binding("book", rex_expr::Ty::class("demo", "Book"));
//! let ty = checker.type_of(parsed.ast.as_ref().unwrap()).expect("well-typed");
//! assert_eq!(ty, rex_expr::Ty::int());
//! ```
//!
//! [docs/EXPRESSIONS.md]: https://github.com/yestechgroup/rexlang/blob/main/docs/EXPRESSIONS.md

pub mod ast;
pub mod checker;
pub mod error;
pub mod lexer;
pub mod parser;
pub mod types;

pub use ast::{AlgebraKind, BinOp, Expr, ExprKind, Span, Spanned, UnOp};
pub use checker::{TypeChecker, TypeContext};
pub use error::{ExprError, ParseResult};
pub use parser::parse;
pub use types::{NamedKind, Ty};
