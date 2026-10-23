//! Document session: open → refresh → query, shared by the CLI and the app
//! (the equivalent of Power BI Desktop's local engine session).
//!
//! # Lifecycle
//!
//! 1. [`Session::open`] reads any supported input with
//!    [`pbi_core::open_path`] (PBIP, PBIX, PBIT, `.bim`, TMDL, ABF, …).
//!    No data is loaded yet; every model table exists, empty.
//! 2. [`Session::refresh`] loads the data and computes DAX objects:
//!    - **PBIP / PBIT / `.bim` / TMDL**: each table's import-mode M
//!      partitions run in `pbi-m` (concurrently, on blocking threads) with
//!      the model's shared expressions (parameters such as
//!      `StorageContainerUrl`, other queries) in scope. `AzureStorage.Blobs`
//!      goes through the configured [`BlobResolver`](pbi_m::BlobResolver)
//!      ([`SessionOptions::blob_map`]); without one the table fails with
//!      [`OFFLINE_HINT`]. The query result is projected onto the model's
//!      `sourceColumn`s, renamed to the model column names and typed per
//!      the model (`decimal` → `Decimal128(19, 4)`). `DateTimeZone.UtcNow()`
//!      uses the injected [`Clock`].
//!    - **PBIX / ABF**: the embedded VertiPaq data is decoded per table
//!      (`pbi-vertipaq`); stored calculated columns and tables are used as
//!      they are.
//!    - **PBIP local data cache** ([`RefreshSource::Cache`] /
//!      [`Session::load_cached_data`], or [`RefreshSource::Auto`] when
//!      [`Session::has_cached_data`]): Power BI Desktop's last refresh,
//!      `<Model>.SemanticModel/.pbi/cache.abf`, decoded like PBIX data and
//!      mapped onto the model by name; no M evaluation, no network (see
//!      [`cache`]).
//!    - Then **calculated columns and calculated tables** (`pbi-dax`) run in
//!      dependency order (a topological sort of their DAX references,
//!      measures expanded; cycles are reported), each on a fresh engine
//!      snapshot.
//!    - Finally the session's **derived columns** are re-applied (see
//!      below).
//!
//!    A failed table keeps its previous data and is reported with
//!    [`TableStatus::Error`] (`unsupported`, a `hint`, and whether
//!    [`Session::load_csv`] can stand in); everything depending on it is
//!    [`TableStatus::Skipped`] with the reason. The refresh runs on a
//!    private copy of the engine and commits at the end, so queries during
//!    a refresh see the previous data and cancelling leaves the session
//!    unchanged. Progress is reported through [`RefreshEvent`]s.
//! 3. Queries run on immutable engine snapshots:
//!    [`Session::measures`] / [`Session::evaluate_measure`] (values,
//!    formatted with the measure's `formatString`, or ⚠ errors with the
//!    DAX text), [`Session::page`] / [`Session::row_count`] /
//!    [`Session::column_stats`] for the grid, [`Session::sql`],
//!    [`Session::dax_query`] and [`Session::table_batch`].
//!
//! # Filters
//!
//! [`FilterSpec`] is a serializable list of `table[column] IN / NOT IN
//! values` filters, converted to an engine filter context (relationships
//! propagate it, in both directions where the model says so).
//! [`filters_from_report`] / [`filters_from_page`] translate report and
//! page categorical filters; filters that cannot be translated yet
//! (advanced, Top N, relative date, tuples) are listed in
//! [`FilterSpec::unsupported`] rather than dropped.
//!
//! # Async and cancellation
//!
//! Every engine entry point is `async`, takes a [`CancellationToken`] and
//! must run on a Tokio runtime (the CLI uses a blocking runtime, the app
//! Tauri's); CPU-bound work runs on blocking threads. Nothing panics on bad
//! input: M and DAX problems become per-table or per-measure results.
//!
//! ```no_run
//! # async fn demo() -> Result<(), pbi_session::SessionError> {
//! use pbi_session::{CancellationToken, FilterSpec, RefreshOptions, Session, SessionOptions};
//!
//! let session = Session::open(
//!     "reference/examples/acme-outfitters/AcmeOutfitters.pbip",
//!     SessionOptions {
//!         blob_map: Some("reference/examples/acme-outfitters/blob-map.json".into()),
//!         ..SessionOptions::default()
//!     },
//! )?;
//! let report = session.refresh(RefreshOptions::all()).await?;
//! for t in &report.tables {
//!     println!("{}: {} rows ({:?})", t.name, t.rows, t.status);
//! }
//! for m in session.measures(&FilterSpec::new(), CancellationToken::new()).await? {
//!     println!("{} = {}", m.name, m.display());
//! }
//! # Ok(()) }
//! ```
//!
//! # Row-level security ("View as")
//!
//! [`Session::set_view_as`] (or [`Session::set_effective_roles`]) restricts
//! every query to the rows the given roles allow (each role's DAX table
//! filters, propagated along relationships in their security direction,
//! union over the roles) and sets the user `USERPRINCIPALNAME()` returns;
//! [`Session::clear_view_as`] ends it. See [`security`].
//!
//! # Derived columns
//!
//! [`Session::add_derived_column`] adds a session-only column to a table
//! ([`DerivedColumn::People`] — the "parse people field" transform for
//! Python-style lists of people in [`transforms`] —, a DAX [`DerivedColumn::Expression`]
//! or an [`DerivedColumn::Array`]). It is visible to DAX, SQL, the grid,
//! statistics and table exports, re-applied after every refresh and CSV
//! load, and never written to model files (see [`derived`]).

pub mod cache;
mod csv_load;
pub mod derived;
mod error;
pub mod export;
mod filters;
mod options;
mod query;
pub mod refresh;
mod report;
pub mod security;
mod session;
pub mod transforms;

pub use cache::{CACHE_ENTRY, CacheInfo};
pub use derived::{
    DERIVED_ANNOTATION, DERIVED_KIND_ANNOTATION, DerivedColumn, DerivedColumnInfo, DerivedKind,
    is_derived,
};
pub use error::SessionError;
pub use export::{
    CsvExportOptions, ExportError, ExportSummary, MarkdownOptions, ModelDoc, ModelDocOptions,
    ParquetCompression, ParquetExportOptions, annotate_batch, model_doc, model_to_json,
    model_to_markdown, write_csv, write_parquet,
};
pub use filters::{
    ColumnFilterSpec, FilterSpec, UnsupportedFilter, filters_from, filters_from_page,
    filters_from_report,
};
pub use options::{
    CsvOptions, CsvSource, ProgressFn, RefreshOptions, RefreshSource, SessionLimits, SessionOptions,
};
pub use pbi_engine::{CancellationToken, ColumnStats, PageRequest, SortSpec, Value};
pub use pbi_m::{Clock, FixedClock, SystemClock};
pub use query::{MeasureValue, value_to_json};
pub use report::{
    ColumnRefresh, LOAD_MANUALLY_HINT, OFFLINE_HINT, RefreshEvent, RefreshKind, RefreshReport,
    TableRefresh, TableStatus,
};
pub use security::{VIEW_AS_FAILED, ViewAs};
pub use session::{Session, TableInfo};
