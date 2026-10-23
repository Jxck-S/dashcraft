//! VertiPaq column decoding: `.idfmeta` + `.idf` + `.dictionary` → Arrow arrays.
//!
//! For every column the [`StorageMap`](crate::StorageMap) names, per partition, a
//! `.idfmeta` (segment metadata, [`idfmeta`]) and a `.idf` (data ids: hybrid RLE plus
//! bit packing, [`idf`]), and per column an optional `.dictionary` ([`dictionary`]).
//! Data ids become values through the dictionary (hash encoding), arithmetic (value
//! encoding: `(id + BaseId) / Magnitude`) or not at all (hash encoding without a file:
//! every row is BLANK); data id 2 is BLANK, dictionary entry `i` is data id `3 + i`.
//! Values get the Arrow types `pbi-engine` uses:
//!
//! | Logical type | Arrow | Stored as |
//! |---|---|---|
//! | String | `Utf8` | string dictionary (UTF-16 pages, optionally Huffman-compressed) |
//! | Int64 | `Int64` | long dictionary or value encoding |
//! | Double | `Float64` | real dictionary or value encoding |
//! | Decimal | `Decimal128(19, 4)` | value × 10⁴ in a long dictionary / value encoding |
//! | DateTime | `Timestamp(Microsecond, None)` | days since 1899-12-30 (real or integer) |
//! | Boolean | `Boolean` | integer, non-zero = true |
//! | Binary | `Binary` | base64 text in a string dictionary |
//! | Variant / Automatic / unknown | `Int64`, `Float64` or `Utf8` by storage | |
//!
//! A string dictionary behind a non-text logical type stays `Utf8`. Details:
//! `FORMAT-NOTES.md` §7.
//!
//! Entry points: [`decode_column`], [`decode_table`], [`decode_all`],
//! [`decode_all_with_diagnostics`] and [`open_pbix_data`]. All input is untrusted:
//! sizes and counts are bounds-checked, bit widths limited to 1..=32, allocations
//! capped by [`DecodeLimits`], and every malformed input yields a [`VertipaqError`].
//!
//! References (see `REFERENCES.md`): [MS-XLDM] §2.3 and §2.7 (#4), pbixray's
//! `vertipaq_decoder.py` and `.ksy` specs (#3, #7, MIT/CC0; structural port), and
//! xmhuffman (#11, MIT; Huffman bit order and table construction).

mod convert;
mod decode;
pub mod dictionary;
mod error;
mod huffman;
pub mod idf;
pub mod idfmeta;
mod reader;
#[cfg(any(test, feature = "test-support"))]
pub(crate) mod test_support;
#[cfg(test)]
mod tests;

pub use convert::{DECIMAL_PRECISION, DECIMAL_SCALE};
pub use decode::{
    DecodedColumn, DecodedTables, PbixData, decode_all, decode_all_with_diagnostics, decode_column,
    decode_column_storage, decode_column_with_diagnostics, decode_table,
    decode_table_with_diagnostics, open_pbix_data,
};
pub use dictionary::{Dictionary, parse_dictionary};
pub use error::VertipaqError;
pub use idf::decode_idf;
pub use idfmeta::{ColumnPartitionMeta, SegmentEncoding, SegmentMeta, parse_idfmeta};

use crate::xpress9::Limits;

/// Stable diagnostic codes emitted by the decoder.
pub mod codes {
    /// A column could not be decoded and was filled with BLANK (error).
    pub const COLUMN_FAILED: &str = "VPQ001";
    /// Rows with invalid data ids or unconvertible values were set to BLANK (warning).
    pub const INVALID_IDS: &str = "VPQ002";
    /// A table could not be decoded and was skipped (error).
    pub const TABLE_FAILED: &str = "VPQ003";
    /// Decoding stopped because [`super::DecodeLimits::max_total_bytes`] was exceeded.
    pub const BUDGET_EXHAUSTED: &str = "VPQ004";
    /// The model was read from legacy XML metadata (Power Pivot, compatibility level
    /// 1100/1103): info when schema and storage were both read, warning listing the
    /// tables whose storage could not be mapped (their data cannot be decoded).
    pub const XML_METADATA: &str = "VPQ005";
    /// A legacy XML metadata document could not be read and was skipped (warning).
    pub const XML_METADATA_PART: &str = "VPQ006";
}

/// What to do with data ids that map to no value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum InvalidIdPolicy {
    /// Fail the column with [`VertipaqError::InvalidDataIds`] (default).
    #[default]
    Error,
    /// Set the rows to BLANK and report a [`codes::INVALID_IDS`] warning.
    Blank,
}

/// Resource limits and policies for decoding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct DecodeLimits {
    /// XPress9 container limits (used by [`open_pbix_data`]).
    pub container: Limits,
    /// Rows per column / table (all partitions).
    pub max_rows: u64,
    /// Entries per dictionary.
    pub max_dictionary_entries: u64,
    /// Decoded UTF-8 bytes per string dictionary.
    pub max_dictionary_bytes: u64,
    /// UTF-8 / binary bytes of one output column (at most `i32::MAX` for `Utf8`).
    pub max_column_bytes: u64,
    /// Arrow memory of all tables in [`decode_all`] / [`decode_all_with_diagnostics`].
    pub max_total_bytes: u64,
    /// Handling of invalid data ids.
    pub invalid_ids: InvalidIdPolicy,
}

