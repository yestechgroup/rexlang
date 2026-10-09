//! Comment-preserving canonical formatter for `.ifml` sources.
//!
//! `rexlang fmt` dispatches `.ifml` files here. Pest drops comments during
//! parsing, so the formatter does not reprint from the IR: it scans the
//! source into raw tokens (strings and comments byte-exact) and re-emits
//! them under structural layout rules:
//!
//! - declaration blocks (`view`/`domain`/`actor`/`action`/`module`/
//!   `container`/`component` headers) always expand, one statement per line,
//!   4-space indent;
//! - value and binding blocks (`position: { … }`, `use "M" { … }`,
//!   `navigate("V", { … })`, `chart bar { … }`, module `input`/`output`,
//!   `params { … }`) render inline when the whole statement fits in 85
//!   columns, has no nested braces, and contains no nested `on … ->` event
//!   handlers — otherwise they expand;
//! - within a run of consecutive `column "Label" -> …` statements the labels
//!   are padded so the `->` arrows align;
//! - interior blank lines are preserved, collapsed to at most one; top-level
//!   declarations are always separated by exactly one blank line; a comment
//!   group attaches to the declaration it precedes; the document starts
//!   without a blank line and ends with exactly one `\n`.
//!
//! The layout is pinned by the canonical fixture: formatting
//! `tests/conformance/ifml/app.ifml` must reproduce it byte-for-byte.

const INLINE_WIDTH: usize = 80;

#[derive(Debug, Clone, PartialEq, Eq)]
enum Tok {
    Ident(String),
    Str(String),
    Num(String),
    Punct(String),
    LineComment(String),
    BlockComment(String),
}

#[derive(Debug, Clone)]
struct TokInfo {
    tok: Tok,
    /// Blank lines immediately before this token in the source (capped).
    blank_before: u32,
    /// Any line break immediately before this token.
    newline_before: bool,
}

fn tokenize(source: &str) -> Vec<TokInfo> {
    let mut toks = Vec::new();
    let mut chars = source.chars().peekable();
    let mut blank_before: u32 = 0;
    let mut newline_before = false;
    let mut line_has_content = false;
    while let Some(&c) = chars.peek() {
        match c {
            ' ' | '\t' | '\r' => {
                chars.next();
            }
            '\n' => {
                chars.next();
                newline_before = true;
                if !line_has_content {
                    blank_before = (blank_before + 1).min(2);
                }
                line_has_content = false;
            }
            '/' if chars.clone().nth(1) == Some('/') => {
                let mut text = String::new();
                while let Some(&n) = chars.peek() {
                    if n == '\n' {
                        break;
                    }
                    text.push(n);
                    chars.next();
                }
                toks.push(TokInfo {
                    tok: Tok::LineComment(text),
                    blank_before,
                    newline_before,
                });
                blank_before = 0;
                newline_before = false;
                line_has_content = true;
            }
            '/' if chars.clone().nth(1) == Some('*') => {
                chars.next();
                chars.next();
                let mut text = String::from("/*");
                while let Some(&n) = chars.peek() {
                    chars.next();
                    text.push(n);
                    if n == '\n' {
                        newline_before = true;
                    }
                    if n == '*' && chars.peek() == Some(&'/') {
                        chars.next();
                        text.push('/');
                        break;
                    }
                }
                toks.push(TokInfo {
                    tok: Tok::BlockComment(text),
                    blank_before,
                    newline_before,
                });
                blank_before = 0;
                newline_before = false;
                line_has_content = true;
            }
            '"' => {
                let mut text = String::new();
                text.push('"');
                chars.next();
                while let Some(&n) = chars.peek() {
                    chars.next();
                    text.push(n);
                    if n == '\\' {
                        if let Some(&e) = chars.peek() {
                            chars.next();
                            text.push(e);
                        }
                    } else if n == '"' {
                        break;
                    }
                }
                toks.push(TokInfo {
                    tok: Tok::Str(text),
                    blank_before,
                    newline_before,
                });
                blank_before = 0;
                newline_before = false;
                line_has_content = true;
            }
            c if c.is_ascii_alphabetic() || c == '_' => {
                let mut text = String::new();
                while let Some(&n) = chars.peek() {
                    if n.is_ascii_alphanumeric() || n == '_' {
                        text.push(n);
                        chars.next();
                    } else {
                        break;
                    }
                }
                toks.push(TokInfo {
                    tok: Tok::Ident(text),
                    blank_before,
                    newline_before,
                });
                blank_before = 0;
                newline_before = false;
                line_has_content = true;
            }
            c if c.is_ascii_digit()
                || (c == '-' && chars.clone().nth(1).is_some_and(|n| n.is_ascii_digit())) =>
            {
                let mut text = String::new();
                if c == '-' {
                    text.push('-');
                    chars.next();
                }
                while let Some(&n) = chars.peek() {
                    let digit = n.is_ascii_digit();
                    let fractional =
                        n == '.' && chars.clone().nth(1).is_some_and(|m| m.is_ascii_digit());
                    if digit || fractional {
                        text.push(n);
                        chars.next();
                    } else {
                        break;
                    }
                }
                toks.push(TokInfo {
                    tok: Tok::Num(text),
                    blank_before,
                    newline_before,
                });
                blank_before = 0;
                newline_before = false;
                line_has_content = true;
            }
            _ => {
                chars.next();
                let two: String = std::iter::once(c).chain(chars.peek().copied()).collect();
                let punct = match two.as_str() {
                    "->" | "==" | "!=" | "<=" | ">=" | "&&" | "||" | "~=" | "!~" => {
                        chars.next();
                        two
                    }
                    _ => c.to_string(),
                };
                toks.push(TokInfo {
                    tok: Tok::Punct(punct),
                    blank_before,
                    newline_before,
                });
                blank_before = 0;
                newline_before = false;
                line_has_content = true;
            }
        }
    }
    toks
}

