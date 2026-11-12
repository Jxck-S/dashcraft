//! A visual's data as a table: the summarized data that "Show as a table"
//! shows and "Export data" writes, and the underlying rows.
//!
//! Summarized ([`QueryEnv::summarized_data`]): the visual's fields (the
//! shown drill levels, with its sort, filters, drill path and the page's
//! slicers and cross-filter) queried as a table visual
//! ([`data_table_visual`]); totals are not included.
//!
//! Underlying ([`QueryEnv::underlying_data`]): the rows of the visual's base
//! table that are visible in its filter context. The base table is the
//! table of its first aggregated column, else the first table a measure's
//! DAX refers to, else the table of its first column. The columns are the
//! visual's columns (those of other tables through `RELATED`, dropped with
//! a warning when no relationship leads there) followed by the base
//! table's other visible columns. Measure filters do not apply to
//! underlying rows (a warning says so).
//!
//! The data table of focus mode's "Show as a table" is named
//! [`data_table_name`] (`<visual>~table`); filters, interactions and drill
//! treat it as its source visual ([`source_name`]).

use pbi_dax::VirtualTable;
use pbi_model::{
    FieldExpr, Model, Page, Projection, Report, Visual, dax_column_ref, dax_table_ref,
};
use serde::Serialize;

use super::exec::format_cell;
use super::{InteractionState, QueryEnv, QueryLimits, VisualQueryError, drill};
use crate::dataview::CellValue;

/// Suffix of the name of a visual's data table in focus mode.
pub const DATA_TABLE_SUFFIX: &str = "~table";

/// Name of the data table of `visual` in focus mode's "Show as a table".
pub fn data_table_name(visual: &str) -> String {
    format!("{visual}{DATA_TABLE_SUFFIX}")
}

/// The visual a data table shows (`name` itself for other visuals).
pub fn source_name(name: &str) -> &str {
    name.strip_suffix(DATA_TABLE_SUFFIX)
        .filter(|n| !n.is_empty())
        .unwrap_or(name)
}

/// `visual` as a table visual (`tableEx`) over its fields: every role's
/// active projections (all when none is marked active) as `Values`, each
/// field once, in role order; formatting is the table's default.
pub fn data_table_visual(visual: &Visual) -> Visual {
    let mut table = visual.clone();
    table.visual_type = "tableEx".to_owned();
    table.objects = serde_json::Value::Null;
    table.container_objects = serde_json::Value::Null;
    table.annotations.clear();
    let mut projections: Vec<Projection> = Vec::new();
    for role in visual.roles() {
        let all: Vec<&Projection> = visual.projections_for(role).collect();
        let any_active = all.iter().any(|p| p.active == Some(true));
        for p in all {
            if (any_active && p.active != Some(true))
                || projections.iter().any(|q| q.query_ref == p.query_ref)
            {
                continue;
            }
            let mut p = p.clone();
            p.role = "Values".to_owned();
            p.active = None;
            projections.push(p);
        }
    }
    table.projections = projections;
    table
}

/// A visual's data as rows.
#[derive(Debug, Clone, PartialEq, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VisualTable {
    /// Column headers (display names).
    pub headers: Vec<String>,
    /// Rows, one cell per header.
    pub rows: Vec<Vec<CellValue>>,
    /// Rows were cut at the query limits.
    pub truncated: bool,
    /// Filters not applied, columns dropped.
    pub warnings: Vec<String>,
}

impl QueryEnv {
    /// The summarized data of `visual` (see the module docs).
    ///
    /// # Errors
    /// [`VisualQueryError`] when the table cannot be evaluated.
    pub fn summarized_data(
        &self,
        report: &Report,
        page: &Page,
        visual: &Visual,
        state: &InteractionState,
        limits: &QueryLimits,
    ) -> Result<VisualTable, VisualQueryError> {
        let drilled = drill::drilled_visual(visual, state);
        let table = data_table_visual(&drilled);
        let dv = self.query_visual(report, page, &table, state, limits)?;
        let mut out = VisualTable {
            truncated: dv.truncated,
            warnings: dv.warnings.iter().map(|w| w.message.clone()).collect(),
            ..VisualTable::default()
        };
        if let Some(data) = &dv.table {
            out.headers = data
                .columns
                .iter()
                .map(|&mi| {
                    dv.meta(mi)
                        .map(|m| m.display_name.clone())
                        .unwrap_or_default()
                })
                .collect();
            out.rows.clone_from(&data.rows);
        }
        Ok(out)
    }

