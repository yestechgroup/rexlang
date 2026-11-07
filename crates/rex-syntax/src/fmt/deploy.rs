//! The `.deploy` formatter: application/profile/deployment/policy/mapping scanners and the hoist.

use super::*;
use crate::deploy;

impl<'src> Formatter<'src> {
    /// Like [`Formatter::run_evt`], but for `.deploy` files: the only
    /// recognized top-level constructs are `import` declarations and the
    /// contextual `application`/`profile`/`deployment` blocks, canonicalized
    /// into imports → applications → profiles → deployments order by
    /// [`Formatter::hoist_deploy_items`].
    pub(crate) fn run_deploy(&mut self) -> String {
        self.hoist_deploy_items();
        loop {
            let front = self.front().cloned();
            match front {
                None => break,
                Some(Node::Comment { .. }) => self.advance(),
                Some(Node::Token(token, _)) => match token {
                    Token::Import => self.scan_import(),
                    Token::Ident(deploy::APPLICATION) => self.scan_deploy_application(),
                    Token::Ident(deploy::PROFILE) => self.scan_deploy_profile(),
                    Token::Ident(deploy::DEPLOYMENT) => self.scan_deploy_deployment(),
                    _ => self.scan_top_junk_deploy(),
                },
            }
        }
        self.finish()
    }

