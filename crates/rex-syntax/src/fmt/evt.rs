//! The `.evt` formatter: event/channel/subscription scanners and the declaration-kind hoist.

use super::*;

impl<'src> Formatter<'src> {
    /// Like [`Formatter::run_ddd`], but for `.evt` files: the only
    /// recognized top-level constructs are `import` declarations and
    /// `event`/`channel`/`subscription` declarations, canonicalized into
    /// imports → events → channels → subscriptions order by
    /// [`Formatter::hoist_evt_items`].
    pub(crate) fn run_evt(&mut self) -> String {
        self.hoist_evt_items();
        loop {
            let front = self.front().cloned();
            match front {
                None => break,
                Some(Node::Comment { .. }) => self.advance(),
                Some(Node::Token(token, _)) => match token {
                    Token::Import => self.scan_import(),
                    Token::Event => self.scan_evt_event(),
                    Token::Channel => self.scan_evt_channel(),
                    Token::Subscription => self.scan_evt_subscription(),
                    _ => self.scan_top_junk_evt(),
                },
            }
        }
        self.finish()
    }

    /// One `name: Type;` payload field of an event body. The `:` and the
    /// trailing `;` are emitted canonically whether or not the source had
    /// them (the `.ddd` import-`;` rule).
    pub(crate) fn scan_evt_event_item(&mut self) {
        match self.peek_tok() {
            Some(Token::Ident(_) | Token::IdentEscaped(_)) => {
                self.take_name();
                if matches!(self.peek_tok(), Some(Token::Other(':'))) {
                    self.bump_token();
                }
                self.push_text(":", true);
                self.scan_qname();
                if matches!(self.peek_tok(), Some(Token::Other(';'))) {
                    self.bump_token();
                }
                self.push_text(";", true);
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

    /// One `publishes <event>;` entry of a channel body, with the canonical
    /// trailing `;` (the `.ddd` import-`;` rule).
    pub(crate) fn scan_evt_channel_item(&mut self) {
        match self.peek_tok() {
            Some(Token::Publishes) => {
                self.advance();
                self.take_name();
                if matches!(self.peek_tok(), Some(Token::Other(';'))) {
                    self.bump_token();
                }
                self.push_text(";", true);
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

    /// The `events [ <event>* ]` clause of a subscription body, rendered on
    /// one line with spaced brackets (`events [ Ping Pong ]`). A
    /// `consumer` word inside the brackets ends the list (the
    /// contextual-word reading the parser commits to), so a missing `]`
    /// never swallows the consumer line.
    pub(crate) fn scan_evt_events_clause(&mut self) {
        self.advance(); // the contextual `events` word
        if matches!(self.peek_tok(), Some(Token::LBracket)) {
            self.bump_token();
        }
        self.push_text("[", false);
        loop {
            match self.peek_tok() {
                Some(Token::Ident("consumer")) => break,
                Some(Token::Ident(_) | Token::IdentEscaped(_)) => {
                    self.take_name();
                }
                _ => break,
            }
        }
        if matches!(self.peek_tok(), Some(Token::RBracket)) {
            self.bump_token();
        }
        self.push_text("]", false);
    }

    /// One item of a subscription body: the `events [...]` list or the
    /// `consumer <name> ("scope" <name>)?` line. The optional scope binding
    /// rides on the consumer line.
    pub(crate) fn scan_evt_subscription_item(&mut self) {
        match self.peek_tok() {
            Some(Token::Ident("events")) => self.scan_evt_events_clause(),
            Some(Token::Ident("consumer")) => {
                self.advance();
                self.take_name();
                if self.take_if(|token| matches!(token, Token::Ident(text) if *text == "scope")) {
                    self.take_name();
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

    // --- .deploy bodies ------------------------------------------------------

    /// Rewrites the node stream into canonical `.evt` order: a leading
    /// comment run (the file header) stays at the very front, then `import`
    /// declarations, then `event`, `channel`, and `subscription`
    /// declarations — each kind grouped in that order, source order kept
    /// within a kind.
    ///
    /// Items are contiguous node ranges split at brace-depth 0 on the four
    /// top-level keywords; a depth-0 own-line comment run whose next token
    /// is a declaration begins that declaration's item, so its doc block
    /// travels with it. Junk items (parse-error regions) rank last and keep
    /// their relative order (the `.ddd` search-member reorder precedent).
    pub(crate) fn hoist_evt_items(&mut self) {
        fn rank(token: &Token<'_>) -> usize {
            match token {
                Token::Import => 0,
                Token::Event => 1,
                Token::Channel => 2,
                Token::Subscription => 3,
                _ => usize::MAX,
            }
        }

        // The header is the comment run before the first token.
        let header_end = self
            .nodes
            .iter()
            .position(|node| matches!(node, Node::Token(..)))
            .unwrap_or(self.nodes.len());

        let mut items: Vec<(usize, usize, usize)> = Vec::new(); // (rank, start, end)
        let mut depth = 0usize;
        for (index, node) in self.nodes.iter().enumerate().skip(header_end) {
            let (starts_item, rank) = match node {
                Node::Token(token, _) => (depth == 0 && rank(token) != usize::MAX, rank(token)),
                Node::Comment { own_line, .. } => {
                    // A depth-0 own-line comment whose next token is a
                    // top-level declaration begins that declaration's item.
                    let leads_decl = *own_line
                        && depth == 0
                        && self.nodes[index..].iter().find_map(|node| match node {
                            Node::Token(token, _) => Some(rank(token) != usize::MAX),
                            Node::Comment { .. } => None,
                        }) == Some(true);
                    let rank = if leads_decl {
                        self.nodes[index..]
                            .iter()
                            .find_map(|node| match node {
                                Node::Token(token, _) => Some(rank(token)),
                                _ => None,
                            })
                            .unwrap_or(usize::MAX)
                    } else {
                        usize::MAX
                    };
                    (leads_decl, rank)
                }
            };
            if starts_item || items.is_empty() {
                items.push((rank, index, index));
            }
            items.last_mut().unwrap().2 = index + 1;
            if let Node::Token(token, _) = node {
                match token {
                    Token::LBrace => depth += 1,
                    Token::RBrace => depth = depth.saturating_sub(1),
                    _ => {}
                }
            }
        }
        if items.len() < 2 {
            return;
        }
        // Stable: source order survives within a rank (junk included).
        items.sort_by_key(|(rank, _, _)| *rank);
        let mut reordered: Vec<Node<'src>> = self.nodes[..header_end].to_vec();
        for (_, start, end) in &items {
            reordered.extend(self.nodes[*start..*end].iter().cloned());
        }
        self.nodes = reordered;
    }

    /// Consumes and emits one `event <name> ("version" string)? { ... }`
    /// declaration of an `.evt` file.
    pub(crate) fn scan_evt_event(&mut self) {
        self.begin_top_decl();
        self.advance();
        self.take_name();
        if self.take_if(|token| matches!(token, Token::Version)) {
            self.take_if(|token| matches!(token, Token::Str(_)));
        }
        self.scan_body(BodyKind::EvtEvent);
    }

    /// Consumes and emits one `channel <name> ("of" <scope>)? { ... }`
    /// declaration of an `.evt` file. The `of` clause rides on the header
    /// line (the `.deploy` `deployment ... for <app>` precedent).
    pub(crate) fn scan_evt_channel(&mut self) {
        self.begin_top_decl();
        self.advance();
        self.take_name();
        if self.take_if(|token| matches!(token, Token::Ident(text) if *text == "of")) {
            self.take_name();
        }
        self.scan_body(BodyKind::EvtChannel);
    }

    /// Consumes and emits one `subscription <name> { ... }` declaration of
    /// an `.evt` file.
    pub(crate) fn scan_evt_subscription(&mut self) {
        self.begin_top_decl();
        self.advance();
        self.take_name();
        self.scan_body(BodyKind::EvtSubscription);
    }

    /// Unrecognized top-level tokens of an `.evt` file: emit them on one
    /// line, stopping at the next `import`/`event`/`channel`/`subscription`
    /// keyword (mirrors the parser's file-level recovery).
    pub(crate) fn scan_top_junk_evt(&mut self) {
        self.begin_top_decl();
        loop {
            match self.peek_tok() {
                Some(token)
                    if !matches!(
                        token,
                        Token::Import | Token::Event | Token::Channel | Token::Subscription
                    ) =>
                {
                    self.advance()
                }
                _ => break,
            }
        }
        self.flush_line();
    }
}
