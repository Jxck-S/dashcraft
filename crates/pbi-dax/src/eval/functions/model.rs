//! Calculation-group functions (`SELECTEDMEASURE`, `SELECTEDMEASURENAME`,
//! `SELECTEDMEASUREFORMATSTRING`, `ISSELECTEDMEASURE`), the effective
//! user (`USERPRINCIPALNAME`, `USERNAME`, `USEROBJECTID`, `CUSTOMDATA`) and
//! object names (`NAMEOF`).

use std::sync::Arc;

use pbi_engine::{UserIdentity, Value};
use pbi_model::names::{name_key, names_eq};

use super::{Builtin, arg, arity, opt_arg, scalar_fn};
use crate::eval::context::Env;
use crate::eval::error::{DaxError, Result};
use crate::eval::interp::{Interp, MeasureRef, Val, snippet};
use crate::eval::table::VirtualTable;
use crate::syntax::{Expr, ExprKind};

pub(super) const BUILTINS: &[Builtin] = &[
    scalar_fn("SELECTEDMEASURE", selectedmeasure),
    scalar_fn("SELECTEDMEASURENAME", selectedmeasurename),
    scalar_fn("SELECTEDMEASUREFORMATSTRING", selectedmeasureformatstring),
    scalar_fn("ISSELECTEDMEASURE", isselectedmeasure),
    scalar_fn("USERPRINCIPALNAME", userprincipalname),
    scalar_fn("USERNAME", username),
    scalar_fn("USEROBJECTID", userobjectid),
    scalar_fn("CUSTOMDATA", customdata),
    scalar_fn("NAMEOF", nameof),
];

type R = Result<Val>;

fn ok(v: Value) -> R {
    Ok(Val::Scalar(v))
}

/// The measure a calculation item (or dynamic format string) applies to.
fn selected<'e>(env: &'e Env, function: &str) -> Result<&'e Arc<MeasureRef>> {
    env.selected.as_ref().ok_or_else(|| {
        DaxError::evaluation(format!(
            "{function} can only be used in the expression of a calculation item or a dynamic format string."
        ))
    })
}

fn selectedmeasure(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    arity("SELECTEDMEASURE", args, 0, 0)?;
    let m = selected(env, "SELECTEDMEASURE")?;
    ok(i.eval_measure(env, m)?)
}

fn selectedmeasurename(_: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    arity("SELECTEDMEASURENAME", args, 0, 0)?;
    let m = selected(env, "SELECTEDMEASURENAME")?;
    ok(Value::string(m.name.as_str()))
}

fn selectedmeasureformatstring(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    arity("SELECTEDMEASUREFORMATSTRING", args, 0, 0)?;
    let m = selected(env, "SELECTEDMEASUREFORMATSTRING")?;
    ok(i.measure_format_string(env, m)?
        .map_or(Value::Blank, Value::string))
}

fn isselectedmeasure(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    arity("ISSELECTEDMEASURE", args, 1, usize::MAX)?;
    let m = selected(env, "ISSELECTEDMEASURE")?;
    let mut found = false;
    for a in args {
        let ExprKind::ColumnRef { table, column } = &a.unparen().kind else {
            return Err(DaxError::argument(
                "ISSELECTEDMEASURE",
                format!("expects measure references, got {}", snippet(a)),
            ));
        };
        let Some(listed) = i.find_measure(table.as_deref(), column)? else {
            return Err(DaxError::UnknownMeasure {
                name: column.clone(),
            });
        };
        found |= names_eq(&listed.name, &m.name);
    }
    ok(Value::Boolean(found))
}

fn identity<'s>(i: &'s Interp<'_>) -> Option<&'s UserIdentity> {
    i.snapshot.security().map(|s| &s.identity)
}

/// The effective user name (empty text when none is set).
fn user_name(i: &Interp<'_>) -> Value {
    Value::string(
        identity(i)
            .and_then(|u| u.user_name.as_deref())
            .unwrap_or_default(),
    )
}

fn userprincipalname(i: &Interp<'_>, _: &Env, args: &[Expr]) -> R {
    arity("USERPRINCIPALNAME", args, 0, 0)?;
    ok(user_name(i))
}

fn username(i: &Interp<'_>, _: &Env, args: &[Expr]) -> R {
    arity("USERNAME", args, 0, 0)?;
    ok(user_name(i))
}

fn userobjectid(i: &Interp<'_>, _: &Env, args: &[Expr]) -> R {
    arity("USEROBJECTID", args, 0, 0)?;
    ok(Value::string(
        identity(i)
            .and_then(|u| u.user_object_id.as_deref())
            .unwrap_or_default(),
    ))
}

fn customdata(i: &Interp<'_>, _: &Env, args: &[Expr]) -> R {
    arity("CUSTOMDATA", args, 0, 0)?;
    ok(identity(i)
        .and_then(|u| u.custom_data.as_deref())
        .map_or(Value::Blank, Value::string))
}

/// The object a `NAMEOF` argument names, with model spellings.
enum Named {
    Table(String),
    Column(String, String),
    Measure(String, String),
}

