//! Power BI-shaped DataView types (single, categorical, table, matrix)
//! consumed by visuals. (owner: m4-semantic-query)
//!
//! A [`DataView`] is the result of querying one visual
//! ([`crate::query::query_visual`]). It mirrors the Power BI visuals API
//! contract so built-in visuals (and, later, a custom-visual host) read
//! data the same way:
//!
//! | Kind | Filled field | Used by |
//! |---|---|---|
//! | [`DataViewKind::Empty`] | nothing | `textbox`, `shape`, `actionButton`, `image`, groups |
//! | [`DataViewKind::Single`] | [`DataView::single`] | cards with one field |
//! | [`DataViewKind::Categorical`] | [`DataView::categorical`] (+ [`DataView::slicer`]) | slicers, pie/donut, bar/column, line/area, other charts |
//! | [`DataViewKind::Table`] | [`DataView::table`] | `tableEx`, `multiRowCard`, cards with several fields, unknown visual types |
//! | [`DataViewKind::Matrix`] | [`DataView::matrix`] | `pivotTable` |
//!
//! Every column is described once in [`DataView::metadata`]
//! ([`ColumnMeta`]); data structures refer to it by index (`meta_idx`).
//! `display_name` is what the visual shows (projection `displayName`,
//! else the native query ref, else the field name); `source` keeps the
//! model name for tooltips. `query_ref` is the key used by formatting
//! `objects` selectors (`{"metadata": "Products.Category"}`).
//!
//! Cells are [`CellValue`]s: the engine value plus its display text,
//! formatted with the measure's / column's format string in the model
//! culture. BLANK formats as `""` in value cells and as `(Blank)` in
//! single-value cards and category, slicer and matrix header cells, as
//! Power BI does. A measure with a dynamic format string (a calculation
//! item's or its own `formatStringDefinition`) is formatted per cell with
//! its effective format string, kept in [`CellValue::format`] when it
//! differs from [`ColumnMeta::format_string`].
//!
//! **Hidden fields.** Conditional formatting rules based on another field
//! (a gradient "based on" `[Margin]`, a field-value color measure) query
//! that field as a hidden value: its metadata is appended to
//! [`DataView::metadata`] with [`ColumnMeta::hidden`] set and no roles,
//! and its values travel next to the visible data ([`TableData::hidden`],
//! [`Categorical::hidden`], [`Matrix::hidden`] / [`MatrixNode::hidden`]),
//! never in the visible cells.

pub mod value_serde;

use pbi_engine::Value;
use pbi_model::{AggregationFunction, DataType, SortDirection};
use serde::{Deserialize, Serialize};

/// Display text of a BLANK category / group value.
pub const BLANK_LABEL: &str = "(Blank)";

/// Which part of a [`DataView`] is filled.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DataViewKind {
    /// No data (visual without a query).
    #[default]
    Empty,
    Single,
    Categorical,
    Table,
    Matrix,
}

/// A value with its display text.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CellValue {
    /// The engine value (BLANK serializes as `null`, dates as ISO text).
    #[serde(with = "value_serde")]
    pub raw: Value,
    /// Formatted text (format string + culture).
    pub formatted: String,
    /// The effective format string of this cell when it differs from the
    /// column's [`ColumnMeta::format_string`] (dynamic format strings of
    /// calculation items and measures; `""`: no format string).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format: Option<String>,
}

impl CellValue {
    pub fn new(raw: Value, formatted: impl Into<String>) -> Self {
        Self {
            raw,
            formatted: formatted.into(),
            format: None,
        }
    }

    /// The cell with its own effective format string (see
    /// [`CellValue::format`]).
    pub fn with_format(mut self, format: impl Into<String>) -> Self {
        self.format = Some(format.into());
        self
    }

    /// The format string to format this cell's value with: its own
    /// ([`CellValue::format`]; `None` for `""`), else `column`'s.
    pub fn format_or<'a>(&'a self, column: Option<&'a str>) -> Option<&'a str> {
        match self.format.as_deref() {
            Some("") => None,
            Some(f) => Some(f),
            None => column,
        }
    }

    /// BLANK with empty text.
    pub fn blank() -> Self {
        Self::new(Value::Blank, "")
    }

    pub fn is_blank(&self) -> bool {
        self.raw.is_blank()
    }
}

/// What a column refers to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FieldRefKind {
    Column,
    Measure,
    /// Implicit measure: an aggregation of a column (`Sum(T.C)`), or a
    /// column summarized by default in a value role.
    Aggregation,
    HierarchyLevel,
    Other,
}

