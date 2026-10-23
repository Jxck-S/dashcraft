//! Parse errors.

use std::fmt;

use crate::span::{Span, line_col};

/// A lexical or syntactic error in M source text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError {
    /// Human-readable description, e.g. ``expected `in`, found `)` ``.
    pub message: String,
    /// The offending source range (byte offsets).
    pub span: Span,
    /// What the parser would have accepted at `span`, if known (e.g. `` `in` ``, `"expression"`).
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

    #[must_use]
    pub fn with_expected(mut self, expected: Vec<String>) -> Self {
        self.expected = expected;
        self
    }

    /// 1-based `(line, column)` of the start of the error in `src`.
    pub fn line_col(&self, src: &str) -> (usize, usize) {
        line_col(src, self.span.start)
    }

    /// Formats the error as `line:col: message` against the source it was produced from.
    pub fn render(&self, src: &str) -> String {
        let (line, col) = self.line_col(src);
        format!("{line}:{col}: {}", self.message)
    }
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} (at byte {})", self.message, self.span.start)
    }
}

impl std::error::Error for ParseError {}
