//! Format detection, safe containers and readers producing a unified
//! document.
//!
//! # Layers
//!
//! - [`container`]: read-only, validated views over untrusted inputs:
//!   [`DirContainer`](container::DirContainer) (folders),
//!   [`ZipContainer`](container::ZipContainer) (PBIX, PBIT, XLSX, VPAX, ...)
//!   and [`MemContainer`](container::MemContainer), all behind the
//!   [`Container`](container::Container) trait, plus
//!   [`SubContainer`](container::SubContainer) views of sub-folders.
//! - [`detect`]: content-based format detection over paths, bytes and
//!   containers, with human-readable signals and discovered parts.
//! - [`reader`]: [`open_path`] / [`open_bytes`] turn any supported input
//!   into an [`OpenedDocument`]: a unified [`pbi_model::Document`] (model,
//!   reports, diagnostics, source info) plus the container and the parsed
//!   sources needed to load data or save later.
//! - [`save`]: [`save_as_pbip`] (TMDL + PBIR project) and [`export_pbit`]
//!   (template ZIP) for documents opened by the readers.
//! - [`redact`]: secret redaction applied to every stored metadata part.
//! - [`lint`]: the broken-reference linter ([`lint()`](lint::lint) over an
//!   [`OpenedDocument`], [`lint_document`] over a bare document); rules are
//!   documented in `src/lint/RULES.md`.
//!
//! # Security guarantees
//!
//! Every file is untrusted input; this crate is the boundary between disk
//! (or ZIP) bytes and the parsers.
//!
//! - **No path escapes.** Entry names are normalized (`\` → `/`, `.` and
//!   empty components dropped) and archives with absolute paths, `..`
//!   components, drive letters, NUL bytes or raw duplicate names are refused
//!   at open. Callers address entries only by these normalized relative
//!   paths, and lookups never resolve `..`. Nothing is extracted to disk.
//! - **No symlink following.** Folders are walked without following links;
//!   links are skipped and reported. Each directory read re-canonicalizes
//!   the file and refuses paths that now resolve outside the canonical root.
//! - **Bounded resources.** [`Limits`](container::Limits) caps entry count,
//!   per-entry and total uncompressed size, compression ratio (per entry and
//!   archive-wide), path length and depth. ZIP caps are checked on declared
//!   sizes at open (the entry count before the central directory is even
//!   parsed) and on actual bytes during reads, which go through a reader
//!   bounded to the declared size + 1; overlapping-entry zip bombs are
//!   refused. Detection reads at most 32 MiB of a single file and scans
//!   JSON with a bounded shallow parser.
//! - **Limited decoding, no execution.** Only stored and deflate ZIP
//!   entries are decoded; encrypted entries and other methods return
//!   [`ContainerError::Unsupported`](container::ContainerError::Unsupported).
//!   Custom visuals and connectors are only identified, never run.
//! - **No panics, no `unsafe`.** All failures are typed errors.
//!
//! Known gaps: a local attacker who can swap a directory component for a
//! symlink in the short window between canonicalization and `open` can still
//! redirect a directory read (closing it fully needs `openat`-style APIs).
//! Reads of a file-backed ZIP are serialized through a mutex.

pub mod container;
pub mod detect;
mod error;
pub mod lint;
pub mod reader;
pub mod redact;
pub mod save;

pub use error::CoreError;
pub use lint::{Finding, LintOptions, LintReport, LintSummary, lint, lint_document};
pub use reader::{
    ExtraModel, FormatOutline, MetadataPart, ModelSource, OpenOptions, OpenedDocument,
    REMOTE_DESCRIPTION, ReportSource, codes, open_bytes, open_path,
};
pub use save::{SaveOptions, export_pbit, pbip_files, pbit_bytes, save_as_pbip};