fn text_of(tok: &Tok) -> &str {
    match tok {
        Tok::Ident(s) | Tok::Str(s) | Tok::Num(s) | Tok::Punct(s) => s,
        Tok::LineComment(_) | Tok::BlockComment(_) => "",
    }
}

fn is_comment(tok: &Tok) -> bool {
    matches!(tok, Tok::LineComment(_) | Tok::BlockComment(_))
}

fn is_open(tok: &Tok) -> bool {
    matches!(tok, Tok::Punct(p) if matches!(p.as_str(), "(" | "["))
}

fn is_close(tok: &Tok) -> bool {
    matches!(tok, Tok::Punct(p) if matches!(p.as_str(), ")" | "]"))
}

/// Number of spaces required between two adjacent tokens on one line.
fn space_between(left: &Tok, right: &Tok) -> usize {
    // Never space before these.
    if matches!(right, Tok::Punct(p) if matches!(p.as_str(), "," | ";" | ")" | "]" | "." | ":" | "("))
    {
        return 0;
    }
    // Never space after these.
    if matches!(left, Tok::Punct(p) if matches!(p.as_str(), "(" | "[" | "." | "!")) {
        return 0;
    }
    1
}

/// Prefix length of a statement, up to (not including) its `->`, used to
/// align consecutive `column` statements.
fn arrow_prefix_len(toks: &[TokInfo]) -> Option<usize> {
    let mut len = 0usize;
    let mut prev: Option<&Tok> = None;
    for info in toks {
        if info.tok == Tok::Punct("->".to_string()) {
            return if prev.is_some() { Some(len) } else { None };
        }
        if is_comment(&info.tok) {
            return None;
        }
        if let Some(p) = prev {
            len += space_between(p, &info.tok);
        }
        len += text_of(&info.tok).chars().count();
        prev = Some(&info.tok);
    }
    None
}

struct Printer {
    out: String,
    indent: usize,
    toks: Vec<TokInfo>,
    pos: usize,
    at_line_start: bool,
    prev: Option<Tok>,
}

