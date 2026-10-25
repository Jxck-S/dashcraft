//! DashCraft desktop app (`dashcraft-app`): Tauri 2 shell (Rust backend for the Svelte UI in
//! `app/src`).
//!
//! The app is a thin front end over the command registry (`pbi-ops`, ADR-0007): documents live
//! in a `pbi_ops::Workspace` and every domain operation (open, refresh, grid pages, statistics,
//! DAX, measures, the interactive canvas, exports) runs through [`ops::ops_execute`] →
//! `Registry::execute`, the same code path as `dashcraft run`, `dashcraft serve` and agents. Its
//! typed TypeScript client (`app/src/lib/ops.gen.ts`) is generated from the registry's JSON
//! Schemas (`npm run gen:ops`).
//!
//! The registry is `pbi_ai::catalog()`: the core commands plus the opt-in `ai.*` commands
//! (ADR-0008), off until a person enables AI in File → Options and settings → Options; [`run`]
//! installs the AI host with the settings in the app's configuration folder ([`ai`]).
//!
//! The remaining Tauri commands are app-only: UI projections of an open document with object
//! ids (Data pane tree, report outline, Properties / Format panes, TMDL and Model views), the
//! blob map and File → Recent settings, and window plumbing. IPC is typed with tauri-specta:
//! every command and event is listed in [`specta_builder`], and the `export_bindings` test
//! writes the matching TypeScript client to `app/src/lib/bindings.ts`.

mod ai;
mod blob;
mod commands;
pub mod dto;
mod error;
pub mod events;
mod mapping;
mod ops;
mod recent;
mod state;
mod views;

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use tauri::{Manager, Runtime};
use tauri_specta::Event;

pub use error::AppError;
pub use state::AppState;

/// Path passed on the command line (`dashcraft-app <path>`), captured once at startup.
#[derive(Debug, Default, Clone)]
pub struct StartupPath(Option<PathBuf>);

impl StartupPath {
    /// Picks the first non-flag argument (argv[0] excluded) and makes it absolute, so it stays
    /// valid regardless of later working-directory changes. Flags such as macOS's legacy
    /// `-psn_…` process serial number are ignored.
    pub fn from_args<I>(args: I) -> Self
    where
        I: IntoIterator<Item = OsString>,
    {
        let path = args
            .into_iter()
            .find(|a| !a.is_empty() && !a.to_string_lossy().starts_with('-'))
            .map(PathBuf::from)
            .map(|p| std::path::absolute(&p).unwrap_or(p));
        Self(path)
    }

    /// The captured path, if any.
    pub fn get(&self) -> Option<&Path> {
        self.0.as_deref()
    }
}

/// A file the OS asked to open before the UI was ready to listen for
/// [`events::OpenFileRequested`] (macOS delivers document opens as a run event, not as argv).
/// `startup_path` hands it out once.
#[derive(Debug, Default)]
pub struct PendingOpen(std::sync::Mutex<Option<PathBuf>>);

impl PendingOpen {
    // Only macOS/iOS deliver document opens as run events (`handle_run_event`).
    #[cfg(any(target_os = "macos", target_os = "ios", test))]
    fn set(&self, path: PathBuf) {
        match self.0.lock() {
            Ok(mut slot) => *slot = Some(path),
            Err(poisoned) => *poisoned.into_inner() = Some(path),
        }
    }

    /// Takes the pending path, if any.
    pub fn take(&self) -> Option<PathBuf> {
        match self.0.lock() {
            Ok(mut slot) => slot.take(),
            Err(poisoned) => poisoned.into_inner().take(),
        }
    }
}

/// Command registry shared by the runtime invoke handler and the bindings exporter.
pub fn specta_builder<R: Runtime>() -> tauri_specta::Builder<R> {
    tauri_specta::Builder::<R>::new()
        .commands(tauri_specta::collect_commands![
            ops::ops_execute,
            ops::ops_cancel,
            ops::ops_catalog,
            commands::app_version,
            commands::ai_demo_mode,
            commands::startup_path,
            commands::document_views,
            commands::object_details,
            commands::tmdl_view,
            commands::model_diagram,
            commands::visual_format,
            commands::blob_map,
            commands::set_blob_map,
            commands::recent_files,
            commands::add_recent_file,
            commands::remove_recent_file,
        ])
        .events(tauri_specta::collect_events![
            events::OpsProgress,
            events::OpenFileRequested
        ])
        // One TS type per Rust type (no `Foo_Serialize`/`Foo_Deserialize` split); IPC DTOs
        // must therefore keep identical serialize/deserialize shapes.
        .disable_serde_phases()
}

/// Registers plugins, state and commands. Generic over the runtime so tests can use
/// `tauri::test::MockRuntime` without opening a window.
fn configure<R: Runtime>(
    builder: tauri::Builder<R>,
    specta: &tauri_specta::Builder<R>,
    startup: StartupPath,
    state: AppState,
) -> tauri::Builder<R> {
    builder
        .plugin(tauri_plugin_dialog::init())
        .manage(startup)
        .manage(PendingOpen::default())
        .manage(state)
        .invoke_handler(specta.invoke_handler())
}

