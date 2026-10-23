//! Dependency extraction for the refresh planner.

use std::collections::HashSet;

use pbi_model::Model;
use pbi_model::names::name_key;

use super::error::Result;
use super::interp::parse;
use crate::syntax::{Expr, Reference, reference_sites};

/// A model object an expression depends on.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Dependency {
    /// A whole table (`'Web Sessions'`, `RELATEDTABLE(Orders)`).
    Table(String),
    /// A column (`Orders[Store ID]`, or a bare `[Store ID]` resolved to a
    /// column).
    Column { table: String, column: String },
    /// A measure (`[Total Sales]`, `Orders[Total Sales]`).
    Measure(String),
}

impl Dependency {
    fn key(&self) -> (u8, String, String) {
        match self {
            Dependency::Table(t) => (0, name_key(t), String::new()),
            Dependency::Column { table, column } => (1, name_key(table), name_key(column)),
            Dependency::Measure(m) => (2, name_key(m), String::new()),
        }
    }

    /// The table this dependency belongs to (`None` for measures).
    pub fn table(&self) -> Option<&str> {
        match self {
            Dependency::Table(t) | Dependency::Column { table: t, .. } => Some(t),
            Dependency::Measure(_) => None,
        }
    }
}

/// Direct dependencies of a DAX expression, with model spelling. Bare
/// `[Name]` references resolve to a measure, else to the columns of that
/// name (all tables that have one). Unknown references are kept as written.
pub fn dependencies(model: &Model, expr_src: &str) -> Result<Vec<Dependency>> {
    dependencies_in(model, None, expr_src)
}

/// Like [`dependencies`], resolving bare `[Column]` references against
/// `home_table` (the table of a calculated column or measure) first.
pub fn dependencies_in(
    model: &Model,
    home_table: Option<&str>,
    expr_src: &str,
) -> Result<Vec<Dependency>> {
    let expr = parse(expr_src)?;
    Ok(collect(model, home_table, &expr))
}

/// Dependencies including those of every measure reached (transitively,
/// cycle-safe). Measures whose expression does not parse are skipped.
pub fn transitive_dependencies(
    model: &Model,
    home_table: Option<&str>,
    expr_src: &str,
) -> Result<Vec<Dependency>> {
    let mut out = dependencies_in(model, home_table, expr_src)?;
    let mut seen: HashSet<(u8, String, String)> = out.iter().map(Dependency::key).collect();
    let mut i = 0;
    while let Some(dep) = out.get(i).cloned() {
        i += 1;
        let Dependency::Measure(name) = dep else {
            continue;
        };
        let Some((table, measure)) = model.find_measure(&name) else {
            continue;
        };
        let Ok(expr) = parse(&measure.expression) else {
            continue;
        };
        for d in collect(model, Some(&table.name), &expr) {
            if seen.insert(d.key()) {
                out.push(d);
            }
        }
    }
    Ok(out)
}

fn collect(model: &Model, home: Option<&str>, expr: &Expr) -> Vec<Dependency> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    let mut push = |d: Dependency| {
        if seen.insert(d.key()) {
            out.push(d);
        }
    };
    for (r, _) in reference_sites(expr) {
        match r {
            Reference::Table(t) => {
                push(Dependency::Table(
                    model.table(&t).map_or(t, |mt| mt.name.clone()),
                ));
            }
            Reference::Column { table, column } => match model.table(&table) {
                Some(mt) => {
                    if let Some(c) = mt.column(&column) {
                        push(Dependency::Column {
                            table: mt.name.clone(),
                            column: c.name.clone(),
                        });
                    } else if let Some(m) = mt.measure(&column) {
                        push(Dependency::Measure(m.name.clone()));
                    } else {
                        push(Dependency::Column {
                            table: mt.name.clone(),
                            column,
                        });
                    }
                }
                None => push(Dependency::Column { table, column }),
            },
            Reference::Measure(name) => {
                if let Some((_, m)) = model.find_measure(&name) {
                    push(Dependency::Measure(m.name.clone()));
                    continue;
                }
                if let Some(h) = home
                    && let Some(mt) = model.table(h)
                    && let Some(c) = mt.column(&name)
                {
                    push(Dependency::Column {
                        table: mt.name.clone(),
                        column: c.name.clone(),
                    });
                    continue;
                }
                for mt in &model.tables {
                    if let Some(c) = mt.column(&name) {
                        push(Dependency::Column {
                            table: mt.name.clone(),
                            column: c.name.clone(),
                        });
                    }
                }
            }
            Reference::Function(_) => {}
        }
    }
    out
}
