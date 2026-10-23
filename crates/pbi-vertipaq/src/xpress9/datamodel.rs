//! PBIX `DataModel` stream framing on top of XPress9.
//!
//! ```text
//! single-threaded:  UTF-16LE "This backup was created using XPress9 compression." + U+0000
//!                   (102 bytes), then chunks until end of stream
//! multi-threaded:   UTF-16LE "This backup was created using multithreaded XPrs9." + U+0000
//!                   (102 bytes), 5 x u64 LE: main chunks/thread, prefix chunks/thread,
//!                   prefix thread count, main thread count, chunk uncompressed size;
//!                   then the prefix groups followed by the main groups, each group
//!                   being `chunks/thread` consecutive chunks of one independent session
//! uncompressed:     the ABF backup itself, starting with FF FE + UTF-16LE
//!                   "STREAM_STORAGE_SIGNATURE_)!@#$%^&*(" (72 bytes)
//! chunk:            u32 LE uncompressed size, u32 LE compressed size, compressed bytes
//!                   (one or more whole XPress9 blocks)
//! ```
//!
//! In the single-threaded layout all chunks form one XPress9 session (history carries
//! over between chunks); in the multi-threaded layout each group is its own session.

use std::io::{self, Read};

use super::Limits;
use super::decoder::Xpress9Decoder;
use super::error::Xpress9Error;

/// Signature (UTF-16LE, followed by a UTF-16 NUL) of a single-threaded XPress9 stream.
pub const SINGLE_THREADED_SIGNATURE: &str = "This backup was created using XPress9 compression.";
/// Signature (UTF-16LE, followed by a UTF-16 NUL) of a multi-threaded XPress9 stream.
pub const MULTI_THREADED_SIGNATURE: &str = "This backup was created using multithreaded XPrs9.";
/// Signature (after a UTF-16LE BOM) at the start of an uncompressed ABF backup.
pub const STREAM_STORAGE_SIGNATURE: &str = "STREAM_STORAGE_SIGNATURE_)!@#$%^&*(";
/// Length of the XPress9 signature area at the start of a compressed stream.
pub const XPRESS9_SIGNATURE_LEN: usize = 102;
/// Length of the uncompressed ABF signature (BOM + UTF-16LE text).
pub const STREAM_STORAGE_SIGNATURE_LEN: usize = 72;
const MULTI_THREADED_HEADER_LEN: usize = 40;
const CHUNK_HEADER_LEN: usize = 8;
const RAW_PIECE: u64 = 1 << 20;

/// How a `DataModel` stream is encoded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum DataModelFormat {
    /// A plain ABF backup (no XPress9 layer).
    Uncompressed,
    /// One XPress9 session split into sequential chunks.
    Xpress9SingleThreaded,
    /// Independent XPress9 sessions ("thread groups") laid out sequentially.
    Xpress9MultiThreaded,
}

/// The header that follows the signature of a multi-threaded stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MultiThreadedHeader {
    /// Chunks in each main group.
    pub main_chunks_per_thread: u64,
    /// Chunks in each prefix group.
    pub prefix_chunks_per_thread: u64,
    /// Number of prefix groups (decoded first).
    pub prefix_thread_count: u64,
    /// Number of main groups.
    pub main_thread_count: u64,
    /// Nominal uncompressed chunk size (informational).
    pub chunk_uncompressed_size: u64,
}

impl MultiThreadedHeader {
    fn parse(bytes: &[u8; MULTI_THREADED_HEADER_LEN]) -> Self {
        let mut v = [0u64; 5];
        for (value, raw) in v.iter_mut().zip(bytes.as_chunks::<8>().0) {
            *value = u64::from_le_bytes(*raw);
        }
        Self {
            main_chunks_per_thread: v[0],
            prefix_chunks_per_thread: v[1],
            prefix_thread_count: v[2],
            main_thread_count: v[3],
            chunk_uncompressed_size: v[4],
        }
    }
}

fn starts_with_utf16le(bytes: &[u8], text: &str) -> bool {
    let mut at = 0usize;
    for unit in text.encode_utf16() {
        if bytes.get(at..at + 2) != Some(&unit.to_le_bytes()[..]) {
            return false;
        }
        at += 2;
    }
    true
}

