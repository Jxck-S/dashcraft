//! Safe ZIP container (`pbi_core::container::ZipContainer`) with strict limits.
//!
//! Inputs starting with `PK` are opened as raw archives. Anything else is
//! decoded with `arbitrary` into a list of entries (hostile names included:
//! `..`, absolute, drive letters, backslashes, NUL, duplicates, symlinks)
//! that is written with the `zip` crate first, so zip-slip handling is
//! explored structurally, not only through byte mutations.
//!
//! Invariants: never panics; every listed path is normalized and relative
//! (no `..`, `.`, empty or absolute components, no `\` or NUL, no drive
//! letter); entries are sorted and unique; declared sizes respect the
//! limits; a successful read returns exactly the declared size; lookups of
//! escaping paths fail.
#![no_main]

use std::io::{Cursor, Write};

use arbitrary::{Arbitrary, Unstructured};
use libfuzzer_sys::fuzz_target;
use pbi_core::container::{Container, Limits, ZipContainer};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipWriter};

const MAX_ENTRIES: usize = 32;
const MAX_ENTRY_BYTES: usize = 64 * 1024;

#[derive(Debug, Arbitrary)]
enum Kind {
    Stored,
    Deflated,
    Directory,
    Symlink,
}

#[derive(Debug, Arbitrary)]
struct Entry {
    name: Name,
    kind: Kind,
    data: Vec<u8>,
}

/// Mostly path-shaped names built from interesting components.
#[derive(Debug, Arbitrary)]
enum Name {
    Raw(String),
    Parts(Vec<Part>),
}

#[derive(Debug, Arbitrary)]
enum Part {
    Text(String),
    Dot,
    DotDot,
    Empty,
    Drive,
    Backslash,
    Nul,
    Known(u8),
}

const KNOWN: &[&str] = &[
    "Report",
    "Layout",
    "DataModel",
    "DataModelSchema",
    "[Content_Types].xml",
    "Version",
    "definition",
    "pages",
    "report.json",
    "model.tmdl",
    "a",
    "A",
];

impl Name {
    fn render(&self) -> String {
        match self {
            Name::Raw(s) => s.clone(),
            Name::Parts(parts) => {
                let mut out = String::new();
                for part in parts {
                    match part {
                        Part::Text(s) => out.push_str(s),
                        Part::Dot => out.push('.'),
                        Part::DotDot => out.push_str(".."),
                        Part::Empty => {}
                        Part::Drive => out.push_str("C:"),
                        Part::Backslash => out.push('\\'),
                        Part::Nul => out.push('\0'),
                        Part::Known(i) => out.push_str(KNOWN[usize::from(*i) % KNOWN.len()]),
                    }
                    out.push('/');
                }
                out.pop();
                out
            }
        }
    }
}

fn build_zip(u: &mut Unstructured<'_>) -> Option<Vec<u8>> {
    let entries: Vec<Entry> = u.arbitrary().ok()?;
    let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
    for entry in entries.iter().take(MAX_ENTRIES) {
        let name = entry.name.render();
        let data = &entry.data[..entry.data.len().min(MAX_ENTRY_BYTES)];
        let stored = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
        let result = match entry.kind {
            Kind::Stored => zip
                .start_file(name.as_str(), stored)
                .and_then(|()| zip.write_all(data).map_err(Into::into)),
            Kind::Deflated => zip
                .start_file(
                    name.as_str(),
                    SimpleFileOptions::default().compression_method(CompressionMethod::Deflated),
                )
                .and_then(|()| zip.write_all(data).map_err(Into::into)),
            Kind::Directory => zip.add_directory(name.as_str(), stored),
            Kind::Symlink => zip.add_symlink(name.as_str(), "../../etc/passwd", stored),
        };
        if result.is_err() {
            // e.g. a duplicate name: the writer refuses it; keep going.
            let _ = zip.abort_file();
        }
    }
    zip.finish().ok().map(Cursor::into_inner)
}

fn is_normalized(path: &str, limits: &Limits) -> bool {
    !path.is_empty()
        && path.len() <= limits.max_path_len
        && !path.contains(['\\', '\0'])
        && !path.starts_with('/')
        && path.split('/').count() <= limits.max_depth
        && path
            .split('/')
            .all(|c| !c.is_empty() && c != "." && c != "..")
        && !path.split('/').next().is_some_and(|first| {
            first.len() >= 2
                && first.as_bytes()[1] == b':'
                && first.as_bytes()[0].is_ascii_alphabetic()
        })
}

fuzz_target!(|data: &[u8]| {
    let bytes = if data.starts_with(b"PK") {
        data.to_vec()
    } else {
        match build_zip(&mut Unstructured::new(data)) {
            Some(bytes) => bytes,
            None => return,
        }
    };
    let limits = Limits::strict();
    let Ok(zip) = ZipContainer::from_bytes(bytes, limits) else {
        return;
    };

    let entries = zip.entries();
    assert!(entries.len() <= limits.max_entries);
    let mut total: u64 = 0;
    for pair in entries.windows(2) {
        assert!(
            pair[0].path < pair[1].path,
            "entries not sorted/unique: {pair:?}"
        );
    }
    for entry in entries {
        assert!(
            is_normalized(&entry.path, &limits),
            "unsafe entry path {:?}",
            entry.path
        );
        if !entry.is_dir {
            assert!(entry.size <= limits.max_entry_size);
            total += entry.size;
        }
        assert!(zip.exists(&entry.path));
        assert_eq!(zip.is_file(&entry.path), !entry.is_dir);
        if let Ok(content) = zip.read(&entry.path) {
            assert!(!entry.is_dir, "read a directory: {:?}", entry.path);
            assert_eq!(content.len() as u64, entry.size, "{:?}", entry.path);
        }
        let _ = zip.find_case_insensitive(&entry.path.to_uppercase());
        if let Some((dir, _)) = entry.path.rsplit_once('/') {
            assert!(zip.is_dir(dir));
            let sub = zip.sub(dir);
            let _ = sub.list_dir("");
        }
    }
    assert!(total <= limits.max_total_size);
    let _ = zip.list_dir("");
    let _ = zip.list_prefix("");
    for escaping in ["../x", "/etc/passwd", "a/../../x", "C:/x", "..\\x", "a\0b"] {
        assert!(zip.read(escaping).is_err(), "{escaping:?} was readable");
    }
    for skipped in zip.skipped() {
        let _ = format!("{skipped:?}");
    }
});
