//! A comment-preserving formatter for `.mox` sources.
//!
//! The formatter is *token-stream based*: it never builds the AST and never
//! reports syntax errors, so sources with parse errors still format. Only a
//! lex error (which cannot be tokenized) is rejected. See [`format`] for the
//! entry point and the module-level style it implements.
//!
//! Style rules implemented here:
//!
//! * 4 spaces of indentation per `{}` nesting level; one declaration per
//!   line; features, enum literals, binding entries and vocabulary items one
//!   per line.
//! * Single space between tokens, except: no space before `,`/`;`/`]`/`.`,
//!   no space after `[`, multiplicities emitted as a unit (`Book[]`,
//!   `Book[3]`, `[0..1]`, `[1..*]`, `[3..5]`; `[0..*]` normalizes to `[]`),
//!   and `{` preceded by one space.
//! * `}` goes on its own line at the parent indent; an empty `{}` stays
//!   inline (a comment inside the braces forces the multiline shape).
//! * Blank lines: exactly one between top-level declarations, none at the
//!   document start, exactly one `\n` at EOF, and no blank lines inside
//!   bodies.
//! * `id`/`readonly` modifiers normalize to that order, single-spaced,
//!   before the feature's type. Enum literals keep `as`/`=` only when
//!   present in the source.
//! * Comments are preserved verbatim. An own-line comment attaches to the
//!   following line at its indentation (before a closing `}` it keeps the
//!   body indent); consecutive own-line comments form a block. A comment that
//!   followed code on the same source line stays trailing on that line. A
//!   multi-line block comment keeps its internal newlines; only its first
//!   line is indented.
//! * The raw `{ ... }` body of an `op`/`derived` feature is emitted on one
//!   line (braces balanced, single spaces), since its contents are not
//!   grammar.

use crate::ast::Span;
use crate::lexer::{lex_with_comments, CommentKind, LexError, Token};

/// A formatting failure: the source could not be tokenized.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum FormatError {
    /// The source contains a lex error; formatting refuses to guess.
    #[error("cannot format: {0}")]
    Lex(#[from] LexError),
}

/// Format a `.mox` source text, preserving comments verbatim.
///
/// Returns the formatted text ending in exactly one `\n` (empty input formats
/// to empty output), or [`FormatError::Lex`] if the source cannot be tokenized.
pub fn format(source: &str) -> Result<String, FormatError> {
    let (tokens, comments) = lex_with_comments(source)?;
    let mut fmt = Formatter::new(source, tokens, comments);
    Ok(fmt.run())
}

/// One node of the merged stream the formatter walks: a token or a comment,
/// ordered by source position.
#[derive(Debug, Clone)]
enum Node<'src> {
    Token(Token<'src>, Span),
    Comment {
        kind: CommentKind,
        text: &'src str,
        span: Span,
        /// Whether the comment started on its own source line (as opposed to
        /// following code on the same line).
        own_line: bool,
    },
}

impl Node<'_> {
    fn span(&self) -> Span {
        match self {
            Node::Token(_, span) => *span,
            Node::Comment { span, .. } => *span,
        }
    }
}

/// What kind of `{ ... }` body is being formatted; decides how items inside
/// are split across lines.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BodyKind {
    /// Features (attributes, containments, ops, ...).
    Class,
    /// Enum literals.
    Enum,
    /// `name "string"` binding entries (interfaces and datatypes).
    Bindings,
    /// `version`/`key`/`facet` items.
    Vocabulary,
}

struct Formatter<'src> {
    nodes: Vec<Node<'src>>,
    pos: usize,
    out: String,
    /// The line under construction (without indentation).
    line: String,
    /// Indentation captured when `line` was started.
    line_indent: usize,
    indent: usize,
    /// Own-line comments waiting to be emitted before the next token.
    pending: Vec<(CommentKind, &'src str)>,
    /// When set, the next text joins the line without a leading space
    /// (used after `(` so param lists read `size(String unit)`).
    glue_next: bool,
}

