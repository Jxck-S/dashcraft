//! Shared helpers for the integration tests.
#![allow(dead_code)]

use std::path::{Path, PathBuf};

use pbi_pbir::{FileSource, MemoryFileSource};

/// `$PBIVIEW_REFERENCE`, else `<workspace>/reference`.
pub fn reference_root() -> PathBuf {
    std::env::var_os("PBIVIEW_REFERENCE").map_or_else(
        || Path::new(env!("CARGO_MANIFEST_DIR")).join("../../reference"),
        PathBuf::from,
    )
}

/// The Acme Outfitters example's report folders (under
/// `reference/examples/acme-outfitters`), as `(label, folder)`. Missing
/// folders are reported and skipped.
pub fn report_folders() -> Vec<(&'static str, PathBuf)> {
    let example = reference_root().join("examples/acme-outfitters");
    [
        ("overview", "Acme Outfitters - Overview.Report"),
        ("stores", "Acme Outfitters - Stores.Report"),
        ("online", "Acme Outfitters - Online.Report"),
    ]
    .into_iter()
    .filter_map(|(label, rel)| {
        let path = example.join(rel);
        if path.is_dir() {
            Some((label, path))
        } else {
            println!("SKIP: {} missing", path.display());
            None
        }
    })
    .collect()
}

/// One example report folder, or `None` (with a SKIP message) if missing.
pub fn report_folder(label: &str) -> Option<PathBuf> {
    report_folders()
        .into_iter()
        .find(|(l, _)| *l == label)
        .map(|(_, p)| p)
}

/// Every file of `folder` in memory, to edit before reading.
pub fn memory_copy(folder: &Path) -> MemoryFileSource {
    let dir = DirSource(folder.to_path_buf());
    dir.list("")
        .into_iter()
        .map(|p| {
            let bytes = dir.read(&p).unwrap_or_default();
            (p, bytes)
        })
        .collect()
}

/// A plain directory [`FileSource`] for tests (`pbi-core` owns the real,
/// guarded one).
pub struct DirSource(pub PathBuf);

impl FileSource for DirSource {
    fn read(&self, path: &str) -> Option<Vec<u8>> {
        std::fs::read(self.0.join(path)).ok()
    }

    fn list(&self, prefix: &str) -> Vec<String> {
        walkdir::WalkDir::new(&self.0)
            .into_iter()
            .filter_map(Result::ok)
            .filter(|e| e.file_type().is_file())
            .filter_map(|e| {
                let rel = e.path().strip_prefix(&self.0).ok()?;
                let parts: Vec<String> = rel
                    .components()
                    .map(|c| c.as_os_str().to_string_lossy().into_owned())
                    .collect();
                Some(parts.join("/"))
            })
            .filter(|p| p.starts_with(prefix))
            .collect()
    }
}
