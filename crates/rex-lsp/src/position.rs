//! Byte-offset ↔ LSP `Position` mapping over one source text.
//!
//! LSP positions are `(line, character)` pairs where `character` counts
//! UTF-16 code units. rexlang spans are byte offsets. [`PositionMap`]
//! converts between the two, clamping out-of-range inputs instead of
//! panicking (editor traffic is routinely out of sync with the file).

use rex_driver::Span;
use tower_lsp::lsp_types::{Position, Range};

/// One indexed line: byte range of the content (excluding the line
/// terminator) and its UTF-16 length.
#[derive(Debug, Clone, Copy)]
struct LineInfo {
    /// Byte offset of the line's first character.
    start: usize,
    /// Byte offset one past the line's last content byte (i.e. the index of
    /// the `\r` in a CRLF pair, of the `\n`, or of end-of-text).
    content_end: usize,
}

/// A line index over one source text, for byte-offset ↔ `Position` math.
#[derive(Debug, Clone)]
pub struct PositionMap {
    text: String,
    lines: Vec<LineInfo>,
}

/// The UTF-16 length of a string slice.
fn utf16_len(text: &str) -> u32 {
    text.chars().map(|character| character.len_utf16() as u32).sum::<u32>()
}

/// The greatest `offset <= max` that is a `char` boundary of `text`.
fn floor_to_char_boundary(text: &str, mut offset: usize) -> usize {
    offset = offset.min(text.len());
    while offset > 0 && !text.is_char_boundary(offset) {
        offset -= 1;
    }
    offset
}

impl PositionMap {
    /// Indexes `text`. Cheap: one pass over the text.
    pub fn new(text: &str) -> Self {
        let mut lines = Vec::new();
        let mut start = 0;
        for line in text.split_inclusive('\n') {
            let trimmed = line.strip_suffix('\n').unwrap_or(line);
            let trimmed = trimmed.strip_suffix('\r').unwrap_or(trimmed);
            let content_end = start + trimmed.len();
            lines.push(LineInfo {
                start,
                content_end,
            });
            start += line.len();
        }
        // A trailing fragment without a terminator is still a line; empty
        // input gets a single empty line so line 0 always exists.
        if start < text.len() || lines.is_empty() {
            lines.push(LineInfo {
                start,
                content_end: text.len(),
            });
        }
        PositionMap {
            text: text.to_string(),
            lines,
        }
    }

    /// Converts an LSP `Position` to a byte offset. Out-of-range lines and
    /// characters are clamped: a character past the end of its line maps to
    /// the line's content end (before any `\r\n`), and a line past the end
    /// of the text maps into the last line.
    pub fn offset_for(&self, position: Position) -> usize {
        let line = &self.lines[(position.line as usize).min(self.lines.len() - 1)];
        let mut units = 0u32;
        for (index, character) in self.text[line.start..line.content_end].char_indices() {
            if units >= position.character {
                return line.start + index;
            }
            let next = units + character.len_utf16() as u32;
            if next > position.character {
                // The target lands inside `character` (e.g. mid-surrogate
                // pair): clamp to the character's start.
                return line.start + index;
            }
            units = next;
        }
        // Past the end of the line's content (or an empty line): clamp.
        line.content_end
    }

    /// Converts a byte offset to an LSP `Position`. Offsets past the end of
    /// the text clamp to the last position; offsets inside or at the end of
    /// a line terminator map to the end of that line's content.
    pub fn position_for(&self, offset: usize) -> Position {
        let offset = floor_to_char_boundary(&self.text, offset);
        let line_index = match self.lines.binary_search_by(|line| line.start.cmp(&offset)) {
            Ok(index) => index,
            Err(insertion) => insertion.saturating_sub(1),
        };
        let line = &self.lines[line_index];
        let effective = offset.min(line.content_end);
        Position {
            line: line_index as u32,
            character: utf16_len(&self.text[line.start..effective]),
        }
    }

    /// The text this map was built from.
    pub fn text(&self) -> &str {
        &self.text
    }

