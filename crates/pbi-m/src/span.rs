//! Source locations.

use std::fmt;

/// A half-open byte range `start..end` into the source text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, PartialOrd, Ord)]
pub struct Span {
    pub start: usize,
    pub end: usize,
}

impl Span {
    pub const fn new(start: usize, end: usize) -> Self {
        Self { start, end }
    }

    /// The smallest span covering both `self` and `other`.
    #[must_use]
    pub fn to(self, other: Span) -> Span {
        Span {
            start: self.start.min(other.start),
            end: self.end.max(other.end),
        }
    }

    pub const fn len(&self) -> usize {
        self.end.saturating_sub(self.start)
    }

    pub const fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The source text covered by this span, or `""` if the span is out of range or not on
    /// character boundaries.
    pub fn slice<'a>(&self, src: &'a str) -> &'a str {
        src.get(self.start..self.end).unwrap_or("")
    }
}

impl fmt::Display for Span {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}..{}", self.start, self.end)
    }
}

/// Converts a byte offset into a 1-based `(line, column)` pair.
///
/// Columns count Unicode scalar values. `CR LF` counts as a single line break; `CR`, `LF`,
/// `U+0085`, `U+2028` and `U+2029` are each line breaks, as in the M lexical grammar. Offsets past
/// the end of `src` (or inside a multi-byte character) are clamped.
pub fn line_col(src: &str, offset: usize) -> (usize, usize) {
    let mut offset = offset.min(src.len());
    while offset > 0 && !src.is_char_boundary(offset) {
        offset -= 1;
    }
    let prefix = src.get(..offset).unwrap_or("");
    let mut line = 1;
    let mut col = 1;
    let mut prev_cr = false;
    for c in prefix.chars() {
        match c {
            '\n' if prev_cr => {
                // Second half of CR LF: the line break was already counted.
            }
            '\r' | '\n' | '\u{0085}' | '\u{2028}' | '\u{2029}' => {
                line += 1;
                col = 1;
            }
            _ => col += 1,
        }
        prev_cr = c == '\r';
    }
    (line, col)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_col_counts_lines_and_chars() {
        let src = "ab\r\ncé\nx";
        assert_eq!(line_col(src, 0), (1, 1));
        assert_eq!(line_col(src, 2), (1, 3));
        assert_eq!(line_col(src, 4), (2, 1));
        // 'é' is two bytes; offset 7 is after it.
        assert_eq!(line_col(src, 7), (2, 3));
        assert_eq!(line_col(src, 8), (3, 1));
        // Clamped and boundary-safe.
        assert_eq!(line_col(src, 6), (2, 2));
        assert_eq!(line_col(src, 1000), (3, 2));
    }
}