impl DecodeLimits {
    /// Defaults for desktop use: 2²⁸ rows per table, 2³¹ dictionary entries, 4 GiB of
    /// dictionary text, 2 GiB per string column, 16 GiB in total. Raise `max_rows`
    /// for very large models (a single RLE run can declare any row count, so this is
    /// the guard against decompression bombs).
    pub const DEFAULT: DecodeLimits = DecodeLimits {
        container: Limits::DEFAULT,
        max_rows: 1 << 28,
        max_dictionary_entries: 1 << 31,
        max_dictionary_bytes: 4 << 30,
        max_column_bytes: i32::MAX as u64,
        max_total_bytes: 16 << 30,
        invalid_ids: InvalidIdPolicy::Error,
    };

    /// Small limits for fuzzing and untrusted previews: 2²⁰ rows and entries,
    /// 64 MiB per dictionary and column, 256 MiB in total.
    pub const SMALL: DecodeLimits = DecodeLimits {
        container: Limits::SMALL,
        max_rows: 1 << 20,
        max_dictionary_entries: 1 << 20,
        max_dictionary_bytes: 64 << 20,
        max_column_bytes: 64 << 20,
        max_total_bytes: 256 << 20,
        invalid_ids: InvalidIdPolicy::Error,
    };

    /// These limits with a different [`InvalidIdPolicy`].
    #[must_use]
    pub const fn with_invalid_ids(mut self, policy: InvalidIdPolicy) -> Self {
        self.invalid_ids = policy;
        self
    }

    /// These limits with a different row cap.
    #[must_use]
    pub const fn with_max_rows(mut self, rows: u64) -> Self {
        self.max_rows = rows;
        self
    }
}

impl Default for DecodeLimits {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// Fuzzing entry point: runs the `.idfmeta`, `.idf`, dictionary, Huffman and value
/// conversion decoders, and the whole PBIX pipeline, over `data` with
/// [`DecodeLimits::SMALL`]. Never panics.
pub fn fuzz_vertipaq(data: &[u8]) {
    use pbi_model::DataType;

    let limits = DecodeLimits::SMALL.with_invalid_ids(InvalidIdPolicy::Blank);
    let mut ids = Vec::new();
    if let Ok(meta) = parse_idfmeta(data) {
        let _ = decode_idf(data, &meta, limits.max_rows, &mut ids);
    }
    let dictionary = parse_dictionary(data, &limits).ok();

    if let Some((&selector, rest)) = data.split_first() {
        let bit_width = selector % 34;
        let records = rest
            .first_chunk::<2>()
            .map_or(0, |b| u64::from(u16::from_le_bytes(*b)));
        for class in [idfmeta::HYBRID_RLE_CLASS, idfmeta::NO_SPLIT_CLASS_BASE] {
            let meta = synthetic_meta(class, bit_width, records, selector & 1 == 1);
            let mut out = Vec::new();
            let _ = decode_idf(rest, &meta, limits.max_rows, &mut out);
            if ids.is_empty() {
                ids = out;
            }
        }
        if let Some(encode) = rest.get(..128)
            && let Ok(table) = huffman::HuffmanTable::new(encode)
        {
            let stream = huffman::BitStream::new(rest.get(128..).unwrap_or_default());
            let mut out = Vec::new();
            let _ = table.decode(&stream, 0, stream.bit_len().min(1 << 16), &mut out, 1 << 16);
        }
    }

    let types = [
        DataType::String,
        DataType::Int64,
        DataType::Double,
        DataType::Decimal,
        DataType::DateTime,
        DataType::Boolean,
        DataType::Binary,
        DataType::Variant,
    ];
    let mut sources = vec![
        convert::Source::Blank,
        convert::Source::Value {
            base_id: -3,
            magnitude: 1.0,
        },
        convert::Source::Value {
            base_id: i64::MAX,
            magnitude: 0.01,
        },
    ];
    if let Some(dictionary) = &dictionary {
        sources.push(convert::Source::Dictionary(dictionary));
    }
    ids.truncate(4096);
    for source in &sources {
        for data_type in &types {
            let _ = convert::convert(&ids, source, data_type, limits.max_column_bytes);
        }
    }
    let _ = open_pbix_data(data, &limits);
}

/// A one-segment partition description for synthetic decoding.
pub(crate) fn synthetic_meta(
    class: u32,
    bit_width: u8,
    records: u64,
    has_nulls: bool,
) -> ColumnPartitionMeta {
    let hybrid = class == idfmeta::HYBRID_RLE_CLASS;
    let width_class = idfmeta::NO_SPLIT_CLASS_BASE + u32::from(bit_width);
    ColumnPartitionMeta {
        segments: vec![SegmentMeta {
            records,
            base_id: 0,
            compression_class: if hybrid { class } else { width_class },
            sub_compression_class: if hybrid { width_class } else { 0 },
            rle: hybrid.then(idfmeta::RleFacet::default),
            first_run_value: 0,
            stats: idfmeta::SegmentStats {
                min_data_id: idfmeta::FIRST_DATA_ID,
                has_nulls,
                ..idfmeta::SegmentStats::default()
            },
            subsegment: hybrid.then_some(idfmeta::SubSegmentMeta {
                records,
                base_id: 0,
            }),
        }],
        data_objects: None,
    }
}
