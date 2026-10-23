//! PBIR / PBIR-Legacy / PBIX `Layout` report definitions for pbiview.
//!
//! # Formats
//!
//! - **PBIR**: a folder of JSON files (`definition.pbir`, `.platform`,
//!   `definition/report.json`, `version.json`, `pages/pages.json`,
//!   `pages/<id>/page.json`, `pages/<id>/visuals/<id>/visual.json`, themes
//!   under `StaticResources/`). Used by PBIP and by newer PBIX files
//!   (`Report/definition/`).
//! - **PBIR-Legacy / PBIX `Layout`**: one JSON document with stringified
//!   nested JSON (`report.json` in older PBIP, UTF-16LE `Report/Layout` in
//!   PBIX). See [`legacy`].
//!
//! # API
//!
//! - [`FileSource`]: container-agnostic file access (`pbi-core` backs it
//!   with a directory or ZIP).
//! - [`read_pbir`] → [`PbirReport`] (typed files + every file verbatim +
//!   diagnostics); [`write_pbir`] writes it back (byte-identical for
//!   unedited files, see the round-trip tests).
//! - [`read_legacy_layout`] → [`LegacyReport`].
//! - [`to_report_ast`] / [`legacy_to_report_ast`] → [`pbi_model::Report`]
//!   with resolved `SourceRef` aliases, typed filters and origins.
//! - [`theme`]: theme types and `ThemeDataColor` / color resolution;
//!   [`objects`]: formatting-object accessors.
//!
//! # Losslessness
//!
//! Every type is declared with an internal `json_object!` macro: known
//! properties are `Option<T>` fields, everything else (and explicit
//! `null`s) is kept in `extra`, source key order is recorded in
//! `key_order`, and numbers are [`serde_json::Number`]s that keep their
//! source digits. Re-serializing any parsed file yields JSON identical to
//! the input, key order included ([`json::json_identical`]). This avoids
//! `#[serde(flatten)]`, which mangles numbers under `arbitrary_precision`
//! and cannot preserve key order between typed and unknown fields.

pub mod convert;
pub mod error;
pub mod json;
pub mod legacy;
pub mod objects;
pub mod pbir;
pub mod query;
pub mod reader;
pub mod source;
pub mod theme;
pub mod writer;

pub use convert::to_report_ast;
pub use error::{PbirError, codes};
pub use json::{JsonStyle, KeyOrder, json_identical};
pub use legacy::{
    LegacyReport, TextEncoding, legacy_to_report_ast, read_legacy_layout, read_legacy_layout_at,
};
pub use pbir::{
    DefinitionPbir, FilterJson, PageJson, PagesJson, Platform, ReportJson, VersionJson,
    VisualContainerJson,
};
pub use query::{ConditionJson, ExprKind, FieldExprJson, QueryExpr};
pub use reader::{PbirFile, PbirFileKind, PbirPage, PbirReport, PbirTheme, PbirVisual, read_pbir};
pub use source::{FileSource, MemoryFileSource, SubdirFileSource};
pub use theme::{ThemeJson, resolve_color, resolve_theme_data_color};
pub use writer::write_pbir;
