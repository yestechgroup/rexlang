//! The `.mox` formatter: top-level dispatch, class/interface/enum/datatype/vocabulary scanners, `import schema` hoisting, and the plain top-level junk scanner.

use super::*;

impl<'src> Formatter<'src> {
    pub(crate) fn run(&mut self) -> String {
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

    pub(crate) fn scan_class_item(&mut self) {
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

    pub(crate) fn scan_enum_literal(&mut self) {
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

    pub(crate) fn scan_binding(&mut self) {
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

    pub(crate) fn scan_vocabulary_item(&mut self) {
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
    pub(crate) fn hoist_import_schemas(&mut self) {
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

    pub(crate) fn scan_package(&mut self) {
        self.begin_top_decl();
        self.advance();
        self.scan_qname();
        self.flush_line();
    }

    pub(crate) fn scan_annotation(&mut self) {
        self.begin_top_decl();
        self.advance();
        self.take_if(|token| matches!(token, Token::Str(_)));
        if self.take_if(|token| matches!(token, Token::As)) {
            self.take_name();
        }
        self.flush_line();
    }

    pub(crate) fn scan_class(&mut self) {
        self.begin_top_decl();
        self.advance();
        self.scan_qname();
        // The contextual `table` annotation: `table "…"` directly after the
        // class name (an escaped `^table` is a name, never the annotation).
        if self.take_if(|token| matches!(token, Token::Ident(text) if *text == "table")) {
            self.take_if(|token| matches!(token, Token::Str(_)));
        }
        if self.take_if(|token| matches!(token, Token::Extends)) {
            self.scan_qname();
            while self.take_if(|token| matches!(token, Token::Comma)) {
                self.scan_qname();
            }
        }
        self.scan_body(BodyKind::Class);
    }

    pub(crate) fn scan_interface(&mut self) {
        self.begin_top_decl();
        self.advance();
        self.scan_qname();
        self.scan_body(BodyKind::Bindings);
    }

    pub(crate) fn scan_enum(&mut self) {
        self.begin_top_decl();
        self.advance();
        self.scan_qname();
        self.scan_body(BodyKind::Enum);
    }

    pub(crate) fn scan_datatype(&mut self) {
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

    pub(crate) fn scan_vocabulary(&mut self) {
        self.begin_top_decl();
        self.advance();
        self.scan_qname();
        self.take_if(|token| matches!(token, Token::From));
        self.take_if(|token| matches!(token, Token::Str(_)));
        self.scan_body(BodyKind::Vocabulary);
    }

    /// Consumes and emits one `import "path"` declaration. Imports form a
    /// tight section: consecutive lines, no blank lines between them (so
    /// unlike the declaration scanners this deliberately skips
    /// [`Formatter::begin_top_decl`]); the exactly-one blank line before a
    /// following actors block comes from that block's `begin_top_decl`.
    pub(crate) fn scan_import(&mut self) {
        self.flush_line();
        self.flush_pending(0);
        self.advance();
        self.take_if(|token| matches!(token, Token::Str(_)));
        self.flush_line();
    }

    /// Consumes and emits one `import schema "<path>" (as <name>)?` or
    /// `import sigil "<path>"` declaration of a `.mox` file. The section
    /// mirrors the `.actor` import rule: one declaration per line, tight —
    /// the first declaration gets the canonical blank line above it (after
    /// the package), the rest follow with no blank lines in between; the
    /// blank line before the next non-import declaration comes from that
    /// declaration's `begin_top_decl`.
    pub(crate) fn scan_import_schema(&mut self) {
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

    /// Unrecognized top-level tokens: emit them on one line, stopping at the
    /// next declaration keyword (mirrors the parser's declaration-level
    /// recovery).
    pub(crate) fn scan_top_junk(&mut self) {
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
