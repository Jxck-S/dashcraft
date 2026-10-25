//! `dashcraft open`: launches the desktop app (`dashcraft-app`) on a file
//! or folder.
//!
//! The app is looked up, in order: `$DASHCRAFT_APP`; next to this executable;
//! on macOS in a `DashCraft.app` bundle next to this executable, in
//! `/Applications` and in `~/Applications`; then on `PATH`. The path is passed
//! as a single argument (never through a shell).

use std::env;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context as _, anyhow, bail};

use crate::common::{Context, Status, prog};

/// Environment variable overriding the app executable.
const APP_ENV: &str = "DASHCRAFT_APP";
/// File name (without extension) of the desktop app executable.
const APP_BIN: &str = "dashcraft-app";
/// macOS bundle name.
const BUNDLE: &str = "DashCraft.app";

#[derive(Debug, clap::Args)]
pub struct OpenArgs {
    /// File or folder to open in the desktop app.
    pub path: PathBuf,
}

pub fn run(args: &OpenArgs, ctx: &Context) -> anyhow::Result<Status> {
    if !args.path.exists() {
        bail!(
            "cannot open {}: no such file or directory",
            args.path.display()
        );
    }
    // Absolute, so the app does not depend on its working directory and the
    // argument can never be mistaken for a flag.
    let target = std::path::absolute(&args.path)
        .with_context(|| format!("cannot resolve {}", args.path.display()))?;
    let app = find_app()?;
    tracing::info!(app = %app.display(), path = %target.display(), "launching the desktop app");
    let mut cmd = Command::new(&app);
    cmd.arg(&target).stdin(Stdio::null());
    if ctx.verbose == 0 {
        cmd.stdout(Stdio::null()).stderr(Stdio::null());
    }
    cmd.spawn()
        .with_context(|| format!("cannot launch {}", app.display()))?;
    ctx.note(format!(
        "opening {} in {}",
        args.path.display(),
        app.display()
    ));
    Ok(Status::Ok)
}

/// `$DASHCRAFT_APP`; an empty value is ignored.
fn app_override(var: impl Fn(&str) -> Option<OsString>) -> Option<PathBuf> {
    var(APP_ENV).filter(|v| !v.is_empty()).map(PathBuf::from)
}

fn find_app() -> anyhow::Result<PathBuf> {
    if let Some(app) = app_override(|n| env::var_os(n)) {
        if is_executable(&app) {
            return Ok(app);
        }
        bail!(
            "{APP_ENV} is set to {}, which is not an executable file",
            app.display()
        );
    }
    let this = env::current_exe().ok().and_then(|p| p.canonicalize().ok());
    let found = candidates()
        .into_iter()
        .find(|c| is_executable(c) && (this.is_none() || c.canonicalize().ok() != this));
    found.ok_or_else(|| {
        anyhow!(
            "the DashCraft desktop app ({APP_BIN}) was not found next to this executable{}, \
             or on PATH. Install the desktop app, or set {APP_ENV} to its executable. \
             `{} inspect <path>` works without it.",
            if cfg!(target_os = "macos") {
                ", in DashCraft.app (next to it, /Applications or ~/Applications)"
            } else {
                ""
            },
            prog()
        )
    })
}

fn candidates() -> Vec<PathBuf> {
    let file_name = format!("{APP_BIN}{}", env::consts::EXE_SUFFIX);
    let mut out = Vec::new();
    if let Some(dir) = env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(Path::to_path_buf))
    {
        out.push(dir.join(&file_name));
        if cfg!(target_os = "macos") {
            out.push(bundle_in(&dir));
        }
    }
    if cfg!(target_os = "macos") {
        out.push(bundle_in(Path::new("/Applications")));
        if let Some(home) = env::var_os("HOME") {
            out.push(bundle_in(&PathBuf::from(home).join("Applications")));
        }
    }
    if let Some(path) = env::var_os("PATH") {
        out.extend(env::split_paths(&path).map(|dir| dir.join(&file_name)));
    }
    out
}

/// The executable inside a `DashCraft.app` bundle in `dir`.
fn bundle_in(dir: &Path) -> PathBuf {
    dir.join(BUNDLE)
        .join("Contents")
        .join("MacOS")
        .join(APP_BIN)
}

#[cfg(unix)]
fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    path.metadata()
        .is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

#[cfg(not(unix))]
fn is_executable(path: &Path) -> bool {
    path.is_file()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dashcraft_app_overrides_the_lookup_unless_empty() {
        let set = |n: &str| (n == "DASHCRAFT_APP").then(|| OsString::from("/opt/dashcraft-app"));
        assert_eq!(app_override(set), Some(PathBuf::from("/opt/dashcraft-app")));
        assert_eq!(app_override(|_| Some(OsString::new())), None);
        assert_eq!(app_override(|_| None), None);
    }

    #[test]
    fn candidates_look_for_the_dashcraft_app_executable() {
        let found = candidates();
        assert!(!found.is_empty());
        assert!(
            found.iter().all(|p| p
                .file_stem()
                .is_some_and(|s| s.to_string_lossy().eq_ignore_ascii_case(APP_BIN))),
            "{found:?}"
        );
    }
}
