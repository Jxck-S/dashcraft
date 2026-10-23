//! The HTTP request model (with secrets kept apart for previews) and the
//! blocking `ureq` transport, run on a detached thread and awaited
//! cancellably.

use std::time::{Duration, Instant};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio_util::sync::CancellationToken;

use crate::error::{AiError, truncate};
use crate::secrets::{REDACTED, Redactor, Secret};

/// Largest answer body read (4 MiB).
pub const MAX_RESPONSE_BYTES: u64 = 4 << 20;
/// `User-Agent` of every request (no machine or user details).
pub const USER_AGENT: &str = concat!("dashcraft/", env!("CARGO_PKG_VERSION"));
/// Longest HTTP error body kept in an error.
const MAX_ERROR_BODY_CHARS: usize = 2000;
/// Upper bound of the connection phase.
const MAX_CONNECT: Duration = Duration::from_secs(10);

/// A header value; secrets are only exposed to the transport.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HeaderValue {
    Plain(String),
    /// `prefix` + the secret, e.g. `Bearer ` + key.
    Secret {
        prefix: &'static str,
        secret: Secret,
    },
}

impl HeaderValue {
    fn wire(&self) -> String {
        match self {
            HeaderValue::Plain(v) => v.clone(),
            HeaderValue::Secret { prefix, secret } => format!("{prefix}{}", secret.expose()),
        }
    }

    fn shown(&self) -> String {
        match self {
            HeaderValue::Plain(v) => v.clone(),
            HeaderValue::Secret { prefix, .. } => format!("{prefix}{REDACTED}"),
        }
    }

    pub(crate) fn secret(&self) -> Option<&Secret> {
        match self {
            HeaderValue::Secret { secret, .. } => Some(secret),
            HeaderValue::Plain(_) => None,
        }
    }
}

/// An HTTP request as an adapter sends it.
#[derive(Debug, Clone, PartialEq)]
pub struct HttpRequest {
    /// `POST` (or `MOCK` for the mock provider).
    pub method: &'static str,
    pub url: String,
    pub headers: Vec<(String, HeaderValue)>,
    /// The JSON body; sent as `serde_json::to_vec(&body)`.
    pub body: Value,
}

/// One header of a preview.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct HeaderPreview {
    pub name: String,
    /// The value, with keys replaced by `[redacted]`.
    pub value: String,
}

/// An [`HttpRequest`] with its secrets redacted (`ai.preview`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct HttpPreview {
    pub method: String,
    pub url: String,
    pub headers: Vec<HeaderPreview>,
    /// The exact JSON body.
    pub body: Value,
    /// Size of the body as sent, in bytes.
    pub body_bytes: usize,
}

impl HttpRequest {
    /// A `POST` with a JSON body.
    #[must_use]
    pub fn post_json(url: String, body: Value) -> Self {
        Self {
            method: "POST",
            url,
            headers: vec![
                (
                    "Content-Type".into(),
                    HeaderValue::Plain("application/json".into()),
                ),
                (
                    "Accept".into(),
                    HeaderValue::Plain("application/json".into()),
                ),
                ("User-Agent".into(), HeaderValue::Plain(USER_AGENT.into())),
            ],
            body,
        }
    }

    /// Adds a header.
    #[must_use]
    pub fn header(mut self, name: &str, value: HeaderValue) -> Self {
        self.headers.push((name.to_owned(), value));
        self
    }

    /// The bytes sent as the body.
    #[must_use]
    pub fn body_bytes(&self) -> Vec<u8> {
        serde_json::to_vec(&self.body).unwrap_or_default()
    }

    /// The request with secrets redacted.
    #[must_use]
    pub fn preview(&self) -> HttpPreview {
        HttpPreview {
            method: self.method.to_owned(),
            url: self.url.clone(),
            headers: self
                .headers
                .iter()
                .map(|(name, v)| HeaderPreview {
                    name: name.clone(),
                    value: v.shown(),
                })
                .collect(),
            body: self.body.clone(),
            body_bytes: self.body_bytes().len(),
        }
    }

    /// A redactor for the secrets this request carries.
    #[must_use]
    pub fn redactor(&self) -> Redactor {
        self.headers
            .iter()
            .fold(Redactor::default(), |r, (_, v)| r.with(v.secret()))
    }

    /// Scheme + host + port of the URL, for errors.
    fn origin(&self) -> String {
        let after = self.url.find("://").map_or(0, |i| i + 3);
        let end = self
            .url
            .get(after..)
            .and_then(|rest| rest.find('/'))
            .map_or(self.url.len(), |i| after + i);
        self.url.get(..end).unwrap_or(&self.url).to_owned()
    }
}

