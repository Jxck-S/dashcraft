//! The [`Provider`] abstraction, its request/response types, the adapters
//! ([`OpenAiCompatible`], [`AzureOpenAi`], [`Ollama`], [`MockProvider`]) and
//! the [`ProviderFactory`] that builds them from settings.
//!
//! HTTP adapters use `ureq` (blocking, rustls) on a detached thread awaited
//! from async code, with connect and read timeouts; a cancelled token stops
//! waiting at once and returns [`AiError::Cancelled`]. They never follow redirects, and
//! loopback endpoints bypass proxies.

mod http;
mod mock;
mod ollama;
mod openai;
mod output;

use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use pbi_ops::BoxFuture;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio_util::sync::CancellationToken;

pub use http::{
    HeaderPreview, HeaderValue, HttpPreview, HttpRequest, MAX_RESPONSE_BYTES, USER_AGENT,
};
pub use mock::{MockProvider, MockReply};
pub use ollama::Ollama;
pub use openai::{AzureOpenAi, OpenAiCompatible};
pub use output::{extract_json, parse_output};

use crate::capability::Capability;
use crate::endpoint::Endpoint;
use crate::error::AiError;
use crate::secrets::Secret;
use crate::settings::{
    AZURE_DEFAULT_API_VERSION, ProviderKind, ProviderSettings, StructuredOutput,
};

/// Who wrote a chat message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    System,
    User,
    Assistant,
}

/// One chat message.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ChatMessage {
    pub role: Role,
    pub content: String,
}

impl ChatMessage {
    #[must_use]
    pub fn system(content: impl Into<String>) -> Self {
        Self {
            role: Role::System,
            content: content.into(),
        }
    }

    #[must_use]
    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: Role::User,
            content: content.into(),
        }
    }
}

/// The JSON Schema the answer must follow.
#[derive(Debug, Clone, PartialEq)]
pub struct ResponseSchema {
    /// Schema name (`[a-zA-Z0-9_-]`, OpenAI requires one).
    pub name: String,
    pub schema: Value,
}

/// A chat completion request (provider-neutral).
#[derive(Debug, Clone, PartialEq)]
pub struct ChatRequest {
    pub messages: Vec<ChatMessage>,
    pub temperature: Option<f64>,
    pub max_tokens: Option<u32>,
    /// Ask for JSON following this schema where the provider supports it.
    pub response_schema: Option<ResponseSchema>,
    /// Total time allowed for the call.
    pub timeout: Duration,
    /// The capability being run (logging, mock answers; never sent).
    pub capability: Option<Capability>,
}

impl ChatRequest {
    /// A request with `messages`, no limits and a 120 s timeout.
    #[must_use]
    pub fn new(messages: Vec<ChatMessage>) -> Self {
        Self {
            messages,
            temperature: None,
            max_tokens: None,
            response_schema: None,
            timeout: Duration::from_secs(120),
            capability: None,
        }
    }

    /// Total characters of the messages.
    #[must_use]
    pub fn chars(&self) -> usize {
        self.messages
            .iter()
            .map(|m| m.content.chars().count())
            .sum()
    }
}

/// Token counts reported by the provider.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Usage {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completion_tokens: Option<u64>,
}

/// A chat completion.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ChatResponse {
    /// The answer text.
    pub content: String,
    /// The model that answered, when reported.
    pub model: Option<String>,
    /// `stop`, `length`, …
    pub finish_reason: Option<String>,
    pub usage: Usage,
}

/// A chat model behind some API.
pub trait Provider: Send + Sync + fmt::Debug {
    /// Provider id (from the settings).
    fn id(&self) -> &str;

    fn kind(&self) -> ProviderKind;

    /// Base URL; consents bind to its `host[:port]`.
    fn endpoint(&self) -> &Endpoint;

    /// The exact HTTP request [`complete`](Self::complete) sends for `req`
    /// (the key as a [`HeaderValue::Secret`]); `ai.preview` shows it
    /// redacted.
    ///
    /// # Errors
    /// [`AiError::Internal`] if the body cannot be built.
    fn http_request(&self, req: &ChatRequest) -> Result<HttpRequest, AiError>;

    /// Runs `req`. Cancelling `cancel` returns [`AiError::Cancelled`]
    /// without waiting for the network.
    fn complete<'a>(
        &'a self,
        req: ChatRequest,
        cancel: CancellationToken,
    ) -> BoxFuture<'a, Result<ChatResponse, AiError>>;
}

/// Builds providers from settings.
pub trait ProviderFactory: Send + Sync + fmt::Debug {
    /// The provider for `settings`, carrying `key` when it sends one.
    ///
    /// # Errors
    /// [`AiError::InvalidSettings`].
    fn create(
        &self,
        settings: &ProviderSettings,
        key: Option<Secret>,
    ) -> Result<Arc<dyn Provider>, AiError>;
}

/// The built-in adapters (the `mock` kind gives canned answers).
#[derive(Debug, Default, Clone, Copy)]
pub struct DefaultProviderFactory;

impl ProviderFactory for DefaultProviderFactory {
    fn create(
        &self,
        settings: &ProviderSettings,
        key: Option<Secret>,
    ) -> Result<Arc<dyn Provider>, AiError> {
        let endpoint = settings.endpoint()?;
        let structured = settings.structured_output.unwrap_or_default();
        let model = settings.model.clone().unwrap_or_default();
        Ok(match settings.kind {
            ProviderKind::OpenAiCompatible => Arc::new(
                OpenAiCompatible::new(&settings.id, endpoint, model, key)
                    .with_structured_output(structured),
            ),
            ProviderKind::AzureOpenAi => Arc::new(
                AzureOpenAi::new(
                    &settings.id,
                    endpoint,
                    settings.deployment.clone().unwrap_or_default(),
                    settings
                        .api_version
                        .clone()
                        .unwrap_or_else(|| AZURE_DEFAULT_API_VERSION.to_owned()),
                    key,
                )
                .with_structured_output(structured),
            ),
            ProviderKind::Ollama => Arc::new(
                Ollama::new(&settings.id, endpoint, model).with_structured_output(structured),
            ),
            ProviderKind::Mock => Arc::new(MockProvider::new(&settings.id)),
        })
    }
}

/// `response_format` of OpenAI-style chat completions.
pub(crate) fn openai_response_format(
    mode: StructuredOutput,
    schema: Option<&ResponseSchema>,
) -> Option<Value> {
    let schema = schema?;
    match mode {
        StructuredOutput::JsonSchema => Some(serde_json::json!({
            "type": "json_schema",
            "json_schema": { "name": schema.name, "schema": schema.schema, "strict": false },
        })),
        StructuredOutput::JsonObject => Some(serde_json::json!({ "type": "json_object" })),
        StructuredOutput::None => None,
    }
}

pub(crate) fn messages_json(req: &ChatRequest) -> Value {
    serde_json::to_value(&req.messages).unwrap_or(Value::Array(Vec::new()))
}