const NAMEOF_REFERENCE: &str = "Only measure or fully-qualified column or table or calendar references are allowed in NAMEOF function.";

/// `NAMEOF(reference[, component])`: the name of a measure, a fully
/// qualified column or a table as DAX text in the model's spelling
/// (`'Orders'[Amount]`, always quoted). The optional component keyword
/// (`TABLE`, `COLUMN`, `MEASURE`, `FULL`, `SELF`, `PARENT`) picks a part.
fn nameof(i: &Interp<'_>, _: &Env, args: &[Expr]) -> R {
    arity("NAMEOF", args, 1, 2)?;
    let named = nameof_object(i, arg("NAMEOF", args, 0)?)?;
    let component = match opt_arg(args, 1) {
        None => "FULL".to_owned(),
        Some(e) => nameof_component(e)?,
    };
    let quote_table = |t: &str| format!("'{}'", t.replace('\'', "''"));
    let bracket = |n: &str| format!("[{}]", n.replace(']', "]]"));
    let text = match (&named, component.as_str()) {
        (Named::Table(t), "TABLE" | "FULL" | "SELF") => quote_table(t),
        (Named::Column(t, _) | Named::Measure(t, _), "TABLE" | "PARENT") => quote_table(t),
        (Named::Column(_, c), "COLUMN" | "SELF") | (Named::Measure(_, c), "MEASURE" | "SELF") => {
            bracket(c)
        }
        (Named::Column(t, c) | Named::Measure(t, c), "FULL") => {
            format!("{}{}", quote_table(t), bracket(c))
        }
        _ => {
            return Err(DaxError::evaluation(format!(
                "The value of component, '{component}', is not valid for the given object in NAMEOF function."
            )));
        }
    };
    ok(Value::string(text))
}

fn nameof_object(i: &Interp<'_>, e: &Expr) -> Result<Named> {
    match &e.unparen().kind {
        ExprKind::TableRef { name } => {
            if let Some(t) = i.model.table(name) {
                return Ok(Named::Table(t.name.clone()));
            }
            if let Ok(data) = i.snapshot.table(name) {
                return Ok(Named::Table(data.name().to_owned()));
            }
            match i.query_tables.get(&name_key(name)) {
                Some(t) => Ok(Named::Table(query_table_name(t, name))),
                None => Err(DaxError::UnknownTable {
                    name: name.to_owned(),
                }),
            }
        }
        ExprKind::ColumnRef {
            table: Some(table),
            column,
        } => {
            if let Some(m) = i.find_measure(Some(table), column)? {
                return Ok(Named::Measure(m.table, m.name));
            }
            if let Some(t) = i.model.table(table) {
                return match t.column(column) {
                    Some(c) => Ok(Named::Column(t.name.clone(), c.name.clone())),
                    None => Err(nameof_unknown_column(&t.name, column)),
                };
            }
            if let Ok(data) = i.snapshot.table(table) {
                return match data.column_name(column) {
                    Ok(c) => Ok(Named::Column(data.name().to_owned(), c.to_owned())),
                    Err(_) => Err(nameof_unknown_column(data.name(), column)),
                };
            }
            let Some(t) = i.query_tables.get(&name_key(table)) else {
                return Err(DaxError::UnknownTable {
                    name: table.to_owned(),
                });
            };
            let table_name = query_table_name(t, table);
            match t.columns.iter().find(|c| names_eq(&c.name, column)) {
                Some(c) => Ok(Named::Column(table_name, c.name.clone())),
                None => Err(nameof_unknown_column(&table_name, column)),
            }
        }
        ExprKind::ColumnRef {
            table: None,
            column,
        } => match i.find_measure(None, column)? {
            Some(m) => Ok(Named::Measure(m.table, m.name)),
            None => Err(DaxError::evaluation(NAMEOF_REFERENCE)),
        },
        _ => Err(DaxError::evaluation(NAMEOF_REFERENCE)),
    }
}

/// The spelling of a `DEFINE TABLE` name (its columns are qualified by it).
fn query_table_name(t: &VirtualTable, written: &str) -> String {
    t.columns
        .iter()
        .find_map(|c| c.table.clone())
        .unwrap_or_else(|| written.to_owned())
}

fn nameof_unknown_column(table: &str, column: &str) -> DaxError {
    DaxError::evaluation(format!(
        "Column '{column}' in table '{table}' cannot be found or may not be used in this expression."
    ))
}

/// The component keyword of `NAMEOF`, upper-cased. A string or any other
/// expression is an error, like in Power BI.
fn nameof_component(e: &Expr) -> Result<String> {
    let word = match &e.unparen().kind {
        ExprKind::TableRef { name } | ExprKind::Keyword(name) => name.to_ascii_uppercase(),
        _ => String::new(),
    };
    if matches!(
        word.as_str(),
        "TABLE" | "COLUMN" | "MEASURE" | "CALENDAR" | "FULL" | "SELF" | "PARENT"
    ) {
        Ok(word)
    } else {
        Err(DaxError::evaluation(
            "Valid arguments for component are TABLE, COLUMN, MEASURE, CALENDAR, FULL, SELF, and PARENT.",
        ))
    }
}