    /// Consumes and emits one `path: value` setting line at any of the
    /// three levels (component baseline, profile default, deployment
    /// override). The path is a qualified name; the canonical `:` spacing
    /// is emitted whether or not the source had the colon at all (the
    /// `.evt` field-`:` rule).
    pub(crate) fn scan_deploy_setting_item(&mut self) {
        match self.peek_tok() {
            Some(Token::Ident(_) | Token::IdentEscaped(_)) => {
                self.scan_qname();
                if matches!(self.peek_tok(), Some(Token::Other(':'))) {
                    self.bump_token();
                }
                self.push_text(":", true);
                match self.peek_tok() {
                    Some(Token::Str(_) | Token::Int(_)) => self.advance(),
                    // A bare-word value may be any keyword token
                    // (`container` in value position is just a word).
                    Some(token) if token.keyword().is_some() => self.advance(),
                    Some(Token::Ident(_) | Token::IdentEscaped(_)) => {
                        self.take_name();
                    }
                    _ => {}
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

    /// One member of a component body: `requires a, b`, a
    /// `design`/`flow` binding, an `env` block, or a `name: value`
    /// setting.
    pub(crate) fn scan_deploy_component_item(&mut self) {
        match self.peek_tok() {
            Some(Token::Ident(text)) if text == deploy::REQUIRES => {
                self.advance();
                self.take_name();
                while self.take_if(|token| matches!(token, Token::Comma)) {
                    self.take_name();
                }
            }
            Some(Token::Ident(text)) if text == deploy::DESIGN || text == deploy::FLOW => {
                self.advance();
                self.take_if(|token| matches!(token, Token::Str(_)));
            }
            Some(Token::Ident(text)) if text == deploy::ENV => {
                self.advance();
                self.scan_body(BodyKind::DeployEnv);
            }
            _ => self.scan_deploy_setting_item(),
        }
    }

    /// One `NAME = "value"` entry of a component's `env` block. The name is
    /// a bare word — keyword tokens included, the parser's qname-segment
    /// rule — and the canonical `=` spacing is emitted whether or not the
    /// source had the `=` at all.
    pub(crate) fn scan_deploy_env_item(&mut self) {
        match self.peek_tok() {
            Some(token)
                if matches!(token, Token::Ident(_) | Token::IdentEscaped(_))
                    || token.keyword().is_some() =>
            {
                self.advance();
                if matches!(self.peek_tok(), Some(Token::Eq)) {
                    self.bump_token();
                }
                self.push_text("=", false);
                if matches!(self.peek_tok(), Some(Token::Str(_))) {
                    self.advance();
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

    /// One `connects <from> -> <to>` line of an application body.
    pub(crate) fn scan_deploy_connects(&mut self) {
        self.advance(); // the contextual `connects` word
        self.take_name();
        if matches!(self.peek_tok(), Some(Token::Arrow)) {
            // Consumed silently; the canonical `->` is pushed below.
            self.bump_token();
        }
        self.push_text("->", false);
        self.take_name();
    }

    /// One item of an application body: a `component` block or a
    /// `connects` line.
    pub(crate) fn scan_deploy_application_item(&mut self) {
        match self.peek_tok() {
            Some(Token::Ident(text)) if text == deploy::COMPONENT => {
                self.advance();
                self.take_name();
                if matches!(self.peek_tok(), Some(Token::Other(':'))) {
                    self.bump_token();
                }
                self.push_text(":", true);
                self.take_name();
                self.scan_body(BodyKind::DeployComponent);
            }
            Some(Token::Ident(text)) if text == deploy::CONNECTS => self.scan_deploy_connects(),
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

    /// One `require (...)`/`prohibit (...)` policy line. The keyword joins
    /// the line; the parenthesized condition's inner bytes are emitted
    /// verbatim (ends trimmed), the `when`-condition rule — so policy
    /// conditions are a fixpoint.
    pub(crate) fn scan_deploy_policy(&mut self) {
        self.advance(); // the contextual `require`/`prohibit` word
        if !matches!(self.peek_tok(), Some(Token::LParen)) {
            return;
        }
        // The `(` is consumed silently; the emitted parens carry the
        // verbatim inner slice.
        self.bump_token();
        let Some(Node::Token(_, open_span)) = self.nodes.get(self.pos - 1) else {
            return;
        };
        let open_span = *open_span;
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
        // condition was never closed).
        let content_end = self
            .nodes
            .get(close_index - 1)
            .map(|node| node.span().start)
            .unwrap_or(open_span.end);
        let content = &self.source[open_span.end..content_end.max(open_span.end)];
        // Every node inside the parens (tokens *and* comments) is already
        // covered by the verbatim slice; skip them without emitting.
        self.pos = close_index;
        self.push_text("(", false);
        self.line.push_str(content.trim());
        self.line.push(')');
    }

    /// One member of a profile body: the `target:` line, a
    /// `defaults`/`mappings` block, or a policy line.
    pub(crate) fn scan_deploy_profile_item(&mut self) {
        match self.peek_tok() {
            Some(Token::Ident(text)) if text == deploy::TARGET => {
                self.advance();
                if matches!(self.peek_tok(), Some(Token::Other(':'))) {
                    self.bump_token();
                }
                self.push_text(":", true);
                self.take_name();
            }
            Some(Token::Ident(text)) if text == deploy::DEFAULTS || text == deploy::CONFIGURE => {
                self.advance();
                self.scan_body(BodyKind::DeploySettings);
            }
            Some(Token::Ident(text)) if text == deploy::MAPPINGS => {
                self.advance();
                self.scan_body(BodyKind::DeployMappings);
            }
            Some(Token::Ident(text)) if text == deploy::REQUIRE || text == deploy::PROHIBIT => {
                self.scan_deploy_policy();
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

    /// One `kind -> platform.resource` entry of a `mappings` block.
    pub(crate) fn scan_deploy_mapping_item(&mut self) {
        match self.peek_tok() {
            Some(Token::Ident(_) | Token::IdentEscaped(_)) => {
                self.take_name();
                if matches!(self.peek_tok(), Some(Token::Arrow)) {
                    self.bump_token();
                }
                self.push_text("->", false);
                self.scan_qname();
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

    /// One item of a deployment body: the `use <profile>` line or the
    /// `configure { ... }` block.
    pub(crate) fn scan_deploy_deployment_item(&mut self) {
        match self.peek_tok() {
            Some(Token::Ident(text)) if text == deploy::USE => {
                self.advance();
                self.take_name();
            }
            Some(Token::Ident(text)) if text == deploy::CONFIGURE => {
                self.advance();
                self.scan_body(BodyKind::DeploySettings);
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

    // --- .uc bodies ----------------------------------------------------------

    /// Rewrites the node stream into canonical `.deploy` order: a leading
    /// comment run (the file header) stays at the very front, then `import`
    /// declarations, then `application`, `profile`, and `deployment`
    /// declarations — each kind grouped in that order, source order kept
    /// within a kind. The [`Formatter::hoist_evt_items`] machinery with
    /// contextual-word ranks.
    pub(crate) fn hoist_deploy_items(&mut self) {
        fn rank(token: &Token<'_>) -> usize {
            match token {
                Token::Import => 0,
                Token::Ident(deploy::APPLICATION) => 1,
                Token::Ident(deploy::PROFILE) => 2,
                Token::Ident(deploy::DEPLOYMENT) => 3,
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

    /// Consumes and emits the `application <name> { ... }` block of a
    /// `.deploy` file.
    pub(crate) fn scan_deploy_application(&mut self) {
        self.begin_top_decl();
        self.advance();
        self.take_name();
        self.scan_body(BodyKind::DeployApplication);
    }

    /// Consumes and emits the `profile <name> { ... }` block of a
    /// `.deploy` file.
    pub(crate) fn scan_deploy_profile(&mut self) {
        self.begin_top_decl();
        self.advance();
        self.take_name();
        self.scan_body(BodyKind::DeployProfile);
    }

    /// Consumes and emits the `deployment <name> for <application> { ... }`
    /// block of a `.deploy` file.
    pub(crate) fn scan_deploy_deployment(&mut self) {
        self.begin_top_decl();
        self.advance();
        self.take_name();
        if self.take_if(|token| matches!(token, Token::Ident(text) if *text == deploy::FOR)) {
            self.take_name();
        }
        self.scan_body(BodyKind::DeployDeployment);
    }

    /// Unrecognized top-level tokens of a `.deploy` file: emit them on one
    /// line, stopping at the next `import` keyword or the contextual
    /// `application`/`profile`/`deployment` words (mirrors the parser's
    /// file-level recovery).
    pub(crate) fn scan_top_junk_deploy(&mut self) {
        self.begin_top_decl();
        loop {
            match self.peek_tok() {
                Some(token)
                    if !matches!(
                        token,
                        Token::Import
                            | Token::Ident(deploy::APPLICATION)
                            | Token::Ident(deploy::PROFILE)
                            | Token::Ident(deploy::DEPLOYMENT)
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
