//! Entry-path normalization and validation shared by all containers.
//!
//! Canonical form: relative, `/`-separated, no empty / `.` / `..`
//! components, no leading `./` and no trailing `/`.

use super::{ContainerError, LimitKind, Limits, UnsafePathReason};

/// A validated entry name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct NormalizedName {
    pub path: String,
    /// The raw name ended with `/` (or `\`).
    pub dir_hint: bool,
}

/// Validates and normalizes an entry name read from an untrusted container.
///
/// Backslashes are converted to `/` *before* the checks, so `..\evil` is
/// caught as traversal. Returns `Ok(None)` for names that denote the root
/// itself (`./`, `.`).
pub(crate) fn normalize_entry_name(
    raw: &str,
    limits: &Limits,
) -> Result<Option<NormalizedName>, ContainerError> {
    if raw.contains('\0') {
        return Err(ContainerError::unsafe_path(
            raw.replace('\0', "\\0"),
            UnsafePathReason::Nul,
        ));
    }
    let unified = raw.replace('\\', "/");
    if unified.starts_with('/') {
        return Err(ContainerError::unsafe_path(raw, UnsafePathReason::Absolute));
    }
    let dir_hint = unified.ends_with('/');
    let mut parts: Vec<&str> = Vec::new();
    for part in unified.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                return Err(ContainerError::unsafe_path(
                    raw,
                    UnsafePathReason::ParentTraversal,
                ));
            }
            // The first *kept* component: `./C:/x` normalizes to `C:/x`.
            other if parts.is_empty() && has_drive_prefix(other) => {
                return Err(ContainerError::unsafe_path(
                    raw,
                    UnsafePathReason::DriveLetter,
                ));
            }
            other => parts.push(other),
        }
    }
    if parts.is_empty() {
        return Ok(None);
    }
    if parts.len() > limits.max_depth {
        return Err(ContainerError::limit(
            LimitKind::Depth,
            format!(
                "{raw:?} has {} components (max {})",
                parts.len(),
                limits.max_depth
            ),
        ));
    }
    let path = parts.join("/");
    if path.len() > limits.max_path_len {
        return Err(ContainerError::limit(
            LimitKind::PathLength,
            format!(
                "entry path is {} bytes (max {})",
                path.len(),
                limits.max_path_len
            ),
        ));
    }
    Ok(Some(NormalizedName { path, dir_hint }))
}

/// Normalizes a path supplied by a *caller* for lookup (`read`, `exists`).
///
/// Lenient about separators, `./` and leading `/`, but never resolves `..`:
/// such lookups simply match nothing. `Some("")` denotes the root.
pub(crate) fn normalize_lookup(path: &str) -> Option<String> {
    if path.contains('\0') {
        return None;
    }
    let unified = path.replace('\\', "/");
    let mut parts: Vec<&str> = Vec::new();
    for part in unified.split('/') {
        match part {
            "" | "." => {}
            ".." => return None,
            other => parts.push(other),
        }
    }
    Some(parts.join("/"))
}

/// Joins a normalized prefix and a normalized relative path.
pub(crate) fn join(prefix: &str, rel: &str) -> String {
    match (prefix.is_empty(), rel.is_empty()) {
        (true, _) => rel.to_owned(),
        (false, true) => prefix.to_owned(),
        (false, false) => format!("{prefix}/{rel}"),
    }
}

/// `true` if `path` lies strictly under the directory `dir` (both
/// normalized; `""` is the root).
pub(crate) fn is_under(path: &str, dir: &str) -> bool {
    if dir.is_empty() {
        return !path.is_empty();
    }
    path.len() > dir.len() && path.starts_with(dir) && path.as_bytes()[dir.len()] == b'/'
}

fn has_drive_prefix(first: &str) -> bool {
    let b = first.as_bytes();
    b.len() >= 2 && b[0].is_ascii_alphabetic() && b[1] == b':'
}

#[cfg(test)]
mod tests {
    use super::*;

    fn norm(raw: &str) -> Result<Option<String>, ContainerError> {
        normalize_entry_name(raw, &Limits::default()).map(|o| o.map(|n| n.path))
    }

    fn reason(raw: &str) -> UnsafePathReason {
        match norm(raw) {
            Err(ContainerError::UnsafePath { reason, .. }) => reason,
            other => panic!("expected UnsafePath for {raw:?}, got {other:?}"),
        }
    }

    #[test]
    fn normalizes_benign_names() {
        assert_eq!(norm("a/b.txt").unwrap().as_deref(), Some("a/b.txt"));
        assert_eq!(norm("./a//b/").unwrap().as_deref(), Some("a/b"));
        assert_eq!(norm("a\\b").unwrap().as_deref(), Some("a/b"));
        assert_eq!(norm("./").unwrap(), None);
        assert_eq!(
            norm("[Content_Types].xml").unwrap().as_deref(),
            Some("[Content_Types].xml")
        );
        assert_eq!(norm("a/C:b").unwrap().as_deref(), Some("a/C:b"));
    }

    #[test]
    fn rejects_hostile_names() {
        assert_eq!(reason("../evil"), UnsafePathReason::ParentTraversal);
        assert_eq!(reason("a/../../b"), UnsafePathReason::ParentTraversal);
        assert_eq!(reason("a\\..\\..\\b"), UnsafePathReason::ParentTraversal);
        assert_eq!(reason("/abs"), UnsafePathReason::Absolute);
        assert_eq!(reason("\\\\server\\share"), UnsafePathReason::Absolute);
        assert_eq!(reason("C:\\x"), UnsafePathReason::DriveLetter);
        assert_eq!(reason("c:x"), UnsafePathReason::DriveLetter);
        // Found by `cargo fuzz` (zip_container): a leading `./` or `//`
        // hid the drive letter from the check.
        assert_eq!(reason("./C:/x"), UnsafePathReason::DriveLetter);
        assert_eq!(reason(".\\\\c:x"), UnsafePathReason::DriveLetter);
        assert_eq!(reason("a\0b"), UnsafePathReason::Nul);
    }

    #[test]
    fn lookup_is_lenient_but_never_traverses() {
        assert_eq!(
            normalize_lookup("./Report\\Layout").as_deref(),
            Some("Report/Layout")
        );
        assert_eq!(normalize_lookup("/x/").as_deref(), Some("x"));
        assert_eq!(normalize_lookup("a/../b"), None);
        assert!(is_under("a/b", "a"));
        assert!(!is_under("ab", "a"));
        assert!(!is_under("a", "a"));
        assert!(is_under("a", ""));
    }
}
