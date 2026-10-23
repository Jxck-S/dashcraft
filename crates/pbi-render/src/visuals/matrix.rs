//! Matrix (`pivotTable`): row/column headers, subtotals, stepped layout.
//! (owner: visuals-grids)
//!
//! Builds a grid model ([`super::table::grid`]) from the DataView's
//! [`Matrix`] and renders it with the same engine (and the same formatting
//! objects) as the table; see [`super::table`] for the shared objects.
//!
//! # Rows
//!
//! - **Stepped layout** (default; `rowHeaders.stepped` = false switches to
//!   one column per row level): one row-header column; each level is
//!   indented by `rowHeaders.steppedLayoutIndentation` (default 10px).
//!   An expanded parent row shows its subtotals (bold) on the row itself
//!   when `subTotals.rowSubtotals` is on, else empty cells.
//! - **Tabular layout**: one column per row level; an expanded group is
//!   followed by a bold "Total" row with its subtotals.
//! - Expansion: like the service, only the first row level shows unless
//!   expanded — [`MatrixOptions::expand_depth`] / [`MatrixOptions::toggled`],
//!   or (best effort) the visual's persisted `expansionStates` (leading
//!   levels with `isCollapsed: false`, root children with `isToggled`).
//!   `rowHeaders.showExpandCollapseButtons` draws `+`/`−` boxes on parents.
//! - The grand total row ("Total", bold, accent line on top) is pinned at
//!   the bottom when row subtotals are on.
//!
//! # Columns
//!
//! One header row per column level (cells span their leaf columns ×
//! values), plus a row of value names when there are several values (or no
//! column levels). Subtotal and grand-total columns are headed "Total".
//! Values are right-aligned.
//!
//! # Conditional formatting
//!
//! As the table ([`super::table`]), per value field (`selector.metadata`):
//! "values" are the cells of the rows at the lowest visible level and of
//! the non-total columns; "totals" are subtotal and grand-total rows and
//! columns. Gradient bounds and percent rules use the values. Expanded
//! parents without subtotals (empty cells) are not formatted.
//!
//! # Interactive canvas
//!
//! Every body cell is a group with `HitId::data_point(visual, column,
//! row)`: `column` = grid column (row-header columns first), `row` = index
//! into [`visible_rows`] (which gives the row's level and group path).
//! [`render_with`] takes a scroll offset and the expansion state;
//! [`metrics`] returns the scroll geometry.

use serde::{Deserialize, Serialize};

use crate::conditional::{CellSource, CfSpec, Input, evaluate_cells, find_input};
use crate::dataview::{BLANK_LABEL, CellValue, DataView, Matrix, MatrixNode};
use crate::layout::TextAlign;
use crate::scene::Group;
use crate::theme::as_bool;

use super::VisualCtx;
use super::table::grid::{
    EXPANDER_LEAD, Grid, GridCell, GridColumn, GridMetrics, GridModel, GridRow, GridScroll,
    GridStyle, HeaderCell, PreparedGrid, RowKind,
};
use super::table::{dataview, env, parse_align, set_style};

/// Label of subtotal and grand-total rows and columns.
pub const TOTAL_LABEL: &str = "Total";
/// Default stepped-layout indentation per level.
pub const DEFAULT_INDENT: f64 = 10.0;

/// Interactive state of a matrix.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct MatrixOptions {
    /// Scroll offset of the body (page units; clamped).
    pub scroll: GridScroll,
    /// Expand every row level above this depth (`0`: first level only,
    /// `usize::MAX`: everything). `None`: the visual's persisted state.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expand_depth: Option<usize>,
    /// Row groups (formatted values from the first level down) whose
    /// expansion is the opposite of `expand_depth`'s.
    pub toggled: Vec<Vec<String>>,
}

/// A visible body row.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MatrixRowRef {
    /// Row level of the group (`None` for a tabular subtotal row: see
    /// `subtotal`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub level: Option<usize>,
    /// Group values from the first level down to this row.
    pub path: Vec<CellValue>,
    /// The row shows subtotals (expanded parent or tabular total row).
    pub subtotal: bool,
}

/// Renders the visual content (see module docs).
pub fn render(ctx: &VisualCtx<'_>) -> Option<Group> {
    render_with(ctx, &MatrixOptions::default())
}

/// Renders the matrix with a scroll offset and expansion state.
pub fn render_with(ctx: &VisualCtx<'_>, opts: &MatrixOptions) -> Option<Group> {
    let Some((model, style, _)) = build(ctx, opts) else {
        return super::placeholder::render(ctx);
    };
    let mut grid = Grid::new(env(ctx), &model, &style, opts.scroll);
    Some(grid.render())
}

