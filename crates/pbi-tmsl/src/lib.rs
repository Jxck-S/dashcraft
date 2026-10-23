//! TMSL (Tabular Model Scripting Language) JSON reader and writer.
//!
//! TMSL is the JSON form of a tabular database: legacy PBIP `model.bim`,
//! Tabular Editor / SSAS `.bim` files, Fabric Git exports, and the PBIT
//! `DataModelSchema` entry (UTF-16LE). This crate maps it to and from the
//! TOM-like [`pbi_model::Model`].
//!
//! # API
//!
//! - Reading: [`read_bim`] (text), [`read_data_model_schema`] (bytes in any
//!   supported encoding, also fine for `model.bim` bytes), or
//!   [`read_text`]/[`read_bytes`] with [`ReadOptions`] (source path for
//!   [`pbi_model::Origin`]s). The result, [`LoadedTmsl`], carries the model
//!   (with `model.database` set from the root object), diagnostics and the
//!   detected [`TmslFormat`].
//! - Writing: [`write_bim`] (UTF-8 text, Tabular Editor layout),
//!   [`write_data_model_schema`] (UTF-16LE bytes, no BOM, as Power BI
//!   Desktop stores the entry), or [`write_text`]/[`write_bytes`]/[`to_json`]
//!   with [`WriteOptions`]. [`LoadedTmsl::write_options`] reproduces the
//!   source layout.
//!
//! # Mapping
//!
//! Database root → [`pbi_model::Database`]; `model` → [`Model`]; tables,
//! columns (`type` `data`/`calculated`/`calculatedTableColumn`/`rowNumber`
//! → [`pbi_model::ColumnKind`]), measures, partitions (`source.type` `m`,
//! `calculated`, `query` (also when `type` is absent, as in legacy files),
//! `entity`, anything else → [`pbi_model::PartitionSource::Unknown`]),
//! hierarchies/levels, relationships (TOM defaults when absent), named
//! expressions (M `meta [...]` split into `meta`), roles/table permissions
//! (`tablePermissions[].name` is the table), calculation groups
//! (`calculationGroup` → [`pbi_model::CalculationGroup`] with its
//! `calculationItems` and selection expressions), cultures (`linguisticMetadata`
//! content as text), perspectives, query groups and annotations. Text
//! properties may be a string or an array of lines; arrays are joined with
//! `\n`.
//!
//! # Round-trip guarantees
//!
//! For any accepted document, `to_json(read(x))` equals `x` as a JSON value
//! (object key order aside), and with [`LoadedTmsl::write_options`] the
//! output is byte-identical when the source used the TOM serializer order
//! or Power BI Desktop's alphabetical order, a consistent layout, and no
//! redundant escapes. How:
//!
//! - Properties the model does not type are kept in each object's `extra`
//!   bag as [`pbi_model::PropValue::Json`], in source order, and written
//!   back. Unknown keys of a partition `source` live in the partition's
//!   `extra["source"]` (a JSON object).
//! - Text is written as an array of lines when it contains `\n` (the TOM
//!   `SplitMultilineStrings` convention), otherwise as a string. When the
//!   source deviates (multi-line string, one-line array), writes a default
//!   explicitly (`"isHidden": false`), or spells an enum differently, the
//!   original JSON is kept in `extra` under the same key as a *shadow*. The
//!   writer re-emits a shadow only while it still decodes to the current
//!   typed value, so edits always win. An empty
//!   [`pbi_model::PropValue::List`] marks a key that was absent although
//!   its typed default would be written.
//! - Annotations or `linguisticMetadata` the model cannot represent, and
//!   unknown column types, are kept verbatim (with a diagnostic).
//!
//! Limitations: unknown keys are re-emitted right after the nearest
//! preceding known key, not necessarily at their exact original position;
//! duplicate JSON keys keep the last value (reported as `TMSL001`); JSON
//! escapes are normalized (`\u00e9` is written as `é`) and so is the
//! exponent marker of numbers (`1E5` is written as `1e5`; digits are kept).
//!
//! # Diagnostics
//!
//! `TMSL001` duplicate key, `TMSL002` annotations kept verbatim, `TMSL003`
//! unknown column type, `TMSL004` object without a name, `TMSL005`
//! linguistic metadata kept verbatim, `TMSL006` a calculation item's
//! `formatStringDefinition` kept verbatim, `TMSL007` a calculation group
//! that could not be typed, kept verbatim.

mod codec;
mod dupcheck;
mod encoding;
mod error;
mod format;
mod obj;
mod order;
mod read;
mod write;

use pbi_model::{Diagnostic, Model, Severity};
use serde_json::Value;

pub use encoding::{DecodedText, TextEncoding, decode_text, detect_encoding, encode_text};
pub use error::TmslError;
pub use format::{KeyOrder, LineEnding, TmslFormat};

/// A TMSL document read into the model.
#[derive(Debug, Clone, PartialEq)]
pub struct LoadedTmsl {
    /// Root `name` (the database name; a GUID in PBIT files, absent in
    /// Power BI Desktop PBIP files). Also in `model.database`.
    pub database_name: Option<String>,
    /// Root `compatibilityLevel`. Also in `model.database`.
    pub compatibility_level: Option<i64>,
    /// The model; `model.database` holds the root properties. Object IDs are
    /// unassigned.
    pub model: Model,
    pub diagnostics: Vec<Diagnostic>,
    /// Layout of the source, for byte-faithful writing.
    pub format: TmslFormat,
}

