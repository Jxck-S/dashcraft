//! `.dictionary`: hash-encoded column dictionaries ([MS-XLDM] §2.3.2).
//!
//! ```text
//! i32 dictionary_type   (-1 invalid, 0 long, 1 real, 2 string)
//! i32 × 6               hash information (unused by readers)
//! long / real:  u64 count  u32 element_size  value × count
//!               (long: 4 or 8 bytes, signed; real: 8-byte IEEE double)
//! string:       i64 store_string_count  i8 store_compressed  i64 longest_string
//!               i64 page_count  page × page_count
//!               u64 handle_count  u32 handle_size (8)  (u32 offset, u32 page) × handle_count
//! page:         u64 page_mask  u8 contains_nulls  u64 start_index  u64 string_count
//!               u8 compressed  DD CC BB AA  store  CD AB CD AB
//! uncompressed store: u64 remaining_chars  u64 used_chars  u64 allocation_bytes
//!                     UTF-16LE NUL-terminated strings (allocation_bytes)
//! compressed store:   u32 total_bits  u32 charset_mode (0xABA91 single, 0xABA92 multi)
//!                     u64 allocation_bytes  [u8 charset_byte, single mode only]
//!                     u32 decode_bits  u8[128] encode_array  u64 buffer_size
//!                     u8[allocation_bytes] Huffman bit stream
//! ```
//!
//! Record handle `k` belongs to the `k`-th dictionary entry, i.e. data id
//! [`FIRST_DATA_ID`](super::idfmeta::FIRST_DATA_ID)` + k`. For compressed pages the
//! handle offset is a bit offset into the page's stream (the string ends at the next
//! handle of the same page, the last at `total_bits`); uncompressed pages are split at
//! their NUL terminators and matched to their handles in order. Layout from [MS-XLDM]
//! §2.3.2.1 and pbixray's `dictionary.ksy` (#7); string assembly follows pbixray's
//! `vertipaq_decoder.py` (#3); Huffman decoding: `huffman.rs` (xmhuffman, #11).

use super::DecodeLimits;
use super::error::VertipaqError;
use super::huffman::{BitStream, HuffmanTable};
use super::reader::{Reader, le_u32_at};

const WHAT: &str = "dictionary";

/// `XM_HUFFMAN_SINGLECHARSET`: one byte per character, the shared high byte stored once.
pub const HUFFMAN_SINGLE_CHARSET: u32 = 703_121;
/// `XM_HUFFMAN_MULTICHARSET`: Huffman over the raw UTF-16LE bytes.
pub const HUFFMAN_MULTI_CHARSET: u32 = 703_122;

/// Largest page count allowed by [MS-XLDM] §2.3.2.1.3.2.
pub const MAX_PAGES: u64 = 524_288;

const PAGE_BEGIN: &[u8] = &[0xDD, 0xCC, 0xBB, 0xAA];
const PAGE_END: &[u8] = &[0xCD, 0xAB, 0xCD, 0xAB];

/// A decoded hash dictionary; entry `i` is the value of data id `FIRST_DATA_ID + i`.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum Dictionary {
    /// `XM_TYPE_LONG` (4- or 8-byte integers, widened).
    Long(Vec<i64>),
    /// `XM_TYPE_REAL`.
    Real(Vec<f64>),
    /// `XM_TYPE_STRING`.
    String(Vec<String>),
}

impl Dictionary {
    /// Number of entries.
    pub fn len(&self) -> usize {
        match self {
            Dictionary::Long(v) => v.len(),
            Dictionary::Real(v) => v.len(),
            Dictionary::String(v) => v.len(),
        }
    }

