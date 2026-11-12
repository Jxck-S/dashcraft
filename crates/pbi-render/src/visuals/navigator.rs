//! Page and bookmark navigators (`pageNavigator`, `bookmarkNavigator`): one
//! button per page or bookmark, the current page (the bookmark applied last)
//! in the selected state. Each button is a
//! [`HitKind::NavItem`](crate::scene::HitKind::NavItem) hit region whose
//! index [`item_action`] turns into a page navigation or a bookmark.
//! (owner: visuals-basics)
//!
//! # Items
//!
//! - Pages: the report's pages in order, without hidden pages
//!   (`pages.showHiddenPages`, default `false`) and tooltip pages
//!   (`pages.showTooltipPages`, default `false`).
//! - Bookmarks: the bookmarks of the group `bookmarks.bookmarkGroup`
//!   (internal or display name; empty: every bookmark), without the
//!   deselection bookmark when `bookmarks.hideDeselectedBookmark`.
//!
//! # Formatting objects
//!
//! | Object.property | Meaning | Default |
//! |---|---|---|
//! | `layout.orientation` | `2` horizontal, `1` vertical, `0` grid | `2` |
//! | `layout.rowCount` / `columnCount` | grid size | √n columns |
//! | `layout.cellPadding` | gap between buttons (px) | `4` |
//! | `shape`, `fill`, `outline`, `text` (default selector) | every button, as in [`super::button`] | light fill, gray outline, label text |
//! | `fill.fillColor`, `text.fontColor`, `outline.lineColor` (`"$id": "selected"`) | the selected button | theme foreground fill, theme background text |
//! | `bookmarks.selectedBookmark` | the selected bookmark before any is applied | none |
//!
//! Per-page `pages.showPage`, hover and disabled states are not read.

use pbi_model::{Page, Report, Visual};
use serde::Serialize;
use serde_json::Value;

use crate::layout::TextAlign;
use crate::scene::{Clip, Color, Group, HitId, Paint, Rect, RectNode, TextAnchor};
use crate::theme::{PropLookup, ThemeResolver};

use super::VisualCtx;
use super::button::{ButtonAction, button_shape};
use super::common::{ellipsize, family_metrics, text_node};