impl Printer {
    fn new(toks: Vec<TokInfo>) -> Self {
        Printer {
            out: String::new(),
            indent: 0,
            toks,
            pos: 0,
            at_line_start: true,
            prev: None,
        }
    }

    fn peek(&self) -> Option<&TokInfo> {
        self.toks.get(self.pos)
    }

    fn current_line_len(&self) -> usize {
        self.out.rsplit('\n').next().unwrap_or("").chars().count()
    }

    fn push_text(&mut self, text: &str) {
        self.out.push_str(text);
        self.at_line_start = false;
        self.prev = None;
    }

    fn emit_token(&mut self, info: &TokInfo) {
        let was_at_line_start = self.at_line_start;
        self.emit_indent();
        if is_comment(&info.tok) && !was_at_line_start && !info.newline_before {
            self.push_text(" ");
        } else {
            let spaces = match &self.prev {
                // Comments carry their own placement.
                _ if is_comment(&info.tok) => 0,
                Some(prev) => space_between(prev, &info.tok),
                None => 0,
            };
            for _ in 0..spaces {
                self.push_text(" ");
            }
        }
        let text = match &info.tok {
            Tok::LineComment(t) | Tok::BlockComment(t) => t.clone(),
            other => text_of(other).to_string(),
        };
        self.push_text(&text);
        self.prev = Some(info.tok.clone());
        self.pos += 1;
    }

    fn emit_newline(&mut self) {
        self.out.push('\n');
        self.at_line_start = true;
        self.prev = None;
    }

    fn emit_indent(&mut self) {
        if self.at_line_start {
            for _ in 0..self.indent {
                self.push_text("    ");
            }
        }
    }

    /// Whether the balanced block starting at the `{` at `self.pos` may
    /// render inline given the current line, and its inner text.
    fn inline_block_render(&self) -> Option<String> {
        if self.peek().map(|t| &t.tok) != Some(&Tok::Punct("{".to_string())) {
            return None;
        }
        let mut inner = String::new();
        let mut stmt_start = true;
        let mut prev: Option<Tok> = None;
        let mut i = self.pos + 1;
        while i < self.toks.len() {
            let info = &self.toks[i];
            match &info.tok {
                Tok::Punct(p) if p == "{" => return None,
                Tok::Punct(p) if p == "}" => break,
                t if is_comment(t) => return None,
                Tok::Ident(name) if stmt_start && name == "on" => return None,
                _ => {}
            }
            if let Tok::Punct(p) = &info.tok {
                stmt_start = p == ";" || p == ",";
            } else {
                stmt_start = false;
            }
            let spaces = match &prev {
                None => 0,
                Some(p) => space_between(p, &info.tok),
            };
            for _ in 0..spaces {
                inner.push(' ');
            }
            inner.push_str(text_of(&info.tok));
            prev = Some(info.tok.clone());
            i += 1;
        }
        if i >= self.toks.len() {
            return None;
        }
        let mut total = self.current_line_len();
        if !self.at_line_start {
            total += 1; // space before "{"
        }
        total += inner.chars().count() + 3; // "{ " and " }"
        for info in self.toks[i + 1..].iter() {
            match &info.tok {
                Tok::Punct(p) if p == ")" || p == ";" => total += 1,
                _ => break,
            }
        }
        if total > INLINE_WIDTH {
            return None;
        }
        Some(inner)
    }

    /// Index of the `}` closing the `{` at `open_pos` (no nested braces can
    /// occur: inline candidates reject them, expanded blocks recurse).
    fn closing_brace(&self, open_pos: usize) -> usize {
        self.toks[open_pos..]
            .iter()
            .position(|t| t.tok == Tok::Punct("}".to_string()))
            .map(|p| open_pos + p)
            .unwrap_or(self.toks.len().saturating_sub(1))
    }

    /// Keywords that always get a separating blank line before them inside
    /// a block (canonical grouping), regardless of the source.
    fn needs_structural_blank(tok: &Tok) -> bool {
        matches!(tok, Tok::Ident(name) if matches!(name.as_str(), "component" | "container" | "use" | "field" | "column"))
    }

