//! A comment-preserving formatter for `.mox` sources.
//!
//! The formatter is *token-stream based*: it never builds the AST and never
//! reports syntax errors, so sources with parse errors still format. Only a
//! lex error (which cannot be tokenized) is rejected. See [`format()`] for the
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
//!   present in the source. An attribute's constraint block renders
//!   single-spaced on the feature's line (`{ pattern "..." minLength 3 }`);
//!   an empty block collapses to `{}`.
//! * Comments are preserved verbatim. An own-line comment attaches to the
//!   following line at its indentation (before a closing `}` it keeps the
//!   body indent); consecutive own-line comments form a block. A comment that
//!   followed code on the same source line stays trailing on that line. A
//!   multi-line block comment keeps its internal newlines; only its first
//!   line is indented.
//! * The raw `{ ... }` body of a `derived` feature is emitted on one line
//!   (braces balanced, single spaces), since its contents are not grammar.
//! * An `op` body is target-tagged (`{ <target> { ... } ... }`): the op's
//!   `{` closes its own line and each `<target> {` block sits at the next
//!   indent. A target body whose inner bytes contain no newline renders
//!   inline (`<target> { …bytes… }`); a multi-line target body keeps its
//!   inner bytes VERBATIM (leading brace newline dropped, trailing whitespace
//!   trimmed) between the `<target> {` line and a `}` at the `<target>`
//!   indent. Because body bytes are emitted untouched, re-formatting the
//!   output is a fixpoint. A bare (untagged) body still renders inline on
//!   the feature's line (the driver rejects it semantically). The same
//!   rendering applies to a datatype's `create { ... }`/`convert { ... }`
//!   blocks and to a grant's raw `cedar { ... }` entry.
//! * Inside `actors { ... }`, `grant { ... }` and `delegation { ... }`
//!   bodies, items keep their grouping: at most one separating blank line
//!   between items is preserved (all other bodies drop interior blank
//!   lines). A `never_both { A, B }` constraint renders on one line. A `when (...)` condition's inner bytes
//!   are raw too: they are emitted verbatim (ends trimmed), so conditions
//!   are a fixpoint just like target bodies.
//! * An `.actor` source (see [`format_actors`]) formats its imports first in
//!   source order (one per line, tight — no blank lines between them), then
//!   its actors blocks; exactly one blank line separates the import section
//!   from the first block.
//! * A `.ddd` source (see [`format_ddd`]) mirrors the `.actor` layout:
//!   imports are hoisted into a tight first section (one per line, in source
//!   order, each ending in `;`), then the `application` block with exactly
//!   one blank line between the sections. Inside the application the `base`
//!   line comes first, then modules and their members in source order. A
//!   design's flags are canonicalized to the fixed order `scaffold
//!   auditable optimisticLocking nonPersistent cache` regardless of source
//!   order (with `abstract` before the stereotype), and every
//!   operation/`inject`/import line ends in `;`. The `capability a, b`
//!   clause stays on the operation's line. A search projection's members
//!   are canonicalized to the fixed order `entity text filters sort
//!   document ranking analyzer pagination capability` regardless of source
//!   order; clause *contents* (text fields, filter/sort names, document
//!   entries, pagination members) keep their source order, and a document
//!   expression is emitted verbatim token-by-token at canonical spacing
//!   (so `;` inside string literals survives and nested braces balance).
//! * An `import schema` or `import sigil` declaration of a `.mox` source is
//!   hoisted into a canonical section directly after the `package`
//!   declaration (mirroring the `.actor` import rule): the section renders
//!   before every other declaration, one per line, tight — no blank lines
//!   between imports, but one blank line separating the section from the
//!   package above and from the first following declaration. A top-level
//!   item is an import declaration only when it starts `import` → `schema`
//!   or `sigil` → string literal, so keyword or identifier lookalikes
//!   inside dotted references (`nz.package.Thing`, `nz.import.schema.X`,
//!   `nz.import.sigil.X`) are never hoisted.

use crate::ast::Span;
use crate::ddd;
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

/// Format an `.actor` source text, preserving comments verbatim.
///
/// The imports come first in source order, one per line with no blank lines
/// between them; actors blocks follow with the canonical block formatting,
/// and exactly one blank line separates the import section from the first
/// block. Returns the formatted text ending in exactly one `\n` (empty input
/// formats to empty output), or [`FormatError::Lex`] if the source cannot be
/// tokenized.
pub fn format_actors(source: &str) -> Result<String, FormatError> {
    let (tokens, comments) = lex_with_comments(source)?;
    let mut fmt = Formatter::new(source, tokens, comments);
    Ok(fmt.run_actors())
}

/// Format a `.ddd` source text, preserving comments verbatim.
///
/// The canonical layout mirrors [`format_actors`]: `import` declarations are
/// hoisted into a tight first section (one per line, source order, each
/// ending in `;`), followed by the `application` block after exactly one
/// blank line. Inside the application the `base` line comes first, then
/// modules and their members in source order. A design's flags are
/// canonicalized to `scaffold auditable optimisticLocking nonPersistent
/// cache` regardless of source order. Returns the formatted text ending in
/// exactly one `\n` (empty input formats to empty output), or
/// [`FormatError::Lex`] if the source cannot be tokenized.
pub fn format_ddd(source: &str) -> Result<String, FormatError> {
    let (tokens, comments) = lex_with_comments(source)?;
    let mut fmt = Formatter::new(source, tokens, comments);
    Ok(fmt.run_ddd())
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
    /// `actor`/`capability`/`purpose`/`grant`/`never_both` items.
    Actors,
    /// `permit`/`forbid`/`cedar` entries of a `grant` block.
    Grant,
    /// `from`/`to`/`purpose` lines and `permit`/`forbid` entries of a
    /// `delegation`.
    Delegation,
    /// `base`/`module` items of a `.ddd` application.
    DddApplication,
    /// `service`/design items of a `.ddd` module.
    DddModule,
    /// declared/delegated operations and `inject` lines of a `.ddd` service.
    DddService,
    /// builtin/declared operations of a `.ddd` repository.
    DddRepository,
    /// members of a `.ddd` search projection.
    DddSearch,
    /// indexed fields of a `.ddd` search's `text` clause.
    DddSearchField,
    /// names of a `.ddd` search's `filters`/`sort` clauses.
    DddSearchNames,
    /// `name = expr;` entries of a `.ddd` search's `document` clause.
    DddSearchDocument,
    /// `limit`/`max`/`cursor` members of a `.ddd` search's `pagination`.
    DddSearchPagination,
}

