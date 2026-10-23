//! `.idf`: the data ids of one column partition.
//!
//! Each segment (in `.idfmeta` order) is stored as
//!
//! ```text
//! hybrid RLE:  u64 n  (u32 value, u32 repeat) × n        primary (RLE) segment
//!              u64 m  u64 word × m                       bit-packed subsegment
//! bit-packed:  u64 m  u64 word × m
//! ```
//!
//! A primary entry whose value equals `0xFFFF_FFFF − k`, where `k` is the number of
//! bit-packed values consumed so far, is a *bit-packing entry*: its `repeat` rows come
//! from the subsegment. Any other entry is an RLE run. Trailing primary slots are
//! allocation padding, so runs are capped at the segment's `records` ([MS-XLDM]
//! §2.7.3; pbixray caps the same way because some writers leave stale padding).
//! Subsegment words hold `64 / bit_width` values each, low bits first, and every value
//! is offset by [`SegmentMeta::bit_pack_base`] ([MS-XLDM] §2.7.1).

use super::error::VertipaqError;
use super::idfmeta::{ColumnPartitionMeta, SegmentEncoding, SegmentMeta};
use super::reader::{Reader, le_u32_at, le_u64_at};

const WHAT: &str = "idf";

/// Decodes the data ids of every segment of `meta` from `idf`, appending them to
/// `out`. At most `max_rows` ids may be appended in total (`out.len()` included).
pub fn decode_idf(
    idf: &[u8],
    meta: &ColumnPartitionMeta,
    max_rows: u64,
    out: &mut Vec<u32>,
) -> Result<(), VertipaqError> {
    let records = meta
        .records()
        .ok_or_else(|| VertipaqError::corrupt(WHAT, "segment record counts overflow"))?;
    let total = (out.len() as u64).saturating_add(records);
    if total > max_rows {
        return Err(VertipaqError::LimitExceeded {
            what: "rows in a column",
            value: total,
            limit: max_rows,
        });
    }
    let additional = usize::try_from(records).map_err(|_| VertipaqError::LimitExceeded {
        what: "rows in a column",
        value: records,
        limit: usize::MAX as u64,
    })?;
    out.try_reserve(additional)
        .map_err(|_| VertipaqError::LimitExceeded {
            what: "memory for data ids (bytes)",
            value: records.saturating_mul(4),
            limit: 0,
        })?;

    let mut r = Reader::new(idf, WHAT);
    for (index, segment) in meta.segments.iter().enumerate() {
        decode_segment(&mut r, segment, out).map_err(|err| match err {
            VertipaqError::Corrupt { what, message } => VertipaqError::Corrupt {
                what,
                message: format!("segment {index}: {message}"),
            },
            other => other,
        })?;
    }
    Ok(())
}

fn decode_segment(
    r: &mut Reader<'_>,
    segment: &SegmentMeta,
    out: &mut Vec<u32>,
) -> Result<(), VertipaqError> {
    let base = segment.bit_pack_base();
    match segment.encoding() {
        SegmentEncoding::HybridRle { bit_width } => {
            let entries = r.u64()?;
            let primary = r.array(entries, 8)?;
            // Hybrid segments always carry a subsegment; tolerate its absence only at
            // the very end of the file when the metadata says there is none.
            let words = if segment.subsegment.is_none() && r.is_empty() {
                &[][..]
            } else {
                let count = r.u64()?;
                r.array(count, 8)?
            };
            let unpacker = Unpacker::new(words, bit_width, base)?;
            decode_hybrid(primary, &unpacker, segment.records, out)
        }
        SegmentEncoding::BitPacked { bit_width } => {
            let count = r.u64()?;
            let words = r.array(count, 8)?;
            let unpacker = Unpacker::new(words, bit_width, base)?;
            unpacker.extend(0, segment.records, out)
        }
        SegmentEncoding::Unsupported { class, sub_class } => Err(VertipaqError::unsupported(
            "segment compression",
            format!("class {class:#x} / sub-class {sub_class:#x}"),
        )),
    }
}

