//! Workspace paths and cargo invocations shared by the commands.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context, Result, bail};

/// The workspace root (the parent of `xtask/`).
pub fn workspace_root() -> PathBuf {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    manifest.parent().unwrap_or(manifest).to_path_buf()
}

/// `cargo` (the one running `cargo xtask`, when known), started in the workspace root.
pub fn cargo() -> Command {
    let program = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    let mut cmd = Command::new(program);
    cmd.current_dir(workspace_root());
    cmd
}

/// `cargo metadata --format-version 1 <extra>` as JSON.
pub fn cargo_metadata(extra: &[&str]) -> Result<serde_json::Value> {
    let output = cargo()
        .args(["metadata", "--format-version", "1"])
        .args(extra)
        .stderr(Stdio::inherit())
        .output()
        .context("cannot run `cargo metadata`")?;
    if !output.status.success() {
        bail!("`cargo metadata` failed ({})", output.status);
    }
    serde_json::from_slice(&output.stdout).context("cannot parse `cargo metadata` output")
}

/// Trimmed stdout of a command, or `None` when it cannot run or fails.
pub fn command_stdout(program: &str, args: &[&str]) -> Option<String> {
    let output = Command::new(program)
        .args(args)
        .current_dir(workspace_root())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    (!text.is_empty()).then_some(text)
}
