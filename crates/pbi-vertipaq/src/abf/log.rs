//! The backup log: the UTF-16 `<BackupLog>` document stored as the `LOG` entry, which
//! maps original server file paths to virtual-directory storage names.

use super::error::AbfError;
use super::xml;

const WHAT: &str = "backup log";

/// Summary of the backup log (`BackupLog` document).
///
/// Absolute server paths (`ServerRoot`, `PersistLocationPath`, file `Path`s) are **not**
/// kept: they contain the workstation user profile directory.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[non_exhaustive]
pub struct BackupLogInfo {
    /// `BackupRestoreSyncVersion` (e.g. `11.53`).
    pub sync_version: Option<String>,
    /// `ObjectName`: the backed-up database name (a GUID in Power BI files).
    pub object_name: Option<String>,
    /// `ObjectId`.
    pub object_id: Option<String>,
    /// `IsTabular`.
    pub is_tabular: Option<bool>,
    /// `CompressionFlag`.
    pub compression_flag: bool,
    /// `EncryptionFlag`.
    pub encryption_flag: bool,
    /// `Languages/Language` LCIDs.
    pub languages: Vec<i64>,
    /// `Collations/Collation` names.
    pub collations: Vec<String>,
    /// `FileGroups/FileGroup` entries in document order.
    pub file_groups: Vec<FileGroupInfo>,
}

/// One `FileGroup` of the backup log.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[non_exhaustive]
pub struct FileGroupInfo {
    /// `Class` (e.g. 100002 = database definition, 100069 = tabular model files).
    pub class: Option<i64>,
    /// `ID`.
    pub id: Option<String>,
    /// `Name`.
    pub name: Option<String>,
    /// `ObjectVersion`.
    pub object_version: Option<i64>,
    /// `PersistLocation`.
    pub persist_location: Option<i64>,
    /// Last component of `PersistLocationPath` (e.g. `<database>.1.db`).
    pub persist_location_name: Option<String>,
    /// Number of `FileList/BackupFile` entries.
    pub file_count: usize,
}

/// One `FileList/BackupFile` entry, with its path already made relative.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LogFile {
    pub group: usize,
    pub logical_path: String,
    pub storage_path: String,
    pub size: Option<u64>,
    pub last_write_time: Option<i64>,
}

/// Parses the backup log. `max_files` bounds the number of file entries.
pub(crate) fn parse_log(
    bytes: &[u8],
    max_files: usize,
) -> Result<(BackupLogInfo, Vec<LogFile>), AbfError> {
    let text = xml::decode_document(bytes, WHAT)?;
    let doc = xml::parse(&text, WHAT)?;
    let root = xml::expect_root(&doc, "BackupLog", WHAT)?;

    let mut info = BackupLogInfo {
        sync_version: xml::opt_text(root, "BackupRestoreSyncVersion"),
        object_name: xml::opt_text(root, "ObjectName"),
        object_id: xml::opt_text(root, "ObjectId"),
        is_tabular: xml::opt_bool(root, "IsTabular", WHAT)?,
        compression_flag: xml::opt_bool(root, "CompressionFlag", WHAT)?.unwrap_or(false),
        encryption_flag: xml::opt_bool(root, "EncryptionFlag", WHAT)?.unwrap_or(false),
        ..BackupLogInfo::default()
    };
    if let Some(languages) = xml::child(root, "Languages") {
        for node in xml::children(languages, "Language") {
            if let Some(text) = node.text()
                && let Ok(lcid) = text.trim().parse()
            {
                info.languages.push(lcid);
            }
        }
    }
    if let Some(collations) = xml::child(root, "Collations") {
        for node in xml::children(collations, "Collation") {
            if let Some(text) = node.text().map(str::trim).filter(|t| !t.is_empty()) {
                info.collations.push(text.to_owned());
            }
        }
    }

    let mut roots: Vec<String> = Vec::new();
    if let Some(server_root) = xml::opt_text(root, "ServerRoot") {
        roots.push(server_root);
    }
    let groups: Vec<_> = xml::child(root, "FileGroups")
        .map(|g| xml::children(g, "FileGroup").collect())
        .unwrap_or_default();
    for group in &groups {
        for name in ["PersistLocationPath", "StorageLocationPath"] {
            if let Some(path) = xml::opt_text(*group, name) {
                roots.push(path);
            }
        }
    }
    // Longest root first, so a database folder wins over the server root.
    roots.sort_by_key(|r| std::cmp::Reverse(r.len()));

    let mut files = Vec::new();
    for (index, group) in groups.iter().enumerate() {
        let file_nodes: Vec<_> = xml::child(*group, "FileList")
            .map(|list| xml::children(list, "BackupFile").collect())
            .unwrap_or_default();
        for node in &file_nodes {
            if files.len() >= max_files {
                return Err(AbfError::LimitExceeded {
                    what: "backup log files",
                    value: files.len() as u64 + 1,
                    limit: max_files as u64,
                });
            }
            let Some(storage_path) = xml::opt_text(*node, "StoragePath") else {
                tracing::warn!("backup log entry without StoragePath skipped");
                continue;
            };
            let original = xml::opt_text(*node, "Path").unwrap_or_default();
            files.push(LogFile {
                group: index,
                logical_path: logical_path(&original, &roots),
                storage_path,
                size: xml::opt_u64(*node, "Size", WHAT)?,
                last_write_time: xml::opt_i64(*node, "LastWriteTime", WHAT)?,
            });
        }
        info.file_groups.push(FileGroupInfo {
            class: xml::opt_i64(*group, "Class", WHAT)?,
            id: xml::opt_text(*group, "ID"),
            name: xml::opt_text(*group, "Name"),
            object_version: xml::opt_i64(*group, "ObjectVersion", WHAT)?,
            persist_location: xml::opt_i64(*group, "PersistLocation", WHAT)?,
            persist_location_name: xml::opt_text(*group, "PersistLocationPath")
                .and_then(|p| last_component(&p).map(str::to_owned)),
            file_count: file_nodes.len(),
        });
    }
    Ok((info, files))
}

