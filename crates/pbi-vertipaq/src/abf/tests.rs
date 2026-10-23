//! Unit tests for the ABF container on synthetic backups.

use super::test_support::{AbfBuilder, DB_FOLDER};
use super::*;

fn sample() -> AbfBuilder {
    AbfBuilder::new()
        .file(&format!("{DB_FOLDER}.xml"), b"<Load/>".to_vec())
        .file("1.CryptKey.bin", vec![7u8; 16])
        .file("metadata.sqlitedb", b"SQLite format 3\0rest".to_vec())
        .file(
            r"Sales & Co (12).tbl\15.prt\80.Sales & Co (12).Amount (20).0.idf",
            vec![1, 2, 3],
        )
        .file(
            r"Sales & Co (12).tbl\80.Sales & Co (12).Amount (20).dictionary",
            vec![4, 5],
        )
        .file("empty.bin", Vec::new())
}

#[test]
fn parses_synthetic_backup() {
    let archive = AbfArchive::parse(sample().build()).unwrap();
    let header = archive.header();
    assert_eq!(header.backup_restore_sync_version, Some(140));
    assert_eq!(header.data_offset, Some(4096));
    assert_eq!(header.directory_offset % PAGE_SIZE as u64, 0);
    assert_eq!(header.file_count, Some(archive.directory().len() as u64));
    // ADDITIONAL_LOG + PARTITIONS + 6 files + LOG.
    assert_eq!(archive.directory().len(), 9);
    assert_eq!(archive.files().len(), 6);

    let paths: Vec<_> = archive
        .files()
        .iter()
        .map(|f| f.logical_path.as_str())
        .collect();
    assert_eq!(
        paths,
        [
            "testdb.1.db.xml",
            "1.CryptKey.bin",
            "metadata.sqlitedb",
            "Sales & Co (12).tbl/15.prt/80.Sales & Co (12).Amount (20).0.idf",
            "Sales & Co (12).tbl/80.Sales & Co (12).Amount (20).dictionary",
            "empty.bin",
        ]
    );
    // No absolute server path survives.
    assert!(
        archive
            .files()
            .iter()
            .all(|f| !f.logical_path.contains("Users"))
    );

    assert_eq!(
        archive.read("metadata.sqlitedb").unwrap(),
        b"SQLite format 3\0rest"
    );
    assert_eq!(
        archive
            .read(r"sales & co (12).TBL\15.prt\80.Sales & Co (12).Amount (20).0.IDF")
            .unwrap(),
        &[1, 2, 3]
    );
    let dict = archive.find_suffix(".dictionary").unwrap();
    assert_eq!(dict.extension(), Some("dictionary"));
    assert_eq!(
        dict.file_name(),
        "80.Sales & Co (12).Amount (20).dictionary"
    );
    assert_eq!(archive.read(dict).unwrap(), &[4, 5]);
    assert_eq!(archive.read("empty.bin").unwrap(), b"");
    assert_eq!(
        archive.metadata_file().unwrap().logical_path,
        "metadata.sqlitedb"
    );
    assert_eq!(archive.files_with_suffix(".IDF").count(), 1);
    assert_eq!(
        archive
            .find_file_name("80.SALES & CO (12).AMOUNT (20).0.IDF")
            .map(|f| f.storage_path.as_str()),
        Some("00000000000000000004")
    );
    assert!(matches!(
        archive.read("missing.idf"),
        Err(AbfError::FileNotFound(_))
    ));
    assert_eq!(
        archive.read_storage("ADDITIONAL_LOG").unwrap(),
        b"additional-log"
    );
    assert!(
        archive
            .read_storage("LOG")
            .unwrap()
            .starts_with(&[0xFF, 0xFE])
    );

    let log = archive.log_info();
    assert_eq!(log.is_tabular, Some(true));
    assert_eq!(log.languages, [1033]);
    assert_eq!(log.file_groups.len(), 2);
    assert_eq!(log.file_groups[0].class, Some(100002));
    assert_eq!(log.file_groups[1].file_count, 4);
    assert_eq!(
        log.file_groups[1].persist_location_name.as_deref(),
        Some(DB_FOLDER)
    );
    let debug = format!("{archive:?}");
    assert!(debug.contains("files: 6") && !debug.contains("Users"));

    // Cheap clones share the data.
    let clone = archive.clone();
    assert_eq!(clone.as_bytes().as_ptr(), archive.as_bytes().as_ptr());
}

