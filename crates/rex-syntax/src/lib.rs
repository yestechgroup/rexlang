//! Syntax crate for rexlang: a logos-based lexer, a fault-tolerant
//! chumsky-based parser, and the spanned AST they produce for `.mox` model
//! sources, `.actor` actor-policy sources, `.ddd` design sources, `.evt`
//! event-contract sources, `.deploy` deployment sources, and `.uc` use case
//! sources.
//!
//! # Grammar authority
//!
//! The normative EBNF for the chumsky surfaces lives in the [parser
//! module docs](crate::parser) — the single place the grammar is written
//! down; `docs/LANGUAGE.md` and `docs/DDD.md` link there instead of
//! restating it. The `.ifml` surface is parsed by rex-ifml and its grammar
//! authority is that crate's Pest grammar file
//! (`crates/rex-ifml/src/grammar/ifml.pest`). For `.ddd` keywords the
//! tables in the crate-internal `ddd` module are the authority the parser
//! and formatter share; the `deploy` module plays the same role for
//! `.deploy`, and the `usecases` module for `.uc`.
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
//! the shape of [`parse`]; `.ddd` files use [`parse_ddd`], `.evt` files use
//! [`parse_evt`], `.deploy` files use [`parse_deploy`], and `.uc` files use
//! [`parse_usecases`].

pub mod ast;
pub(crate) mod cmmn;
pub(crate) mod ddd;
pub(crate) mod deploy;
pub mod fmt;
pub mod lexer;
pub mod parser;
pub(crate) mod usecases;

pub use ast::{
    ActorFile, AnnotationDecl, BindingEntry, ChannelDecl, ClassDecl, DatatypeDecl, DddBase,
    DddBuiltinOp, DddDelegation, DddDesign, DddDocumentEntry, DddFile, DddFlagKind, DddFlags,
    DddMachine, DddMachineTransition, DddModule, DddPagination, DddParam, DddRanking,
    DddRepository, DddRepositoryOp, DddSearch, DddSearchField, DddService, DddServiceOp,
    DddStereotype, Decl, DefaultValue, CmmnCase, CmmnCriterion, CmmnExecute, CmmnFile,
    CmmnFileItem, CmmnImport, CmmnItemDecl, CmmnItemKind, CmmnOnPart, CmmnRole,
    DeployApplicationDecl, DeployBindingDecl, DeployComponentDecl, DeployConnectionDecl,
    DeployDeploymentDecl, DeployFile, DeployMappingDecl, DeployPolicyDecl, DeployProfileDecl,
    DeploySettingDecl, DeploySettingValue, EnumDecl, EnumLiteral, EventDecl, EventFieldDecl,
    EvtFile, FeatureDecl, ImportDecl, ImportKind, InterfaceDecl, Model, MultBound, Multiplicity,
    MultiplicityKind, Name, Param, PublishesDecl, QualifiedName, Span, SubscriptionDecl, TypeRef,
    UcDecl, UcEndpoint, UcFile, UcImport, UcMessage, UcNodeRef, UcOccurrence, UcParticipant,
    UcRealizes, UcStep, UcSuccession, VocabularyDecl, VocabularyFacetDecl, Wraps,
};
pub use fmt::{
    format, format_actors, format_ddd, format_deploy, format_evt, format_usecases, FormatError,
};
pub use lexer::{lex, lex_with_comments, Comment, CommentKind, LexError, Token};
pub use parser::{
    parse, parse_actors, parse_cmmn, parse_ddd, parse_deploy, parse_evt, parse_usecases,
    ActorsParseResult, CmmnParseResult, DddParseResult, DeployParseResult, EvtParseResult,
    ParseError, ParseResult, UcParseResult,
};
