//! OpenAI-style chat completions: [`OpenAiCompatible`] (OpenAI, GitHub
//! Models, LM Studio, vLLM, …) and [`AzureOpenAi`].

use pbi_ops::BoxFuture;
use serde_json::{Map, Value, json};
use tokio_util::sync::CancellationToken;

use super::http::{HeaderValue, HttpRequest, send_json};
use super::{ChatRequest, ChatResponse, Provider, Usage, messages_json, openai_response_format};
use crate::endpoint::Endpoint;
use crate::error::{AiError, truncate};
use crate::secrets::Secret;
use crate::settings::{ProviderKind, StructuredOutput};

/// `POST {endpoint}/chat/completions` with `Authorization: Bearer <key>`
/// (no header without a key, e.g. LM Studio).
#[derive(Debug, Clone)]
pub struct OpenAiCompatible {
    id: String,
    endpoint: Endpoint,
    model: String,
    key: Option<Secret>,
    structured: StructuredOutput,
}

impl OpenAiCompatible {
    #[must_use]
    pub fn new(
        id: &str,
        endpoint: Endpoint,
        model: impl Into<String>,
        key: Option<Secret>,
    ) -> Self {
        Self {
            id: id.to_owned(),
            endpoint,
            model: model.into(),
            key,
            structured: StructuredOutput::default(),
        }
    }

    #[must_use]
    pub fn with_structured_output(mut self, mode: StructuredOutput) -> Self {
        self.structured = mode;
        self
    }

    /// OpenAI's own API wants `max_completion_tokens`; compatible servers
    /// take `max_tokens`.
    fn max_tokens_field(&self) -> &'static str {
        if self.endpoint.host() == "api.openai.com" {
            "max_completion_tokens"
        } else {
            "max_tokens"
        }
    }
}

fn chat_body(
    req: &ChatRequest,
    model: Option<&str>,
    max_tokens_field: &str,
    structured: StructuredOutput,
) -> Value {
    let mut body = Map::new();
    if let Some(m) = model {
        body.insert("model".into(), Value::String(m.to_owned()));
    }
    body.insert("messages".into(), messages_json(req));
    if let Some(t) = req.temperature {
        body.insert("temperature".into(), json!(t));
    }
    if let Some(n) = req.max_tokens {
        body.insert(max_tokens_field.into(), json!(n));
    }
    if let Some(format) = openai_response_format(structured, req.response_schema.as_ref()) {
        body.insert("response_format".into(), format);
    }
    body.insert("stream".into(), Value::Bool(false));
    Value::Object(body)
}

/// Parses a chat completion (`choices[0].message.content`).
pub(crate) fn parse_chat_completion(endpoint: &str, v: &Value) -> Result<ChatResponse, AiError> {
    let protocol = |message: String| AiError::Protocol {
        endpoint: endpoint.to_owned(),
        message,
    };
    if let Some(err) = v.get("error").filter(|e| !e.is_null()) {
        let msg = err
            .get("message")
            .and_then(Value::as_str)
            .map_or_else(|| err.to_string(), str::to_owned);
        return Err(protocol(format!("error: {}", truncate(&msg, 500))));
    }
    let choice = v
        .get("choices")
        .and_then(|c| c.get(0))
        .ok_or_else(|| protocol("no choices in the answer".into()))?;
    let message = choice.get("message").unwrap_or(&Value::Null);
    if let Some(refusal) = message.get("refusal").and_then(Value::as_str)
        && !refusal.trim().is_empty()
    {
        return Err(protocol(format!(
            "the model refused: {}",
            truncate(refusal, 500)
        )));
    }
    let content = message
        .get("content")
        .and_then(Value::as_str)
        .ok_or_else(|| protocol("the answer has no message content".into()))?;
    let usage = v.get("usage").unwrap_or(&Value::Null);
    Ok(ChatResponse {
        content: content.to_owned(),
        model: v.get("model").and_then(Value::as_str).map(str::to_owned),
        finish_reason: choice
            .get("finish_reason")
            .and_then(Value::as_str)
            .map(str::to_owned),
        usage: Usage {
            prompt_tokens: usage.get("prompt_tokens").and_then(Value::as_u64),
            completion_tokens: usage.get("completion_tokens").and_then(Value::as_u64),
        },
    })
}

