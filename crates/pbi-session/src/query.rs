//! Query API: measures (with formatting), grid pages, column statistics,
//! SQL and DAX queries.

use std::sync::Arc;

use arrow::record_batch::RecordBatch;
use pbi_dax::{DaxError, EvalContext};
use pbi_engine::{ColumnStats, FilterContext, PageRequest, Value};
use pbi_model::{Measure, Model, dax_column_ref, names_eq};
use serde::{Serialize, Serializer};
use serde_json::Value as Json;
use tokio_util::sync::CancellationToken;

use crate::error::SessionError;
use crate::filters::FilterSpec;
use crate::refresh::runtime;
use crate::session::Session;

/// The value of one measure in a filter context. Evaluation errors are
/// reported here (never as a panic): the UI shows "⚠ + `error` + `dax`".
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MeasureValue {
    /// Home table of the measure.
    pub table: String,
    /// Measure name.
    pub name: String,
    /// The value (`Some(Value::Blank)` for BLANK); `None` on error.
    #[serde(serialize_with = "serialize_value")]
    pub value: Option<Value>,
    /// `value` formatted with `format_string` (BLANK → `""`).
    pub formatted: Option<String>,
    /// The effective format string: a calculation item's or the measure's
    /// dynamic format string (`formatStringDefinition`) in this filter
    /// context, else the measure's `formatString`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub format_string: Option<String>,
    /// Error message when the measure could not be evaluated.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// The error is an unsupported DAX feature (not a model/data problem).
    pub unsupported: bool,
    /// The measure's DAX expression.
    pub dax: String,
    /// Hidden measure.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub hidden: bool,
    /// Display folder.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_folder: Option<String>,
}

impl MeasureValue {
    /// `true` if the measure failed.
    #[must_use]
    pub fn is_error(&self) -> bool {
        self.error.is_some()
    }

    /// Display text: the formatted value, or `⚠ <error>`.
    #[must_use]
    pub fn display(&self) -> String {
        match (&self.formatted, &self.error) {
            (_, Some(e)) => format!("⚠ {e}"),
            (Some(f), None) => f.clone(),
            (None, None) => String::new(),
        }
    }
}

fn serialize_value<S: Serializer>(value: &Option<Value>, s: S) -> Result<S::Ok, S::Error> {
    match value {
        Some(v) => value_to_json(v).serialize(s),
        None => s.serialize_none(),
    }
}

/// JSON form of an engine value: BLANK → `null`, numbers as numbers
/// (non-finite doubles as `"NaN"`, `"Infinity"`, `"-Infinity"`), dates as
/// ISO text `YYYY-MM-DDThh:mm:ss[.ffffff]`.
#[must_use]
pub fn value_to_json(value: &Value) -> Json {
    match value {
        Value::Blank => Json::Null,
        Value::Boolean(b) => Json::Bool(*b),
        Value::Integer(i) => Json::from(*i),
        Value::Double(_) | Value::Decimal(_) => match value.as_f64() {
            Some(f) if f.is_finite() => {
                serde_json::Number::from_f64(f).map_or(Json::Null, Json::Number)
            }
            Some(f) if f.is_nan() => Json::String("NaN".into()),
            Some(f) if f > 0.0 => Json::String("Infinity".into()),
            Some(_) => Json::String("-Infinity".into()),
            None => Json::Null,
        },
        Value::String(s) => Json::String(s.to_string()),
        Value::DateTime(_) => Json::String(value.to_string().replacen(' ', "T", 1)),
    }
}

/// Evaluates one measure; errors become the entry's `error`.
fn measure_value(
    ctx: &EvalContext,
    table: &str,
    measure: &Measure,
    culture: &str,
) -> Result<MeasureValue, SessionError> {
    let mut out = MeasureValue {
        table: table.to_owned(),
        name: measure.name.clone(),
        value: None,
        formatted: None,
        format_string: measure.format_string.clone(),
        error: None,
        unsupported: false,
        dax: measure.expression.clone(),
        hidden: measure.is_hidden,
        display_folder: measure.display_folder.clone(),
    };
    match ctx.evaluate_measure_formatted(&dax_column_ref(table, &measure.name)) {
        Ok((v, format)) => {
            out.formatted = Some(pbi_dax::format_value(&v, format.as_deref(), culture));
            out.format_string = format;
            out.value = Some(v);
        }
        Err(DaxError::Cancelled) => return Err(SessionError::Cancelled),
        Err(e) => {
            out.unsupported = e.is_unsupported();
            out.error = Some(e.to_string());
        }
    }
    Ok(out)
}

impl Session {
    fn eval_context(
        &self,
        filter: &FilterSpec,
        cancel: &CancellationToken,
    ) -> Result<(EvalContext, String, Arc<Model>, u64), SessionError> {
        let (snapshot, model, revision) = self.snapshot_model_and_revision();
        let ctx: FilterContext = filter.to_filter_context(&model)?;
        let culture = model
            .culture
            .as_deref()
            .filter(|culture| !culture.trim().is_empty())
            .unwrap_or("en-US")
            .to_owned();
        Ok((
            EvalContext::new(snapshot, Arc::clone(&model))
                .with_filter(ctx)
                .with_cancellation_token(cancel.clone()),
            culture,
            model,
            revision,
        ))
    }

