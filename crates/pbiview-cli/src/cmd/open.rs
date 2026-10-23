//! `dashcraft open`: launches the desktop app (`dashcraft-app`) on a file
//! or folder.
//!
//! The app is looked up, in order: `$DASHCRAFT_APP` (or the legacy
//! `$PBIVIEW_APP`; the new name wins); next to this executable; on macOS in a
//! `DashCraft.app` bundle next to this executable, in `/Applications` and in
//! `~/Applications`; then on `PATH`. Each location is tried with the current
//! binary name (`dashcraft-app`, bundle `DashCraft.app`) before the legacy
//! one (`pbiview-app`, `pbiview.app`). The path is passed as a single
//! argument (never through a shell).

use std::env;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context as _, anyhow, bail};

use crate::common::{Context, Status, prog};

/// Environment variable overriding the app executable.
const APP_ENV: &str = "DASHCRAFT_APP";
/// Former name of [`APP_ENV`], still honoured when it is not set.
const APP_ENV_ALIAS: &str = "PBIVIEW_APP";
/// File names (without extension) of the desktop app executable, current
/// first, then the name used before the rename to DashCraft.
const APP_BINS: [&str; 2] = ["dashcraft-app", "pbiview-app"];
/// macOS bundle names, current first.
const BUNDLES: [&str; 2] = ["DashCraft.app", "pbiview.app"];

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

/// `$DASHCRAFT_APP`, else `$PBIVIEW_APP` (empty values are ignored), with
/// the name of the variable that was used.
fn app_override(var: impl Fn(&str) -> Option<OsString>) -> Option<(&'static str, PathBuf)> {
    [APP_ENV, APP_ENV_ALIAS].into_iter().find_map(|name| {
        var(name)
            .filter(|v| !v.is_empty())
            .map(|v| (name, PathBuf::from(v)))
    })
}

fn find_app() -> anyhow::Result<PathBuf> {
    if let Some((name, app)) = app_override(|n| env::var_os(n)) {
        if is_executable(&app) {
            return Ok(app);
        }
        bail!(
            "{name} is set to {}, which is not an executable file",
            app.display()
        );
    }
    let this = env::current_exe().ok().and_then(|p| p.canonicalize().ok());
    let found = candidates()
        .into_iter()
        .find(|c| is_executable(c) && (this.is_none() || c.canonicalize().ok() != this));
    found.ok_or_else(|| {
        anyhow!(
            "the DashCraft desktop app ({}) was not found next to this executable{}, \
             or on PATH. Install the desktop app, or set {APP_ENV} to its executable. \
             `{} inspect <path>` works without it.",
            APP_BINS[0],
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
    let file_names = APP_BINS.map(|bin| format!("{bin}{}", env::consts::EXE_SUFFIX));
    let in_dir = |dir: &Path| file_names.iter().map(|f| dir.join(f)).collect::<Vec<_>>();
    let mut out = Vec::new();
    if let Some(dir) = env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(Path::to_path_buf))
    {
        out.extend(in_dir(&dir));
        if cfg!(target_os = "macos") {
            out.extend(bundles_in(&dir));
        }
    }
    if cfg!(target_os = "macos") {
        out.extend(bundles_in(Path::new("/Applications")));
        if let Some(home) = env::var_os("HOME") {
            out.extend(bundles_in(&PathBuf::from(home).join("Applications")));
        }
    }
    if let Some(path) = env::var_os("PATH") {
        out.extend(env::split_paths(&path).flat_map(|dir| in_dir(&dir)));
    }
    out
}

/// Executables the macOS bundles in `dir` may contain, current bundle and
/// binary name first.
fn bundles_in(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for bundle in BUNDLES {
        let macos = dir.join(bundle).join("Contents").join("MacOS");
        out.extend(APP_BINS.iter().map(|bin| macos.join(bin)));
        // Bundles built before `mainBinaryName` was set named the binary after the product.
        out.push(macos.join("pbiview"));
    }
    out
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
    fn dashcraft_app_wins_over_the_legacy_variable() {
        let both = |n: &str| match n {
            "DASHCRAFT_APP" => Some(OsString::from("/new/dashcraft-app")),
            "PBIVIEW_APP" => Some(OsString::from("/old/pbiview-app")),
            _ => None,
        };
        assert_eq!(
            app_override(both),
            Some(("DASHCRAFT_APP", PathBuf::from("/new/dashcraft-app")))
        );
        let legacy = |n: &str| match n {
            "DASHCRAFT_APP" => Some(OsString::new()),
            "PBIVIEW_APP" => Some(OsString::from("/old/pbiview-app")),
            _ => None,
        };
        assert_eq!(
            app_override(legacy),
            Some(("PBIVIEW_APP", PathBuf::from("/old/pbiview-app")))
        );
        assert_eq!(app_override(|_| None), None);
    }

    #[test]
    fn the_current_app_name_is_tried_before_the_legacy_one() {
        let found = candidates();
        let pos = |name: &str| {
            found
                .iter()
                .position(|p| p.file_stem().is_some_and(|s| s == name))
        };
        assert!(pos("dashcraft-app") < pos("pbiview-app"), "{found:?}");
    }
}
