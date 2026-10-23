//! Xpress ("XPress8") decompression of files stored compressed inside an ABF backup.
//!
//! When a backup's header sets `ApplyCompression` — as Power Pivot workbooks do for
//! `xl/model/item.data` ([MS-XLDM] §2.1) — every model file in the files section is
//! stored as a sequence of chunks:
//!
//! ```text
//! chunk := u16 LE uncompressed_size, u16 LE compressed_size, u8[compressed_size] payload
//! ```
//!
//! * `compressed_size == uncompressed_size`: the payload is stored verbatim;
//! * otherwise the payload is one **independent** [MS-XCA] §2.3 *Plain LZ77* block (no
//!   history is shared between chunks) that decodes to exactly `uncompressed_size`
//!   bytes.
//!
//! The concatenated output must equal the file size recorded in the backup log. The
//! backup log, the virtual directory and the `ADDITIONAL_LOG`/`PARTITIONS` entries are
//! never compressed.
//!
//! Plain LZ77 ([MS-XCA] §2.4): the block starts with a 32-bit little-endian flag word;
//! flags are consumed from the most significant bit, and a new flag word is read after
//! 32 tokens. A `0` flag is a literal byte; a `1` flag is a match: a little-endian
//! `u16` with offset `(v >> 3) + 1` and length `v & 7`. Length 7 continues in a shared
//! nibble (low nibble of a fresh byte, then the high nibble of that same byte for the
//! next long match), nibble 15 continues in a byte, byte 255 in a `u16` (and a zero
//! `u16` in a `u32`) holding the full length minus 3 (it must be ≥ 22); the final match
//! length is the sum plus 3. Matches may overlap their output.
//!
//! Every input is untrusted: sizes are checked before allocating, offsets must stay
//! inside the current chunk's output, output never exceeds the declared sizes, and every
//! malformed input returns an [`Xpress8Error`].
//!
//! Clean-room note: original Rust written from the [MS-XCA] specification; the chunk
//! framing and the identification of the codec as Microsoft's ESE `_xpress` library
//! (`CODING_DIRECT2`, MIT) come from `xpress8-python` and pbixray (both MIT). See
//! `crates/pbi-vertipaq/REFERENCES.md` and `FORMAT-NOTES.md` §4.1.

use thiserror::Error;

/// Size of a chunk header (two little-endian `u16`).
pub const CHUNK_HEADER_LEN: usize = 4;

/// Largest output one chunk can declare.
pub const MAX_CHUNK_SIZE: usize = u16::MAX as usize;

/// Errors produced while decompressing an Xpress-compressed file.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum Xpress8Error {
    /// The input ended inside a chunk header or payload.
    #[error("chunk {chunk} at offset {offset}: input truncated")]
    Truncated {
        /// Index of the chunk.
        chunk: usize,
        /// Byte offset of the chunk header in the input.
        offset: usize,
    },
    /// A chunk declares a payload larger than its output.
    #[error(
        "chunk {chunk} at offset {offset}: compressed size {compressed} exceeds uncompressed size {uncompressed}"
    )]
    InvalidChunk {
        /// Index of the chunk.
        chunk: usize,
        /// Byte offset of the chunk header in the input.
        offset: usize,
        /// Declared compressed size.
        compressed: usize,
        /// Declared uncompressed size.
        uncompressed: usize,
    },
    /// The LZ77 stream of a chunk is malformed.
    #[error("chunk {chunk}: {message}")]
    Corrupt {
        /// Index of the chunk.
        chunk: usize,
        /// What is wrong (no content).
        message: String,
    },
    /// The decompressed size differs from the size recorded in the backup log.
    #[error("decompressed {actual} bytes, the backup log records {expected}")]
    SizeMismatch {
        /// Size from the backup log.
        expected: u64,
        /// Bytes produced (or about to be produced).
        actual: u64,
    },
    /// The output would exceed the caller's cap.
    #[error("decompressed size exceeds the limit of {limit} bytes")]
    LimitExceeded {
        /// The cap.
        limit: u64,
    },
}

/// Error of [`decompress_block`] (without chunk context).
#[derive(Debug, Clone, PartialEq, Eq)]
struct BlockError(String);

fn corrupt(message: impl Into<String>) -> BlockError {
    BlockError(message.into())
}