impl<'src> Formatter<'src> {
    fn new(
        source: &'src str,
        tokens: Vec<(Token<'src>, Span)>,
        comments: Vec<crate::lexer::Comment<'src>>,
    ) -> Self {
        // Merge tokens and comments into one position-ordered stream. Both
        // inputs are sorted, so a stable sort keeps within-token order.
        let mut nodes: Vec<Node<'src>> = tokens
            .into_iter()
            .map(|(token, span)| Node::Token(token, span))
            .chain(comments.into_iter().map(|comment| Node::Comment {
                kind: comment.kind,
                text: comment.text,
                span: comment.span,
                own_line: false,
            }))
            .collect();
        nodes.sort_by_key(|node| node.span().start);

        // Classify each comment: it is an own-line comment when it did not
        // follow code on the same source line.
        let line_starts = {
            let mut starts = vec![0];
            for (index, byte) in source.bytes().enumerate() {
                if byte == b'\n' {
                    starts.push(index + 1);
                }
            }
            starts
        };
        let line_of = |byte: usize| -> usize {
            line_starts
                .partition_point(|&start| start <= byte)
                .saturating_sub(1)
        };
        let mut prev_end_line: Option<usize> = None;
        for node in &mut nodes {
            if let Node::Comment { span, own_line, .. } = node {
                *own_line = prev_end_line.is_none_or(|line| line != line_of(span.start));
            }
            let span = node.span();
            prev_end_line = Some(line_of(span.end.saturating_sub(1)));
        }

        Formatter {
            nodes,
            pos: 0,
            out: String::new(),
            line: String::new(),
            line_indent: 0,
            indent: 0,
            pending: Vec::new(),
            glue_next: false,
        }
    }

    fn run(&mut self) -> String {
        loop {
            let front = self.front().cloned();
            match front {
                None => break,
                Some(Node::Comment { .. }) => self.advance(),
                Some(Node::Token(token, _)) => match token {
                    Token::Package => self.scan_package(),
                    Token::Annotation => self.scan_annotation(),
                    Token::Class => self.scan_class(),
                    Token::Interface => self.scan_interface(),
                    Token::Enum => self.scan_enum(),
                    Token::Type => self.scan_datatype(),
                    Token::Vocabulary => self.scan_vocabulary(),
                    _ => self.scan_top_junk(),
                },
            }
        }
        self.finish()
    }

    // --- node consumption ----------------------------------------------------

    fn front(&self) -> Option<&Node<'src>> {
        self.nodes.get(self.pos)
    }

    /// Consumes the front node and emits it: own-line comments are queued
    /// (flushed just before the next token), inline comments join the current
    /// line, tokens are pushed with canonical spacing.
    fn advance(&mut self) {
        let node = match self.front() {
            Some(node) => node.clone(),
            None => return,
        };
        self.pos += 1;
        match node {
            Node::Token(token, _) => {
                let tight = is_tight(&token);
                let text = token_text(&token);
                self.flush_pending(self.indent);
                self.push_text(&text, tight);
                // `(`, `.` and `[` also glue to whatever *follows* them.
                self.glue_next = matches!(token, Token::LParen | Token::Dot | Token::LBracket);
            }
            Node::Comment {
                kind,
                text,
                own_line: true,
                ..
            } => {
                self.pending.push((kind, text));
            }
            Node::Comment {
                kind,
                text,
                own_line: false,
                ..
            } => {
                if self.line.is_empty() && self.out.ends_with('\n') {
                    // The code this comment trails was already flushed
                    // (e.g. a `}` closed by the body scanner): reopen that
                    // line and re-attach the comment.
                    self.out.pop();
                    self.out.push(' ');
                    self.out.push_str(text);
                    if matches!(kind, CommentKind::Line) {
                        self.out.push('\n');
                    }
                    return;
                }
                if self.line.is_empty() {
                    self.line_indent = self.indent;
                }
                self.push_text(text, false);
                if matches!(kind, CommentKind::Line) {
                    // A trailing line comment always ends its line.
                    self.flush_line();
                }
            }
        }
    }

    /// Consumes the front token without emitting its text (used when the text
    /// is re-emitted later, e.g. reordered modifiers or normalized
    /// multiplicities). Still flushes queued comments first.
    fn bump_token(&mut self) {
        if matches!(self.front(), Some(Node::Token(..))) {
            self.pos += 1;
            self.flush_pending(self.indent);
        }
    }

