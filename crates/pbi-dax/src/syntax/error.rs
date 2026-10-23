//! Parse errors with spans and line/column helpers.

use std::fmt;

use super::ast::Span;

/// A lexing or parsing error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError {
    /// Human-readable description, e.g. `unexpected ')'`.
    pub message: String,
    /// Offending source range (zero-width at end of input for EOF errors).
    pub span: Span,
    /// What the parser would have accepted here, e.g. `["expression"]` or
    /// `["','", "')'"]`.
    pub expected: Vec<String>,
}

impl ParseError {
    pub fn new(message: impl Into<String>, span: Span) -> Self {
        Self {
            message: message.into(),
            span,
            expected: Vec::new(),
        }
    }

    pub fn with_expected<I, S>(mut self, expected: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.expected = expected.into_iter().map(Into::into).collect();
        self
    }

    /// 1-based `(line, column)` of the error start in `src`.
    pub fn line_col(&self, src: &str) -> (usize, usize) {
        line_col(src, self.span.start)
    }

    /// `line L, column C: message (expected …)`.
    pub fn render(&self, src: &str) -> String {
        let (line, col) = self.line_col(src);
        format!("line {line}, column {col}: {self}")
    }
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)?;
        match self.expected.as_slice() {
            [] => Ok(()),
            [one] => write!(f, ", expected {one}"),
            [init @ .., last] => write!(f, ", expected {} or {last}", init.join(", ")),
        }
    }
}

impl std::error::Error for ParseError {}

/// 1-based `(line, column)` for a byte `offset` in `src`. Columns count
/// Unicode scalar values; lines are split on `\n`. Offsets past the end clamp
/// to the end; offsets inside a multi-byte char round down.
pub fn line_col(src: &str, offset: usize) -> (usize, usize) {
    let mut offset = offset.min(src.len());
    while offset > 0 && !src.is_char_boundary(offset) {
        offset -= 1;
    }
    let before = src.get(..offset).unwrap_or("");
    let line = before.matches('\n').count() + 1;
    let line_start = before.rfind('\n').map_or(0, |i| i + 1);
    let col = before.get(line_start..).unwrap_or("").chars().count() + 1;
    (line, col)
}