    /// Keywords that always begin a new statement: when one appears on a
    /// fresh source line mid-statement, the statement ends there.
    fn starts_statement(tok: &Tok) -> bool {
        matches!(tok, Tok::Ident(name) if matches!(
            name.as_str(),
            "view" | "domain" | "actor" | "action" | "module" | "container" | "component"
                | "field" | "use" | "on" | "column" | "import" | "params" | "input" | "output"
                | "chart"
        ))
    }

    /// Prints one statement: through its terminating `;` or depth-0 `,`, or
    /// through a block and any immediate trailing punctuation. Returns
    /// without consuming a `}`/`)`/`]` that belongs to the enclosing
    /// context.
    fn print_statement(&mut self) {
        let mut first = true;
        let mut emitted = false;
        let mut declaration = false;
        let mut depth = 0usize;
        loop {
            let Some(info) = self.peek().cloned() else {
                return;
            };
            if emitted {
                if self.at_line_start {
                    // Canonical blank-line policy: a structural declaration
                    // gets one blank before it; otherwise source blanks are
                    // preserved, collapsed to one.
                    if Self::needs_structural_blank(&info.tok) || info.blank_before > 0 {
                        self.out.push('\n');
                    }
                } else if info.newline_before
                    && depth == 0
                    && !is_comment(&info.tok)
                    && Self::starts_statement(&info.tok)
                {
                    // A statement starter on a fresh source line ends this
                    // statement.
                    self.emit_newline();
                    return;
                }
            }
            match &info.tok {
                t if is_comment(t) => {
                    // An own-line comment stays on its own line even when a
                    // preceding inline block left us mid-line; only genuine
                    // trailing comments (no source newline) attach.
                    if !self.at_line_start && info.newline_before {
                        // Undo a caller's already-emitted indentation when
                        // the current line carries nothing but it; a line
                        // with real content breaks before the comment.
                        let line_start = self.out.rfind('\n').map(|i| i + 1).unwrap_or(0);
                        if self.out[line_start..]
                            .chars()
                            .all(|c| c == ' ' || c == '\t')
                        {
                            self.out.truncate(line_start);
                            self.at_line_start = true;
                            self.prev = None;
                        } else {
                            self.emit_newline();
                        }
                    }
                    self.emit_token(&info);
                    self.emit_newline();
                    emitted = true;
                }
                Tok::Punct(p) if p == ";" => {
                    self.emit_token(&info);
                    // A trailing comment on the same line stays trailing.
                    if let Some(next) = self.peek() {
                        if is_comment(&next.tok) && !next.newline_before {
                            let next = next.clone();
                            self.emit_token(&next);
                        }
                    }
                    self.emit_newline();
                    return;
                }
                Tok::Punct(p) if p == "," && depth == 0 => {
                    self.emit_token(&info);
                    self.emit_newline();
                    return;
                }
                Tok::Punct(p) if p == "}" && depth == 0 => return,
                Tok::Punct(p) if p == ")" && depth == 0 => return,
                Tok::Punct(p) if p == "{" => {
                    // Call-argument binding blocks (`navigate("V", { … })`)
                    // and declaration headers always expand; other value
                    // blocks may go inline when they fit.
                    let force_expand =
                        declaration || matches!(&self.prev, Some(Tok::Punct(p)) if p == ",");
                    if !force_expand {
                        if let Some(inner) = self.inline_block_render() {
                            self.push_text(" { ");
                            self.push_text(&inner);
                            self.push_text(" }");
                            self.pos = self.closing_brace(self.pos) + 1;
                            self.prev = Some(Tok::Punct("}".to_string()));
                            first = false;
                            emitted = true;
                            continue;
                        }
                    }
                    self.push_text(" {");
                    self.emit_newline();
                    self.indent += 1;
                    self.pos += 1;
                    let mut inner_first = true;
                    while let Some(next) = self.peek().cloned() {
                        if next.tok == Tok::Punct("}".to_string()) {
                            if !self.at_line_start {
                                self.emit_newline();
                            }
                            break;
                        }
                        if !inner_first
                            && (next.blank_before > 0 || Self::needs_structural_blank(&next.tok))
                        {
                            self.out.push('\n');
                        }
                        self.emit_indent();
                        self.print_statement();
                        inner_first = false;
                    }
                    self.indent -= 1;
                    self.emit_indent();
                    if let Some(close) = self.peek().cloned() {
                        if close.tok == Tok::Punct("}".to_string()) {
                            self.emit_token(&close);
                        }
                    }
                    // Continuation on the closing line: `});` / `};`.
                    loop {
                        let Some(next) = self.peek().cloned() else {
                            self.emit_newline();
                            return;
                        };
                        match &next.tok {
                            Tok::Punct(p) if p == ";" => {
                                self.emit_token(&next);
                                self.emit_newline();
                                return;
                            }
                            Tok::Punct(p) if p == ")" => self.emit_token(&next),
                            _ => {
                                self.emit_newline();
                                return;
                            }
                        }
                    }
                }
                Tok::Ident(name)
                    if first
                        && matches!(
                            name.as_str(),
                            "view"
                                | "domain"
                                | "actor"
                                | "action"
                                | "module"
                                | "container"
                                | "component"
                        ) =>
                {
                    declaration = true;
                    self.emit_token(&info);
                    first = false;
                    emitted = true;
                }
                Tok::Ident(name) if first && name == "column" => {
                    self.print_column_run();
                    return;
                }
                _ => {
                    if is_open(&info.tok) {
                        depth += 1;
                    } else if is_close(&info.tok) {
                        depth = depth.saturating_sub(1);
                    }
                    self.emit_token(&info);
                    first = false;
                    emitted = true;
                }
            }
        }
    }