#[test]
fn crc_markers_are_excluded_from_content() {
    let archive = AbfArchive::parse(sample().crc_markers().build()).unwrap();
    assert!(archive.header().error_code);
    let file = archive.find("metadata.sqlitedb").unwrap();
    assert_eq!(file.stored_size, file.size + CRC_MARKER_LEN);
    assert_eq!(archive.read(file).unwrap(), b"SQLite format 3\0rest");
    assert_eq!(archive.read("empty.bin").unwrap(), b"");
    assert_eq!(archive.read_storage("PARTITIONS").unwrap().len(), 28);
}

#[test]
fn compressed_files_are_decompressed_by_read_file() {
    use crate::xpress8::test_support::{chunk, literal_then_match};
    let archive = AbfArchive::parse(
        sample()
            .file_stored_as(
                "packed.bin",
                b"abcabcabcabc".to_vec(),
                chunk(12, &literal_then_match(b"abc", 3, 9)),
            )
            .apply_compression()
            .crc_markers()
            .build(),
    )
    .unwrap();
    assert!(archive.is_compressed());
    assert!(matches!(
        archive.read("metadata.sqlitedb"),
        Err(AbfError::CompressedFile { path }) if path == "metadata.sqlitedb"
    ));
    let stored = archive.read_stored("metadata.sqlitedb").unwrap();
    assert_eq!(&stored[4..], b"SQLite format 3\0rest");
    assert_eq!(
        archive.read_file("metadata.sqlitedb").unwrap().as_ref(),
        b"SQLite format 3\0rest"
    );
    let file = archive.find("packed.bin").unwrap();
    assert_eq!(file.log_size, Some(12));
    assert_eq!(archive.read_file(file).unwrap().as_ref(), b"abcabcabcabc");
    assert_eq!(archive.read_file("empty.bin").unwrap().as_ref(), b"");

    // A stored size that disagrees with the backup log is an error, not a short read.
    let lying = AbfArchive::parse(
        sample()
            .file_stored_as("bad.bin", b"0123456789".to_vec(), chunk(3, b"abc"))
            .apply_compression()
            .build(),
    )
    .unwrap();
    assert!(matches!(
        lying.read_file("bad.bin"),
        Err(AbfError::Decompress { path, .. }) if path == "bad.bin"
    ));
}

#[test]
fn uncompressed_read_file_borrows() {
    let archive = AbfArchive::parse(sample().build()).unwrap();
    assert!(!archive.is_compressed());
    assert!(matches!(
        archive.read_file("metadata.sqlitedb").unwrap(),
        std::borrow::Cow::Borrowed(b) if b == b"SQLite format 3\0rest"
    ));
}

#[test]
fn accepts_any_byte_container_and_open_variants() {
    let bytes = sample().build();
    let shared: Arc<[u8]> = bytes.clone().into();
    assert_eq!(AbfArchive::parse(shared).unwrap().files().len(), 6);
    assert_eq!(
        AbfArchive::open(bytes.clone(), &Limits::default())
            .unwrap()
            .files()
            .len(),
        6
    );
    assert_eq!(
        open_pbix_datamodel(&bytes, &Limits::default())
            .unwrap()
            .files()
            .len(),
        6
    );
    assert_eq!(sniff(&bytes), Some(DataModelFormat::Uncompressed));
    assert!(matches!(
        open_pbix_datamodel(b"PK\x03\x04", &Limits::default()),
        Err(AbfError::UnknownSignature)
    ));
    assert!(matches!(
        AbfArchive::open(Vec::new(), &Limits::default()),
        Err(AbfError::UnknownSignature)
    ));
    assert!(matches!(
        AbfArchive::parse(b"not an abf".to_vec()),
        Err(AbfError::NotAbf)
    ));
}

