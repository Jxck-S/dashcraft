//! Text decoding with encoding detection (BOM or NUL-byte pattern).
//!
//! Power BI stores several parts as UTF-16LE (`Version`, `Report/Layout`,
//! `DataModelSchema`, `Settings`, ...), usually without a BOM. Readers must
//! never assume an encoding from the file name.

use encoding_rs::{UTF_16BE, UTF_16LE};

/// Encoding detected by [`decode_text`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum TextEncoding {
    /// UTF-8 without BOM (also the fallback for unknown bytes).
    Utf8,
    /// UTF-8 with an `EF BB BF` BOM.
    Utf8Bom,
    /// UTF-16LE with an `FF FE` BOM.
    Utf16LeBom,
    /// UTF-16LE detected from the NUL-byte pattern (no BOM).
    Utf16Le,
    /// UTF-16BE with an `FE FF` BOM.
    Utf16BeBom,
    /// UTF-16BE detected from the NUL-byte pattern (no BOM).
    Utf16Be,
}

impl TextEncoding {
    /// Short label, e.g. `"UTF-16LE"`.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Utf8 => "UTF-8",
            Self::Utf8Bom => "UTF-8 (BOM)",
            Self::Utf16LeBom => "UTF-16LE (BOM)",
            Self::Utf16Le => "UTF-16LE",
            Self::Utf16BeBom => "UTF-16BE (BOM)",
            Self::Utf16Be => "UTF-16BE",
        }
    }
}

/// Result of [`decode_text`]: the text (BOM removed) and how it was decoded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedText {
    /// Decoded text without the BOM. Invalid sequences become U+FFFD.
    pub text: String,
    /// Detected encoding.
    pub encoding: TextEncoding,
    /// `true` if invalid sequences were replaced.
    pub had_errors: bool,
}

/// Decodes bytes as text, detecting UTF-8 / UTF-16LE / UTF-16BE from a BOM
/// or, without a BOM, from the NUL-byte pattern of the first bytes.
#[must_use]
pub fn decode_text(bytes: &[u8]) -> DecodedText {
    if let Some(rest) = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        return utf8(rest, TextEncoding::Utf8Bom);
    }
    if let Some(rest) = bytes.strip_prefix(&[0xFF, 0xFE]) {
        return utf16(rest, false, TextEncoding::Utf16LeBom);
    }
    if let Some(rest) = bytes.strip_prefix(&[0xFE, 0xFF]) {
        return utf16(rest, true, TextEncoding::Utf16BeBom);
    }
    match utf16_pattern(bytes) {
        Some(false) => utf16(bytes, false, TextEncoding::Utf16Le),
        Some(true) => utf16(bytes, true, TextEncoding::Utf16Be),
        None => utf8(bytes, TextEncoding::Utf8),
    }
}

/// Detects BOM-less UTF-16 from the first 512 bytes: `Some(false)` for LE
/// (NUL in most odd positions, few in even ones), `Some(true)` for BE.
pub(crate) fn utf16_pattern(bytes: &[u8]) -> Option<bool> {
    let n = bytes.len().min(512) & !1;
    if n < 4 {
        return None;
    }
    let pairs = n / 2;
    let (mut even_nul, mut odd_nul) = (0usize, 0usize);
    for pair in bytes[..n].as_chunks::<2>().0 {
        even_nul += usize::from(pair[0] == 0);
        odd_nul += usize::from(pair[1] == 0);
    }
    if odd_nul * 10 >= pairs * 7 && even_nul * 10 <= pairs {
        Some(false)
    } else if even_nul * 10 >= pairs * 7 && odd_nul * 10 <= pairs {
        Some(true)
    } else {
        None
    }
}

fn utf8(bytes: &[u8], encoding: TextEncoding) -> DecodedText {
    match std::str::from_utf8(bytes) {
        Ok(s) => DecodedText {
            text: s.to_owned(),
            encoding,
            had_errors: false,
        },
        Err(_) => DecodedText {
            text: String::from_utf8_lossy(bytes).into_owned(),
            encoding,
            had_errors: true,
        },
    }
}

fn utf16(bytes: &[u8], big_endian: bool, encoding: TextEncoding) -> DecodedText {
    let codec = if big_endian { UTF_16BE } else { UTF_16LE };
    let (text, had_errors) = codec.decode_without_bom_handling(bytes);
    DecodedText {
        text: text.into_owned(),
        encoding,
        had_errors,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn utf16le(s: &str) -> Vec<u8> {
        s.encode_utf16().flat_map(u16::to_le_bytes).collect()
    }

    #[test]
    fn detects_encodings() {
        let d = decode_text(b"\xEF\xBB\xBF{}");
        assert_eq!((d.text.as_str(), d.encoding), ("{}", TextEncoding::Utf8Bom));
        let d = decode_text(&utf16le("1.32"));
        assert_eq!(
            (d.text.as_str(), d.encoding),
            ("1.32", TextEncoding::Utf16Le)
        );
        let mut bom = vec![0xFF, 0xFE];
        bom.extend(utf16le("{\"a\":1}"));
        let d = decode_text(&bom);
        assert_eq!(
            (d.text.as_str(), d.encoding),
            ("{\"a\":1}", TextEncoding::Utf16LeBom)
        );
        let be: Vec<u8> = "abc".encode_utf16().flat_map(u16::to_be_bytes).collect();
        assert_eq!(decode_text(&be).encoding, TextEncoding::Utf16Be);
        let d = decode_text(b"plain");
        assert_eq!((d.text.as_str(), d.encoding), ("plain", TextEncoding::Utf8));
        assert!(decode_text(b"a\xC3").had_errors);
    }
}