    /// A run of consecutive `column "Label" -> …` statements: pad each
    /// statement's prefix so the `->` arrows align.
    fn print_column_run(&mut self) {
        let start = self.pos;
        let mut end = start;
        let mut max_prefix = 0usize;
        while end < self.toks.len() {
            match &self.toks[end].tok {
                Tok::Ident(name) if name == "column" => {
                    let mut stmt_end = end;
                    while stmt_end < self.toks.len()
                        && self.toks[stmt_end].tok != Tok::Punct(";".to_string())
                    {
                        stmt_end += 1;
                    }
                    if let Some(prefix) = arrow_prefix_len(&self.toks[end..=stmt_end]) {
                        max_prefix = max_prefix.max(prefix);
                    }
                    end = stmt_end + 1;
                    if self.toks.get(end).is_some_and(|n| n.blank_before > 0) {
                        break;
                    }
                }
                _ => break,
            }
        }
        let mut first_in_run = true;
        while self.pos < end {
            if !first_in_run {
                self.emit_indent();
            }
            while self.pos < end && self.toks[self.pos].tok != Tok::Punct(";".to_string()) {
                if self.toks[self.pos].tok == Tok::Punct("->".to_string()) {
                    let line = self.current_line_len();
                    let target = self.indent * 4 + max_prefix + 1;
                    for _ in line..target {
                        self.push_text(" ");
                    }
                }
                let info = self.peek().cloned().expect("pos < end");
                self.emit_token(&info);
            }
            if self.pos < end {
                let semi = self.peek().cloned().expect("pos < end");
                self.emit_token(&semi);
            }
            self.emit_newline();
            first_in_run = false;
            // Blank handling after the run is the caller's: the enclosing
            // loop applies the same blank policy it uses for any statement.
        }
    }

    fn print_document(&mut self) {
        let mut first = true;
        let mut prev_was_import = false;
        while self.peek().is_some() {
            // Comments at document level always print together with the
            // declaration they precede (print_statement consumes them).
            // Every document-level statement gets one blank line before it,
            // except consecutive `import` statements, which group tight.
            let starts_import = self.toks[self.pos..]
                .iter()
                .find(|info| !is_comment(&info.tok))
                .is_some_and(|info| info.tok == Tok::Ident("import".to_string()));
            if !first && !(starts_import && prev_was_import) {
                self.out.push('\n');
            }
            self.emit_indent();
            self.print_statement();
            first = false;
            prev_was_import = starts_import;
        }
        while self.out.ends_with('\n') {
            self.out.pop();
        }
        self.out.push('\n');
    }
}

