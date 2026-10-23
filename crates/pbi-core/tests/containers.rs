//! Container security tests: zip-slip, bombs, size lies, limits,
//! duplicates, unsupported entries, symlinks and directory escapes. Every
//! hostile input must produce a typed error, never a panic.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::io::{Cursor, Write};
use std::sync::Arc;

use pbi_core::container::{
    Container, ContainerError, ContainerKind, DirContainer, LimitKind, Limits, MemContainer,
    SkipReason, SubContainer, UnsafePathReason, ZipContainer,
};
use pretty_assertions::assert_eq;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipWriter};

fn build_zip(files: &[(&str, &[u8])], method: CompressionMethod) -> Vec<u8> {
    let opts = SimpleFileOptions::default().compression_method(method);
    let mut w = ZipWriter::new(Cursor::new(Vec::new()));
    for (name, data) in files {
        if name.ends_with('/') {
            w.add_directory(*name, opts).unwrap();
        } else {
            w.start_file(*name, opts).unwrap();
            w.write_all(data).unwrap();
        }
    }
    w.finish().unwrap().into_inner()
}

fn stored(files: &[(&str, &[u8])]) -> Vec<u8> {
    build_zip(files, CompressionMethod::Stored)
}

fn deflated(files: &[(&str, &[u8])]) -> Vec<u8> {
    build_zip(files, CompressionMethod::Deflated)
}

const LOCAL_SIG: &[u8; 4] = b"PK\x03\x04";
const CENTRAL_SIG: &[u8; 4] = b"PK\x01\x02";

/// Overwrites `value` at `offset` from every occurrence of `sig`.
fn patch(bytes: &mut [u8], sig: &[u8; 4], offset: usize, value: &[u8]) {
    let positions: Vec<usize> = bytes
        .windows(4)
        .enumerate()
        .filter(|(_, w)| *w == sig)
        .map(|(i, _)| i)
        .collect();
    assert!(!positions.is_empty(), "signature not found");
    for p in positions {
        bytes[p + offset..p + offset + value.len()].copy_from_slice(value);
    }
}

/// Patches a header field in both local (`local_off`) and central
/// (`central_off`) headers.
fn patch_both(bytes: &mut [u8], local_off: usize, central_off: usize, value: &[u8]) {
    patch(bytes, LOCAL_SIG, local_off, value);
    patch(bytes, CENTRAL_SIG, central_off, value);
}

fn replace_all(bytes: &mut [u8], from: &[u8], to: &[u8]) {
    assert_eq!(from.len(), to.len());
    let mut i = 0;
    while i + from.len() <= bytes.len() {
        if &bytes[i..i + from.len()] == from {
            bytes[i..i + from.len()].copy_from_slice(to);
            i += from.len();
        } else {
            i += 1;
        }
    }
}

fn open(bytes: Vec<u8>) -> Result<ZipContainer, ContainerError> {
    ZipContainer::from_bytes(bytes, Limits::default())
}

fn unsafe_reason(r: Result<ZipContainer, ContainerError>) -> UnsafePathReason {
    match r {
        Err(ContainerError::UnsafePath { reason, .. }) => reason,
        other => panic!("expected UnsafePath, got {other:?}"),
    }
}

fn limit_kind<T: std::fmt::Debug>(r: Result<T, ContainerError>) -> LimitKind {
    match r {
        Err(ContainerError::LimitExceeded { limit, .. }) => limit,
        other => panic!("expected LimitExceeded, got {other:?}"),
    }
}

#[test]
fn zip_lists_and_reads_normalized_entries() {
    let bytes = deflated(&[
        ("Version", b"1\x00.\x003\x002\x00"),
        ("Report/", b""),
        ("Report/definition/report.json", b"{\"a\":1}"),
        ("./Report/definition/pages/p1/page.json", b"{}"),
    ]);
    let zip = open(bytes).unwrap();
    assert_eq!(zip.kind(), ContainerKind::Zip);
    let paths: Vec<&str> = zip.entries().iter().map(|e| e.path.as_str()).collect();
    assert_eq!(
        paths,
        [
            "Report",
            "Report/definition/pages/p1/page.json",
            "Report/definition/report.json",
            "Version"
        ]
    );
    assert!(zip.entry("Report").unwrap().is_dir);
    assert_eq!(
        zip.read("Report/definition/report.json").unwrap(),
        b"{\"a\":1}"
    );
    assert_eq!(
        zip.read("./Report\\definition/report.json").unwrap(),
        b"{\"a\":1}"
    );
    assert_eq!(zip.read_text("Version").unwrap().text, "1.32");
    assert!(matches!(
        zip.read("Report"),
        Err(ContainerError::IsDirectory(_))
    ));
    assert!(matches!(zip.read("nope"), Err(ContainerError::NotFound(_))));
    assert!(matches!(
        zip.read("Report/../Version"),
        Err(ContainerError::NotFound(_))
    ));
    assert!(zip.exists("Report/definition"), "implied directory");
    assert!(zip.is_dir("Report/definition/pages"));
    assert!(!zip.is_file("Report/definition"));
    assert_eq!(
        zip.find_case_insensitive("report/DEFINITION/Report.json")
            .map(|e| e.path.as_str()),
        Some("Report/definition/report.json")
    );
    let children: Vec<(String, bool)> = zip
        .list_dir("")
        .into_iter()
        .map(|c| (c.name, c.is_dir))
        .collect();
    assert_eq!(
        children,
        [("Report".to_owned(), true), ("Version".to_owned(), false)]
    );
    assert_eq!(zip.list_prefix("Report/definition").len(), 2);
}

