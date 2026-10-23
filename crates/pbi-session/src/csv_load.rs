//! "Load CSV manually…": parse a CSV into a model table's data columns
//! and recompute what depends on it.

use std::sync::Arc;
use std::time::Instant;

use arrow::array::{
    ArrayRef, BooleanBuilder, Decimal128Builder, Float64Builder, Int64Builder, StringBuilder,
    TimestampMicrosecondBuilder,
};
use arrow::datatypes::{Field, Schema};
use arrow::record_batch::{RecordBatch, RecordBatchOptions};
use encoding_rs::Encoding;
use pbi_engine::Value;
use pbi_engine::value::{DECIMAL_PRECISION, DECIMAL_SCALE};
use pbi_model::{DataType, Diagnostic, Table, names_eq};
use tokio_util::sync::CancellationToken;

use crate::error::SessionError;
use crate::filters::parse_datetime;
use crate::options::{CsvOptions, CsvSource};
use crate::refresh::mquery::{ColumnSpec, data_columns};
use crate::refresh::plan::{Node, Plan, Source};
use crate::refresh::{Run, codes, runtime};
use crate::report::RefreshReport;
use crate::session::Session;

/// Resolves an encoding label or Windows code page number.
fn encoding_for(label: &str) -> Option<&'static Encoding> {
    let label = label.trim();
    let by_code_page = match label {
        "65001" => Some(encoding_rs::UTF_8),
        "1200" => Some(encoding_rs::UTF_16LE),
        "1201" => Some(encoding_rs::UTF_16BE),
        "1252" | "28591" => Some(encoding_rs::WINDOWS_1252),
        "20127" => Some(encoding_rs::WINDOWS_1252),
        _ => None,
    };
    by_code_page.or_else(|| Encoding::for_label(label.as_bytes()))
}

/// Decodes CSV bytes to text (BOM removed).
fn decode(bytes: &[u8], label: Option<&str>) -> Result<String, String> {
    let encoding = match label {
        Some(l) => encoding_for(l).ok_or_else(|| format!("unknown encoding '{l}'"))?,
        None => match Encoding::for_bom(bytes) {
            Some((e, _)) => e,
            None if std::str::from_utf8(bytes).is_ok() => encoding_rs::UTF_8,
            None => encoding_rs::WINDOWS_1252,
        },
    };
    let (text, _, _) = encoding.decode(bytes);
    Ok(text.into_owned())
}

/// Builds one model column from CSV fields; returns the array and the
/// number of fields that could not be converted (loaded as BLANK).
fn build_column(fields: &[Option<&str>], data_type: &DataType) -> (ArrayRef, usize) {
    let mut bad = 0usize;
    macro_rules! fill {
        ($builder:expr, |$s:ident| $conv:expr) => {{
            let mut b = $builder;
            for f in fields {
                match f.map(str::trim).filter(|s| !s.is_empty()) {
                    None => b.append_null(),
                    Some($s) => match $conv {
                        Some(v) => b.append_value(v),
                        None => {
                            bad += 1;
                            b.append_null();
                        }
                    },
                }
            }
            Arc::new(b.finish()) as ArrayRef
        }};
    }
    let array = match data_type {
        DataType::Int64 => fill!(Int64Builder::new(), |s| s.parse::<i64>().ok().or_else(
            || {
                s.parse::<f64>()
                    .ok()
                    .filter(|f| f.fract() == 0.0 && f.abs() < 9.2e18)
                    .map(|f| f as i64)
            }
        )),
        DataType::Double => fill!(Float64Builder::new(), |s| s.parse::<f64>().ok()),
        DataType::Decimal => {
            let builder = Decimal128Builder::new()
                .with_precision_and_scale(DECIMAL_PRECISION, DECIMAL_SCALE)
                .unwrap_or_else(|_| Decimal128Builder::new());
            fill!(builder, |s| s
                .parse::<f64>()
                .ok()
                .and_then(Value::decimal_from_f64)
                .and_then(|v| match v {
                    Value::Decimal(d) => Some(d),
                    _ => None,
                }))
        }
        DataType::Boolean => fill!(
            BooleanBuilder::new(),
            |s| match s.to_ascii_lowercase().as_str() {
                "true" | "yes" | "1" => Some(true),
                "false" | "no" | "0" => Some(false),
                _ => None,
            }
        ),
        DataType::DateTime => fill!(TimestampMicrosecondBuilder::new(), |s| parse_datetime(s)
            .or_else(|| parse_us_datetime(s))
            .and_then(|v| v.as_datetime_micros())),
        // Text keeps fields verbatim (an empty field is "", as in M).
        _ => {
            let mut b = StringBuilder::new();
            for f in fields {
                match f {
                    Some(s) => b.append_value(s),
                    None => b.append_null(),
                }
            }
            Arc::new(b.finish()) as ArrayRef
        }
    };
    (array, bad)
}

