//! Value rules shared by the reader and the writer: expression blocks,
//! scalar text values and object names.
//!
//! # Expression de-indentation rule
//!
//! A default property or expression property (`measure X =`, `source =`,
//! `annotation A =`) holds either
//!
//! - a **single-line** value: the text after `=`, trimmed;
//! - an **indented block**: the following lines that are blank or indented
//!   deeper than the object's properties. The block's common leading
//!   whitespace (the longest common tab/space prefix of its non-blank lines)
//!   is removed from every line, whitespace-only lines become empty, blank
//!   lines after the last non-blank line are not part of the expression and
//!   trailing whitespace at the very end is dropped. Relative indentation
//!   inside the block (e.g. four spaces before an M step) is kept. Text
//!   after `=` on the header line, if any, becomes the first line;
//! - a **fenced block** (`= ```` ``` ```` … `` ``` ``): every line up to the
//!   closing ```` ``` ```` line, verbatim, with the indentation of the closing
//!   delimiter removed from each line.
//!
//! Lines are joined with `\n` whatever the file's line endings are; stray
//! carriage returns at the end of a line count as part of the line ending.
//!
//! The writer indents block lines two levels below the line that declares
//! the expression (as Power BI Desktop does), leaves empty lines empty, and
//! falls back to a fenced block whenever the indented form would not decode
//! back to the same text (leading/trailing whitespace, whitespace-only
//! lines, common indentation). So model → TMDL → model is stable.

use crate::lexer::leading_ws_len;

/// Longest common tab/space prefix of the non-blank `lines`.
fn common_ws_prefix<'a>(lines: &[&'a str]) -> &'a str {
    let mut prefix: Option<&'a str> = None;
    for line in lines {
        if line.trim().is_empty() {
            continue;
        }
        let ws = line.get(..leading_ws_len(line)).unwrap_or_default();
        prefix = Some(match prefix {
            None => ws,
            Some(p) => {
                let n = p
                    .bytes()
                    .zip(ws.bytes())
                    .take_while(|(a, b)| a == b)
                    .count();
                p.get(..n).unwrap_or_default()
            }
        });
    }
    prefix.unwrap_or_default()
}

/// Decodes the lines of an indented expression block (see module docs).
pub(crate) fn decode_indented(lines: &[&str]) -> String {
    let prefix = common_ws_prefix(lines).len();
    let mut out = String::new();
    for (i, line) in lines.iter().enumerate() {
        if i > 0 {
            out.push('\n');
        }
        if line.trim().is_empty() {
            continue;
        }
        let line = line.trim_end_matches('\r');
        out.push_str(line.get(prefix..).unwrap_or(line));
    }
    let end = out.trim_end().len();
    out.truncate(end);
    out
}

/// Decodes the content lines of a fenced block; `closing_indent` is the
/// whitespace before the closing delimiter.
pub(crate) fn decode_fenced(lines: &[&str], closing_indent: &str) -> String {
    let mut out = String::new();
    for (i, line) in lines.iter().enumerate() {
        if i > 0 {
            out.push('\n');
        }
        let line = line.trim_end_matches('\r');
        let n = line
            .bytes()
            .zip(closing_indent.bytes())
            .take_while(|(a, b)| a == b)
            .count();
        out.push_str(line.get(n..).unwrap_or(line));
    }
    out
}

/// `true` if `line` closes a fenced block.
pub(crate) fn is_fence_line(line: &str) -> bool {
    line.trim() == "```"
}

/// How the writer represents an expression.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum EncodedExpr {
    /// On the header line after `= `.
    Inline(String),
    /// Indented block; lines are unindented (the writer adds indentation to
    /// non-empty lines).
    Block(Vec<String>),
    /// Fenced block; lines are unindented.
    Fenced(Vec<String>),
}

/// Chooses the representation of `expr` (see module docs). `needs_block_
/// for_empty`: an empty value must not be written as a bare `=` because the
/// node has children that would otherwise be read as the expression.
pub(crate) fn encode_expression(expr: &str, needs_block_for_empty: bool) -> EncodedExpr {
    let expr = expr.replace("\r\n", "\n");
    if expr.is_empty() {
        return if needs_block_for_empty {
            EncodedExpr::Fenced(Vec::new())
        } else {
            EncodedExpr::Inline(String::new())
        };
    }
    if !expr.contains('\n') && expr.trim() == expr && !expr.starts_with("```") {
        return EncodedExpr::Inline(expr);
    }
    let lines: Vec<&str> = expr.split('\n').collect();
    if decode_indented(&lines) == expr {
        return EncodedExpr::Block(lines.iter().map(|l| (*l).to_owned()).collect());
    }
    if !lines.iter().any(|l| is_fence_line(l)) {
        return EncodedExpr::Fenced(lines.iter().map(|l| (*l).to_owned()).collect());
    }
    EncodedExpr::Block(lines.iter().map(|l| (*l).to_owned()).collect())
}

/// `true` if `inner` (text between the outer quotes) has every `"` doubled.
fn is_escaped(inner: &str) -> bool {
    let mut chars = inner.chars();
    while let Some(c) = chars.next() {
        if c == '"' && chars.next() != Some('"') {
            return false;
        }
    }
    true
}

