//! Bookmarks: PBIR `definition/bookmarks/` and the legacy `config.bookmarks`
//! of a PBIX `Layout`, read into [`pbi_model::Bookmark`]s in pane order.
//!
//! The source files stay verbatim in [`crate::PbirReport::files`]; this only
//! reads them.

use indexmap::IndexMap;
use pbi_model::{Bookmark, Origin};
use serde_json::Value;

use crate::convert::Ctx;
use crate::error::codes;
use crate::reader::PbirFile;

/// Folder of the bookmark files.
pub const BOOKMARKS_DIR: &str = "definition/bookmarks/";
/// Order and groups of the Bookmarks pane.
pub const BOOKMARKS_JSON: &str = "definition/bookmarks/bookmarks.json";
const BOOKMARK_SUFFIX: &str = ".bookmark.json";

/// The bookmarks of a PBIR report: `bookmarks.json` order and groups, then
/// bookmark files it does not list (by file name).
pub(crate) fn pbir_bookmarks(files: &IndexMap<String, PbirFile>, ctx: &mut Ctx) -> Vec<Bookmark> {
    let mut by_name: IndexMap<String, Bookmark> = IndexMap::new();
    for (path, file) in files {
        let Some(rest) = path.strip_prefix(BOOKMARKS_DIR) else {
            continue;
        };
        if rest.contains('/') || !rest.ends_with(BOOKMARK_SUFFIX) {
            continue;
        }
        let Some(json) = parse(path, &file.bytes, ctx) else {
            continue;
        };
        let mut b = bookmark(&json);
        if b.name.is_empty() {
            b.name = rest.trim_end_matches(BOOKMARK_SUFFIX).to_owned();
        }
        by_name.insert(b.name.clone(), b);
    }
    if by_name.is_empty() && !files.contains_key(BOOKMARKS_JSON) {
        return Vec::new();
    }

    let mut out = Vec::new();
    let meta = files
        .get(BOOKMARKS_JSON)
        .and_then(|f| parse(BOOKMARKS_JSON, &f.bytes, ctx));
    for item in meta
        .as_ref()
        .and_then(|m| m.get("items"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let name = str_of(item, "name");
        match item.get("children").and_then(Value::as_array) {
            Some(children) => {
                let children = children
                    .iter()
                    .filter_map(|c| {
                        let child = match c {
                            Value::String(s) => s.clone(),
                            other => str_of(other, "name"),
                        };
                        take(&mut by_name, &child, ctx)
                    })
                    .collect();
                out.push(Bookmark {
                    display_name: str_of(item, "displayName"),
                    name,
                    children,
                    ..Bookmark::default()
                });
            }
            None => out.extend(take(&mut by_name, &name, ctx)),
        }
    }
    out.extend(by_name.into_values());
    out
}

/// The bookmarks of a legacy layout's `config.bookmarks` (full objects,
/// groups with nested `children`).
pub(crate) fn legacy_bookmarks(value: Option<&Value>) -> Vec<Bookmark> {
    value
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|item| {
            let mut b = bookmark(item);
            b.children = item
                .get("children")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .map(bookmark)
                .collect();
            b
        })
        .collect()
}

fn bookmark(json: &Value) -> Bookmark {
    Bookmark {
        name: str_of(json, "name"),
        display_name: str_of(json, "displayName"),
        children: Vec::new(),
        options: json.get("options").cloned().unwrap_or(Value::Null),
        exploration_state: json.get("explorationState").cloned().unwrap_or(Value::Null),
    }
}

fn str_of(json: &Value, key: &str) -> String {
    json.get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned()
}

fn take(by_name: &mut IndexMap<String, Bookmark>, name: &str, ctx: &mut Ctx) -> Option<Bookmark> {
    let found = by_name.shift_remove(name);
    if found.is_none() {
        ctx.warn(
            codes::MISSING_FILE,
            format!("bookmark '{name}' is listed in bookmarks.json but has no file"),
            &Origin::file(BOOKMARKS_JSON),
        );
    }
    found
}

fn parse(path: &str, bytes: &[u8], ctx: &mut Ctx) -> Option<Value> {
    let text = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes);
    match serde_json::from_slice::<Value>(text) {
        Ok(v) if v.is_object() => Some(v),
        Ok(_) => {
            ctx.warn(
                codes::INVALID_FILE,
                "a bookmark file is not a JSON object".into(),
                &Origin::file(path),
            );
            None
        }
        Err(e) => {
            ctx.warn(
                codes::INVALID_FILE,
                format!("invalid bookmark JSON: {e}"),
                &Origin::file(path),
            );
            None
        }
    }
}
