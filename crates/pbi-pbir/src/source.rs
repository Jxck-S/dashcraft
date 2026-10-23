//! Container-agnostic file access for the PBIR reader.

use std::collections::BTreeMap;

/// Read-only access to the files of one report folder.
///
/// Paths are `/`-separated and relative to the report root: the folder
/// that contains `definition/` (a PBIP `*.Report` folder, or `Report/`
/// inside a PBIX). `pbi-core` implements this for directories and ZIPs
/// (with its size caps and path guards); [`MemoryFileSource`] and
/// [`SubdirFileSource`] cover tests and nesting.
pub trait FileSource {
    /// Contents of the file at `path`, or `None` if it does not exist.
    fn read(&self, path: &str) -> Option<Vec<u8>>;

    /// Every file path under `prefix` (recursive). An empty prefix lists all
    /// files. Order does not matter; the reader sorts.
    fn list(&self, prefix: &str) -> Vec<String>;
}

impl<T: FileSource + ?Sized> FileSource for &T {
    fn read(&self, path: &str) -> Option<Vec<u8>> {
        (**self).read(path)
    }

    fn list(&self, prefix: &str) -> Vec<String> {
        (**self).list(prefix)
    }
}

/// In-memory [`FileSource`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MemoryFileSource {
    pub files: BTreeMap<String, Vec<u8>>,
}

impl MemoryFileSource {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, path: impl Into<String>, bytes: impl Into<Vec<u8>>) {
        self.files.insert(path.into(), bytes.into());
    }
}

impl<P: Into<String>, B: Into<Vec<u8>>> FromIterator<(P, B)> for MemoryFileSource {
    fn from_iter<I: IntoIterator<Item = (P, B)>>(iter: I) -> Self {
        Self {
            files: iter
                .into_iter()
                .map(|(p, b)| (p.into(), b.into()))
                .collect(),
        }
    }
}

impl FileSource for MemoryFileSource {
    fn read(&self, path: &str) -> Option<Vec<u8>> {
        self.files.get(path).cloned()
    }

    fn list(&self, prefix: &str) -> Vec<String> {
        self.files
            .keys()
            .filter(|p| p.starts_with(prefix))
            .cloned()
            .collect()
    }
}

/// Views the files under `prefix` of another source as a root, e.g.
/// `Report/` inside a PBIX.
#[derive(Debug, Clone)]
pub struct SubdirFileSource<S> {
    inner: S,
    prefix: String,
}

impl<S: FileSource> SubdirFileSource<S> {
    /// `prefix` is normalized to end with `/` (empty means the whole source).
    pub fn new(inner: S, prefix: impl Into<String>) -> Self {
        let mut prefix = prefix.into();
        if !prefix.is_empty() && !prefix.ends_with('/') {
            prefix.push('/');
        }
        Self { inner, prefix }
    }
}

impl<S: FileSource> FileSource for SubdirFileSource<S> {
    fn read(&self, path: &str) -> Option<Vec<u8>> {
        self.inner.read(&format!("{}{path}", self.prefix))
    }

    fn list(&self, prefix: &str) -> Vec<String> {
        self.inner
            .list(&format!("{}{prefix}", self.prefix))
            .into_iter()
            .filter_map(|p| p.strip_prefix(&self.prefix).map(str::to_owned))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subdir_source_strips_the_prefix() {
        let mem: MemoryFileSource = [
            ("Report/definition/report.json", "{}"),
            ("Report/LinguisticSchema", "x"),
            ("Version", "1.32"),
        ]
        .into_iter()
        .collect();
        let sub = SubdirFileSource::new(&mem, "Report");
        assert_eq!(sub.list(""), ["LinguisticSchema", "definition/report.json"]);
        assert_eq!(sub.list("definition/"), ["definition/report.json"]);
        assert_eq!(
            sub.read("definition/report.json").as_deref(),
            Some(&b"{}"[..])
        );
        assert_eq!(sub.read("Version"), None);
    }
}
