//! Table (`tableEx`): header, rows, totals, column widths (auto-size),
//! word wrap, alternating rows, grid lines, scrolling clip. (owner:
//! visuals-grids)
//!
//! Builds a [`grid::GridModel`] from the [`TableData`] of the DataView and
//! renders it with the shared grid engine ([`grid`], also used by
//! [`super::matrix`]); see there for geometry, auto-sizing and scrolling.
//!
//! # Formatting objects read
//!
//! | Object | Properties |
//! |---|---|
//! | `columnHeaders` | `fontColor`, `backColor`, `fontFamily`, `fontSize`, `bold`, `italic`, `underline`, `alignment`, `wordWrap` (default on), `outline` / `outlineStyle` (`None` hides the bottom line) |
//! | `values` | `fontColorPrimary`/`fontColor`, `backColorPrimary`/`backColor`, `fontColorSecondary`, `backColorSecondary` (alternate rows; default `#EEEDED`), font properties, `wordWrap` (default on) |
//! | `total` | `fontColor`, `backColor`, font properties (bold by default); the row itself comes from the DataView (`total.totals`) |
//! | `grid` | `gridHorizontal` (default on), `gridHorizontalColor` (`#E5E5E5`), `gridHorizontalWeight`, `gridVertical` (default off), `gridVerticalColor`, `gridVerticalWeight`, `rowPadding`, `outlineColor` (default theme `tableAccent`), `outlineWeight`, `textSize` |
//! | `columnWidth` | `value` per column (selector `metadata` = query ref) |
//! | `columnFormatting` | `alignment`, `fontColor`, `backColor` per column |
//! | `stylePreset` | `name`: `None` (no bands, lines or outline), `Minimal`/`Sparse`/`BoldHeader`/`Condensed` (no bands), anything else = `Default` |
//!
//! Values of measures and numeric columns are right-aligned; headers are
//! left-aligned unless `columnHeaders.alignment` says otherwise. A sort
//! arrow is drawn under the header of each column of an explicit
//! `sortDefinition`. BLANK values show as empty cells; the total row shows
//! `Total` in its first column when that cell is empty.
//!
//! # Interactive canvas
//!
//! [`render_with`] takes a [`TableOptions::scroll`] offset (page units,
//! clamped); [`metrics`] returns the scroll geometry ([`GridMetrics`]:
//! body rect, content size, maximum offsets, visible rows). Every body
//! cell is a group with `HitId::data_point(visual, column, row)` (`series`
//! = display column, `index` = row of [`TableData::rows`]).
//!
//! # Conditional formatting
//!
//! Background and font colors (gradients, rules, field values), data bars
//! and icons of the `values` / `columnFormatting` objects, per column
//! (`selector.metadata`), on values, totals or both
//! ([`crate::conditional`]). Gradient bounds and percent rules use the
//! column's data rows (not the total). Rules based on other fields read
//! the hidden fields of the DataView.
//!
//! Not supported yet: URL icons / images, header sort hit areas.

pub(crate) mod grid;

use serde::{Deserialize, Serialize};

use crate::conditional::{CellSource, CellStyle, CfSpec, Input, evaluate_cells, find_input};
use crate::dataview::{DataView, TableData};
use crate::layout::TextAlign;
use crate::scene::Group;
use crate::theme::{as_number, as_text};

use super::VisualCtx;
use grid::{
    Grid, GridCell, GridColumn, GridEnv, GridModel, GridRow, GridStyle, HeaderCell, PreparedGrid,
    RowKind, column_property,
};

pub use grid::{GridMetrics, GridScroll, line_metrics};

/// Label of the total row (first column).
pub const TOTAL_LABEL: &str = "Total";

/// Interactive state of a table.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TableOptions {
    /// Scroll offset of the body (page units; clamped).
    pub scroll: GridScroll,
}

/// Renders the visual content (see module docs).
pub fn render(ctx: &VisualCtx<'_>) -> Option<Group> {
    render_with(ctx, &TableOptions::default())
}

