//! PBIR report folder reader.

use indexmap::IndexMap;
use pbi_model::{Diagnostic, Origin};
use serde::de::DeserializeOwned;

use crate::error::{PbirError, codes};
use crate::json::strip_bom;
use crate::pbir::{
    DefinitionPbir, PageJson, PagesJson, Platform, ReportJson, ThemeMetadataJson, VersionJson,
    VisualContainerJson,
};
use crate::source::FileSource;
use crate::theme::ThemeJson;

/// `definition.pbir` (report root).
pub const DEFINITION_PBIR: &str = "definition.pbir";
/// `.platform` (report root).
pub const PLATFORM: &str = ".platform";
/// `definition/report.json`.
pub const REPORT_JSON: &str = "definition/report.json";
/// `definition/version.json`.
pub const VERSION_JSON: &str = "definition/version.json";
/// `definition/pages/pages.json`.
pub const PAGES_JSON: &str = "definition/pages/pages.json";

const PAGES_DIR: &str = "definition/pages/";

/// What a path inside a report folder holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PbirFileKind {
    DefinitionPbir,
    Platform,
    Report,
    Version,
    Pages,
    /// `definition/pages/<page>/page.json`.
    Page {
        page: String,
    },
    /// `definition/pages/<page>/visuals/<visual>/visual.json`.
    Visual {
        page: String,
        visual: String,
    },
    /// Anything else (themes, images, bookmarks, `mobile.json`, …).
    Other,
}

impl PbirFileKind {
    /// Classifies a report-relative, `/`-separated path.
    pub fn of(path: &str) -> Self {
        match path {
            DEFINITION_PBIR => return Self::DefinitionPbir,
            PLATFORM => return Self::Platform,
            REPORT_JSON => return Self::Report,
            VERSION_JSON => return Self::Version,
            PAGES_JSON => return Self::Pages,
            _ => {}
        }
        let Some(rest) = path.strip_prefix(PAGES_DIR) else {
            return Self::Other;
        };
        let parts: Vec<&str> = rest.split('/').collect();
        match parts.as_slice() {
            [page, "page.json"] if !page.is_empty() => Self::Page {
                page: (*page).to_owned(),
            },
            [page, "visuals", visual, "visual.json"] if !page.is_empty() && !visual.is_empty() => {
                Self::Visual {
                    page: (*page).to_owned(),
                    visual: (*visual).to_owned(),
                }
            }
            _ => Self::Other,
        }
    }
}

/// A file of the report folder, verbatim.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PbirFile {
    pub bytes: Vec<u8>,
    /// `true` if the file was parsed into a typed struct of the
    /// [`PbirReport`] (the writer then serializes the struct instead).
    pub parsed: bool,
}

/// A page and its visuals.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct PbirPage {
    /// Folder name under `definition/pages/` (normally the page `name`).
    pub dir: String,
    pub page: PageJson,
    /// Visuals sorted by folder name.
    pub visuals: Vec<PbirVisual>,
}

/// A visual container.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct PbirVisual {
    /// Folder name under `visuals/` (normally the visual `name`).
    pub dir: String,
    pub visual: VisualContainerJson,
}

/// A theme file referenced by `report.json`.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct PbirTheme {
    /// Report-relative path, e.g.
    /// `StaticResources/SharedResources/BaseThemes/CY24SU10.json`.
    pub path: String,
    pub theme: ThemeJson,
}

/// A PBIR report folder: typed files plus every file verbatim.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct PbirReport {
    pub definition_pbir: Option<DefinitionPbir>,
    pub platform: Option<Platform>,
    pub report: Option<ReportJson>,
    pub version: Option<VersionJson>,
    pub pages_meta: Option<PagesJson>,
    /// Pages in `pageOrder`, then unlisted pages by folder name.
    pub pages: Vec<PbirPage>,
    pub themes: Vec<PbirTheme>,
    /// Every file of the folder (sorted by path), for lossless writing.
    pub files: IndexMap<String, PbirFile>,
    /// Problems found while reading.
    pub diagnostics: Vec<Diagnostic>,
}

impl PbirPage {
    /// `definition/pages/<dir>/page.json`.
    pub fn path(&self) -> String {
        format!("{PAGES_DIR}{}/page.json", self.dir)
    }