/// The model field behind a column (for tooltips and cross-filtering).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FieldRef {
    pub kind: FieldRefKind,
    /// Table (measure: home table).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entity: Option<String>,
    /// Column / measure name (the source name, before any display name).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub property: Option<String>,
    /// Aggregation applied to the column.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub aggregation: Option<AggregationFunction>,
    /// Hierarchy of a hierarchy level.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hierarchy: Option<String>,
    /// DAX-style source name for tooltips: `Orders[Quantity]`.
    pub source_name: String,
}

/// One column of a [`DataView`] (one per distinct projection).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ColumnMeta {
    /// Data roles the field is bound to (`Category`, `Y`, `Values`, …).
    pub roles: Vec<String>,
    /// Projection `queryRef` (key of `objects` selectors).
    pub query_ref: String,
    /// Label to show.
    pub display_name: String,
    pub source: FieldRef,
    /// Measure or implicit measure (aggregation): a value, not a group.
    pub is_measure: bool,
    /// Format string used for the formatted values.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub format_string: Option<String>,
    /// Model data type (columns), or the type of the results (measures).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data_type: Option<DataType>,
    /// DAX evaluated for the column: the measure expression, or the
    /// generated aggregation (`SUM(Orders[Quantity])`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expression: Option<String>,
    /// Sort applied on this column (first sort key first).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sort: Option<SortDirection>,
    /// Position of this column among the sort keys (0 = primary).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sort_order: Option<usize>,
    /// A hidden value queried for conditional formatting (no roles; its
    /// values are in the `hidden` lists of the data, not in the cells).
    #[serde(default, skip_serializing_if = "is_false")]
    pub hidden: bool,
}

fn is_false(b: &bool) -> bool {
    !*b
}

/// The values of one hidden field ([`ColumnMeta::hidden`]), aligned with
/// the rows of a [`TableData`] or the categories of a [`Categorical`].
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HiddenValues {
    pub meta_idx: usize,
    /// One value per row / category.
    pub values: Vec<CellValue>,
    /// The value in the totals row (tables).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total: Option<CellValue>,
}

/// A category (group) column of a categorical DataView.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CategoryColumn {
    pub meta_idx: usize,
    /// One value per category row.
    pub values: Vec<CellValue>,
}

/// A value (measure) column of a categorical DataView, for one series.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ValueColumn {
    pub meta_idx: usize,
    /// Series (legend) value this column belongs to, when the visual has a
    /// series field.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub series: Option<CellValue>,
    /// One value per category row (BLANK where the series has no data).
    pub values: Vec<CellValue>,
    /// Highlighted values (highlight cross-filtering; not produced yet:
    /// highlight interactions are applied as filters).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub highlights: Option<Vec<CellValue>>,
}

/// Categorical data: aligned category and value columns.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Categorical {
    pub categories: Vec<CategoryColumn>,
    /// Value columns: for each series (in series order), one column per
    /// value field (in projection order).
    pub values: Vec<ValueColumn>,
    /// Metadata index of the series (legend) field.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub series_meta_idx: Option<usize>,
    /// Hidden fields, one value per category (charts without a series
    /// field).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub hidden: Vec<HiddenValues>,
}

impl Categorical {
    /// Number of category rows.
    pub fn len(&self) -> usize {
        self.categories
            .first()
            .map(|c| c.values.len())
            .or_else(|| self.values.first().map(|v| v.values.len()))
            .unwrap_or(0)
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// Tabular data (`tableEx`, multi-row card).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TableData {
    /// Metadata indices of the columns, in display order.
    pub columns: Vec<usize>,
    /// Rows (sorted), one cell per column.
    pub rows: Vec<Vec<CellValue>>,
    /// Totals row: measures re-evaluated without the grouping columns
    /// (group columns are BLANK). `None` when the table has no value
    /// column or totals are switched off.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub totals: Option<Vec<CellValue>>,
    /// Hidden fields, one value per row (and the totals row's).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub hidden: Vec<HiddenValues>,
}

/// A column header of a [`Matrix`] (flattened, in display order).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MatrixColumn {
    /// Values of the column levels down to this header (shorter than the
    /// number of column levels for subtotals; empty for the grand total).
    pub path: Vec<CellValue>,
    /// A subtotal or grand-total column.
    pub is_total: bool,
}

/// A row-hierarchy node of a [`Matrix`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MatrixNode {
    /// Row level of this node (`None` for the root / grand-total row).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub level: Option<usize>,
    /// The group value (`None` for the root).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<CellValue>,
    /// Cells: `cells[c * value_count + v]` for column `c` of
    /// [`Matrix::columns`] and value field `v`. Inner nodes hold subtotals,
    /// the root the grand totals.
    pub cells: Vec<CellValue>,
    /// Child groups (next row level), sorted.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<MatrixNode>,
    /// Hidden fields: `hidden[c * hidden_count + h]` for column `c` of
    /// [`Matrix::columns`] and hidden field `h` of [`Matrix::hidden`].
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub hidden: Vec<CellValue>,
}