impl Provider for OpenAiCompatible {
    fn id(&self) -> &str {
        &self.id
    }

    fn kind(&self) -> ProviderKind {
        ProviderKind::OpenAiCompatible
    }

    fn endpoint(&self) -> &Endpoint {
        &self.endpoint
    }

    fn http_request(&self, req: &ChatRequest) -> Result<HttpRequest, AiError> {
        let body = chat_body(
            req,
            Some(&self.model),
            self.max_tokens_field(),
            self.structured,
        );
        let mut http = HttpRequest::post_json(self.endpoint.join("chat/completions"), body);
        if let Some(key) = &self.key {
            http = http.header(
                "Authorization",
                HeaderValue::Secret {
                    prefix: "Bearer ",
                    secret: key.clone(),
                },
            );
        }
        Ok(http)
    }

    fn complete<'a>(
        &'a self,
        req: ChatRequest,
        cancel: CancellationToken,
    ) -> BoxFuture<'a, Result<ChatResponse, AiError>> {
        Box::pin(async move {
            let http = self.http_request(&req)?;
            let v = send_json(http, req.timeout, self.endpoint.is_loopback(), &cancel).await?;
            parse_chat_completion(&self.endpoint.base(), &v)
        })
    }
}

/// `POST {endpoint}/openai/deployments/{deployment}/chat/completions?api-version=…`
/// with an `api-key` header.
#[derive(Debug, Clone)]
pub struct AzureOpenAi {
    id: String,
    endpoint: Endpoint,
    deployment: String,
    api_version: String,
    key: Option<Secret>,
    structured: StructuredOutput,
}

impl AzureOpenAi {
    #[must_use]
    pub fn new(
        id: &str,
        endpoint: Endpoint,
        deployment: impl Into<String>,
        api_version: impl Into<String>,
        key: Option<Secret>,
    ) -> Self {
        Self {
            id: id.to_owned(),
            endpoint,
            deployment: deployment.into(),
            api_version: api_version.into(),
            key,
            structured: StructuredOutput::default(),
        }
    }

    #[must_use]
    pub fn with_structured_output(mut self, mode: StructuredOutput) -> Self {
        self.structured = mode;
        self
    }
}

fn path_segment(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
        .collect()
}

impl Provider for AzureOpenAi {
    fn id(&self) -> &str {
        &self.id
    }

    fn kind(&self) -> ProviderKind {
        ProviderKind::AzureOpenAi
    }

    fn endpoint(&self) -> &Endpoint {
        &self.endpoint
    }

    fn http_request(&self, req: &ChatRequest) -> Result<HttpRequest, AiError> {
        let url = format!(
            "{}?api-version={}",
            self.endpoint.join(&format!(
                "openai/deployments/{}/chat/completions",
                path_segment(&self.deployment)
            )),
            path_segment(&self.api_version)
        );
        let body = chat_body(req, None, "max_tokens", self.structured);
        let mut http = HttpRequest::post_json(url, body);
        if let Some(key) = &self.key {
            http = http.header(
                "api-key",
                HeaderValue::Secret {
                    prefix: "",
                    secret: key.clone(),
                },
            );
        }
        Ok(http)
    }

    fn complete<'a>(
        &'a self,
        req: ChatRequest,
        cancel: CancellationToken,
    ) -> BoxFuture<'a, Result<ChatResponse, AiError>> {
        Box::pin(async move {
            let http = self.http_request(&req)?;
            let v = send_json(http, req.timeout, self.endpoint.is_loopback(), &cancel).await?;
            parse_chat_completion(&self.endpoint.base(), &v)
        })
    }
}
