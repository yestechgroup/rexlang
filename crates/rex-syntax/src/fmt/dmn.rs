//! The `.dmn` formatter: model-member scanners, the canonical member
//! reorder, and the hoist.

use super::*;
use crate::dmn;

impl<'src> Formatter<'src> {
    /// Like [`Formatter::run_usecases`], but for `.dmn` files: the only
    /// recognized top-level constructs are `import` declarations and the
    /// contextual `decision_model` block, canonicalized into imports →
    /// model order by [`Formatter::hoist_dmn_items`].
    pub(crate) fn run_dmn(&mut self) -> String {
        self.hoist_dmn_items();
        loop {
            let front = self.front().cloned();
            match front {
                None => break,
                Some(Node::Comment { .. }) => self.advance(),
                Some(Node::Token(token, _)) => match token {
                    Token::Import => self.scan_dmn_import(),
                    Token::Ident(text) if text == dmn::DECISION_MODEL => self.scan_decision_model(),
                    _ => self.scan_top_junk_dmn(),
                },
            }
        }
        self.finish()
    }

    // --- node consumption ----------------------------------------------------

    /// Consumes and emits one `import "<path>";` declaration of a `.dmn`
    /// file: the tight import section of the `.actor` formatter, with the
    /// canonical trailing `;` emitted whether or not the source had one.
    pub(crate) fn scan_dmn_import(&mut self) {
        self.flush_line();
        self.flush_pending(0);
        self.advance();
        self.take_if(|token| matches!(token, Token::Str(_)));
        if matches!(self.peek_tok(), Some(Token::Other(';'))) {
            self.bump_token();
        }
        self.push_text(";", true);
        self.flush_line();
    }

    /// Consumes and emits the `decision_model <name> { ... }` declaration.
    /// The body's members are rewritten into the canonical order (see
    /// [`Formatter::reorder_dmn_items`]) before the linear scan.
    pub(crate) fn scan_decision_model(&mut self) {
        self.begin_top_decl();
        self.advance();
        self.take_name();
        self.reorder_dmn_items(dmn_model_member_rank);
        self.scan_body(BodyKind::DmnModel);
    }

    /// One metadata line: the contextual `namespace` word with its dotted
    /// name, the `version` keyword with its string, or a contextual
    /// `id`/`effective`/`jurisdiction` word with its string — each with
    /// its canonical `;`.
    pub(crate) fn scan_dmn_metadata_line(&mut self) {
        match self.peek_tok() {
            Some(Token::Version) => {
                self.advance();
                self.take_if(|token| matches!(token, Token::Str(_)));
            }
            Some(Token::Ident(text)) if text == dmn::NAMESPACE => {
                self.advance();
                self.scan_qname();
            }
            _ => {
                self.advance();
                self.take_if(|token| matches!(token, Token::Str(_)));
            }
        }
        self.scan_dmn_semicolon();
    }

    /// One member of a decision model body: a metadata line or one of the
    /// six blocks. The `knowledge` and `requirements` blocks have their
    /// members reordered before scanning (sources then models; the three
    /// requirement groups in canonical order).
    pub(crate) fn scan_dmn_model_item(&mut self) {
        match self.peek_tok() {
            Some(Token::Version) => self.scan_dmn_metadata_line(),
            Some(Token::Ident(text)) if text == dmn::NAMESPACE => self.scan_dmn_metadata_line(),
            Some(Token::Ident(text))
                if text == dmn::ID || text == dmn::EFFECTIVE || text == dmn::JURISDICTION =>
            {
                self.scan_dmn_metadata_line()
            }
            Some(Token::Ident(text)) if text == dmn::INPUTS => {
                self.advance();
                self.scan_body(BodyKind::DmnInputs);
            }
            Some(Token::Ident(text)) if text == dmn::KNOWLEDGE => {
                self.advance();
                self.reorder_dmn_items(dmn_knowledge_member_rank);
                self.scan_body(BodyKind::DmnKnowledge);
            }
            Some(Token::Ident(text)) if text == dmn::DECISIONS => {
                self.advance();
                self.scan_body(BodyKind::DmnDecisions);
            }
            Some(Token::Ident(text)) if text == dmn::REQUIREMENTS => {
                self.advance();
                self.reorder_dmn_items(dmn_requirement_group_rank);
                self.scan_body(BodyKind::DmnRequirements);
            }
            Some(Token::Ident(text)) if text == dmn::SERVICES => {
                self.advance();
                self.scan_body(BodyKind::DmnServices);
            }
            Some(Token::Ident(text)) if text == dmn::VIEWS => {
                self.advance();
                self.scan_body(BodyKind::DmnViews);
            }
            _ => self.scan_dmn_junk_member(),
        }
    }

