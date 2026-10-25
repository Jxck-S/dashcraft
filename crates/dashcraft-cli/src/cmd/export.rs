//! `dashcraft export`: a table's data as CSV or Parquet (after a refresh), or
//! the model documentation as JSON (model doc, `schemaVersion` 1) or a
//! Markdown data dictionary.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, anyhow, bail};
use pbi_model::Model;
use pbi_session::{
    CsvExportOptions, ExportSummary, MarkdownOptions, ModelDocOptions, ParquetCompression,
    ParquetExportOptions, Session, TableStatus, model_doc, model_to_markdown,
};

use crate::common::{Context, Status, open_document, with_stdout, write_file};
use crate::data::{
    DataArgs, cli_hint, open_session, print_problems, refresh, require_model, resolve_table,
    runtime,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum Format {
    /// CSV (with --table).
    Csv,
    /// Apache Parquet (with --table).
    Parquet,
    /// Model doc JSON, `schemaVersion` 1 (with --model).
    Json,
    /// Markdown data dictionary (with --model).
    #[value(alias = "markdown")]
    Md,
}

impl Format {
    fn name(self) -> &'static str {
        match self {
            Format::Csv => "csv",
            Format::Parquet => "parquet",
            Format::Json => "json",
            Format::Md => "md",
        }
    }

    /// From an output file extension.
    fn from_path(path: &Path) -> Option<Self> {
        let ext = path.extension()?.to_str()?.to_ascii_lowercase();
        Some(match ext.as_str() {
            "csv" => Format::Csv,
            "parquet" | "pq" => Format::Parquet,
            "json" => Format::Json,
            "md" | "markdown" => Format::Md,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum Compression {
    /// Zstandard (default).
    Zstd,
    /// Snappy.
    Snappy,
    /// No compression.
    None,
}

#[derive(Debug, clap::Args)]
pub struct ExportArgs {
    /// File or folder: .pbip, .pbix, .pbit, *.SemanticModel, model.bim, …
    pub path: PathBuf,
    /// Export this table's data (the document is refreshed first).
    #[arg(
        long,
        value_name = "NAME",
        required_unless_present = "model",
        conflicts_with = "model"
    )]
    pub table: Option<String>,
    /// Export the model documentation (tables, columns, measures with DAX,
    /// relationships, M partitions, parameters, roles); no refresh needed.
    #[arg(long)]
    pub model: bool,
    /// Output format (default: from the --out extension). csv/parquet with
    /// --table, json/md with --model.
    #[arg(long, value_enum)]
    pub format: Option<Format>,
    /// Output file; `-` writes to stdout (csv, json, md).
    #[arg(long, short, value_name = "FILE")]
    pub out: PathBuf,
    /// Overwrite an existing output file.
    #[arg(long)]
    pub force: bool,
    /// CSV: format values with the model's format strings (as Power BI
    /// shows them) instead of raw values.
    #[arg(long)]
    pub formatted: bool,
    /// CSV: start with a UTF-8 byte order mark (for Excel).
    #[arg(long)]
    pub bom: bool,
    /// CSV: field delimiter, one ASCII character (default `,`).
    #[arg(long, value_name = "CHAR")]
    pub delimiter: Option<char>,
    /// CSV: guard against spreadsheet formula injection: text cells (and
    /// header names) starting with `=`, `+`, `-`, `@`, TAB or CR get a
    /// leading `'`. Numbers, dates and booleans are unchanged.
    #[arg(long)]
    pub excel_safe: bool,
    /// Parquet: compression codec (default zstd).
    #[arg(long, value_enum)]
    pub compression: Option<Compression>,
    /// --model: leave out DAX, M and query text (e.g. for sharing).
    #[arg(long)]
    pub no_expressions: bool,
    /// --model: leave out hidden tables, columns, measures and hierarchies.
    #[arg(long)]
    pub no_hidden: bool,
    /// --model --format md: document title (default: the file or project
    /// name).
    #[arg(long, value_name = "TEXT")]
    pub title: Option<String>,
    #[command(flatten)]
    pub data: DataArgs,
}

/// Where the export goes.
enum Target {
    Stdout,
    File {
        path: PathBuf,
        /// Replace the file if it exists.
        overwrite: bool,
    },
}

pub fn run(args: &ExportArgs, ctx: &Context) -> anyhow::Result<Status> {
    let format = resolve_format(args)?;
    let target = target(&args.out, args.force, format)?;
    match &args.table {
        Some(table) => export_table(args, table, format, &target, ctx),
        None => export_model(args, format, &target, ctx),
    }
}

fn resolve_format(args: &ExportArgs) -> anyhow::Result<Format> {
    for (set, flag) in [
        (args.no_expressions, "--no-expressions"),
        (args.no_hidden, "--no-hidden"),
    ] {
        if set && args.table.is_some() {
            bail!("{flag} only applies to --model");
        }
    }
    let format = match args.format {
        Some(f) => f,
        None => Format::from_path(&args.out).ok_or_else(|| {
            anyhow!(
                "cannot tell the format from '{}'; use --format csv|parquet (--table) or json|md (--model)",
                args.out.display()
            )
        })?,
    };
    match (args.table.is_some(), format) {
        (true, Format::Json | Format::Md) => bail!(
            "--table exports csv or parquet, not {}; use --model for the model documentation",
            format.name()
        ),
        (false, Format::Csv | Format::Parquet) => bail!(
            "--model exports json or md, not {}; use --table NAME for table data",
            format.name()
        ),
        _ => {}
    }
    let only = |set: bool, flag: &str, wanted: Format| {
        if set && format != wanted {
            Err(anyhow!("{flag} only applies to --format {}", wanted.name()))
        } else {
            Ok(())
        }
    };
    only(args.formatted, "--formatted", Format::Csv)?;
    only(args.bom, "--bom", Format::Csv)?;
    only(args.delimiter.is_some(), "--delimiter", Format::Csv)?;
    only(args.excel_safe, "--excel-safe", Format::Csv)?;
    only(args.compression.is_some(), "--compression", Format::Parquet)?;
    only(args.title.is_some(), "--title", Format::Md)?;
    Ok(format)
}

/// Checks the output before any work: `-` (not for Parquet), or a file
/// that does not exist yet (or is empty) unless `force`.
fn target(out: &Path, force: bool, format: Format) -> anyhow::Result<Target> {
    if out.as_os_str() == "-" {
        if format == Format::Parquet {
            bail!("Parquet is a binary format; write it to a file with --out FILE");
        }
        return Ok(Target::Stdout);
    }
    let overwrite = match fs::metadata(out) {
        Ok(meta) if meta.is_dir() => bail!("{} is a folder; --out takes a file", out.display()),
        Ok(meta) if meta.len() > 0 && !force => bail!(
            "{} already exists; use --force to overwrite it",
            out.display()
        ),
        Ok(_) => true,
        Err(_) => force,
    };
    Ok(Target::File {
        path: out.to_path_buf(),
        overwrite,
    })
}

fn export_table(
    args: &ExportArgs,
    table: &str,
    format: Format,
    target: &Target,
    ctx: &Context,
) -> anyhow::Result<Status> {
    let session = open_session(&args.path, &args.data)?;
    require_model(&session, &args.path)?;
    let name = resolve_table(&session.model(), table)?;
    let rt = runtime()?;
    let report = refresh(&rt, &session, None, &args.data, ctx)?;
    let failed = print_problems(ctx, &report);
    if let Some(TableStatus::Error { message, hint, .. }) = report.table(&name).map(|t| &t.status) {
        let hint = hint
            .as_deref()
            .map(|h| format!(" (hint: {})", cli_hint(h)))
            .unwrap_or_default();
        bail!("cannot export '{name}': it failed to refresh: {message}{hint}");
    }
    let summary = match format {
        Format::Csv => {
            let options = CsvExportOptions {
                delimiter: args.delimiter.unwrap_or(','),
                header: true,
                formatted: args.formatted,
                bom: args.bom,
                escape_formulas: args.excel_safe,
            };
            write_csv(&session, &name, target, &options)?
        }
        Format::Parquet => {
            let compression = match args.compression.unwrap_or(Compression::Zstd) {
                Compression::Zstd => ParquetCompression::Zstd,
                Compression::Snappy => ParquetCompression::Snappy,
                Compression::None => ParquetCompression::None,
            };
            let options = ParquetExportOptions { compression };
            let Target::File { path, overwrite } = target else {
                bail!("Parquet is a binary format; write it to a file with --out FILE");
            };
            write_file(path, *overwrite, |w| {
                session
                    .export_parquet(&name, w, &options)
                    .map_err(|e| anyhow!(e))
            })?
        }
        Format::Json | Format::Md => bail!("internal error: not a table format"),
    };
    if let Target::File { path, .. } = target {
        ctx.note(format!(
            "wrote {} row{} × {} column{} of '{name}' to {}",
            summary.rows,
            if summary.rows == 1 { "" } else { "s" },
            summary.columns,
            if summary.columns == 1 { "" } else { "s" },
            path.display()
        ));
    }
    Ok(if failed {
        Status::ErrorDiagnostics
    } else {
        Status::Ok
    })
}

fn write_csv(
    session: &Session,
    table: &str,
    target: &Target,
    options: &CsvExportOptions,
) -> anyhow::Result<ExportSummary> {
    match target {
        Target::Stdout => {
            let mut summary = None;
            with_stdout(|w| {
                summary = Some(session.export_csv(table, w, options)?);
                Ok(())
            })?;
            // A closed pipe (`| head`) stops the export early: not an error.
            Ok(summary.unwrap_or(ExportSummary {
                rows: 0,
                columns: 0,
            }))
        }
        Target::File { path, overwrite } => write_file(path, *overwrite, |w| {
            session
                .export_csv(table, w, options)
                .map_err(|e| anyhow!(e))
        }),
    }
}

fn export_model(
    args: &ExportArgs,
    format: Format,
    target: &Target,
    ctx: &Context,
) -> anyhow::Result<Status> {
    if args.data.blob_map.is_some() {
        ctx.warn("--blob-map is ignored with --model (no data is loaded)");
    }
    if args.data.source.is_some() {
        ctx.warn("--source is ignored with --model (no data is loaded)");
    }
    let opened = open_document(&args.path, &args.data.read.open_options())?;
    let Some(model) = &opened.document.model else {
        bail!(
            "{} has no local semantic model (a report bound to a remote model?)",
            args.path.display()
        );
    };
    let (include_expressions, include_hidden) = (!args.no_expressions, !args.no_hidden);
    let doc = model_doc(
        model,
        &ModelDocOptions {
            include_expressions,
            include_hidden,
        },
    );
    let text = match format {
        Format::Json => {
            let mut text =
                serde_json::to_string_pretty(&doc).context("cannot serialize the model doc")?;
            text.push('\n');
            text
        }
        Format::Md => model_to_markdown(
            model,
            &MarkdownOptions {
                include_expressions,
                include_hidden,
                title: Some(
                    args.title
                        .clone()
                        .unwrap_or_else(|| default_title(&args.path, model)),
                ),
            },
        ),
        Format::Csv | Format::Parquet => bail!("internal error: not a model format"),
    };
    match target {
        Target::Stdout => with_stdout(|w| w.write_all(text.as_bytes()))?,
        Target::File { path, overwrite } => {
            write_file(path, *overwrite, |w| {
                w.write_all(text.as_bytes())
                    .context("cannot write the model documentation")
            })?;
            ctx.note(format!(
                "wrote the model documentation ({} tables, {} columns, {} measures) to {}",
                doc.summary.tables,
                doc.summary.columns,
                doc.summary.measures,
                path.display()
            ));
        }
    }
    Ok(Status::of(&opened.document))
}

/// The file or project name (`AcmeOutfitters` for `AcmeOutfitters.pbip` or
/// `AcmeOutfitters.SemanticModel`), else the model name.
fn default_title(path: &Path, model: &Model) -> String {
    path.file_stem()
        .and_then(|s| s.to_str())
        .map(str::trim)
        .filter(|s| {
            !s.is_empty()
                && !s.starts_with('.')
                && !["model", "definition", "datamodelschema"]
                    .iter()
                    .any(|generic| s.eq_ignore_ascii_case(generic))
        })
        .map_or_else(|| model.name.clone(), str::to_owned)
}
