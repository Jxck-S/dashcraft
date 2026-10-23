//! Simple, serializable filter descriptions ([`FilterSpec`]) and their
//! translation to engine filter contexts, plus report-level categorical
//! filters ([`filters_from_report`]).
//!
//! This covers the slicer / filter-pane selections the explorer and the
//! KPI checks need: `table[column] IN {values}` and `NOT IN`. Full
//! semantic-query filter translation (advanced, Top N, relative date,
//! tuples) is Phase 2; such filters are listed in
//! [`FilterSpec::unsupported`] instead of being dropped silently.

use std::collections::HashSet;
use std::sync::Arc;

use pbi_engine::{ColumnFilter, FilterContext, Value, ValueKey};
use pbi_model::{
    Condition, DataType, FieldExpr, Filter, FilterKind, LiteralValue, Model, Page, Report,
};
use serde::{Deserialize, Serialize};
use serde_json::Value as Json;

use crate::error::SessionError;

/// `table[column] IN values` (or `NOT IN` with `exclude`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ColumnFilterSpec {
    /// Table name (case-insensitive).
    pub table: String,
    /// Column name (case-insensitive).
    pub column: String,
    /// Selected values: JSON strings, numbers, booleans or `null` (BLANK).
    /// Values are converted to the column's type (`"42"` matches an
    /// integer column; ISO text matches a date column). Text matching is
    /// case-insensitive, as in DAX.
    pub values: Vec<Json>,
    /// Keep the rows whose value is *not* in `values`.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub exclude: bool,
}

impl ColumnFilterSpec {
    /// `table[column] IN values`.
    pub fn include<I, V>(table: impl Into<String>, column: impl Into<String>, values: I) -> Self
    where
        I: IntoIterator<Item = V>,
        V: Into<Json>,
    {
        Self {
            table: table.into(),
            column: column.into(),
            values: values.into_iter().map(Into::into).collect(),
            exclude: false,
        }
    }

    /// `NOT table[column] IN values`.
    pub fn exclude<I, V>(table: impl Into<String>, column: impl Into<String>, values: I) -> Self
    where
        I: IntoIterator<Item = V>,
        V: Into<Json>,
    {
        Self {
            exclude: true,
            ..Self::include(table, column, values)
        }
    }
}

/// A report filter that [`filters_from_report`] could not translate.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UnsupportedFilter {
    /// Filter name (PBIR `name`).
    pub name: String,
    /// Filter type (`Categorical`, `Advanced`, `TopN`, …).
    pub kind: String,
    /// Filtered field, as a query ref (`Stores.Region`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub field: Option<String>,
    /// Why it was not applied.
    pub reason: String,
}

/// A set of column filters (combined with AND), plus report filters that
/// could not be translated.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct FilterSpec {
    /// Column filters, all applied.
    pub filters: Vec<ColumnFilterSpec>,
    /// Filters that are *not* applied (see [`filters_from_report`]).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub unsupported: Vec<UnsupportedFilter>,
}

impl FilterSpec {
    /// No filter.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// `true` if no filter is applied.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.filters.is_empty()
    }

    /// Adds a column filter.
    #[must_use]
    pub fn with(mut self, filter: ColumnFilterSpec) -> Self {
        self.filters.push(filter);
        self
    }

    /// Appends `other`'s filters and unsupported entries (e.g. report plus
    /// page filters).
    pub fn extend(&mut self, other: FilterSpec) {
        self.filters.extend(other.filters);
        self.unsupported.extend(other.unsupported);
    }

    /// The engine filter context: one column filter per entry (several
    /// entries on one column intersect).
    ///
    /// # Errors
    /// [`SessionError::UnknownTable`] / [`SessionError::UnknownColumn`] for
    /// names not in `model`, [`SessionError::InvalidFilter`] for values that
    /// are not scalars.
    pub fn to_filter_context(&self, model: &Model) -> Result<FilterContext, SessionError> {
        let mut ctx = FilterContext::new();
        for f in &self.filters {
            let table = model
                .table(&f.table)
                .ok_or_else(|| SessionError::UnknownTable(f.table.clone()))?;
            let column = table
                .column(&f.column)
                .ok_or_else(|| SessionError::UnknownColumn {
                    table: table.name.clone(),
                    column: f.column.clone(),
                })?;
            let mut keys = HashSet::with_capacity(f.values.len());
            for v in &f.values {
                let value = json_to_value(v, &column.data_type).map_err(|reason| {
                    SessionError::InvalidFilter(format!(
                        "{}: {reason}",
                        pbi_model::dax_column_ref(&table.name, &column.name)
                    ))
                })?;
                keys.insert(ValueKey::from(&value));
            }
            let filter = if f.exclude {
                let keys = Arc::new(keys);
                ColumnFilter::predicate(&table.name, &column.name, move |v| {
                    !keys.contains(&ValueKey::from(v))
                })
                .with_label("NOT IN")
            } else {
                ColumnFilter::in_keys(&table.name, &column.name, keys)
            };
            ctx.add(filter);
        }
        Ok(ctx)
    }
}

