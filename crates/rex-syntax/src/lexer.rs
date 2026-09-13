//! Token definitions and the lexer entry point.

use std::fmt;

use logos::Logos;

use crate::ast::Span;

/// A lexical token. Payloads borrow from the source text.
///
/// Keywords can be used as identifiers by prefixing them with `^`; the lexer
/// emits [`Token::IdentEscaped`] with the caret stripped from the payload,
/// while the token span still covers the raw `^keyword` text.
///
/// Note that `get`, `set`, `id` and `readonly` are *not* keywords: they lex as
/// ordinary identifiers so they may be used as feature names unescaped.
#[derive(Debug, Clone, PartialEq, Eq, Logos)]
pub enum Token<'src> {
    #[token("package")]
    Package,
    #[token("annotation")]
    Annotation,
    #[token("as")]
    As,
    #[token("class")]
    Class,
    #[token("extends")]
    Extends,
    #[token("interface")]
    Interface,
    #[token("enum")]
    Enum,
    #[token("type")]
    Type,
    #[token("wraps")]
    Wraps,
    #[token("opaque")]
    Opaque,
    #[token("contains")]
    Contains,
    #[token("refers")]
    Refers,
    #[token("container")]
    Container,
    #[token("opposite")]
    Opposite,
    #[token("op")]
    Op,
    #[token("derived")]
    Derived,
    #[token("true")]
    True,
    #[token("false")]
    False,
    #[token("vocabulary")]
    Vocabulary,
    #[token("from")]
    From,
    #[token("version")]
    Version,
    #[token("key")]
    Key,
    #[token("facet")]
    Facet,
    #[token("actors")]
    Actors,
    #[token("actor")]
    Actor,
    #[token("capability")]
    Capability,
    #[token("grant")]
    Grant,
    #[token("permit")]
    Permit,
    #[token("forbid")]
    Forbid,
    #[token("when")]
    When,
    #[token("obligation")]
    Obligation,
    #[token("on")]
    On,
    #[token("never_both")]
    NeverBoth,
    #[token("cedar")]
    Cedar,
    #[token("import")]
    Import,

    /// An identifier: `[A-Za-z_][A-Za-z0-9_]*`.
    #[regex("[A-Za-z_][A-Za-z0-9_]*", |lexer| lexer.slice())]
    Ident(&'src str),
    /// An escaped keyword `^name`; the payload has the leading `^` stripped.
    #[regex(r"\^[A-Za-z_][A-Za-z0-9_]*", |lexer| &lexer.slice()[1..])]
    IdentEscaped(&'src str),
    /// A string literal payload: the raw text between the quotes, with
    /// escapes left intact (unescaping happens in the parser).
    #[regex(r#""([^"\\\n\r]|\\.)*""#, |lexer| {
        let slice = lexer.slice();
        &slice[1..slice.len() - 1]
    })]
    Str(&'src str),
    /// A decimal integer literal with optional leading `-`.
    #[regex("-?[0-9]+", |lexer| lexer.slice().parse::<i64>().map_err(|_| ()))]
    Int(i64),

    #[token(".")]
    Dot,
    #[token(",")]
    Comma,
    #[token("(")]
    LParen,
    #[token(")")]
    RParen,
    #[token("{")]
    LBrace,
    #[token("}")]
    RBrace,
    #[token("[")]
    LBracket,
    #[token("]")]
    RBracket,
    #[token("=")]
    Eq,
    #[token("*")]
    Star,

    // Trivia: whitespace is skipped outright. Comments are captured as
    // dedicated tokens (priority 1 beats the `Other` catch-all) so that
    // [`lex_with_comments`] can hand them to the formatter; [`lex`] filters
    // them out, keeping the parser's input unchanged. Block comments are
    // non-nested; the shape of the regex is the classic DFA-friendly "C block
    // comment" pattern (logos rejects lazy quantifiers).
    #[regex(r"[ \t\r\n\f]+", logos::skip, priority = 1)]
    Whitespace,

    /// A `// ...` line comment; the payload is the verbatim text including
    /// the leading `//` and stops before the line break.
    #[regex(r"//[^\r\n]*", |lexer| lexer.slice(), priority = 1)]
    LineComment(&'src str),
    /// A `/* ... */` block comment; the payload is the verbatim text
    /// including the delimiters and any internal newlines.
    #[regex(r"/\*[^*]*\*+([^/*][^*]*\*+)*/", |lexer| lexer.slice(), priority = 1)]
    BlockComment(&'src str),

    /// Emitted by logos for characters that match no pattern. This keeps raw
    /// `op`/`derived` bodies tokenizable regardless of their contents
    /// (operators, arrows, ...). Priority 0 makes every more specific pattern
    /// (keywords, identifiers, punctuation, skipped whitespace/comments) win.
    #[regex(".", |lexer| lexer.slice().chars().next().unwrap_or('\u{0}'), priority = 0)]
    Other(char),

    /// Emitted by logos for characters that match no pattern. Given the
    /// [`Token::Other`] catch-all this should never occur, but it is kept for
    /// robustness.
    Error,
}

impl Token<'_> {
    /// The static text of a keyword token, if this is a keyword.
    pub fn keyword(&self) -> Option<&'static str> {
        Some(match self {
            Token::Package => "package",
            Token::Annotation => "annotation",
            Token::As => "as",
            Token::Class => "class",
            Token::Extends => "extends",
            Token::Interface => "interface",
            Token::Enum => "enum",
            Token::Type => "type",
            Token::Wraps => "wraps",
            Token::Opaque => "opaque",
            Token::Contains => "contains",
            Token::Refers => "refers",
            Token::Container => "container",
            Token::Opposite => "opposite",
            Token::Op => "op",
            Token::Derived => "derived",
            Token::True => "true",
            Token::False => "false",
            Token::Vocabulary => "vocabulary",
            Token::From => "from",
            Token::Version => "version",
            Token::Key => "key",
            Token::Facet => "facet",
            Token::Actors => "actors",
            Token::Actor => "actor",
            Token::Capability => "capability",
            Token::Grant => "grant",
            Token::Permit => "permit",
            Token::Forbid => "forbid",
            Token::When => "when",
            Token::Obligation => "obligation",
            Token::On => "on",
            Token::NeverBoth => "never_both",
            Token::Cedar => "cedar",
            Token::Import => "import",
            _ => return None,
        })
    }
}

