//! [`FilterBuilder`]: report-AST filter conditions → engine filters.
//!
//! # Translation
//!
//! A condition is split into its top-level `And` conjuncts; each conjunct
//! becomes one or more engine filters (all intersected):
//!
//! - **one column**: `In` → [`ColumnFilter::in_values`] (strict `IN`
//!   semantics, text case-insensitive); anything else (`Not In`,
//!   comparisons, `Contains`, `StartsWith`, `Between`, is (not) blank,
//!   `And`/`Or`/`Not` trees) → a value predicate ([`ColumnFilter::predicate`]),
//!   evaluated once per distinct value. Filters on one column propagate
//!   over relationships like any column filter.
//! - **several columns of one table** (tuple `In`, `Or` across columns):
//!   the table's rows are scanned and kept in a base-table
//!   [`TableFilter`]; the table's blank row passes if the condition holds
//!   for all-BLANK values.
//! - **columns of several tables**: the rows of a table whose expanded
//!   table holds every column (e.g. `Orders` for `Stores` + `Orders`
//!   columns) are scanned following the many → one paths and
//!   kept in an expanded [`TableFilter`] (DAX tuple-filter semantics).
//!   Without such a table, a tuple `In` is applied column by column (exact
//!   for one tuple, a superset otherwise — reported as approximate); other
//!   conditions are not applied.
//!
//! Literals are coerced to the column's data type: `'Camping'` stays
//! text, `221L` is an integer, `1.5D` a double, `1.5M` a fixed decimal,
//! `datetime'2026-01-01T00:00:00'` (or ISO text on a date column) a
//! date/time, `null` BLANK. Comparisons use DAX semantics (BLANK compares
//! as 0 / empty text, text is case-insensitive); `= null` means "is blank"
//! (strictly BLANK) and `Not(= null)` "is not blank".
//!
//! Conditions over measures or aggregations (visual-level "value"
//! filters) are not column filters: [`FilterBuilder::plan`] returns them
//! as [`FilterPlan::value_conditions`] for the query engine to apply to the
//! visual's rows. Top N filters are returned as [`FilterPlan::top_n`].

use std::collections::HashSet;
use std::sync::Arc;

use pbi_engine::value::fold_text;
use pbi_engine::{
    ColumnFilter, EngineSnapshot, Filter as EngineFilter, FilterContext, RowSet, TableFilter,
    Value, ValueKey,
};
use pbi_model::names::names_eq;
use pbi_model::{
    ComparisonKind, Condition, DataType, FieldExpr, Filter, FilterKind, LiteralValue, Model,
};

use super::semantic::{TopNSpec, top_n};

/// Cap on the rows scanned for one multi-column filter.
const MAX_SCAN_ROWS: usize = 50_000_000;

/// A model column resolved to its canonical spelling and type.
#[derive(Debug, Clone, PartialEq)]
pub struct ColumnRef {
    pub table: String,
    pub column: String,
    pub data_type: DataType,
}

impl ColumnRef {
    /// `Table[Column]` (DAX).
    pub fn dax(&self) -> String {
        pbi_model::dax_column_ref(&self.table, &self.column)
    }

    fn same(&self, other: &ColumnRef) -> bool {
        names_eq(&self.table, &other.table) && names_eq(&self.column, &other.column)
    }
}

/// What a report-AST filter turns into.
#[derive(Debug, Clone, Default)]
pub struct FilterPlan {
    /// Engine filters (intersect them into the context).
    pub filters: Vec<EngineFilter>,
    /// Conditions over measures / aggregations, applied to the visual's
    /// rows (visual-level filters only).
    pub value_conditions: Vec<Condition>,
    /// A Top N filter to evaluate in the visual's context.
    pub top_n: Option<TopNSpec>,
    /// Parts that were not applied, with the reason.
    pub skipped: Vec<String>,
    /// Parts applied approximately, with the reason.
    pub approximate: Vec<String>,
}

impl FilterPlan {
    /// `true` if nothing filters.
    pub fn is_empty(&self) -> bool {
        self.filters.is_empty() && self.value_conditions.is_empty() && self.top_n.is_none()
    }
}