/// Converts a JSON filter value to an engine value of a column's type.
fn json_to_value(v: &Json, data_type: &DataType) -> Result<Value, String> {
    Ok(match v {
        Json::Null => Value::Blank,
        Json::Bool(b) => Value::Boolean(*b),
        Json::Number(n) => {
            let int = n.as_i64();
            let float = n.as_f64();
            match data_type {
                DataType::String => Value::String(Arc::from(n.to_string())),
                DataType::Decimal => float
                    .and_then(Value::decimal_from_f64)
                    .ok_or_else(|| format!("{n} is not a valid fixed decimal"))?,
                DataType::DateTime => float
                    .and_then(Value::from_oadate)
                    .ok_or_else(|| format!("{n} is not a valid date serial number"))?,
                _ => match (int, float) {
                    (Some(i), _) => Value::Integer(i),
                    (None, Some(f)) => Value::Double(f),
                    _ => return Err(format!("{n} is not a supported number")),
                },
            }
        }
        Json::String(s) => text_to_value(s, data_type),
        Json::Array(_) | Json::Object(_) => {
            return Err("filter values must be strings, numbers, booleans or null".into());
        }
    })
}

/// Text filter value of a column's type; text that does not parse stays
/// text (and so matches nothing in a numeric column).
fn text_to_value(s: &str, data_type: &DataType) -> Value {
    let t = s.trim();
    let parsed = match data_type {
        DataType::Int64 => t
            .parse::<i64>()
            .ok()
            .map(Value::Integer)
            .or_else(|| t.parse::<f64>().ok().map(Value::Double)),
        DataType::Double => t.parse::<f64>().ok().map(Value::Double),
        DataType::Decimal => t.parse::<f64>().ok().and_then(Value::decimal_from_f64),
        DataType::Boolean => match t.to_ascii_lowercase().as_str() {
            "true" => Some(Value::Boolean(true)),
            "false" => Some(Value::Boolean(false)),
            _ => None,
        },
        DataType::DateTime => parse_datetime(t),
        _ => None,
    };
    parsed.unwrap_or_else(|| Value::String(Arc::from(s)))
}

/// `YYYY-MM-DD[(T| )hh:mm[:ss[.fff]]]` (an optional trailing `Z` is
/// ignored).
pub(crate) fn parse_datetime(text: &str) -> Option<Value> {
    let t = text.trim().trim_end_matches('Z');
    for f in [
        "%Y-%m-%dT%H:%M:%S%.f",
        "%Y-%m-%d %H:%M:%S%.f",
        "%Y-%m-%dT%H:%M",
        "%Y-%m-%d %H:%M",
    ] {
        if let Ok(dt) = chrono::NaiveDateTime::parse_from_str(t, f) {
            return Some(Value::DateTime(dt.and_utc().timestamp_micros()));
        }
    }
    chrono::NaiveDate::parse_from_str(t, "%Y-%m-%d")
        .ok()
        .and_then(|d| d.and_hms_opt(0, 0, 0))
        .map(|dt| Value::DateTime(dt.and_utc().timestamp_micros()))
}

/// Report-level filters of `report` as a [`FilterSpec`].
#[must_use]
pub fn filters_from_report(report: &Report) -> FilterSpec {
    filters_from(&report.filters)
}

/// Page-level filters of `page` as a [`FilterSpec`] (combine with
/// [`filters_from_report`] via [`FilterSpec::extend`]).
#[must_use]
pub fn filters_from_page(page: &Page) -> FilterSpec {
    filters_from(&page.filters)
}

/// Translates report-AST filters: single-column categorical `In` / `Not In`
/// selections become [`ColumnFilterSpec`]s; filters without a selection are
/// no-ops and skipped; everything else is listed in
/// [`FilterSpec::unsupported`].
#[must_use]
pub fn filters_from(filters: &[Filter]) -> FilterSpec {
    let mut spec = FilterSpec::default();
    for f in filters {
        let Some(condition) = &f.condition else {
            continue;
        };
        let unsupported = |reason: &str| UnsupportedFilter {
            name: f.name.clone(),
            kind: f.kind.as_str().to_owned(),
            field: f.field.as_ref().and_then(FieldExpr::query_ref),
            reason: reason.to_owned(),
        };
        if matches!(
            f.kind,
            FilterKind::TopN | FilterKind::RelativeDate | FilterKind::RelativeTime
        ) {
            spec.unsupported.push(unsupported(
                "Top N and relative date/time filters are not supported yet",
            ));
            continue;
        }
        let (inner, exclude) = match condition {
            Condition::Not(inner) => (inner.as_ref(), true),
            other => (other, false),
        };
        match in_filter(inner, exclude) {
            Ok(column_filter) => spec.filters.push(column_filter),
            Err(reason) => spec.unsupported.push(unsupported(&reason)),
        }
    }
    spec
}

