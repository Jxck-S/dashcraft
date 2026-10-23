use thiserror::Error;

/// Errors produced while decoding an XPress9-compressed PBIX `DataModel` stream.
///
/// Every malformed input yields one of these; the decoder never panics.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum Xpress9Error {
    /// The input contains no bytes at all.
    #[error("DataModel stream is empty")]
    Empty,
    /// The stream starts with neither an XPress9 signature nor an uncompressed ABF signature.
    #[error("unrecognised DataModel signature (not an XPress9 or uncompressed ABF backup stream)")]
    UnknownSignature,
    /// The input ended before a structure was complete.
    #[error("truncated input: {what} at input offset {offset}")]
    Truncated {
        /// The structure that was being read.
        what: &'static str,
        /// Byte offset (relative to the buffer being parsed) where the structure starts.
        offset: u64,
    },
    /// An XPress9 block header is invalid (bad magic, reserved bits, impossible sizes, ...).
    #[error("invalid XPress9 block header: {0}")]
    BadHeader(&'static str),
    /// The CRC-32C stored in an XPress9 block header does not match the header bytes.
    #[error(
        "XPress9 block header checksum mismatch (stored {stored:#010x}, computed {computed:#010x})"
    )]
    HeaderChecksum {
        /// Checksum stored in the header.
        stored: u32,
        /// Checksum computed over the first 28 header bytes.
        computed: u32,
    },
    /// Blocks of a session are out of order.
    #[error("XPress9 block sequence error: expected block index {expected}, found {found}")]
    BlockSequence {
        /// Expected block index.
        expected: u32,
        /// Block index found in the header.
        found: u32,
    },
    /// A block belongs to a different compression session than the preceding blocks.
    #[error("XPress9 session signature mismatch (expected {expected:#010x}, found {found:#010x})")]
    SessionMismatch {
        /// Session signature of the first block.
        expected: u32,
        /// Session signature found in the current block.
        found: u32,
    },
    /// An encoded Huffman code-length table is invalid.
    #[error("corrupt XPress9 Huffman table: {0}")]
    BadHuffmanTable(&'static str),
    /// The compressed symbol stream is invalid.
    #[error("corrupt XPress9 data: {0}")]
    Corrupt(&'static str),
    /// A declared size does not match the number of bytes actually decoded.
    #[error("size mismatch: {what} declares {declared} bytes but {actual} bytes were decoded")]
    SizeMismatch {
        /// The structure whose size is wrong.
        what: &'static str,
        /// Declared size.
        declared: u64,
        /// Actual (or would-be) decoded size.
        actual: u64,
    },
    /// A configured [`Limits`](super::Limits) value would be exceeded.
    #[error("limit exceeded: {what} would be {value} bytes, the limit is {limit} bytes")]
    LimitExceeded {
        /// The quantity that is limited.
        what: &'static str,
        /// The value the input asks for.
        value: u64,
        /// The configured limit.
        limit: u64,
    },
    /// Memory for the decoded output could not be reserved.
    #[error("out of memory while reserving {0} bytes for decoded output")]
    OutOfMemory(u64),
    /// The decoder was used again after it reported an error.
    #[error("XPress9 decoder is in a failed state after a previous error")]
    DecoderFailed,
    /// An error inside one chunk of a `DataModel` stream (adds location context).
    #[error("DataModel chunk {chunk} at input offset {offset}: {source}")]
    InChunk {
        /// Zero-based chunk index within the stream.
        chunk: u64,
        /// Byte offset of the chunk header within the stream.
        offset: u64,
        /// The underlying error.
        source: Box<Xpress9Error>,
    },
    /// Reading the underlying source failed.
    #[error("I/O error while reading the DataModel stream: {0}")]
    Io(#[from] std::io::Error),
}

impl Xpress9Error {
    /// Returns the innermost error, stripping [`Xpress9Error::InChunk`] context wrappers.
    #[must_use]
    pub fn innermost(&self) -> &Xpress9Error {
        let mut err = self;
        while let Xpress9Error::InChunk { source, .. } = err {
            err = source;
        }
        err
    }
}
