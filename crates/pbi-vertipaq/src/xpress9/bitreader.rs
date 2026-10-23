//! LSB-first bit reader used by the XPress9 decoder.
//!
//! Bit `k` of the stream is bit `k % 8` of byte `k / 8`; multi-bit fields are read
//! least-significant bit first. Reads past the end of the input yield zero bits so the
//! decoder never touches memory outside the slice; callers detect overruns by comparing
//! [`BitReader::bits_consumed`] with the bit length declared in the block header.

pub(crate) struct BitReader<'a> {
    data: &'a [u8],
    /// Number of bytes loaded into `buf` so far (may exceed `data.len()` by the zero padding).
    pos: usize,
    /// Bit buffer; the next stream bit is bit 0.
    buf: u64,
    /// Number of valid bits in `buf`.
    count: u32,
}

impl<'a> BitReader<'a> {
    pub(crate) fn new(data: &'a [u8]) -> Self {
        Self {
            data,
            pos: 0,
            buf: 0,
            count: 0,
        }
    }

    /// Guarantees that at least `n` (≤ 56) bits are buffered.
    #[inline(always)]
    pub(crate) fn ensure(&mut self, n: u32) {
        if self.count < n {
            self.refill();
        }
    }

    #[inline(always)]
    fn refill(&mut self) {
        let fast = self
            .pos
            .checked_add(8)
            .and_then(|end| self.data.get(self.pos..end))
            .and_then(|bytes| <[u8; 8]>::try_from(bytes).ok());
        match fast {
            Some(bytes) => {
                // Bits above `count` either are zero or already hold these same stream bits,
                // so OR-ing the whole word is idempotent.
                self.buf |= u64::from_le_bytes(bytes) << self.count;
                let added = (63 - self.count) >> 3;
                self.pos += added as usize;
                self.count += added * 8;
            }
            None => self.refill_slow(),
        }
    }

    #[cold]
    fn refill_slow(&mut self) {
        while self.count <= 56 {
            let byte = self.data.get(self.pos).copied().unwrap_or(0);
            self.buf |= u64::from(byte) << self.count;
            self.pos = self.pos.saturating_add(1);
            self.count += 8;
        }
    }

    /// The next 32 buffered bits (call [`BitReader::ensure`] first).
    #[inline(always)]
    pub(crate) fn peek32(&self) -> u32 {
        self.buf as u32
    }

    /// Drops `n` (≤ 32) bits that were previously made available with [`BitReader::ensure`].
    #[inline(always)]
    pub(crate) fn consume(&mut self, n: u32) {
        self.buf = self.buf.checked_shr(n).unwrap_or(0);
        self.count = self.count.saturating_sub(n);
    }

    /// Reads an `n`-bit (≤ 32) little-endian field.
    #[inline(always)]
    pub(crate) fn read(&mut self, n: u32) -> u32 {
        self.ensure(n);
        let value = if n >= 32 {
            self.peek32()
        } else {
            self.peek32() & ((1u32 << n) - 1)
        };
        self.consume(n);
        value
    }

    /// Total number of bits consumed since the start of the slice.
    #[inline(always)]
    pub(crate) fn bits_consumed(&self) -> u64 {
        (self.pos as u64)
            .saturating_mul(8)
            .saturating_sub(u64::from(self.count))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_lsb_first_and_pads_with_zeros() {
        let data = [0b1010_1101u8, 0xFF, 0x01];
        let mut r = BitReader::new(&data);
        assert_eq!(r.read(1), 1);
        assert_eq!(r.read(2), 0b10);
        assert_eq!(r.read(5), 0b10101);
        assert_eq!(r.bits_consumed(), 8);
        assert_eq!(r.read(9), 0x1FF);
        assert_eq!(r.read(32), 0);
        assert_eq!(r.bits_consumed(), 49);
        assert_eq!(r.read(32), 0);
        assert_eq!(r.bits_consumed(), 81);
    }

    #[test]
    fn fast_and_slow_refill_agree() {
        let data: Vec<u8> = (0..64u32).map(|i| (i * 37 + 11) as u8).collect();
        let mut expected = Vec::new();
        for byte in &data {
            for bit in 0..8 {
                expected.push(u32::from((byte >> bit) & 1));
            }
        }
        for width in 1..=27u32 {
            let mut r = BitReader::new(&data);
            let mut at = 0usize;
            while at + width as usize <= expected.len() {
                let mut want = 0u32;
                for k in 0..width as usize {
                    want |= expected[at + k] << k;
                }
                assert_eq!(r.read(width), want, "width {width} at bit {at}");
                at += width as usize;
                assert_eq!(r.bits_consumed(), at as u64);
            }
        }
    }
}
