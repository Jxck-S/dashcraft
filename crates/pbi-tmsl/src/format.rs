//! Output layout: key order, indentation, line endings, encoding.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use serde_json::ser::{PrettyFormatter, Serializer};

use crate::encoding::TextEncoding;

/// Order of keys inside the objects the writer builds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum KeyOrder {
    /// The Tabular Object Model serializer order used by Analysis Services,
    /// Tabular Editor, SSDT and PBIT `DataModelSchema` (`name` first,
    /// scalar properties, child collections, `annotations` last).
    #[default]
    Tom,
    /// `name` first, then all other keys sorted by ordinal (byte) order, as
    /// written by Power BI Desktop for PBIP `model.bim`.
    Alphabetical,
}

/// Line ending used between JSON lines.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LineEnding {
    #[default]
    Lf,
    CrLf,
}

impl LineEnding {
    pub fn as_str(self) -> &'static str {
        match self {
            LineEnding::Lf => "\n",
            LineEnding::CrLf => "\r\n",
        }
    }
}

/// How a TMSL document is laid out on disk. Readers detect it
/// ([`crate::LoadedTmsl::format`]) so that writing an unedited model back
/// reproduces the original bytes whenever the original used one of the
/// supported key orders.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TmslFormat {
    pub key_order: KeyOrder,
    /// One indentation level (`"  "` like Tabular Editor and Power BI
    /// Desktop). Empty means compact single-line JSON.
    pub indent: String,
    pub line_ending: LineEnding,
    /// Whether the document ends with a line ending.
    pub trailing_newline: bool,
    /// Used by [`crate::write_bytes`] only.
    pub encoding: TextEncoding,
    /// Byte-order mark; used by [`crate::write_bytes`] only.
    pub bom: bool,
}

impl Default for TmslFormat {
    /// `model.bim` style: TOM key order, two-space indent, LF, UTF-8 without
    /// BOM, no trailing newline.
    fn default() -> Self {
        Self {
            key_order: KeyOrder::Tom,
            indent: "  ".to_owned(),
            line_ending: LineEnding::Lf,
            trailing_newline: false,
            encoding: TextEncoding::Utf8,
            bom: false,
        }
    }
}

impl TmslFormat {
    /// PBIT `DataModelSchema` style: like [`TmslFormat::default`] but
    /// UTF-16LE without BOM, which is how Power BI Desktop stores the entry.
    pub fn data_model_schema() -> Self {
        Self {
            encoding: TextEncoding::Utf16Le,
            ..Self::default()
        }
    }
}

/// Detects indentation, line ending and trailing newline of JSON text.
pub(crate) fn detect_layout(text: &str) -> (String, LineEnding, bool) {
    let line_ending = if text.contains("\r\n") {
        LineEnding::CrLf
    } else {
        LineEnding::Lf
    };
    let trailing_newline = text.ends_with('\n');
    let indent = match text.trim_end().find('\n') {
        None => String::new(),
        Some(pos) => {
            let rest = text.get(pos + 1..).unwrap_or_default();
            let unit: String = rest
                .chars()
                .take_while(|c| matches!(c, ' ' | '\t'))
                .collect();
            if unit.is_empty() {
                "  ".to_owned()
            } else {
                unit
            }
        }
    };
    (indent, line_ending, trailing_newline)
}

/// Serializes `value` with the layout of `format` (encoding and BOM are
/// applied by the caller).
pub(crate) fn render(value: &Value, format: &TmslFormat) -> String {
    let mut buf = Vec::new();
    let result = if format.indent.is_empty() {
        serde_json::to_writer(&mut buf, value)
    } else {
        let formatter = PrettyFormatter::with_indent(format.indent.as_bytes());
        let mut serializer = Serializer::with_formatter(&mut buf, formatter);
        serde::Serialize::serialize(value, &mut serializer)
    };
    // Serializing a `Value` into memory cannot fail and always yields UTF-8.
    if result.is_err() {
        buf.clear();
    }
    let mut text = String::from_utf8(buf).unwrap_or_default();
    if format.line_ending == LineEnding::CrLf {
        // JSON strings escape line breaks, so every `\n` is layout.
        text = text.replace('\n', "\r\n");
    }
    if format.trailing_newline {
        text.push_str(format.line_ending.as_str());
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout_detection() {
        assert_eq!(
            detect_layout("{\r\n    \"a\": 1\r\n}"),
            ("    ".to_owned(), LineEnding::CrLf, false)
        );
        assert_eq!(
            detect_layout("{\n\t\"a\": 1\n}\n"),
            ("\t".to_owned(), LineEnding::Lf, true)
        );
        assert_eq!(
            detect_layout("{\"a\":1}"),
            (String::new(), LineEnding::Lf, false)
        );
    }

    #[test]
    fn render_honours_layout() {
        let value = serde_json::json!({"a": [1, {"b": "x\ny"}], "c": []});
        let crlf = TmslFormat {
            line_ending: LineEnding::CrLf,
            trailing_newline: true,
            ..TmslFormat::default()
        };
        assert_eq!(
            render(&value, &crlf),
            "{\r\n  \"a\": [\r\n    1,\r\n    {\r\n      \"b\": \"x\\ny\"\r\n    }\r\n  ],\r\n  \"c\": []\r\n}\r\n"
        );
        let compact = TmslFormat {
            indent: String::new(),
            ..TmslFormat::default()
        };
        assert_eq!(render(&value, &compact), r#"{"a":[1,{"b":"x\ny"}],"c":[]}"#);
    }
}
