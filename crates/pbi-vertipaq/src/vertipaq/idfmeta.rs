//! `.idfmeta`: per-segment metadata of one column partition.
//!
//! Tagged binary layout (all integers little-endian; tags are ASCII + NUL):
//!
//! ```text
//! "<1:CP\0"  u64 segment_count  CS × segment_count  "CP:1>\0"  [SDOs]
//! CS  := "<1:CS\0" u64 records u64 base_id
//!        (primary only:) u32 compression_class u32 sub_compression_class
//!                        [RLE facet, if class == 0xABA5A: u64 bookmark_bits,
//!                         u64 alloc_size, u64 used_size, u8 needs_resizing]
//!                        u32 first_run_value  SS
//!        u8 has_subsegment  [CS (subsegment, no nesting)]  "CS:1>\0"
//! SS  := "<1:SS\0" u64 distinct_states u32 min_data_id u32 max_data_id
//!        u32 original_min_segment_data_id i64 rle_sort_order u64 row_count
//!        u8 has_nulls u64 rle_runs u64 others_rle_runs "SS:1>\0"
//! SDOs:= "<1:SDOs\0" CSDOs × segment_count "SDOs:1>\0"
//! CSDOs:= "<1:CSDOs\0" i64 size i64 offset [CSDOs, if the CS has a subsegment] "CSDOs:1>\0"
//! ```
//!
//! Layout and class ids from pbixray's `idfmeta.ksy` (CC0/MIT, reference #7 in
//! `REFERENCES.md`); compression semantics from [MS-XLDM] §2.7.

use super::error::VertipaqError;
use super::reader::Reader;

const WHAT: &str = "idfmeta";

/// `XMHybridRLECompressionInfo`: RLE primary segment + bit-packed subsegment.
pub const HYBRID_RLE_CLASS: u32 = 0x000A_BA5A;
/// `XMREGeneralCompressionInfo` (not used for persisted column data).
pub const GENERAL_CLASS: u32 = 0x000A_BA57;
/// `XM123CompressionInfo` (only the internal row-number column).
pub const XM123_CLASS: u32 = 0x000A_BA5B;
/// `XMRENoSplitCompressionInfo<n>` has class `NO_SPLIT_CLASS_BASE + n`
/// (observed/defined for n = 1..=10, 12, 16, 21, 32).
pub const NO_SPLIT_CLASS_BASE: u32 = 0x000A_BA36;

/// Data id reserved for NULL/BLANK (`XM_DATA_ID_NULL`).
pub const NULL_DATA_ID: u32 = 2;
/// First data id of a hash dictionary (`XM_DATA_ID_NULL + 1`).
pub const FIRST_DATA_ID: u32 = 3;

/// Bit width of a `XMRENoSplitCompressionInfo<n>` class, if `class` is one.
pub fn no_split_bit_width(class: u32) -> Option<u8> {
    let n = class.checked_sub(NO_SPLIT_CLASS_BASE)?;
    u8::try_from(n).ok().filter(|n| (1..=32).contains(n))
}

/// How the segment's data ids are compressed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum SegmentEncoding {
    /// `XMHybridRLE` with `XMRENoSplit<bit_width>` sub-compression: RLE runs plus a
    /// bit-packed subsegment (the normal case for column data).
    HybridRle {
        /// Bits per bit-packed value (1..=32).
        bit_width: u8,
    },
    /// Plain `XMRENoSplit<bit_width>`: every row is bit-packed, no RLE runs.
    BitPacked {
        /// Bits per value (1..=32).
        bit_width: u8,
    },
    /// Anything else (`XM123`, `General`, unknown classes).
    Unsupported {
        /// `compression_class`.
        class: u32,
        /// `sub_compression_class`.
        sub_class: u32,
    },
}

/// RLE facet of a hybrid segment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub struct RleFacet {
    /// `BookmarkBits`.
    pub bookmark_bits: u64,
    /// Allocated RLE storage (8-byte units).
    pub storage_alloc_size: u64,
    /// Used RLE storage (8-byte units).
    pub storage_used_size: u64,
    /// `m_fSegmentNeedsResizing`.
    pub segment_needs_resizing: bool,
}