    /// One `name ":" type_ref ";"` entry of an inputs/outputs/parameters
    /// block, with its canonical `;`.
    pub(crate) fn scan_dmn_typed_entry(&mut self) {
        match self.peek_tok() {
            Some(Token::Ident(_) | Token::IdentEscaped(_)) => {
                self.take_name();
                if matches!(self.peek_tok(), Some(Token::Other(':'))) {
                    self.bump_token();
                    self.push_text(":", true);
                    self.scan_qname();
                }
                self.scan_dmn_semicolon();
            }
            _ => self.scan_dmn_junk_member(),
        }
    }

    /// One item of a knowledge block: a `source` or `model` declaration.
    pub(crate) fn scan_dmn_knowledge_item(&mut self) {
        match self.peek_tok() {
            Some(Token::Ident(text)) if text == dmn::SOURCE => {
                self.advance();
                self.take_name();
                self.scan_body(BodyKind::DmnSource);
            }
            Some(Token::Ident(text)) if text == dmn::MODEL => {
                self.advance();
                self.take_name();
                self.scan_body(BodyKind::DmnBkm);
            }
            _ => self.scan_dmn_junk_member(),
        }
    }

    /// One property of a knowledge source: the display `name` or the
    /// `reference`, each with its canonical `;`.
    pub(crate) fn scan_dmn_source_prop(&mut self) {
        match self.peek_tok() {
            Some(Token::Ident(text)) if text == dmn::NAME || text == dmn::REFERENCE => {
                self.advance();
                self.take_if(|token| matches!(token, Token::Str(_)));
                self.scan_dmn_semicolon();
            }
            _ => self.scan_dmn_junk_member(),
        }
    }

    /// One clause of a model (BKM) body: `parameters`, `output`, or
    /// `logic`. Source order is kept (the formatter does not reorder a
    /// BKM's clauses).
    pub(crate) fn scan_dmn_bkm_item(&mut self) {
        match self.peek_tok() {
            Some(Token::Ident(text)) if text == dmn::PARAMETERS => {
                self.advance();
                self.scan_body(BodyKind::DmnInputs);
            }
            Some(Token::Ident(text)) if text == dmn::OUTPUT => {
                self.advance();
                self.scan_qname();
                self.scan_dmn_semicolon();
            }
            Some(Token::Ident(text)) if text == dmn::LOGIC => self.scan_dmn_logic(),
            _ => self.scan_dmn_junk_member(),
        }
    }

    /// One `decision` declaration of a decisions block.
    pub(crate) fn scan_dmn_decisions_item(&mut self) {
        match self.peek_tok() {
            Some(Token::Ident(text)) if text == dmn::DECISION => {
                self.advance();
                self.take_name();
                self.scan_body(BodyKind::DmnDecision);
            }
            _ => self.scan_dmn_junk_member(),
        }
    }

    /// One member of a decision body: the anonymous `output` line, the
    /// named `outputs` block, or the `logic` clause. Source order is kept.
    pub(crate) fn scan_dmn_decision_item(&mut self) {
        match self.peek_tok() {
            Some(Token::Ident(text)) if text == dmn::OUTPUT => {
                self.advance();
                self.scan_qname();
                self.scan_dmn_semicolon();
            }
            Some(Token::Ident(text)) if text == dmn::OUTPUTS => {
                self.advance();
                self.scan_body(BodyKind::DmnInputs);
            }
            Some(Token::Ident(text)) if text == dmn::LOGIC => self.scan_dmn_logic(),
            _ => self.scan_dmn_junk_member(),
        }
    }

