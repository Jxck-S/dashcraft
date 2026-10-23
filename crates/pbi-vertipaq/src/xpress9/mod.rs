//! XPress9 decompression for Power BI `DataModel` streams.
//!
//! A PBIX `DataModel` ZIP entry (and the `xl/model/item.data` part of a Power Pivot
//! workbook) is an Analysis Services backup (ABF) compressed with Microsoft's XPress9
//! codec — LZ77 with a 2^16..2^23 byte window, move-to-front offset reuse and canonical
//! Huffman coding — inside a small chunked container. This module decodes:
//!
//! * the container: UTF-16LE signature, single-threaded (one session) and multi-threaded
//!   (independent per-thread sessions) chunk layouts, and uncompressed ABF passthrough
//!   ([`DataModelDecoder`], [`decompress_datamodel`], [`detect_format`]);
//! * XPress9 sessions and blocks ([`Xpress9Decoder`], [`decompress_block`]).
//!
//! The output of [`decompress_datamodel`] is the raw ABF backup, which starts with
//! `FF FE` + UTF-16LE `STREAM_STORAGE_SIGNATURE_)!@#$%^&*(`.
//!
//! All input is treated as untrusted: every structure is bounds-checked, output sizes
//! are capped by [`Limits`], memory grows only with data actually decoded, and malformed
//! input returns an [`Xpress9Error`] instead of panicking.
//!
//! Clean-room note: this is original Rust code. The bit-level decoding rules follow
//! Microsoft's MIT-licensed XPress9 reference implementation and the container framing
//! follows pbixray (MIT); see `crates/pbi-vertipaq/REFERENCES.md` and `FORMAT-NOTES.md`.

mod bitreader;
mod crc32c;
mod datamodel;
mod decoder;
mod error;
mod huffman;
#[cfg(test)]
mod test_support;

pub use datamodel::{
    DataModelDecoder, DataModelFormat, MULTI_THREADED_SIGNATURE, MultiThreadedHeader,
    SINGLE_THREADED_SIGNATURE, STREAM_STORAGE_SIGNATURE, STREAM_STORAGE_SIGNATURE_LEN,
    XPRESS9_SIGNATURE_LEN, detect_format,
};
pub use decoder::{StreamParams, Xpress9Decoder};
pub use error::Xpress9Error;

/// Resource limits that protect against decompression bombs and lying size fields.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// Maximum total decompressed size of a stream (default 4 GiB).
    pub max_output_bytes: u64,
    /// Maximum declared uncompressed size of one chunk (default 2 GiB − 1, the largest
    /// chunk the reference implementation can represent).
    pub max_chunk_uncompressed_bytes: u64,
    /// Maximum compressed size of one chunk (default 2 GiB − 1).
    pub max_chunk_compressed_bytes: u64,
}

impl Limits {
    /// The default limits.
    pub const DEFAULT: Limits = Limits {
        max_output_bytes: 4 << 30,
        max_chunk_uncompressed_bytes: i32::MAX as u64,
        max_chunk_compressed_bytes: i32::MAX as u64,
    };

    /// Small limits suitable for fuzzing and untrusted previews (64 MiB output,
    /// 16 MiB per chunk).
    pub const SMALL: Limits = Limits {
        max_output_bytes: 64 << 20,
        max_chunk_uncompressed_bytes: 16 << 20,
        max_chunk_compressed_bytes: 16 << 20,
    };

    /// Returns these limits with a different total output cap.
    #[must_use]
    pub const fn with_max_output_bytes(mut self, bytes: u64) -> Self {
        self.max_output_bytes = bytes;
        self
    }
}

impl Default for Limits {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// Decompresses a whole PBIX `DataModel` stream into the raw ABF backup bytes.
///
/// Accepts single-threaded and multi-threaded XPress9 streams and uncompressed ABF
/// backups (returned unchanged).
pub fn decompress_datamodel(bytes: &[u8], limits: &Limits) -> Result<Vec<u8>, Xpress9Error> {
    let mut decoder = DataModelDecoder::new(bytes, *limits)?;
    let mut out = Vec::new();
    while let Some(piece) = decoder.next_chunk()? {
        out.try_reserve(piece.len())
            .map_err(|_| Xpress9Error::OutOfMemory(piece.len() as u64))?;
        out.extend_from_slice(piece);
    }
    Ok(out)
}

/// Decompresses one standalone XPress9 buffer — the payload of a single `DataModel`
/// chunk that starts a session (one or more consecutive blocks) — which must expand to
/// exactly `uncompressed_len` bytes.
///
/// For chunks that continue a session use [`Xpress9Decoder`].
pub fn decompress_block(
    compressed: &[u8],
    uncompressed_len: usize,
    limits: &Limits,
) -> Result<Vec<u8>, Xpress9Error> {
    let mut decoder = Xpress9Decoder::new(*limits);
    decoder
        .decompress_chunk(compressed, uncompressed_len)
        .map(<[u8]>::to_vec)
}

/// Fuzzing entry point: runs every decoder over `data` with [`Limits::SMALL`] and
/// discards the result. Never panics.
pub fn fuzz_decompress(data: &[u8]) {
    let limits = Limits::SMALL;
    let _ = decompress_datamodel(data, &limits);
    if let Some((len, rest)) = data.split_first_chunk::<4>() {
        let expected = (u32::from_le_bytes(*len) % (1 << 20)) as usize;
        let _ = decompress_block(rest, expected, &limits);
    }
    if let Ok(mut decoder) = DataModelDecoder::new(data, limits) {
        let mut sink = [0u8; 4096];
        while let Ok(n) = std::io::Read::read(&mut decoder, &mut sink) {
            if n == 0 {
                break;
            }
        }
    }
}
