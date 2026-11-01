//! The `.cmmn` formatter: case/plan-item/roles/case-file scanners and the hoist.

use super::*;
use crate::cmmn;

impl<'src> Formatter<'src> {
    /// Like [`Formatter::run_usecases`], but for `.cmmn` files: the only
    /// recognized top-level constructs are `import` declarations and the
    /// contextual `case` blocks, canonicalized into imports → cases order
    /// by [`Formatter::hoist_cmmn_items`].
    pub(crate) fn run_cmmn(&mut self) -> String {
        self.hoist_cmmn_items();
        loop {
            let front = self.front().cloned();
            match front {
                None => break,
                Some(Node::Comment { .. }) => self.advance(),
                Some(Node::Token(token, _)) => match token {
                    Token::Import => self.scan_uc_import(),
                    Token::Ident(text) if text == cmmn::CASE => self.scan_case(),
                    _ => self.scan_top_junk_cmmn(),
                },
            }
        }
        self.finish()
    }

    /// One member of a `case` body: a `roles` block, a `case_file` block,
    /// or a plan item.
    pub(crate) fn scan_cmmn_case_item(&mut self) {
        match self.peek_tok() {
            Some(Token::Ident(text)) if text == cmmn::ROLES => {
                self.advance();
                self.scan_body(BodyKind::CmmnRoles);
            }
            Some(Token::Ident(text)) if text == cmmn::CASE_FILE => {
                self.advance();
                self.scan_body(BodyKind::CmmnCaseFile);
            }
            Some(Token::Ident(text))
                if text == cmmn::TASK
                    || text == cmmn::SERVICE
                    || text == cmmn::MILESTONE
                    || text == cmmn::STAGE
                    || text == cmmn::DISCRETIONARY =>
            {
                self.scan_cmmn_plan_item();
            }
            _ => self.scan_cmmn_junk_member("expected a case member"),
        }
    }

    /// One member of a plan item body: a `performer`/`execute` clause, an
    /// `entry`/`exit` criterion, a control flag, or (in a stage) a nested
    /// plan item. Nested items inside a non-stage body are junk — the
    /// formatter echoes them on one line, mirroring the parser's recovery.
    pub(crate) fn scan_cmmn_plan_item(&mut self) {
        match self.peek_tok() {
            Some(Token::Ident(text))
                if text == cmmn::TASK
                    || text == cmmn::SERVICE
                    || text == cmmn::MILESTONE
                    || text == cmmn::STAGE =>
            {
                self.advance();
                self.take_name();
                self.scan_body(BodyKind::CmmnPlan);
            }
            Some(Token::Ident(text)) if text == cmmn::DISCRETIONARY => {
                self.advance();
                match self.peek_tok() {
                    Some(Token::Ident(text))
                        if text == cmmn::TASK
                            || text == cmmn::SERVICE
                            || text == cmmn::MILESTONE =>
                    {
                        self.advance();
                        self.take_name();
                        self.scan_body(BodyKind::CmmnPlan);
                    }
                    _ => self.scan_cmmn_junk_member("expected a plan item member"),
                }
            }
            Some(Token::Ident(text)) if text == cmmn::PERFORMER => {
                self.advance();
                if matches!(self.peek_tok(), Some(Token::Other(':'))) {
                    self.bump_token();
                    self.push_text(":", true);
                    self.take_name();
                }
                self.scan_cmmn_semicolon();
            }
            Some(Token::Ident(text)) if text == cmmn::EXECUTE => {
                self.advance();
                self.scan_qname();
                self.scan_cmmn_semicolon();
            }
            Some(Token::Ident(text)) if text == cmmn::ENTRY || text == cmmn::EXIT => {
                self.advance();
                if matches!(self.peek_tok(), Some(Token::On)) {
                    self.advance();
                    loop {
                        // One on-part: `name` or `name.tail` (tight dots).
                        self.take_name();
                        if matches!(self.peek_tok(), Some(Token::Dot)) {
                            self.advance();
                            self.take_if(|token| {
                                matches!(token, Token::Ident(_) | Token::IdentEscaped(_))
                            });
                        }
                        if matches!(self.peek_tok(), Some(Token::Comma)) {
                            self.advance();
                        } else {
                            break;
                        }
                    }
                }
                if matches!(self.peek_tok(), Some(Token::When)) {
                    self.advance();
                    self.scan_raw_parens();
                }
                self.scan_cmmn_semicolon();
            }
            Some(Token::Ident(text))
                if text == cmmn::REQUIRED || text == cmmn::REPEATS || text == cmmn::AUTO =>
            {
                self.advance();
                self.scan_cmmn_semicolon();
            }
            _ => self.scan_cmmn_junk_member("expected a plan item member"),
        }
    }

