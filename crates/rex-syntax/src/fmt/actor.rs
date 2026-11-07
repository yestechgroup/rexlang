//! The `.actor` formatter (also drives `.mox` inline `actors` blocks): actor/delegation/grant/never-both scanners.

use super::*;

impl<'src> Formatter<'src> {
    /// Like [`Formatter::run`], but for `.actor` files: the only recognized
    /// top-level constructs are `import` declarations and `actors` blocks.
    pub(crate) fn run_actors(&mut self) -> String {
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

    pub(crate) fn scan_actors_item(&mut self) {
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
            Some(Token::Obligation) => {
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
    pub(crate) fn scan_delegation_item(&mut self) {
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

    pub(crate) fn scan_grant_entry(&mut self) {
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
    pub(crate) fn scan_never_both(&mut self) {
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

    pub(crate) fn scan_actors(&mut self) {
        self.begin_top_decl();
        self.advance();
        self.take_name();
        self.scan_body(BodyKind::Actors);
    }

    /// Unrecognized top-level tokens of an `.actor` file: emit them on one
    /// line, stopping at the next `import`/`actors` keyword (mirrors the
    /// parser's declaration-level recovery for actor files).
    pub(crate) fn scan_top_junk_actors(&mut self) {
        self.begin_top_decl();
        loop {
            match self.peek_tok() {
                Some(token) if !matches!(token, Token::Import | Token::Actors) => self.advance(),
                _ => break,
            }
        }
        self.flush_line();
    }
}