    /// Drains comment nodes ahead of the next token so token-lookahead
    /// helpers see through comments.
    fn drain_comments(&mut self) {
        while matches!(self.front(), Some(Node::Comment { .. })) {
            self.advance();
        }
    }

    /// The next token, skipping over (and thereby handling) any comments.
    fn peek_tok(&mut self) -> Option<Token<'src>> {
        self.drain_comments();
        match self.front() {
            Some(Node::Token(token, _)) => Some(token.clone()),
            _ => None,
        }
    }

    /// Consumes and emits the next token when it matches `pred`.
    fn take_if(&mut self, pred: impl Fn(&Token) -> bool) -> bool {
        self.drain_comments();
        match self.front() {
            Some(Node::Token(token, _)) if pred(token) => {
                self.advance();
                true
            }
            _ => false,
        }
    }

    // --- output building -----------------------------------------------------

    fn push_text(&mut self, text: &str, tight_before: bool) {
        let glued = std::mem::take(&mut self.glue_next);
        if self.line.is_empty() {
            self.line_indent = self.indent;
        }
        if !self.line.is_empty() && !tight_before && !glued {
            self.line.push(' ');
        }
        self.line.push_str(text);
    }

    /// Emits the current line with its indentation and a `\n`.
    fn flush_line(&mut self) {
        if self.line.is_empty() {
            return;
        }
        self.out.push_str(&indent_str(self.line_indent));
        self.out.push_str(&self.line);
        self.out.push('\n');
        self.line.clear();
        self.glue_next = false;
    }

    /// Emits queued own-line comments, each on its own line at `indent`.
    /// Closes the line under construction first, if any.
    fn flush_pending(&mut self, indent: usize) {
        if self.pending.is_empty() {
            return;
        }
        self.flush_line();
        for (_, text) in std::mem::take(&mut self.pending) {
            self.out.push_str(&indent_str(indent));
            self.out.push_str(text);
            self.out.push('\n');
        }
    }

    /// Starts a top-level declaration: closes the previous line, separates it
    /// with exactly one blank line (unless at the document start), then emits
    /// any queued comments ahead of the declaration.
    fn begin_top_decl(&mut self) {
        self.flush_line();
        if !self.out.is_empty() && !self.out.ends_with("\n\n") {
            self.out.push('\n');
        }
        self.flush_pending(0);
    }

    /// Ends the document: exactly one `\n` at EOF, with any trailing
    /// comment block separated by a blank line.
    fn finish(&mut self) -> String {
        self.flush_line();
        if !self.pending.is_empty() {
            if !self.out.is_empty() && !self.out.ends_with("\n\n") {
                self.out.push('\n');
            }
            self.flush_pending(0);
        }
        std::mem::take(&mut self.out)
    }

    // --- shared scans --------------------------------------------------------

    fn scan_qname(&mut self) {
        if !self.take_name() {
            return;
        }
        while matches!(self.peek_tok(), Some(Token::Dot)) {
            self.advance();
            self.take_name();
        }
    }

    fn take_name(&mut self) -> bool {
        self.take_if(|token| matches!(token, Token::Ident(_) | Token::IdentEscaped(_)))
    }

    /// Consumes and emits an optional `[...]` multiplicity, normalized as a
    /// single bracketed unit (no inner spacing, `[0..*]` becomes `[]`).
    fn scan_multiplicity(&mut self) {
        if !matches!(self.peek_tok(), Some(Token::LBracket)) {
            return;
        }
        // Both brackets are consumed silently; the normalized unit carries
        // its own brackets.
        self.bump_token();
        let mut parts: Vec<Token<'src>> = Vec::new();
        loop {
            match self.peek_tok() {
                Some(Token::RBracket) | None => break,
                Some(token) => {
                    self.bump_token();
                    parts.push(token);
                }
            }
        }
        self.bump_token();
        let text = normalize_multiplicity(&parts);
        self.push_text(&text, true);
    }

    fn scan_opposite(&mut self) {
        if self.take_if(|token| matches!(token, Token::Opposite)) {
            self.take_name();
        }
    }

    fn scan_params(&mut self) {
        if !self.take_if(|token| matches!(token, Token::LParen)) {
            return;
        }
        if self.take_if(|token| matches!(token, Token::RParen)) {
            return;
        }
        loop {
            self.scan_qname();
            self.take_name();
            if !self.take_if(|token| matches!(token, Token::Comma)) {
                break;
            }
        }
        self.take_if(|token| matches!(token, Token::RParen));
    }

    fn scan_default(&mut self) {
        if !self.take_if(|token| matches!(token, Token::Eq)) {
            return;
        }
        match self.peek_tok() {
            Some(Token::Str(_) | Token::Int(_) | Token::True | Token::False) => {
                self.take_if(|token| {
                    matches!(
                        token,
                        Token::Str(_) | Token::Int(_) | Token::True | Token::False
                    )
                });
            }
            Some(Token::Ident(_) | Token::IdentEscaped(_)) => self.scan_qname(),
            _ => {}
        }
    }

    /// Consumes and emits the balanced `{ ... }` raw body of an
    /// `op`/`derived` feature on a single line.
    fn scan_raw_body(&mut self) {
        if !self.take_if(|token| matches!(token, Token::LBrace)) {
            return;
        }
        let mut depth = 1;
        while depth > 0 {
            match self.peek_tok() {
                None => break,
                Some(Token::LBrace) => {
                    self.advance();
                    depth += 1;
                }
                Some(Token::RBrace) => {
                    self.advance();
                    depth -= 1;
                }
                Some(_) => self.advance(),
            }
        }
    }

    /// Consumes and emits the `{ ... }` body of a declaration, or just a
    /// line break when the body is absent. An empty `{}` stays inline (a
    /// comment inside the braces forces the multiline shape).
    fn scan_body(&mut self, kind: BodyKind) {
        self.drain_comments();
        // `pos` is at the `{` here; the body is empty when a bare `}` (not a
        // comment, which is multiline-forcing) immediately follows.
        let empty = matches!(
            self.nodes.get(self.pos + 1),
            Some(Node::Token(Token::RBrace, _))
        );
        if !self.take_if(|token| matches!(token, Token::LBrace)) {
            self.flush_line();
            return;
        }
        if empty {
            self.pos += 1;
            self.push_text("}", true);
            self.flush_line();
            return;
        }
        self.flush_line();
        self.indent += 1;
        loop {
            match self.front() {
                Some(Node::Token(Token::RBrace, _)) | None => break,
                _ => {}
            }
            self.flush_pending(self.indent);
            match kind {
                BodyKind::Class => self.scan_class_item(),
                BodyKind::Enum => self.scan_enum_literal(),
                BodyKind::Bindings => self.scan_binding(),
                BodyKind::Vocabulary => self.scan_vocabulary_item(),
            }
            self.flush_line();
        }
        self.close_body();
    }

    /// Emits the closing `}` of a body on its own line at the parent indent.
    /// Queued comments keep the body indent (they read as annotations of the
    /// body, not of the closing brace).
    fn close_body(&mut self) {
        self.flush_line();
        self.flush_pending(self.indent);
        self.indent -= 1;
        self.take_if(|token| matches!(token, Token::RBrace));
        self.flush_line();
    }

    /// Consumes ungrammatical tokens as one junk line, stopping at the tokens
    /// that can begin a proper item (or `stop`).
    fn scan_junk_until(&mut self, stop: impl Fn(&Token) -> bool) {
        loop {
            match self.peek_tok() {
                Some(token) if !stop(&token) => self.advance(),
                _ => break,
            }
        }
    }

    // --- class body ----------------------------------------------------------

    fn scan_class_item(&mut self) {
        // Leading contextual `id`/`readonly` modifiers: collected (never
        // rewriting them into other tokens), deduplicated, normalized to
        // `id readonly` order, and emitted before the feature's type.
        let mut id = false;
        let mut read_only = false;
        loop {
            match self.peek_tok() {
                Some(Token::Ident("id")) => {
                    self.bump_token();
                    id = true;
                }
                Some(Token::Ident("readonly")) => {
                    self.bump_token();
                    read_only = true;
                }
                _ => break,
            }
        }
        if id {
            self.push_text("id", false);
        }
        if read_only {
            self.push_text("readonly", false);
        }

        match self.peek_tok() {
            Some(Token::Contains) | Some(Token::Refers) => {
                self.advance();
                self.scan_qname();
                self.scan_multiplicity();
                self.take_name();
                self.scan_opposite();
            }
            Some(Token::Container) => {
                self.advance();
                self.scan_qname();
                self.take_name();
                self.scan_opposite();
            }
            Some(Token::Op) => {
                self.advance();
                self.scan_qname();
                self.take_name();
                self.scan_params();
                self.scan_raw_body();
            }
            Some(Token::Derived) => {
                self.advance();
                self.scan_qname();
                self.scan_multiplicity();
                self.take_name();
                self.scan_raw_body();
            }
            Some(Token::Ident(_) | Token::IdentEscaped(_)) => {
                self.scan_qname();
                self.scan_multiplicity();
                self.take_name();
                self.scan_default();
            }
            _ => {
                // Ungrammatical tokens: emit them on one line, stopping where
                // a proper feature could begin (mirrors the parser's junk
                // recovery so the shape survives formatting).
                self.scan_junk_until(|token| {
                    matches!(
                        token,
                        Token::RBrace
                            | Token::Ident(_)
                            | Token::IdentEscaped(_)
                            | Token::Contains
                            | Token::Refers
                            | Token::Container
                            | Token::Op
                            | Token::Derived
                    )
                });
            }
        }
    }

    // --- other bodies --------------------------------------------------------

    fn scan_enum_literal(&mut self) {
        if self
            .peek_tok()
            .is_some_and(|token| matches!(token, Token::Ident(_) | Token::IdentEscaped(_)))
        {
            self.take_name();
            if self.take_if(|token| matches!(token, Token::As)) {
                self.take_if(|token| matches!(token, Token::Str(_)));
            }
            if self.take_if(|token| matches!(token, Token::Eq)) {
                self.take_if(|token| matches!(token, Token::Int(_)));
            }
        } else {
            self.scan_junk_until(|token| {
                matches!(
                    token,
                    Token::RBrace | Token::Ident(_) | Token::IdentEscaped(_)
                )
            });
        }
    }

    fn scan_binding(&mut self) {
        if self
            .peek_tok()
            .is_some_and(|token| matches!(token, Token::Ident(_) | Token::IdentEscaped(_)))
        {
            self.take_name();
            self.take_if(|token| matches!(token, Token::Str(_)));
        } else {
            self.scan_junk_until(|token| {
                matches!(
                    token,
                    Token::RBrace | Token::Ident(_) | Token::IdentEscaped(_)
                )
            });
        }
    }

    fn scan_vocabulary_item(&mut self) {
        match self.peek_tok() {
            Some(Token::Version) => {
                self.advance();
                self.take_if(|token| matches!(token, Token::Str(_)));
            }
            Some(Token::Key) => {
                self.advance();
                self.take_name();
            }
            Some(Token::Facet) => {
                self.advance();
                self.scan_qname();
                self.take_name();
            }
            _ => self.scan_junk_until(|token| {
                matches!(
                    token,
                    Token::RBrace | Token::Version | Token::Key | Token::Facet
                )
            }),
        }
    }

    // --- top-level declarations ----------------------------------------------

    fn scan_package(&mut self) {
        self.begin_top_decl();
        self.advance();
        self.scan_qname();
        self.flush_line();
    }

    fn scan_annotation(&mut self) {
        self.begin_top_decl();
        self.advance();
        self.take_if(|token| matches!(token, Token::Str(_)));
        if self.take_if(|token| matches!(token, Token::As)) {
            self.take_name();
        }
        self.flush_line();
    }

    fn scan_class(&mut self) {
        self.begin_top_decl();
        self.advance();
        self.scan_qname();
        if self.take_if(|token| matches!(token, Token::Extends)) {
            self.scan_qname();
            while self.take_if(|token| matches!(token, Token::Comma)) {
                self.scan_qname();
            }
        }
        self.scan_body(BodyKind::Class);
    }

    fn scan_interface(&mut self) {
        self.begin_top_decl();
        self.advance();
        self.scan_qname();
        self.scan_body(BodyKind::Bindings);
    }

    fn scan_enum(&mut self) {
        self.begin_top_decl();
        self.advance();
        self.scan_qname();
        self.scan_body(BodyKind::Enum);
    }

    fn scan_datatype(&mut self) {
        self.begin_top_decl();
        self.advance();
        self.scan_qname();
        if self.take_if(|token| matches!(token, Token::Wraps))
            && !self.take_if(|token| matches!(token, Token::Opaque))
        {
            self.scan_qname();
        }
        self.scan_body(BodyKind::Bindings);
    }

    fn scan_vocabulary(&mut self) {
        self.begin_top_decl();
        self.advance();
        self.scan_qname();
        self.take_if(|token| matches!(token, Token::From));
        self.take_if(|token| matches!(token, Token::Str(_)));
        self.scan_body(BodyKind::Vocabulary);
    }

    /// Unrecognized top-level tokens: emit them on one line, stopping at the
    /// next declaration keyword (mirrors the parser's declaration-level
    /// recovery).
    fn scan_top_junk(&mut self) {
        self.begin_top_decl();
        loop {
            match self.peek_tok() {
                Some(token) if !is_top_keyword(&token) => self.advance(),
                _ => break,
            }
        }
        self.flush_line();
    }
}

