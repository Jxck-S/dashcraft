//! Lossless TMDL (Tabular Model Definition Language) parser and writer.
//!
//! TMDL is the text format of the semantic model in PBIP projects
//! (`*.SemanticModel/definition/*.tmdl`). This crate has two layers:
//!
//! # 1. CST (lossless)
//!
//! [`parse_file`] turns one file into a [`TmdlFile`]: a concrete syntax tree
//! of [`Node`]s built from indentation ([`lexer`] tokenizes each line into a
//! [`lexer::Header`] with byte spans; [`cst`] builds the tree). Every byte of
//! the input (BOM, CRLF/LF, trailing whitespace, blank lines, `//` comments,
//! `///` descriptions, expression blocks) belongs to exactly one line of the
//! tree, so `file.to_string() == input`. Nodes carry 1-based line/column
//! positions. Syntax errors ([`TmdlError::Syntax`]) carry path, line and
//! column; nothing panics on malformed input and nesting is bounded
//! ([`cst::MAX_DEPTH`]).
//!
//! # 2. Model mapping
//!
//! [`load_model`] parses a set of files and maps them to a
//! [`pbi_model::Model`] (plus the optional [`pbi_model::Database`]). Known
//! object types and properties map to typed fields (case-insensitively, as
//! TMDL is read); unknown ones are kept in the owner's `extra` bag as
//! `PropValue::{Bool, Text, Expression, Object, List}` with an Info
//! diagnostic ([`codes::UNMODELED`]), never an error. `///` lines become
//! descriptions; `ref table` lines in `model.tmdl` define the table order
//! (likewise roles, cultures, perspectives); a table declared in several
//! files is merged (TMDL partial declarations); columns of calculated tables
//! become `ColumnKind::CalculatedTableColumn`; a table's `calculationGroup`
//! (with its `calculationItem`s, `formatStringDefinition`s and selection
//! expressions) becomes `Table::calculation_group` (a second or named
//! declaration stays in `extra`); named M expressions are split
//! into value + `meta` record. Objects get an `origin` (path + line). IDs are
//! left unassigned (`Document::assign_ids` assigns them).
//!
//! Expression values (DAX, M, annotation values) are de-indented as
//! described in [`expr`]: single-line values are trimmed; for indented
//! blocks the common leading whitespace of the non-blank lines is removed,
//! whitespace-only lines become empty and trailing blank lines are dropped;
//! ```` ``` ```` fenced blocks are verbatim minus the closing fence's
//! indentation. The CST keeps the exact text either way.
//!
//! # Writer
//!
//! [`write_model`] serializes a model to canonical TMDL files the way Power
//! BI Desktop lays them out (tabs, Desktop property order, blank lines
//! between child objects, `ref` ordering, minimal name quoting, `extra` and
//! annotations written back). It re-indents expressions two levels below
//! their declaration and switches to a fenced block whenever the indented
//! form would not read back identically, so `load(write(m)) == m`.
//! [`write_model_preserving`] (or [`LoadedModel::write`]) additionally
//! re-emits every file whose content is semantically unchanged byte for
//! byte from its CST, so saving an edited model only rewrites the files of
//! edited objects.
//!
//! The crate never touches the file system except in
//! [`load_model_from_dir`]; containers (ZIP, folders) pass `(path, text)`
//! pairs to [`load_model`].

pub mod cst;
mod error;
pub mod expr;
pub mod lexer;
mod load;
mod writer;

use std::path::{Path, PathBuf};

use pbi_model::{Database, Diagnostic, Model};

pub use cst::{ExprBlock, MAX_DEPTH, Node, TmdlFile, codes, parse_file};
pub use error::{SyntaxError, TmdlError};
pub use lexer::{IndentStyle, Line, LineEnding};
pub use load::{COLON_VALUE_KEY, clear_origins};
pub use writer::{WriteOptions, write_model, write_model_preserving, write_model_with};

/// Largest `.tmdl` file [`load_model_from_dir`] reads.
pub const MAX_FILE_BYTES: u64 = 64 * 1024 * 1024;
/// Most `.tmdl` files [`load_model_from_dir`] reads.
pub const MAX_FILES: usize = 20_000;
const MAX_DIR_DEPTH: usize = 8;