    /// A `logic <kind>` clause: `table "<source>"`, `expression "<text>"`,
    /// or `invoke <model> { <bindings> }`. No `;` — the clause ends at its
    /// string or brace block.
    pub(crate) fn scan_dmn_logic(&mut self) {
        self.advance(); // the contextual `logic` word
        match self.peek_tok() {
            Some(Token::Ident(text)) if text == dmn::TABLE || text == dmn::EXPRESSION => {
                self.advance();
                self.take_if(|token| matches!(token, Token::Str(_)));
            }
            Some(Token::Ident(text)) if text == dmn::INVOKE => {
                self.advance();
                self.take_name();
                self.scan_body(BodyKind::DmnInvoke);
            }
            _ => {}
        }
    }

    /// One `name ":" "<expression>";` binding of an invoke body, with its
    /// canonical `;`.
    pub(crate) fn scan_dmn_invoke_binding(&mut self) {
        match self.peek_tok() {
            Some(Token::Ident(_) | Token::IdentEscaped(_)) => {
                self.take_name();
                if matches!(self.peek_tok(), Some(Token::Other(':'))) {
                    self.bump_token();
                    self.push_text(":", true);
                    self.take_if(|token| matches!(token, Token::Str(_)));
                }
                self.scan_dmn_semicolon();
            }
            _ => self.scan_dmn_junk_member(),
        }
    }

    /// One requirement group: the `information`/`knowledge`/`authority`
    /// word and its lines.
    pub(crate) fn scan_dmn_requirements_item(&mut self) {
        match self.peek_tok() {
            Some(Token::Ident(text))
                if text == dmn::INFORMATION || text == dmn::KNOWLEDGE || text == dmn::AUTHORITY =>
            {
                self.advance();
                self.scan_body(BodyKind::DmnRequirementLines);
            }
            _ => self.scan_dmn_junk_member(),
        }
    }

    /// One `<name> <connective> <name>;` requirement line, with its
    /// canonical `;`. Any identifier starts a line, mirroring the parser.
    pub(crate) fn scan_dmn_requirement_line(&mut self) {
        match self.peek_tok() {
            Some(Token::Ident(_) | Token::IdentEscaped(_)) => {
                self.take_name();
                if self.take_if(|token| {
                    matches!(
                        token,
                        Token::Ident(text)
                            if *text == dmn::REQUIRES
                                || *text == dmn::INVOKES
                                || *text == dmn::GOVERNED_BY
                    )
                }) {
                    self.take_name();
                }
                self.scan_dmn_semicolon();
            }
            _ => self.scan_dmn_junk_member(),
        }
    }

    /// One `service` declaration of a services block.
    pub(crate) fn scan_dmn_services_item(&mut self) {
        match self.peek_tok() {
            Some(Token::Ident(text)) if text == dmn::SERVICE => {
                self.advance();
                self.take_name();
                self.scan_body(BodyKind::DmnServiceItems);
            }
            _ => self.scan_dmn_junk_member(),
        }
    }

    /// One member of a service body: the `description` line or one of the
    /// three bare `{ name* }` lists.
    pub(crate) fn scan_dmn_service_item(&mut self) {
        match self.peek_tok() {
            Some(Token::Ident(text)) if text == dmn::DESCRIPTION => {
                self.advance();
                self.take_if(|token| matches!(token, Token::Str(_)));
                self.scan_dmn_semicolon();
            }
            Some(Token::Ident(text))
                if text == dmn::INPUTS || text == dmn::OUTPUTS || text == dmn::ENCAPSULATES =>
            {
                self.advance();
                self.scan_dmn_name_list(DMN_SERVICE_LIST_STOPS);
            }
            _ => self.scan_dmn_junk_member(),
        }
    }

    /// One `view` declaration of a views block.
    pub(crate) fn scan_dmn_views_item(&mut self) {
        match self.peek_tok() {
            Some(Token::Ident(text)) if text == dmn::VIEW => {
                self.advance();
                self.take_name();
                self.scan_body(BodyKind::DmnViewItems);
            }
            _ => self.scan_dmn_junk_member(),
        }
    }

