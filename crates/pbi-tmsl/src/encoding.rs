//! Text encoding detection for TMSL files.
//!
//! `model.bim` is normally UTF-8 (sometimes with a BOM); a PBIT
//! `DataModelSchema` entry is UTF-16LE **without** a BOM. Detection order:
//! BOM, then NUL bytes (UTF-8 JSON never contains NUL, so any NUL in the
//! first 512 bytes means UTF-16; the first code unit — JSON starts with an
//! ASCII character — or else the NUL positions give the byte order),
//! otherwise UTF-8. Decoding is strict: malformed input is an error, never
//! replaced with U+FFFD.

use serde::{Deserialize, Serialize};

use crate::error::TmslError;

/// Text encoding of a TMSL file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TextEncoding {
    #[default]
    Utf8,
    Utf16Le,
    Utf16Be,
}

impl TextEncoding {
    pub fn label(self) -> &'static str {
        match self {
            TextEncoding::Utf8 => "UTF-8",
            TextEncoding::Utf16Le => "UTF-16LE",
            TextEncoding::Utf16Be => "UTF-16BE",
        }
    }

    fn bom(self) -> &'static [u8] {
        match self {
            TextEncoding::Utf8 => &[0xEF, 0xBB, 0xBF],
            TextEncoding::Utf16Le => &[0xFF, 0xFE],
            TextEncoding::Utf16Be => &[0xFE, 0xFF],
        }
    }
}

/// Decoded text plus how it was stored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedText {
    /// The text, without a byte-order mark.
    pub text: String,
    pub encoding: TextEncoding,
    /// Whether the bytes started with a byte-order mark.
    pub bom: bool,
}

/// Detects the encoding of `bytes` (see the module docs). Returns the
/// encoding and whether a BOM is present.
pub fn detect_encoding(bytes: &[u8]) -> (TextEncoding, bool) {
    for encoding in [
        TextEncoding::Utf8,
        TextEncoding::Utf16Le,
        TextEncoding::Utf16Be,
    ] {
        if bytes.starts_with(encoding.bom()) {
            return (encoding, true);
        }
    }
    let sample = bytes.get(..512).unwrap_or(bytes);
    // JSON text starts with an ASCII character (`{`, `[` or whitespace), so
    // the zero byte of the first UTF-16 code unit gives the byte order, even
    // when non-ASCII text dominates the rest of the sample.
    match sample {
        [first, 0, ..] if *first != 0 => return (TextEncoding::Utf16Le, false),
        [0, second, ..] if *second != 0 => return (TextEncoding::Utf16Be, false),
        _ => {}
    }
    let (mut even, mut odd) = (0usize, 0usize);
    for (i, &b) in sample.iter().enumerate() {
        if b == 0 {
            if i % 2 == 0 {
                even += 1;
            } else {
                odd += 1;
            }
        }
    }
    match (even, odd) {
        (0, 0) => (TextEncoding::Utf8, false),
        (even, odd) if odd >= even => (TextEncoding::Utf16Le, false),
        _ => (TextEncoding::Utf16Be, false),
    }
}

/// Decodes TMSL bytes (UTF-8 or UTF-16, with or without BOM).
pub fn decode_text(bytes: &[u8]) -> Result<DecodedText, TmslError> {
    let (encoding, bom) = detect_encoding(bytes);
    let body = if bom {
        bytes.get(encoding.bom().len()..).unwrap_or_default()
    } else {
        bytes
    };
    let text = match encoding {
        TextEncoding::Utf8 => std::str::from_utf8(body)
            .map_err(|e| TmslError::Encoding {
                encoding: encoding.label(),
                message: e.to_string(),
            })?
            .to_owned(),
        TextEncoding::Utf16Le | TextEncoding::Utf16Be => {
            let codec = if encoding == TextEncoding::Utf16Le {
                encoding_rs::UTF_16LE
            } else {
                encoding_rs::UTF_16BE
            };
            codec
                .decode_without_bom_handling_and_without_replacement(body)
                .ok_or_else(|| TmslError::Encoding {
                    encoding: encoding.label(),
                    message: "malformed code unit sequence (odd length or unpaired surrogate)"
                        .to_owned(),
                })?
                .into_owned()
        }
    };
    Ok(DecodedText {
        text,
        encoding,
        bom,
    })
}

/// Encodes `text` (which must not start with U+FEFF) as `encoding`,
/// optionally prefixed with a byte-order mark.
pub fn encode_text(text: &str, encoding: TextEncoding, bom: bool) -> Vec<u8> {
    let mut out = Vec::with_capacity(match encoding {
        TextEncoding::Utf8 => text.len() + 3,
        _ => text.len().saturating_mul(2).saturating_add(2),
    });
    if bom {
        out.extend_from_slice(encoding.bom());
    }
    match encoding {
        TextEncoding::Utf8 => out.extend_from_slice(text.as_bytes()),
        TextEncoding::Utf16Le => text
            .encode_utf16()
            .for_each(|u| out.extend_from_slice(&u.to_le_bytes())),
        TextEncoding::Utf16Be => text
            .encode_utf16()
            .for_each(|u| out.extend_from_slice(&u.to_be_bytes())),
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_boms_and_nul_patterns() {
        assert_eq!(detect_encoding(b"{}"), (TextEncoding::Utf8, false));
        assert_eq!(
            detect_encoding(b"\xEF\xBB\xBF{}"),
            (TextEncoding::Utf8, true)
        );
        assert_eq!(
            detect_encoding(b"\xFF\xFE{\0}\0"),
            (TextEncoding::Utf16Le, true)
        );
        assert_eq!(
            detect_encoding(b"\xFE\xFF\0{\0}"),
            (TextEncoding::Utf16Be, true)
        );
        assert_eq!(detect_encoding(b"{\0}\0"), (TextEncoding::Utf16Le, false));
        assert_eq!(detect_encoding(b"\0{\0}"), (TextEncoding::Utf16Be, false));
        assert_eq!(detect_encoding(b""), (TextEncoding::Utf8, false));
    }

    /// Found by `cargo fuzz` (tmsl): text whose code units mostly have a zero
    /// low byte (U+0100, U+2000, ...) outvoted the ASCII structure, so a
    /// written `DataModelSchema` read back as UTF-16BE.
    #[test]
    fn byte_order_follows_the_first_code_unit() {
        let text = format!("{{\"name\": \"{}\"}}", "\u{100}\u{2000}".repeat(200));
        for encoding in [TextEncoding::Utf16Le, TextEncoding::Utf16Be] {
            let bytes = encode_text(&text, encoding, false);
            assert_eq!(detect_encoding(&bytes), (encoding, false));
            assert_eq!(decode_text(&bytes).unwrap().text, text);
        }
    }

    #[test]
    fn encode_decode_round_trip_with_non_bmp_text() {
        let text = "{\"name\": \"Grüße 📊\"}";
        for encoding in [
            TextEncoding::Utf8,
            TextEncoding::Utf16Le,
            TextEncoding::Utf16Be,
        ] {
            for bom in [false, true] {
                let bytes = encode_text(text, encoding, bom);
                let decoded = decode_text(&bytes).unwrap();
                assert_eq!(decoded.text, text);
                assert_eq!((decoded.encoding, decoded.bom), (encoding, bom));
            }
        }
    }

    #[test]
    fn malformed_input_is_an_error() {
        assert!(decode_text(b"{\xC3}").is_err());
        // Odd length and a lone high surrogate.
        assert!(decode_text(b"{\0}\0x").is_err());
        assert!(decode_text(b"{\0\x00\xD8}\0").is_err());
    }
}
