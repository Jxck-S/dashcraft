//! DAX queries: `DEFINE` (MEASURE, VAR, TABLE, FUNCTION), `EVALUATE`,
//! `ORDER BY`, `START AT`.

use std::cmp::Ordering;
use std::sync::Arc;

use pbi_engine::EngineSnapshot;
use pbi_model::Model;
use pbi_model::names::name_key;

use super::context::{Env, VarValue};
use super::error::{DaxError, Result};
use super::functions::{cmp_keys, sort_positions};
use super::interp::{Interp, QueryMeasure, Udf, Val};
use super::table::{RowPos, VirtualTable};
use super::{EvalOptions, on_eval_thread};
use crate::syntax::{Define, Evaluate, SortDir, parse_query};

/// Evaluates a DAX query and returns one table per `EVALUATE`, sorted per
/// its `ORDER BY` (and trimmed by `START AT`).
///
/// `DEFINE MEASURE`, `VAR`, `TABLE` and `FUNCTION` (parameters bound by
/// value) are supported; `DEFINE COLUMN` returns
/// [`DaxError::Unsupported`].
pub fn evaluate_query(
    snapshot: &EngineSnapshot,
    model: &Model,
    query_src: &str,
) -> Result<Vec<VirtualTable>> {
    evaluate_query_with(snapshot, model, query_src, &EvalOptions::default())
}

pub(crate) fn evaluate_query_with(
    snapshot: &EngineSnapshot,
    model: &Model,
    query_src: &str,
    opts: &EvalOptions,
) -> Result<Vec<VirtualTable>> {
    let query = parse_query(query_src).map_err(|e| DaxError::from_parse(&e, query_src))?;
    on_eval_thread(|| {
        let mut interp = Interp::with_options(snapshot, model, opts);
        for d in &query.defines {
            match d {
                Define::Measure {
                    table, name, expr, ..
                } => {
                    let table = model
                        .table(table)
                        .map_or_else(|| table.clone(), |t| t.name.clone());
                    interp.query_measures.insert(
                        name_key(name),
                        QueryMeasure {
                            table,
                            name: name.clone(),
                            expr: Arc::new(expr.clone()),
                        },
                    );
                }
                Define::Function {
                    name, params, body, ..
                } => {
                    interp.functions.insert(
                        name_key(name),
                        Udf {
                            params: params.clone(),
                            body: Arc::new(body.clone()),
                        },
                    );
                }
                Define::Column { table, name, .. } => {
                    return Err(DaxError::Unsupported {
                        feature: format!(
                            "DEFINE COLUMN {}",
                            pbi_model::dax_column_ref(table, name)
                        ),
                    });
                }
                Define::Var { .. } | Define::Table { .. } => {}
            }
        }
        interp.query_vars = opts.vars.clone();
        for d in &query.defines {
            let env = base_env(&interp, opts);
            match d {
                Define::Var { name, expr, .. } => {
                    let value = match interp.eval(&env, expr)? {
                        Val::Scalar(v) => VarValue::Scalar(v),
                        Val::Table(t) => VarValue::Table(t),
                    };
                    interp.query_vars = interp.query_vars.bind(name, value);
                }
                Define::Table { name, expr, .. } => {
                    let table = interp.table(&env, expr)?.requalified(name);
                    interp.query_tables.insert(name_key(name), Arc::new(table));
                }
                _ => {}
            }
        }
        let mut out = Vec::with_capacity(query.evaluates.len());
        for ev in &query.evaluates {
            let env = base_env(&interp, opts);
            out.push(evaluate_one(&interp, &env, ev)?);
        }
        Ok(out)
    })
}

fn base_env(interp: &Interp<'_>, opts: &EvalOptions) -> Env {
    let mut env = opts.env();
    env.vars = interp.query_vars.clone();
    env
}

fn evaluate_one(interp: &Interp<'_>, env: &Env, ev: &Evaluate) -> Result<VirtualTable> {
    let table = interp.table(env, &ev.expr)?;
    if ev.order_by.is_empty() {
        return Ok((*table).clone());
    }
    let specs: Vec<_> = ev
        .order_by
        .iter()
        .map(|(e, dir)| (e, *dir == SortDir::Desc))
        .collect();
    let desc: Vec<bool> = specs.iter().map(|(_, d)| *d).collect();
    let sorted = sort_positions(interp, env, &table, &specs)?;
    let start = ev
        .start_at
        .iter()
        .map(|e| interp.scalar(env, e))
        .collect::<Result<Vec<_>>>()?;
    let keep: Vec<RowPos> = sorted
        .into_iter()
        .filter(|(keys, _)| {
            start.is_empty()
                || cmp_keys(keys.get(..start.len()).unwrap_or(keys), &start, &desc)
                    != Ordering::Less
        })
        .map(|(_, p)| p)
        .collect();
    Ok(table.subset_ordered(&keep))
}