impl BodyKind {
    /// Whether separating blank lines between items are preserved (collapsed
    /// to at most one). Only the new `actors`/`grant`/`delegation` bodies
    /// keep them — their items form visual groups — while every other body
    /// kind drops interior blank lines.
    fn keeps_blank_lines(self) -> bool {
        matches!(
            self,
            BodyKind::Actors | BodyKind::Grant | BodyKind::Delegation
        )
    }
}

struct Formatter<'src> {
    /// The original source text; target body contents are sliced verbatim.
    source: &'src str,
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
    /// Whether a `.mox` import declaration (`import schema`/`import sigil`)
    /// was emitted already; the section's first declaration gets the
    /// canonical blank line above it (after the package), the rest render
    /// tight.
    import_emitted: bool,
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
            source,
            nodes,
            pos: 0,
            out: String::new(),
            line: String::new(),
            line_indent: 0,
            indent: 0,
            pending: Vec::new(),
            glue_next: false,
            import_emitted: false,
        }
    }

    fn run(&mut self) -> String {
        self.hoist_import_schemas();
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
                    Token::Actors => self.scan_actors(),
                    Token::Import => self.scan_import_schema(),
                    _ => self.scan_top_junk(),
                },
            }
        }
        self.finish()
    }

    /// Like [`Formatter::run`], but for `.actor` files: the only recognized
    /// top-level constructs are `import` declarations and `actors` blocks.
    fn run_actors(&mut self) -> String {
        loop {
            let front = self.front().cloned();
            match front {
                None => break,
                Some(Node::Comment { .. }) => self.advance(),
                Some(Node::Token(token, _)) => match token {
                    Token::Import => self.scan_import(),
                    Token::Actors => self.scan_actors(),
                    _ => self.scan_top_junk_actors(),
                },
            }
        }
        self.finish()
    }

    /// Like [`Formatter::run_actors`], but for `.ddd` files: the only
    /// recognized top-level constructs are `import` declarations and the
    /// `application` block, with imports hoisted ahead of it.
    fn run_ddd(&mut self) -> String {
        self.hoist_ddd_imports();
        loop {
            let front = self.front().cloned();
            match front {
                None => break,
                Some(Node::Comment { .. }) => self.advance(),
                Some(Node::Token(token, _)) => match token {
                    Token::Import => self.scan_ddd_import(),
                    Token::Ident(ddd::APPLICATION) => self.scan_ddd_application(),
                    _ => self.scan_top_junk_ddd(),
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
        if !self.take_ref_name() {
            return;
        }
        while matches!(self.peek_tok(), Some(Token::Dot)) {
            self.advance();
            self.take_ref_name();
        }
    }

    fn take_name(&mut self) -> bool {
        self.take_if(|token| matches!(token, Token::Ident(_) | Token::IdentEscaped(_)))
    }

    /// Like [`Formatter::take_name`], but also accepts keyword tokens:
    /// inside a dotted reference a keyword is just a name (e.g. the trailing
    /// `actors` in `package rex.conformance.actors`), matching the parser's
    /// `qname_segment`.
    fn take_ref_name(&mut self) -> bool {
        self.take_if(|token| {
            matches!(token, Token::Ident(_) | Token::IdentEscaped(_)) || token.keyword().is_some()
        })
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

    /// Consumes and emits an attribute's `{ pattern "..." minLength 3 }`
    /// constraint block. The empty block stays inline (`{}`); a non-empty
    /// block renders its `keyword value` pairs single-spaced on the
    /// feature's line, with an interior comment (unusual) forcing the
    /// following pairs onto a continuation line — both shapes are fixpoints.
    fn scan_constraint_block(&mut self) {
        if !matches!(self.peek_tok(), Some(Token::LBrace)) {
            return;
        }
        if matches!(
            self.nodes.get(self.pos + 1),
            Some(Node::Token(Token::RBrace, _))
        ) {
            // Empty `{}` stays inline; both brackets are consumed silently.
            self.bump_token();
            self.bump_token();
            self.push_text("{}", false);
            return;
        }
        self.advance(); // the `{` joins the feature's line
        self.indent += 1;
        loop {
            self.flush_pending(self.indent);
            match self.peek_tok() {
                Some(Token::RBrace) | None => break,
                Some(Token::Ident(text)) if is_constraint_keyword(text) => {
                    self.advance();
                    if let Some(Token::Str(_) | Token::Int(_)) = self.peek_tok() {
                        self.advance();
                    }
                }
                _ => {
                    // Ungrammatical tail: one junk line before the close.
                    self.scan_junk_until(|token| matches!(token, Token::RBrace));
                    self.flush_line();
                }
            }
        }
        self.indent -= 1;
        self.take_if(|token| matches!(token, Token::RBrace));
        self.flush_line();
    }

    /// Consumes and emits the balanced `{ ... }` raw body of a `derived`
    /// feature (or a bare op body) on a single line.
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

    /// Consumes and emits a balanced `(...)` region (the raw condition of a
    /// `when` clause). The inner bytes are not grammar: they are emitted
    /// verbatim (ends trimmed) — inline on the current line when they
    /// contain no newline, otherwise as-is between the `(` line and a `)`
    /// at the current indent — mirroring target bodies, so re-formatting is
    /// a fixpoint.
    fn scan_raw_parens(&mut self) {
        let Some(Node::Token(Token::LParen, open_span)) = self.front() else {
            return;
        };
        let open_span = *open_span;
        // The `(` is re-emitted below with the canonical `when (` spacing.
        self.bump_token();
        // Find the matching close paren by depth over the token stream
        // (parens inside comments or string tokens never count).
        let mut depth = 1usize;
        let mut close_index = self.pos;
        while depth > 0 {
            match self.nodes.get(close_index) {
                None => break,
                Some(Node::Token(Token::LParen, _)) => {
                    depth += 1;
                    close_index += 1;
                }
                Some(Node::Token(Token::RParen, _)) => {
                    depth -= 1;
                    close_index += 1;
                }
                Some(_) => close_index += 1,
            }
        }
        // `close_index - 1` is the matching `)` (or the last node when the
        // region was never closed).
        let content_end = self
            .nodes
            .get(close_index - 1)
            .map(|node| node.span().start)
            .unwrap_or(open_span.end);
        let content = &self.source[open_span.end..content_end.max(open_span.end)];
        // Every node inside the parens (tokens *and* comments) is already
        // covered by the verbatim slice; skip them without emitting.
        self.pos = close_index;
        if content.contains('\n') {
            self.push_text("(", false);
            self.flush_line();
            let text = content.strip_prefix('\n').unwrap_or(content).trim_end();
            self.out.push_str(text);
            self.out.push('\n');
            self.out.push_str(&indent_str(self.indent));
            self.out.push_str(")\n");
        } else {
            self.push_text(&format!("({})", content.trim()), false);
        }
    }

    /// Consumes and emits the body of an `op` feature: a target-tagged body
    /// (`{ rust { ... } java { ... } }`) when it has that shape, else a
    /// bare body rendered inline as before.
    fn scan_op_body(&mut self) {
        let tagged = matches!(
            (
                self.nodes.get(self.pos),
                self.nodes.get(self.pos + 1),
                self.nodes.get(self.pos + 2),
            ),
            (
                Some(Node::Token(Token::LBrace, _)),
                Some(Node::Token(Token::Ident(_) | Token::IdentEscaped(_), _)),
                Some(Node::Token(Token::LBrace, _)),
            )
        );
        if tagged {
            self.scan_target_body_list();
        } else {
            self.scan_raw_body();
        }
    }

    /// Consumes and emits `{ <target> { ... } ... }` — the wrapping brace
    /// joins the line under construction (e.g. the op signature or the
    /// `create` keyword), each `<target> { ... }` block starts on its own
    /// line at the next indent, and the wrapping `}` closes at the parent
    /// indent.
    fn scan_target_body_list(&mut self) {
        if !self.take_if(|token| matches!(token, Token::LBrace)) {
            return;
        }
        self.flush_line();
        self.indent += 1;
        loop {
            self.flush_pending(self.indent);
            match self.peek_tok() {
                Some(Token::RBrace) | None => break,
                Some(Token::Ident(_) | Token::IdentEscaped(_))
                    if matches!(
                        self.nodes.get(self.pos + 1),
                        Some(Node::Token(Token::LBrace, _))
                    ) =>
                {
                    self.scan_target_block();
                }
                _ => {
                    // Ungrammatical tail: one junk line before the close.
                    self.scan_junk_until(|token| matches!(token, Token::RBrace));
                    self.flush_line();
                }
            }
        }
        self.indent -= 1;
        self.take_if(|token| matches!(token, Token::RBrace));
        self.flush_line();
    }

    /// Consumes and emits one `<target> { ... }` block at the current indent.
    /// The body's inner bytes are emitted verbatim: single-line contents join
    /// the `<target> {` line; multi-line contents are written as-is between
    /// the `<target> {` line and a `}` at the `<target>` indent (leading
    /// brace newline dropped, trailing whitespace trimmed), which keeps
    /// re-formatting a fixpoint.
    fn scan_target_block(&mut self) {
        self.advance(); // the target name
        self.advance(); // the `{` joins the line
        let Some(Node::Token(_, open_span)) = self.nodes.get(self.pos - 1) else {
            return;
        };
        let open_span = *open_span;
        // Find the matching close brace by depth over the token stream
        // (braces inside comments or string tokens never count).
        let mut depth = 1usize;
        let mut close_index = self.pos;
        while depth > 0 {
            match self.nodes.get(close_index) {
                None => break,
                Some(Node::Token(Token::LBrace, _)) => {
                    depth += 1;
                    close_index += 1;
                }
                Some(Node::Token(Token::RBrace, _)) => {
                    depth -= 1;
                    close_index += 1;
                }
                Some(_) => close_index += 1,
            }
        }
        // `close_index - 1` is the matching `}` (or the last node when the
        // body was never closed).
        let content_end = self
            .nodes
            .get(close_index - 1)
            .map(|node| node.span().start)
            .unwrap_or(open_span.end);
        let content = &self.source[open_span.end..content_end.max(open_span.end)];
        // Every node inside the braces (tokens *and* comments) is already
        // covered by the verbatim slice; skip them without emitting.
        self.pos = close_index;
        if content.contains('\n') {
            self.flush_line();
            let text = content.strip_prefix('\n').unwrap_or(content).trim_end();
            self.out.push_str(text);
            self.out.push('\n');
            self.out.push_str(&indent_str(self.indent));
            self.out.push_str("}\n");
        } else {
            self.line.push_str(content);
            self.line.push('}');
            self.flush_line();
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
        let mut first_item = true;
        loop {
            match self.front() {
                Some(Node::Token(Token::RBrace, _)) | None => break,
                _ => {}
            }
            // Actors/grant bodies keep their item grouping: a blank line in
            // the source becomes at most one blank line in the output (never
            // before the first item, never before the closing brace).
            if !first_item && kind.keeps_blank_lines() && self.blank_line_precedes_front() {
                self.flush_line();
                self.flush_pending(self.indent);
                if !self.out.is_empty() && !self.out.ends_with("\n\n") {
                    self.out.push('\n');
                }
            } else {
                self.flush_pending(self.indent);
            }
            first_item = false;
            match kind {
                BodyKind::Class => self.scan_class_item(),
                BodyKind::Enum => self.scan_enum_literal(),
                BodyKind::Bindings => self.scan_binding(),
                BodyKind::Vocabulary => self.scan_vocabulary_item(),
                BodyKind::Actors => self.scan_actors_item(),
                BodyKind::Grant => self.scan_grant_entry(),
                BodyKind::Delegation => self.scan_delegation_item(),
                BodyKind::DddApplication => self.scan_ddd_application_item(),
                BodyKind::DddModule => self.scan_ddd_module_item(),
                BodyKind::DddService => self.scan_ddd_service_item(),
                BodyKind::DddRepository => self.scan_ddd_repository_item(),
                BodyKind::DddSearch => self.scan_ddd_search_item(),
                BodyKind::DddSearchField => self.scan_ddd_search_field_item(),
                BodyKind::DddSearchNames => self.scan_ddd_search_name_item(),
                BodyKind::DddSearchDocument => self.scan_ddd_search_document_item(),
                BodyKind::DddSearchPagination => self.scan_ddd_search_pagination_item(),
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

    /// Whether the source has a blank line immediately before the front node
    /// (two or more newlines in the whitespace run directly preceding it).
    /// Reading the raw source (not the node stream) keeps this independent of
    /// how many comment nodes were drained in between.
    fn blank_line_precedes_front(&self) -> bool {
        let Some(node) = self.front() else {
            return false;
        };
        let bytes = self.source.as_bytes();
        let mut index = node.span().start;
        let mut newlines = 0;
        while index > 0 {
            match bytes[index - 1] {
                b'\n' => {
                    newlines += 1;
                    index -= 1;
                }
                b' ' | b'\t' | b'\r' | 0x0c => index -= 1,
                _ => break,
            }
        }
        newlines >= 2
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
                self.scan_op_body();
            }
            Some(Token::Derived) => {
                self.advance();
                self.scan_qname();
                self.scan_multiplicity();
                self.take_name();
                // Tier 2: same body shapes as an `op` — target-tagged blocks
                // (`{ expr { ... } }`) or the bare `{ ... }` a driver rejects.
                self.scan_op_body();
            }
            Some(Token::Ident(_) | Token::IdentEscaped(_)) => {
                self.scan_qname();
                self.scan_multiplicity();
                self.take_name();
                self.scan_default();
                self.scan_constraint_block();
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
        // `create { ... }` / `convert { ... }` body blocks: a contextual
        // keyword directly followed by a brace.
        if matches!(
            self.peek_tok(),
            Some(Token::Ident(text) | Token::IdentEscaped(text))
                if text == "create" || text == "convert"
        ) && matches!(
            self.nodes.get(self.pos + 1),
            Some(Node::Token(Token::LBrace, _))
        ) {
            self.advance();
            self.scan_target_body_list();
            return;
        }
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

    // --- actors bodies -------------------------------------------------------

    fn scan_actors_item(&mut self) {
        match self.peek_tok() {
            Some(Token::Actor | Token::Agent) => {
                self.advance();
                self.take_name();
                if self.take_if(|token| matches!(token, Token::Extends)) {
                    self.take_name();
                }
            }
            Some(Token::Capability) => {
                self.advance();
                self.take_name();
                if self.take_if(|token| matches!(token, Token::On)) {
                    self.scan_qname();
                }
            }
            Some(Token::Purpose) => {
                self.advance();
                self.take_name();
            }
            Some(Token::Grant) => {
                self.advance();
                self.take_name();
                self.scan_body(BodyKind::Grant);
            }
            Some(Token::Delegation) => {
                self.advance();
                self.take_name();
                self.scan_body(BodyKind::Delegation);
            }
            Some(Token::NeverBoth) => self.scan_never_both(),
            _ => self.scan_junk_until(|token| {
                matches!(
                    token,
                    Token::RBrace
                        | Token::Actor
                        | Token::Agent
                        | Token::Capability
                        | Token::Purpose
                        | Token::Grant
                        | Token::Delegation
                        | Token::NeverBoth
                )
            }),
        }
    }

    /// One item of a `delegation` body: the required `from`/`to` lines (the
    /// `to` marker is a contextual identifier), the optional `purpose` line
    /// and grant-shaped effect entries. A `cedar { ... }` entry is a parse
    /// error, but the formatter never rejects: it renders exactly like a
    /// grant's cedar entry.
    fn scan_delegation_item(&mut self) {
        match self.peek_tok() {
            Some(Token::From) => {
                self.advance();
                self.take_name();
            }
            Some(Token::Ident("to")) => {
                self.advance();
                self.take_name();
            }
            Some(Token::Purpose) => {
                self.advance();
                self.take_name();
            }
            Some(Token::Permit | Token::Forbid | Token::Cedar) => self.scan_grant_entry(),
            _ => self.scan_junk_until(|token| {
                matches!(
                    token,
                    Token::RBrace
                        | Token::From
                        | Token::Purpose
                        | Token::Permit
                        | Token::Forbid
                        | Token::Cedar
                )
            }),
        }
    }

    fn scan_grant_entry(&mut self) {
        match self.peek_tok() {
            Some(Token::Permit | Token::Forbid) => {
                self.advance();
                self.take_name();
                if self.take_if(|token| matches!(token, Token::When)) {
                    self.scan_raw_parens();
                }
                while self.take_if(|token| matches!(token, Token::Obligation)) {
                    self.take_name();
                }
            }
            // `cedar { ... }` renders exactly like a `<target> { ... }` body.
            Some(Token::Cedar)
                if matches!(
                    self.nodes.get(self.pos + 1),
                    Some(Node::Token(Token::LBrace, _))
                ) =>
            {
                self.scan_target_block();
            }
            _ => self.scan_junk_until(|token| {
                matches!(
                    token,
                    Token::RBrace | Token::Permit | Token::Forbid | Token::Cedar
                )
            }),
        }
    }

    /// Consumes and emits a `never_both { A, B }` exclusivity constraint on
    /// the current line.
    fn scan_never_both(&mut self) {
        self.advance();
        if !self.take_if(|token| matches!(token, Token::LBrace)) {
            return;
        }
        loop {
            if !self.take_name() {
                break;
            }
            if !self.take_if(|token| matches!(token, Token::Comma)) {
                break;
            }
        }
        self.take_if(|token| matches!(token, Token::RBrace));
    }

    // --- .ddd bodies ---------------------------------------------------------

    fn scan_ddd_application_item(&mut self) {
        match self.peek_tok() {
            Some(Token::Ident(ddd::BASE)) => {
                self.advance();
                self.scan_qname();
            }
            Some(Token::Ident(ddd::MODULE)) => {
                self.advance();
                self.take_name();
                self.scan_body(BodyKind::DddModule);
            }
            _ => {
                // Junk always consumes the front token first (the stop set
                // contains identifiers, and the body loop cannot make
                // progress otherwise) — unless the front is the body's
                // closing `}` (or the stream ended), which the loop breaks
                // on; a drained trailing comment may have just revealed it.
                if !matches!(self.peek_tok(), Some(Token::RBrace) | None) {
                    self.advance();
                    self.scan_junk_until(|token| {
                        matches!(
                            token,
                            Token::RBrace | Token::Ident(_) | Token::IdentEscaped(_)
                        )
                    });
                }
            }
        }
    }

    fn scan_ddd_module_item(&mut self) {
        match self.peek_tok() {
            Some(Token::Ident(ddd::SERVICE)) => {
                self.advance();
                self.take_name();
                self.scan_body(BodyKind::DddService);
            }
            Some(Token::Ident(ddd::SEARCH)) => self.scan_ddd_search(),
            Some(Token::Ident(text)) if ddd::starts_design_decl(text) => self.scan_ddd_design(),
            _ => {
                // Junk always consumes the front token first (the stop set
                // contains identifiers, and the body loop cannot make
                // progress otherwise) — unless the front is the body's
                // closing `}` (or the stream ended), which the loop breaks
                // on; a drained trailing comment may have just revealed it.
                if !matches!(self.peek_tok(), Some(Token::RBrace) | None) {
                    self.advance();
                    self.scan_junk_until(|token| {
                        matches!(
                            token,
                            Token::RBrace | Token::Ident(_) | Token::IdentEscaped(_)
                        )
                    });
                }
            }
        }
    }

    /// One design declaration: `abstract`? stereotype, the class name, the
    /// five design flags — collected and re-emitted in the canonical order
    /// of [`ddd::DESIGN_FLAGS`] (`scaffold auditable optimisticLocking
    /// nonPersistent cache`) whatever the source order (mirroring the
    /// parser's `DddFlags`, whose fields the driver reads positionally) —
    /// and an optional `repository` block.
    fn scan_ddd_design(&mut self) {
        if matches!(self.peek_tok(), Some(Token::Ident(ddd::ABSTRACT))) {
            self.advance();
        }
        self.advance(); // the stereotype keyword
        self.take_name();
        let mut seen = [false; ddd::DESIGN_FLAGS.len()];
        loop {
            let index = match self.peek_tok() {
                Some(Token::Ident(text)) => ddd::DESIGN_FLAGS
                    .iter()
                    .position(|(keyword, _)| *keyword == text),
                _ => None,
            };
            match index {
                Some(index) => {
                    self.bump_token();
                    seen[index] = true;
                }
                None => break,
            }
        }
        for (index, (keyword, _)) in ddd::DESIGN_FLAGS.iter().enumerate() {
            if seen[index] {
                self.push_text(keyword, false);
            }
        }
        if matches!(self.peek_tok(), Some(Token::Ident(ddd::REPOSITORY))) {
            self.advance();
            self.take_name();
            self.scan_body(BodyKind::DddRepository);
        }
    }

    fn scan_ddd_service_item(&mut self) {
        match self.peek_tok() {
            Some(Token::Ident(ddd::INJECT)) => {
                self.advance();
                self.take_name();
                self.take_if(|token| matches!(token, Token::Other(';')));
            }
            Some(Token::Ident(_) | Token::IdentEscaped(_)) => {
                self.scan_qname();
                if self.take_if(|token| matches!(token, Token::FatArrow)) {
                    // Delegated: the `Target.operation` pair (one dotted
                    // reference).
                    self.scan_qname();
                } else {
                    // Declared: multiplicity, operation name, parameters.
                    self.scan_multiplicity();
                    self.take_name();
                    self.scan_ddd_params();
                }
                self.scan_ddd_capabilities();
                self.take_if(|token| matches!(token, Token::Other(';')));
            }
            _ => {
                // Junk always consumes the front token first (the stop set
                // contains identifiers, and the body loop cannot make
                // progress otherwise) — unless the front is the body's
                // closing `}` (or the stream ended), which the loop breaks
                // on; a drained trailing comment may have just revealed it.
                if !matches!(self.peek_tok(), Some(Token::RBrace) | None) {
                    self.advance();
                    self.scan_junk_until(|token| {
                        matches!(
                            token,
                            Token::RBrace | Token::Ident(_) | Token::IdentEscaped(_)
                        )
                    });
                }
            }
        }
    }

    /// Consumes and emits an optional `capability A, B` clause, kept on the
    /// operation's line.
    fn scan_ddd_capabilities(&mut self) {
        if !self.take_if(|token| matches!(token, Token::Capability)) {
            return;
        }
        self.take_name();
        while self.take_if(|token| matches!(token, Token::Comma)) {
            self.take_name();
        }
    }

    /// Like [`Formatter::scan_params`], but for `.ddd` parameters, which may
    /// carry a multiplicity between the type and the name.
    fn scan_ddd_params(&mut self) {
        if !self.take_if(|token| matches!(token, Token::LParen)) {
            return;
        }
        if self.take_if(|token| matches!(token, Token::RParen)) {
            return;
        }
        loop {
            self.scan_qname();
            self.scan_multiplicity();
            self.take_name();
            if !self.take_if(|token| matches!(token, Token::Comma)) {
                break;
            }
        }
        self.take_if(|token| matches!(token, Token::RParen));
    }

    fn scan_ddd_repository_item(&mut self) {
        match self.peek_tok() {
            Some(Token::Ident(text)) if ddd::builtin_from_keyword(text).is_some() => {
                self.advance();
                self.take_if(|token| matches!(token, Token::Other(';')));
            }
            Some(Token::Ident(_) | Token::IdentEscaped(_)) => {
                self.scan_qname();
                self.scan_multiplicity();
                self.take_name();
                self.scan_ddd_params();
                self.take_if(|token| matches!(token, Token::Other(';')));
            }
            _ => {
                // Junk always consumes the front token first (the stop set
                // contains identifiers, and the body loop cannot make
                // progress otherwise) — unless the front is the body's
                // closing `}` (or the stream ended), which the loop breaks
                // on; a drained trailing comment may have just revealed it.
                if !matches!(self.peek_tok(), Some(Token::RBrace) | None) {
                    self.advance();
                    self.scan_junk_until(|token| {
                        matches!(
                            token,
                            Token::RBrace | Token::Ident(_) | Token::IdentEscaped(_)
                        )
                    });
                }
            }
        }
    }

    /// Consumes and emits one `search <name> { ... }` projection. The body's
    /// members are rewritten into the canonical order (see
    /// [`Formatter::reorder_ddd_search_members`]) before the linear scan.
    fn scan_ddd_search(&mut self) {
        self.advance(); // the `search` keyword
        self.take_name();
        self.reorder_ddd_search_members();
        self.scan_body(BodyKind::DddSearch);
    }

    /// Rewrites the node stream of the search body ahead of the formatter
    /// (positioned at the body's `{`) so its members appear in the canonical
    /// order (the [`ddd::SEARCH_MEMBERS`] table order, via
    /// [`ddd::search_member_rank`]) whatever the source order — the same
    /// node-range technique the import hoists use. Segments split at
    /// brace-depth 0 on the member-leading keywords; segments that start
    /// with anything else (junk from a parse-error region) keep their
    /// relative order at the end, and a segment's leading comments travel
    /// with it.
    fn reorder_ddd_search_members(&mut self) {
        if !matches!(self.front(), Some(Node::Token(Token::LBrace, _))) {
            return;
        }
        let open = self.pos;
        let mut depth = 0usize;
        let close = loop {
            match self.nodes.get(self.pos) {
                Some(Node::Token(Token::LBrace, _)) => {
                    depth += 1;
                    self.pos += 1;
                }
                Some(Node::Token(Token::RBrace, _)) => {
                    depth = depth.saturating_sub(1);
                    self.pos += 1;
                    if depth == 0 {
                        break self.pos;
                    }
                }
                Some(_) => self.pos += 1,
                None => return,
            }
        };
        let mut segments: Vec<(usize, Vec<Node<'src>>)> = Vec::new();
        let mut depth = 0usize;
        for node in self.nodes[open + 1..close - 1].iter() {
            let rank = match node {
                Node::Token(token, _) if depth == 0 => ddd::search_member_rank(token),
                _ => None,
            };
            if segments.is_empty() {
                segments.push((rank.unwrap_or(usize::MAX), Vec::new()));
            } else if let Some(rank) = rank {
                segments.push((rank, Vec::new()));
            }
            match node {
                Node::Token(Token::LBrace, _) => depth += 1,
                Node::Token(Token::RBrace, _) => depth = depth.saturating_sub(1),
                _ => {}
            }
            segments.last_mut().unwrap().1.push(node.clone());
        }
        segments.sort_by_key(|(rank, _)| *rank);
        let mut reordered: Vec<Node<'src>> = self.nodes[..=open].to_vec();
        for (_, segment) in &segments {
            reordered.extend(segment.iter().cloned());
        }
        reordered.extend(self.nodes[close - 1..].iter().cloned());
        self.pos = open;
        self.nodes = reordered;
    }

    fn scan_ddd_search_item(&mut self) {
        match self
            .peek_tok()
            .and_then(|token| ddd::search_member_kind(&token))
        {
            Some(ddd::SearchMemberKind::Entity) => {
                self.advance();
                self.scan_qname();
            }
            Some(ddd::SearchMemberKind::Text) => {
                self.advance();
                self.scan_body(BodyKind::DddSearchField);
            }
            Some(ddd::SearchMemberKind::Filters | ddd::SearchMemberKind::Sort) => {
                self.advance();
                self.scan_body(BodyKind::DddSearchNames);
            }
            Some(ddd::SearchMemberKind::Document) => {
                self.advance();
                self.scan_body(BodyKind::DddSearchDocument);
            }
            Some(ddd::SearchMemberKind::Ranking) => {
                self.advance();
                if matches!(self.peek_tok(), Some(Token::Ident(_)) | Some(Token::Str(_))) {
                    self.advance();
                }
            }
            Some(ddd::SearchMemberKind::Analyzer) => {
                self.advance();
                self.take_if(|token| matches!(token, Token::Str(_)));
            }
            Some(ddd::SearchMemberKind::Pagination) => {
                self.advance();
                self.scan_body(BodyKind::DddSearchPagination);
            }
            Some(ddd::SearchMemberKind::Capability) => self.scan_ddd_capabilities(),
            _ => {
                // Junk always consumes the front token first (the stop set
                // contains identifiers, and the body loop cannot make
                // progress otherwise) — unless the front is the body's
                // closing `}` (or the stream ended), which the loop breaks
                // on; a drained trailing comment may have just revealed it.
                if !matches!(self.peek_tok(), Some(Token::RBrace) | None) {
                    self.advance();
                    self.scan_junk_until(|token| {
                        matches!(
                            token,
                            Token::RBrace | Token::Ident(_) | Token::IdentEscaped(_)
                        )
                    });
                }
            }
        }
    }

    /// One `text` field: `<property> (boost <int>)? (analyzer "<...>")?`,
    /// one field per line.
    fn scan_ddd_search_field_item(&mut self) {
        match self.peek_tok() {
            Some(Token::Ident(_) | Token::IdentEscaped(_)) => {
                self.scan_qname();
                if self.take_if(|token| matches!(token, Token::Ident(ddd::BOOST))) {
                    self.take_if(|token| matches!(token, Token::Int(_)));
                }
                if self.take_if(|token| matches!(token, Token::Ident(ddd::ANALYZER))) {
                    self.take_if(|token| matches!(token, Token::Str(_)));
                }
            }
            _ => {
                if !matches!(self.peek_tok(), Some(Token::RBrace) | None) {
                    self.advance();
                    self.scan_junk_until(|token| {
                        matches!(
                            token,
                            Token::RBrace | Token::Ident(_) | Token::IdentEscaped(_)
                        )
                    });
                }
            }
        }
    }

    /// One name of a `filters`/`sort` clause, one per line.
    fn scan_ddd_search_name_item(&mut self) {
        match self.peek_tok() {
            Some(Token::Ident(_) | Token::IdentEscaped(_)) => self.scan_qname(),
            _ => {
                if !matches!(self.peek_tok(), Some(Token::RBrace) | None) {
                    self.advance();
                    self.scan_junk_until(|token| {
                        matches!(
                            token,
                            Token::RBrace | Token::Ident(_) | Token::IdentEscaped(_)
                        )
                    });
                }
            }
        }
    }

    /// One `name = <expr>;` entry of a `document` clause: the expression is
    /// re-emitted verbatim token by token (canonical spacing), terminating
    /// at the depth-0 `;`. String literals are single tokens, so a `;`
    /// inside one never terminates; a missing terminator leaves the closing
    /// `}` for the body loop.
    fn scan_ddd_search_document_item(&mut self) {
        match self.peek_tok() {
            Some(Token::Ident(_) | Token::IdentEscaped(_)) => {
                self.take_name();
                self.take_if(|token| matches!(token, Token::Eq));
                let mut depth = 0usize;
                loop {
                    match self.peek_tok() {
                        None => break,
                        Some(Token::Other(';')) if depth == 0 => {
                            self.advance();
                            break;
                        }
                        Some(Token::RBrace) if depth == 0 => break,
                        Some(token) => {
                            match token {
                                Token::LParen | Token::LBracket | Token::LBrace => depth += 1,
                                Token::RParen | Token::RBracket | Token::RBrace => {
                                    depth = depth.saturating_sub(1)
                                }
                                _ => {}
                            }
                            self.advance();
                        }
                    }
                }
            }
            _ => {
                if !matches!(self.peek_tok(), Some(Token::RBrace) | None) {
                    self.advance();
                    self.scan_junk_until(|token| {
                        matches!(
                            token,
                            Token::RBrace | Token::Ident(_) | Token::IdentEscaped(_)
                        )
                    });
                }
            }
        }
    }

    /// One `limit <int>` / `max <int>` / `cursor` member of `pagination`.
    fn scan_ddd_search_pagination_item(&mut self) {
        match self.peek_tok() {
            Some(Token::Ident(text)) if text == ddd::LIMIT || text == ddd::MAX => {
                self.advance();
                self.take_if(|token| matches!(token, Token::Int(_)));
            }
            Some(Token::Ident(ddd::CURSOR)) => self.advance(),
            _ => {
                if !matches!(self.peek_tok(), Some(Token::RBrace) | None) {
                    self.advance();
                    self.scan_junk_until(|token| {
                        matches!(
                            token,
                            Token::RBrace | Token::Ident(_) | Token::IdentEscaped(_)
                        )
                    });
                }
            }
        }
    }

    // --- top-level declarations ----------------------------------------------

    /// Rewrites the node stream into canonical order: `package` (when it is
    /// the first item) stays at the front, `import schema`/`import sigil`
    /// declarations are hoisted directly after it (in source order), and
    /// every other top-level item keeps its relative order.
    ///
    /// Items are contiguous node ranges split at brace-depth 0 on the
    /// top-level declaration keywords (plus `Import`); the first node
    /// unconditionally begins item 0, so leading comments travel with it. An
    /// item counts as an import declaration only when its first three tokens
    /// are `import`, the contextual `schema`/`sigil` identifier and a string
    /// literal — lookalikes inside dotted references are left alone.
    fn hoist_import_schemas(&mut self) {
        /// The tokens that begin a top-level item (mirrors the parser's
        /// declaration set; `Import` is included so junk recovery and the
        /// import section agree on item boundaries).
        fn starts_item(token: &Token<'_>) -> bool {
            matches!(
                token,
                Token::Package
                    | Token::Annotation
                    | Token::Class
                    | Token::Interface
                    | Token::Enum
                    | Token::Type
                    | Token::Vocabulary
                    | Token::Actors
                    | Token::Import
            )
        }

        /// Whether the item starting at `start` is an `import schema "…"` or
        /// `import sigil "…"` declaration: the import keyword, the
        /// contextual `schema`/`sigil` identifier, then a string literal
        /// (comments skipped).
        fn is_import_schema(nodes: &[Node<'_>], start: usize) -> bool {
            let mut tokens = nodes[start..].iter().filter_map(|node| match node {
                Node::Token(token, _) => Some(token),
                _ => None,
            });
            matches!(
                (tokens.next(), tokens.next(), tokens.next()),
                (
                    Some(Token::Import),
                    Some(Token::Ident("schema") | Token::Ident("sigil")),
                    Some(Token::Str(_))
                )
            )
        }

        let mut items: Vec<(usize, usize)> = Vec::new();
        let mut depth = 0usize;
        for (index, node) in self.nodes.iter().enumerate() {
            let token = match node {
                Node::Token(token, _) => Some(token),
                Node::Comment { .. } => None,
            };
            if items.is_empty() || (depth == 0 && token.is_some_and(starts_item)) {
                items.push((index, index));
            }
            let last = items.len() - 1;
            items[last].1 = index + 1;
            match token {
                Some(Token::LBrace) => depth += 1,
                Some(Token::RBrace) => depth = depth.saturating_sub(1),
                _ => {}
            }
        }
        if items.len() < 2 {
            return;
        }
        // The package anchors the import section only when it is genuinely
        // the first item; a keyword lookalike elsewhere is never moved.
        let anchor = if matches!(self.nodes.first(), Some(Node::Token(Token::Package, _))) {
            1
        } else {
            0
        };
        let imports: Vec<(usize, usize)> = items
            .iter()
            .enumerate()
            .skip(anchor)
            .filter_map(|(_position, (start, end))| {
                is_import_schema(&self.nodes, *start).then_some((*start, *end))
            })
            .collect();
        if imports.is_empty() {
            return;
        }
        let is_moved = |index: usize, imports: &[(usize, usize)]| {
            imports
                .iter()
                .any(|(start, end)| index >= *start && index < *end)
        };
        let mut reordered: Vec<Node<'src>> = Vec::with_capacity(self.nodes.len());
        for (position, range) in items.iter().enumerate() {
            if position == anchor {
                // The canonical import section sits directly before the
                // first non-package item.
                for (start, end) in &imports {
                    reordered.extend(self.nodes[*start..*end].iter().cloned());
                }
            }
            if is_moved(range.0, &imports) {
                continue;
            }
            reordered.extend(self.nodes[range.0..range.1].iter().cloned());
        }
        self.nodes = reordered;
    }

    /// Rewrites the node stream into canonical order for `.ddd` files:
    /// `import` declarations first (source order), then the `application`
    /// block and any junk items in source order.
    ///
    /// Items are contiguous node ranges split at brace-depth 0 on `Import`
    /// tokens and the contextual `application` identifier; the first node
    /// unconditionally begins item 0, so leading comments travel with their
    /// item.
    fn hoist_ddd_imports(&mut self) {
        let mut items: Vec<(usize, usize)> = Vec::new();
        let mut depth = 0usize;
        for (index, node) in self.nodes.iter().enumerate() {
            let token = match node {
                Node::Token(token, _) => Some(token),
                Node::Comment { .. } => None,
            };
            let starts_item = matches!(
                token,
                Some(Token::Import) | Some(Token::Ident("application"))
            );
            if items.is_empty() || (depth == 0 && starts_item) {
                items.push((index, index));
            }
            let last = items.len() - 1;
            items[last].1 = index + 1;
            match token {
                Some(Token::LBrace) => depth += 1,
                Some(Token::RBrace) => depth = depth.saturating_sub(1),
                _ => {}
            }
        }
        if items.len() < 2 {
            return;
        }
        // A leading comment block (a file header) stays at the very front:
        // the import section is inserted after it.
        let anchor = if matches!(self.nodes.first(), Some(Node::Comment { .. })) {
            1
        } else {
            0
        };
        let imports: Vec<(usize, usize)> = items
            .iter()
            .copied()
            .filter(|(start, _)| matches!(self.nodes[*start], Node::Token(Token::Import, _)))
            .collect();
        if imports.is_empty() {
            return;
        }
        let is_moved = |index: usize, imports: &[(usize, usize)]| {
            imports
                .iter()
                .any(|(start, end)| index >= *start && index < *end)
        };
        let mut reordered: Vec<Node<'src>> = Vec::with_capacity(self.nodes.len());
        for range in &items[..anchor] {
            reordered.extend(self.nodes[range.0..range.1].iter().cloned());
        }
        for range in &imports {
            reordered.extend(self.nodes[range.0..range.1].iter().cloned());
        }
        for range in &items[anchor..] {
            if is_moved(range.0, &imports) {
                continue;
            }
            reordered.extend(self.nodes[range.0..range.1].iter().cloned());
        }
        self.nodes = reordered;
    }

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

    fn scan_actors(&mut self) {
        self.begin_top_decl();
        self.advance();
        self.take_name();
        self.scan_body(BodyKind::Actors);
    }

    /// Consumes and emits one `import "path"` declaration. Imports form a
    /// tight section: consecutive lines, no blank lines between them (so
    /// unlike the declaration scanners this deliberately skips
    /// [`Formatter::begin_top_decl`]); the exactly-one blank line before a
    /// following actors block comes from that block's `begin_top_decl`.
    fn scan_import(&mut self) {
        self.flush_line();
        self.flush_pending(0);
        self.advance();
        self.take_if(|token| matches!(token, Token::Str(_)));
        self.flush_line();
    }

    /// Consumes and emits one `import "<path>";` declaration of a `.ddd`
    /// file: the tight import section of the `.actor` formatter, plus the
    /// canonical trailing `;` (emitted whether or not the source had one; a
    /// source `;` is consumed silently so it never doubles up).
    fn scan_ddd_import(&mut self) {
        self.flush_line();
        self.flush_pending(0);
        self.advance();
        self.take_if(|token| matches!(token, Token::Str(_)));
        if matches!(self.front(), Some(Node::Token(Token::Other(';'), _))) {
            self.bump_token();
        }
        self.push_text(";", true);
        self.flush_line();
    }

    /// Consumes and emits one `import schema "<path>" (as <name>)?` or
    /// `import sigil "<path>"` declaration of a `.mox` file. The section
    /// mirrors the `.actor` import rule: one declaration per line, tight —
    /// the first declaration gets the canonical blank line above it (after
    /// the package), the rest follow with no blank lines in between; the
    /// blank line before the next non-import declaration comes from that
    /// declaration's `begin_top_decl`.
    fn scan_import_schema(&mut self) {
        if self.import_emitted {
            self.flush_line();
            self.flush_pending(0);
        } else {
            self.import_emitted = true;
            self.begin_top_decl();
        }
        self.advance(); // `import`
        if matches!(
            self.peek_tok(),
            Some(Token::Ident(text)) if text == "schema" || text == "sigil"
        ) {
            self.advance(); // the contextual `schema`/`sigil` word
        }
        self.take_if(|token| matches!(token, Token::Str(_)));
        if self.take_if(|token| matches!(token, Token::As)) {
            self.take_name();
        }
        self.flush_line();
    }

    /// Unrecognized top-level tokens of an `.actor` file: emit them on one
    /// line, stopping at the next `import`/`actors` keyword (mirrors the
    /// parser's declaration-level recovery for actor files).
    fn scan_top_junk_actors(&mut self) {
        self.begin_top_decl();
        loop {
            match self.peek_tok() {
                Some(token) if !matches!(token, Token::Import | Token::Actors) => self.advance(),
                _ => break,
            }
        }
        self.flush_line();
    }

    /// Consumes and emits the `application <name> { ... }` block of a
    /// `.ddd` file.
    fn scan_ddd_application(&mut self) {
        self.begin_top_decl();
        self.advance();
        self.take_name();
        self.scan_body(BodyKind::DddApplication);
    }

    /// Unrecognized top-level tokens of a `.ddd` file: emit them on one
    /// line, stopping at the next `import` keyword or the contextual
    /// `application` keyword (mirrors the parser's file-level recovery).
    fn scan_top_junk_ddd(&mut self) {
        self.begin_top_decl();
        loop {
            match self.peek_tok() {
                Some(token) if !matches!(token, Token::Import | Token::Ident("application")) => {
                    self.advance()
                }
                _ => break,
            }
        }
        self.flush_line();
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
            | Token::Actors
            | Token::Import
    )
}

/// The closed set of constraint keywords allowed inside an attribute's
/// constraint block (mirrors the parser's set). `unique` is the one
/// value-less keyword; the scanner already consumes its literal value only
/// when present.
fn is_constraint_keyword(text: &str) -> bool {
    matches!(
        text,
        "pattern" | "minLength" | "maxLength" | "minimum" | "maximum" | "unique"
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
        Token::FatArrow => "=>".to_string(),
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
