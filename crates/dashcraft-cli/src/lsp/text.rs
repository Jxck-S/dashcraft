//! Text positions as LSP counts them, the languages served, and `file:` URIs.

use serde::{Deserialize, Serialize};

/// An LSP position: 0-based line and UTF-16 code units into it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Position {
    pub line: u32,
    pub character: u32,
}

/// An LSP range (end exclusive).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Range {
    pub start: Position,
    pub end: Position,
}

fn to_u32(n: usize) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}

/// Where the lines of a text start. Lines end at `\n`, `\r\n` or `\r`, as LSP counts them.
#[derive(Debug, Clone)]
pub struct LineIndex {
    starts: Vec<usize>,
}

impl LineIndex {
    pub fn new(text: &str) -> Self {
        let bytes = text.as_bytes();
        let mut starts = vec![0];
        for (i, &b) in bytes.iter().enumerate() {
            match b {
                b'\n' => starts.push(i + 1),
                b'\r' if bytes.get(i + 1) != Some(&b'\n') => starts.push(i + 1),
                _ => {}
            }
        }
        Self { starts }
    }

    /// The start and end (before the line break) of line `line`.
    fn line_span(&self, text: &str, line: usize) -> Option<(usize, usize)> {
        let start = *self.starts.get(line)?;
        let next = self.starts.get(line + 1).copied().unwrap_or(text.len());
        let content = text.get(start..next)?.trim_end_matches(['\n', '\r']);
        Some((start, start + content.len()))
    }

    /// The position of a UTF-8 byte offset (clamped to the text, rounded down to a character
    /// boundary).
    pub fn position(&self, text: &str, offset: usize) -> Position {
        let mut offset = offset.min(text.len());
        while !text.is_char_boundary(offset) {
            offset -= 1;
        }
        let line = self
            .starts
            .partition_point(|&s| s <= offset)
            .saturating_sub(1);
        let start = self.starts.get(line).copied().unwrap_or(0);
        let character = text
            .get(start..offset)
            .map_or(0, |s| s.encode_utf16().count());
        Position {
            line: to_u32(line),
            character: to_u32(character),
        }
    }

    /// The UTF-8 byte offset of a position. A character past the end of its line means the
    /// end of the line, inside a surrogate pair the character's start, and a line past the
    /// last one the end of the text.
    pub fn offset(&self, text: &str, pos: Position) -> usize {
        let Some((start, end)) = self.line_span(text, pos.line as usize) else {
            return text.len();
        };
        let want = pos.character as usize;
        let mut units = 0;
        for (i, c) in text.get(start..end).unwrap_or("").char_indices() {
            if units >= want {
                return start + i;
            }
            units += c.len_utf16();
            if units > want {
                return start + i;
            }
        }
        end
    }

    /// The range of UTF-8 byte offsets `start..end`.
    pub fn range(&self, text: &str, start: usize, end: usize) -> Range {
        Range {
            start: self.position(text, start),
            end: self.position(text, end.max(start)),
        }
    }

    /// The range of the whole text.
    pub fn whole(&self, text: &str) -> Range {
        self.range(text, 0, text.len())
    }

    /// The length of line `line` in UTF-16 code units, without its line break.
    pub fn line_length(&self, text: &str, line: u32) -> u32 {
        self.line_span(text, line as usize)
            .and_then(|(s, e)| text.get(s..e))
            .map_or(0, |s| to_u32(s.encode_utf16().count()))
    }
}

/// Applies one `contentChanges` entry: a `range` replaced by `text`, or the whole text.
pub fn apply_change(text: &mut String, lines: &LineIndex, range: Option<Range>, new_text: &str) {
    match range {
        Some(r) => {
            let start = lines.offset(text, r.start);
            let end = lines.offset(text, r.end).max(start);
            text.replace_range(start..end, new_text);
        }
        None => new_text.clone_into(text),
    }
}

/// A language the server knows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Language {
    Dax,
    M,
    Tmdl,
}