    /// Evaluates every model measure (model order) in `filter`. A measure
    /// that fails is returned with `error` set (and `unsupported` for DAX
    /// features DashCraft does not implement); it never aborts the call.
    ///
    /// # Errors
    /// An invalid `filter` ([`FilterSpec::to_filter_context`]),
    /// [`SessionError::Cancelled`].
    pub async fn measures(
        &self,
        filter: &FilterSpec,
        cancel: CancellationToken,
    ) -> Result<Vec<MeasureValue>, SessionError> {
        let (_, measures) = self.measures_with_revision(filter, cancel).await?;
        Ok(measures)
    }

    /// Like [`measures`](Self::measures), returning the semantic revision
    /// captured with the model and engine snapshot.
    pub async fn measures_with_revision(
        &self,
        filter: &FilterSpec,
        cancel: CancellationToken,
    ) -> Result<(u64, Vec<MeasureValue>), SessionError> {
        let handle = runtime()?;
        let cancel = cancel.child_token();
        let _stop_on_drop = cancel.clone().drop_guard();
        let (ctx, culture, model, revision) = self.eval_context(filter, &cancel)?;
        let measures = handle
            .spawn_blocking(move || {
                let mut out = Vec::new();
                for t in &model.tables {
                    for m in &t.measures {
                        if cancel.is_cancelled() {
                            return Err(SessionError::Cancelled);
                        }
                        out.push(measure_value(&ctx, &t.name, m, &culture)?);
                    }
                }
                Ok(out)
            })
            .await??;
        Ok((revision, measures))
    }

    /// Evaluates one measure (`Name`, `[Name]` or `Table[Name]`) in `filter`.
    ///
    /// # Errors
    /// [`SessionError::UnknownMeasure`], an invalid `filter`,
    /// [`SessionError::Cancelled`]. Evaluation errors are in the result.
    pub async fn evaluate_measure(
        &self,
        name: &str,
        filter: &FilterSpec,
        cancel: CancellationToken,
    ) -> Result<MeasureValue, SessionError> {
        let (_, value) = self
            .evaluate_measure_with_revision(name, filter, cancel)
            .await?;
        Ok(value)
    }

    /// Like [`evaluate_measure`](Self::evaluate_measure), returning the
    /// semantic revision captured with the model and engine snapshot.
    pub async fn evaluate_measure_with_revision(
        &self,
        name: &str,
        filter: &FilterSpec,
        cancel: CancellationToken,
    ) -> Result<(u64, MeasureValue), SessionError> {
        let handle = runtime()?;
        let cancel = cancel.child_token();
        let _stop_on_drop = cancel.clone().drop_guard();
        let (ctx, culture, model, revision) = self.eval_context(filter, &cancel)?;
        let wanted = name.trim();
        let (table_part, measure_part) = match wanted.find('[') {
            Some(i) if wanted.ends_with(']') => (
                Some(
                    wanted
                        .get(..i)
                        .unwrap_or_default()
                        .trim()
                        .trim_matches('\'')
                        .replace("''", "'"),
                ),
                wanted
                    .get(i + 1..wanted.len() - 1)
                    .unwrap_or_default()
                    .replace("]]", "]"),
            ),
            _ => (None, wanted.to_owned()),
        };
        let found = model.tables.iter().find_map(|t| {
            let table_ok = table_part
                .as_deref()
                .is_none_or(|tp| tp.is_empty() || names_eq(tp, &t.name));
            t.measures
                .iter()
                .find(|m| table_ok && names_eq(&m.name, &measure_part))
                .map(|m| (t.name.clone(), m.clone()))
        });
        let Some((table, measure)) = found else {
            return Err(SessionError::UnknownMeasure(name.to_owned()));
        };
        let value = handle
            .spawn_blocking(move || measure_value(&ctx, &table, &measure, &culture))
            .await??;
        Ok((revision, value))
    }

    /// Evaluates a DAX query (`EVALUATE …`, with `DEFINE` / `ORDER BY`) with
    /// `filter` as the outer filter context; one batch per `EVALUATE`.
    ///
    /// # Errors
    /// DAX syntax/evaluation errors ([`SessionError::Dax`]), an invalid
    /// `filter`, [`SessionError::Cancelled`].
    pub async fn dax_query(
        &self,
        query: &str,
        filter: &FilterSpec,
        cancel: CancellationToken,
    ) -> Result<Vec<RecordBatch>, SessionError> {
        let (_, _, tables) = self.dax_query_with_revision(query, filter, cancel).await?;
        Ok(tables)
    }