/// `M/D/YYYY[ h:mm[:ss][ AM|PM]]`.
fn parse_us_datetime(s: &str) -> Option<Value> {
    for f in [
        "%m/%d/%Y %I:%M:%S %p",
        "%m/%d/%Y %I:%M %p",
        "%m/%d/%Y %H:%M:%S",
        "%m/%d/%Y %H:%M",
    ] {
        if let Ok(dt) = chrono::NaiveDateTime::parse_from_str(s, f) {
            return Some(Value::DateTime(dt.and_utc().timestamp_micros()));
        }
    }
    chrono::NaiveDate::parse_from_str(s, "%m/%d/%Y")
        .ok()
        .and_then(|d| d.and_hms_opt(0, 0, 0))
        .map(|dt| Value::DateTime(dt.and_utc().timestamp_micros()))
}

/// Parses CSV text into the table's data columns (model order and names).
pub(crate) fn parse_csv(
    text: &str,
    table: &Table,
    options: &CsvOptions,
    max_rows: usize,
) -> Result<(RecordBatch, Vec<Diagnostic>), String> {
    let columns: Vec<ColumnSpec> = data_columns(table);
    if columns.is_empty() {
        return Err("the table has no data columns".into());
    }
    let delimiter = u8::try_from(u32::from(options.delimiter))
        .ok()
        .filter(u8::is_ascii)
        .ok_or_else(|| {
            format!(
                "the delimiter {:?} is not an ASCII character",
                options.delimiter
            )
        })?;
    let mut reader = csv::ReaderBuilder::new()
        .delimiter(delimiter)
        .has_headers(false)
        .flexible(true)
        .from_reader(text.as_bytes());
    let mut records = reader.records();
    let mut diagnostics = Vec::new();
    let mapping: Vec<Option<usize>> = if options.has_header {
        let header = match records.next() {
            Some(r) => r.map_err(|e| format!("invalid CSV header: {e}"))?,
            None => return Err("the CSV is empty".into()),
        };
        let names: Vec<&str> = header.iter().map(str::trim).collect();
        columns
            .iter()
            .map(|c| {
                names
                    .iter()
                    .position(|h| names_eq(h, &c.source))
                    .or_else(|| names.iter().position(|h| names_eq(h, &c.name)))
            })
            .collect()
    } else {
        (0..columns.len()).map(Some).collect()
    };
    if options.has_header && mapping.iter().all(Option::is_none) {
        let expected: Vec<&str> = columns.iter().map(|c| c.name.as_str()).collect();
        return Err(format!(
            "none of the CSV columns match the table's columns ({})",
            expected.join(", ")
        ));
    }
    for (c, m) in columns.iter().zip(&mapping) {
        if m.is_none() {
            diagnostics.push(Diagnostic::warning(
                codes::CSV_MISSING_COLUMN,
                format!(
                    "'{}'[{}]: no CSV column named '{}'; loaded as blank",
                    table.name, c.name, c.source
                ),
            ));
        }
    }
    let mut rows: Vec<csv::StringRecord> = Vec::new();
    for record in records {
        let record = record.map_err(|e| format!("invalid CSV: {e}"))?;
        if record.len() == 1 && record.get(0).is_some_and(|f| f.trim().is_empty()) {
            continue;
        }
        if rows.len() >= max_rows {
            return Err(format!("the CSV has more than {max_rows} rows"));
        }
        rows.push(record);
    }
    let mut fields = Vec::with_capacity(columns.len());
    let mut arrays = Vec::with_capacity(columns.len());
    for (c, m) in columns.iter().zip(&mapping) {
        let cells: Vec<Option<&str>> = rows.iter().map(|r| m.and_then(|i| r.get(i))).collect();
        let (array, bad) = build_column(&cells, &c.data_type);
        if bad > 0 {
            diagnostics.push(Diagnostic::warning(
                codes::CSV_CONVERSION,
                format!(
                    "'{}'[{}]: {bad} value(s) could not be read as {} and were loaded as blank",
                    table.name,
                    c.name,
                    c.data_type.as_str()
                ),
            ));
        }
        fields.push(Field::new(&c.name, array.data_type().clone(), true));
        arrays.push(array);
    }
    let options = RecordBatchOptions::new().with_row_count(Some(rows.len()));
    let batch = RecordBatch::try_new_with_options(Arc::new(Schema::new(fields)), arrays, &options)
        .map_err(|e| format!("cannot build the table: {e}"))?;
    Ok((batch, diagnostics))
}