    /// `definition/pages/<dir>/visuals/<visual dir>/visual.json`.
    pub fn visual_path(&self, visual: &PbirVisual) -> String {
        format!("{PAGES_DIR}{}/visuals/{}/visual.json", self.dir, visual.dir)
    }
}

impl PbirReport {
    /// Theme file of a `themeCollection` entry.
    pub fn theme_for(&self, meta: &ThemeMetadataJson) -> Option<&ThemeJson> {
        let path = self
            .report
            .as_ref()?
            .resource_path(meta.name.as_deref()?, meta.package_type.as_deref())?;
        self.themes
            .iter()
            .find(|t| t.path == path)
            .map(|t| &t.theme)
    }

    /// The base theme (`themeCollection.baseTheme`).
    pub fn base_theme(&self) -> Option<&ThemeJson> {
        let meta = self
            .report
            .as_ref()?
            .theme_collection
            .as_ref()?
            .base_theme
            .as_ref()?;
        self.theme_for(meta)
    }

    /// The custom theme (`themeCollection.customTheme`).
    pub fn custom_theme(&self) -> Option<&ThemeJson> {
        let meta = self
            .report
            .as_ref()?
            .theme_collection
            .as_ref()?
            .custom_theme
            .as_ref()?;
        self.theme_for(meta)
    }

    /// Page by `name` (or folder name).
    pub fn page(&self, name: &str) -> Option<&PbirPage> {
        self.pages
            .iter()
            .find(|p| p.page.name.as_deref() == Some(name) || p.dir == name)
    }

    /// Parses the file at `path` into `T`, marking it parsed; on failure
    /// records a diagnostic and returns `None`.
    fn parse_file<T: DeserializeOwned>(&mut self, path: &str) -> Option<T> {
        let file = self.files.get_mut(path)?;
        match parse_json::<T>(&file.bytes) {
            Ok(value) => {
                file.parsed = true;
                Some(value)
            }
            Err(e) => {
                self.diagnostics.push(
                    Diagnostic::error(codes::INVALID_FILE, format!("{path}: {e}"))
                        .with_origin(Origin::file(path)),
                );
                None
            }
        }
    }
}

/// Parses UTF-8 JSON (an optional BOM is skipped).
pub(crate) fn parse_json<T: DeserializeOwned>(bytes: &[u8]) -> Result<T, serde_json::Error> {
    serde_json::from_slice(strip_bom(bytes))
}

/// Reads a PBIR report folder.
///
/// Fails only if the source holds no PBIR definition at all. Invalid or
/// unexpected files become diagnostics in [`PbirReport::diagnostics`]; every
/// file is kept verbatim in [`PbirReport::files`].
pub fn read_pbir(files: &impl FileSource) -> Result<PbirReport, PbirError> {
    let mut paths = files.list("");
    paths.sort();
    paths.dedup();
    let has_definition = paths
        .iter()
        .any(|p| p == REPORT_JSON || p.starts_with(PAGES_DIR));
    if !has_definition {
        return Err(PbirError::NotPbir(format!(
            "no {REPORT_JSON} or {PAGES_DIR} found"
        )));
    }

    let mut out = PbirReport::default();
    for path in paths {
        if let Some(bytes) = files.read(&path) {
            out.files.insert(
                path,
                PbirFile {
                    bytes,
                    parsed: false,
                },
            );
        }
    }

    out.definition_pbir = out.parse_file(DEFINITION_PBIR);
    out.platform = out.parse_file(PLATFORM);
    out.report = out.parse_file(REPORT_JSON);
    out.version = out.parse_file(VERSION_JSON);
    out.pages_meta = out.parse_file(PAGES_JSON);
    if !out.files.contains_key(REPORT_JSON) {
        out.diagnostics.push(
            Diagnostic::warning(codes::MISSING_FILE, format!("{REPORT_JSON} is missing"))
                .with_origin(Origin::file(REPORT_JSON)),
        );
    }

    read_pages(&mut out);
    read_themes(&mut out);
    Ok(out)
}