#[test]
fn sub_container_is_a_rooted_view() {
    let bytes = stored(&[
        ("Report/definition/report.json", b"R"),
        ("Report/definition/pages/p/page.json", b"P"),
        ("Reports/x", b"X"),
        ("Version", b"V"),
    ]);
    let zip = open(bytes).unwrap();
    let report = zip.sub("Report");
    let paths: Vec<&str> = report.entries().iter().map(|e| e.path.as_str()).collect();
    assert_eq!(
        paths,
        ["definition/pages/p/page.json", "definition/report.json"]
    );
    assert_eq!(report.read("definition/report.json").unwrap(), b"R");
    assert!(report.read("../Version").is_err());
    assert!(report.read("Version").is_err());
    let def = report.sub("definition");
    assert_eq!(def.read("pages/p/page.json").unwrap(), b"P");
    assert!(def.root_label().ends_with("Report/definition"));

    // Readers take any of these uniformly as `&dyn Container`.
    fn count(c: &dyn Container) -> usize {
        c.entries().len()
    }
    assert_eq!(count(&def), 2);
    assert_eq!(count(&report), 2);

    let shared: Arc<dyn Container> = Arc::new(zip);
    let owned: SubContainer<'static> = SubContainer::shared(Arc::clone(&shared), "Report");
    assert_eq!(owned.read("definition/report.json").unwrap(), b"R");
    let invalid = shared.sub("../x");
    assert!(invalid.entries().is_empty());
    assert!(invalid.read("anything").is_err());

    assert_eq!(count(&owned), 2);
    assert_eq!(count(&shared), 4);
}

#[test]
fn zip_slip_names_are_refused() {
    let cases: &[(&str, UnsafePathReason)] = &[
        ("../evil", UnsafePathReason::ParentTraversal),
        ("/abs", UnsafePathReason::Absolute),
        ("C:\\x", UnsafePathReason::DriveLetter),
        ("C:/x", UnsafePathReason::DriveLetter),
        ("a/../../b", UnsafePathReason::ParentTraversal),
        ("..\\evil", UnsafePathReason::ParentTraversal),
        ("a\\..\\..\\b", UnsafePathReason::ParentTraversal),
        ("\\\\server\\share\\x", UnsafePathReason::Absolute),
        ("ok/./../../x", UnsafePathReason::ParentTraversal),
    ];
    for (name, expected) in cases {
        let bytes = stored(&[("fine.txt", b"x"), (name, b"payload")]);
        assert_eq!(unsafe_reason(open(bytes)), *expected, "name {name:?}");
    }
}

#[test]
fn nul_in_entry_name_is_refused() {
    let mut bytes = stored(&[("aXb", b"x")]);
    replace_all(&mut bytes, b"aXb", b"a\0b");
    assert_eq!(unsafe_reason(open(bytes)), UnsafePathReason::Nul);
}

#[test]
fn ratio_bomb_is_refused_at_open() {
    let zeros = vec![0u8; 2 * 1024 * 1024];
    let bytes = deflated(&[("bomb.bin", &zeros)]);
    assert!(bytes.len() < 20 * 1024, "deflate should crush zeros");
    let r = ZipContainer::from_bytes(bytes.clone(), Limits::strict());
    assert_eq!(limit_kind(r), LimitKind::CompressionRatio);

    // The same archive passes when the ratio cap allows it.
    let lenient = Limits {
        max_compression_ratio: 10_000,
        ..Limits::strict()
    };
    let zip = ZipContainer::from_bytes(bytes, lenient).unwrap();
    assert_eq!(zip.read("bomb.bin").unwrap().len(), zeros.len());
}