/// Translates conditions on model columns into engine filters (see the
/// module docs).
#[derive(Debug, Clone, Copy)]
pub struct FilterBuilder<'a> {
    model: &'a Model,
    snapshot: &'a EngineSnapshot,
}

impl<'a> FilterBuilder<'a> {
    pub fn new(model: &'a Model, snapshot: &'a EngineSnapshot) -> Self {
        Self { model, snapshot }
    }

    /// Resolves a column or (non-variation) hierarchy-level field.
    ///
    /// # Errors
    /// A user-facing reason for measures, unknown fields and columns that
    /// are not loaded.
    pub fn resolve_column(&self, field: &FieldExpr) -> Result<ColumnRef, String> {
        let (table, column) = match field {
            FieldExpr::Column { entity, property } => (entity.as_str(), property.as_str()),
            FieldExpr::HierarchyLevel {
                entity,
                hierarchy,
                level,
                variation,
            } => {
                if variation.is_some() {
                    return Err(format!(
                        "auto date/time hierarchy level {entity}.{hierarchy}.{level} is not supported"
                    ));
                }
                let lvl = self
                    .model
                    .table(entity)
                    .and_then(|t| t.hierarchy(hierarchy))
                    .and_then(|h| h.level(level))
                    .ok_or_else(|| {
                        format!("hierarchy level {entity}.{hierarchy}.{level} is not in the model")
                    })?;
                (entity.as_str(), lvl.column.as_str())
            }
            FieldExpr::Measure { entity, property } => {
                return Err(format!("{entity}.{property} is a measure, not a column"));
            }
            other => {
                return Err(format!(
                    "unsupported field {}",
                    other.query_ref().unwrap_or_else(|| "expression".into())
                ));
            }
        };
        let t = self
            .model
            .table(table)
            .ok_or_else(|| format!("table '{table}' is not in the model"))?;
        let c = t
            .column(column)
            .ok_or_else(|| format!("column {table}.{column} is not in the model"))?;
        let loaded = self
            .snapshot
            .table(&t.name)
            .is_ok_and(|d| d.has_column(&c.name));
        if !loaded {
            return Err(format!("column {}.{} has no data", t.name, c.name));
        }
        Ok(ColumnRef {
            table: t.name.clone(),
            column: c.name.clone(),
            data_type: c.data_type.clone(),
        })
    }

    /// Plans a report/page/visual filter card.
    pub fn plan(&self, filter: &Filter) -> FilterPlan {
        let mut plan = FilterPlan::default();
        match filter.kind {
            FilterKind::RelativeDate | FilterKind::RelativeTime => {
                if filter.condition.is_some() {
                    plan.skipped.push(format!(
                        "{} filters are not supported yet",
                        filter.kind.as_str()
                    ));
                }
                return plan;
            }
            FilterKind::TopN => {
                let def = filter.raw.get("filter");
                match def.and_then(top_n) {
                    Some(spec) => plan.top_n = Some(spec),
                    None if def.is_some() => plan
                        .skipped
                        .push("the Top N filter definition could not be read".into()),
                    None => {}
                }
                return plan;
            }
            _ => {}
        }
        let Some(condition) = &filter.condition else {
            return plan;
        };
        self.plan_condition(condition, &mut plan);
        plan
    }

    /// Plans a bare condition (adds to `plan`).
    pub fn plan_condition(&self, condition: &Condition, plan: &mut FilterPlan) {
        let mut conjuncts = Vec::new();
        flatten_and(condition, &mut conjuncts);
        for c in conjuncts {
            match classify(c) {
                Operands::Columns => match self.condition_filters(c) {
                    Ok((filters, approx)) => {
                        plan.filters.extend(filters);
                        plan.approximate.extend(approx);
                    }
                    Err(reason) => plan.skipped.push(reason),
                },
                Operands::Values => plan.value_conditions.push(c.clone()),
                Operands::Mixed => plan
                    .skipped
                    .push("conditions mixing columns and measures are not supported".into()),
                Operands::Unknown(reason) => plan.skipped.push(reason),
            }
        }
    }