/// Scroll geometry of the matrix (`None` without matrix data).
pub fn metrics(ctx: &VisualCtx<'_>, opts: &MatrixOptions) -> Option<GridMetrics> {
    let (model, style, _) = build(ctx, opts)?;
    let grid = Grid::new(env(ctx), &model, &style, opts.scroll);
    Some(grid.metrics().clone())
}

/// The matrix's model, style, layout and body rows for an expansion state,
/// for drawing at any scroll offset (`None` without matrix data;
/// [`render_with`] then draws the placeholder).
pub(crate) fn prepare(ctx: &VisualCtx<'_>, opts: &MatrixOptions) -> Option<PreparedGrid> {
    let (model, style, refs) = build(ctx, opts)?;
    let mut prepared = PreparedGrid::new(env(ctx), model, style);
    prepared.rows = refs;
    Some(prepared)
}

/// The body rows in display order (hit `index` → row identity).
pub fn visible_rows(ctx: &VisualCtx<'_>, opts: &MatrixOptions) -> Vec<MatrixRowRef> {
    build(ctx, opts)
        .map(|(_, _, refs)| refs)
        .unwrap_or_default()
}

/// Expansion settings.
#[derive(Debug, Clone)]
struct Expansion {
    depth: usize,
    toggled: Vec<Vec<String>>,
}

impl Expansion {
    fn from(ctx: &VisualCtx<'_>, opts: &MatrixOptions) -> Self {
        match opts.expand_depth {
            Some(depth) => Self {
                depth,
                toggled: opts.toggled.clone(),
            },
            None => {
                let mut e = persisted_expansion(ctx);
                e.toggled.extend(opts.toggled.iter().cloned());
                e
            }
        }
    }

    fn expanded(&self, depth: usize, path: &[String]) -> bool {
        let toggled = self.toggled.iter().any(|t| t.as_slice() == path);
        (depth < self.depth) != toggled
    }
}

/// Best-effort read of `visual.expansionStates` (see module docs).
fn persisted_expansion(ctx: &VisualCtx<'_>) -> Expansion {
    let mut e = Expansion {
        depth: 0,
        toggled: Vec::new(),
    };
    let Some(json) = ctx
        .visual
        .extra
        .get("expansionStates")
        .map(pbi_model::PropValue::to_json)
    else {
        return e;
    };
    let Some(states) = json.as_array() else {
        return e;
    };
    let rows = states.iter().find(|s| {
        s.get("roles")
            .and_then(serde_json::Value::as_array)
            .is_some_and(|r| r.iter().any(|v| v.as_str() == Some("Rows")))
    });
    let Some(state) = rows else {
        return e;
    };
    if let Some(levels) = state.get("levels").and_then(serde_json::Value::as_array) {
        e.depth = levels
            .iter()
            .take_while(|l| l.get("isCollapsed").and_then(as_bool) == Some(false))
            .count();
    }
    fn walk(node: &serde_json::Value, prefix: &[String], out: &mut Vec<Vec<String>>, depth: u8) {
        if depth > 32 {
            return;
        }
        let Some(children) = node.get("children").and_then(serde_json::Value::as_array) else {
            return;
        };
        for child in children {
            let mut path = prefix.to_vec();
            if let Some(ids) = child
                .get("identityValues")
                .and_then(serde_json::Value::as_array)
            {
                path.extend(ids.iter().filter_map(identity_text));
            }
            if child.get("isToggled").and_then(as_bool) == Some(true) {
                out.push(path.clone());
            }
            walk(child, &path, out, depth + 1);
        }
    }
    if let Some(root) = state.get("root") {
        walk(root, &[], &mut e.toggled, 0);
    }
    e
}

/// Display text of a persisted identity literal (`'Camping'` → `Camping`).
fn identity_text(v: &serde_json::Value) -> Option<String> {
    let raw = v
        .get("Literal")
        .or_else(|| v.get("expr").and_then(|e| e.get("Literal")))?
        .get("Value")?
        .as_str()?;
    let lit = pbi_model::Literal::new(raw).value().ok()?;
    Some(match lit {
        pbi_model::LiteralValue::Text(s) => s,
        pbi_model::LiteralValue::Null => BLANK_LABEL.to_owned(),
        other => other
            .as_f64()
            .map_or_else(|| raw.to_owned(), |n| n.to_string()),
    })
}

