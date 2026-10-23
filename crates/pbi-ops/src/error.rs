//! [`OpError`]: the one error type of every command, with stable string
//! codes and their JSON-RPC and CLI exit-code mappings.

use std::fmt;

use pbi_core::CoreError;
use pbi_session::{ExportError, SessionError};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Stable error codes. The string form (`invalidParams`, …) is part of the
/// command contract: it never changes for an existing code; new codes may
/// be added.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum OpErrorCode {
    /// The parameters do not match the command's schema, or a value is out
    /// of range.
    InvalidParams,
    /// No command with that id.
    UnknownCommand,
    /// A named object does not exist: a file, table, column, measure,
    /// report, page or visual.
    NotFound,
    /// The `docId` is not open, or the document lacks what the command
    /// needs (a local semantic model, a report).
    NoDocument,
    /// The input or the operation is not supported (unsupported or refused
    /// file, a feature DashCraft does not implement).
    Unsupported,
    /// The path policy refused a path (outside the read/write roots,
    /// symlink or `..` escape).
    PathDenied,
    /// A file could not be read or written (or already exists).
    Io,
    /// A DAX, SQL, M or CSV evaluation failed: the input was read but the
    /// data or expression has errors.
    Query,
    /// The command was cancelled.
    Cancelled,
    /// Too many concurrent requests or open documents.
    Busy,
    /// A bug: a panic or an unexpected failure.
    Internal,
}

impl OpErrorCode {
    /// Every code, in declaration order.
    pub const ALL: [OpErrorCode; 11] = [
        OpErrorCode::InvalidParams,
        OpErrorCode::UnknownCommand,
        OpErrorCode::NotFound,
        OpErrorCode::NoDocument,
        OpErrorCode::Unsupported,
        OpErrorCode::PathDenied,
        OpErrorCode::Io,
        OpErrorCode::Query,
        OpErrorCode::Cancelled,
        OpErrorCode::Busy,
        OpErrorCode::Internal,
    ];

    /// The stable string code (`invalidParams`, `notFound`, …).
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            OpErrorCode::InvalidParams => "invalidParams",
            OpErrorCode::UnknownCommand => "unknownCommand",
            OpErrorCode::NotFound => "notFound",
            OpErrorCode::NoDocument => "noDocument",
            OpErrorCode::Unsupported => "unsupported",
            OpErrorCode::PathDenied => "pathDenied",
            OpErrorCode::Io => "io",
            OpErrorCode::Query => "query",
            OpErrorCode::Cancelled => "cancelled",
            OpErrorCode::Busy => "busy",
            OpErrorCode::Internal => "internal",
        }
    }

    /// JSON-RPC 2.0 error code: the reserved codes where one fits
    /// (`-32602` invalid params, `-32601` method not found, `-32603`
    /// internal error, `-32800` request cancelled as in LSP), else the
    /// `-32001…-32007` server-error range.
    #[must_use]
    pub fn jsonrpc_code(self) -> i64 {
        match self {
            OpErrorCode::InvalidParams => -32602,
            OpErrorCode::UnknownCommand => -32601,
            OpErrorCode::Internal => -32603,
            OpErrorCode::Cancelled => -32800,
            OpErrorCode::NotFound => -32001,
            OpErrorCode::NoDocument => -32002,
            OpErrorCode::Unsupported => -32003,
            OpErrorCode::PathDenied => -32004,
            OpErrorCode::Io => -32005,
            OpErrorCode::Query => -32006,
            OpErrorCode::Busy => -32007,
        }
    }

    /// `dashcraft` exit code: 2 when the input was read but its data or
    /// expressions have errors ([`OpErrorCode::Query`]), else 1 (fatal or
    /// usage error), as for the other subcommands.
    #[must_use]
    pub fn exit_code(self) -> u8 {
        match self {
            OpErrorCode::Query => 2,
            _ => 1,
        }
    }
}

