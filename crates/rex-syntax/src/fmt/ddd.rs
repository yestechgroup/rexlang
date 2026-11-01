//! The `.ddd` formatter: application/module/service/design/repository/search scanners, search-member reordering, and the `import` hoist.

use super::*;
use crate::ddd;

impl<'src> Formatter<'src> {
    /// Like [`Formatter::run_actors`], but for `.ddd` files: the only
    /// recognized top-level constructs are `import` declarations and the
    /// `application` block, with imports hoisted ahead of it.
    pub(crate) fn run_ddd(&mut self) -> String {
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

    pub(crate) fn scan_ddd_application_item(&mut self) {
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

    pub(crate) fn scan_ddd_module_item(&mut self) {
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
    /// five design flags (each possibly negated, `!auditable`) — collected
    /// and re-emitted in the canonical order of [`ddd::DESIGN_FLAGS`]
    /// (`scaffold auditable optimisticLocking nonPersistent cache`) whatever
    /// the source order (mirroring the parser's `DddFlags`, whose fields the
    /// driver reads positionally) — and an optional `repository` block.
    pub(crate) fn scan_ddd_design(&mut self) {
        if matches!(self.peek_tok(), Some(Token::Ident(ddd::ABSTRACT))) {
            self.advance();
        }
        self.advance(); // the stereotype keyword
        self.take_name();
        let mut seen = [None; ddd::DESIGN_FLAGS.len()];
        loop {
            // A negated flag is the two-token run `! keyword`; the `!` is
            // consumed without emitting (flags re-emit canonically below)
            // only once a flag keyword confirms behind it.
            let negated = matches!(self.peek_tok(), Some(Token::Other('!')))
                && self.peek_tok_nth(1).is_some_and(
                    |token| matches!(&token, Token::Ident(text) if ddd::flag_kind(text).is_some()),
                );
            if negated {
                self.bump_token();
            }
            let index = match self.peek_tok() {
                Some(Token::Ident(text)) => ddd::DESIGN_FLAGS
                    .iter()
                    .position(|(keyword, _)| *keyword == text),
                _ => None,
            };
            let Some(index) = index else {
                break;
            };
            self.bump_token();
            seen[index] = Some(negated);
        }
        for (index, (keyword, _)) in ddd::DESIGN_FLAGS.iter().enumerate() {
            if let Some(negated) = seen[index] {
                if negated {
                    self.push_text("!", false);
                    self.push_text(keyword, true);
                } else {
                    self.push_text(keyword, false);
                }
            }
        }
        if matches!(self.peek_tok(), Some(Token::Ident(ddd::REPOSITORY))) {
            self.advance();
            self.take_name();
            self.scan_body(BodyKind::DddRepository);
        }
        if matches!(self.peek_tok(), Some(Token::Ident(ddd::MACHINE))) {
            // The machine block starts on its own line (unlike the
            // `repository` block, which joins the design line).
            self.flush_line();
            self.advance();
            self.take_name();
            if matches!(self.peek_tok(), Some(Token::Other(':'))) {
                self.bump_token();
            }
            self.push_text(":", true);
            self.scan_qname();
            self.scan_body(BodyKind::DddMachine);
        }
    }

    /// One member of a `machine` body: an `initial <State>;` line, a
    /// `terminal <State>;` line, or a `<From> -> <To> [via <Trigger>]
    /// [when (...)] ;` transition (the guard's parens re-emit verbatim, the
    /// `when`-condition rule).
    pub(crate) fn scan_ddd_machine_item(&mut self) {
        match self.peek_tok() {
            // The member keywords are contextual: a leading `initial` or
            // `terminal` followed by the arrow is a transition *from* a
            // state of that name, not a member line.
            Some(Token::Ident(ddd::INITIAL | ddd::TERMINAL))
                if !matches!(self.peek_tok_nth(1), Some(Token::Arrow)) =>
            {
                self.advance();
                self.take_name();
                self.take_if(|token| matches!(token, Token::Other(';')));
            }
            Some(Token::Ident(_) | Token::IdentEscaped(_)) => {
                self.take_name();
                if self.take_if(|token| matches!(token, Token::Arrow)) {
                    self.take_name();
                    if self.take_if(|token| matches!(token, Token::Ident(ddd::VIA))) {
                        self.take_name();
                    }
                    if self.take_if(|token| matches!(token, Token::When)) {
                        self.scan_raw_parens();
                    }
                }
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

    pub(crate) fn scan_ddd_service_item(&mut self) {
        match self.peek_tok() {
            Some(Token::Ident(ddd::INJECT)) => {
                self.advance();
                self.take_name();
                self.take_if(|token| matches!(token, Token::Other(';')));
            }
            Some(Token::Ident(_) | Token::IdentEscaped(_)) => {
                // An optional `protected` modifier precedes the operation
                // (contextual: anywhere else it is an ordinary name, and
                // `advance()` re-emits it verbatim).
                if matches!(self.peek_tok(), Some(Token::Ident(ddd::PROTECTED))) {
                    self.advance();
                }
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
                if self.take_if(|token| matches!(token, Token::When)) {
                    self.scan_raw_parens();
                }
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
    pub(crate) fn scan_ddd_capabilities(&mut self) {
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
    pub(crate) fn scan_ddd_params(&mut self) {
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

    pub(crate) fn scan_ddd_repository_item(&mut self) {
        match self.peek_tok() {
            // A `protected` modifier on a built-in (`protected findByKeys;`):
            // both tokens re-emit verbatim via `advance()`.
            Some(Token::Ident(ddd::PROTECTED))
                if self.peek_tok_nth(1).is_some_and(|token| {
                    matches!(&token, Token::Ident(text) if ddd::builtin_from_keyword(text).is_some())
                }) =>
            {
                self.advance();
                self.advance();
                self.take_if(|token| matches!(token, Token::Other(';')));
            }
            Some(Token::Ident(text)) if ddd::builtin_from_keyword(text).is_some() => {
                self.advance();
                self.take_if(|token| matches!(token, Token::Other(';')));
            }
            Some(Token::Ident(_) | Token::IdentEscaped(_)) => {
                // An optional `protected` modifier precedes the operation
                // (contextual: anywhere else it is an ordinary name, and
                // `advance()` re-emits it verbatim).
                if matches!(self.peek_tok(), Some(Token::Ident(ddd::PROTECTED))) {
                    self.advance();
                }
                self.scan_qname();
                self.scan_multiplicity();
                self.take_name();
                self.scan_ddd_params();
                if self.take_if(|token| matches!(token, Token::When)) {
                    self.scan_raw_parens();
                }
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
    pub(crate) fn scan_ddd_search(&mut self) {
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
    pub(crate) fn reorder_ddd_search_members(&mut self) {
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

    pub(crate) fn scan_ddd_search_item(&mut self) {
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
    pub(crate) fn scan_ddd_search_field_item(&mut self) {
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
    pub(crate) fn scan_ddd_search_name_item(&mut self) {
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
    pub(crate) fn scan_ddd_search_document_item(&mut self) {
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
    pub(crate) fn scan_ddd_search_pagination_item(&mut self) {
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

    // --- .evt bodies ---------------------------------------------------------

    /// Rewrites the node stream into canonical order for `.ddd` files:
    /// `import` declarations first (source order), then the `application`
    /// block and any junk items in source order.
    ///
    /// Items are contiguous node ranges split at brace-depth 0 on `Import`
    /// tokens and the contextual `application` identifier; the first node
    /// unconditionally begins item 0, so leading comments travel with their
    /// item.
    pub(crate) fn hoist_ddd_imports(&mut self) {
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

    /// Consumes and emits one `import "<path>";` declaration of a `.ddd`
    /// file: the tight import section of the `.actor` formatter, plus the
    /// canonical trailing `;` (emitted whether or not the source had one; a
    /// source `;` is consumed silently so it never doubles up).
    pub(crate) fn scan_ddd_import(&mut self) {
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

    /// Consumes and emits the `application <name> { ... }` block of a
    /// `.ddd` file.
    pub(crate) fn scan_ddd_application(&mut self) {
        self.begin_top_decl();
        self.advance();
        self.take_name();
        self.scan_body(BodyKind::DddApplication);
    }

    /// Unrecognized top-level tokens of a `.ddd` file: emit them on one
    /// line, stopping at the next `import` keyword or the contextual
    /// `application` keyword (mirrors the parser's file-level recovery).
    pub(crate) fn scan_top_junk_ddd(&mut self) {
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
}