/// Identifies the encoding of a `DataModel` stream from its first bytes (102 bytes are
/// enough). Returns `None` if the prefix matches no known signature.
#[must_use]
pub fn detect_format(prefix: &[u8]) -> Option<DataModelFormat> {
    if prefix.starts_with(&[0xFF, 0xFE])
        && starts_with_utf16le(
            prefix.get(2..).unwrap_or_default(),
            STREAM_STORAGE_SIGNATURE,
        )
    {
        Some(DataModelFormat::Uncompressed)
    } else if starts_with_utf16le(prefix, SINGLE_THREADED_SIGNATURE) {
        Some(DataModelFormat::Xpress9SingleThreaded)
    } else if starts_with_utf16le(prefix, MULTI_THREADED_SIGNATURE) {
        Some(DataModelFormat::Xpress9MultiThreaded)
    } else {
        None
    }
}

/// Reads until `buf` is full or the source is exhausted; returns the bytes read.
fn read_up_to<R: Read>(src: &mut R, buf: &mut [u8]) -> io::Result<usize> {
    let mut filled = 0;
    while filled < buf.len() {
        match src.read(&mut buf[filled..]) {
            Ok(0) => break,
            Ok(n) => filled += n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e),
        }
    }
    Ok(filled)
}

/// Position within the prefix/main groups of a multi-threaded stream.
#[derive(Debug, Clone, Copy)]
struct GroupCursor {
    /// (group count, chunks per group) for the prefix phase and the main phase.
    phases: [(u64, u64); 2],
    phase: usize,
    group: u64,
    chunk: u64,
}

impl GroupCursor {
    fn new(header: &MultiThreadedHeader) -> Self {
        Self {
            phases: [
                (header.prefix_thread_count, header.prefix_chunks_per_thread),
                (header.main_thread_count, header.main_chunks_per_thread),
            ],
            phase: 0,
            group: 0,
            chunk: 0,
        }
    }

    /// Advances to the next chunk; returns whether it starts a new group, or `None` at
    /// the end of the stream.
    fn next(&mut self) -> Option<bool> {
        while let Some(&(groups, per_group)) = self.phases.get(self.phase) {
            if groups == 0 || per_group == 0 || self.group >= groups {
                self.phase += 1;
                self.group = 0;
                self.chunk = 0;
                continue;
            }
            let starts_group = self.chunk == 0;
            self.chunk += 1;
            if self.chunk == per_group {
                self.chunk = 0;
                self.group += 1;
            }
            return Some(starts_group);
        }
        None
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Current {
    Nothing,
    Decoded,
    Raw,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    Running,
    Finished,
    Failed,
}

/// Streaming decoder for a PBIX `DataModel` stream (or a raw `.abf`).
///
/// Use [`DataModelDecoder::next_chunk`] to receive the decoded ABF bytes chunk by chunk,
/// or use it as a [`std::io::Read`]. Memory use is bounded by one compressed chunk, one
/// decoded chunk and the XPress9 window (at most 8 MiB, typically 4 MiB).
pub struct DataModelDecoder<R> {
    src: R,
    limits: Limits,
    format: DataModelFormat,
    multi: Option<MultiThreadedHeader>,
    cursor: Option<GroupCursor>,
    decoder: Xpress9Decoder,
    compressed: Vec<u8>,
    raw: Vec<u8>,
    current: Current,
    read_pos: usize,
    produced: u64,
    chunk_index: u64,
    input_offset: u64,
    state: State,
}

impl<R> std::fmt::Debug for DataModelDecoder<R> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DataModelDecoder")
            .field("format", &self.format)
            .field("multi", &self.multi)
            .field("limits", &self.limits)
            .field("produced", &self.produced)
            .field("chunk_index", &self.chunk_index)
            .field("input_offset", &self.input_offset)
            .field("state", &self.state)
            .finish_non_exhaustive()
    }
}