    /// One `role` line of a roles block: the name and the optional
    /// `as <actor>` binding, with its canonical `;`.
    pub(crate) fn scan_cmmn_roles_item(&mut self) {
        self.advance(); // the contextual `role` word
        self.take_name();
        if matches!(self.peek_tok(), Some(Token::As)) {
            self.advance();
            self.take_name();
        }
        self.scan_cmmn_semicolon();
    }

    /// One typed item of a case-file block: the `.mox` attribute production
    /// (`Type[mult]? name;`) — dotted type, normalized tight multiplicity,
    /// then the name.
    pub(crate) fn scan_cmmn_case_file_item(&mut self) {
        self.scan_qname();
        self.scan_multiplicity();
        self.take_name();
        self.scan_cmmn_semicolon();
    }

    /// Junk inside a `.cmmn` body: consumes the front token (progress) and
    /// scans to the next token that could begin a proper member.
    pub(crate) fn scan_cmmn_junk_member(&mut self, _expected: &str) {
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

    /// Consumes a source `;` silently and emits the canonical one (the
    /// `.evt` field-`;` rule).
    pub(crate) fn scan_cmmn_semicolon(&mut self) {
        if matches!(self.peek_tok(), Some(Token::Other(';'))) {
            self.bump_token();
        }
        self.push_text(";", true);
    }

    // --- top-level declarations ----------------------------------------------

    /// Consumes and emits one `case <name> { ... }` declaration of a
    /// `.cmmn` file. The body keeps its source order (roles block,
    /// case-file block, plan items).
    pub(crate) fn scan_case(&mut self) {
        self.begin_top_decl();
        self.advance();
        self.take_name();
        self.scan_body(BodyKind::CmmnCase);
    }

    /// The `.cmmn` hoist: identical to [`Formatter::hoist_uc_items`], with
    /// the contextual `case` word ranking after the imports.
    pub(crate) fn hoist_cmmn_items(&mut self) {
        fn rank(token: &Token<'_>) -> usize {
            match token {
                Token::Import => 0,
                Token::Ident(text) if *text == cmmn::CASE => 1,
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
        items.sort_by_key(|(rank, _, _)| *rank);
        let mut reordered: Vec<Node<'src>> = self.nodes[..header_end].to_vec();
        for (_, start, end) in &items {
            reordered.extend(self.nodes[*start..*end].iter().cloned());
        }
        self.nodes = reordered;
    }

    /// Unrecognized top-level tokens of a `.cmmn` file: emit them on one
    /// line, stopping at the next `import` keyword or the contextual `case`
    /// word (mirrors the parser's file-level recovery).
    pub(crate) fn scan_top_junk_cmmn(&mut self) {
        self.begin_top_decl();
        loop {
            match self.peek_tok() {
                Some(token)
                    if !matches!(token, Token::Import)
                        && !matches!(token, Token::Ident(text) if text == cmmn::CASE) =>
                {
                    self.advance()
                }
                _ => break,
            }
        }
        self.flush_line();
    }
}
