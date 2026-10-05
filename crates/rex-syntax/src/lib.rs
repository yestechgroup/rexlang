//! Syntax crate for rexlang: a logos-based lexer, a fault-tolerant
//! chumsky-based parser, and the spanned AST they produce for `.mox` model
//! sources, `.actor` actor-policy sources, `.ddd` design sources, and `.evt`
//! event-contract sources.
//!
//! # Grammar authority
//!
//! The normative EBNF for all four chumsky surfaces lives in the [parser
//! module docs](crate::parser) — the single place the grammar is written
//! down; `docs/LANGUAGE.md` and `docs/DDD.md` link there instead of
//! restating it. The `.ifml` surface is parsed by rex-ifml and its grammar
//! authority is that crate's Pest grammar file
//! (`crates/rex-ifml/src/grammar/ifml.pest`). For `.ddd` keywords the
//! tables in the crate-internal `ddd` module are the authority the parser
//! and formatter share.
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
//! the shape of [`parse`]; `.ddd` files use [`parse_ddd`] the same way, and
//! `.evt` files use [`parse_evt`].

pub mod ast;
pub(crate) mod ddd;
pub mod fmt;
pub mod lexer;
pub mod parser;

pub use ast::{
    ActorFile, AnnotationDecl, BindingEntry, ChannelDecl, ClassDecl, DatatypeDecl, DddBase,
    DddBuiltinOp, DddDelegation, DddDesign, DddDocumentEntry, DddFile, DddFlagKind, DddFlags,
    DddModule, DddPagination, DddParam, DddRanking, DddRepository, DddRepositoryOp, DddSearch,
    DddSearchField, DddService, DddServiceOp, DddStereotype, Decl, DefaultValue, EnumDecl,
    EnumLiteral, EventDecl, EventFieldDecl, EvtFile, FeatureDecl, ImportDecl, ImportKind,
    InterfaceDecl, Model, MultBound, Multiplicity, MultiplicityKind, Name, Param, PublishesDecl,
    QualifiedName, Span, SubscriptionDecl, TypeRef, VocabularyDecl, VocabularyFacetDecl, Wraps,
};
pub use fmt::{format, format_actors, format_ddd, format_evt, FormatError};
pub use lexer::{lex, lex_with_comments, Comment, CommentKind, LexError, Token};
pub use parser::{
    parse, parse_actors, parse_ddd, parse_evt, ActorsParseResult, DddParseResult, EvtParseResult,
    ParseError, ParseResult,
};
