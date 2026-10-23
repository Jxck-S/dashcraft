//! Query execution: DAX generation (`SUMMARIZECOLUMNS` / `ROW`),
//! evaluation with the visual's filter context, sorting, limits and
//! DataView assembly per query shape.

use std::collections::HashMap;

use indexmap::IndexMap;
use pbi_dax::DaxError;
use pbi_engine::{Value, ValueKey};
use pbi_model::names::names_eq;
use pbi_model::{Condition, DataType, FieldExpr, Model, Page, Report, SortDirection, Visual};

use crate::conditional::CfSpec;
use crate::dataview::{
    BLANK_LABEL, Categorical, CategoryColumn, CellValue, DataView, DataViewKind, FieldRefKind,
    HiddenValues, Matrix, MatrixColumn, MatrixNode, QueryWarning, SlicerData, TableData,
    ValueColumn, WarningScope,
};

use super::context::{VisualFilters, visual_filters};
use super::filter::{ColumnRef, Pred, coerce_value, compile_pred};
use super::interaction::InteractionState;
use super::plan::{
    PlannedField, RoleKind, Shape, SortKey, ValueExpr, plan_cf_inputs, plan_fields, same_field,
    shape_of, sort_keys, sorts_by_value_by_default, value_field,
};
use super::{QueryEnv, QueryLimits, VisualQueryError};

type Row = Vec<Value>;

/// DAX query for a grouped evaluation: `SUMMARIZECOLUMNS(groups…, "v0",
/// expr0, …)`, or `ROW("v0", expr0, …)` without groups.
pub(crate) fn summarize_dax(group: &[&ColumnRef], values: &[&str]) -> String {
    let mut args: Vec<String> = group.iter().map(|c| c.dax()).collect();
    for (i, v) in values.iter().enumerate() {
        args.push(format!("\"v{i}\", {v}"));
    }
    if group.is_empty() {
        format!("EVALUATE ROW({})", args.join(", "))
    } else {
        format!("EVALUATE SUMMARIZECOLUMNS({})", args.join(", "))
    }
}

/// A formatted cell. BLANK is `(Blank)` for categories / headers and empty
/// for values.
pub(crate) fn format_cell(
    culture: &str,
    v: Value,
    format: Option<&str>,
    category: bool,
) -> CellValue {
    if v.is_blank() {
        return CellValue::new(Value::Blank, if category { BLANK_LABEL } else { "" });
    }
    let text = pbi_dax::format_value(&v, format, culture);
    CellValue::new(v, text)
}

fn data_type_of(v: &Value) -> Option<DataType> {
    Some(match v {
        Value::Blank => return None,
        Value::Boolean(_) => DataType::Boolean,
        Value::Integer(_) => DataType::Int64,
        Value::Double(_) => DataType::Double,
        Value::Decimal(_) => DataType::Decimal,
        Value::String(_) => DataType::String,
        Value::DateTime(_) => DataType::DateTime,
    })
}

fn keys_of(values: &[Value]) -> Vec<ValueKey> {
    values.iter().map(ValueKey::from).collect()
}

fn sort_rows(rows: &mut [Row], keys: &[(usize, bool)]) {
    if keys.is_empty() {
        return;
    }
    rows.sort_by(|a, b| {
        for &(i, desc) in keys {
            let (x, y) = (
                a.get(i).unwrap_or(&Value::Blank),
                b.get(i).unwrap_or(&Value::Blank),
            );
            let o = x.total_cmp(y);
            let o = if desc { o.reverse() } else { o };
            if o.is_ne() {
                return o;
            }
        }
        std::cmp::Ordering::Equal
    });
}

/// Measure conditions applied to the visual's rows.
struct Having {
    pred: Pred,
    exprs: Vec<ValueExpr>,
}

/// Columns of one grouped query.
struct Layout {
    groups: Vec<ColumnRef>,
    values: Vec<ValueExpr>,
    /// Field indices of the visible groups (row positions `0..`).
    group_fields: Vec<usize>,
    /// Field indices of the visible values (row positions after all groups).
    value_fields: Vec<usize>,
    /// Start of the `Having` operands in `values`.
    having_at: Option<usize>,
    /// Row index, descending.
    sort: Vec<(usize, bool)>,
    /// Value fields with a dynamic format string: their effective format
    /// strings follow the values in each row (text, BLANK for none).
    formats: Vec<usize>,
}

impl Layout {
    /// Row position of the effective format string of value field `field`.
    fn format_pos(&self, field: usize) -> Option<usize> {
        self.formats
            .iter()
            .position(|&f| f == field)
            .map(|k| self.groups.len() + self.values.len() + k)
    }

    fn group_pos(&self, field: usize) -> Option<usize> {
        self.group_fields.iter().position(|&f| f == field)
    }

    fn value_pos(&self, field: usize) -> Option<usize> {
        self.value_fields
            .iter()
            .position(|&f| f == field)
            .map(|q| self.groups.len() + q)
    }

    /// Row position of a visible field.
    fn pos(&self, field: usize) -> Option<usize> {
        self.group_pos(field).or_else(|| self.value_pos(field))
    }
}

pub(crate) struct Exec<'a> {
    env: &'a QueryEnv,
    visual: &'a Visual,
    limits: &'a QueryLimits,
    /// The projected fields, then the hidden conditional-formatting inputs.
    fields: Vec<PlannedField>,
    /// Number of projected (visible) fields.
    visible: usize,
    filters: VisualFilters,
    having: Option<Having>,
    truncated: bool,
    warnings: Vec<QueryWarning>,
    /// Calculation groups that set format strings.
    calc_formats: Vec<CalcFormat>,
}

/// A calculation group with format string definitions.
struct CalcFormat {
    table: String,
    /// Its `noSelectionExpression` sets a format string (applies even when
    /// the group is not filtered).
    unfiltered: bool,
}

