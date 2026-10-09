//! Single source of truth for the `.ddd` design DSL's contextual keywords.
//!
//! Every `.ddd` contextual keyword is an ordinary identifier that is special
//! only in a particular grammar position, so two consumers must agree on the
//! same word list: the parser (which matches them positionally) and the
//! formatter (which recognizes them and re-emits some groups in a canonical
//! order). This module is the one table both consume:
//!
//! * [`STEREOTYPES`] — the stereotype keywords, in grammar order. The
//!   parser's `choice` alternatives are built from this table in order; the
//!   formatter's design dispatch recognizes `abstract` plus these words
//!   ([`starts_design_decl`]).
//! * [`DESIGN_FLAGS`] — the design flags, in **canonical order**: the parser
//!   accepts them in any source order, the formatter re-emits them in table
//!   order. [`DddFlagKind::keyword`] in [`crate::ast`] derives from this
//!   table too.
//! * [`REPOSITORY_BUILTINS`] — the built-in repository operations; the
//!   parser and the formatter recognize them through
//!   [`builtin_from_keyword`], and [`DddBuiltinOp::from_keyword`] and
//!   [`DddBuiltinOp::keyword`] derive from the same rows.
//! * [`SEARCH_MEMBERS`] — the search-body member kinds, in **canonical
//!   order**: the parser's member `choice` alternatives are built from this
//!   table in order, and the formatter sorts a search body into this order
//!   ([`search_member_rank`]) and dispatches its scans by
//!   [`search_member_kind`].
//!
//! The remaining single words ([`APPLICATION`], [`BASE`], [`MODULE`],
//! [`SERVICE`], [`SEARCH`], [`ABSTRACT`], [`REPOSITORY`], [`INJECT`],
//! [`BOOST`], [`ANALYZER`], [`LIMIT`], [`MAX`], [`CURSOR`]) are matched on
//! both sides at fixed grammar positions; they carry no canonical order.
//! The `ranking` value words (`bm25`, `tfIdf`, `exact`, `custom`) are
//! parser-only — the formatter re-emits whatever follows the `ranking`
//! keyword — so they stay in the parser alone.
//!
//! Tests at the bottom pin every order against the spec (mirrored by
//! `docs/DDD.md`), so a deliberate table reorder fails a test instead of
//! silently changing canonical output.

use crate::ast::{DddBuiltinOp, DddFlagKind, DddStereotype};
use crate::lexer::Token;

/// The `application <name> { ... }` keyword.
pub(crate) const APPLICATION: &str = "application";
/// The `base <qualified-name>` keyword of the application body.
pub(crate) const BASE: &str = "base";
/// The `module <name> { ... }` keyword of the application body.
pub(crate) const MODULE: &str = "module";
/// The `service <name> { ... }` keyword of a module body.
pub(crate) const SERVICE: &str = "service";
/// The `search <name> { ... }` keyword of a module body.
pub(crate) const SEARCH: &str = "search";
/// The optional `abstract` modifier of a design declaration.
pub(crate) const ABSTRACT: &str = "abstract";
/// The `repository <name> { ... }` keyword of a design declaration.
pub(crate) const REPOSITORY: &str = "repository";
/// The `inject <name>;` keyword of a service body.
pub(crate) const INJECT: &str = "inject";
/// The optional `protected` visibility modifier of a service or repository
/// operation (Sculptor's `protected` — the operation stays off the public
/// interface). A contextual word like the rest: in leading position it is
/// the modifier, anywhere else an ordinary name.
pub(crate) const PROTECTED: &str = "protected";
/// The `boost <int>` keyword of a `text` field.
pub(crate) const BOOST: &str = "boost";
/// The `analyzer "<...>"` keyword, both as a search member and as a `text`
/// field clause.
pub(crate) const ANALYZER: &str = "analyzer";
/// The `limit <int>` member of a `pagination` block.
pub(crate) const LIMIT: &str = "limit";
/// The `max <int>` member of a `pagination` block.
pub(crate) const MAX: &str = "max";
/// The `cursor` member of a `pagination` block.
pub(crate) const CURSOR: &str = "cursor";

