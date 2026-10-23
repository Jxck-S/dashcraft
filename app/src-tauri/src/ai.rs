//! The opt-in AI layer (`pbi-ai`, ADR-0008) in the app. The registry is `pbi_ai::catalog()` (the
//! core commands plus `ai.*`, see [`crate::state::app_registry`]); this module installs the AI
//! host at startup: settings in `ai.json` in the app's configuration folder, API keys in the OS
//! keychain (shared with `dashcraft ai login`), the built-in provider adapters. AI stays off until
//! a person enables it in File → Options and settings → Options → AI, and nothing is sent without
//! a consent recorded there or in the consent dialog.
//!
//! Debug builds only: `DASHCRAFT_AI_MOCK=1` swaps the adapters for a demo provider ([`demo`]) that
//! previews the real request but answers with canned text and never touches the network. The
//! module is compiled out of release builds.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use pbi_ai::AiHost;

#[cfg(debug_assertions)]
mod demo;

/// The demo provider is installed (debug builds with `DASHCRAFT_AI_MOCK=1`).
static DEMO: AtomicBool = AtomicBool::new(false);

/// The app's AI settings file: `$DASHCRAFT_CONFIG_DIR/ai.json` (alias `PBIVIEW_CONFIG_DIR`, shared
/// with the CLI) when set, else `<app config dir>/ai.json`, else the CLI's default location.
pub fn settings_path(
    env: impl Fn(&str) -> Option<OsString>,
    app_config_dir: Option<&Path>,
) -> Option<PathBuf> {
    let var = |name: &str| env(name).filter(|v| !v.is_empty()).map(PathBuf::from);
    var("DASHCRAFT_CONFIG_DIR")
        .or_else(|| var("PBIVIEW_CONFIG_DIR"))
        .or_else(|| app_config_dir.map(Path::to_path_buf))
        .map(|dir| dir.join("ai.json"))
        .or_else(pbi_ai::default_settings_path)
}

/// Installs the process-wide AI host (once, at startup).
pub fn install(app_config_dir: Option<&Path>) {
    let path = settings_path(|name| std::env::var_os(name), app_config_dir);
    let host = match path {
        Some(path) => AiHost::new(path),
        None => AiHost::from_default_path(),
    };
    #[cfg(debug_assertions)]
    let host = if demo::requested(|name| std::env::var_os(name)) {
        DEMO.store(true, Ordering::Relaxed);
        host.with_provider_factory(demo::DemoFactory)
    } else {
        host
    };
    host.install();
}

/// Whether AI requests are answered by the debug-only demo provider.
pub fn demo_active() -> bool {
    cfg!(debug_assertions) && DEMO.load(Ordering::Relaxed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_live_in_the_app_config_folder_unless_overridden() {
        let app = Path::new("/cfg/io.github.jxck-s.dashcraft");
        let none = |_: &str| None;
        assert_eq!(
            settings_path(none, Some(app)),
            Some(app.join("ai.json")),
            "the app's own folder by default"
        );
        let env = |name: &str| (name == "PBIVIEW_CONFIG_DIR").then(|| OsString::from("/shared"));
        assert_eq!(
            settings_path(env, Some(app)),
            Some(Path::new("/shared").join("ai.json"))
        );
        let empty = |_: &str| Some(OsString::new());
        assert_eq!(settings_path(empty, Some(app)), Some(app.join("ai.json")));
    }

    /// The demo provider never reaches a release build: the module and the only reader of
    /// `DASHCRAFT_AI_MOCK` are behind `cfg(debug_assertions)`.
    #[test]
    fn the_demo_provider_is_compiled_only_into_debug_builds() {
        let this = include_str!("ai.rs");
        assert!(this.contains("#[cfg(debug_assertions)]\nmod demo;"));
        assert!(this.contains("#[cfg(debug_assertions)]\n    let host = if demo::requested"));
        let needle = format!("\"{}\"", ["DASHCRAFT", "_AI_MOCK"].concat());
        let demo = include_str!("ai/demo.rs");
        assert!(demo.contains(&needle));
        for (name, src) in [
            ("ai.rs", this),
            ("lib.rs", include_str!("lib.rs")),
            ("state.rs", include_str!("state.rs")),
            ("ops.rs", include_str!("ops.rs")),
            ("commands.rs", include_str!("commands.rs")),
        ] {
            let code = src.split("#[cfg(test)]").next().unwrap_or(src);
            assert!(
                !code.contains(&needle),
                "{name} reads {needle} outside the demo module"
            );
        }
    }
}
