//! Resource caps applied when opening and reading untrusted containers.

const KIB: u64 = 1024;
const MIB: u64 = 1024 * KIB;
const GIB: u64 = 1024 * MIB;

/// Caps on what a [`Container`](super::Container) accepts.
///
/// ZIP containers enforce every cap on the *declared* sizes when the archive
/// is opened and again on the *actual* bytes produced by each read.
/// Directory containers enforce `max_entries`, `max_path_len` and `max_depth`
/// while walking, and `max_entry_size` on each read (files on disk are not
/// compressed, so the ratio and total caps do not apply to them).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// Maximum number of entries (files and directories).
    pub max_entries: usize,
    /// Maximum uncompressed size of a single entry, in bytes.
    pub max_entry_size: u64,
    /// Maximum sum of the uncompressed sizes of all entries, in bytes.
    pub max_total_size: u64,
    /// Maximum uncompressed:compressed ratio (`N` means `N:1`) for entries
    /// larger than [`ratio_check_min_size`](Self::ratio_check_min_size). Also
    /// applied archive-wide (total uncompressed size vs. archive size).
    pub max_compression_ratio: u64,
    /// Entries at or below this uncompressed size are exempt from the ratio
    /// check (small, repetitive files legitimately compress very well).
    pub ratio_check_min_size: u64,
    /// Maximum length in bytes of a normalized entry path.
    pub max_path_len: usize,
    /// Maximum number of path components of an entry.
    pub max_depth: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_entries: 100_000,
            max_entry_size: 2 * GIB,
            max_total_size: 8 * GIB,
            max_compression_ratio: 1000,
            ratio_check_min_size: MIB,
            max_path_len: 1024,
            max_depth: 64,
        }
    }
}

impl Limits {
    /// Small caps for tests and fuzzing: 1 000 entries, 16 MiB per entry,
    /// 64 MiB in total, 100:1 ratio above 64 KiB, 256-byte paths, depth 16.
    #[must_use]
    pub fn strict() -> Self {
        Self {
            max_entries: 1_000,
            max_entry_size: 16 * MIB,
            max_total_size: 64 * MIB,
            max_compression_ratio: 100,
            ratio_check_min_size: 64 * KIB,
            max_path_len: 256,
            max_depth: 16,
        }
    }

    /// `true` if `uncompressed` bytes produced from `compressed` bytes exceed
    /// the ratio cap.
    #[must_use]
    pub fn ratio_exceeded(&self, uncompressed: u64, compressed: u64) -> bool {
        uncompressed > self.ratio_check_min_size
            && uncompressed > compressed.saturating_mul(self.max_compression_ratio)
    }
}