struct Input<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl Input<'_> {
    fn u8(&mut self) -> Result<u8, BlockError> {
        let b = *self
            .bytes
            .get(self.pos)
            .ok_or_else(|| corrupt("input ends inside a token"))?;
        self.pos += 1;
        Ok(b)
    }

    fn array<const N: usize>(&mut self) -> Result<[u8; N], BlockError> {
        let end = self.pos.checked_add(N).filter(|&e| e <= self.bytes.len());
        let slice = end
            .and_then(|end| self.bytes.get(self.pos..end))
            .ok_or_else(|| corrupt("input ends inside a token"))?;
        let mut out = [0u8; N];
        out.copy_from_slice(slice);
        self.pos += N;
        Ok(out)
    }
}

/// Decodes one Plain LZ77 block, appending exactly `size` bytes to `out`. Matches may
/// only reference bytes produced by this block. A final match that runs past `size` is
/// truncated, as the reference decoder does.
fn decode_block_into(input: &[u8], size: usize, out: &mut Vec<u8>) -> Result<(), BlockError> {
    let start = out.len();
    let end = start
        .checked_add(size)
        .ok_or_else(|| corrupt("size overflow"))?;
    out.try_reserve(size)
        .map_err(|_| corrupt("cannot allocate the chunk output"))?;
    let mut input = Input {
        bytes: input,
        pos: 0,
    };
    let mut flags: u32 = 0;
    let mut flag_count: u32 = 0;
    let mut half_byte: Option<usize> = None;
    while out.len() < end {
        if flag_count == 0 {
            flags = u32::from_le_bytes(input.array::<4>()?);
            flag_count = 32;
        }
        flag_count -= 1;
        if flags & (1u32 << flag_count) == 0 {
            out.push(input.u8()?);
            continue;
        }
        let token = u16::from_le_bytes(input.array::<2>()?);
        let offset = usize::from(token >> 3) + 1;
        let mut length = usize::from(token & 7);
        if length == 7 {
            length = match half_byte.take() {
                None => {
                    half_byte = Some(input.pos);
                    usize::from(input.u8()? & 0x0F)
                }
                Some(at) => usize::from(input.bytes.get(at).copied().unwrap_or_default() >> 4),
            };
            if length == 15 {
                length = usize::from(input.u8()?);
                if length == 255 {
                    let mut full = u64::from(u16::from_le_bytes(input.array::<2>()?));
                    if full == 0 {
                        full = u64::from(u32::from_le_bytes(input.array::<4>()?));
                    }
                    if full < 15 + 7 {
                        return Err(corrupt(format!("invalid long match length {full}")));
                    }
                    length = usize::try_from(full - (15 + 7))
                        .map_err(|_| corrupt("match length overflow"))?;
                }
                length = length
                    .checked_add(15)
                    .ok_or_else(|| corrupt("match length overflow"))?;
            }
            length += 7;
        }
        length = length
            .checked_add(3)
            .ok_or_else(|| corrupt("match length overflow"))?;
        let produced = out.len() - start;
        if offset > produced {
            return Err(corrupt(format!(
                "match offset {offset} reaches before the start of the block ({produced} bytes decoded)"
            )));
        }
        let length = length.min(end - out.len());
        let from = out.len() - offset;
        if offset >= length {
            out.extend_from_within(from..from + length);
        } else {
            for i in 0..length {
                let byte = out.get(from + i).copied().unwrap_or_default();
                out.push(byte);
            }
        }
    }
    Ok(())
}

/// Decodes one standalone Plain LZ77 block of `size` output bytes.
///
/// # Errors
/// [`Xpress8Error::Corrupt`] (chunk 0) for malformed input.
pub fn decompress_block(input: &[u8], size: usize) -> Result<Vec<u8>, Xpress8Error> {
    let mut out = Vec::new();
    decode_block_into(input, size, &mut out)
        .map_err(|BlockError(message)| Xpress8Error::Corrupt { chunk: 0, message })?;
    Ok(out)
}

