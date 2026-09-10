//! Syntax crate for rexlang (`.mox` sources): a logos-based lexer, a
//! fault-tolerant chumsky-based parser, and the spanned AST they produce.
//!
//! Typical use:
//!
//! ```
//! let result = rex_syntax::parse("class Book { int pages }");
//! assert!(result.errors.is_empty());
//! let model = result.ast.unwrap();
//! assert_eq!(model.declarations.len(), 1);
//! ```

pub mod ast;
pub mod fmt;
pub mod lexer;
pub mod parser;

pub use ast::{
    AnnotationDecl, BindingEntry, ClassDecl, DatatypeDecl, Decl, DefaultValue, EnumDecl,
    EnumLiteral, FeatureDecl, InterfaceDecl, Model, MultBound, Multiplicity, MultiplicityKind,
    Name, Param, QualifiedName, Span, TypeRef, VocabularyDecl, VocabularyFacetDecl, Wraps,
};
pub use fmt::{format, FormatError};
pub use lexer::{lex, lex_with_comments, Comment, CommentKind, LexError, Token};
pub use parser::{parse, ParseError, ParseResult};