/// A model read from TMDL files.
#[derive(Debug, Clone, PartialEq)]
pub struct LoadedModel {
    /// The model (`model.database` is set too when a `database` was found).
    pub model: Model,
    /// The `database` object (`database.tmdl`), if any.
    pub database: Option<Database>,
    /// The lossless CSTs, in input order (for byte-identical re-emit).
    pub files: Vec<TmdlFile>,
    /// Lexical and mapping diagnostics (never errors).
    pub diagnostics: Vec<Diagnostic>,
}

impl LoadedModel {
    /// Writes the (possibly edited) model back: unchanged files verbatim,
    /// changed files canonically (see [`write_model_preserving`]).
    pub fn write(&self) -> Vec<(String, String)> {
        write_model_preserving(&self.model, self.database.as_ref(), &self.files)
    }
}

/// Parses and maps a set of TMDL files. Paths are only used for
/// diagnostics, origins and [`write_model_preserving`] matching (use paths
/// relative to the opened container, `/`-separated). Fails on the first
/// syntax error.
pub fn load_model(files: &[(String, String)]) -> Result<LoadedModel, TmdlError> {
    let parsed = files
        .iter()
        .map(|(path, text)| parse_file(path, text))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(load_parsed(parsed))
}

/// Maps already parsed files to a model.
pub fn load_parsed(files: Vec<TmdlFile>) -> LoadedModel {
    let mut loader = load::Loader::default();
    for f in &files {
        loader.add_file(f);
    }
    let loaded = loader.finish(true);
    LoadedModel {
        model: loaded.model,
        database: loaded.database,
        files,
        diagnostics: loaded.diagnostics,
    }
}

/// Loads every `*.tmdl` file under `dir` (a `definition/` folder, or a
/// `*.SemanticModel` folder containing one). Paths are relative to the
/// definition folder and sorted. Symbolic links are not followed.
pub fn load_model_from_dir(dir: &Path) -> Result<LoadedModel, TmdlError> {
    let definition = dir.join("definition");
    let root = if !dir.join("model.tmdl").is_file() && definition.is_dir() {
        definition
    } else {
        dir.to_path_buf()
    };
    let mut found = Vec::new();
    collect_tmdl(&root, "", 0, &mut found)?;
    found.sort();
    let mut files = Vec::with_capacity(found.len());
    for (rel, abs) in found {
        let io = |source| TmdlError::Io {
            path: rel.clone(),
            source,
        };
        let size = std::fs::metadata(&abs).map_err(io)?.len();
        if size > MAX_FILE_BYTES {
            return Err(TmdlError::Limit {
                path: rel,
                what: "file size",
                limit: MAX_FILE_BYTES,
            });
        }
        let bytes = std::fs::read(&abs).map_err(io)?;
        let text =
            String::from_utf8(bytes).map_err(|_| TmdlError::Encoding { path: rel.clone() })?;
        files.push((rel, text));
    }
    load_model(&files)
}

fn collect_tmdl(
    dir: &Path,
    rel: &str,
    depth: usize,
    out: &mut Vec<(String, PathBuf)>,
) -> Result<(), TmdlError> {
    let io = |source| TmdlError::Io {
        path: dir.display().to_string(),
        source,
    };
    if depth > MAX_DIR_DEPTH {
        return Ok(());
    }
    for entry in std::fs::read_dir(dir).map_err(io)? {
        let entry = entry.map_err(io)?;
        let kind = entry.file_type().map_err(io)?;
        let name = entry.file_name().to_string_lossy().into_owned();
        let child_rel = if rel.is_empty() {
            name.clone()
        } else {
            format!("{rel}/{name}")
        };
        if kind.is_dir() {
            collect_tmdl(&entry.path(), &child_rel, depth + 1, out)?;
        } else if kind.is_file()
            && Path::new(&name)
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("tmdl"))
        {
            if out.len() >= MAX_FILES {
                return Err(TmdlError::Limit {
                    path: dir.display().to_string(),
                    what: "number of .tmdl files",
                    limit: MAX_FILES as u64,
                });
            }
            out.push((child_rel, entry.path()));
        }
    }
    Ok(())
}
