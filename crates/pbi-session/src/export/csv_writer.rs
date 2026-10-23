//! CSV export ([`write_csv`]).

use std::io::Write;

use arrow::array::ArrayRef;
use arrow::datatypes::DataType as ArrowType;
use arrow::record_batch::RecordBatch;
use arrow::util::display::{ArrayFormatter, FormatOptions};
use pbi_engine::value::{canonical_type, to_canonical, value_from_array};
use serde::{Deserialize, Serialize};

use super::{ExportError, ExportSummary, meta};

/// UTF-8 byte order mark.
const BOM: &[u8] = b"\xEF\xBB\xBF";

/// Options of [`write_csv`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct CsvExportOptions {
    /// Field delimiter: one ASCII character other than `"`, CR or LF
    /// (default `,`).
    pub delimiter: char,
    /// Write a header record with the column names (default `true`).
    pub header: bool,
    /// Format every value with its column's model format string
    /// ([`meta::FORMAT_STRING`], `General` without one), as Power BI shows
    /// it. Default `false`: raw, machine-readable values.
    pub formatted: bool,
    /// Start with a UTF-8 byte order mark, so Excel detects the encoding
    /// (default `false`).
    pub bom: bool,
    /// Spreadsheet formula-injection guard (OWASP "CSV injection"): text
    /// cells and header names that start with `=`, `+`, `-`, `@`, TAB or CR
    /// get a leading `'`, so Excel, LibreOffice or Google Sheets show them
    /// as text instead of running them as formulas. Numeric, date and
    /// boolean columns are never changed. Default `false` (the data stays
    /// exactly as stored, for machine consumers).
    pub escape_formulas: bool,
}

impl Default for CsvExportOptions {
    fn default() -> Self {
        Self {
            delimiter: ',',
            header: true,
            formatted: false,
            bom: false,
            escape_formulas: false,
        }
    }
}

/// `true` if a spreadsheet would read `text` as a formula
/// ([`CsvExportOptions::escape_formulas`]).
fn looks_like_formula(text: &str) -> bool {
    text.starts_with(['=', '+', '-', '@', '\t', '\r'])
}

/// `text` with a leading `'` when it [`looks_like_formula`].
fn escape_formula(text: &mut String) {
    if looks_like_formula(text) {
        text.insert(0, '\'');
    }
}

/// Text columns (the only ones [`CsvExportOptions::escape_formulas`]
/// changes).
fn is_text(t: &ArrowType) -> bool {
    match t {
        ArrowType::Utf8 | ArrowType::LargeUtf8 | ArrowType::Utf8View => true,
        ArrowType::Dictionary(_, values) => is_text(values),
        _ => false,
    }
}

/// How the cells of one column are written.
enum Cells<'a> {
    /// Arrow's display of the raw value (BLANK → empty).
    Raw(ArrayFormatter<'a>),
    /// [`pbi_dax::format_value`] of the DAX value.
    Formatted {
        array: ArrayRef,
        format: Option<&'a str>,
        culture: &'a str,
    },
}

