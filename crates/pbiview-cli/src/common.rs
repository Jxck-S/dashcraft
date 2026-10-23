//! Helpers shared by the subcommands: global flags, exit status, opening
//! inputs, writing to stdout and text shortening.

use std::fs::{self, File};
use std::io::{self, BufWriter, Write};
use std::path::Path;
use std::process::ExitCode;
use std::sync::OnceLock;

use anyhow::{Context as _, anyhow};
use pbi_core::{CoreError, OpenOptions, OpenedDocument, open_path};
use pbi_model::{Document, Severity};
use serde::Serialize;

/// The program's name: `pbiview` when invoked through the alias binary
/// (the former product name), else `dashcraft`. Used in usage, help,
/// `--version` and every message on stderr.
pub fn prog() -> &'static str {
    static NAME: OnceLock<&'static str> = OnceLock::new();
    NAME.get_or_init(|| prog_from_arg0(std::env::args_os().next().as_deref()))
}

fn prog_from_arg0(arg0: Option<&std::ffi::OsStr>) -> &'static str {
    let stem = arg0
        .map(Path::new)
        .and_then(Path::file_stem)
        .map(|s| s.to_string_lossy().to_ascii_lowercase());
    if stem.as_deref() == Some("pbiview") {
        "pbiview"
    } else {
        "dashcraft"
    }
}

/// Global flags, available to every subcommand.
#[derive(Debug, Clone, Copy, Default)]
pub struct Context {
    /// `-v` count.
    pub verbose: u8,
    /// `--quiet`: only errors on stderr.
    pub quiet: bool,
}

impl Context {
    /// Prints a progress/info note on stderr unless `--quiet`.
    pub fn note(&self, message: impl std::fmt::Display) {
        if !self.quiet {
            eprintln!("{}: {message}", prog());
        }
    }

    /// Prints a warning on stderr unless `--quiet`.
    pub fn warn(&self, message: impl std::fmt::Display) {
        if !self.quiet {
            eprintln!("{}: warning: {message}", prog());
        }
    }
}

/// How a successful command ended. Fatal errors are `Err` values of the
/// command and exit with code 1.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    /// Exit code 0 (warnings and infos do not change it).
    Ok,
    /// Exit code 2: the input was read, but has Error diagnostics.
    ErrorDiagnostics,
}

/// Exit code for fatal errors (unreadable, refused or unsupported input,
/// usage errors, I/O failures).
pub const EXIT_FATAL: u8 = 1;
/// Exit code when the input has Error diagnostics.
pub const EXIT_ERROR_DIAGNOSTICS: u8 = 2;

impl Status {
    /// `ErrorDiagnostics` if `doc` has any Error diagnostic.
    pub fn of(doc: &Document) -> Self {
        if doc
            .diagnostics
            .iter()
            .any(|d| d.severity == Severity::Error)
        {
            Status::ErrorDiagnostics
        } else {
            Status::Ok
        }
    }
}

impl From<Status> for ExitCode {
    fn from(s: Status) -> Self {
        match s {
            Status::Ok => ExitCode::SUCCESS,
            Status::ErrorDiagnostics => ExitCode::from(EXIT_ERROR_DIAGNOSTICS),
        }
    }
}

/// Reader flags shared by commands that open a document.
#[derive(Debug, Clone, Copy, Default, clap::Args)]
pub struct ReadArgs {
    /// Keep internal VertiPaq tables (`H$`, `R$`, `U$`) of PBIX/ABF models.
    #[arg(long)]
    pub include_system_tables: bool,
    /// Do not decompress the PBIX `DataModel` (or PBIP `.pbi/cache.abf`) to
    /// read the model schema; faster, but the PBIX model is reported as not
    /// loaded.
    #[arg(long)]
    pub no_pbix_schema: bool,
}

impl ReadArgs {
    pub fn open_options(&self) -> OpenOptions {
        OpenOptions {
            include_system_tables: self.include_system_tables,
            load_pbix_schema: !self.no_pbix_schema,
            ..OpenOptions::default()
        }
    }
}

/// Opens `path` with a user-facing error message on failure.
pub fn open_document(path: &Path, opts: &OpenOptions) -> anyhow::Result<OpenedDocument> {
    tracing::info!(path = %path.display(), "opening");
    open_path(path, opts).map_err(|e| open_error(path, e))
}

/// Turns a reader error into a message that names the input.
pub fn open_error(path: &Path, e: CoreError) -> anyhow::Error {
    let shown = path.display();
    match e {
        CoreError::Io { source, .. } if source.kind() == io::ErrorKind::NotFound => {
            anyhow!("cannot open {shown}: no such file or directory")
        }
        CoreError::Io { source, .. } => anyhow!("cannot open {shown}: {source}"),
        CoreError::Container(c) => anyhow!("cannot open {shown}: refused: {c}"),
        CoreError::Unsupported { format, hint } => {
            anyhow!("cannot open {shown}: not a supported Power BI format ({format}): {hint}")
        }
        other => anyhow!(other).context(format!("cannot open {shown}")),
    }
}

