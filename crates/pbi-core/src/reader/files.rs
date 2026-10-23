//! Small container helpers shared by the readers.

use std::cell::RefCell;
use std::sync::Arc;

use pbi_pbir::FileSource;

use crate::container::{
    Container, ContainerError, ContainerKind, EntryInfo, TextEncoding, decode_text,
};

/// Largest JSON metadata part parsed into [`MetadataPart::json`](super::MetadataPart::json).
pub(crate) const METADATA_JSON_MAX: u64 = 16 * 1024 * 1024;

/// A container holding one file, used for single-file inputs (`.bim`,
/// `.abf`, `.pbids`, …). The bytes are shared, so large backups are not
/// copied when handed to the ABF reader.
#[derive(Clone)]
pub(crate) struct BytesContainer {
    label: String,
    entries: Vec<EntryInfo>,
    data: Arc<[u8]>,
}

impl std::fmt::Debug for BytesContainer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BytesContainer")
            .field("label", &self.label)
            .field("entry", &self.entry_name())
            .field("size", &self.data.len())
            .finish()
    }
}

impl BytesContainer {
    /// `name` becomes the single entry's path when it is a safe file name
    /// (one component, no separators or NUL), `input` otherwise.
    pub(crate) fn new(label: impl Into<String>, name: &str, data: Arc<[u8]>) -> Self {
        let safe = !name.is_empty()
            && name.len() <= 255
            && name != "."
            && name != ".."
            && !name.contains(['/', '\\', '\0', ':']);
        let path = if safe { name } else { "input" };
        Self {
            label: label.into(),
            entries: vec![EntryInfo {
                path: path.to_owned(),
                size: data.len() as u64,
                compressed_size: None,
                is_dir: false,
            }],
            data,
        }
    }

    pub(crate) fn entry_name(&self) -> &str {
        self.entries.first().map_or("input", |e| e.path.as_str())
    }

    pub(crate) fn data(&self) -> Arc<[u8]> {
        Arc::clone(&self.data)
    }
}

impl Container for BytesContainer {
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
        if path == self.entry_name() {
            Ok(self.data.to_vec())
        } else {
            Err(ContainerError::NotFound(path.to_owned()))
        }
    }

    fn as_dyn(&self) -> &dyn Container {
        self
    }
}

/// [`FileSource`] over a container (normally a [`Container::sub`] view of a
/// report folder). Read failures other than "not found" are recorded so the
/// reader can report them instead of silently treating files as missing.
pub(crate) struct ContainerFileSource<'a> {
    container: &'a dyn Container,
    errors: RefCell<Vec<(String, ContainerError)>>,
}

impl<'a> ContainerFileSource<'a> {
    pub(crate) fn new(container: &'a dyn Container) -> Self {
        Self {
            container,
            errors: RefCell::new(Vec::new()),
        }
    }

    pub(crate) fn take_errors(&self) -> Vec<(String, ContainerError)> {
        std::mem::take(&mut *self.errors.borrow_mut())
    }
}

impl FileSource for ContainerFileSource<'_> {
    fn read(&self, path: &str) -> Option<Vec<u8>> {
        match self.container.read(path) {
            Ok(bytes) => Some(bytes),
            Err(ContainerError::NotFound(_) | ContainerError::IsDirectory(_)) => None,
            Err(e) => {
                self.errors.borrow_mut().push((path.to_owned(), e));
                None
            }
        }
    }

    fn list(&self, prefix: &str) -> Vec<String> {
        self.container
            .entries()
            .iter()
            .filter(|e| !e.is_dir && e.path.starts_with(prefix))
            .map(|e| e.path.clone())
            .collect()
    }
}

/// Reads a JSON part leniently: any encoding, BOM tolerated, at most
/// [`METADATA_JSON_MAX`] bytes.
pub(crate) fn read_json(
    c: &dyn Container,
    path: &str,
) -> Result<(serde_json::Value, TextEncoding), String> {
    read_json_capped(c, path, METADATA_JSON_MAX)
}

/// [`read_json`] with another size cap. Nesting is bounded by `serde_json`'s
/// recursion limit (128).
pub(crate) fn read_json_capped(
    c: &dyn Container,
    path: &str,
    max: u64,
) -> Result<(serde_json::Value, TextEncoding), String> {
    let text = read_text_capped(c, path, max)?;
    serde_json::from_str(text.0.trim_start_matches('\u{feff}'))
        .map(|v| (v, text.1))
        .map_err(|e| format!("not valid JSON: {e}"))
}

/// Reads a text part (any encoding, BOM detected) of at most `max` bytes.
pub(crate) fn read_text_capped(
    c: &dyn Container,
    path: &str,
    max: u64,
) -> Result<(String, TextEncoding), String> {
    let entry = c.entry(path).ok_or_else(|| "missing".to_owned())?;
    if entry.is_dir {
        return Err("is a directory".into());
    }
    if entry.size > max {
        return Err(format!(
            "{} bytes is larger than the {max}-byte limit for this part",
            entry.size
        ));
    }
    let bytes = c.read(path).map_err(|e| e.to_string())?;
    let decoded = decode_text(&bytes);
    Ok((decoded.text, decoded.encoding))
}

/// Path of the root-level file `name` (case-insensitive), if any.
pub(crate) fn root_file(c: &dyn Container, name: &str) -> Option<String> {
    c.entries()
        .iter()
        .find(|e| !e.is_dir && !e.path.contains('/') && e.path.eq_ignore_ascii_case(name))
        .map(|e| e.path.clone())
}

/// Joins a container folder and a relative path (`""` folder = root).
pub(crate) fn join(dir: &str, rel: &str) -> String {
    match (dir.is_empty(), rel.is_empty()) {
        (true, _) => rel.to_owned(),
        (_, true) => dir.to_owned(),
        _ => format!("{dir}/{rel}"),
    }
}

/// Last component of a container path (`""` → `""`).
pub(crate) fn file_name(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

/// `name` without its final extension (`a.b.pbix` → `a.b`).
pub(crate) fn file_stem(name: &str) -> &str {
    match name.rsplit_once('.') {
        Some((stem, _)) if !stem.is_empty() => stem,
        _ => name,
    }
}

/// Folder name without a Fabric item suffix (`X.Report` → `X`).
pub(crate) fn item_name(folder: &str) -> String {
    let name = file_name(folder);
    for suffix in [".Report", ".SemanticModel", ".Dataset"] {
        if let Some(cut) = name.len().checked_sub(suffix.len())
            && cut > 0
            && name
                .get(cut..)
                .is_some_and(|s| s.eq_ignore_ascii_case(suffix))
            && let Some(stem) = name.get(..cut)
        {
            return stem.to_owned();
        }
    }
    name.to_owned()
}
