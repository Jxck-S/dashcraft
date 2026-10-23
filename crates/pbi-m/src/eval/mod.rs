//! The M evaluator: [`Engine`], its [`EvalOptions`] and resource [`Limits`].
//!
//! Every engine call runs on a dedicated thread whose stack is sized from
//! [`Limits::max_depth`] (at least [`EVAL_STACK_BYTES`], [`STACK_BYTES_PER_DEPTH`] per level),
//! so programs that respect the depth limit cannot overflow the stack, even in unoptimized
//! builds. Work is bounded by [`Limits::max_steps`], and the cancellation flag is polled at
//! every evaluation step and in library loops.
//!
//! # Semantics notes
//!
//! * `let` bindings, record fields and list items are lazy and memoized; a binding that
//!   (transitively) needs its own value raises `A cyclic reference was encountered during
//!   evaluation.` A binding may refer to itself without `@` only when no outer scope or library
//!   function has that name (Power Query accepts such recursive functions).
//! * Function arguments are evaluated eagerly (before the call).
//! * Table operations from the library are eager: per-cell results are computed when the table
//!   is built and errors are stored in the cell (raised when the cell is read, as in M).
//! * `try` catches only M errors ([`MError::Evaluation`]); unsupported functions, resolver
//!   failures, limits and cancellation always propagate.
//!
//! # Deviations from Power Query
//!
//! * Numbers are `f64`; `Decimal.Type`/`Currency.Type` precision is approximated by rounding
//!   (`Currency.Type` to 4 decimals, integer facets half-to-even).
//! * Lists and tables are materialized (no streaming or query folding); [`Limits::max_rows`]
//!   bounds them, so `List.Count({1..100000000})` fails instead of streaming.
//! * Text positions and lengths count Unicode scalar values, not UTF-16 code units.
//! * Culture arguments are ignored: parsing accepts invariant and en-US forms and `Text.From`
//!   formats dates the en-US way. Blank text converts to `null` for non-text types.
//! * `table{[key]}?` with several matching rows is still an error (only "no match" yields
//!   `null`).
//! * `Csv.Document` without `Columns` uses the widest row; short rows are padded with `null`;
//!   extra fields are ignored unless `ExtraValues.Error`/`ExtraValues.List` is given; an
//!   unterminated quoted field is a `DataFormat.Error`.
//! * A cell holding an error fails [`arrow::table_to_record_batch`] (Power BI Desktop loads
//!   such cells as blanks and reports an error count).
//! * Function values in a returned result cannot be invoked after the engine call ends (their
//!   environments are released to break reference cycles).

pub mod arrow;
pub(crate) mod env;
pub(crate) mod interp;
pub(crate) mod ir;
pub(crate) mod thunk;

use std::fmt;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use ::arrow::datatypes::DataType;
use ::arrow::record_batch::RecordBatch;
use chrono::{DateTime, FixedOffset, Offset, Utc};
use indexmap::IndexMap;

use crate::ast::Document;
use crate::connectors::{BlobResolver, NoBlobResolver};
use crate::eval::interp::Interp;
use crate::eval::ir::{Compiler, SourceText};
use crate::merror::MError;
use crate::span::line_col;
use crate::value::Value;

/// Minimum stack size of the evaluator thread.
pub const EVAL_STACK_BYTES: usize = 256 * 1024 * 1024;

/// Evaluator stack reserved per [`Limits::max_depth`] level. Unoptimized builds use up to
/// ~16 KiB per level (measured on `let` binding chains); this leaves a 3x margin.
pub const STACK_BYTES_PER_DEPTH: usize = 48 * 1024;

/// Source of the current time (`DateTimeZone.UtcNow`, `DateTime.LocalNow`). Injectable for
/// deterministic tests.
pub trait Clock: Send + Sync + fmt::Debug {
    fn now_utc(&self) -> DateTime<Utc>;

    /// The local UTC offset used by `DateTime.LocalNow` and local-time conversions.
    fn local_offset(&self) -> FixedOffset {
        Utc.fix()
    }
}

/// The system clock and time zone.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now_utc(&self) -> DateTime<Utc> {
        Utc::now()
    }

    fn local_offset(&self) -> FixedOffset {
        chrono::Local::now().offset().fix()
    }
}

/// A clock frozen at a given instant and offset.
#[derive(Debug, Clone, Copy)]
pub struct FixedClock {
    pub now: DateTime<Utc>,
    pub offset: FixedOffset,
}

impl FixedClock {
    pub fn utc(now: DateTime<Utc>) -> Self {
        Self {
            now,
            offset: Utc.fix(),
        }
    }
}

impl Clock for FixedClock {
    fn now_utc(&self) -> DateTime<Utc> {
        self.now
    }

    fn local_offset(&self) -> FixedOffset {
        self.offset
    }
}

/// Resource limits for one evaluation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// Maximum rows of a table / items of a list built by the evaluator.
    pub max_rows: usize,
    /// Maximum evaluation steps (expression evaluations, row iterations, ...).
    pub max_steps: u64,
    /// Maximum nesting of evaluations (recursion depth). The evaluator thread reserves
    /// [`STACK_BYTES_PER_DEPTH`] of (virtual) stack per level.
    pub max_depth: usize,
    /// Maximum size of a binary (blob) or text value, in bytes.
    pub max_binary_bytes: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_rows: 10_000_000,
            max_steps: 2_000_000_000,
            max_depth: 4_000,
            max_binary_bytes: 512 * 1024 * 1024,
        }
    }
}