// --- helpers -------------------------------------------------------------------

fn indent_str(indent: usize) -> String {
    "    ".repeat(indent)
}

fn is_top_keyword(token: &Token<'_>) -> bool {
    matches!(
        token,
        Token::Package
            | Token::Annotation
            | Token::Class
            | Token::Interface
            | Token::Enum
            | Token::Type
            | Token::Vocabulary
    )
}

/// Tokens that hug the preceding text: `,` `;` `.` brackets and parens.
fn is_tight(token: &Token<'_>) -> bool {
    matches!(
        token,
        Token::Comma
            | Token::Dot
            | Token::LParen
            | Token::RParen
            | Token::LBracket
            | Token::RBracket
            | Token::Other(';')
    )
}

/// The canonical text of a token, reconstructing string delimiters and
/// escape carets so payloads round-trip verbatim.
fn token_text(token: &Token<'_>) -> String {
    match token {
        Token::Ident(text) => (*text).to_string(),
        Token::IdentEscaped(text) => format!("^{text}"),
        Token::Str(text) => format!("\"{text}\""),
        Token::Int(value) => value.to_string(),
        Token::Other(char) => char.to_string(),
        Token::Dot => ".".to_string(),
        Token::Comma => ",".to_string(),
        Token::LParen => "(".to_string(),
        Token::RParen => ")".to_string(),
        Token::LBrace => "{".to_string(),
        Token::RBrace => "}".to_string(),
        Token::LBracket => "[".to_string(),
        Token::RBracket => "]".to_string(),
        Token::Eq => "=".to_string(),
        Token::Star => "*".to_string(),
        _ => token.keyword().map(str::to_string).unwrap_or_default(),
    }
}

/// Normalizes a multiplicity's inner tokens to canonical bracketed text:
/// `[]`, `[n]`, `[a..b]`, `[a..*]`; `[0..*]` (and a bare `[*]`) collapses to
/// `[]`. Anything else is echoed tight.
fn normalize_multiplicity(parts: &[Token<'_>]) -> String {
    match parts {
        [] => "[]".to_string(),
        [Token::Star] => "[]".to_string(),
        [Token::Int(value)] => format!("[{value}]"),
        [Token::Int(lower), Token::Dot, Token::Dot, Token::Star] if *lower == 0 => "[]".to_string(),
        [Token::Int(lower), Token::Dot, Token::Dot, bound] => format!(
            "[{lower}..{}]",
            match bound {
                Token::Int(value) => value.to_string(),
                _ => "*".to_string(),
            }
        ),
        other => other.iter().map(token_text).collect::<Vec<_>>().concat(),
    }
}
