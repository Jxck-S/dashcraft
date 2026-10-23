//! Provider adapters against a fake HTTP server on 127.0.0.1: request
//! method, path, headers and body; response parsing; HTTP errors (redacted);
//! timeouts and cancellation. No real network.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use std::time::{Duration, Instant};

use common::{FakeServer, KEY};
use pbi_ai::{
    AiError, AzureOpenAi, ChatMessage, ChatRequest, Endpoint, Ollama, OpenAiCompatible, Provider,
    ResponseSchema, Secret, StructuredOutput,
};
use serde_json::json;
use tokio_util::sync::CancellationToken;

const COMPLETION: &str = r#"{"id":"c1","model":"gpt-test","choices":[{"index":0,"message":{"role":"assistant","content":"{\"answer\": 42}"},"finish_reason":"stop"}],"usage":{"prompt_tokens":12,"completion_tokens":3,"total_tokens":15}}"#;

fn request() -> ChatRequest {
    let mut req = ChatRequest::new(vec![
        ChatMessage::system("rules"),
        ChatMessage::user("question"),
    ]);
    req.temperature = Some(0.2);
    req.max_tokens = Some(256);
    req.response_schema = Some(ResponseSchema {
        name: "dax_explain".into(),
        schema: json!({"type": "object", "properties": {"answer": {"type": "integer"}}}),
    });
    req.timeout = Duration::from_secs(10);
    req
}

