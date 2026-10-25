//! [`AiError`] and its mapping to the registry's [`OpError`].
//!
//! Messages never contain API keys: adapters redact response bodies and
//! transport errors before building an error (see [`crate::secrets::Redactor`]).

use pbi_ops::{OpError, OpErrorCode};
use serde_json::{Value, json};

use crate::capability::{Capability, DataExposure};

/// What a missing consent would cover; carried by
/// [`AiError::ConsentRequired`] and shown by `ai.preview`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConsentScope {
    /// Provider id from the settings.
    pub provider: String,
    /// `host[:port]` of the provider endpoint.
    pub endpoint_host: String,
    pub capability: Capability,
    pub exposure: DataExposure,
    /// The endpoint is this machine (loopback) or the mock provider.
    pub local: bool,
}

/// Errors of the AI layer.
#[derive(Debug, Clone, thiserror::Error)]
#[non_exhaustive]
pub enum AiError {
    /// AI features are off (the default).
    #[error(
        "AI features are disabled (the default); a person can enable them with \
         `dashcraft run ai.settings.set` or in the app (see docs/AI.md)"
    )]
    Disabled,
    /// No settings location could be determined.
    #[error("no location for the AI settings: set DASHCRAFT_CONFIG_DIR (or HOME/APPDATA)")]
    NoSettingsPath,
    /// Settings (or a settings update) are invalid.
    #[error("{0}")]
    InvalidSettings(String),
    /// The settings file could not be read or written.
    #[error("{0}")]
    SettingsIo(String),
    /// No provider is configured.
    #[error("no AI provider is configured; add one with ai.settings.set (see docs/AI.md)")]
    NoProvider,
    /// The named provider is not configured.
    #[error("no AI provider '{name}' (configured: {known})")]
    UnknownProvider { name: String, known: String },
    /// Data would leave the process without a recorded consent.
    #[error("{}", consent_message(.0))]
    ConsentRequired(Box<ConsentScope>),
    /// The provider needs an API key and none is available.
    #[error(
        "no API key for AI provider '{provider}': store one in the OS keychain (`dashcraft ai \
         login`, the app, or pbi_ai::store_api_key) or set DASHCRAFT_AI_API_KEY{}",
        detail.as_deref().map(|d| format!(" ({d})")).unwrap_or_default()
    )]
    MissingApiKey {
        provider: String,
        detail: Option<String>,
    },
    /// The OS keychain failed.
    #[error("OS keychain: {0}")]
    Keychain(String),
    /// The capability is part of the contract but not implemented yet.
    #[error("AI capability {0} is not implemented yet")]
    Unsupported(Capability),
    /// A caller is not allowed to run this operation.
    #[error("{0}")]
    Forbidden(String),
    /// Bad command input.
    #[error("{0}")]
    InvalidInput(String),
    /// A named model object does not exist.
    #[error("{message}")]
    NotFound {
        message: String,
        candidates: Vec<String>,
    },
    /// The prompt would exceed the capability's input limit.
    #[error(
        "the request for {capability} is {chars} characters, above the limit of {limit} \
         (limits.{capability}.maxInputChars)"
    )]
    InputTooLarge {
        capability: Capability,
        chars: usize,
        limit: usize,
    },
    /// The endpoint could not be reached.
    #[error("cannot reach {endpoint}: {message}")]
    Connection { endpoint: String, message: String },
    /// No (complete) answer within the timeout.
    #[error("{endpoint} did not answer within {:.1} s", *millis as f64 / 1000.0)]
    Timeout { endpoint: String, millis: u64 },
    /// The endpoint answered with an HTTP error.
    #[error("{endpoint} answered HTTP {status}: {body}")]
    Http {
        endpoint: String,
        status: u16,
        body: String,
    },
    /// The endpoint answered something that is not the expected protocol.
    #[error("unexpected answer from {endpoint}: {message}")]
    Protocol { endpoint: String, message: String },
    /// The model's answer does not match the capability's output schema.
    #[error("the model's answer is not a valid {capability} result: {message}; raw answer: {raw}")]
    InvalidOutput {
        capability: Capability,
        message: String,
        /// The answer, truncated and redacted.
        raw: String,
    },
    /// Cancelled through the context's token.
    #[error("the operation was cancelled")]
    Cancelled,
    /// A bug.
    #[error("{0}")]
    Internal(String),
}

fn consent_message(scope: &ConsentScope) -> String {
    let place = if scope.local {
        "a local endpoint on this machine (data does not leave it, but consent is still required)"
            .to_owned()
    } else {
        "a remote endpoint (data leaves this machine)".to_owned()
    };
    format!(
        "consent required: sending {} for {} to AI provider '{}' at {} ({place}) has not been \
         granted. A person must grant it, e.g. `dashcraft run ai.consent.grant -p provider={} -p \
         capability={}{}` or in the app; preview the exact request with ai.preview",
        scope.exposure.description(),
        scope.capability,
        scope.provider,
        scope.endpoint_host,
        scope.provider,
        scope.capability,
        if scope.exposure == DataExposure::Metadata {
            String::new()
        } else {
            format!(" -p exposure={}", scope.exposure.as_str())
        }
    )
}

