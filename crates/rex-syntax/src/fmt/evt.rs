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

    /// Stable-sorts the node range `[start, end)` into `rank` order,
    /// splitting items at brace-depth-0 tokens where `starts_item` is true
    /// — or at an own-line comment that leads such a token, so its doc
    /// block travels with it. With `separator_required`, a token may start
    /// an item only right after `{`/`}`/`;` (or the range start), so
    /// qualified-name segments and clause values never split; without it,
    /// any `starts_item` token at depth 0 begins an item (for block entries
    /// whose grammar has no separators, like `cloudEvents`). Items keep
    /// source order within a rank; junk keeps its relative order (the
    /// `.ddd` search-member reorder precedent). The nodes before `start`
    /// and from `end` on are untouched.
    fn sort_node_range(
        &mut self,
        start: usize,
        end: usize,
        separator_required: bool,
        starts_item: impl Fn(&Token<'src>) -> bool,
        rank: impl Fn(&Token<'src>) -> usize,
    ) {
        let mut items: Vec<(usize, usize, usize)> = Vec::new(); // (rank, start, end)
        let mut depth = 0usize;
        let mut after_separator = true;
        for index in start..end {
            let (begins_item, item_rank) = match &self.nodes[index] {
                Node::Token(token, _) => {
                    let begins = depth == 0
                        && starts_item(token)
                        && (!separator_required || after_separator);
                    let item_rank = if begins { rank(token) } else { usize::MAX };
                    match token {
                        Token::LBrace => depth += 1,
                        Token::RBrace => depth = depth.saturating_sub(1),
                        _ => {}
                    }
                    after_separator =
                        matches!(token, Token::Other(';') | Token::LBrace | Token::RBrace);
                    (begins, item_rank)
                }
                Node::Comment { .. } => {
                    // A depth-0 comment whose next token begins an item is
                    // that item's prefix — own-line or not (an inline
                    // `{ // note` rides the `{` line, a doc block travels
                    // with its member). Any other comment extends the
                    // current item (or starts a junk item).
                    let next = self.nodes[index..end].iter().find_map(|node| match node {
                        Node::Token(token, _) => Some(token.clone()),
                        Node::Comment { .. } => None,
                    });
                    let begins = depth == 0
                        && next.as_ref().is_some_and(&starts_item)
                        && (!separator_required || after_separator);
                    let item_rank = next
                        .filter(|_| begins)
                        .map(|token| rank(&token))
                        .unwrap_or(usize::MAX);
                    (begins, item_rank)
                }
            };
            if begins_item || items.is_empty() {
                items.push((item_rank, index, index));
            }
            items.last_mut().unwrap().2 = index + 1;
        }
        if items.len() < 2 {
            return;
        }
        items.sort_by_key(|(item_rank, _, _)| *item_rank);
        let mut reordered: Vec<Node<'src>> = self.nodes[..start].to_vec();
        for (_, from, to) in &items {
            reordered.extend(self.nodes[*from..*to].iter().cloned());
        }
        reordered.extend(self.nodes[end..].iter().cloned());
        self.nodes = reordered;
    }

    /// The brace range `[open, close_exclusive)` of the `{ ... }` body
    /// whose open brace is the first `LBrace` node at or after `from`,
    /// balanced over the token stream (braces inside comments or string
    /// tokens never count). `(usize::MAX, usize::MAX)` when no open brace
    /// exists.
    fn brace_range(&self, from: usize) -> (usize, usize) {
        let Some(open) = (from..self.nodes.len())
            .find(|index| matches!(self.nodes[*index], Node::Token(Token::LBrace, _)))
        else {
            return (usize::MAX, usize::MAX);
        };
        let mut depth = 1usize;
        let mut close = open + 1;
        while depth > 0 {
            match self.nodes.get(close) {
                None => break,
                Some(Node::Token(Token::LBrace, _)) => {
                    depth += 1;
                    close += 1;
                }
                Some(Node::Token(Token::RBrace, _)) => {
                    depth -= 1;
                    close += 1;
                }
                Some(_) => close += 1,
            }
        }
        (open, close)
    }

    /// Rewrites the event declaration's body members into canonical order —
    /// fields, then `owner`, `classification`, `retention`, `pii`, and the
    /// `cloudEvents` block — before [`Formatter::scan_body`] walks them.
    /// Runs on the raw node stream (the formatter never builds the AST), so
    /// sources with parse errors still order.
    pub(crate) fn hoist_evt_event_members(&mut self) {
        fn member_rank(token: &Token<'_>) -> usize {
            match token {
                Token::Ident("owner") => 1,
                Token::Ident("classification") => 2,
                Token::Ident("retention") => 3,
                Token::Ident("pii") => 4,
                Token::Ident("cloudEvents") => 5,
                // Payload fields — including a field named `type` (the
                // keyword lookalike).
                _ => 0,
            }
        }
        let (open, close) = self.brace_range(self.pos);
        if open == usize::MAX {
            return;
        }
        self.sort_node_range(
            open + 1,
            close - 1,
            true,
            |token| {
                matches!(
                    token,
                    Token::Ident(_) | Token::IdentEscaped(_) | Token::Type
                )
            },
            member_rank,
        );
    }

    /// Rewrites a `delivery` block's entries into canonical order — `mode`,
    /// `maxAttempts`, `backoffSeconds`, `timeoutSeconds`, `deadLetter` —
    /// before the scanner emits them one per line.
    pub(crate) fn hoist_evt_delivery_entries(&mut self) {
        fn entry_rank(token: &Token<'_>) -> usize {
            match token {
                Token::Ident("mode") => 0,
                Token::Ident("maxAttempts") => 1,
                Token::Ident("backoffSeconds") => 2,
                Token::Ident("timeoutSeconds") => 3,
                Token::Ident("deadLetter") => 4,
                _ => usize::MAX,
            }
        }
        let (open, close) = self.brace_range(self.pos);
        if open == usize::MAX {
            return;
        }
        self.sort_node_range(
            open + 1,
            close - 1,
            false,
            |token| {
                matches!(
                    token,
                    Token::Ident("mode")
                        | Token::Ident("maxAttempts")
                        | Token::Ident("backoffSeconds")
                        | Token::Ident("timeoutSeconds")
                        | Token::Ident("deadLetter")
                )
            },
            entry_rank,
        );
    }

    /// Rewrites a `cloudEvents` block's entries into canonical order —
    /// `type`, `source`, `subject` — before the scanner emits them one per
    /// line.
    pub(crate) fn hoist_evt_cloud_events_entries(&mut self) {
        fn entry_rank(token: &Token<'_>) -> usize {
            match token {
                Token::Type => 0,
                Token::Ident("source") => 1,
                Token::Ident("subject") => 2,
                _ => usize::MAX,
            }
        }
        let (open, close) = self.brace_range(self.pos);
        if open == usize::MAX {
            return;
        }
        self.sort_node_range(
            open + 1,
            close - 1,
            false,
            |token| {
                matches!(token, Token::Type)
                    || matches!(token, Token::Ident("source") | Token::Ident("subject"))
            },
            entry_rank,
        );
    }

    /// Consumes the trailing `;` of a `.evt` statement line and emits the
    /// canonical semicolon whether or not the source had one (the
    /// `.ddd` import-`;` rule).
    fn push_evt_semicolon(&mut self) {
        if matches!(self.peek_tok(), Some(Token::Other(';'))) {
            self.bump_token();
        }
        self.push_text(";", true);
    }

    /// Consumes and emits an event payload field's name: ordinarily a name,
    /// plus the `type` keyword itself (the parser's keyword-lookalike rule).
    fn take_evt_field_name(&mut self) -> bool {
        self.take_if(|token| {
            matches!(
                token,
                Token::Ident(_) | Token::IdentEscaped(_) | Token::Type
            )
        })
    }

    /// Consumes and emits a spaced-bracket name list — `pii [ A B ]` — the
    /// `events [ ... ]` clause shape.
    fn scan_evt_name_list(&mut self) {
        if matches!(self.peek_tok(), Some(Token::LBracket)) {
            self.bump_token();
        }
        self.push_text("[", false);
        while let Some(Token::Ident(_) | Token::IdentEscaped(_)) = self.peek_tok() {
            self.take_name();
        }
        if matches!(self.peek_tok(), Some(Token::RBracket)) {
            self.bump_token();
        }
        self.push_text("]", false);
    }

    /// Consumes and emits the `cloudEvents { ... }` block of an event body:
    /// one `type`/`source`/`subject` string entry per line (no `;` — the
    /// block's grammar is bracket-only), an empty block staying inline.
    pub(crate) fn scan_evt_cloud_events(&mut self) {
        self.advance(); // the contextual `cloudEvents` word
        self.hoist_evt_cloud_events_entries();
        if !self.take_if(|token| matches!(token, Token::LBrace)) {
            self.flush_line();
            return;
        }
        if matches!(
            self.nodes.get(self.pos + 1),
            Some(Node::Token(Token::RBrace, _))
        ) {
            self.pos += 1;
            self.push_text("}", true);
            self.flush_line();
            return;
        }
        self.flush_line();
        self.indent += 1;
        loop {
            self.flush_pending(self.indent);
            match self.peek_tok() {
                Some(Token::RBrace) | None => break,
                Some(Token::Type)
                | Some(Token::Ident("source"))
                | Some(Token::Ident("subject")) => {
                    self.advance();
                    self.take_if(|token| matches!(token, Token::Str(_)));
                    self.flush_line();
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

    /// Consumes and emits the `delivery { ... }` block of a subscription
    /// body: one `<word> <value>;` entry per line in canonical order, an
    /// empty block staying inline.
    pub(crate) fn scan_evt_delivery(&mut self) {
        self.advance(); // the contextual `delivery` word
        self.hoist_evt_delivery_entries();
        if !self.take_if(|token| matches!(token, Token::LBrace)) {
            self.flush_line();
            return;
        }
        if matches!(
            self.nodes.get(self.pos + 1),
            Some(Node::Token(Token::RBrace, _))
        ) {
            self.pos += 1;
            self.push_text("}", true);
            self.flush_line();
            return;
        }
        self.flush_line();
        self.indent += 1;
        loop {
            self.flush_pending(self.indent);
            match self.peek_tok() {
                Some(Token::RBrace) | None => break,
                Some(Token::Ident(word))
                    if matches!(
                        word,
                        "mode" | "maxAttempts" | "backoffSeconds" | "timeoutSeconds" | "deadLetter"
                    ) =>
                {
                    self.advance();
                    if matches!(word, "mode" | "deadLetter") {
                        self.take_name();
                    } else {
                        self.take_if(|token| matches!(token, Token::Int(_)));
                    }
                    self.push_evt_semicolon();
                    self.flush_line();
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

    /// One member of an event body: a governance line, the `cloudEvents`
    /// block, or a `name: Type;` payload field. Every statement line gets
    /// its canonical `;`; the members arrive in canonical order (see
    /// [`Formatter::hoist_evt_event_members`]).
    pub(crate) fn scan_evt_event_item(&mut self) {
        match self.peek_tok() {
            Some(Token::Ident("owner")) | Some(Token::Ident("classification")) => {
                self.advance();
                self.take_name();
                self.push_evt_semicolon();
            }
            Some(Token::Ident("retention")) => {
                self.advance();
                self.take_if(|token| matches!(token, Token::Str(_)));
                self.push_evt_semicolon();
            }
            Some(Token::Ident("pii")) => {
                self.advance();
                self.scan_evt_name_list();
                self.push_evt_semicolon();
            }
            Some(Token::Ident("cloudEvents")) => self.scan_evt_cloud_events(),
            Some(Token::Ident(_) | Token::IdentEscaped(_) | Token::Type) => {
                self.take_evt_field_name();
                if matches!(self.peek_tok(), Some(Token::Other(':'))) {
                    self.bump_token();
                }
                self.push_text(":", true);
                self.scan_qname();
                self.push_evt_semicolon();
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

    /// One item of a subscription body: the `events [...]` list, the
    /// `consumer <name> ("scope" <name>)?` line, the `when (...)` filter
    /// (interior verbatim, never wrapped — the `when`-condition rule), or
    /// the `delivery { ... }` block. The clauses arrive in canonical order.
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
            Some(Token::When) => {
                self.advance();
                self.scan_raw_parens();
            }
            Some(Token::Ident("delivery")) => self.scan_evt_delivery(),
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
    /// declaration of an `.evt` file. The body members are reordered into
    /// the canonical shape (fields → governance → cloudEvents) first.
    pub(crate) fn scan_evt_event(&mut self) {
        self.begin_top_decl();
        self.advance();
        self.take_name();
        if self.take_if(|token| matches!(token, Token::Version)) {
            self.take_if(|token| matches!(token, Token::Str(_)));
        }
        self.hoist_evt_event_members();
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