/// Wiring that needs the built app: the typed events and the command progress sink.
fn attach<R: Runtime>(app: &tauri::App<R>, specta: &tauri_specta::Builder<R>) {
    specta.mount_events(app);
    let handle = app.handle().clone();
    app.state::<AppState>()
        .set_progress_sink(state::ProgressSink::new(move |progress| {
            // Nothing to do if the window is gone; the command result still reaches the caller.
            let _ = progress.emit(&handle);
        }));
}

/// Builds and runs the Tauri application. Blocks until the last window closes.
pub fn run() -> Result<(), tauri::Error> {
    let startup = StartupPath::from_args(std::env::args_os().skip(1));
    let specta = specta_builder::<tauri::Wry>();
    let app = configure(
        tauri::Builder::default(),
        &specta,
        startup,
        AppState::from_env(),
    )
    .build(tauri::generate_context!())?;
    attach(&app, &specta);
    // File → Recent persists in the app config folder (tests keep it in memory).
    let config_dir = app.path().app_config_dir().ok();
    if let Some(dir) = &config_dir {
        app.state::<AppState>()
            .recent
            .set_store(dir.join("recent-files.json"));
    }
    // The AI settings (`ai.json`, off by default) live there too; keys stay in the OS keychain.
    ai::install(config_dir.as_deref());
    app.run(handle_run_event);
    Ok(())
}

