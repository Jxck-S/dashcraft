//! ABF container and `metadata.sqlitedb` tests on the `DataModel` of the
//! Microsoft sample PBIX files (`reference/samples/microsoft`, MIT).
//!
//! Every test prints `SKIP: ...` and returns when a sample is absent.
//! Assertions cover structure, names and counts. The model facts come from
//! the samples' `expected.json` (cross-checked with pbixray 0.15.5); the
//! backup header, directory, file groups and metadata table counts were
//! checked against pbixray's ABF parser and Python's `sqlite3` over the
//! same `metadata.sqlitedb`, run read-only (see `FORMAT-NOTES.md` §5).

// The fixture helpers below are test code too.
#![allow(clippy::unwrap_used)]

mod common;

use std::collections::BTreeMap;

use common::{ALL_SAMPLES, REVENUE_OPPORTUNITIES, Sample, datamodel, expected};
use pbi_model::{PartitionSource, TableKind};
use pbi_vertipaq::abf::{
    ADDITIONAL_LOG_STORAGE_NAME, LOG_STORAGE_NAME, PARTITIONS_STORAGE_NAME, sniff,
};
use pbi_vertipaq::metadata::{ColumnKindCode, DictionaryEncoding};
use pbi_vertipaq::xpress9::DataModelFormat;
use pbi_vertipaq::{
    AbfArchive, Limits, LoadedModel, MetadataDb, ModelOptions, load_model, open_pbix_datamodel,
};

fn open_archive(sample: Sample) -> Option<AbfArchive> {
    let bytes = datamodel(sample)?;
    assert_eq!(sniff(&bytes), Some(DataModelFormat::Xpress9SingleThreaded));
    Some(open_pbix_datamodel(&bytes, &Limits::default()).unwrap())
}

fn load(sample: Sample) -> Option<(AbfArchive, LoadedModel)> {
    let archive = open_archive(sample)?;
    let loaded = load_model(&archive, &ModelOptions::default()).unwrap();
    Some((archive, loaded))
}

fn kind_name(kind: TableKind) -> &'static str {
    match kind {
        TableKind::Regular => "regular",
        TableKind::CalculatedTable => "calculatedTable",
        TableKind::CalculationGroup => "calculationGroup",
        _ => "other",
    }
}

#[test]
fn fixture_backup_structure() {
    let Some(archive) = open_archive(REVENUE_OPPORTUNITIES) else {
        return;
    };
    let header = archive.header();
    assert_eq!(header.backup_restore_sync_version, Some(140));
    assert!(!header.error_code);
    assert!(!header.apply_compression);
    assert!(!header.encryption_flag);
    assert_eq!(header.data_offset, Some(4096));
    assert_eq!(header.directory_offset, 1_445_888);
    assert_eq!(header.directory_offset % 4096, 0);

    // 453 directory entries = ADDITIONAL_LOG + PARTITIONS + 450 model files + LOG.
    assert_eq!(header.file_count, Some(453));
    assert_eq!(archive.directory().len(), 453);
    for special in [
        ADDITIONAL_LOG_STORAGE_NAME,
        PARTITIONS_STORAGE_NAME,
        LOG_STORAGE_NAME,
    ] {
        assert!(archive.read_storage(special).is_ok(), "{special}");
    }
    assert_eq!(archive.files().len(), 450);
    assert_eq!(archive.directory().len(), archive.files().len() + 3);

    let mut by_extension: BTreeMap<&str, usize> = BTreeMap::new();
    for file in archive.files() {
        *by_extension
            .entry(file.extension().unwrap_or(""))
            .or_default() += 1;
        assert_eq!(archive.read(file).unwrap().len() as u64, file.size);
        assert_eq!(file.log_size, Some(file.size));
        // Logical paths are model-relative: no drive, UNC prefix or user directory
        // (the sample's backup log names the author's workspace folder).
        let path = &file.logical_path;
        assert!(!path.contains(':') && !path.contains('\\') && !path.starts_with('/'));
        assert!(!path.to_lowercase().contains("users/"), "{path}");
        assert_eq!(
            archive.find(path).map(|f| f.storage_path.as_str()),
            Some(file.storage_path.as_str())
        );
    }
    let expected: BTreeMap<&str, usize> = [
        ("bin", 1),
        ("dictionary", 36),
        ("hidx", 33),
        ("idf", 189),
        ("idfmeta", 189),
        ("sqlitedb", 1),
        ("xml", 1),
    ]
    .into();
    assert_eq!(by_extension, expected);

    let metadata = archive.metadata_file().unwrap();
    assert_eq!(metadata.logical_path, "metadata.sqlitedb");
    assert_eq!(
        archive
            .find_suffix("METADATA.SQLITEDB")
            .map(|f| &f.logical_path),
        Some(&metadata.logical_path)
    );
    assert!(
        archive
            .read(metadata)
            .unwrap()
            .starts_with(b"SQLite format 3\0")
    );

    let log = archive.log_info();
    assert_eq!(log.is_tabular, Some(true));
    assert_eq!(
        log.file_groups
            .iter()
            .map(|g| g.file_count)
            .collect::<Vec<_>>(),
        [2, 448]
    );
    assert!(
        !format!("{archive:?} {log:?}")
            .to_lowercase()
            .contains("users")
    );
}