    /// The underlying rows of `visual` (see the module docs).
    ///
    /// # Errors
    /// [`VisualQueryError`] when the visual has no table to read or the
    /// rows cannot be evaluated.
    pub fn underlying_data(
        &self,
        report: &Report,
        page: &Page,
        visual: &Visual,
        state: &InteractionState,
        limits: &QueryLimits,
    ) -> Result<VisualTable, VisualQueryError> {
        let drilled = drill::drilled_visual(visual, state);
        let visual = &*drilled;
        let builder = self.builder();
        let base = base_table(&self.model, visual).ok_or_else(|| {
            VisualQueryError::new(visual, "the visual has no table with underlying rows")
        })?;
        let (filter, warnings) = self.page_filter_context(report, page, visual, state);
        let mut out = VisualTable {
            warnings: warnings.into_iter().map(|w| w.message).collect(),
            ..VisualTable::default()
        };
        // (header, DAX, format string, related)
        let mut columns: Vec<(String, String, Option<String>, bool)> = Vec::new();
        let push = |columns: &mut Vec<(String, String, Option<String>, bool)>,
                    table: &str,
                    column: &str,
                    header: String| {
            let dax = dax_column_ref(table, column);
            if columns.iter().any(|c| c.1 == dax) {
                return;
            }
            let format = self
                .model
                .table(table)
                .and_then(|t| t.column(column))
                .and_then(|c| c.format_string.clone());
            let related = !table.eq_ignore_ascii_case(&base);
            columns.push((header, dax, format, related));
        };
        let shown = visual
            .projections
            .iter()
            .filter(|p| p.active == Some(true) || !has_active(visual, &p.role));
        for p in shown {
            let field = match &p.field {
                FieldExpr::Aggregation { expr, .. } => expr.as_ref(),
                other => other,
            };
            if let Ok(col) = builder.resolve_column(field) {
                let header = if matches!(p.field, FieldExpr::Aggregation { .. }) {
                    col.column.clone()
                } else {
                    p.label().to_owned()
                };
                push(&mut columns, &col.table, &col.column, header);
            }
        }
        if let Some(t) = self.model.table(&base) {
            let loaded = self.snapshot.table(&t.name).ok();
            for c in t.columns.iter().filter(|c| !c.is_hidden) {
                if loaded.as_ref().is_some_and(|d| d.has_column(&c.name)) {
                    push(&mut columns, &t.name, &c.name, c.name.clone());
                }
            }
        }
        let query = |cols: &[&(String, String, Option<String>, bool)]| {
            let args: Vec<String> = cols
                .iter()
                .enumerate()
                .map(|(i, (_, dax, _, related))| {
                    let expr = if *related {
                        format!("RELATED({dax})")
                    } else {
                        dax.clone()
                    };
                    format!("\"c{i}\", {expr}")
                })
                .collect();
            format!(
                "EVALUATE SELECTCOLUMNS({}, {})",
                dax_table_ref(&base),
                args.join(", ")
            )
        };
        let ctx = self.eval_context(&filter, limits);
        let all: Vec<_> = columns.iter().collect();
        let (used, tables) = match ctx.evaluate_query(&query(&all)) {
            Ok(t) => (all, t),
            Err(_) if all.iter().any(|c| c.3) => {
                let own: Vec<_> = columns.iter().filter(|c| !c.3).collect();
                for c in columns.iter().filter(|c| c.3) {
                    out.warnings.push(format!(
                        "{} is not related to the rows of {base} and is left out",
                        c.1
                    ));
                }
                let tables = ctx
                    .evaluate_query(&query(&own))
                    .map_err(|e| VisualQueryError::new(visual, e.to_string()))?;
                (own, tables)
            }
            Err(e) => return Err(VisualQueryError::new(visual, e.to_string())),
        };
        if limits.cancel.is_cancelled() {
            return Err(VisualQueryError::new(visual, "cancelled").cancelled());
        }
        out.headers = used.iter().map(|c| c.0.clone()).collect();
        let rows = tables.first().map(VirtualTable::rows).unwrap_or_default();
        out.truncated = rows.len() > limits.max_rows;
        out.rows = rows
            .into_iter()
            .take(limits.max_rows)
            .map(|row| {
                row.into_iter()
                    .zip(&used)
                    .map(|(v, c)| format_cell(&self.culture, v, c.2.as_deref(), false))
                    .collect()
            })
            .collect();
        Ok(out)
    }
}