impl<R: Read> DataModelDecoder<R> {
    /// Reads and validates the stream signature (and the multi-threaded header).
    pub fn new(mut src: R, limits: Limits) -> Result<Self, Xpress9Error> {
        let mut prefix = [0u8; XPRESS9_SIGNATURE_LEN];
        let got = read_up_to(&mut src, &mut prefix)?;
        if got == 0 {
            return Err(Xpress9Error::Empty);
        }
        let prefix = &prefix[..got];
        let format = detect_format(prefix).ok_or(Xpress9Error::UnknownSignature)?;
        let mut decoder = Self {
            src,
            limits,
            format,
            multi: None,
            cursor: None,
            decoder: Xpress9Decoder::new(limits),
            compressed: Vec::new(),
            raw: Vec::new(),
            current: Current::Nothing,
            read_pos: 0,
            produced: 0,
            chunk_index: 0,
            input_offset: got as u64,
            state: State::Running,
        };
        match format {
            DataModelFormat::Uncompressed => {
                // The signature bytes are part of the ABF itself; emit them first.
                decoder.raw.extend_from_slice(prefix);
            }
            DataModelFormat::Xpress9SingleThreaded => {}
            DataModelFormat::Xpress9MultiThreaded => {
                let mut header = [0u8; MULTI_THREADED_HEADER_LEN];
                if read_up_to(&mut decoder.src, &mut header)? != MULTI_THREADED_HEADER_LEN {
                    return Err(Xpress9Error::Truncated {
                        what: "multi-threaded stream header",
                        offset: XPRESS9_SIGNATURE_LEN as u64,
                    });
                }
                let header = MultiThreadedHeader::parse(&header);
                decoder.input_offset += MULTI_THREADED_HEADER_LEN as u64;
                decoder.multi = Some(header);
                decoder.cursor = Some(GroupCursor::new(&header));
            }
        }
        tracing::debug!(?format, multi = ?decoder.multi, "DataModel stream detected");
        Ok(decoder)
    }

    /// The stream encoding.
    pub fn format(&self) -> DataModelFormat {
        self.format
    }

    /// The multi-threaded header, for multi-threaded streams.
    pub fn multi_threaded_header(&self) -> Option<MultiThreadedHeader> {
        self.multi
    }

    /// Total number of decoded bytes produced so far.
    pub fn produced(&self) -> u64 {
        self.produced
    }

    /// Decodes the next chunk and returns its bytes, or `None` at the end of the stream.
    ///
    /// The returned slice can be empty (empty chunks are legal).
    pub fn next_chunk(&mut self) -> Result<Option<&[u8]>, Xpress9Error> {
        if self.advance()? {
            Ok(Some(self.current_slice()))
        } else {
            Ok(None)
        }
    }

    fn current_slice(&self) -> &[u8] {
        match self.current {
            Current::Nothing => &[],
            Current::Decoded => self.decoder.output(),
            Current::Raw => &self.raw,
        }
    }

    fn advance(&mut self) -> Result<bool, Xpress9Error> {
        match self.state {
            State::Failed => return Err(Xpress9Error::DecoderFailed),
            State::Finished => return Ok(false),
            State::Running => {}
        }
        self.read_pos = 0;
        self.current = Current::Nothing;
        let result = match self.format {
            DataModelFormat::Uncompressed => self.advance_raw(),
            DataModelFormat::Xpress9SingleThreaded => self.advance_single(),
            DataModelFormat::Xpress9MultiThreaded => self.advance_multi(),
        };
        match result {
            Ok(true) => Ok(true),
            Ok(false) => {
                self.state = State::Finished;
                tracing::debug!(
                    produced = self.produced,
                    chunks = self.chunk_index,
                    "DataModel stream decoded"
                );
                Ok(false)
            }
            Err(err) => {
                self.state = State::Failed;
                Err(err)
            }
        }
    }

    fn charge_output(&mut self, bytes: u64) -> Result<(), Xpress9Error> {
        let total = self.produced.saturating_add(bytes);
        if total > self.limits.max_output_bytes {
            return Err(Xpress9Error::LimitExceeded {
                what: "total decompressed size",
                value: total,
                limit: self.limits.max_output_bytes,
            });
        }
        self.produced = total;
        Ok(())
    }