    /// One member of a view body: the `description` line, the `show`
    /// list, or the `direction` line.
    pub(crate) fn scan_dmn_view_item(&mut self) {
        match self.peek_tok() {
            Some(Token::Ident(text)) if text == dmn::DESCRIPTION => {
                self.advance();
                self.take_if(|token| matches!(token, Token::Str(_)));
                self.scan_dmn_semicolon();
            }
            Some(Token::Ident(text)) if text == dmn::SHOW => {
                self.advance();
                self.scan_dmn_name_list(DMN_VIEW_LIST_STOPS);
            }
            Some(Token::Ident(text)) if text == dmn::DIRECTION => {
                self.advance();
                self.take_name();
                self.scan_dmn_semicolon();
            }
            _ => self.scan_dmn_junk_member(),
        }
    }

    /// A bare `{ name* }` list of a service or view body. The list stays
    /// inline on its line while it fits within
    /// [`DMN_NAME_LIST_BUDGET`] columns; past the budget — or when the
    /// source already listed names across lines, carried a comment or
    /// other token inside the braces — it renders one name per line at
    /// the next indent. Both shapes are fixpoints.
    pub(crate) fn scan_dmn_name_list(&mut self, stops: &'static [&'static str]) {
        if !matches!(self.front(), Some(Node::Token(Token::LBrace, _))) {
            return;
        }
        // Measure the list first: the names between the braces (a
        // next-member word ends an unterminated list, the `.evt`
        // events-list rule), whether the `}` was written, and whether the
        // interior spans lines or carries a comment or other token.
        let open_span = match self.front() {
            Some(Node::Token(_, span)) => *span,
            _ => return,
        };
        let mut names: Vec<&'src str> = Vec::new();
        let mut close_index = None;
        let mut interior_junk = false;
        let mut index = self.pos + 1;
        while let Some(node) = self.nodes.get(index) {
            match node {
                Node::Token(Token::RBrace, span) => {
                    close_index = Some((index, *span));
                    break;
                }
                Node::Token(Token::Ident(text) | Token::IdentEscaped(text), _) => {
                    if stops.contains(text) {
                        break;
                    }
                    names.push(text);
                    index += 1;
                }
                Node::Comment { .. } => {
                    interior_junk = true;
                    index += 1;
                }
                Node::Token(..) => {
                    interior_junk = true;
                    index += 1;
                }
            }
        }
        let region_end = close_index.map(|(_, span)| span.start).unwrap_or_else(|| {
            self.nodes
                .get(index)
                .map(|node| node.span().start)
                .unwrap_or(open_span.end)
        });
        let multiline = self.source[open_span.end..region_end.max(open_span.end)].contains('\n');
        let fits = self.line_indent * 4 + self.line.len() + 1 + names.join(" ").len() + 4
            <= DMN_NAME_LIST_BUDGET;
        if let Some((close, _)) = close_index.filter(|_| !interior_junk && !multiline && fits) {
            // Inline: the braces and names are consumed silently and
            // re-emitted at canonical spacing.
            self.bump_token();
            self.pos = close + 1;
            self.push_text("{", false);
            for name in &names {
                self.push_text(name, false);
            }
            self.push_text("}", false);
            return;
        }
        // Multiline: one name per line at the next indent.
        self.bump_token(); // the `{` joins the current line
        self.push_text("{", false);
        self.flush_line();
        self.indent += 1;
        loop {
            self.flush_pending(self.indent);
            match self.peek_tok() {
                Some(Token::RBrace) | None => break,
                Some(Token::Ident(text) | Token::IdentEscaped(text)) if stops.contains(&text) => {
                    break
                }
                Some(Token::Ident(_) | Token::IdentEscaped(_)) => {
                    self.advance();
                    self.flush_line();
                }
                Some(_) => {
                    self.advance();
                    self.scan_junk_until(|token| {
                        matches!(
                            token,
                            Token::RBrace | Token::Ident(_) | Token::IdentEscaped(_)
                        )
                    });
                    self.flush_line();
                }
            }
        }
        self.indent -= 1;
        self.take_if(|token| matches!(token, Token::RBrace));
        self.flush_line();
    }

    /// Consumes a source `;` silently and emits the canonical one (the
    /// `.evt` field-`;` rule).
    pub(crate) fn scan_dmn_semicolon(&mut self) {
        if matches!(self.peek_tok(), Some(Token::Other(';'))) {
            self.bump_token();
        }
        self.push_text(";", true);
    }

    /// Junk inside a `.dmn` body: consumes the front token (progress) and
    /// scans to the next token that could begin a proper member (mirrors
    /// the parser's member recovery).
    pub(crate) fn scan_dmn_junk_member(&mut self) {
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

    // --- reordering and hoisting ---------------------------------------------

    /// Rewrites the node stream of a `.dmn` body ahead of the formatter
    /// (positioned at the body's `{`) so its members appear in the rank
    /// order the `rank` function prescribes, source order kept within a
    /// rank. The same node-range technique the import hoists and the `.uc`
    /// member reorder use. A member's leading comments travel with it;
    /// junk chunks (parse-error regions) keep their relative order at the
    /// end.
    pub(crate) fn reorder_dmn_items(&mut self, rank: fn(&Token<'_>) -> Option<(usize, DmnExtent)>) {
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
                Some(ref token) => match rank(token) {
                    Some((member_rank, extent)) => {
                        segments.push((member_rank, Vec::new()));
                        self.dmn_member_extent(j, inner_end, extent)
                    }
                    None => {
                        // Junk: consume through the chunk up to the next
                        // member start so it moves as one (comments-only
                        // chunks too). The chunk never reaches past the
                        // body's closing brace.
                        segments.push((usize::MAX, Vec::new()));
                        let mut k = j;
                        k += 1;
                        while k < inner_end {
                            match self.nodes.get(k) {
                                Some(Node::Token(token, _)) if rank(token).is_some() => break,
                                _ => k += 1,
                            }
                        }
                        k.min(inner_end)
                    }
                },
                None => {
                    segments.push((usize::MAX, Vec::new()));
                    inner_end
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

    /// The exclusive end of the `.dmn` body member whose leading token
    /// sits at `start`. A statement member runs word + value + optional
    /// `;`; a named block runs word + name + balanced braces; a bare
    /// block runs word + balanced braces (missing braces consume only
    /// what the shape needs, so the enclosing body's `}` is never
    /// swallowed).
    pub(crate) fn dmn_member_extent(
        &self,
        start: usize,
        inner_end: usize,
        extent: DmnExtent,
    ) -> usize {
        let is_ident = |index: usize| {
            matches!(
                self.nodes.get(index),
                Some(Node::Token(Token::Ident(_) | Token::IdentEscaped(_), _))
            )
        };
        let mut i = start + 1; // past the member-leading token
        match extent {
            DmnExtent::Statement => {
                if matches!(self.nodes.get(i), Some(Node::Token(Token::Str(_), _))) {
                    i += 1;
                } else if is_ident(i) {
                    // A dotted qualified name (tight dots).
                    i += 1;
                    while matches!(self.nodes.get(i), Some(Node::Token(Token::Dot, _))) {
                        i += 1;
                        if is_ident(i) {
                            i += 1;
                        } else {
                            break;
                        }
                    }
                }
                if matches!(self.nodes.get(i), Some(Node::Token(Token::Other(';'), _))) {
                    i += 1;
                }
                i
            }
            DmnExtent::Named => {
                if is_ident(i) {
                    i += 1;
                }
                self.dmn_brace_extent(i, inner_end)
            }
            DmnExtent::Block => self.dmn_brace_extent(i, inner_end),
        }
    }

    /// The exclusive end of the balanced `{ ... }` block at `index`, or
    /// `index` itself when no brace is there (or the block was never
    /// closed).
    fn dmn_brace_extent(&self, index: usize, inner_end: usize) -> usize {
        if !matches!(self.nodes.get(index), Some(Node::Token(Token::LBrace, _))) {
            return index;
        }
        let mut i = index;
        let mut depth = 0usize;
        while i < inner_end {
            match self.nodes.get(i) {
                Some(Node::Token(Token::LBrace, _)) => depth += 1,
                Some(Node::Token(Token::RBrace, _)) => {
                    depth -= 1;
                    if depth == 0 {
                        return i + 1;
                    }
                }
                _ => {}
            }
            i += 1;
        }
        inner_end
    }

    /// The `.dmn` hoist: identical to [`Formatter::hoist_cmmn_items`], with
    /// the contextual `decision_model` word ranking after the imports.
    pub(crate) fn hoist_dmn_items(&mut self) {
        fn rank(token: &Token<'_>) -> usize {
            match token {
                Token::Import => 0,
                Token::Ident(text) if *text == dmn::DECISION_MODEL => 1,
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

    /// Unrecognized top-level tokens of a `.dmn` file: emit them on one
    /// line, stopping at the next `import` keyword or the contextual
    /// `decision_model` word (mirrors the parser's file-level recovery).
    pub(crate) fn scan_top_junk_dmn(&mut self) {
        self.begin_top_decl();
        loop {
            match self.peek_tok() {
                Some(token)
                    if !matches!(token, Token::Import)
                        && !matches!(token, Token::Ident(text) if text == dmn::DECISION_MODEL) =>
                {
                    self.advance()
                }
                _ => break,
            }
        }
        self.flush_line();
    }
}

/// The shape of a reordered `.dmn` body member; decides how far its node
/// extent runs (see [`Formatter::dmn_member_extent`]).
#[derive(Debug, Clone, Copy)]
pub(crate) enum DmnExtent {
    /// A statement member (metadata): word + value + optional `;`.
    Statement,
    /// A named block (knowledge source / model): word + name + braces.
    Named,
    /// A bare block (inputs, decisions, ...): word + braces.
    Block,
}

/// The canonical rank and extent shape of a `.dmn` model body member:
/// metadata lines first (`namespace`, `id`, `version`, `effective`,
/// `jurisdiction` — the formatter's canonical spelling order), then the
/// six blocks in their canonical order.
fn dmn_model_member_rank(token: &Token<'_>) -> Option<(usize, DmnExtent)> {
    match token {
        Token::Ident(text) => {
            let (rank, extent) = match *text {
                dmn::NAMESPACE => (0, DmnExtent::Statement),
                dmn::ID => (1, DmnExtent::Statement),
                dmn::EFFECTIVE => (3, DmnExtent::Statement),
                dmn::JURISDICTION => (4, DmnExtent::Statement),
                dmn::INPUTS => (10, DmnExtent::Block),
                dmn::KNOWLEDGE => (20, DmnExtent::Block),
                dmn::DECISIONS => (30, DmnExtent::Block),
                dmn::REQUIREMENTS => (40, DmnExtent::Block),
                dmn::SERVICES => (50, DmnExtent::Block),
                dmn::VIEWS => (60, DmnExtent::Block),
                _ => return None,
            };
            Some((rank, extent))
        }
        Token::Version => Some((2, DmnExtent::Statement)),
        _ => None,
    }
}

/// The canonical rank of a `.dmn` knowledge block member: sources first,
/// then models (BKMs).
fn dmn_knowledge_member_rank(token: &Token<'_>) -> Option<(usize, DmnExtent)> {
    match token {
        Token::Ident(text) if *text == dmn::SOURCE => Some((0, DmnExtent::Named)),
        Token::Ident(text) if *text == dmn::MODEL => Some((1, DmnExtent::Named)),
        _ => None,
    }
}

/// The canonical rank of a `.dmn` requirements block member: information,
/// knowledge, authority.
fn dmn_requirement_group_rank(token: &Token<'_>) -> Option<(usize, DmnExtent)> {
    match token {
        Token::Ident(text) if *text == dmn::INFORMATION => Some((0, DmnExtent::Block)),
        Token::Ident(text) if *text == dmn::KNOWLEDGE => Some((1, DmnExtent::Block)),
        Token::Ident(text) if *text == dmn::AUTHORITY => Some((2, DmnExtent::Block)),
        _ => None,
    }
}

/// The canonical column budget of a `.dmn` bare name list: a service's or
/// view's `{ name* }` list stays inline on its line while it fits, and
/// renders one name per line past it (or when the source already listed
/// names across lines or carried a comment inside the braces).
const DMN_NAME_LIST_BUDGET: usize = 100;

/// The words that end a service body's bare name list: the words that can
/// begin the next service member (the `.evt` events-list rule; escaped
/// forms are names).
const DMN_SERVICE_LIST_STOPS: &[&str] = &[
    dmn::DESCRIPTION,
    dmn::INPUTS,
    dmn::OUTPUTS,
    dmn::ENCAPSULATES,
];

/// The words that end a view body's bare name list: the words that can
/// begin the next view member (escaped forms are names).
const DMN_VIEW_LIST_STOPS: &[&str] = &[dmn::DESCRIPTION, dmn::SHOW, dmn::DIRECTION];