/// Decompresses a chunked file (see the module docs).
///
/// `expected` is the size from the backup log, if known; `max_output` caps the output
/// regardless. Memory grows only with chunks actually decoded.
///
/// # Errors
/// Any [`Xpress8Error`].
pub fn decompress_chunked(
    input: &[u8],
    expected: Option<u64>,
    max_output: u64,
) -> Result<Vec<u8>, Xpress8Error> {
    if let Some(expected) = expected {
        if expected > max_output {
            return Err(Xpress8Error::LimitExceeded { limit: max_output });
        }
        // Each chunk produces at most 65535 bytes from at least 4 input bytes.
        let possible = (input.len() as u64 / CHUNK_HEADER_LEN as u64) * MAX_CHUNK_SIZE as u64;
        if expected > possible {
            return Err(Xpress8Error::SizeMismatch {
                expected,
                actual: possible,
            });
        }
    }
    let limit = expected.unwrap_or(max_output).min(max_output);
    let mut out = Vec::new();
    if let Some(expected) = expected {
        let hint = usize::try_from(expected.min(64 << 20)).unwrap_or(0);
        out.try_reserve(hint)
            .map_err(|_| Xpress8Error::LimitExceeded { limit: expected })?;
    }
    let mut pos = 0usize;
    let mut chunk = 0usize;
    while pos < input.len() {
        let header = input
            .get(pos..pos + CHUNK_HEADER_LEN)
            .ok_or(Xpress8Error::Truncated { chunk, offset: pos })?;
        let uncompressed = usize::from(u16::from_le_bytes([header[0], header[1]]));
        let compressed = usize::from(u16::from_le_bytes([header[2], header[3]]));
        let payload_start = pos + CHUNK_HEADER_LEN;
        let payload = input
            .get(payload_start..payload_start + compressed)
            .ok_or(Xpress8Error::Truncated { chunk, offset: pos })?;
        if compressed > uncompressed {
            return Err(Xpress8Error::InvalidChunk {
                chunk,
                offset: pos,
                compressed,
                uncompressed,
            });
        }
        let total = out.len() as u64 + uncompressed as u64;
        if total > limit {
            return Err(match expected {
                Some(expected) if expected <= max_output => Xpress8Error::SizeMismatch {
                    expected,
                    actual: total,
                },
                _ => Xpress8Error::LimitExceeded { limit: max_output },
            });
        }
        if compressed == uncompressed {
            out.extend_from_slice(payload);
        } else {
            decode_block_into(payload, uncompressed, &mut out)
                .map_err(|BlockError(message)| Xpress8Error::Corrupt { chunk, message })?;
        }
        pos = payload_start + compressed;
        chunk += 1;
    }
    if let Some(expected) = expected
        && out.len() as u64 != expected
    {
        return Err(Xpress8Error::SizeMismatch {
            expected,
            actual: out.len() as u64,
        });
    }
    Ok(out)
}

/// Fuzzing entry point: decodes `data` as a chunked file and as a single block. Never
/// panics.
pub fn fuzz_xpress8(data: &[u8]) {
    let _ = decompress_chunked(data, None, 1 << 20);
    if let Some((&size, rest)) = data.split_first() {
        let _ = decompress_block(rest, usize::from(size) * 64);
    }
}

#[cfg(any(test, feature = "test-support"))]
#[doc(hidden)]
pub mod test_support {
    //! Test-vector helpers: chunk framing around stored or hand-made payloads. There is
    //! deliberately no encoder (ADR-0003: decoders only).

    use super::CHUNK_HEADER_LEN;

    /// Frames `content` as stored chunks (no compression) of at most `chunk` bytes.
    pub fn stored_chunks(content: &[u8], chunk: usize) -> Vec<u8> {
        let chunk = chunk.clamp(1, super::MAX_CHUNK_SIZE);
        let mut out = Vec::with_capacity(content.len() + CHUNK_HEADER_LEN);
        for part in content.chunks(chunk) {
            let len = u16::try_from(part.len()).unwrap_or(u16::MAX);
            out.extend_from_slice(&len.to_le_bytes());
            out.extend_from_slice(&len.to_le_bytes());
            out.extend_from_slice(part);
        }
        out
    }

    /// Plain LZ77 payload for `literal` followed by one match copying `copy` bytes
    /// from `offset` bytes back (`copy` ≥ 3, `offset` ≥ 1).
    pub fn literal_then_match(literal: &[u8], offset: usize, copy: usize) -> Vec<u8> {
        assert!(literal.len() < 31 && copy >= 3 && (1..=8192).contains(&offset));
        let flags: u32 = 1u32 << (31 - literal.len());
        let mut out = flags.to_le_bytes().to_vec();
        out.extend_from_slice(literal);
        let extra = copy - 3;
        let token_len = extra.min(7);
        let token = u16::try_from(((offset - 1) << 3) | token_len).unwrap_or(0);
        out.extend_from_slice(&token.to_le_bytes());
        if extra >= 7 {
            let rest = extra - 7;
            if rest < 15 {
                out.push(u8::try_from(rest).unwrap_or(0));
            } else {
                out.push(15);
                let rest = rest - 15;
                if rest < 255 {
                    out.push(u8::try_from(rest).unwrap_or(0));
                } else {
                    out.push(255);
                    let full = u16::try_from(extra).unwrap_or(0);
                    out.extend_from_slice(&full.to_le_bytes());
                }
            }
        }
        out
    }

