//! Output files: policy checks and atomic writes.

use std::fs::{self, File};
use std::io::{self, BufWriter, Write};
use std::path::{Path, PathBuf};

use crate::error::OpError;
use crate::path_policy::PathPolicy;

/// Checks an output file before any work: the path policy, not a folder,
/// and no existing non-empty file unless `overwrite`.
///
/// # Errors
/// `pathDenied`, `invalidParams` (a folder), `io` (exists).
pub fn check_output_file(
    policy: &PathPolicy,
    path: &Path,
    overwrite: bool,
) -> Result<PathBuf, OpError> {
    let checked = policy.check_write(path)?;
    match fs::metadata(&checked) {
        Ok(meta) if meta.is_dir() => Err(OpError::invalid_params(format!(
            "{} is a folder; a file path is needed",
            path.display()
        ))),
        Ok(meta) if meta.len() > 0 && !overwrite => Err(OpError::io(format!(
            "{} already exists; pass \"overwrite\": true to replace it",
            path.display()
        ))),
        _ => Ok(checked),
    }
}

/// Writes `path` through a temporary file in the same folder, renamed into
/// place when complete: a failed write leaves nothing behind, and without
/// `overwrite` an existing file is never replaced.
///
/// # Errors
/// `io`, or the error of `write`.
pub fn write_file_atomic<T>(
    path: &Path,
    overwrite: bool,
    write: impl FnOnce(&mut BufWriter<&mut File>) -> Result<T, OpError>,
) -> Result<T, OpError> {
    let dir = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(dir)
        .map_err(|e| OpError::io(format!("cannot create {}: {e}", dir.display())))?;
    let mut builder = tempfile::Builder::new();
    builder.prefix(".dashcraft-").suffix(".tmp");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        builder.permissions(fs::Permissions::from_mode(0o666));
    }
    let mut temp = builder
        .tempfile_in(dir)
        .map_err(|e| OpError::io(format!("cannot create a file in {}: {e}", dir.display())))?;
    let value = {
        let mut w = BufWriter::new(temp.as_file_mut());
        let value = write(&mut w)?;
        w.flush()
            .map_err(|e| OpError::io(format!("cannot write {}: {e}", path.display())))?;
        value
    };
    let persisted = if overwrite {
        temp.persist(path)
    } else {
        temp.persist_noclobber(path)
    };
    persisted.map_err(|e| match e.error.kind() {
        io::ErrorKind::AlreadyExists => OpError::io(format!(
            "{} already exists; pass \"overwrite\": true to replace it",
            path.display()
        )),
        _ => OpError::io(format!("cannot write {}: {}", path.display(), e.error)),
    })?;
    Ok(value)
}

/// Writes `bytes` to `path` atomically.
///
/// # Errors
/// As [`write_file_atomic`].
pub fn write_bytes(path: &Path, overwrite: bool, bytes: &[u8]) -> Result<(), OpError> {
    write_file_atomic(path, overwrite, |w| {
        w.write_all(bytes)
            .map_err(|e| OpError::io(format!("cannot write {}: {e}", path.display())))
    })
}
