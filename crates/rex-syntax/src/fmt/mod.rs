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
//!   A design may carry a `machine` block after its optional `repository`
//!   block: the `initial`/`terminal` lines and the transitions keep their
//!   source order, a transition renders `<From> -> <To>` plus its optional
//!   `via <Trigger>` and `when (...)` clauses on one line, and a guard's
//!   parenthesized condition is emitted verbatim (ends trimmed, the
//!   `when`-condition rule). A service operation's optional `when (...)`
//!   guard follows the `capability` clause on the operation's line.
//! * An `.evt` source (see [`format_evt`]) canonicalizes the whole file into
//!   section order: `import` declarations first (tight, one per line), then
//!   `event`, `channel`, and `subscription` declarations — each kind grouped
//!   in that order regardless of source order (source order is kept within a
//!   kind). Event bodies are reordered into fields → `owner` →
//!   `classification` → `retention` → `pii` → `cloudEvents`; every
//!   statement line gets its canonical `;` (`owner order_team;`,
//!   `pii [ A B ];` — the `.ddd` import rule). The `cloudEvents` block
//!   renders one bracket-only entry per line (`type "..."`, `source "..."`,
//!   optional `subject "..."` — reordered into that order); a `delivery`
//!   block renders one `word value;` entry per line, reordered into `mode`,
//!   `maxAttempts`, `backoffSeconds`, `timeoutSeconds`, `deadLetter`. A
//!   subscription's `when (...)` filter is emitted on its own line with the
//!   condition's inner bytes verbatim (ends trimmed, never wrapped — the
//!   `when`-condition rule); empty `{}` blocks stay inline. `publishes`
//!   entries render with a canonical trailing `;` (emitted whether or not
//!   the source had one, like the `.ddd` import rule).
//! * A `.deploy` source (see [`format_deploy`]) canonicalizes the whole file
//!   into section order like the `.evt` formatter: `import` declarations
//!   first (tight, one per line), then `application`, `profile`, and
//!   `deployment` declarations — each kind grouped in that order regardless
//!   of source order (source order is kept within a kind). Component bodies
//!   render `name: value` settings, `requires a, b` clauses, and
//!   `design`/`flow` bindings one per line; `connects a -> b` entries stay
//!   on one line. A profile renders `target: name`, then its `defaults {}`
//!   and `mappings {}` blocks (dotted settings one per line; mappings
//!   `kind -> platform.resource`) and its `require (...)`/`prohibit (...)`
//!   policies — a policy's parenthesized condition is emitted verbatim
//!   (ends trimmed), the `when`-condition rule. A deployment renders its
//!   `use` line and optional `configure {}` block. Component-kind and
//!   target names are echoed as written (their vocabularies are driver
//!   concerns, not syntax).
//! * A `.uc` source (see [`format_usecases`]) canonicalizes the whole file
//!   into section order like the `.evt` formatter: `import` declarations
//!   first (tight, one per line, aliases kept), then `usecase` declarations
//!   with exactly one blank line between them. Inside a usecase body the
//!   members are regrouped: the `subject` line first, then the `actor`
//!   lines, then messages, successions, and realizes bindings — source
//!   order kept within each group (a member's leading comments travel with
//!   it). Participant bodies keep their occurrence/perform source order.
//!   Every statement line (`occurrence`, `perform`, `message`, `first ...`,
//!   `realizes`) gets its canonical `;` whether or not the source had one;
//!   a `requires [...]` list renders space-separated inside spaced
//!   brackets. A message's `from`/`to` clauses stay on its line while the
//!   line fits within 100 columns; past the budget the clause breaks onto a
//!   continuation line indented one level further (checked per clause).
//! * A `.cmmn` source (see [`format_cmmn`]) canonicalizes the whole file
//!   into section order like the `.evt` formatter: `import` declarations
//!   first (tight, one per line, aliases kept), then `case` declarations
//!   with exactly one blank line between them. Inside a case body the
//!   members keep their source order (roles block, case-file block, plan
//!   items), as do plan-item bodies (the `performer`/`execute` clauses,
//!   criteria, flags, and — for stages — nested plan items). Every
//!   statement line (`role`, case-file item, `performer`, `execute`,
//!   criterion, flag) gets its canonical `;` whether or not the source had
//!   one; on-parts are comma-space separated, `.` chains stay tight, and a
//!   criterion's `when (...)` parens re-emit verbatim (the
//!   `when`-condition rule).
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
use crate::lexer::{lex_with_comments, CommentKind, LexError, Token};
use crate::usecases;

