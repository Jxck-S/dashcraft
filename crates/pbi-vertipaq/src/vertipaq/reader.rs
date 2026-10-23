//! Bounds-checked little-endian cursor over an untrusted byte slice.

use super::error::VertipaqError;

#[derive(Debug, Clone)]
pub(crate) struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
    what: &'static str,
}

impl<'a> Reader<'a> {
    pub(crate) fn new(data: &'a [u8], what: &'static str) -> Self {
        Self { data, pos: 0, what }
    }

    pub(crate) fn remaining(&self) -> usize {
        self.data.len().saturating_sub(self.pos)
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.remaining() == 0
    }

    pub(crate) fn rest(&self) -> &'a [u8] {
        self.data.get(self.pos..).unwrap_or_default()
    }

    fn truncated(&self, needed: u64) -> VertipaqError {
        VertipaqError::Truncated {
            what: self.what,
            offset: self.pos as u64,
            needed,
            available: self.remaining() as u64,
        }
    }

    pub(crate) fn bytes(&mut self, n: usize) -> Result<&'a [u8], VertipaqError> {
        let end = self
            .pos
            .checked_add(n)
            .ok_or_else(|| self.truncated(n as u64))?;
        let slice = self
            .data
            .get(self.pos..end)
            .ok_or_else(|| self.truncated(n as u64))?;
        self.pos = end;
        Ok(slice)
    }

    /// `count` elements of `size` bytes each, with the product checked.
    pub(crate) fn array(&mut self, count: u64, size: usize) -> Result<&'a [u8], VertipaqError> {
        let total = usize::try_from(count)
            .ok()
            .and_then(|c| c.checked_mul(size))
            .ok_or_else(|| self.truncated(count.saturating_mul(size as u64)))?;
        self.bytes(total)
    }

    fn fixed<const N: usize>(&mut self) -> Result<[u8; N], VertipaqError> {
        let slice = self.bytes(N)?;
        let mut out = [0u8; N];
        out.copy_from_slice(slice);
        Ok(out)
    }

    pub(crate) fn u8(&mut self) -> Result<u8, VertipaqError> {
        Ok(u8::from_le_bytes(self.fixed()?))
    }

    pub(crate) fn i8(&mut self) -> Result<i8, VertipaqError> {
        Ok(i8::from_le_bytes(self.fixed()?))
    }

    pub(crate) fn u32(&mut self) -> Result<u32, VertipaqError> {
        Ok(u32::from_le_bytes(self.fixed()?))
    }

    pub(crate) fn i32(&mut self) -> Result<i32, VertipaqError> {
        Ok(i32::from_le_bytes(self.fixed()?))
    }

    pub(crate) fn u64(&mut self) -> Result<u64, VertipaqError> {
        Ok(u64::from_le_bytes(self.fixed()?))
    }

    pub(crate) fn i64(&mut self) -> Result<i64, VertipaqError> {
        Ok(i64::from_le_bytes(self.fixed()?))
    }

    /// Consumes `tag` or fails with a [`VertipaqError::Corrupt`] naming it.
    pub(crate) fn expect_tag(&mut self, tag: &[u8], name: &str) -> Result<(), VertipaqError> {
        let at = self.pos;
        let found = self.bytes(tag.len())?;
        if found != tag {
            return Err(VertipaqError::corrupt(
                self.what,
                format!("expected {name} marker at offset {at}"),
            ));
        }
        Ok(())
    }

    /// `true` if the unread bytes start with `tag`.
    pub(crate) fn peek_tag(&self, tag: &[u8]) -> bool {
        self.rest().starts_with(tag)
    }
}

/// Little-endian `u32` at `index` of a slice of 4-byte elements.
pub(crate) fn le_u32_at(bytes: &[u8], index: usize) -> Option<u32> {
    let start = index.checked_mul(4)?;
    let chunk = bytes.get(start..start.checked_add(4)?)?;
    let mut word = [0u8; 4];
    word.copy_from_slice(chunk);
    Some(u32::from_le_bytes(word))
}

/// Little-endian `u64` at `index` of a slice of 8-byte elements.
pub(crate) fn le_u64_at(bytes: &[u8], index: usize) -> Option<u64> {
    let start = index.checked_mul(8)?;
    let chunk = bytes.get(start..start.checked_add(8)?)?;
    let mut word = [0u8; 8];
    word.copy_from_slice(chunk);
    Some(u64::from_le_bytes(word))
}
