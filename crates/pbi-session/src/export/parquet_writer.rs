//! Parquet export ([`write_parquet`]).

use std::io::Write;

use arrow::record_batch::RecordBatch;
use parquet::arrow::ArrowWriter;
use parquet::basic::{Compression, ZstdLevel};
use parquet::file::metadata::KeyValue;
use parquet::file::properties::WriterProperties;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value as Json};

use super::{EXPORT_METADATA_VERSION, ExportError, ExportSummary, meta};

/// Field metadata keys copied into `dashcraft.columns`, with their JSON names.
const COLUMN_KEYS: [(&str, &str); 6] = [
    (meta::DATA_TYPE, "dataType"),
    (meta::KIND, "kind"),
    (meta::DESCRIPTION, "description"),
    (meta::FORMAT_STRING, "formatString"),
    (meta::SOURCE_COLUMN, "sourceColumn"),
    (meta::DERIVED, "derived"),
];

/// Compression codec of the Parquet column chunks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ParquetCompression {
    /// Zstandard (default level): small files, fast to read.
    #[default]
    Zstd,
    /// Snappy: the most widely supported codec.
    Snappy,
    /// No compression.
    None,
}

/// Options of [`write_parquet`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ParquetExportOptions {
    /// Compression codec (default Zstd).
    pub compression: ParquetCompression,
}

/// Writes `batch` as a Parquet file. Types map one to one (fixed decimals
/// stay `Decimal128(19, 4)`, date/times are `TIMESTAMP(MICROS)` without a
/// time zone). The Arrow schema, with the field metadata of
/// [`annotate_batch`](super::annotate_batch), is embedded as usual; the
/// file's key-value metadata adds `dashcraft.schemaVersion`, the schema's
/// `dashcraft.table`, `dashcraft.tableDescription` and `dashcraft.culture`, and
/// `dashcraft.columns`: a JSON array of `{ name, dataType?, kind?,
/// description?, formatString?, sourceColumn? }` in column order.
///
/// # Errors
/// [`ExportError::Parquet`] (including write failures).
pub fn write_parquet<W: Write + Send>(
    batch: &RecordBatch,
    writer: W,
    options: &ParquetExportOptions,
) -> Result<ExportSummary, ExportError> {
    let compression = match options.compression {
        ParquetCompression::Zstd => Compression::ZSTD(ZstdLevel::default()),
        ParquetCompression::Snappy => Compression::SNAPPY,
        ParquetCompression::None => Compression::UNCOMPRESSED,
    };
    let props = WriterProperties::builder()
        .set_compression(compression)
        .set_key_value_metadata(Some(key_value_metadata(batch)))
        .build();
    let mut out = ArrowWriter::try_new(writer, batch.schema(), Some(props))?;
    out.write(batch)?;
    out.close()?;
    Ok(ExportSummary {
        rows: batch.num_rows(),
        columns: batch.num_columns(),
    })
}

fn key_value_metadata(batch: &RecordBatch) -> Vec<KeyValue> {
    let schema = batch.schema();
    let mut kv = vec![KeyValue::new(
        meta::SCHEMA_VERSION.to_owned(),
        EXPORT_METADATA_VERSION.to_string(),
    )];
    for key in [meta::TABLE, meta::TABLE_DESCRIPTION, meta::CULTURE] {
        if let Some(value) = schema.metadata().get(key) {
            kv.push(KeyValue::new(key.to_owned(), value.clone()));
        }
    }
    let columns: Vec<Json> = schema
        .fields()
        .iter()
        .map(|f| {
            let mut obj = Map::new();
            obj.insert("name".into(), Json::String(f.name().clone()));
            for (key, name) in COLUMN_KEYS {
                if let Some(value) = f.metadata().get(key) {
                    obj.insert(name.into(), Json::String(value.clone()));
                }
            }
            Json::Object(obj)
        })
        .collect();
    kv.push(KeyValue::new(
        meta::COLUMNS.to_owned(),
        Json::Array(columns).to_string(),
    ));
    kv
}