/// Runs `f` against a buffered stdout. A closed pipe (`dashcraft … | head`)
/// is not an error.
pub fn with_stdout(f: impl FnOnce(&mut dyn Write) -> io::Result<()>) -> anyhow::Result<()> {
    let stdout = io::stdout();
    let mut out = io::BufWriter::new(stdout.lock());
    let result = f(&mut out).and_then(|()| out.flush());
    match result {
        Err(e) if e.kind() == io::ErrorKind::BrokenPipe => Ok(()),
        other => other.context("cannot write to stdout"),
    }
}

/// Writes `value` as JSON (compact or pretty) followed by a newline.
pub fn print_json(value: &impl Serialize, pretty: bool) -> anyhow::Result<()> {
    let text = if pretty {
        serde_json::to_string_pretty(value)
    } else {
        serde_json::to_string(value)
    }
    .context("cannot serialize JSON output")?;
    with_stdout(|out| writeln!(out, "{text}"))
}

/// Writes `path` through a temporary file in the same folder, renamed into
/// place when complete: a failed write leaves no partial file, and without
/// `overwrite` an existing file is never replaced.
pub fn write_file<T>(
    path: &Path,
    overwrite: bool,
    write: impl FnOnce(&mut BufWriter<&mut File>) -> anyhow::Result<T>,
) -> anyhow::Result<T> {
    let dir = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(dir).with_context(|| format!("cannot create {}", dir.display()))?;
    let mut builder = tempfile::Builder::new();
    builder.prefix(".dashcraft-").suffix(".tmp");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        // Like `File::create`: 0o666 minus the umask (tempfile uses 0o600).
        builder.permissions(fs::Permissions::from_mode(0o666));
    }
    let mut temp = builder
        .tempfile_in(dir)
        .with_context(|| format!("cannot create a file in {}", dir.display()))?;
    let value = {
        let mut w = BufWriter::new(temp.as_file_mut());
        let value = write(&mut w)?;
        w.flush()
            .with_context(|| format!("cannot write {}", path.display()))?;
        value
    };
    let persisted = if overwrite {
        temp.persist(path)
    } else {
        temp.persist_noclobber(path)
    };
    persisted.map_err(|e| match e.error.kind() {
        io::ErrorKind::AlreadyExists => anyhow!(
            "{} already exists; use --force to overwrite it",
            path.display()
        ),
        _ => anyhow!(e.error).context(format!("cannot write {}", path.display())),
    })?;
    Ok(value)
}

/// Collapses whitespace (including line breaks) to single spaces and
/// shortens to at most `max` characters, ending with `…` when cut.
pub fn one_line(text: &str, max: usize) -> String {
    let collapsed = text.split_whitespace().collect::<Vec<_>>().join(" ");
    truncate(&collapsed, max)
}

/// First non-blank line, trimmed and shortened to `max` characters; `…` is
/// appended when anything was left out.
pub fn first_line(text: &str, max: usize) -> String {
    let mut lines = text.lines().map(str::trim).filter(|l| !l.is_empty());
    let first = lines.next().unwrap_or_default();
    let more = lines.next().is_some();
    let short = truncate(first, max);
    if more && !short.ends_with('…') {
        format!("{short} …")
    } else {
        short
    }
}

/// Shortens to at most `max` characters (not bytes), ending with `…`.
pub fn truncate(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_owned();
    }
    let keep = max.saturating_sub(1);
    let mut out: String = text.chars().take(keep).collect();
    out.push('…');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_is_shortened_on_char_boundaries() {
        assert_eq!(
            one_line("CALCULATE(\n\t[A],\n  B )", 80),
            "CALCULATE( [A], B )"
        );
        assert_eq!(truncate("ééééé", 3), "éé…");
        assert_eq!(truncate("abc", 3), "abc");
        assert_eq!(first_line("\n  let\n  Source = 1\nin Source", 80), "let …");
        assert_eq!(first_line("one line", 4), "one…");
        assert_eq!(first_line("", 10), "");
    }

    #[test]
    fn program_name_follows_the_invoked_binary() {
        use std::ffi::OsStr;
        let name = |arg0: &str| prog_from_arg0(Some(OsStr::new(arg0)));
        assert_eq!(name("/usr/local/bin/pbiview"), "pbiview");
        assert_eq!(name("PBIVIEW.EXE"), "pbiview");
        assert_eq!(name("target/debug/dashcraft"), "dashcraft");
        assert_eq!(name("pbiview-app"), "dashcraft");
        assert_eq!(prog_from_arg0(None), "dashcraft");
    }
}