fn label(cell: Option<&CellValue>) -> String {
    match cell {
        Some(c) if !(c.formatted.is_empty() && c.is_blank()) => c.formatted.clone(),
        Some(_) => BLANK_LABEL.to_owned(),
        None => String::new(),
    }
}

fn build(
    ctx: &VisualCtx<'_>,
    opts: &MatrixOptions,
) -> Option<(GridModel, GridStyle, Vec<MatrixRowRef>)> {
    let dv = dataview(ctx)?;
    let m = dv.matrix.as_ref()?;
    let props = ctx.theme.visual(ctx.visual);
    let o = props.objects();
    let style = GridStyle::resolve(&props, "subTotals");
    let settings = Settings {
        stepped: o.bool("rowHeaders", "stepped").unwrap_or(true),
        indent: o
            .number("rowHeaders", "steppedLayoutIndentation")
            .filter(|n| *n >= 0.0)
            .unwrap_or(DEFAULT_INDENT)
            .min(200.0),
        buttons: o
            .bool("rowHeaders", "showExpandCollapseButtons")
            .unwrap_or(false),
        expansion: Expansion::from(ctx, opts),
        header_align: o
            .text("columnHeaders", "alignment")
            .and_then(|a| parse_align(&a)),
    };
    let (mut model, refs, nodes) = model(dv, m, &settings);
    apply_conditional(ctx, dv, m, &mut model, &nodes);
    Some((model, style, refs))
}

#[derive(Debug, Clone)]
struct Settings {
    stepped: bool,
    indent: f64,
    buttons: bool,
    expansion: Expansion,
    header_align: Option<TextAlign>,
}

/// The grid model, the body rows' identities and the node each body row
/// (then the grand-total row) shows values of (`None`: empty cells).
fn model<'m>(
    dv: &DataView,
    m: &'m Matrix,
    s: &Settings,
) -> (GridModel, Vec<MatrixRowRef>, Vec<Option<&'m MatrixNode>>) {
    let vc = m.values.len().max(1);
    let row_cols = if s.stepped {
        1
    } else {
        m.row_levels.len().max(1)
    };
    let value_cols = m.columns.len() * vc;
    let name = |idx: usize| {
        dv.meta(idx)
            .map(|meta| meta.display_name.clone())
            .unwrap_or_default()
    };

    let mut columns = vec![GridColumn::default(); row_cols];
    for _ in 0..value_cols {
        columns.push(GridColumn {
            align: TextAlign::Right,
            header_align: s.header_align.unwrap_or(TextAlign::Left),
            ..GridColumn::default()
        });
    }

    // Header rows: one per column level, then value names.
    let levels = m.column_levels.len();
    let value_names = m.values.len() > 1 || levels == 0;
    let mut header_rows: Vec<Vec<HeaderCell>> = Vec::new();
    for l in 0..levels {
        let mut row = Vec::new();
        let mut c = 0;
        while c < m.columns.len() {
            let Some(col) = m.columns.get(c) else {
                break;
            };
            let col_start = row_cols + c * vc;
            if col.is_total && col.path.len() == l {
                row.push(HeaderCell {
                    col: col_start,
                    span: vc,
                    text: TOTAL_LABEL.to_owned(),
                    align: None,
                });
                c += 1;
                continue;
            }
            if col.path.len() <= l {
                c += 1;
                continue;
            }
            let prefix = col.path.get(..=l);
            let mut end = c + 1;
            while m
                .columns
                .get(end)
                .is_some_and(|n| n.path.len() > l && n.path.get(..=l) == prefix)
            {
                end += 1;
            }
            row.push(HeaderCell {
                col: col_start,
                span: (end - c) * vc,
                text: label(col.path.get(l)),
                align: None,
            });
            c = end;
        }
        header_rows.push(row);
    }
    if value_names {
        let mut row = Vec::new();
        for c in 0..m.columns.len() {
            for (v, &idx) in m.values.iter().enumerate() {
                row.push(HeaderCell {
                    col: row_cols + c * vc + v,
                    span: 1,
                    text: name(idx),
                    align: None,
                });
            }
        }
        header_rows.push(row);
    }
    if header_rows.is_empty() {
        header_rows.push(Vec::new());
    }
    if let Some(last) = header_rows.last_mut() {
        if s.stepped {
            last.insert(
                0,
                HeaderCell {
                    col: 0,
                    span: 1,
                    text: m.row_levels.first().map(|&i| name(i)).unwrap_or_default(),
                    align: Some(TextAlign::Left),
                },
            );
        } else {
            for (k, &idx) in m.row_levels.iter().enumerate() {
                last.insert(
                    k,
                    HeaderCell {
                        col: k,
                        span: 1,
                        text: name(idx),
                        align: Some(TextAlign::Left),
                    },
                );
            }
        }
    }

    // Body rows.
    let mut b = RowBuilder {
        s,
        row_cols,
        value_cols,
        row_subtotals: m.row_subtotals,
        rows: Vec::new(),
        refs: Vec::new(),
        nodes: Vec::new(),
    };
    b.walk(&m.rows, 0, &[], &[]);
    let total = (m.row_subtotals && !m.row_levels.is_empty()).then(|| {
        let mut cells = vec![GridCell::default(); row_cols];
        if let Some(first) = cells.first_mut() {
            first.text = TOTAL_LABEL.to_owned();
            first.row_header = true;
        }
        cells.extend((0..value_cols).map(|k| {
            GridCell::text(
                m.rows
                    .cells
                    .get(k)
                    .map(|c| c.formatted.clone())
                    .unwrap_or_default(),
            )
        }));
        GridRow {
            cells,
            kind: RowKind::Total,
        }
    });
    let mut nodes = b.nodes;
    if total.is_some() {
        nodes.push(Some(&m.rows));
    }
    (
        GridModel {
            columns,
            header_rows,
            rows: b.rows,
            total,
            truncated: dv.truncated,
        },
        b.refs,
        nodes,
    )
}