/// `SS` statistics of a segment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub struct SegmentStats {
    /// Distinct data ids.
    pub distinct_states: u64,
    /// Smallest non-null data id (NULLs only set [`SegmentStats::has_nulls`]).
    pub min_data_id: u32,
    /// Largest data id.
    pub max_data_id: u32,
    /// Minimum data id before remapping.
    pub original_min_segment_data_id: u32,
    /// RLE sort order indicator.
    pub rle_sort_order: i64,
    /// Rows (statistics).
    pub row_count: u64,
    /// The segment contains NULL ([`NULL_DATA_ID`]).
    pub has_nulls: bool,
    /// RLE runs.
    pub rle_runs: u64,
    /// RLE runs in related segments.
    pub others_rle_runs: u64,
}

/// The bit-packed subsegment of a hybrid segment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub struct SubSegmentMeta {
    /// Bit-packed values.
    pub records: u64,
    /// `base_id` field (observed 0).
    pub base_id: u64,
}

/// Metadata of one column segment.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct SegmentMeta {
    /// Rows in the segment (authoritative row count).
    pub records: u64,
    /// `base_id` field of the CS element (not the bit-packing base).
    pub base_id: u64,
    /// `compression_class` (`PF_OBJECT_CLASS`).
    pub compression_class: u32,
    /// `sub_compression_class` (inner class for hybrid RLE).
    pub sub_compression_class: u32,
    /// RLE facet (hybrid segments only).
    pub rle: Option<RleFacet>,
    /// `first_run_value`.
    pub first_run_value: u32,
    /// Statistics.
    pub stats: SegmentStats,
    /// Bit-packed subsegment, if present.
    pub subsegment: Option<SubSegmentMeta>,
}

impl SegmentMeta {
    /// The segment's compression.
    pub fn encoding(&self) -> SegmentEncoding {
        if self.compression_class == HYBRID_RLE_CLASS {
            if let Some(bit_width) = no_split_bit_width(self.sub_compression_class) {
                return SegmentEncoding::HybridRle { bit_width };
            }
        } else if let Some(bit_width) = no_split_bit_width(self.compression_class) {
            return SegmentEncoding::BitPacked { bit_width };
        }
        SegmentEncoding::Unsupported {
            class: self.compression_class,
            sub_class: self.sub_compression_class,
        }
    }

    /// The `Min` added to every bit-packed value: [`NULL_DATA_ID`] when the segment
    /// holds NULLs (its `min_data_id` excludes them), else `min_data_id`.
    pub fn bit_pack_base(&self) -> u32 {
        if self.stats.has_nulls {
            NULL_DATA_ID.min(self.stats.min_data_id)
        } else {
            self.stats.min_data_id
        }
    }
}

/// `CSDOs`: location of a segment's data in the `.idf` (on-demand loading).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct SegmentDataObject {
    /// Size in bytes of the segment data.
    pub size: i64,
    /// Byte offset into the `.idf`.
    pub offset: i64,
    /// `(size, offset)` of the subsegment, if any.
    pub subsegment: Option<(i64, i64)>,
}

/// Parsed `.idfmeta` of one column partition.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[non_exhaustive]
pub struct ColumnPartitionMeta {
    /// Segments in storage order.
    pub segments: Vec<SegmentMeta>,
    /// Optional `SDOs` block.
    pub data_objects: Option<Vec<SegmentDataObject>>,
}

impl ColumnPartitionMeta {
    /// Total rows (sum of segment records), `None` on overflow.
    pub fn records(&self) -> Option<u64> {
        self.segments
            .iter()
            .try_fold(0u64, |acc, s| acc.checked_add(s.records))
    }
}

const CP_OPEN: &[u8] = b"<1:CP\0";
const CP_CLOSE: &[u8] = b"CP:1>\0";
const CS_OPEN: &[u8] = b"<1:CS\0";
const CS_CLOSE: &[u8] = b"CS:1>\0";
const SS_OPEN: &[u8] = b"<1:SS\0";
const SS_CLOSE: &[u8] = b"SS:1>\0";
const SDOS_OPEN: &[u8] = b"<1:SDOs\0";
const SDOS_CLOSE: &[u8] = b"SDOs:1>\0";
const CSDOS_OPEN: &[u8] = b"<1:CSDOs\0";
const CSDOS_CLOSE: &[u8] = b"CSDOs:1>\0";

/// Smallest possible encoded CS element (a subsegment): tags + 2 × u64 + u8.
const MIN_CS_LEN: usize = 6 + 8 + 8 + 1 + 6;