/// `role` of `visual` marks some projections active.
fn has_active(visual: &Visual, role: &str) -> bool {
    visual.projections_for(role).any(|p| p.active == Some(true))
}

/// The table whose rows are the visual's underlying data (see the module
/// docs).
fn base_table(model: &Model, visual: &Visual) -> Option<String> {
    let table_of = |entity: &str| model.table(entity).map(|t| t.name.clone());
    for p in &visual.projections {
        if let FieldExpr::Aggregation { expr, .. } = &p.field
            && let Some(t) = expr.entity().and_then(table_of)
        {
            return Some(t);
        }
    }
    for p in &visual.projections {
        if let FieldExpr::Measure { entity, property } = &p.field
            && let Some(m) = model.measure(entity, property)
            && let Some(t) = first_table_in(model, &m.expression)
        {
            return Some(t);
        }
    }
    visual.projections.iter().find_map(|p| match &p.field {
        FieldExpr::Column { entity, .. } | FieldExpr::HierarchyLevel { entity, .. } => {
            table_of(entity)
        }
        _ => None,
    })
}

/// The first table (with columns) a DAX expression refers to, as
/// `'Table'`, `Table[Column]` or a bare `Table`.
fn first_table_in(model: &Model, dax: &str) -> Option<String> {
    model
        .tables
        .iter()
        .filter(|t| !t.columns.is_empty())
        .filter_map(|t| {
            let quoted = format!("'{}'", t.name.replace('\'', "''"));
            let plain = t.name.clone();
            let at = [quoted, plain]
                .iter()
                .filter_map(|needle| find_reference(dax, needle))
                .min()?;
            Some((at, t.name.clone()))
        })
        .min()
        .map(|(_, name)| name)
}

/// Position of `needle` in `dax` not preceded or followed by an identifier
/// character (so `Sales` does not match inside `OnlineSales`, `SalesYTD` or
/// the measure reference `[Sales]`) and outside string literals.
fn find_reference(dax: &str, needle: &str) -> Option<usize> {
    let ident = |c: char| c.is_alphanumeric() || c == '_' || c == '\'';
    let lower = dax.to_lowercase();
    let needle = needle.to_lowercase();
    let mut from = 0;
    while let Some(i) = lower.get(from..)?.find(&needle) {
        let at = from + i;
        let end = at + needle.len();
        let before = lower[..at].chars().next_back();
        let after = lower[end..].chars().next();
        let in_string = lower[..at].matches('"').count() % 2 == 1;
        let bounded_before =
            needle.starts_with('\'') || !before.is_some_and(|c| ident(c) || c == '[');
        let bounded_after = needle.ends_with('\'') || !after.is_some_and(ident);
        if bounded_before && bounded_after && !in_string {
            return Some(at);
        }
        from = end;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_references_skip_longer_names() {
        assert_eq!(find_reference("SUM(OnlineSales[x])", "sales"), None);
        assert_eq!(
            find_reference("SUM(OnlineSales[x]) + Sales[y]", "sales"),
            Some(22)
        );
        assert_eq!(
            find_reference("SUM('Web Sales'[x])", "'web sales'"),
            Some(4)
        );
        assert_eq!(find_reference("COUNTROWS(Sales)", "sales"), Some(10));
        assert_eq!(
            find_reference("[SalesYTD] & \"Sales\" & [Sales]", "sales"),
            None
        );
        assert_eq!(source_name("abc~table"), "abc");
        assert_eq!(source_name("~table"), "~table");
        assert_eq!(source_name("abc"), "abc");
    }
}