/// What navigators can list: the report's pages and bookmarks, the page
/// shown and the bookmark applied last.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Navigation {
    /// Internal name of the page being rendered.
    pub current_page: String,
    pub pages: Vec<NavPage>,
    /// Bookmarks in pane order, groups flattened.
    pub bookmarks: Vec<NavBookmark>,
    /// Internal name of the bookmark applied last, if any.
    pub active_bookmark: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct NavPage {
    pub name: String,
    pub display_name: String,
    pub hidden: bool,
    pub tooltip: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct NavBookmark {
    pub name: String,
    pub display_name: String,
    /// Internal and display name of its group.
    pub group: Option<(String, String)>,
}

/// An empty [`Navigation`] (no pages, no bookmarks), for renders outside a
/// report.
pub static EMPTY: Navigation = Navigation {
    current_page: String::new(),
    pages: Vec::new(),
    bookmarks: Vec::new(),
    active_bookmark: None,
};

impl Navigation {
    /// The navigation of `report` while `page` is shown.
    pub fn new(report: &Report, page: &Page, active_bookmark: Option<&str>) -> Self {
        let pages = report
            .pages
            .iter()
            .map(|p| NavPage {
                name: p.name.clone(),
                display_name: p.display_name.clone(),
                hidden: p.is_hidden,
                tooltip: is_tooltip_page(p),
            })
            .collect();
        let mut bookmarks = Vec::new();
        for b in &report.bookmarks {
            if b.is_group() {
                for c in &b.children {
                    bookmarks.push(NavBookmark {
                        name: c.name.clone(),
                        display_name: c.display_name.clone(),
                        group: Some((b.name.clone(), b.display_name.clone())),
                    });
                }
            } else {
                bookmarks.push(NavBookmark {
                    name: b.name.clone(),
                    display_name: b.display_name.clone(),
                    group: None,
                });
            }
        }
        Self {
            current_page: page.name.clone(),
            pages,
            bookmarks,
            active_bookmark: active_bookmark.map(str::to_owned),
        }
    }
}

/// A tooltip page (`type` or `pageBinding.type` is `Tooltip`).
fn is_tooltip_page(page: &Page) -> bool {
    let is_tooltip = |t: Option<&str>| t.is_some_and(|t| t.eq_ignore_ascii_case("tooltip"));
    is_tooltip(page.extra.get("type").and_then(|v| v.as_str()))
        || page
            .extra
            .get("pageBinding")
            .is_some_and(|b| is_tooltip(b.to_json().get("type").and_then(Value::as_str)))
}

/// One button of a navigator.
#[derive(Debug, Clone, PartialEq)]
pub struct NavItem {
    pub label: String,
    pub selected: bool,
    pub action: ButtonAction,
}

/// The buttons of navigator `visual` (empty for other visual types).
pub fn items(visual: &Visual, theme: &ThemeResolver, nav: &Navigation) -> Vec<NavItem> {
    let o = theme.visual(visual).objects();
    match visual.visual_type.as_str() {
        "pageNavigator" => {
            let hidden = o.bool("pages", "showHiddenPages").unwrap_or(false);
            let tooltips = o.bool("pages", "showTooltipPages").unwrap_or(false);
            nav.pages
                .iter()
                .filter(|p| (hidden || !p.hidden) && (tooltips || !p.tooltip))
                .map(|p| NavItem {
                    label: p.display_name.clone(),
                    selected: p.name == nav.current_page,
                    action: ButtonAction::PageNavigation {
                        page: p.name.clone(),
                    },
                })
                .collect()
        }
        "bookmarkNavigator" => {
            let group = o
                .text("bookmarks", "bookmarkGroup")
                .map(|g| g.trim().to_owned())
                .filter(|g| !g.is_empty());
            let hide = o
                .bool("bookmarks", "hideDeselectedBookmark")
                .unwrap_or(false)
                .then(|| o.text("bookmarks", "deselectionBookmark"))
                .flatten();
            let active = nav
                .active_bookmark
                .clone()
                .or_else(|| o.text("bookmarks", "selectedBookmark"));
            nav.bookmarks
                .iter()
                .filter(|b| match &group {
                    None => true,
                    Some(g) => b.group.as_ref().is_some_and(|(name, display)| {
                        name == g || display.eq_ignore_ascii_case(g)
                    }),
                })
                .filter(|b| hide.as_deref() != Some(b.name.as_str()))
                .map(|b| NavItem {
                    label: b.display_name.clone(),
                    selected: active.as_deref() == Some(b.name.as_str()),
                    action: ButtonAction::Bookmark {
                        bookmark: b.name.clone(),
                    },
                })
                .collect()
        }
        _ => Vec::new(),
    }
}

/// What clicking button `index` of navigator `visual` does: go to its page
/// or apply its bookmark. A selected bookmark with
/// `bookmarks.allowDeselectionBookmark` launches
/// `bookmarks.deselectionBookmark` instead (nothing without one).
pub fn item_action(
    visual: &Visual,
    theme: &ThemeResolver,
    nav: &Navigation,
    index: usize,
) -> Option<ButtonAction> {
    let item = items(visual, theme, nav).into_iter().nth(index)?;
    if item.selected && matches!(item.action, ButtonAction::Bookmark { .. }) {
        let o = theme.visual(visual).objects();
        if o.bool("bookmarks", "allowDeselectionBookmark")
            .unwrap_or(false)
        {
            return o
                .text("bookmarks", "deselectionBookmark")
                .map(|b| b.trim().to_owned())
                .filter(|b| !b.is_empty())
                .map(|bookmark| ButtonAction::Bookmark { bookmark });
        }
    }
    Some(item.action)
}

/// A color of the `"$id": "selected"` entry of `object` in the visual's
/// `objects`.
fn selected_color(ctx: &VisualCtx<'_>, object: &str, prop: &str) -> Option<Color> {
    pbi_pbir::objects::entries(&ctx.visual.objects, object)
        .filter(|(_, selector)| {
            selector
                .and_then(|s| s.get("id"))
                .and_then(Value::as_str)
                .is_some_and(|id| id == "selected")
        })
        .find_map(|(props, _)| props.get(prop))
        .and_then(|v| ctx.theme.color(v))
}

/// The cells of `n` buttons in `area` (see the module docs).
fn cells(o: &PropLookup<'_>, area: Rect, n: usize) -> Vec<Rect> {
    if n == 0 || area.is_empty() {
        return Vec::new();
    }
    let count = |prop: &str| {
        o.number("layout", prop)
            .filter(|v| v.is_finite() && *v >= 1.0)
            .map(|v| v.min(1000.0) as usize)
    };
    let (rows, cols) = match o.number("layout", "orientation").map(|v| v.round() as i64) {
        Some(1) => (n, 1),
        Some(0) => {
            let cols = count("columnCount")
                .or_else(|| count("rowCount").map(|r| n.div_ceil(r)))
                .unwrap_or_else(|| (n as f64).sqrt().ceil() as usize)
                .clamp(1, n);
            (n.div_ceil(cols), cols)
        }
        _ => (1, n),
    };
    let gap = o
        .number("layout", "cellPadding")
        .filter(|v| v.is_finite())
        .unwrap_or(4.0)
        .clamp(0.0, 100.0);
    let w = ((area.width - gap * (cols - 1) as f64) / cols as f64).max(0.0);
    let h = ((area.height - gap * (rows - 1) as f64) / rows as f64).max(0.0);
    (0..n)
        .map(|i| {
            let (r, c) = (i / cols, i % cols);
            Rect::new(
                area.x + c as f64 * (w + gap),
                area.y + r as f64 * (h + gap),
                w,
                h,
            )
        })
        .collect()
}

/// Renders the navigator (see the module docs).
pub fn render(ctx: &VisualCtx<'_>) -> Option<Group> {
    let area = ctx.frame.content;
    if area.is_empty() {
        return None;
    }
    let props = ctx.theme.visual(ctx.visual);
    let o = props.objects();
    let mut g = Group::new()
        .with_class("pbi-navigator")
        .with_clip(Clip::rect(ctx.frame.inner));
    let items = items(ctx.visual, ctx.theme, ctx.nav);
    let text_on = o.bool("text", "show").unwrap_or(true);
    let base = o.text_style("text", "label");
    let selected_fill = selected_color(ctx, "fill", "fillColor").unwrap_or(ctx.theme.foreground());
    let selected_text = selected_color(ctx, "text", "fontColor").unwrap_or(ctx.theme.background());
    let selected_line = selected_color(ctx, "outline", "lineColor");
    let align = o
        .text("text", "horizontalAlignment")
        .map_or(TextAlign::Center, |a| TextAlign::parse(&a));
    for (i, (item, cell)) in items.iter().zip(cells(&o, area, items.len())).enumerate() {
        if cell.is_empty() {
            continue;
        }
        let mut b = Group::new()
            .with_id(HitId::nav_item(ctx.visual.name.clone(), i))
            .with_class(if item.selected {
                "pbi-nav-item pbi-selected"
            } else {
                "pbi-nav-item"
            });
        b.push(RectNode::new(cell).fill(Paint::new(Color::BLACK, 0.0)));
        let (fill, line) = if item.selected {
            (Some(selected_fill), selected_line)
        } else {
            (None, None)
        };
        button_shape(ctx, &o, cell, true, fill, line, &mut b);
        if text_on {
            let mut style = base.clone();
            if item.selected {
                style.color = selected_text;
            }
            let m = family_metrics(&style);
            let inner = cell.inset_uniform(4.0);
            if inner.width > 0.0 && inner.height >= m.line_height_px(&style) * 0.5 {
                let t = ellipsize(ctx.fonts, &item.label, &style, inner.width);
                let (x, anchor) = match align {
                    TextAlign::Left => (inner.x, TextAnchor::Start),
                    TextAlign::Center => (inner.x + inner.width / 2.0, TextAnchor::Middle),
                    TextAlign::Right => (inner.right(), TextAnchor::End),
                };
                let top = inner.y + (inner.height - m.line_height_px(&style)) / 2.0;
                b.push(text_node(
                    x,
                    top + m.ascent_px(&style),
                    t,
                    &style,
                    anchor,
                    Some(inner),
                ));
            }
        }
        g.push(b);
    }
    Some(g)
}