impl AiError {
    /// Stable machine-readable reason (the `data.reason` of the
    /// [`OpError`]).
    #[must_use]
    pub fn reason(&self) -> &'static str {
        match self {
            AiError::Disabled => "disabled",
            AiError::NoSettingsPath => "noSettingsPath",
            AiError::InvalidSettings(_) => "invalidSettings",
            AiError::SettingsIo(_) => "settingsIo",
            AiError::NoProvider => "noProvider",
            AiError::UnknownProvider { .. } => "unknownProvider",
            AiError::ConsentRequired(_) => "consentRequired",
            AiError::MissingApiKey { .. } => "apiKeyMissing",
            AiError::Keychain(_) => "keychain",
            AiError::Unsupported(_) => "notImplemented",
            AiError::Forbidden(_) => "forbidden",
            AiError::InvalidInput(_) => "invalidInput",
            AiError::NotFound { .. } => "notFound",
            AiError::InputTooLarge { .. } => "inputTooLarge",
            AiError::Connection { .. } => "connection",
            AiError::Timeout { .. } => "timeout",
            AiError::Http { .. } => "http",
            AiError::Protocol { .. } => "protocol",
            AiError::InvalidOutput { .. } => "invalidOutput",
            AiError::Cancelled => "cancelled",
            AiError::Internal(_) => "internal",
        }
    }

    fn code(&self) -> OpErrorCode {
        match self {
            AiError::Disabled
            | AiError::ConsentRequired(_)
            | AiError::MissingApiKey { .. }
            | AiError::Unsupported(_)
            | AiError::Forbidden(_)
            | AiError::NoProvider => OpErrorCode::Unsupported,
            AiError::InvalidSettings(_)
            | AiError::InvalidInput(_)
            | AiError::InputTooLarge { .. } => OpErrorCode::InvalidParams,
            AiError::UnknownProvider { .. } | AiError::NotFound { .. } => OpErrorCode::NotFound,
            AiError::NoSettingsPath
            | AiError::SettingsIo(_)
            | AiError::Keychain(_)
            | AiError::Connection { .. }
            | AiError::Timeout { .. }
            | AiError::Http { .. }
            | AiError::Protocol { .. } => OpErrorCode::Io,
            AiError::InvalidOutput { .. } => OpErrorCode::Query,
            AiError::Cancelled => OpErrorCode::Cancelled,
            AiError::Internal(_) => OpErrorCode::Internal,
        }
    }

    fn details(&self) -> Value {
        let mut data = json!({ "reason": self.reason() });
        let extra = match self {
            AiError::ConsentRequired(scope) => json!({
                "provider": scope.provider,
                "endpointHost": scope.endpoint_host,
                "capability": scope.capability,
                "exposure": scope.exposure,
                "local": scope.local,
                "grant": {
                    "command": "ai.consent.grant",
                    "params": {
                        "provider": scope.provider,
                        "capability": scope.capability,
                        "exposure": scope.exposure,
                    },
                },
            }),
            AiError::MissingApiKey { provider, .. } => json!({ "provider": provider }),
            AiError::Unsupported(capability) => json!({ "capability": capability }),
            AiError::UnknownProvider { name, .. } => json!({ "provider": name }),
            AiError::NotFound { candidates, .. } if !candidates.is_empty() => {
                json!({ "candidates": candidates })
            }
            AiError::Http { status, .. } => json!({ "status": status }),
            AiError::InvalidOutput { raw, .. } => json!({ "raw": raw }),
            _ => Value::Null,
        };
        if let (Value::Object(data), Value::Object(extra)) = (&mut data, extra) {
            data.extend(extra);
        }
        data
    }
}

impl From<AiError> for OpError {
    fn from(e: AiError) -> Self {
        if matches!(e, AiError::Cancelled) {
            return OpError::cancelled();
        }
        OpError::new(e.code(), e.to_string()).with_data(e.details())
    }
}

/// Truncates `text` to at most `max` characters, marking the cut.
#[must_use]
pub fn truncate(text: &str, max: usize) -> String {
    let mut chars = text.char_indices();
    match chars.nth(max) {
        None => text.to_owned(),
        Some((cut, _)) => format!(
            "{}… [{} more characters]",
            text.get(..cut).unwrap_or_default(),
            text.get(cut..).unwrap_or_default().chars().count()
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn consent_errors_explain_how_to_grant() {
        let e = AiError::ConsentRequired(Box::new(ConsentScope {
            provider: "local".into(),
            endpoint_host: "127.0.0.1:11434".into(),
            capability: Capability::DaxExplain,
            exposure: DataExposure::Metadata,
            local: true,
        }));
        let op: OpError = e.into();
        assert_eq!(op.code, OpErrorCode::Unsupported);
        assert!(op.message.contains("ai.consent.grant"), "{}", op.message);
        assert!(op.message.contains("local endpoint"), "{}", op.message);
        let data = op.data.unwrap_or_default();
        assert_eq!(data["reason"], "consentRequired");
        assert_eq!(data["grant"]["params"]["capability"], "dax.explain");
    }

    #[test]
    fn truncation_is_char_safe() {
        assert_eq!(truncate("abc", 5), "abc");
        assert_eq!(truncate("ééééé", 2), "éé… [3 more characters]");
    }
}
