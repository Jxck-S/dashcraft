//! In-memory container (tests, nested packages, synthetic inputs).

use std::sync::Arc;

use super::path::normalize_entry_name;
use super::{Container, ContainerError, ContainerKind, EntryInfo, LimitKind, Limits};

/// A container over in-memory files. Paths are validated exactly like ZIP
/// entry names, so it is safe to fill from untrusted input.
#[derive(Clone)]
pub struct MemContainer {
    label: String,
    limits: Limits,
    entries: Vec<EntryInfo>,
    data: Vec<Arc<[u8]>>,
}

impl std::fmt::Debug for MemContainer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MemContainer")
            .field("label", &self.label)
            .field("entries", &self.entries.len())
            .finish_non_exhaustive()
    }
}

impl MemContainer {
    /// An empty container with default [`Limits`].
    #[must_use]
    pub fn new(label: impl Into<String>) -> Self {
        Self::with_limits(label, Limits::default())
    }

    /// An empty container with the given limits.
    #[must_use]
    pub fn with_limits(label: impl Into<String>, limits: Limits) -> Self {
        Self {
            label: label.into(),
            limits,
            entries: Vec::new(),
            data: Vec::new(),
        }
    }

    /// Builds a container from `(path, bytes)` pairs.
    ///
    /// # Errors
    /// As [`insert`](Self::insert).
    pub fn from_files<P, B, I>(label: impl Into<String>, files: I) -> Result<Self, ContainerError>
    where
        P: AsRef<str>,
        B: Into<Vec<u8>>,
        I: IntoIterator<Item = (P, B)>,
    {
        let mut c = Self::new(label);
        for (path, bytes) in files {
            c.insert(path.as_ref(), bytes)?;
        }
        Ok(c)
    }

    /// Adds or replaces a file. A trailing `/` adds an empty directory.
    ///
    /// # Errors
    /// [`ContainerError::UnsafePath`] for hostile names and
    /// [`ContainerError::LimitExceeded`] when a cap is exceeded.
    pub fn insert(&mut self, path: &str, bytes: impl Into<Vec<u8>>) -> Result<(), ContainerError> {
        let Some(name) = normalize_entry_name(path, &self.limits)? else {
            return Err(ContainerError::IsDirectory(path.to_owned()));
        };
        let bytes: Vec<u8> = bytes.into();
        let size = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
        if !name.dir_hint && size > self.limits.max_entry_size {
            return Err(ContainerError::limit(
                LimitKind::EntrySize,
                format!("{} is {size} bytes", name.path),
            ));
        }
        let info = EntryInfo {
            path: name.path,
            size: if name.dir_hint { 0 } else { size },
            compressed_size: None,
            is_dir: name.dir_hint,
        };
        match self
            .entries
            .binary_search_by(|e| e.path.as_str().cmp(info.path.as_str()))
        {
            Ok(i) => {
                self.entries[i] = info;
                self.data[i] = Arc::from(bytes);
            }
            Err(i) => {
                if self.entries.len() >= self.limits.max_entries {
                    return Err(ContainerError::limit(
                        LimitKind::Entries,
                        format!("more than {} entries", self.limits.max_entries),
                    ));
                }
                self.entries.insert(i, info);
                self.data.insert(i, Arc::from(bytes));
            }
        }
        Ok(())
    }
}

impl Container for MemContainer {
    fn kind(&self) -> ContainerKind {
        ContainerKind::Memory
    }

    fn root_label(&self) -> &str {
        &self.label
    }

    fn entries(&self) -> &[EntryInfo] {
        &self.entries
    }

    fn read(&self, path: &str) -> Result<Vec<u8>, ContainerError> {
        let entry = self
            .entry(path)
            .ok_or_else(|| ContainerError::NotFound(path.to_owned()))?;
        if entry.is_dir {
            return Err(ContainerError::IsDirectory(path.to_owned()));
        }
        let i = self
            .entries
            .binary_search_by(|e| e.path.as_str().cmp(entry.path.as_str()))
            .map_err(|_| ContainerError::NotFound(path.to_owned()))?;
        self.data
            .get(i)
            .map(|d| d.to_vec())
            .ok_or_else(|| ContainerError::NotFound(path.to_owned()))
    }

    fn as_dyn(&self) -> &dyn Container {
        self
    }
}