fn endpoint(server: &FakeServer, path: &str) -> Endpoint {
    Endpoint::parse(&format!("{}{path}", server.url())).unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn openai_compatible_posts_chat_completions_with_a_bearer_key() {
    let server = FakeServer::respond(200, COMPLETION);
    let p = OpenAiCompatible::new(
        "lmstudio",
        endpoint(&server, "/v1"),
        "gpt-test",
        Some(Secret::new(KEY)),
    );
    let out = p
        .complete(request(), CancellationToken::new())
        .await
        .unwrap();
    let rec = server.request();
    assert_eq!(rec.method, "POST");
    assert_eq!(rec.path, "/v1/chat/completions");
    assert_eq!(
        rec.header("authorization"),
        Some(format!("Bearer {KEY}").as_str())
    );
    assert_eq!(rec.header("content-type"), Some("application/json"));
    assert!(
        rec.header("user-agent")
            .is_some_and(|u| u.starts_with("dashcraft/"))
    );
    let b = &rec.body;
    assert_eq!(b["model"], "gpt-test");
    assert_eq!(
        b["messages"][0],
        json!({"role": "system", "content": "rules"})
    );
    assert_eq!(b["messages"][1]["role"], "user");
    assert_eq!(b["temperature"], json!(0.2));
    assert_eq!(b["max_tokens"], json!(256));
    assert_eq!(b["stream"], json!(false));
    assert_eq!(b["response_format"]["type"], "json_schema");
    assert_eq!(b["response_format"]["json_schema"]["name"], "dax_explain");
    assert_eq!(
        b["response_format"]["json_schema"]["schema"]["properties"]["answer"]["type"],
        "integer"
    );
    // The request it sent is exactly the one a preview shows.
    let preview = p.http_request(&request()).unwrap().preview();
    assert_eq!(&preview.body, b);
    assert_eq!(out.content, "{\"answer\": 42}");
    assert_eq!(out.model.as_deref(), Some("gpt-test"));
    assert_eq!(out.finish_reason.as_deref(), Some("stop"));
    assert_eq!(
        (out.usage.prompt_tokens, out.usage.completion_tokens),
        (Some(12), Some(3))
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn openai_compatible_without_a_key_sends_no_authorization_and_json_object_mode() {
    let server = FakeServer::respond(200, COMPLETION);
    let p = OpenAiCompatible::new("local", endpoint(&server, "/v1"), "m", None)
        .with_structured_output(StructuredOutput::JsonObject);
    p.complete(request(), CancellationToken::new())
        .await
        .unwrap();
    let rec = server.request();
    assert_eq!(rec.header("authorization"), None);
    assert_eq!(rec.body["response_format"], json!({"type": "json_object"}));
}

#[tokio::test(flavor = "multi_thread")]
async fn azure_uses_the_deployment_path_api_version_and_api_key_header() {
    let server = FakeServer::respond(200, COMPLETION);
    let p = AzureOpenAi::new(
        "azure",
        endpoint(&server, ""),
        "gpt4o-prod",
        "2024-10-21",
        Some(Secret::new(KEY)),
    );
    let out = p
        .complete(request(), CancellationToken::new())
        .await
        .unwrap();
    let rec = server.request();
    assert_eq!(rec.method, "POST");
    assert_eq!(
        rec.path,
        "/openai/deployments/gpt4o-prod/chat/completions?api-version=2024-10-21"
    );
    assert_eq!(rec.header("api-key"), Some(KEY));
    assert_eq!(rec.header("authorization"), None);
    assert!(
        rec.body.get("model").is_none(),
        "the deployment names the model"
    );
    assert_eq!(rec.body["max_tokens"], json!(256));
    assert_eq!(rec.body["response_format"]["type"], "json_schema");
    assert_eq!(out.content, "{\"answer\": 42}");
}

#[tokio::test(flavor = "multi_thread")]
async fn ollama_posts_api_chat_without_streaming_or_keys() {
    let server = FakeServer::respond(
        200,
        r#"{"model":"llama3.2","created_at":"2026-10-22T00:00:00Z","message":{"role":"assistant","content":"{\"answer\": 7}"},"done":true,"done_reason":"stop","prompt_eval_count":20,"eval_count":5}"#,
    );
    let p = Ollama::new("ollama", endpoint(&server, ""), "llama3.2");
    let out = p
        .complete(request(), CancellationToken::new())
        .await
        .unwrap();
    let rec = server.request();
    assert_eq!(rec.method, "POST");
    assert_eq!(rec.path, "/api/chat");
    assert_eq!(rec.header("authorization"), None);
    assert_eq!(rec.header("api-key"), None);
    assert_eq!(rec.body["model"], "llama3.2");
    assert_eq!(rec.body["stream"], json!(false));
    assert_eq!(rec.body["format"]["type"], "object");
    assert_eq!(
        rec.body["options"],
        json!({"temperature": 0.2, "num_predict": 256})
    );
    assert_eq!(rec.body["messages"][1]["content"], "question");
    assert_eq!(out.content, "{\"answer\": 7}");
    assert_eq!(out.finish_reason.as_deref(), Some("stop"));
    assert_eq!(
        (out.usage.prompt_tokens, out.usage.completion_tokens),
        (Some(20), Some(5))
    );
    assert!(p.endpoint().is_local());
}

#[tokio::test(flavor = "multi_thread")]
async fn http_errors_keep_the_status_and_never_the_key() {
    let body = format!(
        r#"{{"error":{{"message":"Incorrect API key provided: {KEY}. Header was Authorization: Bearer {KEY}","type":"invalid_request_error"}}}}"#
    );
    let server = FakeServer::respond(401, &body);
    let p = OpenAiCompatible::new("x", endpoint(&server, "/v1"), "m", Some(Secret::new(KEY)));
    let err = p
        .complete(request(), CancellationToken::new())
        .await
        .unwrap_err();
    let text = err.to_string();
    assert!(matches!(err, AiError::Http { status: 401, .. }), "{err:?}");
    assert!(!text.contains(KEY), "{text}");
    assert!(
        text.contains("Incorrect API key") && text.contains("[redacted]"),
        "{text}"
    );
    let op: pbi_ops::OpError = err.into();
    assert!(!serde_json::to_string(&op).unwrap().contains(KEY));
}

#[tokio::test(flavor = "multi_thread")]
async fn non_json_answers_are_protocol_errors() {
    let server = FakeServer::respond(200, "<html>proxy login</html>");
    let p = Ollama::new("o", endpoint(&server, ""), "m");
    let err = p
        .complete(request(), CancellationToken::new())
        .await
        .unwrap_err();
    assert!(matches!(err, AiError::Protocol { .. }), "{err:?}");
    let server = FakeServer::respond(200, r#"{"choices": []}"#);
    let p = OpenAiCompatible::new("x", endpoint(&server, "/v1"), "m", None);
    let err = p
        .complete(request(), CancellationToken::new())
        .await
        .unwrap_err();
    assert!(err.to_string().contains("no choices"), "{err}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_silent_server_times_out() {
    let server = FakeServer::silent(Duration::from_secs(5));
    let p = Ollama::new("o", endpoint(&server, ""), "m");
    let mut req = request();
    req.timeout = Duration::from_millis(400);
    let started = Instant::now();
    let err = p.complete(req, CancellationToken::new()).await.unwrap_err();
    assert!(matches!(err, AiError::Timeout { .. }), "{err:?}");
    assert!(
        started.elapsed() < Duration::from_secs(4),
        "{:?}",
        started.elapsed()
    );
    let op: pbi_ops::OpError = err.into();
    assert_eq!(op.code, pbi_ops::OpErrorCode::Io);
}

#[tokio::test(flavor = "multi_thread")]
async fn cancelling_stops_waiting_at_once() {
    let server = FakeServer::silent(Duration::from_secs(10));
    let p = OpenAiCompatible::new("x", endpoint(&server, "/v1"), "m", Some(Secret::new(KEY)));
    let mut req = request();
    req.timeout = Duration::from_secs(60);
    let cancel = CancellationToken::new();
    let trigger = cancel.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(200)).await;
        trigger.cancel();
    });
    let started = Instant::now();
    let err = p.complete(req, cancel).await.unwrap_err();
    assert!(matches!(err, AiError::Cancelled), "{err:?}");
    assert!(
        started.elapsed() < Duration::from_secs(3),
        "{:?}",
        started.elapsed()
    );
    // The request did reach the server before the cancellation.
    assert_eq!(server.request().path, "/v1/chat/completions");
}

#[tokio::test(flavor = "multi_thread")]
async fn unreachable_endpoints_are_connection_errors() {
    // Bind and drop a listener: nothing accepts on that port any more.
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let p = Ollama::new(
        "o",
        Endpoint::parse(&format!("http://127.0.0.1:{port}")).unwrap(),
        "m",
    );
    let err = p
        .complete(request(), CancellationToken::new())
        .await
        .unwrap_err();
    assert!(
        matches!(err, AiError::Connection { .. } | AiError::Timeout { .. }),
        "{err:?}"
    );
}