#[test]
fn archive_is_shareable_across_threads() {
    fn assert_send_sync<T: Send + Sync + Clone>() {}
    assert_send_sync::<AbfArchive>();
    let archive = AbfArchive::parse(sample().build()).unwrap();
    let handle = {
        let archive = archive.clone();
        std::thread::spawn(move || archive.read("metadata.sqlitedb").map(<[u8]>::len).ok())
    };
    assert_eq!(handle.join().unwrap(), Some(20));
}

#[test]
fn encrypted_backups_are_rejected() {
    assert!(matches!(
        AbfArchive::parse(sample().encrypted().build()),
        Err(AbfError::Encrypted)
    ));
}

#[test]
fn truncation_at_every_section_is_an_error() {
    let (bytes, layout) = sample().build_with_layout();
    for cut in [
        0,
        10,
        71,
        72,
        100,
        PAGE_SIZE - 1,
        PAGE_SIZE,
        layout.log_offset + 10,
        layout.directory_offset,
        layout.directory_offset + layout.directory_size / 2,
        layout.directory_offset + layout.directory_size - 1,
    ] {
        let truncated = bytes[..cut].to_vec();
        assert!(
            AbfArchive::parse(truncated).is_err(),
            "truncation at {cut} must fail"
        );
    }
    // Every prefix fails without panicking (sampled for speed).
    for cut in (0..bytes.len()).step_by(97) {
        let _ = AbfArchive::parse(bytes[..cut].to_vec());
    }
}

fn header_with(directory_offset: &str, directory_size: &str) -> String {
    format!(
        "<BackupLog><ErrorCode>false</ErrorCode><m_cbOffsetHeader>{directory_offset}</m_cbOffsetHeader><DataSize>{directory_size}</DataSize></BackupLog>"
    )
}

#[test]
fn bad_header_fields_are_errors() {
    let (_, layout) = sample().build_with_layout();
    let ok = layout.directory_offset.to_string();
    let size = layout.directory_size.to_string();
    // Sanity check: the minimal header works.
    assert!(AbfArchive::parse(sample().header_xml(&header_with(&ok, &size)).build()).is_ok());

    let cases = [
        (header_with("99999999999", &size), "offset past end"),
        (header_with(&ok, "18446744073709551615"), "huge size"),
        (header_with("18446744073709551615", "2"), "offset overflow"),
        (header_with("-1", &size), "negative offset"),
        (header_with(&ok, "abc"), "non-numeric size"),
        (
            "<BackupLog><DataSize>1</DataSize></BackupLog>".into(),
            "missing offset",
        ),
        ("<BackupLog>".into(), "unterminated"),
        ("<Other/>".into(), "wrong root"),
        ("".into(), "empty"),
        (
            format!(
                "<!DOCTYPE x [<!ENTITY e \"1\">]>{}",
                header_with(&ok, &size)
            ),
            "DTD",
        ),
    ];
    for (xml, label) in cases {
        let result = AbfArchive::parse(sample().header_xml(&xml).build());
        assert!(result.is_err(), "{label}: expected an error");
    }
    // Directory of the right size but pointing into the files section.
    let result = AbfArchive::parse(sample().header_xml(&header_with("4096", &size)).build());
    assert!(matches!(
        result,
        Err(AbfError::Encoding(_) | AbfError::Xml { .. })
    ));
}