/// Writes `batch` as CSV: an optional BOM, a header record with the field
/// names, then one record per row. Fields are quoted only when needed
/// (they contain the delimiter, `"`, CR or LF; `"` is doubled); BLANK
/// (null) is an empty field; records end with `\n`.
///
/// Raw values use Arrow's display (`0.5`, `1.2500` for fixed decimals,
/// `2026-10-21T14:05:00`, `true`). With [`CsvExportOptions::formatted`],
/// values are formatted with the field's [`meta::FORMAT_STRING`] in the
/// schema's [`meta::CULTURE`] (see [`annotate_batch`](super::annotate_batch)).
/// With [`CsvExportOptions::escape_formulas`], text cells and header names
/// that a spreadsheet would run as a formula get a leading `'`.
///
/// # Errors
/// An invalid delimiter ([`ExportError::InvalidOption`]), a column that
/// cannot be displayed, or a write failure ([`ExportError::Io`]).
pub fn write_csv<W: Write>(
    batch: &RecordBatch,
    mut writer: W,
    options: &CsvExportOptions,
) -> Result<ExportSummary, ExportError> {
    let delimiter = delimiter_byte(options.delimiter)?;
    let schema = batch.schema();
    let culture = schema
        .metadata()
        .get(meta::CULTURE)
        .map_or("en-US", String::as_str);
    let raw_options = FormatOptions::new().with_display_error(false);
    let columns = batch
        .columns()
        .iter()
        .zip(schema.fields())
        .map(|(array, field)| {
            let formattable = canonical_type(array.data_type()).is_some()
                && !matches!(array.data_type(), ArrowType::Null);
            let column_error = |e: &dyn std::fmt::Display| ExportError::Column {
                column: field.name().clone(),
                reason: e.to_string(),
            };
            if options.formatted && formattable {
                Ok(Cells::Formatted {
                    array: to_canonical(array).map_err(|e| column_error(&e))?,
                    format: field
                        .metadata()
                        .get(meta::FORMAT_STRING)
                        .map(String::as_str),
                    culture,
                })
            } else {
                ArrayFormatter::try_new(array.as_ref(), &raw_options)
                    .map(Cells::Raw)
                    .map_err(|e| column_error(&e))
            }
        })
        .collect::<Result<Vec<_>, _>>()?;

    if options.bom {
        writer.write_all(BOM)?;
    }
    let summary = ExportSummary {
        rows: batch.num_rows(),
        columns: columns.len(),
    };
    if columns.is_empty() {
        writer.flush()?;
        return Ok(summary);
    }
    let mut out = csv::WriterBuilder::new()
        .delimiter(delimiter)
        .has_headers(false)
        .quote_style(csv::QuoteStyle::Necessary)
        .from_writer(writer);
    if options.header {
        let header: Vec<String> = schema
            .fields()
            .iter()
            .map(|f| {
                let mut name = f.name().clone();
                if options.escape_formulas {
                    escape_formula(&mut name);
                }
                name
            })
            .collect();
        out.write_record(&header)?;
    }
    let escape: Vec<bool> = batch
        .columns()
        .iter()
        .map(|a| options.escape_formulas && is_text(a.data_type()))
        .collect();
    let mut record = vec![String::new(); columns.len()];
    for row in 0..batch.num_rows() {
        for (((cells, cell), field), escape) in columns
            .iter()
            .zip(&mut record)
            .zip(schema.fields())
            .zip(&escape)
        {
            cell.clear();
            write_cell(cells, row, cell).map_err(|reason| ExportError::Column {
                column: field.name().clone(),
                reason,
            })?;
            if *escape {
                escape_formula(cell);
            }
        }
        out.write_record(&record)?;
    }
    out.flush()?;
    Ok(summary)
}

fn write_cell(cells: &Cells<'_>, row: usize, out: &mut String) -> Result<(), String> {
    match cells {
        Cells::Raw(formatter) => formatter.value(row).write(out).map_err(|e| e.to_string()),
        Cells::Formatted {
            array,
            format,
            culture,
        } => {
            let value = value_from_array(array, row).map_err(|e| e.to_string())?;
            out.push_str(&pbi_dax::format_value(&value, *format, culture));
            Ok(())
        }
    }
}

fn delimiter_byte(delimiter: char) -> Result<u8, ExportError> {
    match u8::try_from(delimiter) {
        Ok(b) if b.is_ascii() && !matches!(b, b'"' | b'\r' | b'\n') => Ok(b),
        _ => Err(ExportError::InvalidOption(format!(
            "the CSV delimiter must be one ASCII character other than '\"', CR or LF (got {delimiter:?})"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formula_prefixes_are_detected() {
        for text in ["=1+1", "+1", "-2", "@SUM(A1)", "\tx", "\rx"] {
            assert!(looks_like_formula(text), "{text:?}");
            let mut s = text.to_owned();
            escape_formula(&mut s);
            assert_eq!(s, format!("'{text}"));
        }
        for text in ["", "a=1", " =1", "'=1", "1-2"] {
            assert!(!looks_like_formula(text), "{text:?}");
        }
    }
}