impl Session {
    /// "Load CSV manually…": replaces the data of `table` with a CSV and
    /// recomputes the calculated columns and tables that depend on it (and
    /// the session's derived columns).
    ///
    /// CSV columns are matched to the table's data columns by name
    /// (case-insensitive: the column's `sourceColumn`, then its name), or by
    /// position without a header. Missing columns load as BLANK and values
    /// that do not convert to the column type load as BLANK, each with a
    /// warning in the report's diagnostics. A later full
    /// [`refresh`](Self::refresh) evaluates the table's M partition again
    /// (keeping this data if it fails).
    ///
    /// # Errors
    /// [`SessionError::UnknownTable`]; [`SessionError::Csv`] for calculated
    /// tables, unreadable or unmatched CSVs; [`SessionError::Io`].
    pub async fn load_csv(
        &self,
        table: &str,
        source: impl Into<CsvSource>,
        options: CsvOptions,
    ) -> Result<RefreshReport, SessionError> {
        let handle = runtime()?;
        let start = Instant::now();
        let model = self.base_model();
        let index = model
            .tables
            .iter()
            .position(|t| names_eq(&t.name, table))
            .ok_or_else(|| SessionError::UnknownTable(table.to_owned()))?;
        let target = model
            .tables
            .get(index)
            .cloned()
            .ok_or_else(|| SessionError::UnknownTable(table.to_owned()))?;
        let target_name = target.name.clone();
        let csv_error = |reason: String| SessionError::Csv {
            table: target_name.clone(),
            reason,
        };
        if !self.is_pbix()
            && matches!(
                crate::refresh::plan::table_source(&target, false),
                Source::Calculated
            )
        {
            return Err(csv_error(
                "calculated tables are computed from their DAX expression".into(),
            ));
        }
        let limits = self.limits();
        let source = source.into();
        let parsed = handle
            .spawn_blocking(move || -> Result<_, SessionError> {
                let bytes = match source {
                    CsvSource::Bytes(b) => b,
                    CsvSource::Path(path) => {
                        let len = std::fs::metadata(&path)
                            .map_err(|e| SessionError::Io {
                                path: path.clone(),
                                source: e,
                            })?
                            .len();
                        if len > limits.max_csv_bytes {
                            return Ok(Err(format!(
                                "the file is larger than {} bytes",
                                limits.max_csv_bytes
                            )));
                        }
                        std::fs::read(&path).map_err(|e| SessionError::Io { path, source: e })?
                    }
                };
                if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > limits.max_csv_bytes {
                    return Ok(Err(format!(
                        "the CSV is larger than {} bytes",
                        limits.max_csv_bytes
                    )));
                }
                let text = match decode(&bytes, options.encoding.as_deref()) {
                    Ok(t) => t,
                    Err(e) => return Ok(Err(e)),
                };
                Ok(parse_csv(&text, &target, &options, limits.m.max_rows))
            })
            .await??;
        let (batch, diagnostics) = parsed.map_err(csv_error)?;
        let _guard = self.refresh_lock().lock().await;
        let plan = Plan::build(&model, self.is_pbix());
        let seeds: Vec<usize> = plan.index(Node::Table(index)).into_iter().collect();
        let mut run = Run::new(self, plan, &seeds, CancellationToken::new(), None)?;
        run.add_diagnostics(diagnostics);
        run.set_manual(index, batch, start.elapsed());
        run.calculate().await?;
        run.apply_derived(self).await?;
        Ok(run.finish(self))
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;
    use arrow::array::{Array, AsArray};
    use arrow::datatypes::{Int64Type, TimestampMicrosecondType};
    use pbi_model::Column;

    fn table() -> Table {
        let mut t = Table::new("Feeds");
        let mut key = Column::data("App Key", DataType::String);
        key.kind = pbi_model::ColumnKind::Data {
            source_column: "app_key".into(),
        };
        t.columns = vec![
            key,
            Column::data("Count", DataType::Int64),
            Column::data("When", DataType::DateTime),
            Column::calculated("Calc", DataType::Int64, "1"),
        ];
        t
    }

    #[test]
    fn maps_headers_and_converts_types() {
        let csv = "COUNT;APP_KEY;extra;when\n3;a;x;2024-01-02\nnope;b;y;1/3/2024 1:30 PM\n;;;\n";
        let options = CsvOptions {
            delimiter: ';',
            ..CsvOptions::default()
        };
        let (batch, diags) = parse_csv(csv, &table(), &options, 100).unwrap();
        assert_eq!(batch.num_rows(), 3);
        assert_eq!(batch.num_columns(), 3);
        assert_eq!(batch.schema().field(0).name(), "App Key");
        assert_eq!(batch.column(0).as_string::<i32>().value(1), "b");
        let counts = batch.column(1).as_primitive::<Int64Type>();
        assert_eq!(counts.value(0), 3);
        assert!(counts.is_null(1));
        let when = batch.column(2).as_primitive::<TimestampMicrosecondType>();
        assert_eq!(
            Value::DateTime(when.value(1)),
            Value::datetime(2024, 1, 3, 13, 30, 0).unwrap()
        );
        assert_eq!(diags.len(), 1, "{diags:?}");
        assert!(diags[0].message.contains("1 value(s)"));
    }

    #[test]
    fn positional_mapping_without_header_and_errors() {
        let options = CsvOptions {
            has_header: false,
            ..CsvOptions::default()
        };
        let (batch, _) = parse_csv("k,1\n", &table(), &options, 100).unwrap();
        assert_eq!(batch.num_rows(), 1);
        assert!(batch.column(2).is_null(0));
        let err = parse_csv("x,y\n1,2\n", &table(), &CsvOptions::default(), 100).unwrap_err();
        assert!(err.contains("none of the CSV columns"), "{err}");
        let err = parse_csv("count\n1\n2\n", &table(), &CsvOptions::default(), 1).unwrap_err();
        assert!(err.contains("more than 1 rows"), "{err}");
    }

    #[test]
    fn decodes_boms_and_code_pages() {
        let utf16: Vec<u8> = [0xFF, 0xFE]
            .into_iter()
            .chain("a,é".encode_utf16().flat_map(u16::to_le_bytes))
            .collect();
        assert_eq!(decode(&utf16, None).unwrap(), "a,é");
        assert_eq!(decode(&[0x61, 0xE9], None).unwrap(), "aé");
        assert_eq!(decode("é".as_bytes(), Some("65001")).unwrap(), "é");
        assert!(decode(b"x", Some("klingon")).is_err());
    }
}