#[test]
fn bad_directories_are_errors() {
    let entry = |path: &str, size: &str, offset: &str| {
        format!(
            "<BackupFile><Path>{path}</Path><Size>{size}</Size><m_cbOffsetHeader>{offset}</m_cbOffsetHeader></BackupFile>"
        )
    };
    let cases = [
        (
            format!(
                "<VirtualDirectory>{}</VirtualDirectory>",
                entry("LOG", "10", "999999999")
            ),
            "offset past end",
        ),
        (
            format!(
                "<VirtualDirectory>{}</VirtualDirectory>",
                entry("LOG", "18446744073709551615", "4096")
            ),
            "huge size",
        ),
        (
            format!(
                "<VirtualDirectory>{}</VirtualDirectory>",
                entry("LOG", "1", "18446744073709551615")
            ),
            "offset overflow",
        ),
        ("<VirtualDirectory></VirtualDirectory>".into(), "no entries"),
        (
            "<VirtualDirectory><BackupFile><Path>LOG</Path></BackupFile></VirtualDirectory>".into(),
            "missing size",
        ),
        (
            format!(
                "<VirtualDirectory>{}</VirtualDirectory>",
                entry("", "1", "4096")
            ),
            "empty path",
        ),
        ("<VirtualDirectory><BackupFile>".into(), "malformed"),
        ("\u{0}\u{0}\u{0}".into(), "garbage"),
        (
            format!(
                "<VirtualDirectory>{}</VirtualDirectory>",
                entry("LOG", "16", "4096")
            ),
            "log is not XML",
        ),
    ];
    for (xml, label) in cases {
        let result = AbfArchive::parse(sample().directory_bytes(xml.into_bytes()).build());
        assert!(result.is_err(), "{label}: expected an error");
    }
    // Invalid UTF-8 in the directory.
    let result = AbfArchive::parse(
        sample()
            .directory_bytes(vec![b'<', 0xC3, 0x28, b'>'])
            .build(),
    );
    assert!(result.is_err());
}

#[test]
fn crc_marker_larger_than_file_is_an_error() {
    // With ErrorCode set, every stored entry must be at least 4 bytes long.
    let directory = "<VirtualDirectory><BackupFile><Path>LOG</Path><Size>2</Size><m_cbOffsetHeader>4096</m_cbOffsetHeader></BackupFile></VirtualDirectory>";
    let result = AbfArchive::parse(
        sample()
            .crc_markers()
            .directory_bytes(directory.as_bytes().to_vec())
            .build(),
    );
    assert!(matches!(result, Err(AbfError::OutOfBounds { .. })));
}

#[test]
fn log_entries_without_directory_entries_are_skipped() {
    let (bytes, layout) = sample().build_with_layout();
    // Keep only the LOG entry in the directory.
    let archive = AbfArchive::parse(bytes.clone()).unwrap();
    let log = archive
        .directory()
        .iter()
        .find(|e| e.storage_path == LOG_STORAGE_NAME)
        .unwrap()
        .clone();
    let directory = format!(
        "<VirtualDirectory><BackupFile><Path>LOG</Path><Size>{}</Size><m_cbOffsetHeader>{}</m_cbOffsetHeader></BackupFile></VirtualDirectory>",
        log.size, log.offset
    );
    assert_eq!(log.offset as usize, layout.log_offset);
    let archive =
        AbfArchive::parse(sample().directory_bytes(directory.into_bytes()).build()).unwrap();
    assert!(archive.files().is_empty());
    assert_eq!(archive.directory().len(), 1);
}

#[test]
fn random_mutations_never_panic() {
    let bytes = sample().crc_markers().build();
    let mut state = 0x9E37_79B9_7F4A_7C15u64;
    let mut next = || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };
    for _ in 0..300 {
        let mut mutated = bytes.clone();
        for _ in 0..(next() % 8 + 1) {
            let at = (next() as usize) % mutated.len();
            mutated[at] = next() as u8;
        }
        fuzz_abf(&mutated);
    }
    fuzz_abf(&[]);
    fuzz_abf(&bytes);
}