/// The cells of one value field of a matrix as conditional-formatting
/// rules see them: body rows (then the grand-total row) × columns.
struct ValueCells<'a> {
    dv: &'a DataView,
    m: &'a Matrix,
    nodes: &'a [Option<&'a MatrixNode>],
    kinds: Vec<RowKind>,
    value: usize,
}

const OWN: usize = usize::MAX;
const HIDDEN: usize = usize::MAX / 4;

impl ValueCells<'_> {
    fn columns(&self) -> usize {
        self.m.columns.len().max(1)
    }
}

impl CellSource for ValueCells<'_> {
    fn cell_count(&self) -> usize {
        self.nodes.len() * self.columns()
    }

    fn is_total(&self, cell: usize) -> bool {
        let (r, c) = (cell / self.columns(), cell % self.columns());
        self.kinds.get(r).is_some_and(|k| *k != RowKind::Data)
            || self.m.columns.get(c).is_some_and(|col| col.is_total)
    }

    fn bind(&self, input: &Input) -> Option<usize> {
        let Input::Field(field) = input else {
            return Some(OWN);
        };
        let meta = find_input(&self.dv.metadata, field)?;
        if let Some(v) = self.m.values.iter().position(|&x| x == meta) {
            return Some(v);
        }
        let h = self.m.hidden.iter().position(|&x| x == meta)?;
        Some(HIDDEN + h)
    }

    fn value(&self, column: usize, cell: usize) -> pbi_engine::Value {
        let (r, c) = (cell / self.columns(), cell % self.columns());
        let Some(node) = self.nodes.get(r).copied().flatten() else {
            return pbi_engine::Value::Blank;
        };
        let column = if column == OWN { self.value } else { column };
        let found = if column >= HIDDEN {
            node.hidden.get(c * self.m.hidden.len() + (column - HIDDEN))
        } else {
            node.cells.get(c * self.m.values.len() + column)
        };
        found.map(|v| v.raw.clone()).unwrap_or_default()
    }
}

/// Applies the visual's conditional formats to the value cells.
fn apply_conditional(
    ctx: &VisualCtx<'_>,
    dv: &DataView,
    m: &Matrix,
    model: &mut GridModel,
    nodes: &[Option<&MatrixNode>],
) {
    let spec = CfSpec::parse(ctx.visual);
    if spec.is_empty() || m.values.is_empty() {
        return;
    }
    let row_cols = model
        .columns
        .len()
        .saturating_sub(m.columns.len() * m.values.len());
    let kinds: Vec<RowKind> = model
        .rows
        .iter()
        .chain(model.total.iter())
        .map(|r| r.kind)
        .collect();
    for (v, &mi) in m.values.iter().enumerate() {
        let Some(meta) = dv.meta(mi) else {
            continue;
        };
        let rules = spec.rules_for(&meta.query_ref, false);
        if rules.is_empty() {
            continue;
        }
        let source = ValueCells {
            dv,
            m,
            nodes,
            kinds: kinds.clone(),
            value: v,
        };
        let ncols = source.columns();
        let styles = evaluate_cells(&rules, &source, ctx.theme);
        let rows = model.rows.iter_mut().chain(model.total.iter_mut());
        for (r, row) in rows.enumerate() {
            if nodes.get(r).copied().flatten().is_none() {
                continue;
            }
            for c in 0..m.columns.len() {
                let style = styles.get(r * ncols + c).cloned().unwrap_or_default();
                if let Some(cell) = row.cells.get_mut(row_cols + c * m.values.len() + v) {
                    set_style(cell, style);
                }
            }
        }
    }
}