/// Every sample's model has the tables, columns, measures and relationships
/// `expected.json` lists.
#[test]
fn fixture_models_match_expected_json() {
    for sample in ALL_SAMPLES {
        let Some((_, loaded)) = load(sample) else {
            continue;
        };
        let want = &expected(sample)["model"];
        let model = &loaded.model;
        let label = sample.file;
        assert_eq!(model.name, "Model", "{label}");
        assert_eq!(
            model.culture.as_deref(),
            want["culture"].as_str(),
            "{label}"
        );
        assert_eq!(
            model.compatibility_level(),
            want["compatibilityLevel"].as_i64(),
            "{label}"
        );
        assert!(loaded.schema_version.is_some(), "{label}");

        let tables: Vec<(&str, &str, bool, usize, usize)> = model
            .tables
            .iter()
            .map(|t| {
                (
                    t.name.as_str(),
                    kind_name(t.kind()),
                    t.is_hidden,
                    t.columns.len(),
                    t.measures.len(),
                )
            })
            .collect();
        let expected_tables: Vec<(&str, &str, bool, usize, usize)> = want["tables"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| {
                (
                    t["name"].as_str().unwrap(),
                    t["kind"].as_str().unwrap(),
                    t["hidden"].as_bool().unwrap(),
                    t["columns"].as_u64().unwrap() as usize,
                    t["measures"].as_u64().unwrap() as usize,
                )
            })
            .collect();
        assert_eq!(tables, expected_tables, "{label}");

        let counts = &want["counts"];
        let columns = || model.tables.iter().flat_map(|t| &t.columns);
        let count = |key: &str| counts[key].as_u64().unwrap() as usize;
        assert_eq!(columns().count(), count("columns"), "{label}");
        assert_eq!(
            columns().filter(|c| c.is_calculated()).count(),
            count("calculatedColumns"),
            "{label}"
        );
        assert_eq!(
            model
                .tables
                .iter()
                .filter(|t| t.kind() == TableKind::CalculatedTable)
                .count(),
            count("calculatedTables"),
            "{label}"
        );
        let measures: Vec<String> = model
            .tables
            .iter()
            .flat_map(|t| {
                t.measures
                    .iter()
                    .map(move |m| format!("{}[{}]", t.name, m.name))
            })
            .collect();
        let mut expected_measures: Vec<String> = want["measures"]["names"]
            .as_array()
            .unwrap()
            .iter()
            .map(|n| n.as_str().unwrap().to_owned())
            .collect();
        let mut sorted = measures.clone();
        sorted.sort();
        expected_measures.sort();
        assert_eq!(sorted, expected_measures, "{label}");
        assert!(
            model
                .tables
                .iter()
                .flat_map(|t| &t.measures)
                .all(|m| !m.expression.trim().is_empty()),
            "{label}"
        );
        assert_eq!(model.relationships.len(), count("relationships"), "{label}");
        // Row-number columns and internal tables are omitted by default.
        assert!(
            columns().all(|c| !c.name.starts_with("RowNumber")),
            "{label}"
        );
        assert!(model.tables.iter().all(|t| !t.name.starts_with("H$")));

        // Imported tables keep their M partition; calculated tables their DAX.
        for table in &model.tables {
            match table.kind() {
                TableKind::Regular => {
                    let m: Vec<_> = table
                        .partitions
                        .iter()
                        .filter_map(|p| match &p.source {
                            PartitionSource::M { expression } => Some(expression),
                            _ => None,
                        })
                        .collect();
                    assert_eq!(m.len(), 1, "{label}: {}", table.name);
                    assert!(!m[0].trim().is_empty(), "{label}: {}", table.name);
                }
                TableKind::CalculatedTable => assert!(
                    !table.calculated_expression().unwrap().trim().is_empty(),
                    "{label}: {}",
                    table.name
                ),
                _ => {}
            }
        }
    }
}