    /// `true` if the dictionary has no entries.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// Parses a `.dictionary` file.
pub fn parse_dictionary(bytes: &[u8], limits: &DecodeLimits) -> Result<Dictionary, VertipaqError> {
    let mut r = Reader::new(bytes, WHAT);
    let kind = r.i32()?;
    r.bytes(24)?;
    match kind {
        0 | 1 => parse_numbers(&mut r, kind == 1, limits),
        2 => parse_strings(&mut r, limits),
        -1 => Err(VertipaqError::corrupt(
            WHAT,
            "dictionary type is XM_TYPE_INVALID",
        )),
        other => Err(VertipaqError::unsupported(
            "dictionary type",
            other.to_string(),
        )),
    }
}

fn check_entries(count: u64, limits: &DecodeLimits) -> Result<(), VertipaqError> {
    if count > limits.max_dictionary_entries {
        return Err(VertipaqError::LimitExceeded {
            what: "dictionary entries",
            value: count,
            limit: limits.max_dictionary_entries,
        });
    }
    Ok(())
}

fn parse_numbers(
    r: &mut Reader<'_>,
    real: bool,
    limits: &DecodeLimits,
) -> Result<Dictionary, VertipaqError> {
    let count = r.u64()?;
    let size = r.u32()?;
    check_entries(count, limits)?;
    let element = match (real, size) {
        (false, 4 | 8) | (true, 8) => size as usize,
        _ => {
            return Err(VertipaqError::unsupported(
                "dictionary element size",
                format!(
                    "{size} bytes for a {} dictionary",
                    if real { "real" } else { "long" }
                ),
            ));
        }
    };
    let data = r.array(count, element)?;
    let chunks = data.chunks_exact(element);
    Ok(match (real, element) {
        (true, _) => Dictionary::Real(
            chunks
                .map(|c| f64::from_le_bytes(c.try_into().unwrap_or_default()))
                .collect(),
        ),
        (false, 4) => Dictionary::Long(
            chunks
                .map(|c| i64::from(i32::from_le_bytes(c.try_into().unwrap_or_default())))
                .collect(),
        ),
        (false, _) => Dictionary::Long(
            chunks
                .map(|c| i64::from_le_bytes(c.try_into().unwrap_or_default()))
                .collect(),
        ),
    })
}

/// The string store of one page.
enum Store<'a> {
    /// UTF-16LE code units of the used part of the buffer.
    Plain(&'a [u8]),
    Compressed {
        total_bits: u64,
        multi_charset: bool,
        charset_byte: u8,
        encode_array: &'a [u8],
        buffer: &'a [u8],
    },
}

/// Smallest possible page: header fields, both marks and an empty plain store.
const MIN_PAGE_LEN: usize = 8 + 1 + 8 + 8 + 1 + 4 + 24 + 4;

fn parse_strings(r: &mut Reader<'_>, limits: &DecodeLimits) -> Result<Dictionary, VertipaqError> {
    let _store_string_count = r.i64()?;
    let _store_compressed = r.i8()?;
    let _longest = r.i64()?;
    let page_count = r.i64()?;
    let page_count = u64::try_from(page_count)
        .ok()
        .filter(|&n| n <= MAX_PAGES && n <= (r.remaining() / MIN_PAGE_LEN) as u64)
        .ok_or_else(|| VertipaqError::corrupt(WHAT, format!("invalid page count {page_count}")))?;

    let mut stores = Vec::with_capacity(page_count as usize);
    for page in 0..page_count {
        stores.push(parse_page(r).map_err(|err| match err {
            VertipaqError::Corrupt { what, message } => VertipaqError::Corrupt {
                what,
                message: format!("page {page}: {message}"),
            },
            other => other,
        })?);
    }

    let handle_count = r.u64()?;
    let handle_size = r.u32()?;
    if handle_size != 8 {
        return Err(VertipaqError::corrupt(
            WHAT,
            format!("record handle size {handle_size}, expected 8"),
        ));
    }
    check_entries(handle_count, limits)?;
    let handles = r.array(handle_count, 8)?;
    let handle = |k: usize| -> (u32, u32) {
        (
            le_u32_at(handles, 2 * k).unwrap_or(0),
            le_u32_at(handles, 2 * k + 1).unwrap_or(u32::MAX),
        )
    };
    let handle_count = handles.len() / 8;

    // Offsets of each page's strings, in handle order.
    let mut offsets: Vec<Vec<u32>> = vec![Vec::new(); stores.len()];
    for k in 0..handle_count {
        let (offset, page) = handle(k);
        let list = offsets.get_mut(page as usize).ok_or_else(|| {
            VertipaqError::corrupt(
                WHAT,
                format!(
                    "record handle {k} refers to page {page} of {}",
                    stores.len()
                ),
            )
        })?;
        list.push(offset);
    }

    let mut budget = limits.max_dictionary_bytes;
    let mut page_strings: Vec<std::vec::IntoIter<String>> = Vec::with_capacity(stores.len());
    for (page, (store, offsets)) in stores.iter().zip(&offsets).enumerate() {
        let strings = match store {
            Store::Plain(units) => split_plain(units, &mut budget)?,
            Store::Compressed { .. } => decode_compressed(store, offsets, &mut budget)?,
        };
        if strings.len() < offsets.len() {
            return Err(VertipaqError::corrupt(
                WHAT,
                format!(
                    "page {page} holds {} strings but {} record handles refer to it",
                    strings.len(),
                    offsets.len()
                ),
            ));
        }
        page_strings.push(strings.into_iter());
    }

    let mut values = Vec::with_capacity(handle_count);
    for k in 0..handle_count {
        let (_, page) = handle(k);
        let next = page_strings
            .get_mut(page as usize)
            .and_then(Iterator::next)
            .ok_or_else(|| VertipaqError::corrupt(WHAT, "record handles exceed page strings"))?;
        values.push(next);
    }
    Ok(Dictionary::String(values))
}

fn parse_page<'a>(r: &mut Reader<'a>) -> Result<Store<'a>, VertipaqError> {
    let _mask = r.u64()?;
    let _contains_nulls = r.u8()?;
    let _start_index = r.u64()?;
    let _string_count = r.u64()?;
    let compressed = r.u8()?;
    r.expect_tag(PAGE_BEGIN, "string store begin")?;
    let store = match compressed {
        0 => {
            let _remaining = r.u64()?;
            let used = r.u64()?;
            let allocation = r.u64()?;
            let buffer = r.array(allocation, 1)?;
            let used_bytes = usize::try_from(used)
                .ok()
                .and_then(|u| u.checked_mul(2))
                .filter(|&u| u <= buffer.len())
                .ok_or_else(|| {
                    VertipaqError::corrupt(
                        WHAT,
                        format!("{used} used characters exceed the {allocation}-byte store"),
                    )
                })?;
            Store::Plain(buffer.get(..used_bytes).unwrap_or_default())
        }
        1 => {
            let total_bits = u64::from(r.u32()?);
            let mode = r.u32()?;
            let allocation = r.u64()?;
            let multi_charset = match mode {
                HUFFMAN_SINGLE_CHARSET => false,
                HUFFMAN_MULTI_CHARSET => true,
                other => {
                    return Err(VertipaqError::unsupported(
                        "Huffman character set mode",
                        format!("{other:#x}"),
                    ));
                }
            };
            let charset_byte = if multi_charset { 0 } else { r.u8()? };
            let _decode_bits = r.u32()?;
            let encode_array = r.bytes(128)?;
            let _buffer_size = r.u64()?;
            let buffer = r.array(allocation, 1)?;
            Store::Compressed {
                total_bits,
                multi_charset,
                charset_byte,
                encode_array,
                buffer,
            }
        }
        other => {
            return Err(VertipaqError::corrupt(
                WHAT,
                format!("page compression flag {other}"),
            ));
        }
    };
    r.expect_tag(PAGE_END, "string store end")?;
    Ok(store)
}