    /// Engine filters for a condition on columns (all intersected).
    ///
    /// # Errors
    /// A user-facing reason when the condition cannot be translated.
    pub fn condition(&self, condition: &Condition) -> Result<Vec<EngineFilter>, String> {
        let mut conjuncts = Vec::new();
        flatten_and(condition, &mut conjuncts);
        let mut out = Vec::new();
        for c in conjuncts {
            match classify(c) {
                Operands::Columns => out.extend(self.condition_filters(c)?.0),
                Operands::Values | Operands::Mixed => {
                    return Err("conditions on measures are not column filters".into());
                }
                Operands::Unknown(reason) => return Err(reason),
            }
        }
        Ok(out)
    }

    /// Adds the filters of `condition` to `ctx`.
    ///
    /// # Errors
    /// As [`FilterBuilder::condition`]; `ctx` is unchanged on error.
    pub fn add_condition(
        &self,
        condition: &Condition,
        ctx: &mut FilterContext,
    ) -> Result<(), String> {
        for f in self.condition(condition)? {
            ctx.add(f);
        }
        Ok(())
    }

    /// `(columns) IN tuples` (or `NOT IN` with `exclude`) for values from
    /// the UI (slicer selections, selected data points); values are coerced
    /// to the column types.
    ///
    /// # Errors
    /// Unknown columns, tuples of the wrong width, unrelated tables.
    pub fn tuple_filter(
        &self,
        columns: &[FieldExpr],
        tuples: &[Vec<Value>],
        exclude: bool,
    ) -> Result<(Vec<EngineFilter>, Vec<String>), String> {
        let cols = columns
            .iter()
            .map(|c| self.resolve_column(c))
            .collect::<Result<Vec<_>, _>>()?;
        if cols.is_empty() {
            return Err("the selection has no columns".into());
        }
        let mut keys = HashSet::with_capacity(tuples.len());
        for t in tuples {
            if t.len() != cols.len() {
                return Err(format!(
                    "a selected value has {} parts, expected {}",
                    t.len(),
                    cols.len()
                ));
            }
            keys.insert(
                t.iter()
                    .zip(&cols)
                    .map(|(v, c)| ValueKey::from(&coerce_value(v, &c.data_type)))
                    .collect::<Vec<_>>(),
            );
        }
        let pred = Pred::In {
            cols: (0..cols.len()).collect(),
            keys: Arc::new(keys),
        };
        let pred = if exclude {
            Pred::Not(Box::new(pred))
        } else {
            pred
        };
        self.compile(cols, pred)
    }

    fn condition_filters(&self, c: &Condition) -> Result<(Vec<EngineFilter>, Vec<String>), String> {
        let mut cols: Vec<ColumnRef> = Vec::new();
        let pred = {
            let mut resolve = |f: &FieldExpr| -> Result<(usize, Option<DataType>), String> {
                let col = self.resolve_column(f)?;
                let dt = col.data_type.clone();
                let idx = match cols.iter().position(|c| c.same(&col)) {
                    Some(i) => i,
                    None => {
                        cols.push(col);
                        cols.len() - 1
                    }
                };
                Ok((idx, Some(dt)))
            };
            compile_pred(c, &mut resolve)?
        };
        self.compile(cols, pred)
    }

    fn compile(
        &self,
        cols: Vec<ColumnRef>,
        pred: Pred,
    ) -> Result<(Vec<EngineFilter>, Vec<String>), String> {
        match cols.as_slice() {
            [] => Err("the condition references no column".into()),
            [col] => Ok((vec![single_column_filter(col, pred)], Vec::new())),
            _ if cols.iter().all(|c| names_eq(&c.table, &cols[0].table)) => {
                let table = cols[0].table.clone();
                let rows = self.scan(&table, &cols, &pred)?;
                let blank = pred.eval(&vec![Value::Blank; cols.len()]);
                Ok((
                    vec![EngineFilter::Table(
                        TableFilter::new(table, rows)
                            .base_only()
                            .with_blank_row(blank),
                    )],
                    Vec::new(),
                ))
            }
            _ => self.compile_cross_table(cols, pred),
        }
    }