#[test]
fn moderately_compressible_data_passes_strict_limits() {
    let text: Vec<u8> = (0..200_000u32)
        .flat_map(|i| format!("{i},").into_bytes())
        .collect();
    let zip = ZipContainer::from_bytes(deflated(&[("data.csv", &text)]), Limits::strict()).unwrap();
    assert_eq!(zip.read("data.csv").unwrap(), text);
}

#[test]
fn overlapping_entries_are_refused() {
    let opts = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
    let mut w = ZipWriter::new(Cursor::new(Vec::new()));
    w.start_file("a.bin", opts).unwrap();
    w.write_all(&[7u8; 4096]).unwrap();
    w.shallow_copy_file("a.bin", "b.bin").unwrap();
    let bytes = w.finish().unwrap().into_inner();
    assert_eq!(limit_kind(open(bytes)), LimitKind::OverlappingEntries);
}

#[test]
fn declared_size_smaller_than_data_fails_on_read() {
    let data = [b'x'; 100];
    for build in [stored, deflated] {
        let mut bytes = build(&[("lie.txt", &data)]);
        // Uncompressed size: local header +22, central header +24.
        patch_both(&mut bytes, 22, 24, &10u32.to_le_bytes());
        let zip = open(bytes).unwrap();
        assert_eq!(zip.entry("lie.txt").unwrap().size, 10);
        assert_eq!(limit_kind(zip.read("lie.txt")), LimitKind::DeclaredSize);
    }
}

#[test]
fn declared_size_larger_than_data_is_corrupt() {
    let mut bytes = stored(&[("short.txt", &[b'y'; 100])]);
    patch_both(&mut bytes, 22, 24, &200u32.to_le_bytes());
    let zip = open(bytes).unwrap();
    assert!(matches!(
        zip.read("short.txt"),
        Err(ContainerError::Corrupt(_))
    ));
}

#[test]
fn huge_declared_size_is_refused_at_open() {
    let mut bytes = stored(&[("huge.bin", &[b'z'; 100])]);
    patch_both(&mut bytes, 22, 24, &0xFFFF_FFF0u32.to_le_bytes());
    assert_eq!(limit_kind(open(bytes)), LimitKind::EntrySize);
}

#[test]
fn entry_count_and_total_size_limits() {
    let files: Vec<(String, Vec<u8>)> =
        (0..6).map(|i| (format!("f{i}"), vec![b'a'; 600])).collect();
    let refs: Vec<(&str, &[u8])> = files
        .iter()
        .map(|(n, d)| (n.as_str(), d.as_slice()))
        .collect();
    let bytes = stored(&refs);
    let few = Limits {
        max_entries: 5,
        ..Limits::default()
    };
    assert_eq!(
        limit_kind(ZipContainer::from_bytes(bytes.clone(), few)),
        LimitKind::Entries
    );
    let small_total = Limits {
        max_total_size: 1000,
        ..Limits::default()
    };
    assert_eq!(
        limit_kind(ZipContainer::from_bytes(bytes.clone(), small_total)),
        LimitKind::TotalSize
    );
    let small_entry = Limits {
        max_entry_size: 500,
        ..Limits::default()
    };
    assert_eq!(
        limit_kind(ZipContainer::from_bytes(bytes, small_entry)),
        LimitKind::EntrySize
    );
}

#[test]
fn path_length_and_depth_limits() {
    let long = "a".repeat(300);
    let bytes = stored(&[(long.as_str(), b"x")]);
    assert_eq!(
        limit_kind(ZipContainer::from_bytes(bytes, Limits::strict())),
        LimitKind::PathLength
    );
    let deep = vec!["d"; 20].join("/");
    let bytes = stored(&[(deep.as_str(), b"x")]);
    assert_eq!(
        limit_kind(ZipContainer::from_bytes(bytes, Limits::strict())),
        LimitKind::Depth
    );
}

#[test]
fn normalized_duplicates_keep_the_first_entry() {
    let bytes = stored(&[
        ("a/b.txt", b"first"),
        ("a\\b.txt", b"second"),
        ("./a/b.txt", b"third"),
    ]);
    let zip = open(bytes).unwrap();
    assert_eq!(zip.entries().len(), 1);
    assert_eq!(zip.read("a/b.txt").unwrap(), b"first");
    let dups: Vec<_> = zip
        .skipped()
        .iter()
        .filter(|s| s.reason == SkipReason::Duplicate)
        .collect();
    assert_eq!(dups.len(), 2);
}