mod actor;
mod cmmn;
mod ddd;
mod deploy;
mod evt;
mod mox;
mod uc;

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

/// Format an `.evt` source text, preserving comments verbatim.
///
/// The canonical layout mirrors [`format_ddd`]: `import` declarations are
/// hoisted into a tight first section (one per line, in source order), then
/// the `event`, `channel`, and `subscription` declarations follow — grouped
/// by kind in that order whatever the source order (source order is kept
/// within a kind), with exactly one blank line between declarations.
/// Returns the formatted text ending in exactly one `\n` (empty input
/// formats to empty output), or [`FormatError::Lex`] if the source cannot be
/// tokenized.
pub fn format_evt(source: &str) -> Result<String, FormatError> {
    let (tokens, comments) = lex_with_comments(source)?;
    let mut fmt = Formatter::new(source, tokens, comments);
    Ok(fmt.run_evt())
}

/// Format a `.deploy` source text, preserving comments verbatim.
///
/// The canonical layout mirrors [`format_evt`]: `import` declarations are
/// hoisted into a tight first section (one per line, in source order), then
/// the `application`, `profile`, and `deployment` declarations follow —
/// grouped by kind in that order whatever the source order (source order is
/// kept within a kind), with exactly one blank line between declarations.
/// Returns the formatted text ending in exactly one `\n` (empty input
/// formats to empty output), or [`FormatError::Lex`] if the source cannot be
/// tokenized.
pub fn format_deploy(source: &str) -> Result<String, FormatError> {
    let (tokens, comments) = lex_with_comments(source)?;
    let mut fmt = Formatter::new(source, tokens, comments);
    Ok(fmt.run_deploy())
}

/// Format a `.uc` source text, preserving comments verbatim.
///
/// The canonical layout mirrors [`format_evt`]: `import` declarations are
/// hoisted into a tight first section (one per line, in source order, with
/// aliases kept), then the `usecase` declarations follow with exactly one
/// blank line between them. Inside a usecase body the members are regrouped
/// subject → actors → messages/successions/realizes (source order kept
/// within each group), statement lines get their canonical `;`, and a
/// message's `from`/`to` clauses wrap onto continuation lines past the
/// 100-column budget. Returns the formatted text ending in exactly one `\n`
/// (empty input formats to empty output), or [`FormatError::Lex`] if the
/// source cannot be tokenized.
pub fn format_usecases(source: &str) -> Result<String, FormatError> {
    let (tokens, comments) = lex_with_comments(source)?;
    let mut fmt = Formatter::new(source, tokens, comments);
    Ok(fmt.run_usecases())
}