impl fmt::Display for Token<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(keyword) = self.keyword() {
            return write!(f, "`{keyword}`");
        }
        match self {
            Token::Ident(text) => write!(f, "`{text}`"),
            Token::IdentEscaped(text) => write!(f, "`^{text}`"),
            Token::Str(_) => f.write_str("string literal"),
            Token::Int(value) => write!(f, "`{value}`"),
            Token::Other(char) => write!(f, "`{char}`"),
            Token::Dot => f.write_str("`.`"),
            Token::Comma => f.write_str("`,`"),
            Token::LParen => f.write_str("`(`"),
            Token::RParen => f.write_str("`)`"),
            Token::LBrace => f.write_str("`{`"),
            Token::RBrace => f.write_str("`}`"),
            Token::LBracket => f.write_str("`[`"),
            Token::RBracket => f.write_str("`]`"),
            Token::Eq => f.write_str("`=`"),
            Token::Star => f.write_str("`*`"),
            Token::Error => f.write_str("invalid token"),
            // Keyword variants are handled by the `keyword()` early return;
            // this arm is unreachable but keeps the match exhaustive.
            _ => f.write_str("token"),
        }
    }
}

/// A lexical error: an input region that matched no token pattern.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("invalid token at byte offset {}", span.start)]
pub struct LexError {
    /// Span of the offending text.
    pub span: Span,
}

/// Tokenize a source text.
///
/// Whitespace and comments (`//` line, `/* */` block) are skipped. Returns the
/// tokens paired with their spans, or a [`LexError`] describing the first
/// unmatched region.
pub fn lex(source: &str) -> Result<Vec<(Token<'_>, Span)>, LexError> {
    lex_with_comments(source).map(|(tokens, _)| tokens)
}