fn calc_formats(model: &Model) -> Vec<CalcFormat> {
    model
        .tables
        .iter()
        .filter(|t| t.calculation_group.is_some() || t.extra.contains_key("calculationGroup"))
        .filter_map(|t| {
            let g = t.effective_calculation_group()?;
            let item_formats = g
                .calculation_items
                .iter()
                .any(|i| i.format_string_definition.is_some())
                || g.multiple_or_empty_selection_expression
                    .as_ref()
                    .is_some_and(|e| e.format_string_definition.is_some());
            let unfiltered = g
                .no_selection_expression
                .as_ref()
                .is_some_and(|e| e.format_string_definition.is_some());
            (item_formats || unfiltered).then(|| CalcFormat {
                table: t.name.clone(),
                unfiltered,
            })
        })
        .collect()
}

/// Queries one visual (see [`super::query_visual`]).
pub(crate) fn run_visual(
    env: &QueryEnv,
    report: &Report,
    page: &Page,
    visual: &Visual,
    state: &InteractionState,
    limits: &QueryLimits,
) -> Result<DataView, VisualQueryError> {
    let shape = shape_of(visual);
    if shape == Shape::Empty {
        return Ok(DataView::empty());
    }
    if limits.cancel.is_cancelled() {
        return Err(VisualQueryError::new(visual, "the query was cancelled").cancelled());
    }
    let builder = env.builder();
    let mut fields = plan_fields(&env.model, &builder, visual, shape)
        .map_err(|e| VisualQueryError::new(visual, e.message).with_field(e.field))?;
    if fields.is_empty() {
        return Ok(DataView::empty());
    }
    let visible = fields.len();
    let filters = visual_filters(env, report, page, visual, state, limits);
    let mut warnings = filters.warnings.clone();
    let cf_warning = |m: String| QueryWarning::new(WarningScope::Visual, m).not_applied();
    let spec = CfSpec::parse(visual);
    warnings.extend(spec.diagnostics.iter().cloned().map(cf_warning));
    if matches!(shape, Shape::Table | Shape::Matrix | Shape::Chart) && !spec.is_empty() {
        let problems = plan_cf_inputs(&env.model, &builder, &spec, &mut fields);
        warnings.extend(problems.into_iter().map(cf_warning));
    }
    let mut exec = Exec {
        env,
        visual,
        limits,
        fields,
        visible,
        warnings,
        having: None,
        filters,
        truncated: false,
        calc_formats: calc_formats(&env.model),
    };
    exec.check_hidden()?;
    exec.build_having();
    let result = match shape {
        Shape::Card => exec.card(),
        Shape::Slicer => exec.slicer(state),
        Shape::Table => exec.table(),
        Shape::Matrix => exec.matrix(),
        Shape::Chart => exec.chart(),
        Shape::Empty => Ok(DataView::empty()),
    };
    match result {
        Ok(mut dv) => {
            dv.metadata = exec.fields.iter().map(|f| f.meta.clone()).collect();
            dv.truncated = exec.truncated;
            dv.warnings = exec.warnings;
            Ok(dv)
        }
        Err(mut e) => {
            e.warnings = exec.warnings;
            Err(e)
        }
    }
}

