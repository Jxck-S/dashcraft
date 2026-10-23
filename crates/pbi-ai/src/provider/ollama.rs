//! [`Ollama`]: `POST {endpoint}/api/chat` (non-streaming), no key.

use pbi_ops::BoxFuture;
use serde_json::{Map, Value, json};
use tokio_util::sync::CancellationToken;

use super::http::{HttpRequest, send_json};
use super::{ChatRequest, ChatResponse, Provider, Usage, messages_json};
use crate::endpoint::Endpoint;
use crate::error::{AiError, truncate};
use crate::settings::{ProviderKind, StructuredOutput};

/// A local (or remote) Ollama server.
#[derive(Debug, Clone)]
pub struct Ollama {
    id: String,
    endpoint: Endpoint,
    model: String,
    structured: StructuredOutput,
}

impl Ollama {
    #[must_use]
    pub fn new(id: &str, endpoint: Endpoint, model: impl Into<String>) -> Self {
        Self {
            id: id.to_owned(),
            endpoint,
            model: model.into(),
            structured: StructuredOutput::default(),
        }
    }

    #[must_use]
    pub fn with_structured_output(mut self, mode: StructuredOutput) -> Self {
        self.structured = mode;
        self
    }
}

fn parse(endpoint: &str, v: &Value) -> Result<ChatResponse, AiError> {
    let protocol = |message: String| AiError::Protocol {
        endpoint: endpoint.to_owned(),
        message,
    };
    if let Some(err) = v.get("error").and_then(Value::as_str) {
        return Err(protocol(format!("error: {}", truncate(err, 500))));
    }
    let content = v
        .get("message")
        .and_then(|m| m.get("content"))
        .and_then(Value::as_str)
        .ok_or_else(|| protocol("the answer has no message content".into()))?;
    Ok(ChatResponse {
        content: content.to_owned(),
        model: v.get("model").and_then(Value::as_str).map(str::to_owned),
        finish_reason: v
            .get("done_reason")
            .and_then(Value::as_str)
            .map(str::to_owned),
        usage: Usage {
            prompt_tokens: v.get("prompt_eval_count").and_then(Value::as_u64),
            completion_tokens: v.get("eval_count").and_then(Value::as_u64),
        },
    })
}

impl Provider for Ollama {
    fn id(&self) -> &str {
        &self.id
    }

    fn kind(&self) -> ProviderKind {
        ProviderKind::Ollama
    }

    fn endpoint(&self) -> &Endpoint {
        &self.endpoint
    }

    fn http_request(&self, req: &ChatRequest) -> Result<HttpRequest, AiError> {
        let mut body = Map::new();
        body.insert("model".into(), Value::String(self.model.clone()));
        body.insert("messages".into(), messages_json(req));
        body.insert("stream".into(), Value::Bool(false));
        if let Some(schema) = &req.response_schema {
            match self.structured {
                StructuredOutput::JsonSchema => {
                    body.insert("format".into(), schema.schema.clone());
                }
                StructuredOutput::JsonObject => {
                    body.insert("format".into(), Value::String("json".into()));
                }
                StructuredOutput::None => {}
            }
        }
        let mut options = Map::new();
        if let Some(t) = req.temperature {
            options.insert("temperature".into(), json!(t));
        }
        if let Some(n) = req.max_tokens {
            options.insert("num_predict".into(), json!(n));
        }
        if !options.is_empty() {
            body.insert("options".into(), Value::Object(options));
        }
        Ok(HttpRequest::post_json(
            self.endpoint.join("api/chat"),
            Value::Object(body),
        ))
    }

    fn complete<'a>(
        &'a self,
        req: ChatRequest,
        cancel: CancellationToken,
    ) -> BoxFuture<'a, Result<ChatResponse, AiError>> {
        Box::pin(async move {
            let http = self.http_request(&req)?;
            let v = send_json(http, req.timeout, self.endpoint.is_loopback(), &cancel).await?;
            parse(&self.endpoint.base(), &v)
        })
    }
}