/// The kind of a captured comment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommentKind {
    /// A `// ...` line comment.
    Line,
    /// A `/* ... */` block comment (possibly spanning multiple lines).
    Block,
}

/// A comment captured by [`lex_with_comments`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Comment<'src> {
    /// Whether the comment is a line or block comment.
    pub kind: CommentKind,
    /// Verbatim comment text, including the `//` or `/* */` delimiters and
    /// any internal newlines.
    pub text: &'src str,
    /// Span of the comment in the source.
    pub span: Span,
}

impl Comment<'_> {
    /// The normalized description text when this is a *doc* comment
    /// (`/// ...` or `/** ... */`), or `None` for an ordinary comment.
    ///
    /// Normalization strips the doc delimiters, one optional leading space
    /// per line, and the `*` continuation markers of block comments; the
    /// lines are joined with `\n`.
    pub fn doc_content(&self) -> Option<String> {
        match self.kind {
            CommentKind::Line => {
                let content = self.text.strip_prefix("///")?;
                Some(strip_one_space(content).trim_end().to_string())
            }
            CommentKind::Block => {
                let interior = self
                    .text
                    .strip_prefix("/**")?
                    .strip_suffix("*/")?
                    .trim_end();
                if interior.is_empty() {
                    return Some(String::new());
                }
                let mut lines: Vec<String> = Vec::new();
                for line in interior.split('\n').skip(1) {
                    // Continuation lines drop their leading whitespace and
                    // the conventional `*` marker.
                    let mut line = line.trim_start();
                    line = line.strip_prefix('*').unwrap_or(line);
                    lines.push(strip_one_space(line).trim_end().to_string());
                }
                // The first interior line sits on the `/**` line itself.
                let first = strip_one_space(interior.split('\n').next().unwrap_or_default())
                    .trim_start()
                    .trim_end()
                    .to_string();
                let mut all = Vec::with_capacity(lines.len() + 1);
                if !first.is_empty() {
                    all.push(first);
                }
                all.extend(lines);
                while all.first().is_some_and(String::is_empty) {
                    all.remove(0);
                }
                while all.last().is_some_and(String::is_empty) {
                    all.pop();
                }
                Some(all.join("\n"))
            }
        }
    }
}

/// Strips a single leading space, if present.
fn strip_one_space(text: &str) -> &str {
    text.strip_prefix(' ').unwrap_or(text)
}

/// Tokens paired with their spans.
pub type TokenStream<'src> = Vec<(Token<'src>, Span)>;

/// Comments in source order.
pub type Comments<'src> = Vec<Comment<'src>>;