    fn compile_cross_table(
        &self,
        cols: Vec<ColumnRef>,
        pred: Pred,
    ) -> Result<(Vec<EngineFilter>, Vec<String>), String> {
        let graph = self.snapshot.graph();
        let covers = |base: &str| {
            cols.iter()
                .all(|c| graph.expanded_path(base, &c.table).is_some())
        };
        let base = cols
            .iter()
            .map(|c| c.table.clone())
            .find(|t| covers(t))
            .or_else(|| self.snapshot.table_names().into_iter().find(|t| covers(t)));
        if let Some(base) = base {
            let rows = self.scan(&base, &cols, &pred)?;
            return Ok((
                vec![EngineFilter::Table(TableFilter::new(base, rows))],
                Vec::new(),
            ));
        }
        // No common table: a tuple IN is applied per column.
        let Pred::In { keys, .. } = &pred else {
            return Err("the condition spans tables without a relationship path".into());
        };
        let mut filters = Vec::with_capacity(cols.len());
        for (i, col) in cols.iter().enumerate() {
            let values: HashSet<ValueKey> = keys.iter().filter_map(|t| t.get(i).cloned()).collect();
            filters.push(EngineFilter::Column(ColumnFilter::in_keys(
                col.table.as_str(),
                col.column.as_str(),
                values,
            )));
        }
        let approx = if keys.len() > 1 {
            vec![
                "the multi-column selection spans unrelated tables; applied column by column"
                    .into(),
            ]
        } else {
            Vec::new()
        };
        Ok((filters, approx))
    }

    /// Rows of `base` (all rows) whose column values pass `pred`; columns
    /// of other tables are read through the many → one path from `base`.
    fn scan(&self, base: &str, cols: &[ColumnRef], pred: &Pred) -> Result<RowSet, String> {
        let err = |e: pbi_engine::EngineError| e.to_string();
        let data = self.snapshot.table(base).map_err(err)?;
        let n = data.row_count();
        if n.saturating_mul(cols.len()) > MAX_SCAN_ROWS {
            return Err(format!(
                "table '{base}' is too large for a multi-column filter"
            ));
        }
        let graph = self.snapshot.graph();
        let mut readers = Vec::with_capacity(cols.len());
        for c in cols {
            let path = graph
                .expanded_path(base, &c.table)
                .ok_or_else(|| format!("no relationship path from '{base}' to '{}'", c.table))?
                .to_vec();
            let enc = self
                .snapshot
                .table(&c.table)
                .and_then(|t| t.encoded(&c.column))
                .map_err(err)?;
            readers.push((path, enc));
        }
        let mut row_values = vec![Value::Blank; cols.len()];
        let mut keep = Vec::new();
        for row in 0..n {
            for (slot, (path, enc)) in row_values.iter_mut().zip(&readers) {
                *slot = if path.is_empty() {
                    enc.value_at(row).cloned().unwrap_or(Value::Blank)
                } else {
                    graph
                        .follow(path, row)
                        .and_then(|r| enc.value_at(r).cloned())
                        .unwrap_or(Value::Blank)
                };
            }
            if pred.eval(&row_values) {
                keep.push(row);
            }
        }
        Ok(RowSet::from_rows(n, keep))
    }
}

fn single_column_filter(col: &ColumnRef, pred: Pred) -> EngineFilter {
    if let Pred::In { keys, .. } = &pred {
        let keys: HashSet<ValueKey> = keys.iter().filter_map(|t| t.first().cloned()).collect();
        return EngineFilter::Column(ColumnFilter::in_keys(
            col.table.as_str(),
            col.column.as_str(),
            keys,
        ));
    }
    let label = pred.label();
    EngineFilter::Column(
        ColumnFilter::predicate(col.table.as_str(), col.column.as_str(), move |v| {
            pred.eval(std::slice::from_ref(v))
        })
        .with_label(label),
    )
}

fn flatten_and<'c>(c: &'c Condition, out: &mut Vec<&'c Condition>) {
    match c {
        Condition::And(a, b) => {
            flatten_and(a, out);
            flatten_and(b, out);
        }
        other => out.push(other),
    }
}

/// What a condition's operands are.
#[derive(Debug, Clone, PartialEq)]
enum Operands {
    Columns,
    Values,
    Mixed,
    Unknown(String),
}

