//! Syntax crate for rexlang: a logos-based lexer, a fault-tolerant
//! chumsky-based parser, and the spanned AST they produce for `.mox` model
//! sources and `.actor` actor-policy sources.
//!
//! Typical use:
//!
//! ```
//! let result = rex_syntax::parse("class Book { int pages }");
//! assert!(result.errors.is_empty());
//! let model = result.ast.unwrap();
//! assert_eq!(model.declarations.len(), 1);
//! ```
//!
//! `.actor` files use the separate [`parse_actors`] entry point, mirroring
//! the shape of [`parse`].

pub mod ast;
pub mod fmt;
pub mod lexer;
pub mod parser;

pub use ast::{
    ActorFile, AnnotationDecl, BindingEntry, ClassDecl, DatatypeDecl, Decl, DefaultValue, EnumDecl,
    EnumLiteral, FeatureDecl, ImportDecl, InterfaceDecl, Model, MultBound, Multiplicity,
    MultiplicityKind, Name, Param, QualifiedName, Span, TypeRef, VocabularyDecl,
    VocabularyFacetDecl, Wraps,
};
pub use fmt::{format, format_actors, FormatError};
pub use lexer::{lex, lex_with_comments, Comment, CommentKind, LexError, Token};
pub use parser::{parse, parse_actors, ActorsParseResult, ParseError, ParseResult};