/// Engine configuration.
#[derive(Clone)]
pub struct EvalOptions {
    /// Resolves `AzureStorage.Blobs` containers (offline: [`crate::MapBlobResolver`]).
    pub resolver: Arc<dyn BlobResolver>,
    pub clock: Arc<dyn Clock>,
    pub limits: Limits,
    /// Set to `true` to stop a running evaluation with [`MError::Cancelled`].
    pub cancel: Arc<AtomicBool>,
}

impl Default for EvalOptions {
    fn default() -> Self {
        Self {
            resolver: Arc::new(NoBlobResolver),
            clock: Arc::new(SystemClock),
            limits: Limits::default(),
            cancel: Arc::new(AtomicBool::new(false)),
        }
    }
}

impl fmt::Debug for EvalOptions {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EvalOptions")
            .field("resolver", &self.resolver)
            .field("clock", &self.clock)
            .field("limits", &self.limits)
            .field("cancel", &self.cancel)
            .finish()
    }
}

/// Evaluates M queries.
///
/// ```
/// use indexmap::IndexMap;
/// use pbi_m::{Engine, EvalOptions, Value};
///
/// let engine = Engine::new(EvalOptions::default());
/// let mut shared = IndexMap::new();
/// shared.insert("Rate".to_owned(), "2 meta [IsParameterQuery = true]".to_owned());
/// let v = engine.evaluate_query("let f = (x) => x * Rate in f(21)", &shared).unwrap();
/// assert_eq!(v, Value::Number(42.0));
/// ```
#[derive(Debug, Clone, Default)]
pub struct Engine {
    options: EvalOptions,
}

pub(crate) fn syntax_error(query: Option<&str>, src: &str, e: &crate::ParseError) -> MError {
    let (line, column) = line_col(src, e.span.start);
    MError::Syntax {
        query: query.map(str::to_owned),
        message: e.message.clone(),
        span: e.span,
        line,
        column,
    }
}

fn eval_source(it: &Interp, src: &str, shared: &IndexMap<String, String>) -> Result<Value, MError> {
    let doc = crate::parse_document(src).map_err(|e| syntax_error(None, src, &e))?;
    let root = it.set_shared(shared);
    let text = Arc::new(SourceText {
        name: None,
        text: Arc::from(src),
    });
    match doc {
        Document::Expression(e) => {
            let node = Compiler::new(text).expr(&e);
            it.eval(&node, &root)
        }
        Document::Section(s) => it.eval_section(&s, text),
    }
}

impl Engine {
    pub fn new(options: EvalOptions) -> Self {
        Self { options }
    }

    pub fn options(&self) -> &EvalOptions {
        &self.options
    }

    /// Runs `f` on a fresh interpreter on the evaluator thread.
    fn run<T: Send>(
        &self,
        f: impl FnOnce(&Interp) -> Result<T, MError> + Send,
    ) -> Result<T, MError> {
        let options = self.options.clone();
        let stack = EVAL_STACK_BYTES.max(
            options
                .limits
                .max_depth
                .saturating_mul(STACK_BYTES_PER_DEPTH)
                .saturating_add(16 * 1024 * 1024),
        );
        std::thread::scope(|scope| {
            let handle = std::thread::Builder::new()
                .name("pbi-m-eval".into())
                .stack_size(stack)
                .spawn_scoped(scope, move || {
                    let it = Interp::new(options);
                    let r = f(&it);
                    drop(it);
                    r
                })
                .map_err(|e| MError::Internal(format!("cannot start the evaluator thread: {e}")))?;
            handle
                .join()
                .unwrap_or_else(|_| Err(MError::Internal("the evaluator thread panicked".into())))
        })
    }

    /// Evaluates an expression document (a query / partition source) or a section document
    /// (returns the record of its members). `shared` maps names of other queries and
    /// parameters (e.g. `StorageContainerUrl`) to their M source; they are parsed and evaluated
    /// only when referenced.
    ///
    /// The result is resolved: every list item, record field and table cell has been evaluated
    /// (errors are kept per slot; see [`crate::Lazy::resolved`]).
    pub fn evaluate_query(
        &self,
        src: &str,
        shared: &IndexMap<String, String>,
    ) -> Result<Value, MError> {
        self.run(|it| {
            let v = eval_source(it, src, shared)?;
            it.resolve(v)
        })
    }

    /// [`Engine::evaluate_query`] without shared queries.
    pub fn evaluate(&self, src: &str) -> Result<Value, MError> {
        self.evaluate_query(src, &IndexMap::new())
    }

    /// Evaluates a partition source and converts the resulting table to Arrow.
    ///
    /// `expected` lists the model's source columns and their Arrow types; the batch has exactly
    /// these columns in this order, and missing or extra columns are an error that names them.
    /// An empty `expected` infers the schema from the table (see
    /// [`arrow::table_to_record_batch`]).
    pub fn evaluate_partition_to_arrow(
        &self,
        src: &str,
        shared: &IndexMap<String, String>,
        expected: &[(String, DataType)],
    ) -> Result<RecordBatch, MError> {
        self.run(|it| {
            let v = it.resolve(eval_source(it, src, shared)?)?;
            let Some(table) = v.as_table() else {
                return Err(MError::expression(format!(
                    "The query returned a value of type {} instead of a table.",
                    v.kind().name()
                )));
            };
            arrow::table_to_record_batch(table, (!expected.is_empty()).then_some(expected))
        })
    }
}