impl Exec<'_> {
    fn error(&self, message: impl Into<String>) -> VisualQueryError {
        VisualQueryError::new(self.visual, message)
    }

    fn indices(&self, kind: RoleKind) -> Vec<usize> {
        self.fields
            .iter()
            .take(self.visible)
            .enumerate()
            .filter(|(_, f)| f.kind == kind)
            .map(|(i, _)| i)
            .collect()
    }

    /// Indices of the hidden conditional-formatting inputs.
    fn hidden(&self) -> Vec<usize> {
        (self.visible..self.fields.len()).collect()
    }

    /// Drops hidden inputs whose expression cannot be evaluated (their
    /// rules do not apply; the visual still renders).
    fn check_hidden(&mut self) -> Result<(), VisualQueryError> {
        if self.fields.len() == self.visible {
            return Ok(());
        }
        let ctx = self.env.eval_context(&self.filters.context, self.limits);
        let mut i = self.visible;
        while i < self.fields.len() {
            let failed = match self.fields.get(i).and_then(PlannedField::value) {
                Some(v) => match ctx.evaluate_scalar(&v.dax) {
                    Ok(_) => None,
                    Err(DaxError::Cancelled) => {
                        return Err(self.error("the query was cancelled").cancelled());
                    }
                    Err(e) => Some(format!(
                        "conditional formatting based on {} not applied: {e}",
                        v.label
                    )),
                },
                None => Some("conditional formatting input is not a value".into()),
            };
            match failed {
                Some(message) => {
                    self.fields.remove(i);
                    self.warnings
                        .push(QueryWarning::new(WarningScope::Visual, message).not_applied());
                }
                None => i += 1,
            }
        }
        Ok(())
    }

    /// `true` if value field `f` may have a dynamic format string in a
    /// query grouped by `groups`: a measure with its own
    /// `formatStringDefinition`, or one a calculation group with format
    /// strings may apply to (its table is filtered or grouped, or it has a
    /// formatted `noSelectionExpression`). Implicit measures never do.
    fn format_may_vary(&self, f: usize, groups: &[ColumnRef]) -> bool {
        let Some(field) = self.fields.get(f).filter(|p| !p.meta.hidden) else {
            return false;
        };
        let src = &field.meta.source;
        if src.kind != FieldRefKind::Measure {
            return false;
        }
        let (Some(table), Some(name)) = (src.entity.as_deref(), src.property.as_deref()) else {
            return false;
        };
        if self
            .env
            .model
            .measure(table, name)
            .is_some_and(|m| m.format_string_definition().is_some())
        {
            return true;
        }
        self.calc_formats.iter().any(|g| {
            g.unfiltered
                || self.filters.context.is_table_filtered(&g.table)
                || groups.iter().any(|c| names_eq(&c.table, &g.table))
        })
    }

    /// Appends the effective format strings of `layout.formats` to every
    /// row (see [`Layout::formats`]), one batch per field.
    fn append_formats(&self, rows: &mut [Row], layout: &Layout) -> Result<(), VisualQueryError> {
        let columns: Vec<pbi_model::ColumnRef> = layout
            .groups
            .iter()
            .map(|c| pbi_model::ColumnRef::new(c.table.clone(), c.column.clone()))
            .collect();
        let g = layout.groups.len();
        let points: Vec<Vec<Value>> = rows
            .iter()
            .map(|r| r.get(..g).unwrap_or_default().to_vec())
            .collect();
        let ctx = self.env.eval_context(&self.filters.context, self.limits);
        for &f in &layout.formats {
            let Some(field) = self.fields.get(f) else {
                continue;
            };
            let formats = match field.value() {
                Some(v) => match ctx.evaluate_measure_format_strings(&v.dax, &columns, &points) {
                    Ok(formats) => formats,
                    Err(DaxError::Cancelled) => {
                        return Err(self.error("the query was cancelled").cancelled());
                    }
                    Err(_) => vec![field.meta.format_string.clone(); rows.len()],
                },
                None => vec![field.meta.format_string.clone(); rows.len()],
            };
            for (row, format) in rows.iter_mut().zip(formats) {
                row.push(format.map_or(Value::Blank, Value::from));
            }
        }
        Ok(())
    }

    fn build_having(&mut self) {
        let Some(cond) = Condition::and_all(self.filters.value_conditions.iter().cloned()) else {
            return;
        };
        let builder = self.env.builder();
        let mut exprs: Vec<(FieldExpr, ValueExpr)> = Vec::new();
        let compiled = {
            let mut resolve = |f: &FieldExpr| -> Result<(usize, Option<DataType>), String> {
                if let Some(i) = exprs.iter().position(|(e, _)| same_field(e, f)) {
                    return Ok((i, None));
                }
                let (expr, _) = value_field(&self.env.model, &builder, f, false)?;
                exprs.push((f.clone(), expr));
                Ok((exprs.len() - 1, None))
            };
            compile_pred(&cond, &mut resolve)
        };
        match compiled {
            Ok(pred) => {
                self.having = Some(Having {
                    pred,
                    exprs: exprs.into_iter().map(|(_, e)| e).collect(),
                });
            }
            Err(reason) => self.warnings.push(
                QueryWarning::new(
                    WarningScope::Visual,
                    format!("measure filter not applied: {reason}"),
                )
                .not_applied(),
            ),
        }
    }

    /// The model's sort-by column of a group column.
    fn sort_by_column(&self, col: &ColumnRef) -> Option<ColumnRef> {
        let sb = self
            .env
            .model
            .column(&col.table, &col.column)?
            .sort_by_column
            .clone()?;
        let resolved = self
            .env
            .builder()
            .resolve_column(&FieldExpr::column(col.table.clone(), sb))
            .ok()?;
        (!pbi_model::names_eq(&resolved.column, &col.column)).then_some(resolved)
    }

    /// For each field of `fields` (matrix row or column levels), the value
    /// of its model sort-by column per value, in the visual's filter
    /// context; `None` for a level without a sort-by column (or when the
    /// lookup fails: the level then sorts by its own values).
    fn level_orders(&self, fields: &[usize]) -> Vec<Option<HashMap<ValueKey, Value>>> {
        fields
            .iter()
            .map(|&f| {
                let col = self.fields.get(f)?.group()?;
                let sb = self.sort_by_column(col)?;
                let rows = self
                    .env
                    .summarize(&self.filters.context, &[col, &sb], &[], self.limits)
                    .ok()?;
                let mut order = HashMap::with_capacity(rows.len());
                for row in rows {
                    if let [v, by, ..] = row.as_slice() {
                        order
                            .entry(ValueKey::from(v))
                            .and_modify(|o: &mut Value| {
                                if by.total_cmp(o).is_lt() {
                                    *o = by.clone();
                                }
                            })
                            .or_insert_with(|| by.clone());
                    }
                }
                Some(order)
            })
            .collect()
    }

    fn layout(
        &self,
        group_fields: &[usize],
        value_fields: &[usize],
        sort: &[SortKey],
        with_having: bool,
    ) -> Layout {
        let mut groups: Vec<ColumnRef> = group_fields
            .iter()
            .filter_map(|&i| self.fields.get(i).and_then(PlannedField::group).cloned())
            .collect();
        let mut values: Vec<ValueExpr> = value_fields
            .iter()
            .filter_map(|&i| self.fields.get(i).and_then(PlannedField::value).cloned())
            .collect();
        let visible_groups = groups.len();
        enum Target {
            Group(usize),
            Hidden(usize),
            Value(usize),
        }
        let mut hidden: Vec<ColumnRef> = Vec::new();
        let mut targets = Vec::new();
        for k in sort {
            if let Some(p) = group_fields.iter().position(|&g| g == k.field) {
                let target = match groups.get(p).and_then(|c| self.sort_by_column(c)) {
                    Some(sb) => {
                        let h = match hidden.iter().position(|c| c.dax() == sb.dax()) {
                            Some(h) => h,
                            None => {
                                hidden.push(sb);
                                hidden.len() - 1
                            }
                        };
                        Target::Hidden(h)
                    }
                    None => Target::Group(p),
                };
                targets.push((target, k.descending));
            } else if let Some(q) = value_fields.iter().position(|&v| v == k.field) {
                targets.push((Target::Value(q), k.descending));
            }
        }
        groups.extend(hidden);
        let having_at = match (&self.having, with_having) {
            (Some(h), true) => {
                let at = values.len();
                values.extend(h.exprs.iter().cloned());
                Some(at)
            }
            _ => None,
        };
        let total_groups = groups.len();
        let mut sort: Vec<(usize, bool)> = targets
            .into_iter()
            .map(|(t, desc)| {
                let idx = match t {
                    Target::Group(p) => p,
                    Target::Hidden(h) => visible_groups + h,
                    Target::Value(q) => total_groups + q,
                };
                (idx, desc)
            })
            .collect();
        if !sort.is_empty() {
            // Deterministic ties: the remaining groups, ascending.
            for p in 0..visible_groups {
                if !sort.iter().any(|&(i, _)| i == p) {
                    sort.push((p, false));
                }
            }
        }
        let formats = value_fields
            .iter()
            .copied()
            .filter(|&f| self.format_may_vary(f, &groups))
            .collect();
        Layout {
            groups,
            values,
            group_fields: group_fields.to_vec(),
            value_fields: value_fields.to_vec(),
            having_at,
            sort,
            formats,
        }
    }

    fn run(&self, layout: &Layout) -> Result<Vec<Row>, VisualQueryError> {
        let groups: Vec<&ColumnRef> = layout.groups.iter().collect();
        let values: Vec<&str> = layout.values.iter().map(|v| v.dax.as_str()).collect();
        if groups.is_empty() && values.is_empty() {
            return Ok(Vec::new());
        }
        let mut rows =
            match self
                .env
                .summarize(&self.filters.context, &groups, &values, self.limits)
            {
                Ok(rows) => rows,
                Err(e) => return Err(self.diagnose(e, &summarize_dax(&groups, &values), layout)),
            };
        if let (Some(at), Some(h)) = (layout.having_at, &self.having) {
            let g = layout.groups.len();
            rows.retain(|r| h.pred.eval(r.get(g + at..).unwrap_or_default()));
            let visible = layout.value_fields.len();
            if visible > 0 && !groups.is_empty() {
                rows.retain(|r| {
                    r.get(g..g + visible)
                        .is_some_and(|vs| !vs.iter().all(Value::is_blank))
                });
            }
        }
        sort_rows(&mut rows, &layout.sort);
        if !layout.formats.is_empty() {
            self.append_formats(&mut rows, layout)?;
        }
        Ok(rows)
    }

    fn diagnose(&self, err: DaxError, query: &str, layout: &Layout) -> VisualQueryError {
        if matches!(err, DaxError::Cancelled) {
            return self.error("the query was cancelled").cancelled();
        }
        let ctx = self.env.eval_context(&self.filters.context, self.limits);
        for v in &layout.values {
            if let Err(e) = ctx.evaluate_scalar(&v.dax) {
                if matches!(e, DaxError::Cancelled) {
                    return self.error("the query was cancelled").cancelled();
                }
                let mut out = self
                    .error(e.to_string())
                    .with_field(v.label.clone())
                    .with_dax(v.source_dax.clone());
                out.unsupported = e.is_unsupported();
                return out;
            }
        }
        let mut out = self.error(err.to_string()).with_dax(query.to_owned());
        out.unsupported = err.is_unsupported();
        out
    }

    fn cell(&self, v: Value, field: usize, category: bool) -> CellValue {
        let format = self
            .fields
            .get(field)
            .and_then(|f| f.meta.format_string.as_deref());
        format_cell(&self.env.culture, v, format, category)
    }

    /// The cell of field `field` in a row of `layout` (formatted with its
    /// effective format string when it has a dynamic one).
    fn cell_at(&self, row: &[Value], layout: &Layout, field: usize, category: bool) -> CellValue {
        let v = layout
            .pos(field)
            .and_then(|p| row.get(p).cloned())
            .unwrap_or_default();
        self.cell_formatted(
            v,
            field,
            category,
            layout.format_pos(field).and_then(|p| row.get(p)),
        )
    }

    /// [`Exec::cell`] with an effective format string from the query
    /// (`Some(BLANK)`: none), recorded in the cell when it differs from
    /// the column's.
    fn cell_formatted(
        &self,
        v: Value,
        field: usize,
        category: bool,
        format: Option<&Value>,
    ) -> CellValue {
        let Some(format) = format else {
            return self.cell(v, field, category);
        };
        let dynamic = format.as_str();
        let base = self
            .fields
            .get(field)
            .and_then(|f| f.meta.format_string.as_deref());
        let cell = format_cell(&self.env.culture, v, dynamic, category);
        if dynamic != base && !cell.is_blank() {
            cell.with_format(dynamic.unwrap_or_default())
        } else {
            cell
        }
    }

    /// Hidden values per row of `rows` (and of the totals row).
    fn hidden_values(
        &self,
        hidden: &[usize],
        rows: &[Row],
        layout: &Layout,
        total: Option<(&[Value], &Layout)>,
    ) -> Vec<HiddenValues> {
        hidden
            .iter()
            .map(|&h| HiddenValues {
                meta_idx: h,
                values: rows
                    .iter()
                    .map(|r| self.cell_at(r, layout, h, false))
                    .collect(),
                total: total.map(|(r, l)| self.cell_at(r, l, h, false)),
            })
            .collect()
    }

    /// Fills missing measure data types from the results.
    fn infer_types(&mut self, rows: &[Row], layout: &Layout) {
        for &f in &layout.value_fields {
            let Some(pos) = layout.value_pos(f) else {
                continue;
            };
            let Some(field) = self.fields.get_mut(f) else {
                continue;
            };
            if field.meta.data_type.is_none() {
                field.meta.data_type = rows
                    .iter()
                    .filter_map(|r| r.get(pos))
                    .find_map(data_type_of);
            }
        }
    }

    fn mark_sort(&mut self, keys: &[SortKey]) {
        for (n, k) in keys.iter().enumerate() {
            if let Some(f) = self.fields.get_mut(k.field) {
                f.meta.sort = Some(if k.descending {
                    SortDirection::Descending
                } else {
                    SortDirection::Ascending
                });
                f.meta.sort_order = Some(n);
            }
        }
    }

    fn sort_keys(&mut self) -> Vec<SortKey> {
        let visible = self.fields.get(..self.visible).unwrap_or_default();
        let (keys, missing) = sort_keys(self.visual, visible);
        for m in missing {
            self.warnings.push(QueryWarning::new(
                WarningScope::Query,
                format!("sort by {m} ignored: the field is not in the visual"),
            ));
        }
        keys
    }

    fn truncate(&mut self, rows: &mut Vec<Row>, cap: usize) {
        if rows.len() > cap {
            rows.truncate(cap);
            self.truncated = true;
        }
    }

    // ---- shapes -------------------------------------------------------------

    fn card(&mut self) -> Result<DataView, VisualQueryError> {
        let values = self.indices(RoleKind::Value);
        let layout = self.layout(&[], &values, &[], true);
        let rows = self.run(&layout)?;
        self.infer_types(&rows, &layout);
        let row = rows.into_iter().next();
        if let [only] = values.as_slice() {
            let cell = match &row {
                Some(r) => self.cell_at(r, &layout, *only, true),
                None => self.cell(Value::Blank, *only, true),
            };
            return Ok(DataView {
                kind: DataViewKind::Single,
                single: Some(cell),
                ..DataView::default()
            });
        }
        let rows = row
            .map(|r| {
                values
                    .iter()
                    .map(|&f| self.cell_at(&r, &layout, f, false))
                    .collect()
            })
            .into_iter()
            .collect();
        Ok(DataView {
            kind: DataViewKind::Table,
            table: Some(TableData {
                columns: values,
                rows,
                totals: None,
                hidden: Vec::new(),
            }),
            ..DataView::default()
        })
    }

    fn slicer(&mut self, state: &InteractionState) -> Result<DataView, VisualQueryError> {
        let groups = self.indices(RoleKind::Category);
        let mut keys = self.sort_keys();
        if keys.is_empty() {
            keys = groups
                .iter()
                .map(|&field| SortKey {
                    field,
                    descending: false,
                })
                .collect();
        }
        self.mark_sort(&keys);
        let layout = self.layout(&groups, &[], &keys, true);
        let mut rows = self.run(&layout)?;
        self.truncate(&mut rows, self.limits.max_rows);
        let categories = self.category_columns(&rows, &layout);

        let cols: Vec<ColumnRef> = groups
            .iter()
            .filter_map(|&i| self.fields.get(i).and_then(PlannedField::group).cloned())
            .collect();
        let mut slicer = SlicerData {
            all_selected: true,
            ..SlicerData::default()
        };
        if let Some(sel) = state.slicer_selections.get(&self.visual.name) {
            let wanted: Vec<Vec<ValueKey>> = sel
                .values
                .iter()
                .map(|t| {
                    t.iter()
                        .zip(&cols)
                        .map(|(v, c)| ValueKey::from(&coerce_value(v, &c.data_type)))
                        .collect()
                })
                .collect();
            slicer.inverted = sel.inverted;
            slicer.all_selected = sel.is_all();
            slicer.selected = rows
                .iter()
                .enumerate()
                .filter(|(_, r)| {
                    let k = keys_of(r.get(..cols.len()).unwrap_or_default());
                    wanted.contains(&k)
                })
                .map(|(i, _)| i)
                .collect();
            slicer.values = sel
                .values
                .iter()
                .map(|t| {
                    t.iter()
                        .zip(&groups)
                        .zip(&cols)
                        .map(|((v, &f), c)| self.cell(coerce_value(v, &c.data_type), f, true))
                        .collect()
                })
                .collect();
        }
        Ok(DataView {
            kind: DataViewKind::Categorical,
            categorical: Some(Categorical {
                categories,
                values: Vec::new(),
                series_meta_idx: None,
                hidden: Vec::new(),
            }),
            slicer: Some(slicer),
            ..DataView::default()
        })
    }

    fn category_columns(&self, rows: &[Row], layout: &Layout) -> Vec<CategoryColumn> {
        layout
            .group_fields
            .iter()
            .enumerate()
            .map(|(p, &f)| CategoryColumn {
                meta_idx: f,
                values: rows
                    .iter()
                    .map(|r| self.cell(r.get(p).cloned().unwrap_or_default(), f, true))
                    .collect(),
            })
            .collect()
    }

    fn table(&mut self) -> Result<DataView, VisualQueryError> {
        let groups = self.indices(RoleKind::Category);
        let values = self.indices(RoleKind::Value);
        let hidden = self.hidden();
        let all_values: Vec<usize> = values.iter().chain(&hidden).copied().collect();
        let mut keys = self.sort_keys();
        if keys.is_empty()
            && let Some(first) = self.fields.first()
        {
            keys.push(SortKey {
                field: 0,
                descending: first.kind == RoleKind::Value,
            });
        }
        self.mark_sort(&keys);
        let layout = self.layout(&groups, &all_values, &keys, true);
        let mut rows = self.run(&layout)?;
        self.infer_types(&rows, &layout);
        self.truncate(&mut rows, self.limits.max_rows);
        let order: Vec<usize> = (0..self.visible).collect();
        let cells: Vec<Vec<CellValue>> = rows
            .iter()
            .map(|r| {
                order
                    .iter()
                    .map(|&f| self.cell_at(r, &layout, f, false))
                    .collect()
            })
            .collect();
        let totals_on = pbi_pbir::objects::bool_property(&self.visual.objects, "total", "totals")
            != Some(false);
        let total = if totals_on && !values.is_empty() && !groups.is_empty() {
            let total_layout = self.layout(&[], &all_values, &[], false);
            let total_row = self
                .run(&total_layout)?
                .into_iter()
                .next()
                .unwrap_or_default();
            Some((total_row, total_layout))
        } else {
            None
        };
        let totals = total.as_ref().map(|(row, l)| {
            order
                .iter()
                .map(|&f| match l.value_pos(f) {
                    Some(_) => self.cell_at(row, l, f, false),
                    None => CellValue::blank(),
                })
                .collect()
        });
        let hidden = self.hidden_values(
            &hidden,
            &rows,
            &layout,
            total.as_ref().map(|(r, l)| (r.as_slice(), l)),
        );
        Ok(DataView {
            kind: DataViewKind::Table,
            table: Some(TableData {
                columns: order,
                rows: cells,
                totals,
                hidden,
            }),
            ..DataView::default()
        })
    }

    fn chart(&mut self) -> Result<DataView, VisualQueryError> {
        let cats = self.indices(RoleKind::Category);
        let series = self.indices(RoleKind::Series);
        let values = self.indices(RoleKind::Value);
        let hidden = if series.is_empty() {
            self.hidden()
        } else {
            Vec::new()
        };
        let all_values: Vec<usize> = values.iter().chain(&hidden).copied().collect();
        let keys = self.sort_keys();
        let series_desc = keys
            .iter()
            .find(|k| series.contains(&k.field))
            .map(|k| k.descending);
        let mut cat_keys: Vec<SortKey> = keys
            .iter()
            .filter(|k| !series.contains(&k.field))
            .copied()
            .collect();
        if cat_keys.is_empty() {
            match values.first() {
                Some(&v) if sorts_by_value_by_default(&self.visual.visual_type) => {
                    cat_keys.push(SortKey {
                        field: v,
                        descending: true,
                    });
                }
                _ => cat_keys.extend(cats.iter().map(|&field| SortKey {
                    field,
                    descending: false,
                })),
            }
        }
        self.mark_sort(&cat_keys);
        let layout = self.layout(&cats, &all_values, &cat_keys, true);
        let mut rows = self.run(&layout)?;
        self.infer_types(&rows, &layout);
        if !cats.is_empty() {
            self.truncate(&mut rows, self.limits.max_categories);
        }
        let categories = self.category_columns(&rows, &layout);
        let hidden = self.hidden_values(&hidden, &rows, &layout, None);

        let mut value_columns = Vec::new();
        let mut series_meta_idx = None;
        if series.is_empty() {
            for &f in &values {
                value_columns.push(ValueColumn {
                    meta_idx: f,
                    series: None,
                    values: rows
                        .iter()
                        .map(|r| self.cell_at(r, &layout, f, false))
                        .collect(),
                    highlights: None,
                });
            }
        } else {
            if series.len() > 1 {
                self.warnings.push(QueryWarning::new(
                    WarningScope::Query,
                    "several series fields: series are labelled with the first one",
                ));
            }
            series_meta_idx = series.first().copied();
            let groups: Vec<usize> = cats.iter().chain(&series).copied().collect();
            let layout_s = self.layout(&groups, &values, &[], false);
            let rows_s = self.run(&layout_s)?;
            let ncat = cats.len();
            let nser = series.len();
            let cat_index: HashMap<Vec<ValueKey>, usize> = rows
                .iter()
                .enumerate()
                .map(|(i, r)| (keys_of(r.get(..ncat).unwrap_or_default()), i))
                .collect();
            let mut series_values: IndexMap<Vec<ValueKey>, Row> = IndexMap::new();
            for r in &rows_s {
                let cat_key = keys_of(r.get(..ncat).unwrap_or_default());
                if !cat_index.contains_key(&cat_key) {
                    continue;
                }
                let s = r.get(ncat..ncat + nser).unwrap_or_default();
                series_values
                    .entry(keys_of(s))
                    .or_insert_with(|| s.to_vec());
            }
            let mut ordered: Vec<(Vec<ValueKey>, Row)> = series_values.into_iter().collect();
            ordered.sort_by(|a, b| {
                let o =
                    a.1.iter()
                        .zip(&b.1)
                        .map(|(x, y)| x.total_cmp(y))
                        .find(|o| o.is_ne())
                        .unwrap_or(std::cmp::Ordering::Equal);
                if series_desc == Some(true) {
                    o.reverse()
                } else {
                    o
                }
            });
            if ordered.len() > self.limits.max_series {
                ordered.truncate(self.limits.max_series);
                self.truncated = true;
            }
            let series_pos: HashMap<Vec<ValueKey>, usize> = ordered
                .iter()
                .enumerate()
                .map(|(i, (k, _))| (k.clone(), i))
                .collect();
            let nvals = values.len();
            let mut grid: Vec<Vec<CellValue>> =
                vec![vec![CellValue::blank(); rows.len()]; ordered.len() * nvals];
            for r in &rows_s {
                let cat = cat_index.get(&keys_of(r.get(..ncat).unwrap_or_default()));
                let ser = series_pos.get(&keys_of(r.get(ncat..ncat + nser).unwrap_or_default()));
                let (Some(&ci), Some(&si)) = (cat, ser) else {
                    continue;
                };
                for (q, &f) in values.iter().enumerate() {
                    if let Some(slot) = grid.get_mut(si * nvals + q).and_then(|col| col.get_mut(ci))
                    {
                        *slot = self.cell_at(r, &layout_s, f, false);
                    }
                }
            }
            let first_series = series.first().copied().unwrap_or_default();
            let mut grid = grid.into_iter();
            for (_, s) in &ordered {
                let label = self.cell(s.first().cloned().unwrap_or_default(), first_series, true);
                for &f in &values {
                    value_columns.push(ValueColumn {
                        meta_idx: f,
                        series: Some(label.clone()),
                        values: grid.next().unwrap_or_default(),
                        highlights: None,
                    });
                }
            }
        }
        Ok(DataView {
            kind: DataViewKind::Categorical,
            categorical: Some(Categorical {
                categories,
                values: value_columns,
                series_meta_idx,
                hidden,
            }),
            ..DataView::default()
        })
    }

    fn matrix(&mut self) -> Result<DataView, VisualQueryError> {
        let rows_f = self.indices(RoleKind::Rows);
        let cols_f = self.indices(RoleKind::Columns);
        let vals_f = self.indices(RoleKind::Value);
        let hidden = self.hidden();
        let vals_all: Vec<usize> = vals_f.iter().chain(&hidden).copied().collect();
        let keys = self.sort_keys();
        self.mark_sort(&keys);
        if self.having.is_some() {
            self.warnings.push(
                QueryWarning::new(
                    WarningScope::Visual,
                    "measure filters are not applied to matrices yet",
                )
                .not_applied(),
            );
        }
        let (r, c) = (rows_f.len(), cols_f.len());
        let row_subtotals =
            pbi_pbir::objects::bool_property(&self.visual.objects, "subTotals", "rowSubtotals")
                != Some(false);
        let column_subtotals =
            pbi_pbir::objects::bool_property(&self.visual.objects, "subTotals", "columnSubtotals")
                != Some(false);

        // grid[i][j]: values grouped by the first i row levels and j column levels.
        let mut grid: Vec<Vec<Level>> = Vec::with_capacity(r + 1);
        let mut leaves: Vec<Row> = Vec::new();
        let mut column_tuples: Vec<Row> = Vec::new();
        for i in 0..=r {
            let mut level = Vec::with_capacity(c + 1);
            for j in 0..=c {
                let groups: Vec<usize> = rows_f
                    .iter()
                    .take(i)
                    .chain(cols_f.iter().take(j))
                    .copied()
                    .collect();
                let layout = self.layout(&groups, &vals_all, &[], false);
                let result = self.run(&layout)?;
                if i == r && j == 0 {
                    self.infer_types(&result, &layout);
                    leaves = result
                        .iter()
                        .map(|row| row.iter().take(r).cloned().collect())
                        .collect();
                }
                if i == 0 && j == c {
                    column_tuples = result
                        .iter()
                        .map(|row| row.iter().take(c).cloned().collect())
                        .collect();
                }
                let width = i + j;
                let formats = vals_all
                    .iter()
                    .map(|&f| layout.format_pos(f).map(|p| p - width))
                    .collect();
                let rows = result
                    .into_iter()
                    .map(|row| {
                        let key = keys_of(row.get(..width).unwrap_or_default());
                        (key, row.into_iter().skip(width).collect())
                    })
                    .collect();
                level.push(Level { rows, formats });
            }
            grid.push(level);
        }
        if leaves.len() > self.limits.max_rows {
            leaves.sort_by(|a, b| {
                a.iter()
                    .zip(b)
                    .map(|(x, y)| x.total_cmp(y))
                    .find(|o| o.is_ne())
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
            leaves.truncate(self.limits.max_rows);
            self.truncated = true;
        }

        let level_desc = |fields: &[usize], level: usize| {
            fields
                .get(level)
                .and_then(|f| keys.iter().find(|k| k.field == *f))
                .is_some_and(|k| k.descending)
        };
        let value_sort = keys.first().and_then(|k| {
            vals_f
                .iter()
                .position(|&v| v == k.field)
                .map(|q| (q, k.descending))
        });
        let row_orders = self.level_orders(&rows_f);
        let column_orders = self.level_orders(&cols_f);

        // Column headers.
        let mut columns: Vec<(Vec<Value>, bool)> = Vec::new();
        if c == 0 {
            columns.push((Vec::new(), false));
        } else {
            let tuples: Vec<&Row> = column_tuples.iter().collect();
            let mut prefix = Vec::new();
            self.column_headers(
                &tuples,
                &cols_f,
                0,
                &mut prefix,
                column_subtotals,
                &|depth| level_desc(&cols_f, depth),
                &column_orders,
                &mut columns,
            );
            if column_subtotals {
                columns.push((Vec::new(), true));
            }
            let cap = self.limits.max_categories;
            if columns.len() > cap {
                columns.truncate(cap);
                self.truncated = true;
            }
        }

        let ctx = MatrixCtx {
            grid: &grid,
            columns: &columns,
            rows_f: &rows_f,
            vals_f: &vals_f,
            vals_all: &vals_all,
            value_sort,
            row_desc: (0..r).map(|l| level_desc(&rows_f, l)).collect(),
            row_orders: &row_orders,
        };
        let leaf_refs: Vec<&Row> = leaves.iter().collect();
        let root = self.row_node(&ctx, &mut Vec::new(), &leaf_refs);
        let headers = columns
            .iter()
            .map(|(path, is_total)| MatrixColumn {
                path: path
                    .iter()
                    .zip(&cols_f)
                    .map(|(v, &f)| self.cell(v.clone(), f, true))
                    .collect(),
                is_total: *is_total,
            })
            .collect();
        Ok(DataView {
            kind: DataViewKind::Matrix,
            matrix: Some(Matrix {
                row_levels: rows_f.clone(),
                column_levels: cols_f.clone(),
                values: vals_f.clone(),
                columns: headers,
                rows: root,
                row_subtotals,
                column_subtotals,
                hidden,
            }),
            ..DataView::default()
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn column_headers(
        &self,
        tuples: &[&Row],
        cols_f: &[usize],
        depth: usize,
        prefix: &mut Vec<Value>,
        subtotals: bool,
        level_desc: &dyn Fn(usize) -> bool,
        orders: &[Option<HashMap<ValueKey, Value>>],
        out: &mut Vec<(Vec<Value>, bool)>,
    ) {
        let mut children: IndexMap<ValueKey, (Value, Vec<&Row>)> = IndexMap::new();
        for t in tuples {
            let v = t.get(depth).cloned().unwrap_or_default();
            children
                .entry(ValueKey::from(&v))
                .or_insert_with(|| (v, Vec::new()))
                .1
                .push(t);
        }
        let mut children: Vec<(Value, Vec<&Row>)> = children.into_values().collect();
        let desc = level_desc(depth);
        let order = orders.get(depth).and_then(Option::as_ref);
        children.sort_by(|a, b| {
            let o = level_cmp(order, &a.0, &b.0);
            if desc { o.reverse() } else { o }
        });
        let last = depth + 1 == cols_f.len();
        for (v, sub) in children {
            prefix.push(v);
            if last {
                out.push((prefix.clone(), false));
            } else {
                self.column_headers(
                    &sub,
                    cols_f,
                    depth + 1,
                    prefix,
                    subtotals,
                    level_desc,
                    orders,
                    out,
                );
                if subtotals {
                    out.push((prefix.clone(), true));
                }
            }
            prefix.pop();
        }
    }

    fn row_node(
        &self,
        ctx: &MatrixCtx<'_>,
        prefix: &mut Vec<Value>,
        leaves: &[&Row],
    ) -> MatrixNode {
        let d = prefix.len();
        let prefix_keys = keys_of(prefix);
        let nvals = ctx.vals_f.len();
        let mut cells = Vec::with_capacity(ctx.columns.len() * nvals);
        let mut hidden = Vec::with_capacity(ctx.columns.len() * (ctx.vals_all.len() - nvals));
        for (path, _) in ctx.columns {
            let mut key = prefix_keys.clone();
            key.extend(keys_of(path));
            let level = ctx.grid.get(d).and_then(|l| l.get(path.len()));
            let found = level.and_then(|l| l.rows.get(&key));
            for (q, &f) in ctx.vals_all.iter().enumerate() {
                let v = found
                    .and_then(|vals| vals.get(q).cloned())
                    .unwrap_or_default();
                let format = level
                    .and_then(|l| l.formats.get(q).copied().flatten())
                    .and_then(|p| found.and_then(|vals| vals.get(p)));
                let cell = self.cell_formatted(v, f, false, format);
                if q < nvals {
                    cells.push(cell);
                } else {
                    hidden.push(cell);
                }
            }
        }
        let mut children = Vec::new();
        if d < ctx.rows_f.len() {
            let mut groups: IndexMap<ValueKey, (Value, Vec<&Row>)> = IndexMap::new();
            for leaf in leaves {
                let v = leaf.get(d).cloned().unwrap_or_default();
                groups
                    .entry(ValueKey::from(&v))
                    .or_insert_with(|| (v, Vec::new()))
                    .1
                    .push(leaf);
            }
            let mut groups: Vec<(Value, Vec<&Row>)> = groups.into_values().collect();
            let subtotal = |v: &Value, q: usize| -> Value {
                let mut key = prefix_keys.clone();
                key.push(ValueKey::from(v));
                ctx.grid
                    .get(d + 1)
                    .and_then(|l| l.first())
                    .and_then(|m| m.rows.get(&key))
                    .and_then(|vals| vals.get(q).cloned())
                    .unwrap_or_default()
            };
            let desc = ctx.row_desc.get(d).copied().unwrap_or(false);
            let order = ctx.row_orders.get(d).and_then(Option::as_ref);
            groups.sort_by(|a, b| {
                if let Some((q, vdesc)) = ctx.value_sort {
                    let o = subtotal(&a.0, q).total_cmp(&subtotal(&b.0, q));
                    let o = if vdesc { o.reverse() } else { o };
                    if o.is_ne() {
                        return o;
                    }
                }
                let o = level_cmp(order, &a.0, &b.0);
                if desc { o.reverse() } else { o }
            });
            for (v, sub) in groups {
                prefix.push(v);
                children.push(self.row_node(ctx, prefix, &sub));
                prefix.pop();
            }
        }
        let value = d.checked_sub(1).map(|level| {
            let field = ctx.rows_f.get(level).copied().unwrap_or_default();
            self.cell(prefix.last().cloned().unwrap_or_default(), field, true)
        });
        MatrixNode {
            level: d.checked_sub(1),
            value,
            cells,
            children,
            hidden,
        }
    }
}

/// The results of one matrix level query: values (then format strings) by
/// group key, and where each value field's format string is in a row.
struct Level {
    rows: HashMap<Vec<ValueKey>, Row>,
    formats: Vec<Option<usize>>,
}

struct MatrixCtx<'a> {
    grid: &'a [Vec<Level>],
    columns: &'a [(Vec<Value>, bool)],
    rows_f: &'a [usize],
    vals_f: &'a [usize],
    /// `vals_f`, then the hidden conditional-formatting inputs.
    vals_all: &'a [usize],
    /// Sort rows by value field `q` (descending flag).
    value_sort: Option<(usize, bool)>,
    row_desc: Vec<bool>,
    /// Sort-by-column values per row level (see `level_orders`).
    row_orders: &'a [Option<HashMap<ValueKey, Value>>],
}

/// Orders two values of a matrix level: by their sort-by-column values
/// when the level has them (BLANK first), then by the values themselves.
fn level_cmp(order: Option<&HashMap<ValueKey, Value>>, a: &Value, b: &Value) -> std::cmp::Ordering {
    if let Some(order) = order {
        let by = |v: &Value| order.get(&ValueKey::from(v)).cloned().unwrap_or_default();
        let o = by(a).total_cmp(&by(b));
        if o.is_ne() {
            return o;
        }
    }
    a.total_cmp(b)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dax_for_grouped_and_scalar_queries() {
        let c = ColumnRef {
            table: "Web Sessions".into(),
            column: "Device".into(),
            data_type: DataType::String,
        };
        assert_eq!(
            summarize_dax(&[&c], &["'Web Sessions'[Sessions]"]),
            "EVALUATE SUMMARIZECOLUMNS('Web Sessions'[Device], \"v0\", 'Web Sessions'[Sessions])"
        );
        assert_eq!(
            summarize_dax(&[], &["1", "2"]),
            "EVALUATE ROW(\"v0\", 1, \"v1\", 2)"
        );
    }

    #[test]
    fn rows_sort_by_keys_with_blank_first() {
        let mut rows = vec![
            vec![Value::from("b"), Value::Integer(2)],
            vec![Value::Blank, Value::Integer(9)],
            vec![Value::from("a"), Value::Integer(2)],
        ];
        sort_rows(&mut rows, &[(1, true), (0, false)]);
        assert_eq!(rows[0][1], Value::Integer(9));
        assert_eq!(rows[1][0], Value::from("a"));
        sort_rows(&mut rows, &[(0, false)]);
        assert!(rows[0][0].is_blank());
    }

    #[test]
    fn blank_cells_format_per_context() {
        assert_eq!(
            format_cell("en-US", Value::Blank, None, true).formatted,
            "(Blank)"
        );
        assert_eq!(
            format_cell("en-US", Value::Blank, None, false).formatted,
            ""
        );
        assert_eq!(
            format_cell(
                "en-US",
                Value::Double(0.453),
                Some("0.0%;-0.0%;0.0%"),
                false
            )
            .formatted,
            "45.3%"
        );
    }

    #[test]
    fn matrix_levels_order_by_their_sort_by_column() {
        let order: HashMap<ValueKey, Value> = [("Current", 0), ("YTD", 1), ("PY", 2)]
            .into_iter()
            .map(|(k, v)| (ValueKey::from(&Value::from(k)), Value::Integer(v)))
            .collect();
        let mut items: Vec<Value> = ["PY", "Current", "YTD"].map(Value::from).to_vec();
        items.sort_by(|a, b| level_cmp(Some(&order), a, b));
        assert_eq!(items, ["Current", "YTD", "PY"].map(Value::from).to_vec());
        // Without an order (or for values it does not know: BLANK first),
        // the values sort by themselves.
        items.push(Value::from("Other"));
        items.sort_by(|a, b| level_cmp(Some(&order), a, b));
        assert_eq!(items[0], Value::from("Other"));
        items.sort_by(|a, b| level_cmp(None, a, b));
        assert_eq!(
            items,
            ["Current", "Other", "PY", "YTD"].map(Value::from).to_vec()
        );
    }
}