/// Tokenize a source text, keeping comments.
///
/// Like [`lex`], but comments are returned alongside the token stream (in
/// source order, never overlapping token spans). The parser consumes [`lex`],
/// so comment capture never affects parsing.
pub fn lex_with_comments(source: &str) -> Result<(TokenStream<'_>, Comments<'_>), LexError> {
    let mut tokens = Vec::new();
    let mut comments = Vec::new();
    for (result, range) in Token::lexer(source).spanned() {
        match result {
            Err(_) | Ok(Token::Error) => return Err(LexError { span: range.into() }),
            Ok(Token::LineComment(text)) => comments.push(Comment {
                kind: CommentKind::Line,
                text,
                span: range.into(),
            }),
            Ok(Token::BlockComment(text)) => comments.push(Comment {
                kind: CommentKind::Block,
                text,
                span: range.into(),
            }),
            Ok(token) => tokens.push((token, range.into())),
        }
    }
    Ok((tokens, comments))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(source: &str) -> Vec<Token<'_>> {
        lex(source).unwrap().into_iter().map(|(t, _)| t).collect()
    }

    #[test]
    fn lexes_basic_class() {
        let tokens = lex("class Book { int pages }").unwrap();
        assert_eq!(tokens[0], (Token::Class, (0..5).into()));
        assert_eq!(tokens[1], (Token::Ident("Book"), (6..10).into()));
        assert_eq!(tokens[2], (Token::LBrace, (11..12).into()));
        assert_eq!(tokens[3], (Token::Ident("int"), (13..16).into()));
        assert_eq!(tokens[4], (Token::Ident("pages"), (17..22).into()));
        assert_eq!(tokens[5], (Token::RBrace, (23..24).into()));
        assert_eq!(tokens.len(), 6);
    }

    #[test]
    fn contextual_modifiers_are_idents() {
        assert_eq!(
            kinds("get set id readonly"),
            vec![
                Token::Ident("get"),
                Token::Ident("set"),
                Token::Ident("id"),
                Token::Ident("readonly"),
            ]
        );
    }

    #[test]
    fn vocabulary_keywords_lex_as_keywords() {
        assert_eq!(
            kinds("vocabulary from version key facet"),
            vec![
                Token::Vocabulary,
                Token::From,
                Token::Version,
                Token::Key,
                Token::Facet,
            ]
        );
    }

    #[test]
    fn new_keywords_escape_like_any_keyword() {
        let tokens = lex("^facet ^key ^vocabulary").unwrap();
        assert_eq!(tokens[0].0, Token::IdentEscaped("facet"));
        assert_eq!(tokens[1].0, Token::IdentEscaped("key"));
        assert_eq!(tokens[2].0, Token::IdentEscaped("vocabulary"));
        // Spans still cover the raw text including the caret.
        assert_eq!(tokens[0].1, (0..6).into());
    }

    #[test]
    fn actor_keywords_lex_as_keywords() {
        assert_eq!(
            kinds(
                "actors actor capability grant permit forbid when obligation on never_both cedar"
            ),
            vec![
                Token::Actors,
                Token::Actor,
                Token::Capability,
                Token::Grant,
                Token::Permit,
                Token::Forbid,
                Token::When,
                Token::Obligation,
                Token::On,
                Token::NeverBoth,
                Token::Cedar,
            ]
        );
    }

    #[test]
    fn actor_keywords_escape_like_any_keyword() {
        let tokens = lex("^actors ^when ^on ^cedar ^never_both").unwrap();
        assert_eq!(tokens[0].0, Token::IdentEscaped("actors"));
        assert_eq!(tokens[1].0, Token::IdentEscaped("when"));
        assert_eq!(tokens[2].0, Token::IdentEscaped("on"));
        assert_eq!(tokens[3].0, Token::IdentEscaped("cedar"));
        assert_eq!(tokens[4].0, Token::IdentEscaped("never_both"));
        // Spans still cover the raw text including the caret.
        assert_eq!(tokens[0].1, (0..7).into());
        assert_eq!(tokens[4].1, (25..36).into());
    }

    #[test]
    fn import_lexes_as_keyword_and_escapes() {
        assert_eq!(kinds("import"), vec![Token::Import]);
        let tokens = lex("^import").unwrap();
        assert_eq!(tokens[0].0, Token::IdentEscaped("import"));
        // Span still covers the raw text including the caret.
        assert_eq!(tokens[0].1, (0..7).into());
    }

    #[test]
    fn escaped_keyword_strips_caret() {
        let tokens = lex("^class").unwrap();
        assert_eq!(tokens[0].0, Token::IdentEscaped("class"));
        // Span still covers the raw text including the caret.
        assert_eq!(tokens[0].1, (0..6).into());
    }

    #[test]
    fn strings_and_ints() {
        assert_eq!(kinds(r#""a\"b\\c""#), vec![Token::Str("a\\\"b\\\\c")]);
        assert_eq!(
            kinds("-42 0 007"),
            vec![Token::Int(-42), Token::Int(0), Token::Int(7)]
        );
    }

    #[test]
    fn comments_and_whitespace_are_skipped() {
        assert_eq!(
            kinds("// line\nclass /* block */ A"),
            vec![Token::Class, Token::Ident("A")]
        );
    }

    #[test]
    fn unknown_chars_become_other_tokens() {
        assert_eq!(
            kinds("@ # > ! :"),
            vec![
                Token::Other('@'),
                Token::Other('#'),
                Token::Other('>'),
                Token::Other('!'),
                Token::Other(':'),
            ]
        );
    }
}
