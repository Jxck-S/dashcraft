//! Report loading shared by the pbi-render layout tests: the PBIR reports
//! of the Acme Outfitters example (`examples/acme-outfitters/`, committed)
//! read without data.
#![allow(dead_code, clippy::expect_used, clippy::unwrap_used)]

use std::path::{Path, PathBuf};

use pbi_model::{Page, Report};
use pbi_pbir::{FileSource, PbirReport};
use pbi_render::theme::ThemeResolver;

/// Overview report, page "Executive Summary" (FitToPage, 1280×720).
pub const EXECUTIVE_SUMMARY: &str = "ef796b39c816a4afa4f1";
/// Overview report, page "Product Detail" (FitToWidth, 1280×1080).
pub const PRODUCT_DETAIL: &str = "59126dbf5023ae95f4dc";
/// Stores report, page "Store Performance" (FitToPage, 1280×720).
pub const STORE_PERFORMANCE: &str = "ef3ce6b6fadf22960137";
/// Stores report, page "Store Detail" (FitToWidth, hidden, 1280×900).
pub const STORE_DETAIL: &str = "50c645140e8e10eea677";
/// Stores report, page "Store Tooltip" (ActualSize tooltip page, 320×240).
pub const STORE_TOOLTIP: &str = "58db5b3f75d059fec1fb";

/// `$PBIVIEW_REFERENCE`, else `<workspace>/reference`.
pub fn reference_root() -> PathBuf {
    std::env::var_os("PBIVIEW_REFERENCE").map_or_else(
        || Path::new(env!("CARGO_MANIFEST_DIR")).join("../../reference"),
        PathBuf::from,
    )
}

/// A plain directory [`FileSource`] (tests only; `pbi-core` owns the
/// guarded one).
pub struct DirSource(pub PathBuf);

impl DirSource {
    fn walk(dir: &Path, root: &Path, out: &mut Vec<String>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                Self::walk(&path, root, out);
            } else if let Ok(rel) = path.strip_prefix(root) {
                let parts: Vec<String> = rel
                    .components()
                    .map(|c| c.as_os_str().to_string_lossy().into_owned())
                    .collect();
                out.push(parts.join("/"));
            }
        }
    }
}

impl FileSource for DirSource {
    fn read(&self, path: &str) -> Option<Vec<u8>> {
        std::fs::read(self.0.join(path)).ok()
    }

    fn list(&self, prefix: &str) -> Vec<String> {
        let mut out = Vec::new();
        Self::walk(&self.0, &self.0, &mut out);
        out.retain(|p| p.starts_with(prefix));
        out
    }
}

/// A loaded fixture report.
pub struct Fixture {
    pub pbir: PbirReport,
    pub report: Report,
    pub theme: ThemeResolver,
}

impl Fixture {
    pub fn page(&self, name: &str) -> &Page {
        self.report
            .page(name)
            .unwrap_or_else(|| panic!("page {name} missing"))
    }

    /// The first page (the report's `pageOrder`).
    pub fn first_page(&self) -> &Page {
        self.report.pages.first().expect("a page")
    }
}

/// Loads `<root>/examples/acme-outfitters/<rel>` (a PBIR report folder), or
/// prints `SKIP` and returns `None` if it is missing.
pub fn load(rel: &str) -> Option<Fixture> {
    let dir = reference_root().join("examples/acme-outfitters").join(rel);
    if !dir.join("definition").is_dir() {
        println!("SKIP: {} missing", dir.display());
        return None;
    }
    let pbir = pbi_pbir::read_pbir(&DirSource(dir)).expect("read PBIR fixture");
    let (report, _diagnostics) = pbi_pbir::to_report_ast(&pbir);
    let theme = ThemeResolver::from_pbir(&pbir);
    Some(Fixture {
        pbir,
        report,
        theme,
    })
}

pub fn overview_report() -> Option<Fixture> {
    load("Acme Outfitters - Overview.Report")
}

pub fn stores_report() -> Option<Fixture> {
    load("Acme Outfitters - Stores.Report")
}

pub fn online_report() -> Option<Fixture> {
    load("Acme Outfitters - Online.Report")
}
