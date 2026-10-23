//! Calculation groups: applying calculation items to measure references.
//!
//! When a measure reference is evaluated ([`Interp::eval_measure`]), the
//! calculation groups of the model are checked in precedence order
//! (highest first). A group whose column the filter context reduces to one
//! item *applies* that item: the item's DAX is evaluated instead of the
//! measure, with `SELECTEDMEASURE()` standing for the measure. Several
//! filtered items (or one that does not exist) apply the group's
//! `multipleOrEmptySelectionExpression`, an unfiltered group its
//! `noSelectionExpression`, when defined (otherwise the group does not
//! apply; with the model's `selectionExpressionBehavior: visual` a
//! multiple or empty selection without an expression returns BLANK).
//!
//! Every application is recorded in the evaluation's [`Applied`] chain,
//! which flows into everything evaluated below it (the item expression,
//! `SELECTEDMEASURE()`, the measure body and the measures it references).
//! An item already on the chain is not applied again — that is what stops
//! `SELECTEDMEASURE()` from re-applying the item it is evaluated in, keeps
//! nested measure references from being transformed twice, and allows
//! *sideways recursion* (an item that selects another item of its group with
//! `CALCULATE`) while ignoring a cycle back to an applied item, as Analysis
//! Services does. Lower-precedence groups are applied when the
//! higher-precedence item evaluates `SELECTEDMEASURE()`, so the highest
//! precedence wraps the others.
//!
//! Calculation items apply only to measure references (model measures and
//! query `DEFINE MEASURE`s), never to inline aggregations (implicit
//! measures).
//!
//! The effective format string of a measure ([`Interp::measure_format_string`])
//! is the `formatStringDefinition` of the highest-precedence item that
//! applies and has one; else the measure's own dynamic format string
//! (`formatStringDefinition`), else its static `formatString`.

use std::borrow::Cow;
use std::sync::Arc;

use pbi_engine::{FilterContext, Value};
use pbi_model::names::{name_key, names_eq};
use pbi_model::{CalculationGroup, CalculationItem, Model};

use super::context::{Env, RowContext, VarScope};
use super::error::{DaxError, Result};
use super::interp::{Interp, MeasureRef};
use crate::syntax::{Expr, parse_expression};

/// A calculation group of the model, ready for evaluation.
#[derive(Debug)]
pub(crate) struct CalcGroupInfo<'a> {
    /// Model table name.
    pub table: String,
    /// Column holding the item names.
    pub column: String,
    pub group: Cow<'a, CalculationGroup>,
}

/// What an applied group selected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ItemKey {
    /// A calculation item (case-folded name).
    Item(String),
    MultipleOrEmpty,
    NoSelection,
}

#[derive(Debug)]
struct AppliedNode {
    group: usize,
    key: ItemKey,
    parent: Option<Arc<AppliedNode>>,
}

/// Calculation items applied along the current evaluation (a persistent
/// list; cheap to clone).
#[derive(Debug, Clone, Default)]
pub(crate) struct Applied {
    head: Option<Arc<AppliedNode>>,
}

impl Applied {
    pub fn contains(&self, group: usize, key: &ItemKey) -> bool {
        let mut node = self.head.as_deref();
        while let Some(n) = node {
            if n.group == group && &n.key == key {
                return true;
            }
            node = n.parent.as_deref();
        }
        false
    }

    pub fn push(&self, group: usize, key: ItemKey) -> Applied {
        Applied {
            head: Some(Arc::new(AppliedNode {
                group,
                key,
                parent: self.head.clone(),
            })),
        }
    }
}

/// The selection a filter context makes on a calculation group.
enum Selection<'g> {
    /// The group's column is not filtered.
    None,
    Single(&'g CalculationItem),
    /// Several items, or none that exists.
    MultipleOrEmpty,
}

/// An item (or selection expression) to apply.
#[derive(Debug, Clone)]
pub(crate) struct Application<'g> {
    pub group: usize,
    pub key: ItemKey,
    pub label: String,
    pub expression: &'g str,
    pub format: Option<&'g str>,
}