    fn advance_raw(&mut self) -> Result<bool, Xpress9Error> {
        if self.produced == 0 && !self.raw.is_empty() {
            self.charge_output(self.raw.len() as u64)?;
            self.current = Current::Raw;
            return Ok(true);
        }
        self.raw.clear();
        (&mut self.src).take(RAW_PIECE).read_to_end(&mut self.raw)?;
        if self.raw.is_empty() {
            return Ok(false);
        }
        self.charge_output(self.raw.len() as u64)?;
        self.input_offset += self.raw.len() as u64;
        self.current = Current::Raw;
        Ok(true)
    }

    fn advance_single(&mut self) -> Result<bool, Xpress9Error> {
        match self.read_chunk_header()? {
            None => Ok(false),
            Some((uncompressed, compressed)) => {
                self.decode_chunk(uncompressed, compressed)?;
                Ok(true)
            }
        }
    }

    fn advance_multi(&mut self) -> Result<bool, Xpress9Error> {
        let Some(starts_group) = self.cursor.as_mut().and_then(GroupCursor::next) else {
            let mut probe = [0u8; 1];
            if read_up_to(&mut self.src, &mut probe)? != 0 {
                tracing::warn!(
                    offset = self.input_offset,
                    "ignoring trailing bytes after the last multi-threaded chunk group"
                );
            }
            return Ok(false);
        };
        if starts_group {
            self.decoder.reset();
        }
        let (uncompressed, compressed) =
            self.read_chunk_header()?.ok_or(Xpress9Error::Truncated {
                what: "chunk header (stream ended before the declared chunk groups)",
                offset: self.input_offset,
            })?;
        self.decode_chunk(uncompressed, compressed)?;
        Ok(true)
    }

    /// Reads a chunk header; `None` if the stream ends cleanly before it.
    fn read_chunk_header(&mut self) -> Result<Option<(u32, u32)>, Xpress9Error> {
        let mut header = [0u8; CHUNK_HEADER_LEN];
        match read_up_to(&mut self.src, &mut header)? {
            0 => Ok(None),
            CHUNK_HEADER_LEN => {
                let uncompressed = u32::from_le_bytes([header[0], header[1], header[2], header[3]]);
                let compressed = u32::from_le_bytes([header[4], header[5], header[6], header[7]]);
                Ok(Some((uncompressed, compressed)))
            }
            _ => Err(Xpress9Error::Truncated {
                what: "chunk header",
                offset: self.input_offset,
            }),
        }
    }

    fn decode_chunk(&mut self, uncompressed: u32, compressed: u32) -> Result<(), Xpress9Error> {
        let chunk = self.chunk_index;
        let offset = self.input_offset;
        self.decode_chunk_inner(uncompressed, compressed)
            .map_err(|source| Xpress9Error::InChunk {
                chunk,
                offset,
                source: Box::new(source),
            })?;
        self.chunk_index += 1;
        self.input_offset += CHUNK_HEADER_LEN as u64 + u64::from(compressed);
        self.current = Current::Decoded;
        Ok(())
    }

    fn decode_chunk_inner(
        &mut self,
        uncompressed: u32,
        compressed: u32,
    ) -> Result<(), Xpress9Error> {
        let limits = self.limits;
        if u64::from(compressed) > limits.max_chunk_compressed_bytes {
            return Err(Xpress9Error::LimitExceeded {
                what: "compressed chunk size",
                value: u64::from(compressed),
                limit: limits.max_chunk_compressed_bytes,
            });
        }
        if u64::from(uncompressed) > limits.max_chunk_uncompressed_bytes {
            return Err(Xpress9Error::LimitExceeded {
                what: "uncompressed chunk size",
                value: u64::from(uncompressed),
                limit: limits.max_chunk_uncompressed_bytes,
            });
        }
        self.charge_output(u64::from(uncompressed))?;

        // `take` + `read_to_end` grows the buffer with the data actually present, so a
        // lying size field cannot force a large allocation.
        self.compressed.clear();
        let got = (&mut self.src)
            .take(u64::from(compressed))
            .read_to_end(&mut self.compressed)?;
        if got as u64 != u64::from(compressed) {
            return Err(Xpress9Error::Truncated {
                what: "chunk data",
                offset: self.input_offset + CHUNK_HEADER_LEN as u64,
            });
        }
        let expected = usize::try_from(uncompressed)
            .map_err(|_| Xpress9Error::OutOfMemory(u64::from(uncompressed)))?;
        self.decoder.decompress_chunk(&self.compressed, expected)?;
        Ok(())
    }
}

