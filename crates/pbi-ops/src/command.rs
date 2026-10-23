//! The typed [`Command`] trait, its metadata ([`CommandMeta`]) and the
//! type-erased [`CommandSpec`] the [`Registry`](crate::Registry) stores.

use std::fmt;
use std::future::Future;
use std::panic::AssertUnwindSafe;
use std::pin::Pin;
use std::task::{Context, Poll};

use schemars::JsonSchema;
use schemars::generate::SchemaSettings;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::ctx::Ctx;
use crate::error::OpError;
use crate::workspace::Workspace;

/// A boxed, `Send` future.
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// What a command is about; groups the catalog.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "camelCase")]
pub enum Category {
    /// The catalog itself.
    Meta,
    /// Opening, listing and closing documents.
    Workspace,
    /// Model and report structure.
    Inspect,
    /// Broken-reference checks.
    Lint,
    /// Loading and browsing data.
    Data,
    /// DAX queries and measure values.
    Dax,
    /// Report pages to SVG / PNG.
    Render,
    /// The interactive canvas (slicers, cross-filters, scrolling).
    Canvas,
    /// Writing data and documentation.
    Export,
    /// Writing other formats (PBIP, PBIT).
    Convert,
    /// Text transforms.
    Transform,
    /// Opt-in AI capabilities (`ai.*`). Not in `Registry::default_catalog()`;
    /// front ends add them with `pbi_ai::register`.
    Ai,
}

impl Category {
    /// Every category, in catalog order.
    pub const ALL: [Category; 12] = [
        Category::Meta,
        Category::Workspace,
        Category::Inspect,
        Category::Lint,
        Category::Data,
        Category::Dax,
        Category::Render,
        Category::Canvas,
        Category::Export,
        Category::Convert,
        Category::Transform,
        Category::Ai,
    ];

    /// `meta`, `workspace`, …
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Category::Meta => "meta",
            Category::Workspace => "workspace",
            Category::Inspect => "inspect",
            Category::Lint => "lint",
            Category::Data => "data",
            Category::Dax => "dax",
            Category::Render => "render",
            Category::Canvas => "canvas",
            Category::Export => "export",
            Category::Convert => "convert",
            Category::Transform => "transform",
            Category::Ai => "ai",
        }
    }

    /// Section title in the Markdown catalog (`Meta`, …, `AI`).
    #[must_use]
    pub fn title(self) -> String {
        match self {
            Category::Ai => "AI".to_owned(),
            other => {
                let s = other.as_str();
                let mut c = s.chars();
                c.next()
                    .map(|f| f.to_uppercase().chain(c).collect())
                    .unwrap_or_default()
            }
        }
    }
}

impl fmt::Display for Category {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Static description of a command.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CommandMeta {
    /// Stable dotted id, e.g. `doc.open`. Never renamed once released.
    pub id: &'static str,
    /// Short title for menus and tool lists.
    pub title: &'static str,
    /// One line for people and language models: what it does and returns.
    pub summary: &'static str,
    pub category: Category,
    /// Changes the in-memory state of an open document (data, derived
    /// columns, canvas view state, or closes it). Such commands run one at a
    /// time per document, in submission order. Files on disk are never
    /// touched unless [`writes_files`](Self::writes_files).
    pub mutates_document: bool,
    /// May take seconds or more; reports progress and honours cancellation.
    pub long_running: bool,
    /// Exposed to AI agents (MCP) by default: no side effects outside the
    /// workspace except through [`PathPolicy`](crate::PathPolicy)-checked
    /// paths.
    pub agent_safe: bool,
    /// Writes files (always through
    /// [`PathPolicy::check_write`](crate::PathPolicy::check_write)).
    pub writes_files: bool,
    /// Runs other commands of the catalog (`commands.batch`): its effects
    /// are those of any command it may call, so adapters derive its hints
    /// (MCP `readOnlyHint`, …) from the whole catalog it can reach.
    pub invokes_other_commands: bool,
    /// The DashCraft version that added the command.
    pub since: &'static str,
    /// `(title, params JSON)` examples; each must parse into the command's
    /// parameters (checked by tests).
    pub examples: &'static [(&'static str, &'static str)],
}

/// A typed command. Implement it on a unit struct and register it with
/// [`Registry::register`](crate::Registry::register).
///
/// Parameters are a `#[serde(deny_unknown_fields, rename_all =
/// "camelCase")]` struct deriving [`JsonSchema`]; the output any
/// serializable type deriving [`JsonSchema`]. A command never panics on
/// bad input; it returns an [`OpError`].
pub trait Command: Send + Sync + 'static {
    type Params: DeserializeOwned + JsonSchema + Send + 'static;
    type Output: Serialize + JsonSchema + Send + 'static;

    fn meta() -> CommandMeta;

    fn run(
        ws: &Workspace,
        params: Self::Params,
        ctx: &Ctx,
    ) -> impl Future<Output = Result<Self::Output, OpError>> + Send;
}