fn read_pages(out: &mut PbirReport) {
    let mut page_dirs: Vec<String> = Vec::new();
    let mut visual_paths: IndexMap<String, Vec<(String, String)>> = IndexMap::new();
    for path in out.files.keys() {
        match PbirFileKind::of(path) {
            PbirFileKind::Page { page } => page_dirs.push(page),
            PbirFileKind::Visual { page, visual } => visual_paths
                .entry(page)
                .or_default()
                .push((visual, path.clone())),
            _ => {}
        }
    }

    let mut pages: Vec<PbirPage> = Vec::new();
    for dir in page_dirs {
        let path = format!("{PAGES_DIR}{dir}/page.json");
        let Some(page) = out.parse_file::<PageJson>(&path) else {
            continue;
        };
        let mut visuals = Vec::new();
        for (vdir, vpath) in visual_paths.shift_remove(&dir).unwrap_or_default() {
            if let Some(visual) = out.parse_file::<VisualContainerJson>(&vpath) {
                visuals.push(PbirVisual { dir: vdir, visual });
            }
        }
        pages.push(PbirPage { dir, page, visuals });
    }
    for (dir, _) in visual_paths {
        out.diagnostics.push(
            Diagnostic::warning(
                codes::MISSING_FILE,
                format!("visuals under {PAGES_DIR}{dir}/ have no page.json"),
            )
            .with_origin(Origin::file(format!("{PAGES_DIR}{dir}/page.json"))),
        );
    }

    let order: Vec<String> = out
        .pages_meta
        .as_ref()
        .and_then(|m| m.page_order.clone())
        .unwrap_or_default();
    let has_order = out
        .pages_meta
        .as_ref()
        .is_some_and(|m| m.page_order.is_some());
    let mut ordered = Vec::with_capacity(pages.len());
    for name in &order {
        let pos = pages.iter().position(|p| p.dir == *name).or_else(|| {
            pages
                .iter()
                .position(|p| p.page.name.as_deref() == Some(name))
        });
        match pos {
            Some(i) => ordered.push(pages.remove(i)),
            None => out.diagnostics.push(
                Diagnostic::warning(
                    codes::MISSING_FILE,
                    format!("page `{name}` is listed in pageOrder but has no valid page.json"),
                )
                .with_origin(Origin::file(PAGES_JSON).with_pointer("/pageOrder")),
            ),
        }
    }
    for page in pages {
        if has_order {
            out.diagnostics.push(
                Diagnostic::info(
                    codes::UNLISTED_PAGE,
                    format!("page `{}` is not listed in pageOrder", page.dir),
                )
                .with_origin(Origin::file(page.path())),
            );
        }
        ordered.push(page);
    }
    out.pages = ordered;
}

fn read_themes(out: &mut PbirReport) {
    let Some(report) = &out.report else {
        return;
    };
    let mut wanted: Vec<String> = Vec::new();
    for package in report.resource_packages.iter().flatten() {
        let Some(package_name) = package.name.as_deref() else {
            continue;
        };
        for item in package.items.iter().flatten() {
            let is_theme = item
                .item_type
                .as_deref()
                .is_some_and(|t| t.ends_with("Theme"));
            if let (true, Some(path)) = (is_theme, item.path.as_deref()) {
                let full = format!("StaticResources/{package_name}/{path}");
                if !wanted.contains(&full) {
                    wanted.push(full);
                }
            }
        }
    }
    for path in wanted {
        if !out.files.contains_key(&path) {
            out.diagnostics.push(
                Diagnostic::warning(codes::THEME, format!("theme file {path} is missing"))
                    .with_origin(Origin::file(REPORT_JSON).with_pointer("/resourcePackages")),
            );
            continue;
        }
        if let Some(theme) = out.parse_file::<ThemeJson>(&path) {
            out.themes.push(PbirTheme { path, theme });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_paths() {
        assert_eq!(
            PbirFileKind::of("definition/report.json"),
            PbirFileKind::Report
        );
        assert_eq!(
            PbirFileKind::of("definition/pages/p1/page.json"),
            PbirFileKind::Page { page: "p1".into() }
        );
        assert_eq!(
            PbirFileKind::of("definition/pages/p1/visuals/v1/visual.json"),
            PbirFileKind::Visual {
                page: "p1".into(),
                visual: "v1".into()
            }
        );
        assert_eq!(
            PbirFileKind::of("definition/pages/p1/visuals/v1/mobile.json"),
            PbirFileKind::Other
        );
        assert_eq!(
            PbirFileKind::of("definition/pages//page.json"),
            PbirFileKind::Other
        );
    }
}