/// Flattens the row hierarchy into grid rows.
struct RowBuilder<'a, 'm> {
    s: &'a Settings,
    row_cols: usize,
    value_cols: usize,
    row_subtotals: bool,
    rows: Vec<GridRow>,
    refs: Vec<MatrixRowRef>,
    /// The node whose values each row shows (`None`: empty cells).
    nodes: Vec<Option<&'m MatrixNode>>,
}

impl<'m> RowBuilder<'_, 'm> {
    fn push(
        &mut self,
        mut cells: Vec<GridCell>,
        kind: RowKind,
        r: MatrixRowRef,
        node: Option<&'m MatrixNode>,
    ) {
        let index = self.rows.len();
        for (c, cell) in cells.iter_mut().enumerate() {
            cell.hit = Some((c, index));
        }
        self.rows.push(GridRow { cells, kind });
        self.refs.push(r);
        self.nodes.push(node);
    }

    fn value_cells(&self, node: &MatrixNode, blank: bool) -> Vec<GridCell> {
        (0..self.value_cols)
            .map(|k| {
                let text = if blank {
                    String::new()
                } else {
                    node.cells
                        .get(k)
                        .map(|c| c.formatted.clone())
                        .unwrap_or_default()
                };
                GridCell::text(text)
            })
            .collect()
    }

    fn walk(&mut self, node: &'m MatrixNode, depth: usize, labels: &[String], path: &[CellValue]) {
        if depth > 64 {
            return;
        }
        for child in &node.children {
            let text = label(child.value.as_ref());
            let mut child_labels = labels.to_vec();
            child_labels.push(text.clone());
            let mut child_path = path.to_vec();
            child_path.push(child.value.clone().unwrap_or_else(CellValue::blank));
            let has_children = !child.children.is_empty();
            let expanded = has_children && self.s.expansion.expanded(depth, &child_labels);
            let mut header = vec![GridCell::default(); self.row_cols];
            let col = if self.s.stepped { 0 } else { depth };
            if let Some(cell) = header.get_mut(col.min(self.row_cols.saturating_sub(1))) {
                cell.text = text;
                if self.s.stepped {
                    cell.indent = depth as f64 * self.s.indent;
                }
                if self.s.buttons && has_children {
                    cell.expander = Some(expanded);
                } else if self.s.buttons && self.s.stepped {
                    // Leaves keep the expander's space so levels line up.
                    cell.indent += EXPANDER_LEAD;
                }
            }
            for cell in &mut header {
                cell.row_header = true;
            }
            let stepped_subtotal = expanded && self.s.stepped;
            let blank = expanded && (!self.s.stepped || !self.row_subtotals);
            let mut cells = header;
            cells.extend(self.value_cells(child, blank));
            let kind = if stepped_subtotal && self.row_subtotals {
                RowKind::Subtotal
            } else {
                RowKind::Data
            };
            self.push(
                cells,
                kind,
                MatrixRowRef {
                    level: child.level.or(Some(depth)),
                    path: child_path.clone(),
                    subtotal: kind == RowKind::Subtotal,
                },
                (!blank).then_some(child),
            );
            if expanded {
                self.walk(child, depth + 1, &child_labels, &child_path);
                if !self.s.stepped && self.row_subtotals {
                    let mut cells = vec![GridCell::default(); self.row_cols];
                    if let Some(cell) = cells.get_mut(depth.min(self.row_cols.saturating_sub(1))) {
                        cell.text = TOTAL_LABEL.to_owned();
                    }
                    for cell in &mut cells {
                        cell.row_header = true;
                    }
                    cells.extend(self.value_cells(child, false));
                    self.push(
                        cells,
                        RowKind::Subtotal,
                        MatrixRowRef {
                            level: None,
                            path: child_path.clone(),
                            subtotal: true,
                        },
                        Some(child),
                    );
                }
            }
        }
    }
}