/// The type-erased runner of a command.
pub type CallFn =
    for<'a> fn(&'a Workspace, Value, &'a Ctx) -> BoxFuture<'a, Result<Value, OpError>>;

/// A registered command: metadata, JSON Schemas and the erased runner.
#[derive(Clone)]
pub struct CommandSpec {
    pub meta: CommandMeta,
    /// JSON Schema (draft 2020-12) of the parameters (deserialize contract).
    pub params_schema: Value,
    /// JSON Schema (draft 2020-12) of the result (serialize contract).
    pub output_schema: Value,
    call: CallFn,
    check: fn(&Value) -> Result<(), OpError>,
}

impl fmt::Debug for CommandSpec {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CommandSpec")
            .field("meta", &self.meta)
            .finish_non_exhaustive()
    }
}

impl CommandSpec {
    /// The spec of `C`.
    #[must_use]
    pub fn of<C: Command>() -> Self {
        Self {
            meta: C::meta(),
            params_schema: params_schema::<C::Params>(),
            output_schema: output_schema::<C::Output>(),
            call: call_erased::<C>,
            check: check_params::<C::Params>,
        }
    }

    /// Checks `params` against the parameter type without running anything
    /// (unknown fields, missing fields, wrong types).
    ///
    /// # Errors
    /// [`OpErrorCode::InvalidParams`](crate::OpErrorCode::InvalidParams).
    pub fn validate(&self, params: &Value) -> Result<(), OpError> {
        (self.check)(params)
    }

    /// Runs the command (no catalog policy, ordering or panic guard: use
    /// [`Registry::execute`](crate::Registry::execute)).
    pub(crate) fn call<'a>(
        &self,
        ws: &'a Workspace,
        params: Value,
        ctx: &'a Ctx,
    ) -> BoxFuture<'a, Result<Value, OpError>> {
        (self.call)(ws, params, ctx)
    }
}

/// JSON Schema of a parameter type (how it is deserialized).
#[must_use]
pub fn params_schema<T: JsonSchema>() -> Value {
    SchemaSettings::draft2020_12()
        .for_deserialize()
        .into_generator()
        .into_root_schema_for::<T>()
        .to_value()
}

/// JSON Schema of an output type (how it is serialized).
#[must_use]
pub fn output_schema<T: JsonSchema>() -> Value {
    SchemaSettings::draft2020_12()
        .for_serialize()
        .into_generator()
        .into_root_schema_for::<T>()
        .to_value()
}

/// `null` and absent parameters mean `{}`.
fn normalize(params: Value) -> Value {
    match params {
        Value::Null => Value::Object(serde_json::Map::new()),
        other => other,
    }
}

/// Deserializes command parameters.
///
/// # Errors
/// [`OpErrorCode::InvalidParams`](crate::OpErrorCode::InvalidParams) with
/// serde's message (unknown or missing field, wrong type).
pub fn parse_params<P: DeserializeOwned>(params: Value) -> Result<P, OpError> {
    serde_json::from_value(normalize(params))
        .map_err(|e| OpError::invalid_params(format!("invalid params: {e}")))
}

fn check_params<P: DeserializeOwned>(params: &Value) -> Result<(), OpError> {
    parse_params::<P>(params.clone()).map(|_| ())
}

fn call_erased<'a, C: Command>(
    ws: &'a Workspace,
    params: Value,
    ctx: &'a Ctx,
) -> BoxFuture<'a, Result<Value, OpError>> {
    Box::pin(async move {
        let params: C::Params = parse_params(params)?;
        let out = C::run(ws, params, ctx).await?;
        serde_json::to_value(&out)
            .map_err(|e| OpError::internal(format!("cannot serialize the result: {e}")))
    })
}

/// Polls a future, turning a panic into [`OpErrorCode::Internal`](crate::OpErrorCode::Internal).
pub(crate) struct CatchUnwind<'a> {
    inner: BoxFuture<'a, Result<Value, OpError>>,
    id: &'static str,
}

impl<'a> CatchUnwind<'a> {
    pub(crate) fn new(inner: BoxFuture<'a, Result<Value, OpError>>, id: &'static str) -> Self {
        Self { inner, id }
    }
}

impl Future for CatchUnwind<'_> {
    type Output = Result<Value, OpError>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let id = self.id;
        match std::panic::catch_unwind(AssertUnwindSafe(|| self.inner.as_mut().poll(cx))) {
            Ok(poll) => poll,
            Err(panic) => {
                let what = panic
                    .downcast_ref::<&str>()
                    .map(|s| (*s).to_owned())
                    .or_else(|| panic.downcast_ref::<String>().cloned())
                    .unwrap_or_else(|| "unknown panic".to_owned());
                tracing::error!(command = id, "command panicked: {what}");
                Poll::Ready(Err(OpError::internal(format!(
                    "command {id} failed unexpectedly (panic: {what})"
                ))))
            }
        }
    }
}