fn decode_hybrid(
    primary: &[u8],
    unpacker: &Unpacker<'_>,
    records: u64,
    out: &mut Vec<u32>,
) -> Result<(), VertipaqError> {
    let mut produced = 0u64;
    let mut packed = 0u64;
    for entry in 0..primary.len() / 8 {
        if produced >= records {
            break;
        }
        let (Some(value), Some(repeat)) = (
            le_u32_at(primary, entry * 2),
            le_u32_at(primary, entry * 2 + 1),
        ) else {
            break;
        };
        let count = u64::from(repeat).min(records - produced);
        if count == 0 {
            continue;
        }
        if u64::from(value) + packed == u64::from(u32::MAX) {
            unpacker.extend(packed, count, out)?;
            packed += count;
        } else {
            // `count <= records`, which was reserved up front.
            out.extend(std::iter::repeat_n(value, count as usize));
        }
        produced += count;
    }
    if produced != records {
        return Err(VertipaqError::corrupt(
            WHAT,
            format!("{records} rows declared but the RLE entries cover {produced}"),
        ));
    }
    Ok(())
}

/// Reader of `XMRENoSplit<bit_width>` packed words.
#[derive(Debug)]
pub(crate) struct Unpacker<'a> {
    words: &'a [u8],
    bit_width: u32,
    per_word: u64,
    mask: u64,
    base: u32,
    /// The writer stores a single zero word when every value equals the base.
    single_zero_word: bool,
    needs_check: bool,
}

impl<'a> Unpacker<'a> {
    pub(crate) fn new(words: &'a [u8], bit_width: u8, base: u32) -> Result<Self, VertipaqError> {
        if !(1..=32).contains(&bit_width) {
            return Err(VertipaqError::corrupt(
                WHAT,
                format!("bit width {bit_width} outside 1..=32"),
            ));
        }
        let bit_width = u32::from(bit_width);
        let mask = (1u64 << bit_width) - 1;
        Ok(Self {
            words,
            bit_width,
            per_word: u64::from(64 / bit_width),
            mask,
            base,
            single_zero_word: words.len() == 8 && words.iter().all(|&b| b == 0),
            needs_check: u64::from(base) + mask > u64::from(u32::MAX),
        })
    }

    /// Appends values `start .. start + count` to `out`.
    pub(crate) fn extend(
        &self,
        start: u64,
        count: u64,
        out: &mut Vec<u32>,
    ) -> Result<(), VertipaqError> {
        if count == 0 {
            return Ok(());
        }
        let mut word_index = start / self.per_word;
        let mut slot = start % self.per_word;
        let mut remaining = count;
        while remaining > 0 {
            let word = match usize::try_from(word_index)
                .ok()
                .and_then(|i| le_u64_at(self.words, i))
            {
                Some(word) => word,
                None if self.single_zero_word => 0,
                None => {
                    return Err(VertipaqError::corrupt(
                        WHAT,
                        format!(
                            "bit-packed value {} lies beyond the {} subsegment words",
                            word_index.saturating_mul(self.per_word) + slot,
                            self.words.len() / 8
                        ),
                    ));
                }
            };
            let take = (self.per_word - slot).min(remaining);
            // `slot * bit_width < 64` because `slot < 64 / bit_width`.
            let mut bits = word >> (slot as u32 * self.bit_width);
            for _ in 0..take {
                let value = bits & self.mask;
                bits >>= self.bit_width;
                let id = if self.needs_check {
                    u32::try_from(u64::from(self.base) + value).map_err(|_| {
                        VertipaqError::corrupt(WHAT, "bit-packed data id exceeds 32 bits")
                    })?
                } else {
                    // Cannot overflow: base + mask <= u32::MAX.
                    self.base + value as u32
                };
                out.push(id);
            }
            remaining -= take;
            word_index += 1;
            slot = 0;
        }
        Ok(())
    }
}