fn classify(c: &Condition) -> Operands {
    let mut columns = false;
    let mut values = false;
    let mut unknown: Option<String> = None;
    if let Condition::Unknown(_) = c {
        return Operands::Unknown("unsupported filter condition".into());
    }
    let mut nested_unknown = false;
    check_unknown(c, &mut nested_unknown);
    if nested_unknown {
        return Operands::Unknown("unsupported filter condition".into());
    }
    c.visit_fields(&mut |f| match f {
        FieldExpr::Column { .. } | FieldExpr::HierarchyLevel { .. } => columns = true,
        FieldExpr::Measure { .. } | FieldExpr::Aggregation { .. } => values = true,
        FieldExpr::Literal(_) => {}
        FieldExpr::Unknown(_) => {
            unknown.get_or_insert_with(|| "unsupported expression in a filter condition".into());
        }
    });
    if let Some(reason) = unknown {
        return Operands::Unknown(reason);
    }
    match (columns, values) {
        (true, false) => Operands::Columns,
        (false, true) => Operands::Values,
        (true, true) => Operands::Mixed,
        (false, false) => Operands::Unknown("the condition references no field".into()),
    }
}

fn check_unknown(c: &Condition, found: &mut bool) {
    match c {
        Condition::Unknown(_) => *found = true,
        Condition::And(a, b) | Condition::Or(a, b) => {
            check_unknown(a, found);
            check_unknown(b, found);
        }
        Condition::Not(inner) => check_unknown(inner, found),
        _ => {}
    }
}

/// A compiled condition over a list of operands (columns or value
/// expressions), evaluated on one row of operand values.
#[derive(Debug, Clone)]
pub(crate) enum Pred {
    In {
        cols: Vec<usize>,
        keys: Arc<HashSet<Vec<ValueKey>>>,
    },
    Cmp {
        col: usize,
        kind: ComparisonKind,
        value: Value,
    },
    Between {
        col: usize,
        lower: Value,
        upper: Value,
    },
    Contains {
        col: usize,
        needle: String,
    },
    StartsWith {
        col: usize,
        needle: String,
    },
    IsBlank {
        col: usize,
    },
    And(Box<Pred>, Box<Pred>),
    Or(Box<Pred>, Box<Pred>),
    Not(Box<Pred>),
}

impl Pred {
    pub(crate) fn eval(&self, row: &[Value]) -> bool {
        let get = |i: usize| row.get(i).unwrap_or(&Value::Blank);
        match self {
            Pred::In { cols, keys } => {
                let key: Vec<ValueKey> = cols.iter().map(|&i| ValueKey::from(get(i))).collect();
                keys.contains(&key)
            }
            Pred::Cmp { col, kind, value } => {
                let Ok(ord) = get(*col).dax_cmp(value) else {
                    return false;
                };
                match kind {
                    ComparisonKind::Equal => ord.is_eq(),
                    ComparisonKind::GreaterThan => ord.is_gt(),
                    ComparisonKind::GreaterThanOrEqual => ord.is_ge(),
                    ComparisonKind::LessThan => ord.is_lt(),
                    ComparisonKind::LessThanOrEqual => ord.is_le(),
                }
            }
            Pred::Between { col, lower, upper } => {
                let v = get(*col);
                v.dax_cmp(lower).is_ok_and(|o| o.is_ge())
                    && v.dax_cmp(upper).is_ok_and(|o| o.is_le())
            }
            Pred::Contains { col, needle } => fold_text(&get(*col).to_text()).contains(needle),
            Pred::StartsWith { col, needle } => fold_text(&get(*col).to_text()).starts_with(needle),
            Pred::IsBlank { col } => get(*col).is_blank(),
            Pred::And(a, b) => a.eval(row) && b.eval(row),
            Pred::Or(a, b) => a.eval(row) || b.eval(row),
            Pred::Not(p) => !p.eval(row),
        }
    }