#[test]
fn fixture_storage_map_points_at_existing_files() {
    let Some((archive, loaded)) = load(REVENUE_OPPORTUNITIES) else {
        return;
    };
    assert_eq!(loaded.schema_version, Some(123));
    let storage = &loaded.storage;
    assert!(storage.missing_files(&archive).is_empty());
    // 84 metadata tables: 9 model tables + 75 internal ones (H$, R$, U$).
    assert_eq!(storage.tables.len(), 84);
    assert_eq!(storage.tables.iter().filter(|t| t.is_internal).count(), 75);

    let rows: BTreeMap<String, u64> = expected(REVENUE_OPPORTUNITIES)["model"]["tables"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| {
            (
                t["name"].as_str().unwrap().to_owned(),
                t["rows"].as_u64().unwrap(),
            )
        })
        .collect();
    let mut value_columns = 0;
    for table in storage.tables.iter().filter(|t| !t.is_internal) {
        assert!(loaded.model.table(&table.name).is_some(), "{}", table.name);
        assert!(table.folder.as_deref().is_some_and(|f| f.ends_with(".tbl")));
        assert_eq!(table.row_count, rows[&table.name], "{}", table.name);
        for column in table.columns.iter().filter(|c| c.has_values()) {
            value_columns += 1;
            let label = format!("{}[{}]", table.name, column.name);
            assert_eq!(column.partitions.len(), table.partitions.len(), "{label}");
            for partition in &column.partitions {
                let idf = partition.idf.as_deref().unwrap();
                let idfmeta = partition.idfmeta.as_deref().unwrap();
                assert!(
                    idf.ends_with(".idf") && archive.find(idf).is_some(),
                    "{label}"
                );
                assert!(idfmeta.ends_with(".idfmeta") && archive.find(idfmeta).is_some());
                assert_eq!(partition.segment_count, Some(1), "{label}");
                assert_eq!(partition.record_count, Some(table.row_count), "{label}");
            }
            let dictionary = column.dictionary.as_ref().unwrap();
            match dictionary.encoding {
                DictionaryEncoding::Hash => match column.dictionary_path() {
                    Some(path) => {
                        assert!(path.ends_with(".dictionary") && archive.find(path).is_some());
                    }
                    // Columns whose only value is BLANK have no dictionary file.
                    None => assert_eq!(column.statistics.distinct_states, Some(1), "{label}"),
                },
                DictionaryEncoding::Value => {
                    assert!(column.dictionary_path().is_none(), "{label}");
                }
                other => panic!("{label}: unexpected encoding {other:?}"),
            }
            if let Some(hidx) = column.hidx_path() {
                assert!(hidx.ends_with(".hidx") && archive.find(hidx).is_some());
            }
            assert_eq!(
                column.statistics.row_count,
                Some(table.row_count as i64),
                "{label}"
            );
        }
        assert_eq!(
            table
                .columns
                .iter()
                .filter(|c| c.kind == ColumnKindCode::RowNumber)
                .count(),
            1
        );
    }
    // The same 69 columns pbixray decodes (data, calculated and calculated-table columns).
    assert_eq!(value_columns, 69);

    let opportunity = storage.table("Opportunity").unwrap();
    assert_eq!(opportunity.row_count, 486);
    let account = storage.column("Account", "Account").unwrap();
    assert_eq!(account.data_type, pbi_model::DataType::String);
    assert_eq!(
        account.dictionary.as_ref().unwrap().encoding,
        DictionaryEncoding::Hash
    );
}

#[test]
fn fixture_options_and_direct_metadata_access() {
    let Some(archive) = open_archive(REVENUE_OPPORTUNITIES) else {
        return;
    };
    let db = MetadataDb::from_archive(&archive).unwrap();
    assert!(db.has_table("ColumnStorage"));
    let options = ModelOptions::default()
        .with_system_tables(true)
        .with_row_number_columns(true)
        .with_origin_path(None);
    let model = db.read_model(&options).unwrap();
    assert_eq!(model.tables.len(), 84);
    let internal: Vec<_> = model
        .tables
        .iter()
        .filter(|t| t.extra.contains_key("vertipaq.internalTable"))
        .collect();
    assert_eq!(internal.len(), 75);
    assert!(
        internal
            .iter()
            .all(|t| t.is_hidden && ["H$", "R$", "U$"].iter().any(|p| t.name.starts_with(p)))
    );
    // Internal tables add no relationships.
    assert_eq!(model.relationships.len(), 6);
    assert!(model.tables.iter().all(|t| t.origin.is_none()));
    assert_eq!(
        model
            .tables
            .iter()
            .flat_map(|t| &t.columns)
            .filter(|c| c.name.starts_with("RowNumber-"))
            .count(),
        9
    );
    assert_eq!(db.storage_map().unwrap().tables.len(), 84);
}