    /// Converts a byte span to an LSP range.
    pub fn range_for(&self, span: Span) -> Range {
        Range {
            start: self.position_for(span.start),
            end: self.position_for(span.end),
        }
    }

    /// Converts an LSP range to a byte span.
    pub fn span_for(&self, range: Range) -> Span {
        let start = self.offset_for(range.start);
        let end = self.offset_for(range.end);
        (start..end.max(start)).into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pos(line: u32, character: u32) -> Position {
        Position { line, character }
    }

    fn range(start_line: u32, start_char: u32, end_line: u32, end_char: u32) -> Range {
        Range {
            start: pos(start_line, start_char),
            end: pos(end_line, end_char),
        }
    }

    /// Byte span of the `nth` (0-based) occurrence of `needle`.
    fn span_of(source: &str, needle: &str, nth: usize) -> Span {
        let (start, _) = source
            .match_indices(needle)
            .nth(nth)
            .unwrap_or_else(|| panic!("occurrence {nth} of '{needle}' not found"));
        (start..start + needle.len()).into()
    }

    // --- ASCII -------------------------------------------------------------

    #[test]
    fn ascii_positions_round_trip() {
        let text = "package demo\n\nclass Book {\n    String title\n}\n";
        let map = PositionMap::new(text);
        assert_eq!(map.text(), text);

        // Line 0 is "package demo"; line 2 starts at 13; "title" on line 3.
        assert_eq!(map.offset_for(pos(0, 0)), 0);
        assert_eq!(map.offset_for(pos(0, 8)), 8); // 'd' of demo
        assert_eq!(map.position_for(0), pos(0, 0));
        assert_eq!(map.position_for(8), pos(0, 8));
        assert_eq!(map.position_for(12), pos(0, 12)); // last char of line 0

        let title = span_of(text, "title", 0);
        assert_eq!(map.range_for(title), range(3, 11, 3, 16));
        assert_eq!(map.span_for(range(3, 11, 3, 16)), title);
    }

    #[test]
    fn offset_at_line_start_and_end() {
        let text = "ab\ncd\n";
        let map = PositionMap::new(text);
        assert_eq!(map.offset_for(pos(1, 0)), 3);
        // End of line 0 → the offset of the '\n' itself.
        assert_eq!(map.offset_for(pos(0, 2)), 2);
        // End of the last line (trailing '\n') → end of text.
        assert_eq!(map.offset_for(pos(1, 2)), 5);
        assert_eq!(map.position_for(3), pos(1, 0));
        assert_eq!(map.position_for(6), pos(1, 2));
    }

    // --- Multibyte ---------------------------------------------------------

    #[test]
    fn multibyte_latin_counts_characters_in_utf16_units() {
        // 'é' is 2 bytes but 1 UTF-16 code unit.
        let text = "café\nnext";
        let map = PositionMap::new(text);
        assert_eq!(map.position_for(4), pos(0, 3)); // offset of 'é'
        assert_eq!(map.position_for(5), pos(0, 4)); // just past 'é'
        assert_eq!(map.offset_for(pos(0, 3)), 3);
        assert_eq!(map.offset_for(pos(0, 4)), 5);
        assert_eq!(map.position_for(5), pos(0, 4)); // the '\n' clamps to line end
        assert_eq!(map.position_for(6), pos(1, 0));
    }

    #[test]
    fn cjk_characters_count_as_one_utf16_unit() {
        // '世' is 3 bytes, 1 UTF-16 code unit.
        let text = "ab世def";
        let map = PositionMap::new(text);
        assert_eq!(map.position_for(2), pos(0, 2));
        assert_eq!(map.position_for(5), pos(0, 3)); // end of '世'
        assert_eq!(map.position_for(6), pos(0, 4));
        assert_eq!(map.offset_for(pos(0, 2)), 2);
        assert_eq!(map.offset_for(pos(0, 3)), 5);
        assert_eq!(map.offset_for(pos(0, 6)), 8);
    }

    #[test]
    fn emoji_surrogate_pairs_count_as_two_utf16_units() {
        // '😀' is 4 bytes and 2 UTF-16 code units (a surrogate pair).
        let text = "a😀b";
        let map = PositionMap::new(text);
        assert_eq!(map.position_for(1), pos(0, 1)); // start of 😀
        assert_eq!(map.position_for(5), pos(0, 3)); // start of 'b'
        assert_eq!(map.position_for(6), pos(0, 4)); // end of text
        assert_eq!(map.offset_for(pos(0, 1)), 1);
        assert_eq!(map.offset_for(pos(0, 3)), 5);
        // A character index that splits the surrogate pair clamps to the
        // start of the pair — i.e. the emoji itself.
        assert_eq!(map.offset_for(pos(0, 2)), 1);
    }

    // --- CRLF --------------------------------------------------------------

    #[test]
    fn crlf_line_content_excludes_the_carriage_return() {
        let text = "alpha\r\nbeta\r\n";
        let map = PositionMap::new(text);
        assert_eq!(map.position_for(5), pos(0, 5)); // last char of "alpha"
        assert_eq!(map.position_for(6), pos(0, 5)); // the '\r' clamps to line end
        assert_eq!(map.position_for(7), pos(1, 0)); // the '\n' starts line 1
        // A character index at/past the line end maps to the '\r' offset…
        assert_eq!(map.offset_for(pos(0, 5)), 5);
        assert_eq!(map.offset_for(pos(0, 6)), 5);
        assert_eq!(map.offset_for(pos(0, 100)), 5);
        assert_eq!(map.offset_for(pos(1, 100)), 11); // end of "beta"
    }

    // --- Empty / degenerate -------------------------------------------------

    #[test]
    fn empty_text_maps_everything_to_the_origin() {
        let map = PositionMap::new("");
        assert_eq!(map.offset_for(pos(0, 0)), 0);
        assert_eq!(map.position_for(0), pos(0, 0));
        assert_eq!(map.position_for(42), pos(0, 0));
        assert_eq!(map.offset_for(pos(3, 7)), 0);
    }

    #[test]
    fn out_of_bounds_inputs_clamp_instead_of_panicking() {
        let text = "one\ntwo";
        let map = PositionMap::new(text);
        // Line clamping.
        assert_eq!(map.offset_for(pos(99, 0)), 4);
        assert_eq!(map.offset_for(pos(99, 99)), 7);
        // Character clamping within a line.
        assert_eq!(map.offset_for(pos(0, 99)), 3);
        // Offset clamping.
        assert_eq!(map.position_for(999), pos(1, 3));
    }

    // --- Round trips --------------------------------------------------------

    #[test]
    fn positions_and_offsets_round_trip_across_a_mixed_document() {
        let text = "package nz.example.library\r\n\nenum Mood { Happy = 0 }\n// café 😀\n";
        let map = PositionMap::new(text);
        // Every byte offset that is neither half of a CRLF pair round-trips;
        // both terminator bytes intentionally collapse to the line-end
        // position, so they map back to the '\r'.
        let in_crlf = |offset: usize| {
            let bytes = text.as_bytes();
            (bytes[offset] == b'\r' && bytes.get(offset + 1) == Some(&b'\n'))
                || (offset > 0 && bytes[offset - 1] == b'\r' && bytes[offset] == b'\n')
        };
        for (offset, _) in text.char_indices() {
            if in_crlf(offset) {
                continue;
            }
            let position = map.position_for(offset);
            assert_eq!(map.offset_for(position), offset, "round trip at {offset}");
        }
        // And every position derived from a span converts back exactly.
        let happy = span_of(text, "Happy", 0);
        assert_eq!(map.span_for(map.range_for(happy)), happy);
    }

    #[test]
    fn range_and_span_converters_agree_on_multibyte_spans() {
        let text = "class Café { String café }\n";
        let map = PositionMap::new(text);
        let café = span_of(text, "String café", 0).start + "String ".len();
        let café: Span = (café..café + "café".len()).into();
        let range = map.range_for(café);
        assert_eq!(range.start.line, 0);
        assert_eq!(map.span_for(range), café);
    }
}