/// Forwards macOS document-open requests (Finder double-click, "Open With", dock drops) to the
/// UI: stored for a UI that has not started listening yet, and emitted for one that has.
/// Windows and Linux pass documents as command-line arguments instead ([`StartupPath`]).
#[allow(clippy::needless_pass_by_value)] // signature required by `App::run`
fn handle_run_event<R: Runtime>(handle: &tauri::AppHandle<R>, event: tauri::RunEvent) {
    #[cfg(any(target_os = "macos", target_os = "ios"))]
    if let tauri::RunEvent::Opened { urls } = &event {
        for url in urls {
            let Ok(path) = url.to_file_path() else {
                continue;
            };
            if let Some(pending) = handle.try_state::<PendingOpen>() {
                pending.set(path.clone());
            }
            // No window yet is fine: the UI picks the path up from `startup_path`.
            let _ = events::OpenFileRequested {
                path: path.to_string_lossy().into_owned(),
            }
            .emit(handle);
        }
    }
    #[cfg(not(any(target_os = "macos", target_os = "ios")))]
    let _ = (handle, event);
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};
    use std::sync::atomic::{AtomicU64, Ordering};

    /// Regenerates `app/src/lib/bindings.ts` from the registered commands and types.
    /// Run via `npm run bindings` (or `cargo test -p dashcraft-app export_bindings`) after
    /// changing any command signature; commit the result.
    #[test]
    fn export_bindings() {
        let out = Path::new(env!("CARGO_MANIFEST_DIR")).join("../src/lib/bindings.ts");
        specta_builder::<tauri::Wry>()
            .export(
                specta_typescript::Typescript::default()
                    .header("/* eslint-disable */\n// @ts-nocheck"),
                &out,
            )
            .unwrap();
        let generated = std::fs::read_to_string(&out).unwrap();
        for name in [
            "opsExecute",
            "opsCancel",
            "opsCatalog",
            "OpErrorDto",
            "OpsProgress",
            "appVersion",
            "startupPath",
            "documentViews",
            "objectDetails",
            "DocumentSummary",
            "ModelTreeDto",
            "ObjectDetailsDto",
            "AppError",
            "tmdlView",
            "modelDiagram",
            "visualFormat",
            "recentFiles",
            "addRecentFile",
            "ReportThemeDto",
        ] {
            assert!(generated.contains(name), "bindings are missing `{name}`");
        }
    }

    type MockWebview = tauri::WebviewWindow<tauri::test::MockRuntime>;

    /// Builds the app on Tauri's mock runtime (no window, no event loop) with one webview.
    fn mock_app(startup: StartupPath) -> (tauri::App<tauri::test::MockRuntime>, MockWebview) {
        let specta = specta_builder();
        let app = configure(
            tauri::test::mock_builder(),
            &specta,
            startup,
            AppState::default(),
        )
        .build(tauri::test::mock_context(tauri::test::noop_assets()))
        .unwrap();
        attach(&app, &specta);
        let webview = tauri::WebviewWindowBuilder::new(&app, "main", Default::default())
            .build()
            .unwrap();
        (app, webview)
    }

    /// Invokes a command through the real IPC path (mock runtime, no window) and returns the
    /// JSON the frontend would receive.
    fn invoke(webview: &MockWebview, cmd: &str, body: Value) -> Result<Value, Value> {
        let url = if cfg!(windows) {
            "http://tauri.localhost"
        } else {
            "tauri://localhost"
        };
        tauri::test::get_ipc_response(
            webview,
            tauri::webview::InvokeRequest {
                cmd: cmd.into(),
                callback: tauri::ipc::CallbackFn(0),
                error: tauri::ipc::CallbackFn(1),
                url: url.parse().unwrap(),
                body: tauri::ipc::InvokeBody::Json(body),
                headers: Default::default(),
                invoke_key: tauri::test::INVOKE_KEY.to_owned(),
            },
        )
        .map(|res| res.deserialize::<Value>().unwrap())
    }

    static REQUESTS: AtomicU64 = AtomicU64::new(1);

    /// Runs registry command `id` through `ops_execute`, with a fresh request id.
    fn ops_with(
        webview: &MockWebview,
        request: &str,
        id: &str,
        params: Value,
    ) -> Result<Value, Value> {
        invoke(
            webview,
            "ops_execute",
            json!({ "requestId": request, "id": id, "params": params }),
        )
    }

    fn ops(webview: &MockWebview, id: &str, params: Value) -> Result<Value, Value> {
        let request = format!("t{}", REQUESTS.fetch_add(1, Ordering::Relaxed));
        ops_with(webview, &request, id, params)
    }

    fn run(webview: &MockWebview, id: &str, params: Value) -> Value {
        ops(webview, id, params.clone()).unwrap_or_else(|e| panic!("{id} {params}: {e}"))
    }

    /// Opens `path` as the app does (`doc.open` without automatic loading) and returns its
    /// document views.
    fn open(webview: &MockWebview, path: &Path) -> Value {
        let opened = run(
            webview,
            "doc.open",
            json!({ "path": path, "autoRefresh": false }),
        );
        invoke(
            webview,
            "document_views",
            json!({ "docId": opened["docId"] }),
        )
        .unwrap()
    }

    /// Fixture root: `$DASHCRAFT_REFERENCE`, else `<workspace>/reference`.
    fn reference() -> PathBuf {
        std::env::var_os("DASHCRAFT_REFERENCE").map_or_else(
            || Path::new(env!("CARGO_MANIFEST_DIR")).join("../../reference"),
            PathBuf::from,
        )
    }

    /// The synthetic Acme Outfitters example (`reference/examples/acme-outfitters/`).
    const ACME: &str = "examples/acme-outfitters";

    /// The fixture path, or `None` (after printing `SKIP`) when it is missing.
    fn fixture(rel: &str) -> Option<PathBuf> {
        let path = reference().join(rel);
        if path.exists() {
            Some(path)
        } else {
            println!("SKIP: {} missing", path.display());
            None
        }
    }

    fn array<'a>(value: &'a Value, key: &str) -> &'a Vec<Value> {
        value[key]
            .as_array()
            .unwrap_or_else(|| panic!("`{key}` is not an array in {value}"))
    }

    /// Guards the wire contract that `bindings.ts` and `ops.gen.ts` promise: argument names,
    /// camelCase fields and the `{ code, message, data }` / `{ kind, message }` error shapes.
    #[test]
    fn ipc_commands_round_trip() {
        let (_app, webview) = mock_app(StartupPath::from_args([OsString::from("startup.pbix")]));

        assert_eq!(
            invoke(&webview, "app_version", json!({})),
            Ok(json!(env!("CARGO_PKG_VERSION")))
        );
        let startup = invoke(&webview, "startup_path", json!({})).unwrap();
        assert!(startup.as_str().unwrap().ends_with("startup.pbix"));
        assert_eq!(invoke(&webview, "recent_files", json!({})), Ok(json!([])));

        // The catalog: ids, categories and flags.
        let catalog = invoke(&webview, "ops_catalog", json!({})).unwrap();
        let find = |id: &str| {
            array(&json!({ "c": catalog }), "c")
                .iter()
                .find(|c| c["id"] == id)
                .cloned()
                .unwrap_or_else(|| panic!("no {id} in the catalog"))
        };
        assert_eq!(find("doc.open")["category"], "workspace");
        assert_eq!(find("canvas.click")["mutatesDocument"], true);
        assert_eq!(find("data.refresh")["longRunning"], true);

        // Registry errors keep their stable codes.
        let err = ops(&webview, "doc.nope", json!({})).unwrap_err();
        assert_eq!(err["code"], "unknownCommand", "{err}");
        let err = ops(
            &webview,
            "data.page",
            json!({ "docId": "d9", "table": "T" }),
        )
        .unwrap_err();
        assert_eq!(err["code"], "noDocument");
        let err = ops(&webview, "doc.open", json!({ "path": "x", "nope": 1 })).unwrap_err();
        assert_eq!(err["code"], "invalidParams");
        let manifest = Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml");
        let err = ops(
            &webview,
            "doc.open",
            json!({ "path": manifest.with_file_name("missing.pbix") }),
        )
        .unwrap_err();
        assert_eq!(err["code"], "notFound");
        assert!(err["message"].as_str().unwrap().contains("missing.pbix"));
        // Detection is content-based: a TOML file is not a Power BI artifact.
        let err = ops(&webview, "doc.open", json!({ "path": manifest })).unwrap_err();
        assert_eq!(err["code"], "unsupported", "{err}");
        assert_eq!(run(&webview, "doc.list", json!({}))["documents"], json!([]));

        // App-only views of an unknown document; cancelling an unknown request.
        let err = invoke(&webview, "document_views", json!({ "docId": "d1" })).unwrap_err();
        assert_eq!(err["kind"], "noDocument");
        let err = invoke(&webview, "tmdl_view", json!({ "docId": "d1" })).unwrap_err();
        assert_eq!(err["kind"], "noDocument");
        assert_eq!(
            invoke(&webview, "ops_cancel", json!({ "requestId": "nope" })),
            Ok(json!(false))
        );
        let err = ops_with(&webview, "", "doc.list", json!({})).unwrap_err();
        assert_eq!(err["code"], "invalidParams");
    }

    #[test]
    fn requests_are_tracked_and_cancellable() {
        let state = AppState::default();
        let token = state.begin_request("r1", "data.refresh").unwrap();
        assert!(
            state.begin_request("r1", "doc.list").is_err(),
            "ids are unique"
        );
        assert!(state.is_running("data.refresh"));
        assert!(!state.is_running("dax.query"));
        assert!(state.cancel_request("r1"));
        assert!(token.is_cancelled());
        state.end_request("r1");
        assert!(!state.is_running("data.refresh"));
        assert!(!state.cancel_request("r1"));
    }

    /// The app runs `pbi_ai::catalog()`, the catalog `app/src/lib/ops.gen.ts` is generated from
    /// (its content is checked by `pbi-ai`'s `ops_typescript_is_up_to_date`, which CI runs).
    #[test]
    fn app_registry_is_the_generated_catalog() {
        let ids = |r: &pbi_ops::Registry| r.list().iter().map(|s| s.meta.id).collect::<Vec<_>>();
        let app = ids(&AppState::default().registry);
        assert_eq!(app, ids(&pbi_ai::catalog()));
        assert!(app.contains(&"ai.explainMeasure") && app.contains(&"doc.open"));
        let generated = std::fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../src/lib/ops.gen.ts"),
        )
        .unwrap();
        for id in app {
            assert!(
                generated.contains(&format!("  '{id}': {{ params: ")),
                "ops.gen.ts lacks {id}: run `npm run gen:ops` in app/"
            );
        }
    }

    /// The AI commands through the app's IPC path (`CallerKind::App`): off by default, keys only
    /// stored (never returned), consent required before anything runs, then the answer.
    #[test]
    fn ai_commands_run_from_the_app_with_consent() {
        use pbi_ai::SecretStore as _;
        let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/tmp")
            .join(format!("app-ai-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let model = dir.join("model.bim");
        std::fs::write(
            &model,
            json!({
                "name": "AiTest",
                "compatibilityLevel": 1567,
                "model": {"culture": "en-US", "tables": [{
                    "name": "Sales",
                    "columns": [{"name": "Amount", "dataType": "double", "sourceColumn": "Amount"}],
                    "measures": [{"name": "Total", "expression": "SUM(Sales[Amount])"}],
                }]},
            })
            .to_string(),
        )
        .unwrap();
        let keys = std::sync::Arc::new(pbi_ai::MemorySecretStore::new());
        #[derive(Debug)]
        struct Shared(std::sync::Arc<pbi_ai::MemorySecretStore>);
        impl pbi_ai::SecretStore for Shared {
            fn name(&self) -> &'static str {
                "memory"
            }
            fn get(&self, id: &str) -> Result<Option<pbi_ai::Secret>, pbi_ai::AiError> {
                self.0.get(id)
            }
            fn set(&self, id: &str, key: &pbi_ai::Secret) -> Result<(), pbi_ai::AiError> {
                self.0.set(id, key)
            }
            fn delete(&self, id: &str) -> Result<bool, pbi_ai::AiError> {
                self.0.delete(id)
            }
        }
        pbi_ai::AiHost::new(dir.join("ai.json"))
            .with_secret_store(Shared(std::sync::Arc::clone(&keys)))
            .with_env(|_| None)
            .install();

        let (_app, webview) = mock_app(StartupPath::default());
        let doc = run(&webview, "doc.open", json!({ "path": model }))["docId"].clone();
        let status = run(&webview, "ai.status", json!({}));
        assert_eq!(status["enabled"], false);
        assert_eq!(status["settings"]["providers"], json!([]));
        let explain = json!({ "docId": doc, "measure": "Total" });
        let err = ops(&webview, "ai.explainMeasure", explain.clone()).unwrap_err();
        assert_eq!(
            (err["code"].as_str(), err["data"]["reason"].as_str()),
            (Some("unsupported"), Some("disabled"))
        );

        // Keys: accepted from the app, stored, never read back.
        let stored = run(
            &webview,
            "ai.apiKey.set",
            json!({ "provider": "openai", "key": "sk-app-SECRET-1" }),
        );
        assert_eq!(stored["changed"], true);
        assert_eq!(
            keys.get("openai").unwrap().map(|k| k.expose().to_owned()),
            Some("sk-app-SECRET-1".to_owned())
        );

        run(
            &webview,
            "ai.settings.set",
            json!({
                "enabled": true,
                "providers": [
                    {"id": "demo", "kind": "mock"},
                    {"id": "openai", "kind": "openAiCompatible", "model": "m", "endpoint": "https://api.example.test/v1"},
                ],
                "defaultProvider": "demo",
            }),
        );
        let status = run(&webview, "ai.status", json!({}));
        assert_eq!(status["providers"][1]["apiKey"], "keychain");
        assert!(!status.to_string().contains("sk-app-SECRET-1"));

        // No consent: nothing runs; the error says what to grant, the preview what would go.
        let err = ops(&webview, "ai.explainMeasure", explain.clone()).unwrap_err();
        assert_eq!(err["data"]["reason"], "consentRequired", "{err}");
        let grant = err["data"]["grant"]["params"].clone();
        assert_eq!(grant["provider"], "demo");
        let preview = run(
            &webview,
            "ai.preview",
            json!({ "capability": "dax.explain", "docId": doc, "measure": "Total", "provider": "openai" }),
        );
        assert_eq!(preview["wouldSend"], false);
        assert_eq!(preview["consent"]["endpointHost"], "api.example.test");
        assert!(
            preview["request"]["body"]
                .to_string()
                .contains("SUM(Sales[Amount])")
        );
        assert!(!preview.to_string().contains("sk-app-SECRET-1"));

        let granted = run(&webview, "ai.consent.grant", grant);
        assert_eq!(granted["consent"]["grantedBy"], "app");
        let explained = run(&webview, "ai.explainMeasure", explain);
        assert_eq!(explained["measure"], "Total");
        assert!(!explained["summary"].as_str().unwrap_or_default().is_empty());

        run(&webview, "ai.apiKey.clear", json!({ "provider": "openai" }));
        assert!(keys.get("openai").unwrap().is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The example's `expected.json` (values computed by its generator, independently of
    /// DashCraft), or `None` (after printing `SKIP`).
    fn expected() -> Option<Value> {
        let path = fixture(&format!("{ACME}/expected.json"))?;
        Some(serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap())
    }

    #[test]
    fn opens_the_example_project() {
        let Some(pbip) = fixture(&format!("{ACME}/AcmeOutfitters.pbip")) else {
            return;
        };
        let (_app, webview) = mock_app(StartupPath::default());

        let views = open(&webview, &pbip);
        let summary = &views["summary"];
        assert_eq!(summary["docId"], "d1");
        assert_eq!(summary["fileName"], "AcmeOutfitters.pbip");
        assert_eq!(summary["entryKind"], "file");
        assert_eq!(summary["format"], "pbip");
        assert_eq!(summary["modelStatus"]["availability"], "local");
        assert_eq!(summary["counts"]["tables"], 11);
        assert_eq!(summary["counts"]["measures"], 33);
        assert_eq!(summary["counts"]["relationships"], 8);
        assert_eq!(summary["counts"]["reports"], 3);
        assert_eq!(summary["cache"], Value::Null);
        assert_eq!(summary["diagnostics"]["errors"], 0);
        let listed = run(&webview, "doc.list", json!({}));
        assert_eq!(listed["documents"][0]["docId"], "d1");

        // Model tree: 11 tables, 33 measures, 8 relationships (the first bidirectional
        // many-to-one).
        let tree = &views["tree"];
        let tables = array(tree, "tables");
        assert_eq!(tables.len(), 11);
        let measures: Vec<&Value> = tables.iter().flat_map(|t| array(t, "measures")).collect();
        assert_eq!(measures.len(), 33);
        let rels = array(tree, "relationships");
        assert_eq!(rels.len(), 8);
        assert_eq!(rels[0]["fromTable"], "Orders");
        assert_eq!(rels[0]["toTable"], "Customers");
        assert_eq!(rels[0]["cardinality"], "manyToOne");
        assert_eq!(rels[0]["crossFilter"], "both");
        assert_eq!(rels[0]["active"], true);
        assert!(rels.iter().any(|r| r["active"] == false));
        let orders = tables.iter().find(|t| t["name"] == "Orders").unwrap();
        let returned = array(orders, "columns")
            .iter()
            .find(|c| c["name"] == "Is Returned")
            .unwrap();
        assert_eq!(returned["kind"], "calculated");
        assert!(
            returned["expression"]
                .as_str()
                .unwrap()
                .contains("RELATEDTABLE")
        );
        assert_eq!(array(orders, "partitions")[0]["kind"], "m");
        let blob = array(tree, "expressions")
            .iter()
            .find(|e| e["name"] == "StorageContainerUrl")
            .unwrap();
        assert_eq!(blob["isParameter"], true);
        assert_eq!(blob["parameterType"], "Text");

        // Report outline: 3 reports with pages, positioned visuals and resolved fields.
        let reports = array(&views, "reports");
        assert_eq!(reports.len(), 3);
        assert_eq!(reports[0]["name"], "Acme Outfitters - Overview");
        assert_eq!(reports[0]["dataset"]["kind"], "byPath");
        let pages = array(&reports[0], "pages");
        assert_eq!(pages.len(), 3);
        assert!(pages[0]["width"].as_f64().unwrap() > 0.0);
        let visuals: Vec<&Value> = pages.iter().flat_map(|p| array(p, "visuals")).collect();
        assert!(visuals.iter().any(|v| v["visualType"] == "cardVisual"));
        let field = visuals
            .iter()
            .flat_map(|v| array(v, "fields"))
            .find(|f| f["kind"] == "measure")
            .unwrap();
        assert!(field["sourceName"].as_str().unwrap().contains('['));
        assert!(!field["displayName"].as_str().unwrap().is_empty());
        // The page names the canvas commands take are the outline's internal names.
        let pages_out = run(
            &webview,
            "report.pages",
            json!({ "docId": "d1", "report": reports[0]["name"] }),
        );
        assert_eq!(pages_out["pages"][0]["name"], pages[0]["name"]);

        // Properties pane: a measure's details carry its DAX and origin file.
        let total = measures
            .iter()
            .find(|m| m["name"] == "Total Sales")
            .unwrap();
        let details = invoke(
            &webview,
            "object_details",
            json!({ "docId": "d1", "id": total["id"] }),
        )
        .unwrap();
        assert_eq!(details["kind"], "measure");
        assert_eq!(details["name"], "Total Sales");
        assert_eq!(details["parent"], "Orders");
        let code = array(&details, "code");
        assert_eq!(code[0]["language"], "dax");
        assert!(
            code[0]["text"]
                .as_str()
                .unwrap()
                .contains("SUM(Orders[Line Amount])")
        );
        assert!(
            details["origin"]
                .as_str()
                .unwrap()
                .contains("tables/Orders.tmdl")
        );
        let visual_id = visuals[0]["id"].clone();
        let details = invoke(
            &webview,
            "object_details",
            json!({ "docId": "d1", "id": visual_id }),
        )
        .unwrap();
        assert_eq!(details["kind"], "visual");
        assert!(details["origin"].as_str().unwrap().ends_with("visual.json"));
        let err = invoke(
            &webview,
            "object_details",
            json!({ "docId": "d1", "id": 999_999 }),
        )
        .unwrap_err();
        assert_eq!(err["kind"], "unknownObject");

        // Themes gallery: the report's custom theme and the colors of its JSON file.
        let theme_file: Value = serde_json::from_str(
            &std::fs::read_to_string(pbip.parent().unwrap().join(
                "Acme Outfitters - Overview.Report/StaticResources/RegisteredResources/AcmeOutfitters.json",
            ))
            .unwrap(),
        )
        .unwrap();
        let theme = &reports[0]["theme"];
        assert_eq!(theme["customTheme"], "AcmeOutfitters.json", "{theme}");
        assert_eq!(array(theme, "dataColors")[0], theme_file["dataColors"][0]);
        assert_eq!(theme["background"], theme_file["background"]);

        // Format pane: formatting cards plus the container's position.
        let format = |id: &Value| {
            invoke(
                &webview,
                "visual_format",
                json!({ "docId": "d1", "id": id }),
            )
        };
        let layout = format(&visual_id).unwrap();
        assert!(
            array(&layout, "layout")
                .iter()
                .any(|p| p["label"] == "Width"
                    && p["value"].as_str().unwrap().parse::<f64>().is_ok()),
            "{layout}"
        );
        let carded = visuals
            .iter()
            .map(|v| format(&v["id"]).unwrap())
            .find(|f| !array(f, "visual").is_empty())
            .expect("a visual with formatting objects");
        let card = &array(&carded, "visual")[0];
        assert!(!card["label"].as_str().unwrap().is_empty());
        assert!(!array(&array(card, "entries")[0], "properties").is_empty());
        assert_eq!(format(&total["id"]).unwrap_err()["kind"], "unknownObject");

        // TMDL view: the project's .tmdl files, verbatim.
        let tmdl = invoke(&webview, "tmdl_view", json!({ "docId": "d1" })).unwrap();
        assert_eq!(tmdl["source"], "files");
        let files = array(&tmdl, "files");
        assert_eq!(files[0]["path"], "database.tmdl");
        let orders_file = files
            .iter()
            .find(|f| f["path"] == "tables/Orders.tmdl")
            .unwrap_or_else(|| panic!("no tables/Orders.tmdl in {tmdl}"));
        assert_eq!(orders_file["table"], "Orders");
        assert!(
            files
                .iter()
                .any(|f| f["path"] == "roles/Store Manager.tmdl")
        );
        // No diagramLayout.json in this project: the Model view lays the tables out itself.
        assert_eq!(
            invoke(&webview, "model_diagram", json!({ "docId": "d1" })),
            Ok(Value::Null)
        );

        let detection = &views["detection"];
        assert_eq!(detection["format"], "pbip");
        assert_eq!(detection["confidence"], "certain");
        assert_eq!(array(detection, "reports").len(), 3);
        assert!(
            array(&views, "diagnostics")
                .iter()
                .all(|d| d["severity"] != "error"),
            "{}",
            views["diagnostics"]
        );

        // File → Recent lists the opened project once.
        for _ in 0..2 {
            invoke(&webview, "add_recent_file", json!({ "path": pbip })).unwrap();
        }
        let recent = invoke(&webview, "recent_files", json!({})).unwrap();
        assert_eq!(recent.as_array().unwrap().len(), 1, "{recent}");
        assert_eq!(recent[0]["fileName"], "AcmeOutfitters.pbip");
        assert_eq!(recent[0]["exists"], true);
        assert_eq!(
            invoke(&webview, "remove_recent_file", json!({ "path": null })),
            Ok(json!([]))
        );

        // Reopening gives a new document; the UI closes the old one.
        let again = open(&webview, &pbip);
        assert_eq!(again["summary"]["docId"], "d2");
        let closed = run(&webview, "doc.close", json!({ "docId": "d1" }));
        assert_eq!(closed["closed"], true);
        let err = invoke(&webview, "document_views", json!({ "docId": "d1" })).unwrap_err();
        assert_eq!(err["kind"], "noDocument");
        assert_eq!(
            array(&run(&webview, "doc.list", json!({})), "documents").len(),
            1
        );
    }

    /// Copies a folder recursively.
    fn copy_dir(from: &Path, to: &Path) {
        std::fs::create_dir_all(to).unwrap();
        for entry in std::fs::read_dir(from).unwrap() {
            let entry = entry.unwrap();
            let target = to.join(entry.file_name());
            if entry.file_type().unwrap().is_dir() {
                copy_dir(&entry.path(), &target);
            } else {
                std::fs::copy(entry.path(), &target).unwrap();
            }
        }
    }

    /// The example's Overview report connected to a semantic model in the service
    /// (`byConnection`), as a report downloaded from the service is.
    #[test]
    fn live_connected_report_has_a_remote_model() {
        let Some(source) = fixture(&format!("{ACME}/Acme Outfitters - Overview.Report")) else {
            return;
        };
        let folder = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/dashcraft-app-tests/remote/Remote.Report");
        let _ = std::fs::remove_dir_all(&folder);
        copy_dir(&source, &folder);
        std::fs::write(
            folder.join("definition.pbir"),
            json!({
                "version": "4.0",
                "datasetReference": {"byConnection": {"connectionString":
                    "Data Source=\"powerbi://api.powerbi.com/v1.0/acme-workspace/Acme Outfitters\";initial catalog=AcmeOutfitters"}}
            })
            .to_string(),
        )
        .unwrap();
        let (_app, webview) = mock_app(StartupPath::default());

        let views = open(&webview, &folder);
        let summary = &views["summary"];
        assert_eq!(summary["entryKind"], "directory");
        assert_eq!(summary["modelStatus"]["availability"], "remote");
        assert_eq!(summary["counts"]["tables"], 0);
        assert_eq!(views["tree"]["status"]["availability"], "remote");
        assert!(array(&views["tree"], "tables").is_empty());
        let reports = array(&views, "reports");
        assert_eq!(reports[0]["dataset"]["kind"], "byConnection");
        assert_eq!(array(&reports[0], "pages").len(), 3);
        let err = invoke(&webview, "tmdl_view", json!({ "docId": "d1" })).unwrap_err();
        assert_eq!(err["kind"], "unsupported");
        let err = ops(&webview, "measures.values", json!({ "docId": "d1" })).unwrap_err();
        assert_eq!(err["code"], "noDocument", "no local model");
        let _ = std::fs::remove_dir_all(&folder);
    }

    /// Records the payloads of `ops://progress` events.
    fn record_progress(
        app: &tauri::App<tauri::test::MockRuntime>,
    ) -> std::sync::Arc<std::sync::Mutex<Vec<Value>>> {
        use tauri::Listener;
        let events = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let sink = std::sync::Arc::clone(&events);
        app.listen_any("ops://progress", move |event| {
            let payload: Value = serde_json::from_str(event.payload()).unwrap();
            sink.lock().unwrap().push(payload);
        });
        events
    }

    /// The app adapter end to end on the example: a blob map chosen after opening applies to
    /// the next refresh (the workspace's open hook), refresh progress arrives as
    /// `ops://progress` events of the request, and the canvas, grid and measures run through
    /// the registry with the expected values. Domain behaviour is covered in depth by
    /// `crates/pbi-ops/tests` (e2e.rs, canvas.rs).
    #[test]
    fn refreshes_and_renders_through_the_registry() {
        let (Some(pbip), Some(blob_map), Some(expected)) = (
            fixture(&format!("{ACME}/AcmeOutfitters.pbip")),
            fixture(&format!("{ACME}/blob-map.json")),
            expected(),
        ) else {
            return;
        };
        let (app, webview) = mock_app(StartupPath::default());
        let events = record_progress(&app);
        let views = open(&webview, &pbip);
        let doc = views["summary"]["docId"].clone();

        let status = run(&webview, "data.tables", json!({ "docId": doc }));
        assert!(
            array(&status, "tables")
                .iter()
                .all(|t| t["loaded"] == false)
        );
        let err = invoke(&webview, "set_blob_map", json!({ "path": "missing.json" })).unwrap_err();
        assert_eq!(err["kind"], "notFound");
        let map = invoke(&webview, "set_blob_map", json!({ "path": blob_map })).unwrap();
        assert!(map["path"].as_str().unwrap().ends_with("blob-map.json"));
        assert_eq!(invoke(&webview, "blob_map", json!({})).unwrap(), map);

        let report = ops_with(
            &webview,
            "refresh-1",
            "data.refresh",
            json!({ "docId": doc, "source": "query" }),
        )
        .unwrap();
        assert_eq!(report["ok"], true, "{report}");
        {
            let events = events.lock().unwrap();
            assert!(!events.is_empty());
            assert!(
                events
                    .iter()
                    .all(|e| e["requestId"] == "refresh-1" && e["command"] == "data.refresh")
            );
            let kinds: Vec<&Value> = events
                .iter()
                .filter_map(|e| e["data"].get("event"))
                .collect();
            assert_eq!(kinds.first(), Some(&&json!("started")));
            assert_eq!(kinds.last(), Some(&&json!("finished")));
            assert!(events.iter().any(|e| e["data"]["event"] == "tableFinished"
                && e["data"]["table"]["name"] == "Web Sessions"));
            assert_eq!(events.last().unwrap()["fraction"], json!(1.0));
        }
        let rows = &expected["rowCounts"]["Orders"];
        let status = run(&webview, "data.tables", json!({ "docId": doc }));
        let orders = array(&status, "tables")
            .iter()
            .find(|t| t["name"] == "Orders")
            .unwrap();
        assert_eq!((&orders["rows"], &orders["loaded"]), (rows, &json!(true)));

        // The grid's page request.
        let page = run(
            &webview,
            "data.page",
            json!({ "docId": doc, "table": "Orders", "offset": 0, "limit": 50,
                    "formatted": true, "blankAsNull": true }),
        );
        assert_eq!(&page["totalRows"], rows);
        assert_eq!(array(&page, "rows").len(), 50);

        // The canvas and the Measures panel agree on the Overview report's first page
        // (its report filter and saved Year slicer).
        let overview = &array(&views, "reports")[0];
        let summary_page = &expected["reports"][0]["pages"][0];
        let card = array(summary_page, "cards")
            .iter()
            .find(|c| c["measure"] == "Total Sales")
            .unwrap()["formatted"]
            .as_str()
            .unwrap();
        let rendered = run(
            &webview,
            "canvas.render",
            json!({ "docId": doc, "report": overview["name"],
                    "page": array(overview, "pages")[0]["name"] }),
        );
        assert!(
            rendered["svg"]
                .as_str()
                .unwrap()
                .contains(&format!(">{card}<"))
        );
        let measures = run(
            &webview,
            "measures.values",
            json!({ "docId": doc, "filters": rendered["effectiveFilters"]["filters"] }),
        );
        let total = array(&measures, "measures")
            .iter()
            .find(|m| m["name"] == "Total Sales")
            .unwrap();
        assert_eq!(total["display"], card);
        let filters = run(
            &webview,
            "report.filters",
            json!({ "docId": doc, "report": overview["name"] }),
        );
        assert_eq!(array(&filters["filters"], "filters").len(), 1, "{filters}");
    }

    #[test]
    fn os_open_request_wins_over_argv_once() {
        let startup = StartupPath::from_args([OsString::from("/tmp/argv.pbix")]);
        let (app, webview) = mock_app(startup);
        app.state::<PendingOpen>()
            .set(PathBuf::from("/tmp/finder.pbix"));
        let first = invoke(&webview, "startup_path", json!({})).unwrap();
        assert!(first.as_str().unwrap().ends_with("finder.pbix"));
        // Handed out once; afterwards the command-line path is reported again.
        let second = invoke(&webview, "startup_path", json!({})).unwrap();
        assert!(second.as_str().unwrap().ends_with("argv.pbix"));
    }

    #[test]
    fn startup_path_takes_first_positional_arg() {
        let args = ["-psn_0_12345", "report.pbix", "other.pbix"].map(OsString::from);
        let startup = StartupPath::from_args(args);
        let path = startup.get().unwrap();
        assert!(path.is_absolute());
        assert!(path.ends_with("report.pbix"));
    }

    #[test]
    fn startup_path_is_none_without_positional_args() {
        let startup = StartupPath::from_args(["--verbose"].map(OsString::from));
        assert!(startup.get().is_none());
        assert!(
            StartupPath::from_args(Vec::<OsString>::new())
                .get()
                .is_none()
        );
    }
}
