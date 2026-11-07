//! The TMDL folder an open file belongs to, read from disk with a cache.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// Most files read from one folder.
const MAX_FILES: usize = 10_000;
/// Most bytes read from one folder.
const MAX_BYTES: u64 = 256 * 1024 * 1024;
/// Deepest sub-folder searched.
const MAX_DEPTH: usize = 8;
/// Folders above a file searched for the model's `definition` folder.
const MAX_LEVELS: usize = 4;

/// The model folder of a TMDL file: the closest folder above it named `definition` or holding
/// `model.tmdl` or `database.tmdl`.
pub fn tmdl_root(file: &Path) -> Option<PathBuf> {
    file.ancestors().skip(1).take(MAX_LEVELS).find_map(|dir| {
        let named = dir
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.eq_ignore_ascii_case("definition"));
        (named || dir.join("model.tmdl").is_file() || dir.join("database.tmdl").is_file())
            .then(|| dir.to_path_buf())
    })
}

/// `path` relative to `root` with `/` separators (`tables/Orders.tmdl`).
pub fn relative(root: &Path, path: &Path) -> Option<String> {
    let rel = path.strip_prefix(root).ok()?;
    let parts: Option<Vec<&str>> = rel.components().map(|c| c.as_os_str().to_str()).collect();
    let joined = parts?.join("/");
    (!joined.is_empty()).then_some(joined)
}

fn is_tmdl(name: &str) -> bool {
    name.rsplit_once('.')
        .is_some_and(|(_, ext)| ext.eq_ignore_ascii_case("tmdl"))
}

#[derive(Debug)]
struct Cached {
    modified: SystemTime,
    len: u64,
    text: String,
}

/// TMDL files read from disk, kept while their size and modification time stay the same.
#[derive(Debug, Default)]
pub struct DiskFiles {
    cache: HashMap<PathBuf, Cached>,
}

impl DiskFiles {
    /// Every `.tmdl` file under `root` as (relative path, text), sorted by path; at most
    /// 10,000 files and 256 MiB. Unreadable files are left out.
    pub fn folder(&mut self, root: &Path) -> Vec<(String, String)> {
        let mut found = Vec::new();
        walk(root, "", 0, &mut found);
        found.sort();
        let mut out = Vec::with_capacity(found.len());
        let mut total = 0u64;
        for (rel, path) in found {
            let Ok(meta) = fs::metadata(&path) else {
                continue;
            };
            total = total.saturating_add(meta.len());
            if total > MAX_BYTES {
                break;
            }
            let modified = meta.modified().ok();
            let fresh = self
                .cache
                .get(&path)
                .filter(|c| Some(c.modified) == modified && c.len == meta.len());
            let text = match fresh {
                Some(c) => c.text.clone(),
                None => {
                    let Ok(bytes) = fs::read(&path) else {
                        continue;
                    };
                    let text = String::from_utf8_lossy(&bytes);
                    let text = text.strip_prefix('\u{feff}').unwrap_or(&text).to_owned();
                    match modified {
                        Some(modified) => {
                            self.cache.insert(
                                path,
                                Cached {
                                    modified,
                                    len: meta.len(),
                                    text: text.clone(),
                                },
                            );
                        }
                        None => {
                            self.cache.remove(&path);
                        }
                    }
                    text
                }
            };
            out.push((rel, text));
        }
        out
    }
}

fn walk(dir: &Path, rel: &str, depth: usize, out: &mut Vec<(String, PathBuf)>) {
    if depth > MAX_DEPTH || out.len() >= MAX_FILES {
        return;
    }
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        if out.len() >= MAX_FILES {
            return;
        }
        // Not followed: symbolic links.
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        let child = if rel.is_empty() {
            name.to_owned()
        } else {
            format!("{rel}/{name}")
        };
        if kind.is_dir() {
            walk(&entry.path(), &child, depth + 1, out);
        } else if kind.is_file() && is_tmdl(name) {
            out.push((child, entry.path()));
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    #[test]
    fn finds_the_definition_folder_and_reads_it() {
        let scratch = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/tmp");
        fs::create_dir_all(&scratch).unwrap();
        let dir = tempfile::Builder::new()
            .prefix("dashcraft-lsp-files-")
            .tempdir_in(scratch)
            .unwrap();
        let def = dir.path().join("M.SemanticModel").join("definition");
        fs::create_dir_all(def.join("tables")).unwrap();
        fs::write(def.join("model.tmdl"), "model Model\n").unwrap();
        fs::write(def.join("tables").join("A.tmdl"), "\u{feff}table A\n").unwrap();
        fs::write(def.join("tables").join("notes.txt"), "x").unwrap();
        let file = def.join("tables").join("A.tmdl");
        assert_eq!(tmdl_root(&file).unwrap(), def);
        assert_eq!(relative(&def, &file).unwrap(), "tables/A.tmdl");
        assert_eq!(relative(&def, &def), None);
        assert_eq!(tmdl_root(&dir.path().join("loose.tmdl")), None);

        let mut disk = DiskFiles::default();
        let files = disk.folder(&def);
        assert_eq!(
            files,
            vec![
                ("model.tmdl".to_owned(), "model Model\n".to_owned()),
                ("tables/A.tmdl".to_owned(), "table A\n".to_owned()),
            ]
        );
        fs::write(def.join("tables").join("A.tmdl"), "table A2\n").unwrap();
        let again = disk.folder(&def);
        assert_eq!(again.get(1).unwrap().1, "table A2\n");
    }
}
