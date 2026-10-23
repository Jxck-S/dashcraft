#![allow(clippy::unwrap_used, clippy::expect_used, dead_code)]
//! Shared helpers for the integration tests.

pub mod data;

use std::path::{Path, PathBuf};

use pbi_dax::syntax::{Expr, ExprKind, Number, UnaryOp};

/// `(path, contents)` of every `*.dax` file in `tests/data/<sub>`, sorted.
pub fn data_files(sub: &str) -> Vec<(PathBuf, String)> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/data")
        .join(sub);
    let mut files: Vec<(PathBuf, String)> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "dax"))
        .map(|p| {
            let text = std::fs::read_to_string(&p).unwrap();
            (p, text)
        })
        .collect();
    files.sort();
    files
}

/// Compact S-expression rendering of an AST for shape assertions.
pub fn sexp(e: &Expr) -> String {
    let list =
        |items: &[Expr]| -> String { items.iter().map(|i| format!(" {}", sexp(i))).collect() };
    match &e.kind {
        ExprKind::Number(Number::Integer(i)) => i.to_string(),
        ExprKind::Number(Number::Decimal(d)) => format!("{d:?}"),
        ExprKind::String(s) => format!("{s:?}"),
        ExprKind::DateTime(s) => format!("dt{s:?}"),
        ExprKind::Boolean(b) => b.to_string().to_uppercase(),
        ExprKind::TableRef { name } => format!("table:{name}"),
        ExprKind::ColumnRef { table, column } => {
            format!("col:{}[{column}]", table.as_deref().unwrap_or(""))
        }
        ExprKind::Call { name, args, .. } => format!("({name}{})", list(args)),
        ExprKind::Var { bindings, body } => {
            let b: String = bindings
                .iter()
                .map(|b| format!(" [{} {}]", b.name, sexp(&b.expr)))
                .collect();
            format!("(var{b} {})", sexp(body))
        }
        ExprKind::VarRef(n) => format!("${n}"),
        ExprKind::Parameter(n) => format!("@{n}"),
        ExprKind::Keyword(k) => format!("kw:{k}"),
        ExprKind::Unary { op, expr } => {
            let name = match op {
                UnaryOp::Plus => "pos",
                UnaryOp::Minus => "neg",
                UnaryOp::Not => "not",
            };
            format!("({name} {})", sexp(expr))
        }
        ExprKind::Binary { op, lhs, rhs } => {
            format!("({} {} {})", op.symbol(), sexp(lhs), sexp(rhs))
        }
        ExprKind::TableConstructor { rows } => {
            let rows: Vec<String> = rows
                .iter()
                .map(|r| format!("({})", list(r).trim_start()))
                .collect();
            format!("{{{}}}", rows.join(" "))
        }
        ExprKind::Tuple(items) => format!("(tuple{})", list(items)),
        ExprKind::Paren(inner) => format!("(paren {})", sexp(inner)),
        ExprKind::Omitted => "_".to_string(),
    }
}