/// Format a `.cmmn` source text, preserving comments verbatim.
///
/// The canonical layout mirrors [`format_usecases`]: `import` declarations
/// are hoisted into a tight first section (one per line, source order,
/// aliases kept), followed by the `case` declarations with exactly one
/// blank line between them. Bodies keep their source order; every
/// statement line gets its canonical `;` and a criterion's `when (...)`
/// condition re-emits verbatim. Returns the formatted text ending in
/// exactly one `\n` (empty input formats to empty output), or
/// [`FormatError::Lex`] if the source cannot be tokenized.
pub fn format_cmmn(source: &str) -> Result<String, FormatError> {
    let (tokens, comments) = lex_with_comments(source)?;
    let mut fmt = Formatter::new(source, tokens, comments);
    Ok(fmt.run_cmmn())
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
    /// `initial`/`terminal` lines and transitions of a `.ddd` machine.
    DddMachine,
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
    /// typed payload fields of an `.evt` event declaration.
    EvtEvent,
    /// `publishes` entries of an `.evt` channel declaration.
    EvtChannel,
    /// the `events [...]`/`consumer` lines of an `.evt` subscription.
    EvtSubscription,
    /// `component` blocks and `connects` lines of a `.deploy` application.
    DeployApplication,
    /// `requires`/`design`/`flow`/`env`/setting members of a `.deploy`
    /// component.
    DeployComponent,
    /// `NAME = "value"` entries of a `.deploy` component's `env` block.
    DeployEnv,
    /// `target`/`defaults`/`mappings`/policy members of a `.deploy` profile.
    DeployProfile,
    /// dotted `path: value` settings of a `.deploy` `defaults`/`configure`
    /// block.
    DeploySettings,
    /// `kind -> resource` entries of a `.deploy` `mappings` block.
    DeployMappings,
    /// the `use`/`configure` members of a `.deploy` deployment.
    DeployDeployment,
    /// members of a `.uc` usecase body.
    UcUseCase,
    /// `occurrence`/`perform` members of a `.uc` participant body.
    UcParticipant,
    /// `role` declarations of a `.cmmn` roles block.
    CmmnRoles,
    /// typed items of a `.cmmn` case-file block.
    CmmnCaseFile,
    /// members of a `.cmmn` case body: the roles block, the case-file
    /// block, and plan items.
    CmmnCase,
    /// members of a `.cmmn` plan item body: clauses, criteria, flags, and
    /// (stages) nested plan items.
    CmmnPlan,
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

    /// The token `offset` positions ahead of the front token (comments
    /// skipped), for the two-token lookahead the negated design flags need
    /// (`! keyword` — the `!` may only be consumed once the keyword
    /// confirms). `None` when fewer tokens follow.
    fn peek_tok_nth(&mut self, offset: usize) -> Option<Token<'src>> {
        self.drain_comments();
        let mut index = self.pos;
        let mut seen = 0;
        while let Some(node) = self.nodes.get(index) {
            match node {
                Node::Token(token, _) => {
                    if seen == offset {
                        return Some(token.clone());
                    }
                    seen += 1;
                }
                Node::Comment { .. } => {}
            }
            index += 1;
        }
        None
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
                BodyKind::DddMachine => self.scan_ddd_machine_item(),
                BodyKind::DddSearch => self.scan_ddd_search_item(),
                BodyKind::DddSearchField => self.scan_ddd_search_field_item(),
                BodyKind::DddSearchNames => self.scan_ddd_search_name_item(),
                BodyKind::DddSearchDocument => self.scan_ddd_search_document_item(),
                BodyKind::DddSearchPagination => self.scan_ddd_search_pagination_item(),
                BodyKind::EvtEvent => self.scan_evt_event_item(),
                BodyKind::EvtChannel => self.scan_evt_channel_item(),
                BodyKind::EvtSubscription => self.scan_evt_subscription_item(),
                BodyKind::DeployApplication => self.scan_deploy_application_item(),
                BodyKind::DeployComponent => self.scan_deploy_component_item(),
                BodyKind::DeployEnv => self.scan_deploy_env_item(),
                BodyKind::DeployProfile => self.scan_deploy_profile_item(),
                BodyKind::DeploySettings => self.scan_deploy_setting_item(),
                BodyKind::DeployMappings => self.scan_deploy_mapping_item(),
                BodyKind::DeployDeployment => self.scan_deploy_deployment_item(),
                BodyKind::UcUseCase => self.scan_uc_item(),
                BodyKind::UcParticipant => self.scan_uc_participant_item(),
                BodyKind::CmmnRoles => self.scan_cmmn_roles_item(),
                BodyKind::CmmnCaseFile => self.scan_cmmn_case_file_item(),
                BodyKind::CmmnCase => self.scan_cmmn_case_item(),
                BodyKind::CmmnPlan => self.scan_cmmn_plan_item(),
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
        Token::Arrow => "->".to_string(),
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

/// The canonical column budget of a `.uc` message line: a message's
/// `from`/`to` clauses stay on the line while it fits, and wrap onto a
/// continuation line (one extra indent level, checked per clause) past it.
const UC_MESSAGE_LINE_BUDGET: usize = 100;

/// Whether the token starts a `.uc` usecase member: the `subject`/`message`/
/// `first`/`realizes` contextual words, or the `actor` keyword.
fn is_uc_member_start(token: &Token<'_>) -> bool {
    match token {
        Token::Actor => true,
        Token::Ident(text) => {
            *text == usecases::SUBJECT
                || *text == usecases::MESSAGE
                || *text == usecases::FIRST
                || *text == usecases::REALIZES
        }
        _ => false,
    }
}

/// The canonical rank of a `.uc` usecase member: the `subject` line first,
/// then the `actor` lines, then every other member (junk last). The
/// formatter's stable sort emits members in rank order whatever the source
/// order, source order kept within a rank.
fn uc_member_rank(token: &Token<'_>) -> usize {
    match token {
        Token::Ident(text) if *text == usecases::SUBJECT => 0,
        Token::Actor => 1,
        _ => 2,
    }
}
