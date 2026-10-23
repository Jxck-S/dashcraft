//! Chart legends: position, title, circle markers, overflow arrow.
//!
//! Vertical legends (`Left`/`Right`, `…Center` centred vertically) list one
//! item per row (row pitch ≈ 1.4 line heights, as the service draws them)
//! under a bold title, and take at most 40% of the width; horizontal
//! legends (`Top`/`Bottom`, `…Center` centred horizontally) put the title
//! and items on one row. Items that do not fit are replaced by a small
//! arrow (the service pages them).

use crate::scene::{Color, Group, Paint, PathBuilder, PathData, PathNode, Rect, TextAnchor};
use crate::text::{FontBook, TextStyle};
use crate::theme::VisualProps;

use super::labels::{baseline_centered, tailor, width};

/// Legend placement (`legend.position`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LegendPosition {
    Top,
    Bottom,
    Left,
    Right,
    TopCenter,
    BottomCenter,
    LeftCenter,
    RightCenter,
}

impl LegendPosition {
    /// Parses a `legend.position` value (case-insensitive); unknown → `Top`.
    pub fn parse(text: &str) -> Self {
        match text.trim().to_ascii_lowercase().as_str() {
            "bottom" => Self::Bottom,
            "left" => Self::Left,
            "right" => Self::Right,
            "topcenter" => Self::TopCenter,
            "bottomcenter" => Self::BottomCenter,
            "leftcenter" => Self::LeftCenter,
            "rightcenter" => Self::RightCenter,
            _ => Self::Top,
        }
    }

    pub fn is_vertical(self) -> bool {
        matches!(
            self,
            Self::Left | Self::Right | Self::LeftCenter | Self::RightCenter
        )
    }

    fn is_centered(self) -> bool {
        matches!(
            self,
            Self::TopCenter | Self::BottomCenter | Self::LeftCenter | Self::RightCenter
        )
    }
}

/// One legend entry.
#[derive(Debug, Clone, PartialEq)]
pub struct LegendItem {
    pub label: String,
    pub color: Color,
}

/// A resolved legend.
#[derive(Debug, Clone, PartialEq)]
pub struct Legend {
    pub position: LegendPosition,
    pub title: Option<String>,
    pub text_style: TextStyle,
    pub title_style: TextStyle,
    pub items: Vec<LegendItem>,
}

/// Where a drawn legend went and what is left for the plot.
#[derive(Debug, Clone, PartialEq)]
pub struct LegendLayout {
    /// The legend box.
    pub rect: Rect,
    /// The remaining plot area.
    pub plot: Rect,
    /// Items drawn (the rest did not fit).
    pub shown: usize,
}

impl Legend {
    /// Reads the `legend` object. `None` when the legend is off or there
    /// is nothing to list. Text uses the `lightLabel` class (10pt
    /// `#605E5C`) with `labelColor`/`fontSize`/`fontFamily`; the title is
    /// the bold variant.
    pub fn read(
        props: &VisualProps<'_>,
        default_show: bool,
        default_title: Option<String>,
        items: Vec<LegendItem>,
    ) -> Option<Self> {
        let o = props.objects();
        if !o.bool("legend", "show").unwrap_or(default_show) || items.is_empty() {
            return None;
        }
        let position = o
            .text("legend", "position")
            .map_or(LegendPosition::Top, |p| LegendPosition::parse(&p));
        let mut text_style = o.text_style("legend", "lightLabel");
        if let Some(c) = o.color("legend", "labelColor") {
            text_style.color = c;
        }
        let title_style = text_style.clone().bold();
        let title = if o.bool("legend", "showTitle").unwrap_or(true) {
            o.text("legend", "titleText")
                .filter(|t| !t.trim().is_empty())
                .or(default_title)
        } else {
            None
        };
        Some(Self {
            position,
            title,
            text_style,
            title_style,
            items,
        })
    }

    fn marker_diameter(&self) -> f64 {
        (self.text_style.size_px() * 0.72).max(4.0)
    }