/// Renders the table scrolled to `opts.scroll`.
pub fn render_with(ctx: &VisualCtx<'_>, opts: &TableOptions) -> Option<Group> {
    let Some((model, style)) = build(ctx) else {
        return super::placeholder::render(ctx);
    };
    let mut grid = Grid::new(env(ctx), &model, &style, opts.scroll);
    Some(grid.render())
}

/// Scroll geometry of the table at `opts.scroll` (`None` without table
/// data).
pub fn metrics(ctx: &VisualCtx<'_>, opts: &TableOptions) -> Option<GridMetrics> {
    let (model, style) = build(ctx)?;
    let grid = Grid::new(env(ctx), &model, &style, opts.scroll);
    Some(grid.metrics().clone())
}

pub(crate) fn env<'a>(ctx: &VisualCtx<'a>) -> GridEnv<'a> {
    GridEnv {
        fonts: ctx.fonts,
        viewport: ctx.frame.content,
        visual: &ctx.visual.name,
    }
}

/// The successful DataView of the visual.
pub(crate) fn dataview<'a>(ctx: &VisualCtx<'a>) -> Option<&'a DataView> {
    match ctx.data {
        Some(Ok(dv)) => Some(dv),
        _ => None,
    }
}

/// `Left`/`Center`/`Right` (case-insensitive); `None` for `Auto` and
/// anything else.
pub(crate) fn parse_align(text: &str) -> Option<TextAlign> {
    match text.trim().to_ascii_lowercase().as_str() {
        "left" => Some(TextAlign::Left),
        "center" | "centre" => Some(TextAlign::Center),
        "right" => Some(TextAlign::Right),
        _ => None,
    }
}

/// The table's model, style and layout, for drawing at any scroll offset
/// (`None` without table data; [`render_with`] then draws the placeholder).
pub(crate) fn prepare(ctx: &VisualCtx<'_>) -> Option<PreparedGrid> {
    let (model, style) = build(ctx)?;
    Some(PreparedGrid::new(env(ctx), model, style))
}

fn build(ctx: &VisualCtx<'_>) -> Option<(GridModel, GridStyle)> {
    let dv = dataview(ctx)?;
    let table = dv.table.as_ref()?;
    let props = ctx.theme.visual(ctx.visual);
    let style = GridStyle::resolve(&props, "total");
    Some((model(ctx, dv, table), style))
}

fn model(ctx: &VisualCtx<'_>, dv: &DataView, table: &TableData) -> GridModel {
    let objects = &ctx.visual.objects;
    let props = ctx.theme.visual(ctx.visual);
    let header_align = props
        .objects()
        .text("columnHeaders", "alignment")
        .and_then(|a| parse_align(&a))
        .unwrap_or(TextAlign::Left);
    let explicit_sort = !ctx.visual.sort.is_empty();
    let mut columns = Vec::with_capacity(table.columns.len());
    let mut header = Vec::with_capacity(table.columns.len());
    for (ci, &mi) in table.columns.iter().enumerate() {
        let meta = dv.meta(mi);
        let query_ref = meta.map_or("", |m| m.query_ref.as_str());
        let numeric = meta
            .is_some_and(|m| m.is_measure || m.data_type.as_ref().is_some_and(|t| t.is_numeric()));
        let col_prop = |object: &str, prop: &str| column_property(objects, object, prop, query_ref);
        let align = col_prop("columnFormatting", "alignment")
            .and_then(as_text)
            .and_then(|a| parse_align(&a))
            .unwrap_or(if numeric {
                TextAlign::Right
            } else {
                TextAlign::Left
            });
        columns.push(GridColumn {
            width: col_prop("columnWidth", "value")
                .and_then(as_number)
                .filter(|w| *w > 0.0),
            align,
            header_align,
            sort: if explicit_sort {
                meta.and_then(|m| m.sort)
            } else {
                None
            },
            font_color: col_prop("columnFormatting", "fontColor").and_then(|v| ctx.theme.color(v)),
            back_color: col_prop("columnFormatting", "backColor").and_then(|v| ctx.theme.color(v)),
        });
        header.push(HeaderCell {
            col: ci,
            span: 1,
            text: meta.map(|m| m.display_name.clone()).unwrap_or_default(),
            align: None,
        });
    }
    let rows = table
        .rows
        .iter()
        .enumerate()
        .map(|(ri, row)| GridRow {
            cells: row
                .iter()
                .enumerate()
                .map(|(ci, cell)| GridCell {
                    hit: Some((ci, ri)),
                    ..GridCell::text(cell.formatted.clone())
                })
                .collect(),
            kind: RowKind::Data,
        })
        .collect();
    let total = table.totals.as_ref().map(|cells| {
        let mut cells: Vec<GridCell> = cells
            .iter()
            .map(|c| GridCell::text(c.formatted.clone()))
            .collect();
        if let Some(first) = cells.first_mut()
            && first.text.is_empty()
        {
            first.text = TOTAL_LABEL.to_owned();
            first.align = Some(TextAlign::Left);
        }
        GridRow {
            cells,
            kind: RowKind::Total,
        }
    });
    let mut model = GridModel {
        columns,
        header_rows: vec![header],
        rows,
        total,
        truncated: dv.truncated,
    };
    apply_conditional(ctx, dv, table, &mut model);
    model
}