/// A single-column `In` condition as a column filter.
fn in_filter(condition: &Condition, exclude: bool) -> Result<ColumnFilterSpec, String> {
    let Condition::In { exprs, values } = condition else {
        return Err("only categorical (In / Not In) conditions are supported yet".into());
    };
    let [expr] = exprs.as_slice() else {
        return Err("multi-column (tuple) filters are not supported yet".into());
    };
    let FieldExpr::Column { entity, property } = expr else {
        return Err("only column filters are supported yet".into());
    };
    let mut out = Vec::with_capacity(values.len());
    for row in values {
        let [literal] = row.as_slice() else {
            return Err("multi-column (tuple) values are not supported yet".into());
        };
        let value = literal
            .value()
            .map_err(|e| format!("cannot read filter value: {e}"))?;
        out.push(literal_to_json(&value));
    }
    Ok(ColumnFilterSpec {
        table: entity.clone(),
        column: property.clone(),
        values: out,
        exclude,
    })
}

fn literal_to_json(v: &LiteralValue) -> Json {
    match v {
        LiteralValue::Text(s) | LiteralValue::DateTime(s) => Json::String(s.clone()),
        LiteralValue::Integer(i) => Json::from(*i),
        LiteralValue::Double(f) | LiteralValue::Decimal(f) => {
            serde_json::Number::from_f64(*f).map_or(Json::Null, Json::Number)
        }
        LiteralValue::Boolean(b) => Json::Bool(*b),
        LiteralValue::Null => Json::Null,
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;
    use pbi_model::{Column, Literal, Table};

    fn model() -> Model {
        let mut t = Table::new("Stores");
        t.columns = vec![
            Column::data("Region", DataType::String),
            Column::data("Count", DataType::Int64),
            Column::data("When", DataType::DateTime),
        ];
        Model {
            tables: vec![t],
            ..Model::default()
        }
    }

    fn filter(kind: FilterKind, condition: Condition) -> Filter {
        Filter {
            name: "f".into(),
            kind,
            condition: Some(condition),
            ..Filter::default()
        }
    }

    fn in_condition(values: &[Literal]) -> Condition {
        Condition::In {
            exprs: vec![FieldExpr::column("Stores", "Region")],
            values: values.iter().map(|v| vec![v.clone()]).collect(),
        }
    }

    #[test]
    fn categorical_in_and_not_in_are_translated() {
        let filters = vec![
            filter(
                FilterKind::Categorical,
                in_condition(&[Literal::text("West")]),
            ),
            filter(
                FilterKind::Categorical,
                Condition::Not(Box::new(in_condition(&[Literal::null()]))),
            ),
            Filter {
                name: "empty".into(),
                ..Filter::default()
            },
        ];
        let spec = filters_from(&filters);
        assert_eq!(
            spec.filters,
            vec![
                ColumnFilterSpec::include("Stores", "Region", ["West"]),
                ColumnFilterSpec::exclude("Stores", "Region", [Json::Null]),
            ]
        );
        assert!(spec.unsupported.is_empty());
        let ctx = spec.to_filter_context(&model()).unwrap();
        assert_eq!(ctx.len(), 2);
    }

    #[test]
    fn other_filters_are_listed_as_unsupported() {
        let filters = vec![
            filter(
                FilterKind::Advanced,
                Condition::Contains {
                    left: FieldExpr::column("Stores", "Region"),
                    right: FieldExpr::Literal(Literal::text("C")),
                },
            ),
            filter(FilterKind::TopN, in_condition(&[Literal::text("x")])),
        ];
        let spec = filters_from(&filters);
        assert!(spec.filters.is_empty());
        assert_eq!(spec.unsupported.len(), 2);
        assert_eq!(spec.unsupported[1].kind, "TopN");
    }

    #[test]
    fn values_are_converted_to_the_column_type() {
        let m = model();
        let spec = FilterSpec::new()
            .with(ColumnFilterSpec::include("stores", "COUNT", ["42"]))
            .with(ColumnFilterSpec::include(
                "Stores",
                "When",
                ["2024-01-02T03:04:05"],
            ));
        assert!(spec.to_filter_context(&m).is_ok());
        assert_eq!(text_to_value("42", &DataType::Int64), Value::Integer(42));
        assert_eq!(
            text_to_value("2024-01-02", &DataType::DateTime),
            Value::date(2024, 1, 2).unwrap()
        );
        let bad = FilterSpec::new().with(ColumnFilterSpec::include("Nope", "x", [1]));
        assert!(matches!(
            bad.to_filter_context(&m),
            Err(SessionError::UnknownTable(_))
        ));
        let bad = FilterSpec::new().with(ColumnFilterSpec {
            table: "Stores".into(),
            column: "Count".into(),
            values: vec![serde_json::json!([1])],
            exclude: false,
        });
        assert!(matches!(
            bad.to_filter_context(&m),
            Err(SessionError::InvalidFilter(_))
        ));
    }

    #[test]
    fn spec_round_trips_through_json() {
        let spec = FilterSpec::new().with(ColumnFilterSpec::exclude("Stores", "Region", ["East"]));
        let json = serde_json::to_string(&spec).unwrap();
        assert_eq!(
            json,
            r#"{"filters":[{"table":"Stores","column":"Region","values":["East"],"exclude":true}]}"#
        );
        let back: FilterSpec = serde_json::from_str(&json).unwrap();
        assert_eq!(back, spec);
    }
}