    fn label(&self) -> String {
        match self {
            Pred::In { keys, .. } => format!("IN ({} values)", keys.len()),
            Pred::Cmp { kind, value, .. } => format!("{kind:?} {value:?}"),
            Pred::Between { lower, upper, .. } => format!("BETWEEN {lower:?} AND {upper:?}"),
            Pred::Contains { needle, .. } => format!("CONTAINS {needle:?}"),
            Pred::StartsWith { needle, .. } => format!("STARTSWITH {needle:?}"),
            Pred::IsBlank { .. } => "ISBLANK".into(),
            Pred::And(a, b) => format!("({}) AND ({})", a.label(), b.label()),
            Pred::Or(a, b) => format!("({}) OR ({})", a.label(), b.label()),
            Pred::Not(p) => format!("NOT ({})", p.label()),
        }
    }
}

/// Resolves an operand to `(index, column data type)`.
pub(crate) type Resolve<'r> =
    dyn FnMut(&FieldExpr) -> Result<(usize, Option<DataType>), String> + 'r;

/// Compiles a condition; field operands go through `resolve`.
pub(crate) fn compile_pred(c: &Condition, resolve: &mut Resolve<'_>) -> Result<Pred, String> {
    Ok(match c {
        Condition::In { exprs, values } => {
            let mut cols = Vec::with_capacity(exprs.len());
            let mut types = Vec::with_capacity(exprs.len());
            for e in exprs {
                let (i, dt) = resolve(e)?;
                cols.push(i);
                types.push(dt);
            }
            let mut keys = HashSet::with_capacity(values.len());
            for row in values {
                if row.len() != cols.len() {
                    return Err("a filter value tuple does not match its columns".into());
                }
                let mut key = Vec::with_capacity(row.len());
                for (lit, dt) in row.iter().zip(&types) {
                    let lv = lit
                        .value()
                        .map_err(|e| format!("cannot read filter value: {e}"))?;
                    key.push(ValueKey::from(&coerce_literal(&lv, dt.as_ref())));
                }
                keys.insert(key);
            }
            Pred::In {
                cols,
                keys: Arc::new(keys),
            }
        }
        Condition::Comparison { kind, left, right } => {
            let (field, lit, kind) = match (left, right) {
                (f, FieldExpr::Literal(l)) => (f, l, *kind),
                (FieldExpr::Literal(l), f) => (f, l, flip(*kind)),
                _ => return Err("comparisons between two fields are not supported".into()),
            };
            let (col, dt) = resolve(field)?;
            let lv = lit
                .value()
                .map_err(|e| format!("cannot read filter value: {e}"))?;
            if lv == LiteralValue::Null && kind == ComparisonKind::Equal {
                Pred::IsBlank { col }
            } else {
                Pred::Cmp {
                    col,
                    kind,
                    value: coerce_literal(&lv, dt.as_ref()),
                }
            }
        }
        Condition::Between { expr, lower, upper } => {
            let (col, dt) = resolve(expr)?;
            let bound = |b: &FieldExpr| -> Result<Value, String> {
                match b {
                    FieldExpr::Literal(l) => l
                        .value()
                        .map(|v| coerce_literal(&v, dt.as_ref()))
                        .map_err(|e| format!("cannot read filter value: {e}")),
                    _ => Err("Between bounds must be literals".into()),
                }
            };
            Pred::Between {
                col,
                lower: bound(lower)?,
                upper: bound(upper)?,
            }
        }
        Condition::Contains { left, right } | Condition::StartsWith { left, right } => {
            let (col, _) = resolve(left)?;
            let FieldExpr::Literal(l) = right else {
                return Err("text filters need a literal".into());
            };
            let lv = l
                .value()
                .map_err(|e| format!("cannot read filter value: {e}"))?;
            let needle = fold_text(&literal_text(&lv));
            if matches!(c, Condition::Contains { .. }) {
                Pred::Contains { col, needle }
            } else {
                Pred::StartsWith { col, needle }
            }
        }
        Condition::And(a, b) => Pred::And(
            Box::new(compile_pred(a, resolve)?),
            Box::new(compile_pred(b, resolve)?),
        ),
        Condition::Or(a, b) => Pred::Or(
            Box::new(compile_pred(a, resolve)?),
            Box::new(compile_pred(b, resolve)?),
        ),
        Condition::Not(inner) => Pred::Not(Box::new(compile_pred(inner, resolve)?)),
        Condition::Unknown(_) => return Err("unsupported filter condition".into()),
    })
}

