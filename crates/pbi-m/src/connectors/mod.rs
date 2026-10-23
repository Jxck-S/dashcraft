//! Data-source connectors.
//!
//! M code never touches the file system or the network directly. The only data source is
//! `AzureStorage.Blobs(containerUrl)`, served by a [`BlobResolver`]:
//!
//! * [`MapBlobResolver`] — offline: maps container URLs and blob names to local files listed in
//!   a `blob-map.json`;
//! * [`NoBlobResolver`] — no data source configured (the default);
//! * a future Azure resolver (SAS token or `az` token from the OS keychain) implements the same
//!   trait; [`BlobResolver::open_limited`] lets it stream with a size cap.
//!
//! Other connectors (`File.Contents`, `Web.Contents`, `Sql.Database`, ...) evaluate to
//! [`MError::Unsupported`].

mod azure;
mod map;

use std::fmt;

use chrono::{DateTime, Utc};

pub(crate) use azure::azure_storage_blobs;
pub use map::MapBlobResolver;

use crate::merror::{LimitKind, MError, ResolverError, ResolverErrorKind};

/// A blob in a container listing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlobEntry {
    /// The full blob name, e.g. `sales/orders.csv`.
    pub name: String,
    pub size: Option<u64>,
    pub modified: Option<DateTime<Utc>>,
}

/// Lists and reads blobs of Azure Storage containers.
pub trait BlobResolver: Send + Sync + fmt::Debug {
    /// Lists the blobs of a container (`https://account.blob.core.windows.net/container`).
    fn list(&self, container_url: &str) -> Result<Vec<BlobEntry>, ResolverError>;

    /// Reads a blob's content.
    fn open(&self, container_url: &str, name: &str) -> Result<Vec<u8>, ResolverError>;

    /// Reads a blob's content, failing with [`MError::LimitExceeded`] if it is larger than
    /// `max_bytes`. Implementations should check the size before reading.
    fn open_limited(
        &self,
        container_url: &str,
        name: &str,
        max_bytes: u64,
    ) -> Result<Vec<u8>, MError> {
        let bytes = self.open(container_url, name)?;
        if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > max_bytes {
            return Err(MError::limit(LimitKind::Bytes, max_bytes));
        }
        Ok(bytes)
    }
}

/// The resolver used when no blob mapping is configured: every request fails with a clear
/// "offline" error.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoBlobResolver;

impl NoBlobResolver {
    fn error(container_url: &str) -> ResolverError {
        ResolverError::new(
            ResolverErrorKind::Offline,
            format!(
                "offline: no blob mapping configured for AzureStorage.Blobs(\"{container_url}\"). Provide a blob-map.json (--blob-map) or use \"Load CSV manually…\"."
            ),
        )
    }
}

impl BlobResolver for NoBlobResolver {
    fn list(&self, container_url: &str) -> Result<Vec<BlobEntry>, ResolverError> {
        Err(Self::error(container_url))
    }

    fn open(&self, container_url: &str, _name: &str) -> Result<Vec<u8>, ResolverError> {
        Err(Self::error(container_url))
    }
}

/// Normalizes a container URL for lookups: trims whitespace and trailing slashes and lowercases
/// the scheme and host.
pub fn normalize_container_url(url: &str) -> String {
    let url = url.trim().trim_end_matches('/');
    match url.find("://") {
        Some(i) => {
            let host_end = url[i + 3..].find('/').map_or(url.len(), |j| i + 3 + j);
            let mut s = url[..host_end].to_ascii_lowercase();
            s.push_str(&url[host_end..]);
            s
        }
        None => url.to_owned(),
    }
}
