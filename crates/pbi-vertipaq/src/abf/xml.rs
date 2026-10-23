//! Text decoding and small XML helpers for the ABF metadata documents.

use roxmltree::{Document, Node, ParsingOptions};

use super::error::{AbfError, excerpt};

/// Upper bound on XML nodes per document (protects against node-bomb inputs; the
/// virtual directory and the backup log need ~15 nodes per file, see `MAX_ENTRIES`).
pub(crate) const MAX_XML_NODES: u32 = 32 << 20;

/// Decodes UTF-16LE code units until the first NUL unit (or the end of `bytes`).
/// An odd trailing byte is ignored.
pub(crate) fn decode_utf16le_until_nul(
    bytes: &[u8],
    what: &'static str,
) -> Result<String, AbfError> {
    let units = bytes
        .as_chunks::<2>()
        .0
        .iter()
        .map(|&pair| u16::from_le_bytes(pair))
        .take_while(|&unit| unit != 0);
    char::decode_utf16(units)
        .collect::<Result<String, _>>()
        .map_err(|_| AbfError::Encoding(what))
}

fn decode_utf16(bytes: &[u8], big_endian: bool, what: &'static str) -> Result<String, AbfError> {
    let units = bytes.as_chunks::<2>().0.iter().map(|&pair| {
        if big_endian {
            u16::from_be_bytes(pair)
        } else {
            u16::from_le_bytes(pair)
        }
    });
    char::decode_utf16(units)
        .collect::<Result<String, _>>()
        .map_err(|_| AbfError::Encoding(what))
}

/// Decodes an XML document stored as UTF-16 (with or without BOM) or UTF-8 (with or
/// without BOM). Trailing NUL padding is removed.
pub(crate) fn decode_document(bytes: &[u8], what: &'static str) -> Result<String, AbfError> {
    let text = if let Some(rest) = bytes.strip_prefix(&[0xFF, 0xFE]) {
        decode_utf16(rest, false, what)?
    } else if let Some(rest) = bytes.strip_prefix(&[0xFE, 0xFF]) {
        decode_utf16(rest, true, what)?
    } else if let Some(rest) = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        std::str::from_utf8(rest)
            .map_err(|_| AbfError::Encoding(what))?
            .to_owned()
    } else if bytes.len() >= 2 && bytes[0] != 0 && bytes[1] == 0 {
        decode_utf16(bytes, false, what)?
    } else {
        std::str::from_utf8(bytes)
            .map_err(|_| AbfError::Encoding(what))?
            .to_owned()
    };
    Ok(text.trim_end_matches('\0').to_owned())
}

/// Parses `text` as XML (no DTD, bounded node count).
pub(crate) fn parse<'a>(text: &'a str, what: &'static str) -> Result<Document<'a>, AbfError> {
    let options = ParsingOptions {
        allow_dtd: false,
        nodes_limit: MAX_XML_NODES,
        ..ParsingOptions::default()
    };
    Document::parse_with_options(text.trim_start_matches('\u{feff}'), options).map_err(|err| {
        AbfError::Xml {
            what,
            message: err.to_string(),
        }
    })
}

/// Checks that the document element has the expected (local) name.
pub(crate) fn expect_root<'a, 'i>(
    doc: &'a Document<'i>,
    name: &'static str,
    what: &'static str,
) -> Result<Node<'a, 'i>, AbfError> {
    let root = doc.root_element();
    if root.tag_name().name() == name {
        Ok(root)
    } else {
        Err(AbfError::Xml {
            what,
            message: format!("expected root element <{name}>"),
        })
    }
}

/// Element children with the given local name.
pub(crate) fn children<'a, 'i: 'a>(
    node: Node<'a, 'i>,
    name: &'static str,
) -> impl Iterator<Item = Node<'a, 'i>> + 'a {
    node.children()
        .filter(move |n| n.is_element() && n.tag_name().name() == name)
}

/// First element child with the given local name.
pub(crate) fn child<'a, 'i: 'a>(node: Node<'a, 'i>, name: &'static str) -> Option<Node<'a, 'i>> {
    children(node, name).next()
}

/// Concatenated, trimmed text of the first child element `name` (`None` if absent).
pub(crate) fn child_text(node: Node<'_, '_>, name: &'static str) -> Option<String> {
    let element = child(node, name)?;
    let text: String = element
        .children()
        .filter(Node::is_text)
        .filter_map(|n| n.text())
        .collect();
    Some(text.trim().to_owned())
}

/// Optional non-empty text of a child element.
pub(crate) fn opt_text(node: Node<'_, '_>, name: &'static str) -> Option<String> {
    child_text(node, name).filter(|s| !s.is_empty())
}