fn flip(kind: ComparisonKind) -> ComparisonKind {
    match kind {
        ComparisonKind::Equal => ComparisonKind::Equal,
        ComparisonKind::GreaterThan => ComparisonKind::LessThan,
        ComparisonKind::GreaterThanOrEqual => ComparisonKind::LessThanOrEqual,
        ComparisonKind::LessThan => ComparisonKind::GreaterThan,
        ComparisonKind::LessThanOrEqual => ComparisonKind::GreaterThanOrEqual,
    }
}

fn literal_text(v: &LiteralValue) -> String {
    match v {
        LiteralValue::Text(s) | LiteralValue::DateTime(s) => s.clone(),
        LiteralValue::Integer(i) => i.to_string(),
        LiteralValue::Double(f) | LiteralValue::Decimal(f) => f.to_string(),
        LiteralValue::Boolean(b) => b.to_string(),
        LiteralValue::Null => String::new(),
    }
}

/// A semantic-query literal as an engine value of the column's type
/// (`None`: no column, e.g. a measure operand — the literal's own type).
pub fn coerce_literal(v: &LiteralValue, data_type: Option<&DataType>) -> Value {
    let natural = match v {
        LiteralValue::Null => Value::Blank,
        LiteralValue::Text(s) => Value::string(s.as_str()),
        LiteralValue::Integer(i) => Value::Integer(*i),
        LiteralValue::Double(f) => Value::Double(*f),
        LiteralValue::Decimal(f) => Value::decimal_from_f64(*f).unwrap_or(Value::Double(*f)),
        LiteralValue::Boolean(b) => Value::Boolean(*b),
        LiteralValue::DateTime(s) => parse_datetime(s).unwrap_or_else(|| Value::string(s.as_str())),
    };
    match data_type {
        Some(dt) => coerce_value(&natural, dt),
        None => natural,
    }
}

/// Converts a value to a column's data type where it represents the same
/// thing (`"42"` → 42 for an integer column, ISO text → date, 3 → `"3"`
/// for a text column). Values that do not convert are kept (and match
/// nothing).
pub fn coerce_value(v: &Value, data_type: &DataType) -> Value {
    match (v, data_type) {
        (Value::Blank, _) => Value::Blank,
        (Value::String(s), DataType::Int64) => {
            let t = s.trim();
            t.parse::<i64>()
                .map(Value::Integer)
                .ok()
                .or_else(|| t.parse::<f64>().ok().map(Value::Double))
                .unwrap_or_else(|| v.clone())
        }
        (Value::String(s), DataType::Double) => s
            .trim()
            .parse::<f64>()
            .map_or_else(|_| v.clone(), Value::Double),
        (Value::String(s), DataType::Decimal) => s
            .trim()
            .parse::<f64>()
            .ok()
            .and_then(Value::decimal_from_f64)
            .unwrap_or_else(|| v.clone()),
        (Value::String(s), DataType::DateTime) => parse_datetime(s).unwrap_or_else(|| v.clone()),
        (Value::String(s), DataType::Boolean) => match s.trim().to_ascii_lowercase().as_str() {
            "true" => Value::Boolean(true),
            "false" => Value::Boolean(false),
            _ => v.clone(),
        },
        (Value::Integer(_) | Value::Double(_) | Value::Decimal(_), DataType::String) => {
            Value::string(v.to_text())
        }
        (Value::Integer(_) | Value::Double(_) | Value::Decimal(_), DataType::DateTime) => v
            .as_f64()
            .and_then(Value::from_oadate)
            .unwrap_or_else(|| v.clone()),
        (Value::Integer(_) | Value::Double(_), DataType::Decimal) => v
            .as_f64()
            .and_then(Value::decimal_from_f64)
            .unwrap_or_else(|| v.clone()),
        (Value::Boolean(b), DataType::String) => Value::string(if *b { "True" } else { "False" }),
        (Value::DateTime(_), DataType::String) => Value::string(v.to_text()),
        _ => v.clone(),
    }
}

