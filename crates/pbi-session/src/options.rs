//! Options of [`Session::open`](crate::Session::open),
//! [`Session::refresh`](crate::Session::refresh) and
//! [`Session::load_csv`](crate::Session::load_csv).

use std::fmt;
use std::path::PathBuf;
use std::sync::Arc;

use pbi_m::{BlobResolver, Clock};
use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;

use crate::report::RefreshEvent;

/// Resource limits of a session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SessionLimits {
    /// Limits of each M partition evaluation (rows, steps, depth, blob size).
    pub m: pbi_m::Limits,
    /// Limits of the VertiPaq decoder (PBIX / ABF data).
    pub decode: pbi_vertipaq::DecodeLimits,
    /// Largest CSV accepted by [`Session::load_csv`](crate::Session::load_csv).
    pub max_csv_bytes: u64,
    /// Largest number of M partitions evaluated concurrently.
    pub max_parallel_queries: usize,
}

impl Default for SessionLimits {
    fn default() -> Self {
        Self {
            m: pbi_m::Limits::default(),
            decode: pbi_vertipaq::DecodeLimits::default(),
            max_csv_bytes: 1024 * 1024 * 1024,
            max_parallel_queries: std::thread::available_parallelism()
                .map_or(2, std::num::NonZeroUsize::get)
                .clamp(1, 8),
        }
    }
}

/// Options of [`Session::open`](crate::Session::open).
#[derive(Debug, Clone, Default)]
pub struct SessionOptions {
    /// Reader options (container limits, PBIX schema loading).
    pub open: pbi_core::OpenOptions,
    /// Offline `AzureStorage.Blobs` mapping (`blob-map.json`). Without it
    /// (and without [`resolver`](Self::resolver)) every blob access fails
    /// with an "offline" error and the table offers "Load CSV manually…".
    pub blob_map: Option<PathBuf>,
    /// A custom blob resolver; takes precedence over `blob_map`.
    pub resolver: Option<Arc<dyn BlobResolver>>,
    /// Clock for `DateTimeZone.UtcNow()` & co. (default: the system clock).
    /// Inject a fixed clock for reproducible `Last Refreshed` tables.
    pub clock: Option<Arc<dyn Clock>>,
    /// Resource limits.
    pub limits: SessionLimits,
}

/// Progress callback of a refresh (called from the refreshing task; keep
/// it cheap and non-blocking).
pub type ProgressFn = Arc<dyn Fn(RefreshEvent) + Send + Sync>;

/// Where [`Session::refresh`](crate::Session::refresh) takes the data from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RefreshSource {
    /// The local data cache when the document has one (a PBIP's
    /// `<Model>.SemanticModel/.pbi/cache.abf`, see
    /// [`Session::cache_info`](crate::Session::cache_info)): no M
    /// evaluation, no blob access. Otherwise like [`RefreshSource::Query`].
    Auto,
    /// Evaluate the M partitions (PBIP, PBIT, `.bim`, TMDL). PBIX and ABF
    /// files load their embedded data. The default.
    #[default]
    Query,
    /// The local data cache ([`Session::load_cached_data`](crate::Session::load_cached_data));
    /// fails with [`SessionError::NoCache`](crate::SessionError::NoCache)
    /// when there is none. PBIX and ABF files load their embedded data
    /// (which is their cache).
    Cache,
}

/// Options of [`Session::refresh`](crate::Session::refresh).
#[derive(Clone, Default)]
pub struct RefreshOptions {
    /// Tables to refresh (case-insensitive names). `None` refreshes every
    /// table. Calculated columns and tables that depend on a refreshed
    /// table are recomputed as well.
    pub tables: Option<Vec<String>>,
    /// Cancels the refresh; the session then keeps its previous data.
    pub cancel: CancellationToken,
    /// Receives [`RefreshEvent`]s.
    pub progress: Option<ProgressFn>,
    /// Where the data comes from (default [`RefreshSource::Query`]).
    pub source: RefreshSource,
}

impl fmt::Debug for RefreshOptions {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RefreshOptions")
            .field("tables", &self.tables)
            .field("cancel", &self.cancel.is_cancelled())
            .field("progress", &self.progress.is_some())
            .field("source", &self.source)
            .finish()
    }
}

impl RefreshOptions {
    /// Refreshes every table.
    #[must_use]
    pub fn all() -> Self {
        Self::default()
    }

    /// Refreshes the named tables (and what depends on them).
    #[must_use]
    pub fn tables<I, S>(tables: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self {
            tables: Some(tables.into_iter().map(Into::into).collect()),
            ..Self::default()
        }
    }

    /// Sets the cancellation token.
    #[must_use]
    pub fn with_cancel(mut self, cancel: CancellationToken) -> Self {
        self.cancel = cancel;
        self
    }

    /// Sets where the data comes from.
    #[must_use]
    pub fn with_source(mut self, source: RefreshSource) -> Self {
        self.source = source;
        self
    }

    /// Sets the progress callback.
    #[must_use]
    pub fn with_progress(
        mut self,
        progress: impl Fn(RefreshEvent) + Send + Sync + 'static,
    ) -> Self {
        self.progress = Some(Arc::new(progress));
        self
    }
}

/// Options of [`Session::load_csv`](crate::Session::load_csv).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct CsvOptions {
    /// Field delimiter (one ASCII character), default `,`.
    pub delimiter: char,
    /// The first record holds column names (matched case-insensitively to
    /// the model's source column or column names). Without a header,
    /// fields map to the table's data columns by position. Default `true`.
    pub has_header: bool,
    /// Text encoding: a WHATWG label (`utf-8`, `windows-1252`, `utf-16le`,
    /// …) or a Windows code page number as used by M (`65001`, `1252`,
    /// `1200`). `None` detects a BOM, else UTF-8, else Windows-1252.
    pub encoding: Option<String>,
}

impl Default for CsvOptions {
    fn default() -> Self {
        Self {
            delimiter: ',',
            has_header: true,
            encoding: None,
        }
    }
}

/// CSV input of [`Session::load_csv`](crate::Session::load_csv).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CsvSource {
    /// A file on disk.
    Path(PathBuf),
    /// CSV bytes (e.g. from a file picker or the clipboard).
    Bytes(Vec<u8>),
}

impl From<PathBuf> for CsvSource {
    fn from(p: PathBuf) -> Self {
        CsvSource::Path(p)
    }
}

impl From<&std::path::Path> for CsvSource {
    fn from(p: &std::path::Path) -> Self {
        CsvSource::Path(p.to_path_buf())
    }
}

impl From<Vec<u8>> for CsvSource {
    fn from(b: Vec<u8>) -> Self {
        CsvSource::Bytes(b)
    }
}

impl From<&[u8]> for CsvSource {
    fn from(b: &[u8]) -> Self {
        CsvSource::Bytes(b.to_vec())
    }
}

impl From<&str> for CsvSource {
    /// CSV text (not a path; use [`CsvSource::Path`] for files).
    fn from(text: &str) -> Self {
        CsvSource::Bytes(text.as_bytes().to_vec())
    }
}