/// Matrix (`pivotTable`) data.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Matrix {
    /// Metadata indices of the row levels.
    pub row_levels: Vec<usize>,
    /// Metadata indices of the column levels.
    pub column_levels: Vec<usize>,
    /// Metadata indices of the value fields.
    pub values: Vec<usize>,
    /// Column headers (leaves, then subtotals after their group, grand
    /// total last when column subtotals are on).
    pub columns: Vec<MatrixColumn>,
    /// Row hierarchy; the root is the grand-total row.
    pub rows: MatrixNode,
    /// Show subtotal rows and the grand-total row (`subTotals.rowSubtotals`;
    /// their cells are always computed).
    pub row_subtotals: bool,
    /// Column subtotals and the grand-total column are in
    /// [`Matrix::columns`] (`subTotals.columnSubtotals`).
    pub column_subtotals: bool,
    /// Metadata indices of the hidden fields ([`MatrixNode::hidden`]).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub hidden: Vec<usize>,
}

/// Slicer state shown with its value list.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SlicerData {
    /// Indices of the selected rows of [`DataView::categorical`] (for an
    /// inverted selection: the excluded rows).
    pub selected: Vec<usize>,
    /// The selection lists excluded values ("select all" minus these).
    pub inverted: bool,
    /// Nothing is filtered (no selection, or an inverted selection with
    /// nothing excluded).
    pub all_selected: bool,
    /// Selected values, including ones not in the current list.
    pub values: Vec<Vec<CellValue>>,
}

/// Where a query warning comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum WarningScope {
    Report,
    Page,
    Visual,
    Slicer,
    CrossFilter,
    Interaction,
    Query,
}

/// Something that was not applied exactly (never silently dropped).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QueryWarning {
    pub scope: WarningScope,
    /// Filter name or source visual name.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    /// Field involved, as a query ref.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub field: Option<String>,
    /// User-facing message.
    pub message: String,
    /// `true` if the filter was not applied at all (as opposed to applied
    /// approximately).
    pub not_applied: bool,
}

impl QueryWarning {
    pub fn new(scope: WarningScope, message: impl Into<String>) -> Self {
        Self {
            scope,
            source: None,
            field: None,
            message: message.into(),
            not_applied: false,
        }
    }

    pub fn with_source(mut self, source: impl Into<String>) -> Self {
        self.source = Some(source.into());
        self
    }

    pub fn with_field(mut self, field: Option<String>) -> Self {
        self.field = field;
        self
    }

    pub fn not_applied(mut self) -> Self {
        self.not_applied = true;
        self
    }
}

/// The data of one visual.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DataView {
    pub kind: DataViewKind,
    pub metadata: Vec<ColumnMeta>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub single: Option<CellValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub categorical: Option<Categorical>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub table: Option<TableData>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub matrix: Option<Matrix>,
    /// Slicers: the current selection.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub slicer: Option<SlicerData>,
    /// Rows, categories or series were cut at the query limits.
    pub truncated: bool,
    /// Filters / interactions not applied (or applied approximately).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<QueryWarning>,
}

impl DataView {
    /// A DataView without data (visuals that do not query).
    pub fn empty() -> Self {
        Self::default()
    }

    /// Metadata of column `idx`.
    pub fn meta(&self, idx: usize) -> Option<&ColumnMeta> {
        self.metadata.get(idx)
    }

    /// Index of the column with `query_ref`.
    pub fn meta_index(&self, query_ref: &str) -> Option<usize> {
        self.metadata.iter().position(|m| m.query_ref == query_ref)
    }

    /// Metadata indices bound to `role`.
    pub fn role_columns(&self, role: &str) -> Vec<usize> {
        self.metadata
            .iter()
            .enumerate()
            .filter(|(_, m)| m.roles.iter().any(|r| r == role))
            .map(|(i, _)| i)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    #[test]
    fn cells_serialize_raw_values_as_json() {
        let cell = CellValue::new(Value::Integer(2024), "2,024");
        let json = serde_json::to_value(&cell).unwrap();
        assert_eq!(json, serde_json::json!({"raw": 2024, "formatted": "2,024"}));
        let blank = serde_json::to_value(CellValue::blank()).unwrap();
        assert_eq!(blank, serde_json::json!({"raw": null, "formatted": ""}));
        let back: CellValue = serde_json::from_value(json).unwrap();
        assert_eq!(back, cell);
    }

    #[test]
    fn empty_dataview_serializes_compactly() {
        let json = serde_json::to_value(DataView::empty()).unwrap();
        assert_eq!(
            json,
            serde_json::json!({"kind": "empty", "metadata": [], "truncated": false})
        );
    }
}