fn last_component(path: &str) -> Option<&str> {
    path.rsplit(['\\', '/']).find(|s| !s.is_empty())
}

/// `path` with `root` (case-insensitive) and the following separator removed.
fn strip_root<'a>(path: &'a str, root: &str) -> Option<&'a str> {
    let root = root.trim_end_matches(['\\', '/']);
    if root.is_empty() {
        return None;
    }
    let head = path.get(..root.len())?;
    if !head.eq_ignore_ascii_case(root) {
        return None;
    }
    path.get(root.len()..)?.strip_prefix(['\\', '/'])
}

/// `/`-separated relative path without empty, `.` or `..` segments.
pub(crate) fn sanitize(path: &str) -> String {
    path.split(['\\', '/'])
        .filter(|s| !s.is_empty() && *s != "." && *s != "..")
        .collect::<Vec<_>>()
        .join("/")
}

fn is_absolute(path: &str) -> bool {
    path.starts_with(['\\', '/']) || path.as_bytes().get(1) == Some(&b':')
}

/// Maps an original server path to a model-relative logical path: relative to the
/// longest matching persist location / server root, else (for other absolute paths) the
/// file name only, so no workstation path leaks into logical names.
pub(crate) fn logical_path(original: &str, roots: &[String]) -> String {
    for root in roots {
        if let Some(rest) = strip_root(original, root) {
            return sanitize(rest);
        }
    }
    if is_absolute(original) {
        last_component(original).map(sanitize).unwrap_or_default()
    } else {
        sanitize(original)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn logical_paths_are_relative_and_sanitized() {
        let roots = vec![
            r"\\?\C:\Users\someone\Data\db.1.db".to_owned(),
            r"\\?\C:\Users\someone\Data".to_owned(),
        ];
        assert_eq!(
            logical_path(
                r"\\?\C:\Users\someone\Data\db.1.db\metadata.sqlitedb",
                &roots
            ),
            "metadata.sqlitedb"
        );
        assert_eq!(
            logical_path(
                r"\\?\c:\users\SOMEONE\data\DB.1.db\T (1).tbl\2.prt\x.idf",
                &roots
            ),
            "T (1).tbl/2.prt/x.idf"
        );
        // A sibling of the database folder resolves against the server root.
        assert_eq!(
            logical_path(r"\\?\C:\Users\someone\Data\db.1.db.xml", &roots),
            "db.1.db.xml"
        );
        // Unknown absolute roots keep only the file name.
        assert_eq!(logical_path(r"D:\elsewhere\secret\f.bin", &roots), "f.bin");
        assert_eq!(logical_path(r"rel\..\.\a\\b", &roots), "rel/a/b");
        assert_eq!(logical_path("", &roots), "");
        // Multi-byte characters around the root boundary must not panic.
        assert_eq!(logical_path("ééé", &["é".to_owned()]), "ééé");
    }
}
