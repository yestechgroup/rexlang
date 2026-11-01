//! The `.uc` formatter: participant/occurrence/message/succession/realizes scanners, the member reorder, and the hoist.

use super::*;

impl<'src> Formatter<'src> {
    /// Like [`Formatter::run_deploy`], but for `.uc` files: the only
    /// recognized top-level constructs are `import` declarations and the
    /// contextual `usecase` blocks, canonicalized into imports → usecases
    /// order by [`Formatter::hoist_uc_items`].
    pub(crate) fn run_usecases(&mut self) -> String {
        self.hoist_uc_items();
        loop {
            let front = self.front().cloned();
            match front {
                None => break,
                Some(Node::Comment { .. }) => self.advance(),
                Some(Node::Token(token, _)) => match token {
                    Token::Import => self.scan_uc_import(),
                    Token::Ident(usecases::USECASE) => self.scan_usecase(),
                    _ => self.scan_top_junk_uc(),
                },
            }
        }
        self.finish()
    }

    // --- node consumption ----------------------------------------------------

    /// One item of a usecase body: a `subject`/`actor` participant (with
    /// its optional occurrence/perform body), a `message`, a
    /// `first ... then ...` succession, or a `realizes` binding.
    pub(crate) fn scan_uc_item(&mut self) {
        match self.peek_tok() {
            Some(Token::Ident(text)) if text == usecases::SUBJECT || text == usecases::MESSAGE => {
                if text == usecases::SUBJECT {
                    self.scan_uc_participant();
                } else {
                    self.scan_uc_message();
                }
            }
            Some(Token::Ident(text)) if text == usecases::FIRST || text == usecases::REALIZES => {
                if text == usecases::FIRST {
                    self.scan_uc_succession();
                } else {
                    self.scan_uc_realizes();
                }
            }
            Some(Token::Actor) => self.scan_uc_participant(),
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
                            Token::RBrace | Token::Ident(_) | Token::IdentEscaped(_) | Token::Actor
                        )
                    });
                }
            }
        }
    }

    /// One `subject`/`actor` participant line: the introducing word, the
    /// name, the optional `: <type>` clause (tight colon, dotted type),
    /// and the optional occurrence/perform body. The introducing word is
    /// echoed as written (`Token::Actor` for actors, the contextual
    /// `subject` identifier for subjects).
    pub(crate) fn scan_uc_participant(&mut self) {
        self.advance(); // the `subject`/`actor` word
        self.take_name();
        if matches!(self.peek_tok(), Some(Token::Other(':'))) {
            self.bump_token();
            self.push_text(":", true);
            self.scan_qname();
        }
        self.scan_body(BodyKind::UcParticipant);
    }

    /// One member of a participant body: an `occurrence` line or a
    /// `perform` step, each with its canonical `;`.
    pub(crate) fn scan_uc_participant_item(&mut self) {
        match self.peek_tok() {
            Some(Token::Ident(text)) if text == usecases::OCCURRENCE => {
                self.advance();
                self.take_name();
                if matches!(self.peek_tok(), Some(Token::Dot)) {
                    self.advance();
                    self.take_if(|token| matches!(token, Token::Ident(_) | Token::IdentEscaped(_)));
                }
                self.scan_uc_semicolon();
            }
            Some(Token::Ident(text)) if text == usecases::PERFORM => {
                self.advance();
                self.take_name();
                if matches!(self.peek_tok(), Some(Token::Other(':'))) {
                    self.bump_token();
                    self.push_text(":", true);
                    self.take_name();
                }
                if self.take_if(
                    |token| matches!(token, Token::Ident(text) if *text == usecases::REQUIRES),
                ) {
                    self.scan_uc_requires_list();
                }
                self.scan_uc_semicolon();
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

    /// The `requires [ A B ]` clause of a perform step: spaced brackets
    /// (the `.evt` events-list rule) with the names space-separated
    /// inside. The contextual `occurrence`/`perform` words end the list,
    /// so a missing `]` never swallows the next member.
    pub(crate) fn scan_uc_requires_list(&mut self) {
        if matches!(self.peek_tok(), Some(Token::LBracket)) {
            self.bump_token();
        }
        self.push_text("[", false);
        loop {
            match self.peek_tok() {
                Some(Token::Ident(text) | Token::IdentEscaped(text))
                    if text != usecases::OCCURRENCE && text != usecases::PERFORM =>
                {
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

    /// Consumes a source `;` silently and emits the canonical one (the
    /// `.evt` field-`;` rule).
    pub(crate) fn scan_uc_semicolon(&mut self) {
        if matches!(self.peek_tok(), Some(Token::Other(';'))) {
            self.bump_token();
        }
        self.push_text(";", true);
    }

    /// One `participant.anchor` endpoint of a message clause (tight dots).
    pub(crate) fn scan_uc_endpoint(&mut self) {
        self.take_name();
        if matches!(self.peek_tok(), Some(Token::Dot)) {
            self.advance();
            self.take_name();
        }
    }

    /// One `message` line with its optional `of`/`from`/`to` clauses. A
    /// clause breaks onto a continuation line (one extra indent level)
    /// when appending it would push the line past the message-line budget;
    /// the check runs per clause.
    pub(crate) fn scan_uc_message(&mut self) {
        self.advance(); // the contextual `message` word
        self.take_name();
        if self.take_if(|token| matches!(token, Token::Ident(text) if *text == usecases::OF)) {
            self.scan_qname();
        }
        let mut wrapped = 0usize;
        if matches!(self.peek_tok(), Some(Token::From)) {
            if self.uc_clause_overflows("from") {
                self.flush_line();
                self.indent += 1;
                wrapped += 1;
            }
            self.advance();
            self.scan_uc_endpoint();
        }
        if matches!(self.peek_tok(), Some(Token::Ident(text)) if text == usecases::TO) {
            if self.uc_clause_overflows("to") {
                self.flush_line();
                self.indent += 1;
                wrapped += 1;
            }
            self.advance();
            self.scan_uc_endpoint();
        }
        self.scan_uc_semicolon();
        self.indent -= wrapped;
    }

    /// Whether appending the `word participant.anchor` clause at the front
    /// would push the line under construction past the message-line budget.
    /// A malformed clause never wraps (there is nothing to measure).
    pub(crate) fn uc_clause_overflows(&self, word: &str) -> bool {
        let mut tokens = self.nodes[self.pos..].iter().filter_map(|node| match node {
            Node::Token(token, _) => Some(token),
            _ => None,
        });
        let _preposition = tokens.next();
        let clause = match (tokens.next(), tokens.next(), tokens.next()) {
            (
                Some(Token::Ident(participant) | Token::IdentEscaped(participant)),
                Some(Token::Dot),
                Some(Token::Ident(anchor) | Token::IdentEscaped(anchor)),
            ) => word.len() + 1 + participant.len() + 1 + anchor.len(),
            _ => return false,
        };
        self.line_indent * 4 + self.line.len() + 1 + clause > UC_MESSAGE_LINE_BUDGET
    }

    /// One `first <node> then <node>;` succession line.
    pub(crate) fn scan_uc_succession(&mut self) {
        self.advance(); // the contextual `first` word
        self.scan_uc_node_ref();
        if self.take_if(|token| matches!(token, Token::Ident(text) if *text == usecases::THEN)) {
            self.scan_uc_node_ref();
        }
        self.scan_uc_semicolon();
    }

    /// One node reference: the contextual `start`/`done` words, or a
    /// (possibly dotted) name.
    pub(crate) fn scan_uc_node_ref(&mut self) {
        match self.peek_tok() {
            Some(Token::Ident(text)) if text == usecases::START || text == usecases::DONE => {
                self.advance();
            }
            Some(Token::Ident(_) | Token::IdentEscaped(_)) => {
                self.take_name();
                if matches!(self.peek_tok(), Some(Token::Dot)) {
                    self.advance();
                    self.take_name();
                }
            }
            _ => {}
        }
    }

    /// One `realizes alias.Service.operation;` binding (tight dots).
    pub(crate) fn scan_uc_realizes(&mut self) {
        self.advance(); // the contextual `realizes` word
        self.scan_qname();
        self.scan_uc_semicolon();
    }

    // --- .cmmn bodies --------------------------------------------------------

    /// Rewrites the node stream into canonical `.uc` order: a leading
    /// comment run (the file header) stays at the very front, then `import`
    /// declarations, then `usecase` declarations. The
    /// [`Formatter::hoist_evt_items`] machinery with the contextual
    /// `usecase` rank.
    pub(crate) fn hoist_uc_items(&mut self) {
        fn rank(token: &Token<'_>) -> usize {
            match token {
                Token::Import => 0,
                Token::Ident(usecases::USECASE) => 1,
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

    /// Consumes and emits one `import "<path>" (as <name>)?` declaration
    /// of a `.uc` file: the tight import section of the `.actor` formatter,
    /// with the alias kept when written.
    pub(crate) fn scan_uc_import(&mut self) {
        self.flush_line();
        self.flush_pending(0);
        self.advance();
        self.take_if(|token| matches!(token, Token::Str(_)));
        if self.take_if(|token| matches!(token, Token::As)) {
            self.take_name();
        }
        self.flush_line();
    }

    /// Consumes and emits one `usecase <name> { ... }` declaration of a
    /// `.uc` file. The body's members are rewritten into the canonical
    /// subject → actors → rest order (see [`Formatter::reorder_uc_members`])
    /// before the linear scan.
    pub(crate) fn scan_usecase(&mut self) {
        self.begin_top_decl();
        self.advance();
        self.take_name();
        self.reorder_uc_members();
        self.scan_body(BodyKind::UcUseCase);
    }

    /// Rewrites the node stream of the usecase body ahead of the formatter
    /// (positioned at the body's `{`) so its members appear in the
    /// canonical order — the `subject` line, then the `actor` lines, then
    /// messages, successions, and realizes bindings — whatever the source
    /// order, source order kept within each group. The same node-range
    /// technique the import hoists and the `.ddd` search reorder use.
    ///
    /// Member extents mirror the parser's productions: a participant is
    /// word + name + (`:` dotted type)? + (balanced braces)?; a statement
    /// member runs to its depth-0 `;`. A member's leading comments travel
    /// with it; junk chunks (parse-error regions) keep their relative
    /// order at the end.
    pub(crate) fn reorder_uc_members(&mut self) {
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
        let inner_end = close - 1;
        let mut segments: Vec<(usize, Vec<Node<'src>>)> = Vec::new();
        let mut i = open + 1;
        while i < inner_end {
            let start = i;
            // Lookahead past comments to the member-leading token: the
            // comments join the member's segment so doc blocks travel
            // with it.
            let mut j = start;
            let first = loop {
                match self.nodes.get(j) {
                    Some(Node::Token(token, _)) => break Some(token.clone()),
                    Some(Node::Comment { .. }) => j += 1,
                    None => break None,
                }
            };
            let end = match first {
                Some(ref token) if is_uc_member_start(token) => {
                    segments.push((uc_member_rank(token), Vec::new()));
                    self.uc_member_extent(j, inner_end)
                }
                _ => {
                    // Junk: consume through the chunk up to the next member
                    // start so it moves as one (comments-only chunks too).
                    segments.push((usize::MAX, Vec::new()));
                    let mut k = j;
                    if first.is_some() {
                        k += 1;
                    }
                    while k < inner_end {
                        match self.nodes.get(k) {
                            Some(Node::Token(token, _)) if is_uc_member_start(token) => break,
                            _ => k += 1,
                        }
                    }
                    k
                }
            };
            segments
                .last_mut()
                .unwrap()
                .1
                .extend(self.nodes[start..end].iter().cloned());
            i = end;
        }
        segments.sort_by_key(|(rank, _)| *rank);
        let mut reordered: Vec<Node<'src>> = self.nodes[..=open].to_vec();
        for (_, segment) in &segments {
            reordered.extend(segment.iter().cloned());
        }
        reordered.extend(self.nodes[inner_end..].iter().cloned());
        self.pos = open;
        self.nodes = reordered;
    }

    /// The exclusive end of the usecase member whose leading token sits at
    /// `start`: a participant is word + name + (`:` dotted type)? +
    /// (balanced braces)? — the same shape for `subject` and `actor` — and
    /// a statement member runs to its depth-0 `;` (or the end of the body
    /// when the terminator was never written).
    pub(crate) fn uc_member_extent(&self, start: usize, inner_end: usize) -> usize {
        let is_ref_name = |index: usize| {
            matches!(
                self.nodes.get(index),
                Some(Node::Token(token, _)) if matches!(token, Token::Ident(_) | Token::IdentEscaped(_)) || token.keyword().is_some()
            )
        };
        let is_ident = |index: usize| {
            matches!(
                self.nodes.get(index),
                Some(Node::Token(Token::Ident(_) | Token::IdentEscaped(_), _))
            )
        };
        let is_participant = matches!(
            self.nodes.get(start),
            Some(Node::Token(token, _))
                if matches!(token, Token::Actor)
                    || matches!(token, Token::Ident(text) | Token::IdentEscaped(text) if *text == usecases::SUBJECT)
        );
        let mut i = start + 1; // past the member-leading token
        if is_participant {
            if is_ident(i) {
                i += 1;
            }
            if matches!(self.nodes.get(i), Some(Node::Token(Token::Other(':'), _))) {
                i += 1;
                if is_ref_name(i) {
                    i += 1;
                    while matches!(self.nodes.get(i), Some(Node::Token(Token::Dot, _))) {
                        i += 1;
                        if is_ref_name(i) {
                            i += 1;
                        } else {
                            break;
                        }
                    }
                }
            }
        } else {
            // A statement member: to its `;` inclusive.
            while i < inner_end {
                let semi = matches!(self.nodes.get(i), Some(Node::Token(Token::Other(';'), _)));
                i += 1;
                if semi {
                    break;
                }
            }
            return i;
        }
        // The optional balanced participant body.
        if matches!(self.nodes.get(i), Some(Node::Token(Token::LBrace, _))) {
            let mut depth = 0usize;
            while i < inner_end {
                match self.nodes.get(i) {
                    Some(Node::Token(Token::LBrace, _)) => depth += 1,
                    Some(Node::Token(Token::RBrace, _)) => {
                        depth -= 1;
                        if depth == 0 {
                            i += 1;
                            break;
                        }
                    }
                    _ => {}
                }
                i += 1;
            }
        }
        i
    }

    /// Unrecognized top-level tokens of a `.uc` file: emit them on one
    /// line, stopping at the next `import` keyword or the contextual
    /// `usecase` word (mirrors the parser's file-level recovery).
    pub(crate) fn scan_top_junk_uc(&mut self) {
        self.begin_top_decl();
        loop {
            match self.peek_tok() {
                Some(token)
                    if !matches!(token, Token::Import | Token::Ident(usecases::USECASE)) =>
                {
                    self.advance()
                }
                _ => break,
            }
        }
        self.flush_line();
    }
}