impl Language {
    /// From the client's language id, else from the file extension.
    pub fn detect(language_id: &str, uri: &str) -> Option<Self> {
        match language_id.to_ascii_lowercase().as_str() {
            "dax" => return Some(Self::Dax),
            "m" | "powerquery" | "pq" => return Some(Self::M),
            "tmdl" => return Some(Self::Tmdl),
            _ => {}
        }
        let name = uri_file_name(uri)?;
        let ext = name.rsplit_once('.')?.1.to_ascii_lowercase();
        match ext.as_str() {
            "dax" => Some(Self::Dax),
            "m" | "pq" | "pqm" => Some(Self::M),
            "tmdl" => Some(Self::Tmdl),
            _ => None,
        }
    }
}

/// The last path segment of a URI, decoded.
pub fn uri_file_name(uri: &str) -> Option<String> {
    let path = uri.split(['?', '#']).next()?;
    let name = path.rsplit('/').next()?;
    percent_decode(name).filter(|n| !n.is_empty())
}

fn percent_decode(s: &str) -> Option<String> {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while let Some(&b) = bytes.get(i) {
        if b == b'%' {
            let hex = std::str::from_utf8(bytes.get(i + 1..i + 3)?).ok()?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(b);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

fn percent_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"-._~/".contains(&b) {
            out.push(char::from(b));
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// The file path of a `file:` URI as a string, Windows style when `windows`: `C:\x`
/// (upper-case drive) or `\\host\share`.
fn file_path(uri: &str, windows: bool) -> Option<String> {
    let (scheme, rest) = uri.split_once(':')?;
    if !scheme.eq_ignore_ascii_case("file") {
        return None;
    }
    let rest = rest.strip_prefix("//")?.split(['?', '#']).next()?;
    let (host, path) = rest.split_at(rest.find('/')?);
    let host = percent_decode(host)?;
    let path = percent_decode(path)?;
    let local = host.is_empty() || host.eq_ignore_ascii_case("localhost");
    if !windows {
        return local.then_some(path);
    }
    if !local {
        return Some(format!(r"\\{host}{}", path.replace('/', "\\")));
    }
    let path = path.strip_prefix('/').unwrap_or(&path);
    let mut chars = path.chars();
    match (chars.next(), chars.next()) {
        (Some(drive), Some(':')) if drive.is_ascii_alphabetic() => Some(format!(
            "{}{}",
            drive.to_ascii_uppercase(),
            path.get(1..)?.replace('/', "\\")
        )),
        _ => None,
    }
}

/// The `file:` URI of a path given as a string, Windows style when `windows`.
fn file_uri(path: &str, windows: bool) -> String {
    if !windows {
        return format!("file://{}", percent_encode(path));
    }
    let path = path.replace('\\', "/");
    if let Some(unc) = path.strip_prefix("//") {
        let (host, rest) = unc.split_at(unc.find('/').unwrap_or(unc.len()));
        return format!("file://{host}{}", percent_encode(rest));
    }
    let mut chars = path.chars();
    match (chars.next(), chars.next()) {
        (Some(drive), Some(':')) if drive.is_ascii_alphabetic() => format!(
            "file:///{}:{}",
            drive.to_ascii_lowercase(),
            percent_encode(path.get(2..).unwrap_or(""))
        ),
        _ => format!("file:///{}", percent_encode(&path)),
    }
}

/// The local path of a `file:` URI.
pub fn uri_to_path(uri: &str) -> Option<std::path::PathBuf> {
    file_path(uri, cfg!(windows)).map(std::path::PathBuf::from)
}

/// The `file:` URI of a local path.
pub fn path_to_uri(path: &std::path::Path) -> Option<String> {
    path.to_str().map(|p| file_uri(p, cfg!(windows)))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    fn pos(line: u32, character: u32) -> Position {
        Position { line, character }
    }

    #[test]
    fn positions_count_utf16_and_every_line_break() {
        let text = "a\u{e9}\r\n\u{1f600}x\rlast\n";
        let lines = LineIndex::new(text);
        assert_eq!(lines.position(text, 0), pos(0, 0));
        assert_eq!(lines.position(text, 3), pos(0, 2));
        // Inside the CRLF: still the first line.
        assert_eq!(lines.position(text, 4), pos(0, 3));
        assert_eq!(lines.position(text, 5), pos(1, 0));
        // Inside the emoji: its start.
        assert_eq!(lines.position(text, 7), pos(1, 0));
        assert_eq!(lines.position(text, 9), pos(1, 2));
        assert_eq!(lines.position(text, 11), pos(2, 0));
        assert_eq!(lines.position(text, 99), pos(3, 0));
        assert_eq!(lines.offset(text, pos(0, 2)), 3);
        assert_eq!(lines.offset(text, pos(0, 9)), 3);
        assert_eq!(lines.offset(text, pos(1, 1)), 5);
        assert_eq!(lines.offset(text, pos(1, 2)), 9);
        assert_eq!(lines.offset(text, pos(2, 4)), 15);
        assert_eq!(lines.offset(text, pos(3, 0)), text.len());
        assert_eq!(lines.offset(text, pos(9, 0)), text.len());
        assert_eq!(lines.line_length(text, 0), 2);
        assert_eq!(lines.line_length(text, 1), 3);
        assert_eq!(lines.line_length(text, 3), 0);
        for offset in 0..=text.len() {
            if text.is_char_boundary(offset) && !text[..offset].ends_with('\r') {
                let p = lines.position(text, offset);
                assert_eq!(lines.offset(text, p), offset, "{offset}");
            }
        }
    }

    #[test]
    fn applies_incremental_and_full_changes() {
        let mut text = "SUM(\u{1f600}x)\nB".to_owned();
        let lines = LineIndex::new(&text);
        let r = Range {
            start: pos(0, 4),
            end: pos(0, 6),
        };
        apply_change(&mut text, &lines, Some(r), "y");
        assert_eq!(text, "SUM(yx)\nB");
        let lines = LineIndex::new(&text);
        let reversed = Range {
            start: pos(1, 1),
            end: pos(1, 0),
        };
        apply_change(&mut text, &lines, Some(reversed), "C");
        assert_eq!(text, "SUM(yx)\nBC");
        apply_change(&mut text, &lines, None, "new");
        assert_eq!(text, "new");
    }

    #[test]
    fn detects_languages_by_id_then_extension() {
        assert_eq!(Language::detect("dax", "untitled:1"), Some(Language::Dax));
        assert_eq!(
            Language::detect("powerquery", "file:///q.txt"),
            Some(Language::M)
        );
        assert_eq!(
            Language::detect("objective-c", "file:///a/Query.m"),
            Some(Language::M)
        );
        assert_eq!(
            Language::detect("plaintext", "file:///a/Orders.TMDL"),
            Some(Language::Tmdl)
        );
        assert_eq!(Language::detect("plaintext", "file:///a/b.txt"), None);
        assert_eq!(Language::detect("plaintext", "untitled:x"), None);
    }

    #[test]
    fn converts_file_uris_and_paths() {
        assert_eq!(
            file_path("file:///c%3A/My%20Model/tables/Orders.tmdl", true).unwrap(),
            r"C:\My Model\tables\Orders.tmdl"
        );
        assert_eq!(file_path("FILE:///D:/x.dax", true).unwrap(), r"D:\x.dax");
        assert_eq!(
            file_path("file://server/share/a.m", true).unwrap(),
            r"\\server\share\a.m"
        );
        assert_eq!(
            file_path("file:///home/me/caf%C3%A9.dax", false).unwrap(),
            "/home/me/caf\u{e9}.dax"
        );
        assert_eq!(file_path("file://server/a.m", false), None);
        assert_eq!(file_path("untitled:Untitled-1", false), None);
        assert_eq!(file_path("file:///bad%zz", false), None);
        assert_eq!(
            file_uri(r"C:\My Model\a#1.tmdl", true),
            "file:///c:/My%20Model/a%231.tmdl"
        );
        assert_eq!(
            file_uri(r"\\server\share\a.m", true),
            "file://server/share/a.m"
        );
        assert_eq!(
            file_uri("/home/me/caf\u{e9}.dax", false),
            "file:///home/me/caf%C3%A9.dax"
        );
        for (path, windows) in [
            ("C:\\My Model\\caf\u{e9}%.tmdl", true),
            (r"\\server\share\a b.m", true),
            ("/tmp/a b/c%.dax", false),
        ] {
            assert_eq!(file_path(&file_uri(path, windows), windows).unwrap(), path);
        }
        assert_eq!(
            uri_file_name("file:///a/My%20Query.m?x#y").as_deref(),
            Some("My Query.m")
        );
    }
}