/// Parses an `.idfmeta` file.
pub fn parse_idfmeta(bytes: &[u8]) -> Result<ColumnPartitionMeta, VertipaqError> {
    let mut r = Reader::new(bytes, WHAT);
    r.expect_tag(CP_OPEN, "<1:CP")?;
    let count = r.u64()?;
    if count > (r.remaining() / MIN_CS_LEN) as u64 {
        return Err(VertipaqError::corrupt(
            WHAT,
            format!("{count} segments declared in {} bytes", r.remaining()),
        ));
    }
    let mut segments = Vec::with_capacity(count as usize);
    for _ in 0..count {
        segments.push(parse_cs(&mut r)?);
    }
    r.expect_tag(CP_CLOSE, "CP:1>")?;

    // The SDOs block is optional; unknown trailing bytes are ignored.
    let data_objects = if r.peek_tag(SDOS_OPEN) {
        r.expect_tag(SDOS_OPEN, "<1:SDOs")?;
        let mut objects = Vec::with_capacity(segments.len());
        for segment in &segments {
            objects.push(parse_csdos(&mut r, segment.subsegment.is_some())?);
        }
        r.expect_tag(SDOS_CLOSE, "SDOs:1>")?;
        Some(objects)
    } else {
        None
    };
    Ok(ColumnPartitionMeta {
        segments,
        data_objects,
    })
}

fn parse_cs(r: &mut Reader<'_>) -> Result<SegmentMeta, VertipaqError> {
    r.expect_tag(CS_OPEN, "<1:CS")?;
    let records = r.u64()?;
    let base_id = r.u64()?;
    let compression_class = r.u32()?;
    let sub_compression_class = r.u32()?;
    let rle = if compression_class == HYBRID_RLE_CLASS {
        Some(RleFacet {
            bookmark_bits: r.u64()?,
            storage_alloc_size: r.u64()?,
            storage_used_size: r.u64()?,
            segment_needs_resizing: r.u8()? != 0,
        })
    } else {
        None
    };
    let first_run_value = r.u32()?;
    let stats = parse_ss(r)?;
    let subsegment = if r.u8()? != 0 {
        r.expect_tag(CS_OPEN, "<1:CS (subsegment)")?;
        let sub = SubSegmentMeta {
            records: r.u64()?,
            base_id: r.u64()?,
        };
        if r.u8()? != 0 {
            return Err(VertipaqError::corrupt(WHAT, "nested subsegment"));
        }
        r.expect_tag(CS_CLOSE, "CS:1> (subsegment)")?;
        Some(sub)
    } else {
        None
    };
    r.expect_tag(CS_CLOSE, "CS:1>")?;
    Ok(SegmentMeta {
        records,
        base_id,
        compression_class,
        sub_compression_class,
        rle,
        first_run_value,
        stats,
        subsegment,
    })
}

fn parse_ss(r: &mut Reader<'_>) -> Result<SegmentStats, VertipaqError> {
    r.expect_tag(SS_OPEN, "<1:SS")?;
    let stats = SegmentStats {
        distinct_states: r.u64()?,
        min_data_id: r.u32()?,
        max_data_id: r.u32()?,
        original_min_segment_data_id: r.u32()?,
        rle_sort_order: r.i64()?,
        row_count: r.u64()?,
        has_nulls: r.u8()? != 0,
        rle_runs: r.u64()?,
        others_rle_runs: r.u64()?,
    };
    r.expect_tag(SS_CLOSE, "SS:1>")?;
    Ok(stats)
}

fn parse_csdos(
    r: &mut Reader<'_>,
    has_subsegment: bool,
) -> Result<SegmentDataObject, VertipaqError> {
    r.expect_tag(CSDOS_OPEN, "<1:CSDOs")?;
    let size = r.i64()?;
    let offset = r.i64()?;
    let subsegment = if has_subsegment {
        r.expect_tag(CSDOS_OPEN, "<1:CSDOs (subsegment)")?;
        let sub = (r.i64()?, r.i64()?);
        r.expect_tag(CSDOS_CLOSE, "CSDOs:1> (subsegment)")?;
        Some(sub)
    } else {
        None
    };
    r.expect_tag(CSDOS_CLOSE, "CSDOs:1>")?;
    Ok(SegmentDataObject {
        size,
        offset,
        subsegment,
    })
}