#[test]
fn raw_duplicate_names_are_refused() {
    let mut bytes = stored(&[("a.txt", b"one"), ("b.txt", b"two")]);
    replace_all(&mut bytes, b"b.txt", b"a.txt");
    assert_eq!(unsafe_reason(open(bytes)), UnsafePathReason::DuplicateName);
}

#[test]
fn encrypted_entries_are_unsupported() {
    let mut bytes = stored(&[("secret.txt", b"plaintext")]);
    // General-purpose flag bit 0: local header +6, central header +8.
    patch_both(&mut bytes, 6, 8, &1u16.to_le_bytes());
    let zip = open(bytes).unwrap();
    assert!(matches!(
        zip.read("secret.txt"),
        Err(ContainerError::Unsupported(_))
    ));
}

#[test]
fn unsupported_compression_methods_are_reported() {
    let mut bytes = stored(&[("lzma.bin", b"not really lzma")]);
    // Method 14 (LZMA): local header +8, central header +10.
    patch_both(&mut bytes, 8, 10, &14u16.to_le_bytes());
    let zip = open(bytes).unwrap();
    match zip.read("lzma.bin") {
        Err(ContainerError::Unsupported(msg)) => assert!(msg.contains("compression method")),
        other => panic!("expected Unsupported, got {other:?}"),
    }
}

#[test]
fn empty_and_garbage_archives_never_panic() {
    let empty = b"PK\x05\x06\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0".to_vec();
    assert!(open(empty).unwrap().entries().is_empty());
    assert!(matches!(
        open(b"definitely not a zip".to_vec()),
        Err(ContainerError::Corrupt(_))
    ));
    let good = deflated(&[("a/b.json", b"{\"x\": [1,2,3]}"), ("c", &[9u8; 3000])]);
    for cut in 0..good.len() {
        if let Ok(zip) = open(good[..cut].to_vec()) {
            for e in zip.entries() {
                let _ = zip.read(&e.path);
            }
        }
    }
    // Flip every byte in turn: errors are fine, panics are not.
    for i in 0..good.len() {
        let mut bad = good.clone();
        bad[i] ^= 0xFF;
        if let Ok(zip) = open(bad) {
            for e in zip.entries() {
                let _ = zip.read(&e.path);
            }
        }
    }
}

#[test]
fn file_backed_zip_reads_concurrently() {
    let dir = tempdir();
    let files: Vec<(String, Vec<u8>)> = (0..16)
        .map(|i| {
            (
                format!("dir/f{i}.txt"),
                format!("content {i}").repeat(500).into_bytes(),
            )
        })
        .collect();
    let refs: Vec<(&str, &[u8])> = files
        .iter()
        .map(|(n, d)| (n.as_str(), d.as_slice()))
        .collect();
    let path = dir.path().join("t.zip");
    std::fs::write(&path, deflated(&refs)).unwrap();
    let zip = ZipContainer::open(&path, Limits::default()).unwrap();
    assert!(zip.root_label().ends_with("t.zip"));
    std::thread::scope(|s| {
        for _ in 0..4 {
            s.spawn(|| {
                for (name, data) in &files {
                    assert_eq!(&zip.read(name).unwrap(), data);
                }
            });
        }
    });
}

#[test]
fn mem_container_validates_names() {
    let mut mem = MemContainer::new("mem");
    mem.insert("b/c.txt", b"c".to_vec()).unwrap();
    mem.insert("a.txt", b"a".to_vec()).unwrap();
    assert!(matches!(
        mem.insert("../x", b"x".to_vec()),
        Err(ContainerError::UnsafePath { .. })
    ));
    assert_eq!(mem.entries()[0].path, "a.txt");
    assert_eq!(mem.sub("b").read("c.txt").unwrap(), b"c");
}

// ---------------------------------------------------------------------------
// Directories
// ---------------------------------------------------------------------------

fn write(root: &std::path::Path, rel: &str, data: &[u8]) {
    let p = root.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, data).unwrap();
}