impl fmt::Display for OpErrorCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The error of a command: a stable [`code`](OpErrorCode), a user-facing
/// message and optional structured details.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct OpError {
    pub code: OpErrorCode,
    pub message: String,
    /// Structured details (e.g. the failing DAX, the candidates of an
    /// ambiguous name).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

impl fmt::Display for OpError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for OpError {}

macro_rules! constructors {
    ($($name:ident => $code:ident),* $(,)?) => {
        $(
            #[doc = concat!("An [`OpErrorCode::", stringify!($code), "`] error.")]
            pub fn $name(message: impl Into<String>) -> Self {
                Self::new(OpErrorCode::$code, message)
            }
        )*
    };
}

impl OpError {
    pub fn new(code: OpErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            data: None,
        }
    }

    constructors! {
        invalid_params => InvalidParams,
        unknown_command => UnknownCommand,
        not_found => NotFound,
        no_document => NoDocument,
        unsupported => Unsupported,
        path_denied => PathDenied,
        io => Io,
        query => Query,
        busy => Busy,
        internal => Internal,
    }

    /// [`OpErrorCode::Cancelled`].
    #[must_use]
    pub fn cancelled() -> Self {
        Self::new(OpErrorCode::Cancelled, "the operation was cancelled")
    }

    /// Attaches structured details.
    #[must_use]
    pub fn with_data(mut self, data: Value) -> Self {
        self.data = Some(data);
        self
    }

    /// Prefixes the message with context (`"cannot open x: "`).
    #[must_use]
    pub fn context(mut self, context: impl fmt::Display) -> Self {
        self.message = format!("{context}: {}", self.message);
        self
    }

    /// The JSON-RPC error object: `{ code: <integer>, message, data: {
    /// code: "<string code>", details? } }`.
    #[must_use]
    pub fn to_jsonrpc(&self) -> Value {
        let mut data = serde_json::Map::new();
        data.insert("code".into(), Value::String(self.code.as_str().into()));
        if let Some(d) = &self.data {
            data.insert("details".into(), d.clone());
        }
        serde_json::json!({
            "code": self.code.jsonrpc_code(),
            "message": self.message,
            "data": Value::Object(data),
        })
    }

    /// See [`OpErrorCode::exit_code`].
    #[must_use]
    pub fn exit_code(&self) -> u8 {
        self.code.exit_code()
    }
}

impl From<CoreError> for OpError {
    fn from(e: CoreError) -> Self {
        match e {
            CoreError::Io { path, source } if source.kind() == std::io::ErrorKind::NotFound => {
                OpError::not_found(format!("{}: no such file or directory", path.display()))
            }
            CoreError::Io { path, source } => {
                OpError::io(format!("I/O error on {}: {source}", path.display()))
            }
            CoreError::Container(c) => OpError::unsupported(format!("refused: {c}")),
            CoreError::Unsupported { format, hint } => OpError::unsupported(format!(
                "not a supported Power BI format ({format}): {hint}"
            )),
            CoreError::Save(message) => OpError::unsupported(format!("cannot save: {message}")),
            other => OpError::internal(other.to_string()),
        }
    }
}

impl From<SessionError> for OpError {
    fn from(e: SessionError) -> Self {
        let message = e.to_string();
        match e {
            SessionError::Open(core) => core.into(),
            SessionError::BlobMap { .. } | SessionError::Cache { .. } | SessionError::Io { .. } => {
                OpError::io(message)
            }
            SessionError::UnknownTable(_)
            | SessionError::UnknownColumn { .. }
            | SessionError::UnknownMeasure(_)
            | SessionError::UnknownRole(_)
            | SessionError::NoCache => OpError::not_found(message),
            SessionError::InvalidFilter(_) | SessionError::Derived { .. } => {
                OpError::invalid_params(message)
            }
            // A role's DAX row filter is model content that fails to evaluate.
            SessionError::RoleFilter { role, table, .. } => {
                OpError::query(message).with_data(serde_json::json!({
                    "role": role,
                    "table": table,
                }))
            }
            SessionError::Csv { .. } | SessionError::Engine(_) | SessionError::Dax(_) => {
                OpError::query(message)
            }
            SessionError::Cancelled => OpError::cancelled(),
            _ => OpError::internal(message),
        }
    }
}