/// Formats `.ifml` source into canonical form.
pub fn format_ifml(source: &str) -> String {
    let mut printer = Printer::new(tokenize(source));
    printer.print_document();
    printer.out
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &str = include_str!("../../../tests/conformance/ifml/app.ifml");

    #[test]
    fn canonical_fixture_round_trips_byte_identically() {
        assert_eq!(format_ifml(FIXTURE), FIXTURE);
    }

    #[test]
    fn formatting_is_idempotent() {
        assert_eq!(format_ifml(&format_ifml(FIXTURE)), format_ifml(FIXTURE));
    }

    #[test]
    fn scrambled_whitespace_normalizes() {
        let formatted = format_ifml(
            "view   \"A\"  {\n      label \"x\";\ncomponent \"c\" {\n            type:   list;\n        }\n  }\n",
        );
        assert_eq!(
            formatted,
            "view \"A\" {\n    label \"x\";\n\n    component \"c\" {\n        type: list;\n    }\n}\n"
        );
        assert_eq!(format_ifml(&formatted), formatted);
    }

    #[test]
    fn line_comments_survive_and_attach() {
        let src =
            "// header\n\nview \"A\" {\n    // about the label\n    label \"x\"; // trailing\n}\n";
        assert_eq!(format_ifml(src), src);
    }

    #[test]
    fn block_comments_survive() {
        let src = "/* license */\nview \"A\" {\n    /* inner */\n    label \"x\";\n}\n";
        assert_eq!(format_ifml(src), src);
    }

    #[test]
    fn strings_and_escapes_are_byte_exact() {
        let src = "view \"A\" {\n    label \"say \\\"hi\\\"\";\n}\n";
        assert_eq!(format_ifml(src), src);
    }

    #[test]
    fn inline_blocks_stay_inline_and_wide_ones_expand() {
        let src = "view \"A\" {\n    position: { x: 100; y: 200 };\n\n    component \"c\" {\n        type: list;\n\n        on select(row) -> navigate(\"V\", {\n            customerId: row.id\n        });\n    }\n}\n";
        assert_eq!(format_ifml(src), src);
    }

    #[test]
    fn call_argument_binding_blocks_always_expand() {
        let src = "view \"A\" {\n    component \"c\" {\n        on select(row) -> navigate(\"V\", {\n            customerId: row.id\n        });\n    }\n}\n";
        assert_eq!(format_ifml(src), src);
    }

    #[test]
    fn module_input_output_render_inline() {
        let src = "module \"Pagination\" {\n    input { page: Int, pageSize: Int }\n    output { total: Int }\n\n    component \"pager\" {\n        type: list;\n    }\n}\n";
        assert_eq!(format_ifml(src), src);
    }

    #[test]
    fn column_arrows_align_within_a_run() {
        let src = "view \"R\" {\n    container \"D\" {\n        component \"rows\" {\n            type: table;\n\n            column \"Name\" -> field Customer.name;\n            column \"Status\" -> lookup Customer.status via labels;\n            column \"Score\" -> expr score(x) * 100;\n        }\n    }\n}\n";
        let formatted = format_ifml(src);
        assert!(formatted.contains("column \"Name\"   -> field Customer.name;"));
        assert!(formatted.contains("column \"Status\" -> lookup Customer.status via labels;"));
        assert!(formatted.contains("column \"Score\"  -> expr score(x) * 100;"));
    }

    #[test]
    fn blank_lines_collapse_to_one() {
        let src = "view \"A\" {\n    label \"x\";\n\n\n\n    label \"y\";\n}\n";
        assert_eq!(
            format_ifml(src),
            "view \"A\" {\n    label \"x\";\n\n    label \"y\";\n}\n"
        );
    }
}