    /// Lays out the legend in `area`, draws it into `g` and returns the
    /// layout (`plot` is `area` minus the legend).
    pub fn draw(&self, g: &mut Group, area: Rect, fonts: &FontBook) -> LegendLayout {
        if self.position.is_vertical() {
            self.draw_vertical(g, area, fonts)
        } else {
            self.draw_horizontal(g, area, fonts)
        }
    }

    fn draw_vertical(&self, g: &mut Group, area: Rect, fonts: &FontBook) -> LegendLayout {
        const INSET_LEFT: f64 = 1.5;
        const INSET_RIGHT: f64 = 6.0;
        const MARKER_GAP: f64 = 3.0;
        let dm = self.marker_diameter();
        let lh = self.text_style.line_height();
        let pitch = lh * 1.41;
        let title_h = self
            .title
            .as_ref()
            .map_or(0.0, |_| self.title_style.line_height() * 1.19);
        let title_w = self
            .title
            .as_ref()
            .map_or(0.0, |t| width(fonts, t, &self.title_style));
        let item_w = self
            .items
            .iter()
            .take(500)
            .map(|i| width(fonts, &i.label, &self.text_style))
            .fold(0.0, f64::max);
        let natural = INSET_LEFT + title_w.max(dm + MARKER_GAP + item_w) + INSET_RIGHT;
        let w = natural.min(area.width * 0.4).max(0.0);
        let rows = ((area.height - title_h) / pitch).floor().max(0.0) as usize;
        let overflow = self.items.len() > rows;
        let shown = if overflow {
            rows.saturating_sub(1)
        } else {
            self.items.len()
        };
        let block_h = title_h + pitch * (shown + usize::from(overflow)) as f64;
        let x = match self.position {
            LegendPosition::Left | LegendPosition::LeftCenter => area.x,
            _ => area.right() - w,
        };
        let y0 = if self.position.is_centered() {
            area.y + ((area.height - block_h) / 2.0).max(0.0)
        } else {
            area.y
        };
        let rect = Rect::new(x, y0, w, block_h.min(area.height));
        let mut lg = Group::new().with_class("pbi-legend");
        let text_x = x + INSET_LEFT;
        let max_text = (w - INSET_LEFT - INSET_RIGHT).max(0.0);
        if let Some(title) = &self.title {
            let t = tailor(fonts, title, &self.title_style, max_text);
            let cy = y0 + title_h / 2.0;
            lg.text(
                text_x,
                baseline_centered(&self.title_style, cy),
                t,
                self.title_style.clone(),
                TextAnchor::Start,
            );
        }
        for (k, item) in self.items.iter().take(shown).enumerate() {
            let cy = y0 + title_h + pitch * (k as f64 + 0.5);
            push_marker(&mut lg, text_x + dm / 2.0, cy, dm, item.color);
            let label = tailor(
                fonts,
                &item.label,
                &self.text_style,
                (max_text - dm - MARKER_GAP).max(0.0),
            );
            lg.text(
                text_x + dm + MARKER_GAP,
                baseline_centered(&self.text_style, cy),
                label,
                self.text_style.clone(),
                TextAnchor::Start,
            );
        }
        if overflow {
            let cy = y0 + title_h + pitch * (shown as f64 + 0.5);
            push_arrow(
                &mut lg,
                text_x + dm / 2.0,
                cy,
                dm,
                self.text_style.color,
                true,
            );
        }
        g.push(lg);
        let plot = match self.position {
            LegendPosition::Left | LegendPosition::LeftCenter => {
                Rect::new(area.x + w, area.y, (area.width - w).max(0.0), area.height)
            }
            _ => Rect::new(area.x, area.y, (area.width - w).max(0.0), area.height),
        };
        LegendLayout { rect, plot, shown }
    }