    /// One compressed chunk (header + payload).
    pub fn chunk(uncompressed: usize, payload: &[u8]) -> Vec<u8> {
        let mut out = u16::try_from(uncompressed)
            .unwrap_or(u16::MAX)
            .to_le_bytes()
            .to_vec();
        out.extend_from_slice(
            &u16::try_from(payload.len())
                .unwrap_or(u16::MAX)
                .to_le_bytes(),
        );
        out.extend_from_slice(payload);
        out
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::{chunk, literal_then_match, stored_chunks};
    use super::*;

    #[test]
    fn decodes_literals_and_overlapping_matches() {
        // "abc" then copy 9 bytes from 3 back: "abcabcabcabc".
        let payload = literal_then_match(b"abc", 3, 9);
        assert_eq!(decompress_block(&payload, 12).unwrap(), b"abcabcabcabc");
        // Run-length style: offset 1.
        let payload = literal_then_match(b"x", 1, 5);
        assert_eq!(decompress_block(&payload, 6).unwrap(), b"xxxxxx");
    }

    #[test]
    fn decodes_every_length_extension() {
        for copy in [3usize, 9, 10, 24, 25, 60, 300, 4000] {
            let payload = literal_then_match(b"ab", 2, copy);
            let out = decompress_block(&payload, 2 + copy).unwrap();
            assert_eq!(out.len(), 2 + copy, "copy {copy}");
            assert!(out.chunks(2).all(|p| p[0] == b'a'), "copy {copy}");
        }
    }

    #[test]
    fn shares_the_length_nibble_between_two_long_matches() {
        // flags: literal, match, match; first long match uses the low nibble of a fresh
        // byte, the second the high nibble of that same byte.
        let flags: u32 = 0b011 << 29;
        let mut payload = flags.to_le_bytes().to_vec();
        payload.push(b'z');
        payload.extend_from_slice(&7u16.to_le_bytes()); // offset 1, length 7
        payload.push(0x21); // low nibble 1 → 3+7+1 = 11; high nibble 2 → 12
        payload.extend_from_slice(&7u16.to_le_bytes());
        let out = decompress_block(&payload, 1 + 11 + 12).unwrap();
        assert_eq!(out, vec![b'z'; 24]);
    }

    #[test]
    fn chunked_files_mix_stored_and_compressed_chunks() {
        let mut data = stored_chunks(b"hello ", 4);
        data.extend(chunk(12, &literal_then_match(b"abc", 3, 9)));
        data.extend(chunk(0, &[]));
        let out = decompress_chunked(&data, Some(18), 1 << 20).unwrap();
        assert_eq!(out, b"hello abcabcabcabc");
        assert_eq!(decompress_chunked(&[], Some(0), 10).unwrap(), b"");
    }

    #[test]
    fn rejects_malformed_and_lying_input() {
        let good = chunk(12, &literal_then_match(b"abc", 3, 9));
        // Wrong total from the backup log.
        assert!(matches!(
            decompress_chunked(&good, Some(13), 1 << 20),
            Err(Xpress8Error::SizeMismatch { .. })
        ));
        // Truncated payload and header.
        assert!(matches!(
            decompress_chunked(&good[..good.len() - 1], None, 1 << 20),
            Err(Xpress8Error::Truncated { .. })
        ));
        assert!(matches!(
            decompress_chunked(&good[..3], None, 1 << 20),
            Err(Xpress8Error::Truncated { .. })
        ));
        // Payload longer than the output.
        assert!(matches!(
            decompress_chunked(&chunk(2, b"abc"), None, 1 << 20),
            Err(Xpress8Error::InvalidChunk { .. })
        ));
        // Offset before the start of the chunk.
        assert!(matches!(
            decompress_chunked(&chunk(10, &literal_then_match(b"a", 5, 9)), None, 1 << 20),
            Err(Xpress8Error::Corrupt { .. })
        ));
        // Output cap and impossible expectations.
        assert!(matches!(
            decompress_chunked(&good, None, 5),
            Err(Xpress8Error::LimitExceeded { .. })
        ));
        assert!(matches!(
            decompress_chunked(&good, Some(1 << 40), 1 << 50),
            Err(Xpress8Error::SizeMismatch { .. })
        ));
        // Ran out of input before the declared size.
        assert!(decompress_block(&literal_then_match(b"abc", 3, 3), 100).is_err());
    }

    #[test]
    fn fuzz_entry_never_panics() {
        let mut state = 0x2545_F491_4F6C_DD1Du64;
        for len in 0..400 {
            let data: Vec<u8> = (0..len)
                .map(|_| {
                    state ^= state << 13;
                    state ^= state >> 7;
                    state ^= state << 17;
                    (state >> 24) as u8
                })
                .collect();
            fuzz_xpress8(&data);
        }
    }
}