/// The stereotype keywords, in grammar order: the parser's `choice`
/// alternatives are built from this table in order, and the canonical
/// order is the table order.
pub(crate) const STEREOTYPES: [(&str, DddStereotype); 3] = [
    ("entity", DddStereotype::Entity),
    ("value", DddStereotype::Value),
    ("dto", DddStereotype::Dto),
];

/// The design flags, in canonical order: the parser accepts them in any
/// source order; the formatter re-emits them in this order whatever the
/// source order (and `DddFlags`' field order in the AST mirrors the table).
pub(crate) const DESIGN_FLAGS: [(&str, DddFlagKind); 5] = [
    ("scaffold", DddFlagKind::Scaffold),
    ("auditable", DddFlagKind::Auditable),
    ("optimisticLocking", DddFlagKind::OptimisticLocking),
    ("nonPersistent", DddFlagKind::NonPersistent),
    ("cache", DddFlagKind::Cache),
];

/// The built-in repository operations, in table order (their recognition
/// is order-independent; the order pins the keyword list).
pub(crate) const REPOSITORY_BUILTINS: [(&str, DddBuiltinOp); 6] = [
    ("findById", DddBuiltinOp::FindById),
    ("findAll", DddBuiltinOp::FindAll),
    ("findByExample", DddBuiltinOp::FindByExample),
    ("findByKeys", DddBuiltinOp::FindByKeys),
    ("save", DddBuiltinOp::Save),
    ("delete", DddBuiltinOp::Delete),
];

/// One search-body member kind. The `capability` clause is led by the real
/// `capability` token rather than an identifier, which is why its keyword
/// slot is [`None`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SearchMemberKind {
    /// `entity <qualified-name>`
    Entity,
    /// `text { ... }`
    Text,
    /// `filters { ... }`
    Filters,
    /// `sort { ... }`
    Sort,
    /// `document { ... }`
    Document,
    /// `ranking <...>`
    Ranking,
    /// `analyzer "<...>"`
    Analyzer,
    /// `pagination { ... }`
    Pagination,
    /// the `capability <name>` clause
    Capability,
}

/// The search-body member kinds, in canonical order: the parser's member
/// alternatives are built from this table in order, and the formatter sorts
/// a search body into this order whatever the source order.
pub(crate) const SEARCH_MEMBERS: [(SearchMemberKind, Option<&str>); 9] = [
    (SearchMemberKind::Entity, Some("entity")),
    (SearchMemberKind::Text, Some("text")),
    (SearchMemberKind::Filters, Some("filters")),
    (SearchMemberKind::Sort, Some("sort")),
    (SearchMemberKind::Document, Some("document")),
    (SearchMemberKind::Ranking, Some("ranking")),
    (SearchMemberKind::Analyzer, Some(ANALYZER)),
    (SearchMemberKind::Pagination, Some("pagination")),
    (SearchMemberKind::Capability, None),
];

/// Whether the identifier starts a design declaration: the optional
/// `abstract` modifier or a stereotype keyword.
pub(crate) fn starts_design_decl(text: &str) -> bool {
    text == ABSTRACT || STEREOTYPES.iter().any(|(keyword, _)| *keyword == text)
}

/// The design flag introduced by the given identifier, if any.
pub(crate) fn flag_kind(text: &str) -> Option<DddFlagKind> {
    DESIGN_FLAGS
        .iter()
        .find(|(keyword, _)| *keyword == text)
        .map(|(_, kind)| *kind)
}

/// The keyword of a design flag, as written in the source.
pub(crate) fn flag_keyword(kind: DddFlagKind) -> &'static str {
    DESIGN_FLAGS
        .iter()
        .find(|(_, k)| *k == kind)
        .map(|(keyword, _)| *keyword)
        .expect("the design-flag table covers every DddFlagKind")
}

/// The built-in repository operation introduced by the given identifier,
/// if any.
pub(crate) fn builtin_from_keyword(text: &str) -> Option<DddBuiltinOp> {
    REPOSITORY_BUILTINS
        .iter()
        .find(|(keyword, _)| *keyword == text)
        .map(|(_, builtin)| *builtin)
}

/// The keyword of a built-in repository operation, as written in the source.
pub(crate) fn builtin_keyword(builtin: DddBuiltinOp) -> &'static str {
    REPOSITORY_BUILTINS
        .iter()
        .find(|(_, b)| *b == builtin)
        .map(|(keyword, _)| *keyword)
        .expect("the repository-builtin table covers every DddBuiltinOp")
}