    fn draw_horizontal(&self, g: &mut Group, area: Rect, fonts: &FontBook) -> LegendLayout {
        const TITLE_GAP: f64 = 8.0;
        const ITEM_GAP: f64 = 10.0;
        const MARKER_GAP: f64 = 3.0;
        let dm = self.marker_diameter();
        let row_h = self
            .text_style
            .line_height()
            .max(self.title_style.line_height())
            + 4.0;
        let arrow_w = dm + 4.0;
        let mut pieces: Vec<(f64, Option<&LegendItem>, String)> = Vec::new();
        let mut used = 0.0;
        if let Some(title) = &self.title {
            let t = tailor(fonts, title, &self.title_style, area.width * 0.5);
            let tw = width(fonts, &t, &self.title_style);
            pieces.push((tw, None, t));
            used += tw + TITLE_GAP;
        }
        let mut shown = 0;
        for (k, item) in self.items.iter().enumerate() {
            let tw = width(fonts, &item.label, &self.text_style);
            let w = dm + MARKER_GAP + tw;
            let rest_needs_arrow = k + 1 < self.items.len();
            let budget = area.width - if rest_needs_arrow { arrow_w } else { 0.0 };
            if used + w > budget {
                break;
            }
            pieces.push((w, Some(item), item.label.clone()));
            used += w + ITEM_GAP;
            shown += 1;
        }
        let overflow = shown < self.items.len();
        let total = used - if pieces.is_empty() { 0.0 } else { ITEM_GAP }
            + if overflow { arrow_w } else { 0.0 };
        let x0 = if self.position.is_centered() {
            area.x + ((area.width - total) / 2.0).max(0.0)
        } else {
            area.x
        };
        let y0 = match self.position {
            LegendPosition::Bottom | LegendPosition::BottomCenter => area.bottom() - row_h,
            _ => area.y,
        };
        let cy = y0 + row_h / 2.0;
        let mut lg = Group::new().with_class("pbi-legend");
        let mut x = x0;
        for (w, item, text) in &pieces {
            match item {
                None => {
                    lg.text(
                        x,
                        baseline_centered(&self.title_style, cy),
                        text.clone(),
                        self.title_style.clone(),
                        TextAnchor::Start,
                    );
                    x += w + TITLE_GAP;
                }
                Some(item) => {
                    push_marker(&mut lg, x + dm / 2.0, cy, dm, item.color);
                    lg.text(
                        x + dm + MARKER_GAP,
                        baseline_centered(&self.text_style, cy),
                        text.clone(),
                        self.text_style.clone(),
                        TextAnchor::Start,
                    );
                    x += w + ITEM_GAP;
                }
            }
        }
        if overflow {
            push_arrow(&mut lg, x + dm / 2.0, cy, dm, self.text_style.color, false);
        }
        g.push(lg);
        let rect = Rect::new(area.x, y0, area.width, row_h.min(area.height));
        let plot_h = (area.height - row_h - 2.0).max(0.0);
        let plot = match self.position {
            LegendPosition::Bottom | LegendPosition::BottomCenter => {
                Rect::new(area.x, area.y, area.width, plot_h)
            }
            _ => Rect::new(area.x, area.bottom() - plot_h, area.width, plot_h),
        };
        LegendLayout { rect, plot, shown }
    }
}

fn push_marker(g: &mut Group, cx: f64, cy: f64, d: f64, color: Color) {
    g.push(
        PathNode::new(PathData::pie_slice(cx, cy, d / 2.0, 0.0, 0.0, 360.0))
            .fill(Paint::solid(color)),
    );
}

/// A small paging triangle (down for vertical legends, right otherwise).
fn push_arrow(g: &mut Group, cx: f64, cy: f64, size: f64, color: Color, down: bool) {
    let h = size / 2.0;
    let mut b = PathBuilder::new();
    if down {
        b.move_to(cx - h, cy - h / 2.0)
            .line_to(cx + h, cy - h / 2.0)
            .line_to(cx, cy + h / 2.0)
            .close();
    } else {
        b.move_to(cx - h / 2.0, cy - h)
            .line_to(cx + h / 2.0, cy)
            .line_to(cx - h / 2.0, cy + h)
            .close();
    }
    g.push(PathNode::new(b.build()).fill(Paint::solid(color)));
}