/// One column of a [`TableData`] as conditional-formatting rules see it:
/// its rows, then the totals row.
struct ColumnCells<'a> {
    dv: &'a DataView,
    table: &'a TableData,
    column: usize,
}

/// Bound input columns: the formatted column's own cells, another visible
/// column, or a hidden field.
const OWN: usize = usize::MAX;
const HIDDEN: usize = usize::MAX / 4;

impl CellSource for ColumnCells<'_> {
    fn cell_count(&self) -> usize {
        self.table.rows.len() + usize::from(self.table.totals.is_some())
    }

    fn is_total(&self, cell: usize) -> bool {
        cell >= self.table.rows.len()
    }

    fn bind(&self, input: &Input) -> Option<usize> {
        let Input::Field(field) = input else {
            return Some(OWN);
        };
        let meta = find_input(&self.dv.metadata, field)?;
        if let Some(p) = self.table.columns.iter().position(|&m| m == meta) {
            return Some(p);
        }
        let h = self.table.hidden.iter().position(|h| h.meta_idx == meta)?;
        Some(HIDDEN + h)
    }

    fn value(&self, column: usize, cell: usize) -> pbi_engine::Value {
        let rows = self.table.rows.len();
        let column = if column == OWN { self.column } else { column };
        let found = if column >= HIDDEN {
            self.table.hidden.get(column - HIDDEN).and_then(|h| {
                if cell < rows {
                    h.values.get(cell)
                } else {
                    h.total.as_ref()
                }
            })
        } else if cell < rows {
            self.table.rows.get(cell).and_then(|r| r.get(column))
        } else {
            self.table.totals.as_ref().and_then(|t| t.get(column))
        };
        found.map(|c| c.raw.clone()).unwrap_or_default()
    }
}

/// Applies the visual's conditional formats to the model's cells.
fn apply_conditional(ctx: &VisualCtx<'_>, dv: &DataView, table: &TableData, model: &mut GridModel) {
    let spec = CfSpec::parse(ctx.visual);
    if spec.is_empty() {
        return;
    }
    for (column, &mi) in table.columns.iter().enumerate() {
        let Some(meta) = dv.meta(mi) else {
            continue;
        };
        let rules = spec.rules_for(&meta.query_ref, false);
        if rules.is_empty() {
            continue;
        }
        let source = ColumnCells { dv, table, column };
        let styles = evaluate_cells(&rules, &source, ctx.theme);
        let rows = model.rows.iter_mut().chain(model.total.iter_mut());
        for (row, style) in rows.zip(styles) {
            if let Some(cell) = row.cells.get_mut(column) {
                set_style(cell, style);
            }
        }
    }
}

/// Stores a cell's conditional style (data bars that hide the value clear
/// its text, icon-only cells too).
pub(crate) fn set_style(cell: &mut GridCell, style: CellStyle) {
    if style.is_empty() {
        return;
    }
    let icon_only = style
        .icon
        .is_some_and(|i| i.1 == crate::conditional::IconLayout::IconOnly);
    if (style.hide_text && style.bar.is_some()) || icon_only {
        cell.text.clear();
    }
    cell.cf = Some(Box::new(style));
}
