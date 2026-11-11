//! rex-strings — the shared lexical leaf for string-literal escape
//! decoding.
//!
//! Every rexlang surface that lexes `"…"` string literals and then
//! decodes the token payload borrows [`unescape_string`] from here, so
//! the escape contract is written once: exactly `\"` and `\\` decode;
//! every other escape — and a lone trailing backslash — is kept
//! verbatim rather than rejected. The crate is deliberately
//! zero-dependency so parser-front crates can share it without
//! layering entanglement.

/// Decodes a string-literal token payload (quotes already stripped by
/// the lexer). Exactly `\"` and `\\` decode; every other escape — and
/// a lone trailing backslash — is kept verbatim rather than rejected.
pub fn unescape_string(raw: &str) -> String {
    let mut value = String::with_capacity(raw.len());
    let mut chars = raw.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('"') => value.push('"'),
                Some('\\') => value.push('\\'),
                // Unknown escapes are kept verbatim rather than rejected.
                Some(other) => {
                    value.push('\\');
                    value.push(other);
                }
                None => value.push('\\'),
            }
        } else {
            value.push(c);
        }
    }
    value
}

#[cfg(test)]
mod tests {
    use super::unescape_string;

    /// The token payload is what the lexer hands over with the quotes
    /// already stripped, so every case here operates on inner text.
    #[test]
    fn empty_and_plain_text_pass_through() {
        assert_eq!(unescape_string(""), "");
        assert_eq!(unescape_string("plain text"), "plain text");
        assert_eq!(
            unescape_string("no escapes at all 123"),
            "no escapes at all 123"
        );
    }

    /// Exactly two escapes decode: `\"` and `\\`.
    #[test]
    fn the_two_known_escapes_decode() {
        assert_eq!(unescape_string(r#"a\"b"#), "a\"b");
        assert_eq!(unescape_string(r"a\\b"), "a\\b");
        // The pinned consumer case: both escapes in one literal.
        assert_eq!(unescape_string(r#"a\"b\\c"#), "a\"b\\c");
        assert_eq!(unescape_string(r#"\"""#), "\"\"");
        assert_eq!(unescape_string(r"\\\\"), "\\\\");
    }

    /// Every other escape — including the C-family familiars `\n`,
    /// `\t`, `\r` — keeps BOTH characters: the backslash is retained,
    /// never dropped and never turned into a control character.
    #[test]
    fn unknown_escapes_are_kept_verbatim() {
        assert_eq!(unescape_string(r"\n"), "\\n");
        assert_eq!(unescape_string(r"\t"), "\\t");
        assert_eq!(unescape_string(r"\r"), "\\r");
        // `\'` is NOT a known escape: single-quote is an ordinary
        // character in these literals.
        assert_eq!(unescape_string(r"\'"), "\\'");
        assert_eq!(unescape_string(r"\0"), "\\0");
        assert_eq!(unescape_string(r"\x41"), "\\x41");
        // An escaped backslash followed by an unknown escape still
        // decodes the first pair only.
        assert_eq!(unescape_string(r"\\n"), "\\n");
        // The rex-syntax import-path pin: `\"`/`\\` decode, `\n` stays.
        assert_eq!(
            unescape_string(r#"lib\"quote\\back\n"#),
            "lib\"quote\\back\\n"
        );
    }

    /// Multibyte text is copied through unchanged, including when it
    /// directly follows a backslash (the escape consumer is char-based).
    #[test]
    fn multibyte_text_passes_through() {
        assert_eq!(unescape_string("héllo → wörld"), "héllo → wörld");
        assert_eq!(unescape_string("\\é"), "\\é");
        assert_eq!(unescape_string("日本語"), "日本語");
    }

    /// Escapes at both string boundaries, and the trailing lone
    /// backslash (nothing after it to consume) keeps itself.
    #[test]
    fn escapes_at_the_boundaries() {
        assert_eq!(unescape_string(r"\nstart"), "\\nstart");
        assert_eq!(unescape_string(r"\\leading"), "\\leading");
        assert_eq!(unescape_string(r#"end\""#), "end\"");
        assert_eq!(unescape_string(r"end\\"), "end\\");
        assert_eq!(unescape_string(r"\\"), "\\");
        // The `None` arm: a backslash with no successor stays one.
        assert_eq!(unescape_string("end\\"), "end\\");
        assert_eq!(unescape_string("\\"), "\\");
        // Pair-wise scan: `\\` decodes first, then the following `\"`
        // decodes as an escaped quote (not an unknown escape).
        assert_eq!(unescape_string(r#"\\\""#), "\\\"");
    }
}