/// `YYYY-MM-DD[(T| )hh:mm[:ss[.ffffff]]]`, optionally followed by `Z` or a
/// UTC offset (ignored: values are naive date/times).
pub fn parse_datetime(text: &str) -> Option<Value> {
    let t = text.trim();
    let (date, time) = match t.split_once(['T', ' ']) {
        Some((d, rest)) => (d, Some(rest)),
        None => (t, None),
    };
    let mut parts = date.split('-');
    let year: i64 = parts.next()?.trim().parse().ok()?;
    let month: u32 = parts.next()?.parse().ok()?;
    let day: u32 = parts.next()?.parse().ok()?;
    if parts.next().is_some() {
        return None;
    }
    let (mut hour, mut minute, mut second, mut micros) = (0, 0, 0, 0i64);
    if let Some(time) = time {
        let time = time.trim().trim_end_matches(['Z', 'z']);
        let time = time.split(['+', '-']).next()?;
        let mut hms = time.split(':');
        hour = hms.next()?.parse().ok()?;
        minute = hms.next()?.parse().ok()?;
        if let Some(sec) = hms.next() {
            let (whole, frac) = sec.split_once('.').unwrap_or((sec, ""));
            second = whole.parse().ok()?;
            if !frac.is_empty() {
                if !frac.bytes().all(|b| b.is_ascii_digit()) {
                    return None;
                }
                let digits: String = frac.chars().chain(std::iter::repeat('0')).take(6).collect();
                micros = digits.parse().ok()?;
            }
        }
        if hms.next().is_some() {
            return None;
        }
    }
    match Value::datetime(year, month, day, hour, minute, second)? {
        Value::DateTime(us) => Some(Value::DateTime(us.checked_add(micros)?)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    #[test]
    fn literals_coerce_to_column_types() {
        let lit = |raw: &str| pbi_model::Literal::new(raw).value().unwrap();
        assert_eq!(
            coerce_literal(&lit("'Camping'"), Some(&DataType::String)),
            Value::string("Camping")
        );
        assert_eq!(
            coerce_literal(&lit("221L"), Some(&DataType::Int64)),
            Value::Integer(221)
        );
        assert_eq!(
            coerce_literal(&lit("221L"), Some(&DataType::String)),
            Value::string("221")
        );
        assert_eq!(
            coerce_literal(&lit("1.5D"), Some(&DataType::Double)),
            Value::Double(1.5)
        );
        assert_eq!(
            coerce_literal(&lit("1.5M"), Some(&DataType::Decimal)),
            Value::Decimal(15_000)
        );
        assert_eq!(
            coerce_literal(&lit("'42'"), Some(&DataType::Int64)),
            Value::Integer(42)
        );
        assert_eq!(
            coerce_literal(
                &lit("datetime'2026-10-21T14:05:00'"),
                Some(&DataType::DateTime)
            ),
            Value::datetime(2026, 10, 21, 14, 5, 0).unwrap()
        );
        assert_eq!(
            coerce_literal(&lit("'2026-10-21'"), Some(&DataType::DateTime)),
            Value::date(2026, 10, 21).unwrap()
        );
        assert_eq!(
            coerce_literal(&lit("null"), Some(&DataType::Int64)),
            Value::Blank
        );
        assert_eq!(coerce_literal(&lit("true"), None), Value::Boolean(true));
    }

    #[test]
    fn datetimes_parse_leniently() {
        let base = Value::datetime(2026, 1, 2, 3, 4, 5).unwrap();
        let Value::DateTime(us) = base else {
            panic!("not a datetime");
        };
        assert_eq!(parse_datetime("2026-01-02T03:04:05"), Some(base.clone()));
        assert_eq!(parse_datetime("2026-01-02 03:04:05Z"), Some(base.clone()));
        assert_eq!(parse_datetime("2026-01-02T03:04:05+02:00"), Some(base));
        assert_eq!(
            parse_datetime("2026-01-02T03:04:05.25"),
            Some(Value::DateTime(us + 250_000))
        );
        assert_eq!(parse_datetime("2026-01-02"), Value::date(2026, 1, 2));
        for bad in [
            "",
            "2026",
            "2026-13-01",
            "2026-01-02T25:00",
            "x-y-z",
            "2026-01-02T03:04:05.x",
        ] {
            assert_eq!(parse_datetime(bad), None, "{bad}");
        }
    }
}