/// Decodes a scalar property value (`name: value`): surrounding whitespace is
/// trimmed and a `"…"` value with `""` escapes is unquoted. Anything else is
/// returned as written.
pub fn unquote_text(raw: &str) -> String {
    let t = raw.trim();
    if t.len() >= 2
        && t.starts_with('"')
        && t.ends_with('"')
        && let Some(inner) = t.get(1..t.len() - 1)
        && is_escaped(inner)
    {
        return inner.replace("\"\"", "\"");
    }
    t.to_owned()
}

/// Encodes a scalar property value so that [`unquote_text`] returns it.
/// Line feeds cannot be represented and are replaced by spaces.
pub fn quote_text(value: &str) -> String {
    let v = value.replace('\n', " ");
    if !v.is_empty() && v.trim() == v && unquote_text(&v) == v {
        v
    } else {
        format!("\"{}\"", v.replace('"', "\"\""))
    }
}

/// Removes TMDL name quoting from a raw name token (`'Web Sessions'` →
/// `Web Sessions`, `'It''s'` → `It's`). Unquoted and dotted names are returned
/// as written.
pub fn unquote_name(raw: &str) -> String {
    let t = raw.trim();
    let Some(body) = t.strip_prefix('\'') else {
        return t.to_owned();
    };
    let bytes = body.as_bytes();
    let mut i = 0;
    while let Some(&c) = bytes.get(i) {
        if c == b'\'' {
            if bytes.get(i + 1) == Some(&b'\'') {
                i += 2;
                continue;
            }
            if i + 1 == bytes.len() {
                return body.get(..i).unwrap_or_default().replace("''", "'");
            }
            return t.to_owned();
        }
        i += 1;
    }
    t.to_owned()
}

/// Quotes an object name for TMDL (line feeds, which TMDL cannot
/// represent in a name, are replaced by spaces).
pub fn quote_name(name: &str) -> String {
    if name.contains('\n') {
        let clean = name.replace('\n', " ");
        return pbi_model::tmdl_name(&clean).into_owned();
    }
    pbi_model::tmdl_name(name).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn indented_blocks_strip_common_indentation() {
        let lines = [
            "\t\t\t\tlet",
            "\t\t\t\t    Source = 1",
            "\t\t\t\t",
            "",
            "\t\t\t\tin",
            "\t\t\t\t    Source  ",
        ];
        assert_eq!(
            decode_indented(&lines),
            "let\n    Source = 1\n\n\nin\n    Source"
        );
        assert_eq!(decode_indented(&["\t\tx", "\t\t\ty"]), "x\n\ty");
    }

    #[test]
    fn fenced_blocks_strip_closing_indentation() {
        let lines = ["\t\t\tvar x = 1", "\t\t\t\treturn x  ", "", "\t  y"];
        assert_eq!(
            decode_fenced(&lines, "\t\t\t"),
            "var x = 1\n\treturn x  \n\n  y"
        );
    }

    #[test]
    fn encoder_picks_a_lossless_form() {
        assert_eq!(
            encode_expression("SUM(x)", false),
            EncodedExpr::Inline("SUM(x)".into())
        );
        assert_eq!(
            encode_expression("", false),
            EncodedExpr::Inline(String::new())
        );
        assert_eq!(encode_expression("", true), EncodedExpr::Fenced(vec![]));
        assert_eq!(
            encode_expression("let\n    a = 1\nin\n    a", false),
            EncodedExpr::Block(vec![
                "let".into(),
                "    a = 1".into(),
                "in".into(),
                "    a".into()
            ])
        );
        for text in ["  x", "x  ", "\tx\n\ty", "a\n  \nb", "```x", "a\n\n"] {
            let encoded = encode_expression(text, false);
            match &encoded {
                EncodedExpr::Fenced(lines) => {
                    let refs: Vec<&str> = lines.iter().map(String::as_str).collect();
                    assert_eq!(decode_fenced(&refs, ""), text);
                }
                EncodedExpr::Block(lines) => {
                    let refs: Vec<&str> = lines.iter().map(String::as_str).collect();
                    assert_eq!(decode_indented(&refs), text);
                }
                EncodedExpr::Inline(_) => panic!("{text:?} must not be inline"),
            }
        }
    }

    #[test]
    fn text_values_round_trip() {
        assert_eq!(unquote_text(" 0.0%;-0.0%;0.0% "), "0.0%;-0.0%;0.0%");
        assert_eq!(unquote_text("\" My \"\"Amazing\"\" \""), " My \"Amazing\" ");
        assert_eq!(unquote_text("\"$\"#,0"), "\"$\"#,0");
        assert_eq!(unquote_text("\"a\"b\""), "\"a\"b\"");
        for v in ["x", " x", "", "\"q\"", "\"$\"#,0", "a\"b", "\"a\"b\""] {
            assert_eq!(unquote_text(&quote_text(v)), v, "value {v:?}");
        }
        assert_eq!(quote_text("Outlet"), "Outlet");
        assert_eq!(quote_text("a\nb"), "a b");
        assert_eq!(quote_text("a\r"), "\"a\r\"");
    }

    #[test]
    fn names_unquote() {
        assert_eq!(unquote_name("'Web Sessions'"), "Web Sessions");
        assert_eq!(unquote_name("'It''s'"), "It's");
        assert_eq!(unquote_name("Orders"), "Orders");
        assert_eq!(unquote_name("'a'.'b'"), "'a'.'b'");
        assert_eq!(unquote_name("''"), "");
        assert_eq!(quote_name("Web Sessions"), "'Web Sessions'");
        assert_eq!(quote_name("a\nb"), "'a b'");
        assert_eq!(quote_name("a\tb"), "'a\tb'");
    }
}