    /// Like [`dax_query`](Self::dax_query), returning the semantic revision
    /// and model captured with the engine snapshot.
    pub async fn dax_query_with_revision(
        &self,
        query: &str,
        filter: &FilterSpec,
        cancel: CancellationToken,
    ) -> Result<(u64, Arc<Model>, Vec<RecordBatch>), SessionError> {
        let handle = runtime()?;
        let cancel = cancel.child_token();
        let _stop_on_drop = cancel.clone().drop_guard();
        let (ctx, _, model, revision) = self.eval_context(filter, &cancel)?;
        let query = query.to_owned();
        let tables = handle
            .spawn_blocking(move || {
                let tables = ctx.evaluate_query(&query)?;
                Ok::<_, SessionError>(tables.iter().map(pbi_dax::to_record_batch).collect())
            })
            .await??;
        Ok((revision, model, tables))
    }

    /// [`dax_query`](Self::dax_query) returning the typed result tables
    /// (DAX values and `Table[Column]` / `[Name]` column names) instead of
    /// Arrow batches; used where exact DAX types matter (ground-truth
    /// comparison against Power BI Desktop).
    ///
    /// # Errors
    /// As [`dax_query`](Self::dax_query).
    pub async fn dax_query_tables(
        &self,
        query: &str,
        filter: &FilterSpec,
        cancel: CancellationToken,
    ) -> Result<Vec<pbi_dax::VirtualTable>, SessionError> {
        let handle = runtime()?;
        let cancel = cancel.child_token();
        let _stop_on_drop = cancel.clone().drop_guard();
        let (ctx, _, _, _) = self.eval_context(filter, &cancel)?;
        let query = query.to_owned();
        handle
            .spawn_blocking(move || Ok(ctx.evaluate_query(&query)?))
            .await?
    }

    /// One page of a table for the grid (sorted / filtered with SQL
    /// expressions; see [`PageRequest`]).
    ///
    /// # Errors
    /// Unknown table or column, invalid filter expression,
    /// [`SessionError::Cancelled`].
    pub async fn page(
        &self,
        request: &PageRequest,
        cancel: CancellationToken,
    ) -> Result<RecordBatch, SessionError> {
        runtime()?;
        let snapshot = self.snapshot();
        Ok(snapshot.page(request, cancel).await?)
    }

    /// Number of rows of `table` passing the optional SQL `filter` (the
    /// grid's total).
    ///
    /// # Errors
    /// As [`page`](Self::page).
    pub async fn row_count(
        &self,
        table: &str,
        filter: Option<&str>,
        cancel: CancellationToken,
    ) -> Result<usize, SessionError> {
        runtime()?;
        let snapshot = self.snapshot();
        Ok(snapshot.page_row_count(table, filter, cancel).await?)
    }

    /// Column statistics (rows, blanks, distinct count, min, max).
    ///
    /// # Errors
    /// Unknown table or column.
    pub async fn column_stats(
        &self,
        table: &str,
        column: &str,
    ) -> Result<ColumnStats, SessionError> {
        let handle = runtime()?;
        let snapshot = self.snapshot();
        let (table, column) = (table.to_owned(), column.to_owned());
        Ok(handle
            .spawn_blocking(move || snapshot.column_stats(&table, &column))
            .await??)
    }

    /// Runs a read-only SQL query over the model tables (DataFusion; quote
    /// names with spaces: `SELECT "Store Name" FROM "Stores"`).
    ///
    /// # Errors
    /// SQL errors ([`SessionError::Engine`]), [`SessionError::Cancelled`].
    pub async fn sql(
        &self,
        query: &str,
        cancel: CancellationToken,
    ) -> Result<Vec<RecordBatch>, SessionError> {
        runtime()?;
        let snapshot = self.snapshot();
        Ok(snapshot.sql(query, cancel).await?)
    }

    /// Formats a value with a format string in the model culture (see
    /// [`pbi_dax::format`]).
    #[must_use]
    pub fn format_value(&self, value: &Value, format_string: Option<&str>) -> String {
        pbi_dax::format_value(value, format_string, &self.culture())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values_serialize_to_json() {
        assert_eq!(value_to_json(&Value::Blank), Json::Null);
        assert_eq!(
            value_to_json(&Value::Integer(2024)),
            serde_json::json!(2024)
        );
        assert_eq!(value_to_json(&Value::Double(0.5)), serde_json::json!(0.5));
        assert_eq!(
            value_to_json(&Value::Decimal(12_500)),
            serde_json::json!(1.25)
        );
        assert_eq!(
            value_to_json(&Value::Double(f64::INFINITY)),
            serde_json::json!("Infinity")
        );
        assert_eq!(
            value_to_json(&Value::datetime(2026, 10, 21, 14, 5, 0).unwrap_or_default()),
            serde_json::json!("2026-10-21T14:05:00")
        );
    }

    #[test]
    fn error_entries_display_a_warning() {
        let v = MeasureValue {
            table: "T".into(),
            name: "M".into(),
            value: None,
            formatted: None,
            format_string: None,
            error: Some("The DAX function RANKX is not supported yet.".into()),
            unsupported: true,
            dax: "RANKX(T, [x])".into(),
            hidden: false,
            display_folder: None,
        };
        assert!(v.is_error());
        assert_eq!(
            v.display(),
            "⚠ The DAX function RANKX is not supported yet."
        );
        let json = serde_json::to_value(&v).unwrap_or_default();
        assert_eq!(json["value"], Json::Null);
        assert_eq!(json["unsupported"], Json::Bool(true));
    }
}