/// The calculation groups of `model` with a usable column, highest
/// precedence first (model order among equal precedences).
pub(crate) fn calc_groups(model: &Model) -> Vec<CalcGroupInfo<'_>> {
    let mut out: Vec<CalcGroupInfo<'_>> = model
        .tables
        .iter()
        .filter(|t| t.calculation_group.is_some() || t.extra.contains_key("calculationGroup"))
        .filter_map(|t| {
            let group = t.effective_calculation_group()?;
            let column = t.calculation_group_column()?.name.clone();
            Some(CalcGroupInfo {
                table: t.name.clone(),
                column,
                group,
            })
        })
        .collect();
    out.sort_by_key(|g| std::cmp::Reverse(g.group.precedence));
    out
}

/// `selectionExpressionBehavior: visual` on the model.
pub(crate) fn visual_selection_behavior(model: &Model) -> bool {
    model
        .extra
        .get("selectionExpressionBehavior")
        .and_then(pbi_model::PropValue::as_str)
        .is_some_and(|v| v.trim().trim_matches('"').eq_ignore_ascii_case("visual"))
}

impl Interp<'_> {
    /// `true` if a calculation item could apply to a measure evaluated in
    /// `filter`, or in `filter` plus a filter on any of `group_tables`
    /// (fast paths fall back to the general path then).
    pub(crate) fn calc_groups_may_apply<'t>(
        &self,
        filter: &FilterContext,
        group_tables: impl IntoIterator<Item = &'t str>,
    ) -> bool {
        if self.calc_groups.is_empty() {
            return false;
        }
        let tables: Vec<&str> = group_tables.into_iter().collect();
        self.calc_groups.iter().any(|g| {
            g.group.no_selection_expression.is_some()
                || filter.is_table_filtered(&g.table)
                || tables.iter().any(|t| names_eq(t, &g.table))
        })
    }

    fn calc_selection<'g>(
        &self,
        filter: &FilterContext,
        g: &'g CalcGroupInfo<'_>,
    ) -> Result<Selection<'g>> {
        if !filter.is_table_filtered(&g.table) {
            return Ok(Selection::None);
        }
        let Ok(data) = self.snapshot.table(&g.table) else {
            return Ok(Selection::MultipleOrEmpty);
        };
        let visible = self.snapshot.visible(filter, data.name())?;
        let values = self
            .snapshot
            .distinct(data.name(), &g.column, &visible.rows)?;
        Ok(match values.as_slice() {
            [Value::String(name)] => g
                .group
                .item(name)
                .map_or(Selection::MultipleOrEmpty, Selection::Single),
            _ => Selection::MultipleOrEmpty,
        })
    }

    /// The next calculation item to apply to a measure evaluated in
    /// `filter`: the highest-precedence group whose selection applies an
    /// item (or selection expression) not yet in `applied`.
    pub(crate) fn next_application(
        &self,
        filter: &FilterContext,
        applied: &Applied,
    ) -> Result<Option<Application<'_>>> {
        for (gi, g) in self.calc_groups.iter().enumerate() {
            let group: &CalculationGroup = &g.group;
            if group.no_selection_expression.is_none() && !filter.is_table_filtered(&g.table) {
                continue;
            }
            let (key, label, expression, format) = match self.calc_selection(filter, g)? {
                Selection::Single(item) => (
                    ItemKey::Item(name_key(&item.name)),
                    format!("calculation item '{}'", item.name),
                    item.expression.as_str(),
                    item.format_string_definition.as_deref(),
                ),
                Selection::MultipleOrEmpty => match &group.multiple_or_empty_selection_expression {
                    Some(e) => (
                        ItemKey::MultipleOrEmpty,
                        format!("multipleOrEmptySelectionExpression of '{}'", g.table),
                        e.expression.as_str(),
                        e.format_string_definition.as_deref(),
                    ),
                    None if self.visual_selection => (
                        ItemKey::MultipleOrEmpty,
                        format!("selection of '{}'", g.table),
                        "BLANK()",
                        None,
                    ),
                    None => continue,
                },
                Selection::None => match &group.no_selection_expression {
                    Some(e) => (
                        ItemKey::NoSelection,
                        format!("noSelectionExpression of '{}'", g.table),
                        e.expression.as_str(),
                        e.format_string_definition.as_deref(),
                    ),
                    None => continue,
                },
            };
            if applied.contains(gi, &key) {
                continue;
            }
            return Ok(Some(Application {
                group: gi,
                key,
                label,
                expression,
                format,
            }));
        }
        Ok(None)
    }

    /// Parsed expression (`format`: the format string definition) of an
    /// application, cached per interpreter.
    fn calc_expr(&self, app: &Application<'_>, format: bool) -> Result<Arc<Expr>> {
        let source = if format {
            app.format.unwrap_or_default()
        } else {
            app.expression
        };
        let key = format!("{}|{:?}|{format}", app.group, app.key);
        self.cached_expr(&key, source, &app.label)
    }

    pub(crate) fn cached_expr(&self, key: &str, source: &str, label: &str) -> Result<Arc<Expr>> {
        if let Some(e) = self.calc_exprs.borrow().get(key) {
            return Ok(Arc::clone(e));
        }
        let parsed = parse_expression(source).map_err(|e| {
            let (line, col) = e.line_col(source);
            DaxError::Parse {
                message: format!("in {label}: {e}"),
                line,
                col,
            }
        })?;
        let parsed = Arc::new(parsed);
        self.calc_exprs
            .borrow_mut()
            .insert(key.to_owned(), Arc::clone(&parsed));
        Ok(parsed)
    }

    /// Environment of a calculation item (or format string) expression:
    /// no row context, `SELECTEDMEASURE()` bound to `m`.
    fn item_env(
        &self,
        env: &Env,
        filter: Arc<FilterContext>,
        home: &str,
        applied: Applied,
        m: &MeasureRef,
    ) -> Env {
        Env {
            filter,
            rows: RowContext::default(),
            vars: VarScope::new(),
            home: Some(Arc::from(home)),
            shadow: Arc::clone(&env.shadow),
            scope: Arc::clone(&env.scope),
            applied,
            selected: Some(Arc::new(m.clone())),
        }
    }

    /// Evaluates `app` for measure `m` in `filter` (the context after the
    /// measure reference's context transition).
    pub(crate) fn apply_calc_item(
        &self,
        env: &Env,
        filter: Arc<FilterContext>,
        m: &MeasureRef,
        app: &Application<'_>,
    ) -> Result<Value> {
        let expr = self.calc_expr(app, false)?;
        let home = self
            .calc_groups
            .get(app.group)
            .map(|g| g.table.as_str())
            .unwrap_or_default();
        let applied = env.applied.push(app.group, app.key.clone());
        let ienv = self.item_env(env, filter, home, applied, m);
        self.scalar(&ienv, &expr)
    }

    /// The format string of measure `m` evaluated in `env` (see the module
    /// docs): calculation items not in `env`'s chain first, then the
    /// measure's own dynamic and static format strings. `None`: no format
    /// string.
    pub(crate) fn measure_format_string(
        &self,
        env: &Env,
        m: &MeasureRef,
    ) -> Result<Option<String>> {
        let filter = self.transition(env)?;
        let mut applied = env.applied.clone();
        while !self.calc_groups.is_empty() {
            let Some(app) = self.next_application(&filter, &applied)? else {
                break;
            };
            applied = applied.push(app.group, app.key.clone());
            if app.format.is_some() {
                let expr = self.calc_expr(&app, true)?;
                let home = self
                    .calc_groups
                    .get(app.group)
                    .map(|g| g.table.clone())
                    .unwrap_or_default();
                let fenv = self.item_env(env, Arc::clone(&filter), &home, applied, m);
                return Ok(format_text(&self.scalar(&fenv, &expr)?));
            }
        }
        if m.query {
            return Ok(None);
        }
        let Some(measure) = self.model.measure(&m.table, &m.name) else {
            return Ok(None);
        };
        if let Some(definition) = measure.format_string_definition() {
            let key = format!(
                "measure-format|{}|{}",
                name_key(&m.table),
                name_key(&m.name)
            );
            let label = format!("the format string of measure [{}]", m.name);
            let expr = self.cached_expr(&key, &definition, &label)?;
            let fenv = self.item_env(env, filter, &m.table, env.applied.clone(), m);
            if let Some(text) = format_text(&self.scalar(&fenv, &expr)?) {
                return Ok(Some(text));
            }
        }
        Ok(measure.format_string.clone())
    }
}

/// Text of an evaluated format string (`None` for BLANK).
fn format_text(value: &Value) -> Option<String> {
    match value {
        Value::Blank => None,
        Value::String(s) => Some(s.to_string()),
        other => Some(other.to_string()),
    }
}
