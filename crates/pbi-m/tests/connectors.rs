//! Blob resolver and Arrow conversion tests on synthetic files (written under Cargo's
//! per-target temporary directory).

use std::path::PathBuf;
use std::sync::Arc;

use arrow::array::{Array, BooleanArray, Date32Array, Float64Array, Int64Array, StringArray};
use arrow::datatypes::{DataType, TimeUnit};
use indexmap::IndexMap;
use pbi_m::{
    BlobResolver, Engine, EvalOptions, LimitKind, Limits, MError, MapBlobResolver,
    ResolverErrorKind, Value,
};

const URL: &str = "https://acct.blob.core.windows.net/data";

fn scratch(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("pbi-m-{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("csv")).unwrap_or_else(|e| panic!("{e}"));
    dir
}

fn write(path: PathBuf, contents: &[u8]) {
    std::fs::write(&path, contents).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
}

fn setup(name: &str) -> (PathBuf, MapBlobResolver) {
    let dir = scratch(name);
    write(
        dir.join("csv/people.csv"),
        b"Name,Age,Joined,Active\r\nAnn,30,2024-01-31,true\r\n\"Lee, Bo\",,1/2/2023,false\r\n",
    );
    write(
        dir.join("blob-map.json"),
        format!(
            r#"{{"containers": {{"{URL}/": {{"team/people.csv": "csv/people.csv", "team/gone.csv": "csv/gone.csv"}}}}}}"#
        )
        .as_bytes(),
    );
    let resolver =
        MapBlobResolver::from_map_file(dir.join("blob-map.json")).unwrap_or_else(|e| panic!("{e}"));
    (dir, resolver)
}

fn engine(resolver: MapBlobResolver, limits: Limits) -> Engine {
    Engine::new(EvalOptions {
        resolver: Arc::new(resolver),
        limits,
        ..EvalOptions::default()
    })
}

fn shared() -> IndexMap<String, String> {
    let mut shared = IndexMap::new();
    shared.insert(
        "Url".to_owned(),
        format!("\"{URL}\" meta [IsParameterQuery = true]"),
    );
    shared
}

const QUERY: &str = r#"let
    Source = AzureStorage.Blobs(Url),
    Csv = Csv.Document(Source{[Name = "team/people.csv"]}[Content], [Delimiter = ",", Encoding = 65001, QuoteStyle = QuoteStyle.Csv]),
    Promoted = Table.PromoteHeaders(Csv, [PromoteAllScalars = true]),
    Typed = Table.TransformColumnTypes(Promoted, {{"Age", Int64.Type}, {"Joined", type date}, {"Active", type logical}})
in
    Typed"#;

#[test]
fn azure_blobs_navigation_table() {
    let (_dir, resolver) = setup("nav");
    let engine = engine(resolver, Limits::default());
    let v = engine
        .evaluate_query(
            "Table.SelectColumns(AzureStorage.Blobs(Url), {\"Name\", \"Extension\", \"Folder Path\"})",
            &shared(),
        )
        .unwrap_or_else(|e| panic!("{e}"));
    let expected = Engine::default()
        .evaluate(&format!(
            r#"#table({{"Name", "Extension", "Folder Path"}}, {{{{"team/people.csv", ".csv", "{URL}/team/"}}, {{"team/gone.csv", ".csv", "{URL}/team/"}}}})"#
        ))
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(v, expected);
    let size = engine
        .evaluate_query(
            "AzureStorage.Blobs(Url){[Name = \"team/people.csv\"]}[Attributes][Size]",
            &shared(),
        )
        .unwrap_or_else(|e| panic!("{e}"));
    assert!(matches!(size, Value::Number(n) if n > 50.0), "{size}");
}

#[test]
fn csv_blob_to_typed_arrow() {
    let (_dir, resolver) = setup("arrow");
    let engine = engine(resolver, Limits::default());
    let expected = vec![
        ("Name".to_owned(), DataType::Utf8),
        ("Age".to_owned(), DataType::Int64),
        ("Joined".to_owned(), DataType::Date32),
        ("Active".to_owned(), DataType::Boolean),
    ];
    let batch = engine
        .evaluate_partition_to_arrow(QUERY, &shared(), &expected)
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(batch.num_rows(), 2);
    let col = |i: usize| batch.column(i).as_any();
    let names = col(0)
        .downcast_ref::<StringArray>()
        .unwrap_or_else(|| panic!("utf8"));
    assert_eq!((names.value(0), names.value(1)), ("Ann", "Lee, Bo"));
    let ages = col(1)
        .downcast_ref::<Int64Array>()
        .unwrap_or_else(|| panic!("int64"));
    assert_eq!(ages.value(0), 30);
    assert!(ages.is_null(1));
    let joined = col(2)
        .downcast_ref::<Date32Array>()
        .unwrap_or_else(|| panic!("date32"));
    assert_eq!(joined.value(0), 19_753); // 2024-01-31
    let active = col(3)
        .downcast_ref::<BooleanArray>()
        .unwrap_or_else(|| panic!("bool"));
    assert!(active.value(0) && !active.value(1));

    // Without an expected schema, types follow the M column types.
    let inferred = engine
        .evaluate_partition_to_arrow(QUERY, &shared(), &[])
        .unwrap_or_else(|e| panic!("{e}"));
    let types: Vec<DataType> = inferred
        .schema_ref()
        .fields()
        .iter()
        .map(|f| f.data_type().clone())
        .collect();
    assert_eq!(
        types,
        vec![
            DataType::Utf8,
            DataType::Int64,
            DataType::Date32,
            DataType::Boolean
        ]
    );

    // Coercion to other types, e.g. numbers stored as text to Float64.
    let batch = engine
        .evaluate_partition_to_arrow(
            r#"#table({"x", "t"}, {{"1.5", #datetime(2024, 1, 1, 0, 0, 1)}})"#,
            &IndexMap::new(),
            &[
                ("x".to_owned(), DataType::Float64),
                (
                    "t".to_owned(),
                    DataType::Timestamp(TimeUnit::Microsecond, None),
                ),
            ],
        )
        .unwrap_or_else(|e| panic!("{e}"));
    let x = batch
        .column(0)
        .as_any()
        .downcast_ref::<Float64Array>()
        .map(|a| a.value(0));
    assert_eq!(x, Some(1.5));
}

#[test]
fn cell_errors_and_bad_coercions_name_row_and_column() {
    let err = Engine::default()
        .evaluate_partition_to_arrow(
            r#"Table.TransformColumnTypes(#table({"a"}, {{"1"}, {"x"}}), {"a", type number})"#,
            &IndexMap::new(),
            &[("a".to_owned(), DataType::Float64)],
        )
        .err()
        .unwrap_or_else(|| panic!("expected an error"));
    let msg = err.to_string();
    assert!(msg.contains("Row 2, column 'a'"), "{msg}");
    assert!(msg.contains("DataFormat.Error"), "{msg}");

    let err = Engine::default()
        .evaluate_partition_to_arrow(
            r#"#table({"a"}, {{"abc"}})"#,
            &IndexMap::new(),
            &[("a".to_owned(), DataType::Int64)],
        )
        .err()
        .unwrap_or_else(|| panic!("expected an error"));
    assert!(err.to_string().contains("Row 1, column 'a'"), "{err}");

    let err = Engine::default()
        .evaluate_partition_to_arrow("42", &IndexMap::new(), &[])
        .err()
        .unwrap_or_else(|| panic!("expected an error"));
    assert!(err.to_string().contains("instead of a table"), "{err}");
}

#[test]
fn resolver_errors_are_clear() {
    let (_dir, resolver) = setup("errors");
    let err = resolver
        .open(URL, "team/gone.csv")
        .err()
        .unwrap_or_else(|| panic!("expected an error"));
    assert_eq!(err.kind, ResolverErrorKind::NotFound);
    assert!(err.message.contains("csv/gone.csv"), "{err}");
    assert!(
        resolver
            .list("https://acct.blob.core.windows.net/other")
            .is_err()
    );

    let engine = engine(resolver, Limits::default());
    let r = engine.evaluate_query(
        "Csv.Document(AzureStorage.Blobs(Url){[Name = \"team/gone.csv\"]}[Content])",
        &shared(),
    );
    assert!(matches!(r, Err(MError::Resolver(_))), "{r:?}");
    // A blob that is not in the container is a (catchable) navigation error.
    let r = engine.evaluate_query(
        "try AzureStorage.Blobs(Url){[Name = \"nope.csv\"]}[Content] otherwise \"fallback\"",
        &shared(),
    );
    assert_eq!(r.ok(), Some(Value::text("fallback")));
}

#[test]
fn blob_size_limit_is_enforced() {
    let (_dir, resolver) = setup("limit");
    let engine = engine(
        resolver,
        Limits {
            max_binary_bytes: 16,
            ..Limits::default()
        },
    );
    let r = engine.evaluate_query(QUERY, &shared());
    assert!(
        matches!(
            r,
            Err(MError::LimitExceeded {
                kind: LimitKind::Bytes,
                ..
            })
        ),
        "{r:?}"
    );
}

#[test]
fn map_paths_cannot_escape_the_map_directory() {
    let dir = scratch("escape");
    write(
        dir.join("blob-map.json"),
        br#"{"containers": {"u": {"x": "../outside.csv"}}}"#,
    );
    assert!(MapBlobResolver::from_map_file(dir.join("blob-map.json")).is_err());
    write(
        dir.join("blob-map.json"),
        br#"{"containers": {"u": {"x": "/etc/hosts"}}}"#,
    );
    assert!(MapBlobResolver::from_map_file(dir.join("blob-map.json")).is_err());

    #[cfg(unix)]
    {
        let outside = scratch("escape-target");
        write(outside.join("secret.csv"), b"s");
        let link = dir.join("csv/link.csv");
        std::os::unix::fs::symlink(outside.join("secret.csv"), &link)
            .unwrap_or_else(|e| panic!("{e}"));
        write(
            dir.join("blob-map.json"),
            br#"{"containers": {"u": {"x": "csv/link.csv"}}}"#,
        );
        let resolver = MapBlobResolver::from_map_file(dir.join("blob-map.json"))
            .unwrap_or_else(|e| panic!("{e}"));
        let err = resolver
            .open("u", "x")
            .err()
            .unwrap_or_else(|| panic!("escape allowed"));
        assert_eq!(err.kind, ResolverErrorKind::Invalid);
    }
}