/// Sends `req` on its own thread and parses the JSON answer. Waits at most
/// `timeout` (plus scheduling); `cancel` stops waiting immediately.
///
/// The blocking call runs on a detached thread rather than Tokio's blocking
/// pool: a runtime waits for its blocking tasks when it shuts down, so a
/// cancelled request would otherwise keep a CLI process alive until the
/// request's own timeout. The abandoned thread ends at that timeout.
///
/// # Errors
/// [`AiError::Cancelled`], [`AiError::Timeout`], [`AiError::Connection`],
/// [`AiError::Http`] (non-2xx, body truncated and redacted),
/// [`AiError::Protocol`] (not JSON, too large).
pub(crate) async fn send_json(
    req: HttpRequest,
    timeout: Duration,
    local: bool,
    cancel: &CancellationToken,
) -> Result<Value, AiError> {
    if cancel.is_cancelled() {
        return Err(AiError::Cancelled);
    }
    let redactor = req.redactor();
    let origin = req.origin();
    let (tx, rx) = tokio::sync::oneshot::channel();
    std::thread::Builder::new()
        .name("dashcraft-ai-http".into())
        .spawn(move || {
            let _ = tx.send(send_blocking(&req, timeout, local));
        })
        .map_err(|e| AiError::Internal(format!("cannot start the AI request thread: {e}")))?;
    let result = tokio::select! {
        biased;
        () = cancel.cancelled() => return Err(AiError::Cancelled),
        received = rx => received
            .map_err(|_| AiError::Internal("the AI request thread stopped unexpectedly".into()))?,
    };
    let (status, body) = result.map_err(|e| e.into_ai(&origin, timeout, &redactor))?;
    if !(200..300).contains(&status) {
        return Err(AiError::Http {
            endpoint: origin,
            status,
            body: truncate(&redactor.redact(body.trim()), MAX_ERROR_BODY_CHARS),
        });
    }
    serde_json::from_str(&body).map_err(|e| AiError::Protocol {
        endpoint: origin,
        message: format!(
            "the answer is not JSON ({e}): {}",
            truncate(&redactor.redact(body.trim()), 300)
        ),
    })
}

enum SendError {
    Timeout,
    TooLarge,
    Other(String),
}

impl SendError {
    fn into_ai(self, origin: &str, timeout: Duration, redactor: &Redactor) -> AiError {
        match self {
            SendError::Timeout => AiError::Timeout {
                endpoint: origin.to_owned(),
                millis: u64::try_from(timeout.as_millis()).unwrap_or(u64::MAX),
            },
            SendError::TooLarge => AiError::Protocol {
                endpoint: origin.to_owned(),
                message: format!("the answer is larger than {MAX_RESPONSE_BYTES} bytes"),
            },
            SendError::Other(message) => AiError::Connection {
                endpoint: origin.to_owned(),
                message: redactor.redact(&message),
            },
        }
    }
}

fn classify(e: ureq::Error) -> SendError {
    match e {
        ureq::Error::Timeout(_) => SendError::Timeout,
        ureq::Error::BodyExceedsLimit(_) => SendError::TooLarge,
        ureq::Error::Io(io)
            if matches!(
                io.kind(),
                std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
            ) =>
        {
            SendError::Timeout
        }
        other => SendError::Other(other.to_string()),
    }
}

fn send_blocking(
    req: &HttpRequest,
    timeout: Duration,
    local: bool,
) -> Result<(u16, String), SendError> {
    let started = Instant::now();
    let mut config = ureq::Agent::config_builder()
        .http_status_as_error(false)
        .max_redirects(0)
        .user_agent(USER_AGENT)
        .timeout_global(Some(timeout))
        .timeout_connect(Some(timeout.min(MAX_CONNECT)))
        .timeout_recv_response(Some(timeout))
        .timeout_recv_body(Some(timeout));
    if local {
        config = config.proxy(None);
    }
    let agent: ureq::Agent = config.build().into();
    let mut builder = agent.post(&req.url);
    for (name, value) in &req.headers {
        if name.eq_ignore_ascii_case("user-agent") {
            continue;
        }
        builder = builder.header(name.as_str(), value.wire());
    }
    let body = req.body_bytes();
    let mut response = builder.send(&body[..]).map_err(classify)?;
    let status = response.status().as_u16();
    let text = response
        .body_mut()
        .with_config()
        .limit(MAX_RESPONSE_BYTES)
        .lossy_utf8(true)
        .read_to_string()
        .map_err(classify)?;
    tracing::debug!(
        status,
        bytes = text.len(),
        ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
        "AI provider answered"
    );
    Ok((status, text))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn previews_redact_secret_headers() {
        let req = HttpRequest::post_json(
            "https://h.example/v1/chat".into(),
            serde_json::json!({"a": 1}),
        )
        .header(
            "Authorization",
            HeaderValue::Secret {
                prefix: "Bearer ",
                secret: Secret::new("sk-123456"),
            },
        );
        let p = req.preview();
        let text = serde_json::to_string(&p).unwrap();
        assert!(!text.contains("sk-123456"), "{text}");
        assert!(text.contains("Bearer [redacted]"));
        assert_eq!(p.body_bytes, 7);
        assert_eq!(req.origin(), "https://h.example");
        assert_eq!(req.redactor().redact("x sk-123456"), "x [redacted]");
    }
}