#[test]
fn dir_lists_files_and_skips_noise() {
    let tmp = tempdir();
    let root = tmp.path();
    write(root, "a.txt", b"A");
    write(root, "sub/b.txt", b"BB");
    write(root, ".git/config", b"x");
    write(root, "node_modules/m/index.js", b"x");
    write(root, "sub/.DS_Store", b"x");
    let dir = DirContainer::open(root, Limits::default()).unwrap();
    assert_eq!(dir.kind(), ContainerKind::Directory);
    let paths: Vec<&str> = dir.entries().iter().map(|e| e.path.as_str()).collect();
    assert_eq!(paths, ["a.txt", "sub", "sub/b.txt"]);
    assert_eq!(dir.entry("sub/b.txt").unwrap().size, 2);
    assert_eq!(dir.read("sub/b.txt").unwrap(), b"BB");
    assert_eq!(dir.sub("sub").read("b.txt").unwrap(), b"BB");
    let ignored = dir
        .skipped()
        .iter()
        .filter(|s| s.reason == SkipReason::Ignored)
        .count();
    assert_eq!(ignored, 3);
    assert!(matches!(dir.read("../x"), Err(ContainerError::NotFound(_))));
    assert!(matches!(
        dir.read(".git/config"),
        Err(ContainerError::NotFound(_))
    ));
    assert!(matches!(
        dir.read("sub"),
        Err(ContainerError::IsDirectory(_))
    ));
    write(root, "late.txt", b"created after open");
    assert!(matches!(
        dir.read("late.txt"),
        Err(ContainerError::NotFound(_))
    ));
}

#[test]
fn dir_depth_and_entry_limits() {
    let tmp = tempdir();
    write(tmp.path(), "1/2/3/4/5/deep.txt", b"x");
    let shallow = Limits {
        max_depth: 3,
        ..Limits::default()
    };
    assert_eq!(
        limit_kind(DirContainer::open(tmp.path(), shallow)),
        LimitKind::Depth
    );
    let few = Limits {
        max_entries: 3,
        ..Limits::default()
    };
    assert_eq!(
        limit_kind(DirContainer::open(tmp.path(), few)),
        LimitKind::Entries
    );
    assert!(DirContainer::open(tmp.path(), Limits::default()).is_ok());
}

#[test]
fn dir_read_enforces_entry_size() {
    let tmp = tempdir();
    write(tmp.path(), "big.bin", &[1u8; 64]);
    let limits = Limits {
        max_entry_size: 16,
        ..Limits::default()
    };
    let dir = DirContainer::open(tmp.path(), limits).unwrap();
    assert_eq!(limit_kind(dir.read("big.bin")), LimitKind::EntrySize);
}

#[test]
fn dir_root_must_be_a_directory() {
    let tmp = tempdir();
    write(tmp.path(), "f.txt", b"x");
    assert!(matches!(
        DirContainer::open(tmp.path().join("f.txt"), Limits::default()),
        Err(ContainerError::InvalidRoot { .. })
    ));
    assert!(matches!(
        DirContainer::open(tmp.path().join("missing"), Limits::default()),
        Err(ContainerError::Io { .. })
    ));
}

#[cfg(unix)]
#[test]
fn dir_symlinks_are_skipped_and_swaps_are_refused() {
    use std::os::unix::fs::symlink;

    let tmp = tempdir();
    let outside = tmp.path().join("outside");
    write(&outside, "secret.txt", b"secret");
    let root = tmp.path().join("root");
    write(&root, "a.txt", b"inside");
    symlink(outside.join("secret.txt"), root.join("link.txt")).unwrap();
    symlink(&outside, root.join("linkdir")).unwrap();

    let dir = DirContainer::open(&root, Limits::default()).unwrap();
    let paths: Vec<&str> = dir.entries().iter().map(|e| e.path.as_str()).collect();
    assert_eq!(paths, ["a.txt"]);
    let symlinks: Vec<&str> = dir
        .skipped()
        .iter()
        .filter(|s| s.reason == SkipReason::Symlink)
        .map(|s| s.path.as_str())
        .collect();
    assert_eq!(symlinks, ["link.txt", "linkdir"]);
    assert!(matches!(
        dir.read("link.txt"),
        Err(ContainerError::NotFound(_))
    ));
    assert!(matches!(
        dir.read("linkdir/secret.txt"),
        Err(ContainerError::NotFound(_))
    ));

    // Swap a listed file for a symlink pointing outside after the walk.
    std::fs::remove_file(root.join("a.txt")).unwrap();
    symlink(outside.join("secret.txt"), root.join("a.txt")).unwrap();
    match dir.read("a.txt") {
        Err(ContainerError::UnsafePath { reason, .. }) => {
            assert_eq!(reason, UnsafePathReason::EscapesRoot);
        }
        other => panic!("expected EscapesRoot, got {other:?}"),
    }
}

/// A temporary directory under Cargo's per-target scratch folder (keeps
/// test files inside the build tree).
fn tempdir() -> tempfile::TempDir {
    tempfile::Builder::new()
        .prefix("pbi-core-")
        .tempdir_in(env!("CARGO_TARGET_TMPDIR"))
        .unwrap()
}