impl<R: Read> Read for DataModelDecoder<R> {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        if out.is_empty() {
            return Ok(0);
        }
        loop {
            let available = self
                .current_slice()
                .get(self.read_pos..)
                .unwrap_or_default();
            if !available.is_empty() {
                let n = available.len().min(out.len());
                out[..n].copy_from_slice(&available[..n]);
                self.read_pos += n;
                return Ok(n);
            }
            match self.advance() {
                Ok(true) => {}
                Ok(false) => return Ok(0),
                Err(Xpress9Error::Io(err)) => return Err(err),
                Err(err) => return Err(io::Error::new(io::ErrorKind::InvalidData, err)),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::xpress9::test_support::{BlockSpec, Tok, encode_block};

    fn signature(text: &str) -> Vec<u8> {
        let mut out: Vec<u8> = text.encode_utf16().flat_map(u16::to_le_bytes).collect();
        out.extend_from_slice(&[0, 0]);
        out
    }

    fn chunk(uncompressed: u32, data: &[u8]) -> Vec<u8> {
        let mut out = uncompressed.to_le_bytes().to_vec();
        out.extend_from_slice(&(data.len() as u32).to_le_bytes());
        out.extend_from_slice(data);
        out
    }

    fn block(index: u32, session: u32, bytes: &[u8]) -> Vec<u8> {
        let mut spec = BlockSpec::new(bytes.iter().map(|&b| Tok::Lit(b)).collect());
        spec.block_index = index;
        spec.session = session;
        encode_block(&spec)
    }

    fn decode_all(data: &[u8], limits: Limits) -> Result<Vec<u8>, Xpress9Error> {
        let mut dec = DataModelDecoder::new(data, limits)?;
        let mut out = Vec::new();
        while let Some(piece) = dec.next_chunk()? {
            out.extend_from_slice(piece);
        }
        Ok(out)
    }

    #[test]
    fn signatures_are_102_bytes() {
        assert_eq!(
            signature(SINGLE_THREADED_SIGNATURE).len(),
            XPRESS9_SIGNATURE_LEN
        );
        assert_eq!(
            signature(MULTI_THREADED_SIGNATURE).len(),
            XPRESS9_SIGNATURE_LEN
        );
        let mut abf = vec![0xFF, 0xFE];
        abf.extend(
            STREAM_STORAGE_SIGNATURE
                .encode_utf16()
                .flat_map(u16::to_le_bytes),
        );
        assert_eq!(abf.len(), STREAM_STORAGE_SIGNATURE_LEN);
        assert_eq!(detect_format(&abf), Some(DataModelFormat::Uncompressed));
        assert_eq!(detect_format(b"PK\x03\x04"), None);
    }

    #[test]
    fn single_threaded_framing() {
        let mut data = signature(SINGLE_THREADED_SIGNATURE);
        data.extend(chunk(3, &block(0, 7, b"abc")));
        data.extend(chunk(0, &[]));
        data.extend(chunk(2, &block(1, 7, b"de")));
        assert_eq!(decode_all(&data, Limits::default()).unwrap(), b"abcde");

        // Chunk header claims more output than the blocks produce.
        let mut lying = signature(SINGLE_THREADED_SIGNATURE);
        lying.extend(chunk(4, &block(0, 7, b"abc")));
        let err = decode_all(&lying, Limits::default()).unwrap_err();
        assert!(
            matches!(err.innermost(), Xpress9Error::SizeMismatch { .. }),
            "{err}"
        );

        // Compressed size larger than the remaining input.
        let mut short = signature(SINGLE_THREADED_SIGNATURE);
        let mut c = chunk(3, &block(0, 7, b"abc"));
        c[4..8].copy_from_slice(&1_000_000u32.to_le_bytes());
        short.extend(c);
        let err = decode_all(&short, Limits::default()).unwrap_err();
        assert!(
            matches!(err.innermost(), Xpress9Error::Truncated { .. }),
            "{err}"
        );

        // Partial chunk header.
        let mut partial = signature(SINGLE_THREADED_SIGNATURE);
        partial.extend_from_slice(&[1, 2, 3]);
        assert!(matches!(
            decode_all(&partial, Limits::default()),
            Err(Xpress9Error::Truncated { .. })
        ));

        // Total output limit.
        let limits = Limits {
            max_output_bytes: 4,
            ..Limits::default()
        };
        let err = decode_all(&data, limits).unwrap_err();
        assert!(
            matches!(err.innermost(), Xpress9Error::LimitExceeded { .. }),
            "{err}"
        );
    }

    #[test]
    fn multi_threaded_framing() {
        let mut data = signature(MULTI_THREADED_SIGNATURE);
        for v in [2u64, 1, 1, 2, 4096] {
            data.extend_from_slice(&v.to_le_bytes());
        }
        // prefix group (1 chunk), then two main groups of 2 chunks; each group restarts
        // the block numbering because it is a new session.
        data.extend(chunk(1, &block(0, 1, b"P")));
        data.extend(chunk(2, &block(0, 2, b"ab")));
        data.extend(chunk(1, &block(1, 2, b"c")));
        data.extend(chunk(1, &block(0, 3, b"d")));
        data.extend(chunk(2, &block(1, 3, b"ef")));
        assert_eq!(decode_all(&data, Limits::default()).unwrap(), b"Pabcdef");

        let mut dec = DataModelDecoder::new(&data[..], Limits::default()).unwrap();
        assert_eq!(dec.format(), DataModelFormat::Xpress9MultiThreaded);
        assert_eq!(
            dec.multi_threaded_header().map(|h| h.main_thread_count),
            Some(2)
        );
        let mut via_read = Vec::new();
        dec.read_to_end(&mut via_read).unwrap();
        assert_eq!(via_read, b"Pabcdef");

        // Missing final chunk.
        let cut = data.len() - chunk(2, &block(1, 3, b"ef")).len();
        let err = decode_all(&data[..cut], Limits::default()).unwrap_err();
        assert!(matches!(err, Xpress9Error::Truncated { .. }), "{err}");

        // Truncated multi-threaded header.
        let header_only = &data[..XPRESS9_SIGNATURE_LEN + 10];
        assert!(matches!(
            DataModelDecoder::new(header_only, Limits::default()),
            Err(Xpress9Error::Truncated { .. })
        ));
    }

    #[test]
    fn uncompressed_passthrough_and_bad_signatures() {
        let mut abf = vec![0xFF, 0xFE];
        abf.extend(
            STREAM_STORAGE_SIGNATURE
                .encode_utf16()
                .flat_map(u16::to_le_bytes),
        );
        abf.extend((0..3_000_000u32).map(|i| (i % 251) as u8));
        assert_eq!(decode_all(&abf, Limits::default()).unwrap(), abf);
        let limits = Limits {
            max_output_bytes: 1000,
            ..Limits::default()
        };
        assert!(matches!(
            decode_all(&abf, limits),
            Err(Xpress9Error::LimitExceeded { .. })
        ));

        assert!(matches!(
            decode_all(&[], Limits::default()),
            Err(Xpress9Error::Empty)
        ));
        assert!(matches!(
            decode_all(b"not a data model", Limits::default()),
            Err(Xpress9Error::UnknownSignature)
        ));
        let sig = signature(SINGLE_THREADED_SIGNATURE);
        assert!(matches!(
            decode_all(&sig[..50], Limits::default()),
            Err(Xpress9Error::UnknownSignature)
        ));
        assert_eq!(
            decode_all(&sig, Limits::default()).unwrap(),
            Vec::<u8>::new()
        );
    }

    #[test]
    fn read_adapter_reports_errors_as_invalid_data() {
        let mut data = signature(SINGLE_THREADED_SIGNATURE);
        data.extend(chunk(9, &block(0, 7, b"abc")));
        let mut dec = DataModelDecoder::new(&data[..], Limits::default()).unwrap();
        let err = dec.read_to_end(&mut Vec::new()).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
        assert!(dec.read(&mut [0u8; 4]).is_err());
    }
}