pub(crate) fn opt_i64(
    node: Node<'_, '_>,
    name: &'static str,
    what: &'static str,
) -> Result<Option<i64>, AbfError> {
    match opt_text(node, name) {
        None => Ok(None),
        Some(text) => text
            .parse::<i64>()
            .map(Some)
            .map_err(|_| invalid(what, name, &text)),
    }
}

pub(crate) fn opt_u64(
    node: Node<'_, '_>,
    name: &'static str,
    what: &'static str,
) -> Result<Option<u64>, AbfError> {
    match opt_text(node, name) {
        None => Ok(None),
        Some(text) => text
            .parse::<u64>()
            .map(Some)
            .map_err(|_| invalid(what, name, &text)),
    }
}

pub(crate) fn req_u64(
    node: Node<'_, '_>,
    name: &'static str,
    what: &'static str,
) -> Result<u64, AbfError> {
    opt_u64(node, name, what)?.ok_or(AbfError::MissingField {
        what,
        element: name,
    })
}

/// `true`/`false`/`1`/`0` (case-insensitive); absent elements yield `None`.
pub(crate) fn opt_bool(
    node: Node<'_, '_>,
    name: &'static str,
    what: &'static str,
) -> Result<Option<bool>, AbfError> {
    match opt_text(node, name) {
        None => Ok(None),
        Some(text) if text.eq_ignore_ascii_case("true") || text == "1" => Ok(Some(true)),
        Some(text) if text.eq_ignore_ascii_case("false") || text == "0" => Ok(Some(false)),
        Some(text) => Err(invalid(what, name, &text)),
    }
}

fn invalid(what: &'static str, element: &'static str, value: &str) -> AbfError {
    AbfError::InvalidField {
        what,
        element,
        value: excerpt(value),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn utf16(text: &str) -> Vec<u8> {
        text.encode_utf16().flat_map(u16::to_le_bytes).collect()
    }

    #[test]
    fn decodes_all_supported_encodings() {
        let mut le_bom = vec![0xFF, 0xFE];
        le_bom.extend(utf16("<a>é</a>\0\0"));
        assert_eq!(decode_document(&le_bom, "t").unwrap(), "<a>é</a>");
        assert_eq!(decode_document(&utf16("<a/>"), "t").unwrap(), "<a/>");
        let mut be_bom = vec![0xFE, 0xFF];
        be_bom.extend("<b/>".encode_utf16().flat_map(u16::to_be_bytes));
        assert_eq!(decode_document(&be_bom, "t").unwrap(), "<b/>");
        assert_eq!(decode_document(b"\xEF\xBB\xBF<c/>", "t").unwrap(), "<c/>");
        assert_eq!(decode_document(b"<d/>\0\0", "t").unwrap(), "<d/>");
        assert!(matches!(
            decode_document(b"<\xFF\xFE", "t"),
            Err(AbfError::Encoding("t"))
        ));
        // Unpaired surrogate.
        assert!(decode_document(&[0xFF, 0xFE, 0x00, 0xD8, 0x41, 0x00], "t").is_err());
    }

    #[test]
    fn utf16_until_nul_stops_at_padding() {
        let mut bytes = utf16("<x/>");
        bytes.extend([0, 0, b'j', 0, 0x01]);
        assert_eq!(decode_utf16le_until_nul(&bytes, "t").unwrap(), "<x/>");
    }

    #[test]
    fn field_helpers_validate_values() {
        let doc = parse(
            "<r><n> 42 </n><neg>-1</neg><b>TRUE</b><bad>x1</bad><e/><amp>a&amp;b</amp></r>",
            "t",
        )
        .unwrap();
        let root = expect_root(&doc, "r", "t").unwrap();
        assert_eq!(req_u64(root, "n", "t").unwrap(), 42);
        assert_eq!(opt_i64(root, "neg", "t").unwrap(), Some(-1));
        assert!(opt_u64(root, "neg", "t").is_err());
        assert_eq!(opt_bool(root, "b", "t").unwrap(), Some(true));
        assert!(opt_bool(root, "bad", "t").is_err());
        assert!(matches!(
            req_u64(root, "missing", "t"),
            Err(AbfError::MissingField {
                element: "missing",
                ..
            })
        ));
        assert_eq!(opt_text(root, "e"), None);
        assert_eq!(child_text(root, "e").as_deref(), Some(""));
        assert_eq!(opt_text(root, "amp").as_deref(), Some("a&b"));
        assert!(expect_root(&doc, "other", "t").is_err());
    }

    #[test]
    fn rejects_dtd_and_malformed_xml() {
        assert!(parse("<!DOCTYPE x [<!ENTITY a \"b\">]><x>&a;</x>", "t").is_err());
        assert!(parse("<x><y></x>", "t").is_err());
        assert!(parse("", "t").is_err());
    }
}