impl From<ExportError> for OpError {
    fn from(e: ExportError) -> Self {
        match e {
            ExportError::Session(s) => s.into(),
            ExportError::InvalidOption(m) => OpError::invalid_params(m),
            other => OpError::io(other.to_string()),
        }
    }
}

impl From<tokio::task::JoinError> for OpError {
    fn from(e: tokio::task::JoinError) -> Self {
        if e.is_cancelled() {
            OpError::cancelled()
        } else {
            OpError::internal(format!("a worker task failed: {e}"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_have_stable_strings_and_mappings() {
        for code in OpErrorCode::ALL {
            let json = serde_json::to_value(code).unwrap_or_default();
            assert_eq!(json, Value::String(code.as_str().to_owned()));
            assert!(code.jsonrpc_code() < -32000 || code.jsonrpc_code() <= -32600);
        }
        let mut rpc: Vec<i64> = OpErrorCode::ALL.iter().map(|c| c.jsonrpc_code()).collect();
        rpc.sort_unstable();
        rpc.dedup();
        assert_eq!(
            rpc.len(),
            OpErrorCode::ALL.len(),
            "JSON-RPC codes are unique"
        );
        assert_eq!(OpErrorCode::InvalidParams.jsonrpc_code(), -32602);
        assert_eq!(OpErrorCode::UnknownCommand.jsonrpc_code(), -32601);
        assert_eq!(OpErrorCode::Query.exit_code(), 2);
        assert_eq!(OpErrorCode::NotFound.exit_code(), 1);
        assert_eq!(OpErrorCode::Internal.exit_code(), 1);
    }

    #[test]
    fn library_errors_map_to_codes() {
        let e: OpError = SessionError::UnknownTable("X".into()).into();
        assert_eq!(e.code, OpErrorCode::NotFound);
        let e: OpError = SessionError::Cancelled.into();
        assert_eq!(e.code, OpErrorCode::Cancelled);
        let e: OpError = SessionError::InvalidFilter("bad".into()).into();
        assert_eq!(e.code, OpErrorCode::InvalidParams);
        let e: OpError = SessionError::UnknownRole("Nope".into()).into();
        assert_eq!(e.code, OpErrorCode::NotFound);
        let e: OpError = SessionError::RoleFilter {
            role: "East".into(),
            table: "Sales".into(),
            message: "no column [Nope]".into(),
        }
        .into();
        assert_eq!(e.code, OpErrorCode::Query);
        assert!(e.message.contains("no column [Nope]"), "{}", e.message);
        assert_eq!(
            e.data,
            Some(serde_json::json!({"role": "East", "table": "Sales"}))
        );
        let e: OpError = CoreError::Io {
            path: "missing.pbip".into(),
            source: std::io::Error::from(std::io::ErrorKind::NotFound),
        }
        .into();
        assert_eq!(e.code, OpErrorCode::NotFound);
        let e: OpError = CoreError::Unsupported {
            format: "unknown".into(),
            hint: "try a .pbip".into(),
        }
        .into();
        assert_eq!(e.code, OpErrorCode::Unsupported);
    }

    #[test]
    fn jsonrpc_errors_carry_the_string_code() {
        let e = OpError::not_found("no table 'X'").with_data(serde_json::json!({"tables": ["A"]}));
        let rpc = e.to_jsonrpc();
        assert_eq!(rpc["code"], serde_json::json!(-32001));
        assert_eq!(rpc["data"]["code"], serde_json::json!("notFound"));
        assert_eq!(rpc["data"]["details"]["tables"][0], serde_json::json!("A"));
        let back: OpError = serde_json::from_value(serde_json::to_value(&e).unwrap_or_default())
            .unwrap_or_else(|_| OpError::internal("x"));
        assert_eq!(back, e);
    }
}