/// The search-member kind led by the given token, if any.
pub(crate) fn search_member_kind(token: &Token<'_>) -> Option<SearchMemberKind> {
    match token {
        Token::Ident(text) => SEARCH_MEMBERS
            .iter()
            .find(|(_, keyword)| *keyword == Some(text))
            .map(|(kind, _)| *kind),
        Token::Capability => Some(SearchMemberKind::Capability),
        _ => None,
    }
}

/// The canonical rank of a search member's leading token: the row index in
/// [`SEARCH_MEMBERS`], so the formatter's stable sort puts members in table
/// order whatever the source order.
pub(crate) fn search_member_rank(token: &Token<'_>) -> Option<usize> {
    match token {
        Token::Ident(text) => SEARCH_MEMBERS
            .iter()
            .position(|(_, keyword)| *keyword == Some(*text)),
        Token::Capability => SEARCH_MEMBERS
            .iter()
            .position(|(_, keyword)| keyword.is_none()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::{DddBuiltinOp, DddFlagKind, DddStereotype};
    use crate::fmt::format_ddd;

    /// The canonical orders, pinned independently of the tables. This is
    /// today's spec (mirrored by `docs/DDD.md`), so a deliberate table
    /// reorder fails here even though the parser and formatter would stay
    /// mutually consistent.
    #[test]
    fn canonical_orders_match_the_pinned_spec() {
        assert_eq!(
            DESIGN_FLAGS.map(|(keyword, _)| keyword),
            [
                "scaffold",
                "auditable",
                "optimisticLocking",
                "nonPersistent",
                "cache"
            ],
        );
        assert_eq!(
            SEARCH_MEMBERS.map(|(_, keyword)| keyword),
            [
                Some("entity"),
                Some("text"),
                Some("filters"),
                Some("sort"),
                Some("document"),
                Some("ranking"),
                Some("analyzer"),
                Some("pagination"),
                None,
            ],
        );
        assert_eq!(
            STEREOTYPES.map(|(keyword, _)| keyword),
            ["entity", "value", "dto"],
        );
        assert_eq!(
            REPOSITORY_BUILTINS.map(|(keyword, _)| keyword),
            [
                "findById",
                "findAll",
                "findByExample",
                "findByKeys",
                "save",
                "delete"
            ],
        );
    }

    /// Every AST variant has exactly one row in its table, so adding a flag,
    /// builtin, stereotype, or search-member kind to the AST without a
    /// table row (or duplicating one) fails.
    #[test]
    fn tables_cover_every_ast_variant_exactly_once() {
        for kind in [
            DddFlagKind::Scaffold,
            DddFlagKind::Auditable,
            DddFlagKind::OptimisticLocking,
            DddFlagKind::NonPersistent,
            DddFlagKind::Cache,
        ] {
            assert_eq!(DESIGN_FLAGS.iter().filter(|(_, k)| *k == kind).count(), 1);
        }
        for builtin in [
            DddBuiltinOp::FindById,
            DddBuiltinOp::FindAll,
            DddBuiltinOp::FindByExample,
            DddBuiltinOp::FindByKeys,
            DddBuiltinOp::Save,
            DddBuiltinOp::Delete,
        ] {
            assert_eq!(
                REPOSITORY_BUILTINS
                    .iter()
                    .filter(|(_, b)| *b == builtin)
                    .count(),
                1
            );
        }
        for stereotype in [
            DddStereotype::Entity,
            DddStereotype::Value,
            DddStereotype::Dto,
        ] {
            assert_eq!(
                STEREOTYPES.iter().filter(|(_, s)| *s == stereotype).count(),
                1
            );
        }
        for kind in [
            SearchMemberKind::Entity,
            SearchMemberKind::Text,
            SearchMemberKind::Filters,
            SearchMemberKind::Sort,
            SearchMemberKind::Document,
            SearchMemberKind::Ranking,
            SearchMemberKind::Analyzer,
            SearchMemberKind::Pagination,
            SearchMemberKind::Capability,
        ] {
            assert_eq!(SEARCH_MEMBERS.iter().filter(|(k, _)| *k == kind).count(), 1);
        }
    }

    /// The keyword helpers round-trip through the tables: keyword → variant
    /// → keyword is the identity, and every row's keyword is discoverable
    /// from its variant alone (this is what the AST's `keyword` methods
    /// rely on).
    #[test]
    fn keyword_helpers_round_trip_through_the_tables() {
        for (keyword, kind) in DESIGN_FLAGS {
            assert_eq!(flag_kind(keyword), Some(kind));
            assert_eq!(flag_keyword(kind), keyword);
        }
        for (keyword, builtin) in REPOSITORY_BUILTINS {
            assert_eq!(builtin_from_keyword(keyword), Some(builtin));
            assert_eq!(builtin_keyword(builtin), keyword);
        }
        assert_eq!(flag_kind("nope"), None);
        assert_eq!(builtin_from_keyword("nope"), None);
    }

    /// The search-member ranks are exactly the table positions, so the
    /// formatter's stable sort reproduces the table order.
    #[test]
    fn search_member_ranks_follow_the_table() {
        for (rank, (_, keyword)) in SEARCH_MEMBERS.iter().enumerate() {
            let token = match keyword {
                Some(keyword) => Token::Ident(keyword),
                None => Token::Capability,
            };
            assert_eq!(search_member_rank(&token), Some(rank));
            assert_eq!(search_member_kind(&token), Some(SEARCH_MEMBERS[rank].0));
        }
        assert_eq!(search_member_rank(&Token::Ident("mystery")), None);
        assert_eq!(search_member_kind(&Token::Ident("mystery")), None);
    }

    /// A `.ddd` sample whose flags and search members are written in the
    /// *reverse* of the table order renders with both groups in table
    /// order — the formatter's canonical orders are derivable from the
    /// tables. (The parser's alternatives are built from the same tables in
    /// the same order, so parser and formatter cannot disagree without a
    /// table mutation, which the pinned test catches.)
    #[test]
    fn fmt_canonical_order_is_derived_from_the_tables() {
        let flags = DESIGN_FLAGS
            .iter()
            .rev()
            .map(|(keyword, _)| *keyword)
            .collect::<Vec<_>>()
            .join(" ");
        let source = [
            "application Demo {".to_string(),
            "  module demo {".to_string(),
            format!("    entity Widget {flags} repository WidgetRepository {{"),
            "      delete;".to_string(),
            "      findById;".to_string(),
            "    }".to_string(),
            "    search WidgetSearch {".to_string(),
            "      capability SearchWidgets".to_string(),
            "      pagination {".to_string(),
            "        cursor".to_string(),
            "      }".to_string(),
            "      analyzer \"english\"".to_string(),
            "      ranking bm25".to_string(),
            "      document {".to_string(),
            "        headline = title;".to_string(),
            "      }".to_string(),
            "      sort {".to_string(),
            "        title".to_string(),
            "      }".to_string(),
            "      filters {".to_string(),
            "        genre".to_string(),
            "      }".to_string(),
            "      text {".to_string(),
            "        title boost 3".to_string(),
            "      }".to_string(),
            "      entity Widget".to_string(),
            "    }".to_string(),
            "  }".to_string(),
            "}".to_string(),
            String::new(),
        ]
        .join("\n");
        let formatted = format_ddd(&source).unwrap();

        // The design line emits the flags in table order.
        let design_line = formatted
            .lines()
            .find(|line| line.contains("entity Widget"))
            .expect("the design declaration survives formatting");
        let mut last = design_line.find("Widget").unwrap();
        for (keyword, _) in DESIGN_FLAGS {
            let at = design_line[last..]
                .find(keyword)
                .unwrap_or_else(|| panic!("`{keyword}` missing from the design line"));
            assert!(at > 0, "`{keyword}` overlaps the design's name");
            last += at;
        }

        // The search body emits the members in table order.
        let (_, search_body) = formatted
            .split_once("search WidgetSearch")
            .expect("the search declaration survives formatting");
        let mut last = 0;
        for (_, keyword) in SEARCH_MEMBERS
            .iter()
            .take_while(|(_, keyword)| keyword.is_some())
        {
            let keyword = keyword.unwrap();
            let at = search_body[last..]
                .find(keyword)
                .unwrap_or_else(|| panic!("`{keyword}` missing from the search body"));
            assert!(at > 0, "`{keyword}` overlaps the previous member");
            last += at;
        }
    }
}