fn charge(budget: &mut u64, bytes: usize) -> Result<(), VertipaqError> {
    *budget = budget
        .checked_sub(bytes as u64)
        .ok_or(VertipaqError::LimitExceeded {
            what: "decoded dictionary string bytes",
            value: bytes as u64,
            limit: *budget,
        })?;
    Ok(())
}

/// Splits UTF-16LE code units at NUL terminators (a trailing unterminated piece is kept).
fn split_plain(units: &[u8], budget: &mut u64) -> Result<Vec<String>, VertipaqError> {
    let mut strings = Vec::new();
    let mut current = String::new();
    let code_units = units
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| u16::from_le_bytes(*pair));
    for c in char::decode_utf16(code_units) {
        match c {
            Ok('\0') => {
                charge(budget, current.len())?;
                strings.push(std::mem::take(&mut current));
            }
            Ok(c) => current.push(c),
            Err(_) => current.push(char::REPLACEMENT_CHARACTER),
        }
    }
    if !current.is_empty() {
        charge(budget, current.len())?;
        strings.push(current);
    }
    Ok(strings)
}

fn decode_compressed(
    store: &Store<'_>,
    offsets: &[u32],
    budget: &mut u64,
) -> Result<Vec<String>, VertipaqError> {
    let Store::Compressed {
        total_bits,
        multi_charset,
        charset_byte,
        encode_array,
        buffer,
    } = *store
    else {
        return Ok(Vec::new());
    };
    let stream = BitStream::new(buffer);
    if total_bits > stream.bit_len() {
        return Err(VertipaqError::corrupt(
            WHAT,
            format!(
                "{total_bits} bits declared in a {}-byte buffer",
                buffer.len()
            ),
        ));
    }
    if offsets.is_empty() {
        return Ok(Vec::new());
    }
    let table = HuffmanTable::new(encode_array)?;
    let mut strings = Vec::with_capacity(offsets.len());
    let mut symbols = Vec::new();
    for (i, &start) in offsets.iter().enumerate() {
        let start = u64::from(start);
        let end = offsets
            .get(i + 1)
            .map_or(total_bits, |&next| u64::from(next));
        if start > end || end > total_bits {
            return Err(VertipaqError::corrupt(
                WHAT,
                format!("string {i} spans bits {start}..{end} of {total_bits}"),
            ));
        }
        symbols.clear();
        // Every character needs at least one symbol (two in multi-charset mode) and
        // yields at least one UTF-8 byte.
        let max_symbols = usize::try_from(budget.saturating_mul(2)).unwrap_or(usize::MAX);
        table.decode(&stream, start, end, &mut symbols, max_symbols)?;
        let text = if multi_charset {
            let units = symbols
                .as_chunks::<2>()
                .0
                .iter()
                .map(|pair| u16::from_le_bytes(*pair));
            char::decode_utf16(units)
                .map(|c| c.unwrap_or(char::REPLACEMENT_CHARACTER))
                .collect::<String>()
        } else if charset_byte == 0 {
            symbols.iter().map(|&b| char::from(b)).collect::<String>()
        } else {
            let high = u16::from(charset_byte) << 8;
            char::decode_utf16(symbols.iter().map(|&b| high | u16::from(b)))
                .map(|c| c.unwrap_or(char::REPLACEMENT_CHARACTER))
                .collect::<String>()
        };
        charge(budget, text.len())?;
        strings.push(text);
    }
    Ok(strings)
}