impl LoadedTmsl {
    /// Options that write the model back in the source layout.
    pub fn write_options(&self) -> WriteOptions {
        WriteOptions {
            format: self.format.clone(),
            ..WriteOptions::default()
        }
    }
}

/// Options for [`read_text`] and [`read_bytes`].
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ReadOptions {
    /// Path of the file inside the opened container or folder. When set,
    /// objects get an [`pbi_model::Origin`] with this path and their JSON
    /// pointer.
    pub source_path: Option<String>,
}

/// Options for [`write_text`], [`write_bytes`] and [`to_json`].
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct WriteOptions {
    /// Root `name`; `None` uses `model.database.name`.
    pub database_name: Option<String>,
    /// Root `compatibilityLevel`; `None` uses
    /// `model.database.compatibility_level`.
    pub compatibility_level: Option<i64>,
    pub format: TmslFormat,
}

/// Reads `model.bim` text (a leading BOM is tolerated).
pub fn read_bim(json_text: &str) -> Result<LoadedTmsl, TmslError> {
    read_text(json_text, &ReadOptions::default())
}

/// Reads a PBIT `DataModelSchema` entry, or any TMSL bytes: UTF-16LE/BE
/// with or without BOM, or UTF-8 (see [`detect_encoding`]).
pub fn read_data_model_schema(bytes: &[u8]) -> Result<LoadedTmsl, TmslError> {
    read_bytes(bytes, &ReadOptions::default())
}

/// Reads TMSL text.
pub fn read_text(json_text: &str, options: &ReadOptions) -> Result<LoadedTmsl, TmslError> {
    let (text, bom) = match json_text.strip_prefix('\u{FEFF}') {
        Some(rest) => (rest, true),
        None => (json_text, false),
    };
    parse(text, TextEncoding::Utf8, bom, options)
}

/// Reads TMSL bytes in any supported encoding.
pub fn read_bytes(bytes: &[u8], options: &ReadOptions) -> Result<LoadedTmsl, TmslError> {
    let decoded = decode_text(bytes)?;
    parse(&decoded.text, decoded.encoding, decoded.bom, options)
}

fn parse(
    text: &str,
    encoding: TextEncoding,
    bom: bool,
    options: &ReadOptions,
) -> Result<LoadedTmsl, TmslError> {
    let duplicates = dupcheck::find_duplicate_keys(text)?;
    let root: Value = serde_json::from_str(text)?;
    let mut cx = obj::ReadCx::new(options.source_path.clone());
    for pointer in duplicates {
        cx.report(
            Severity::Warning,
            "TMSL001",
            "duplicate JSON key; the last value is used".to_owned(),
            &pointer,
        );
    }
    let model = read::read_root(&root, &mut cx)?;
    let (indent, line_ending, trailing_newline) = format::detect_layout(text);
    let format = TmslFormat {
        key_order: if cx.alphabetical {
            KeyOrder::Alphabetical
        } else {
            KeyOrder::Tom
        },
        indent,
        line_ending,
        trailing_newline,
        encoding,
        bom,
    };
    let database = model.database.as_ref();
    Ok(LoadedTmsl {
        database_name: database.and_then(|d| d.name.clone()),
        compatibility_level: database.and_then(|d| d.compatibility_level),
        diagnostics: cx.diagnostics,
        model,
        format,
    })
}

/// Writes `model` as `model.bim` text: TOM key order, two-space indent, LF.
/// `database_name`/`compatibility_level` override `model.database`.
pub fn write_bim(
    model: &Model,
    database_name: Option<&str>,
    compatibility_level: Option<i64>,
) -> String {
    write_text(
        model,
        &WriteOptions {
            database_name: database_name.map(str::to_owned),
            compatibility_level,
            format: TmslFormat::default(),
        },
    )
}

/// Writes `model` as a PBIT `DataModelSchema` entry: UTF-16LE without BOM,
/// TOM key order, two-space indent, LF.
pub fn write_data_model_schema(
    model: &Model,
    database_name: Option<&str>,
    compatibility_level: Option<i64>,
) -> Vec<u8> {
    write_bytes(
        model,
        &WriteOptions {
            database_name: database_name.map(str::to_owned),
            compatibility_level,
            format: TmslFormat::data_model_schema(),
        },
    )
}

/// The TMSL document as a JSON value.
pub fn to_json(model: &Model, options: &WriteOptions) -> Value {
    write::write_root(model, options)
}

/// Writes TMSL text (layout from `options.format`; encoding and BOM are
/// ignored).
pub fn write_text(model: &Model, options: &WriteOptions) -> String {
    format::render(&to_json(model, options), &options.format)
}

/// Writes TMSL bytes in `options.format.encoding`, with a BOM if requested.
pub fn write_bytes(model: &Model, options: &WriteOptions) -> Vec<u8> {
    encode_text(
        &write_text(model, options),
        options.format.encoding,
        options.format.bom,
    )
}
