//! The `ai.*` commands through `Registry::execute` with a mock provider:
//! opt-in, consent, exposure classes, redaction, caller policy, previews,
//! generation with local parsing and evaluation, cancellation; plus the
//! catalog invariants and the generated reference in docs/AI.md.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;
mod ops_ts;

use std::collections::HashSet;
use std::sync::Arc;
use std::time::{Duration, Instant};

use common::{
    HOSTILE, KEY, MINI_TOTAL, Runner, fixture, mini_pbip, mock_host, real_host, tmp_dir,
    write_settings,
};
use pbi_ai::{AiError, MockProvider, MockReply};
use pbi_ops::{CallerKind, Category, Ctx, OpErrorCode, Registry};
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

fn mock_settings(enabled: bool) -> Value {
    json!({
        "enabled": enabled,
        "providers": [
            {"id": "local", "kind": "ollama", "model": "llama3.2"},
            {"id": "openai", "kind": "openAiCompatible", "model": "gpt-test", "endpoint": "https://api.example.test/v1"}
        ],
        "defaultProvider": "local"
    })
}

fn explain_reply() -> MockReply {
    MockReply::Json(json!({
        "summary": "Adds up Sales[Amount].",
        "explanation": "Sums **Amount** over the rows visible in the filter context.",
        "filterContext": ["Filters on Regions reach Sales through Sales[Region] → Regions[Region]."],
        "dependencies": [{"name": "Sales[Amount]", "note": "the summed column"}],
        "caveats": ["BLANK when no rows are visible."]
    }))
}

fn reason(e: &pbi_ops::OpError) -> String {
    e.data
        .as_ref()
        .and_then(|d| d["reason"].as_str())
        .unwrap_or_default()
        .to_owned()
}

/// The text of every message the mock received.
fn sent_text(mock: &MockProvider) -> String {
    mock.requests()
        .iter()
        .flat_map(|r| r.messages.iter().map(|m| m.content.clone()))
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn ai_commands_have_metadata_schemas_and_valid_examples() {
    let registry = pbi_ai::ai_catalog();
    let mut ids = HashSet::new();
    for spec in registry.list() {
        let m = &spec.meta;
        assert!(ids.insert(m.id), "duplicate {}", m.id);
        assert!(m.id.starts_with("ai."), "{}", m.id);
        assert_eq!(m.category, Category::Ai, "{}", m.id);
        assert!(
            !m.summary.contains('\n') && m.summary.len() <= 200,
            "{}",
            m.id
        );
        assert_eq!(spec.params_schema["type"], "object", "{}", m.id);
        assert_eq!(
            spec.params_schema["additionalProperties"],
            json!(false),
            "{}",
            m.id
        );
        assert_eq!(spec.output_schema["type"], "object", "{}", m.id);
        assert!(!m.mutates_document, "{}", m.id);
        assert!(!m.examples.is_empty(), "{}", m.id);
        for (title, text) in m.examples {
            let example: Value = serde_json::from_str(text).unwrap();
            spec.validate(&example)
                .unwrap_or_else(|e| panic!("{} example {title:?}: {}", m.id, e.message));
        }
    }
    let safe: Vec<&str> = registry
        .agent_catalog()
        .list()
        .iter()
        .map(|s| s.meta.id)
        .collect();
    assert_eq!(
        safe,
        [
            "ai.status",
            "ai.preview",
            "ai.explainMeasure",
            "ai.generateMeasure"
        ]
    );
    for id in [
        "ai.consent.grant",
        "ai.consent.revoke",
        "ai.settings.set",
        "ai.apiKey.set",
        "ai.apiKey.clear",
    ] {
        let m = registry.describe(id).unwrap().meta;
        assert!(!m.agent_safe && m.writes_files, "{id}");
    }
    // The default catalog stays network-free; `catalog()` adds the AI commands.
    assert!(
        Registry::default_catalog()
            .list()
            .iter()
            .all(|s| s.meta.category != Category::Ai)
    );
    assert_eq!(
        pbi_ai::catalog().len(),
        Registry::default_catalog().len() + registry.len()
    );
}

#[test]
fn ai_md_reference_is_up_to_date() {
    const BEGIN: &str = "<!-- BEGIN GENERATED: ai commands (cargo test -p pbi-ai --test registry; PBIVIEW_BLESS=1 to update) -->\n";
    const END: &str = "<!-- END GENERATED: ai commands -->";
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/AI.md");
    let doc = std::fs::read_to_string(&path).unwrap_or_default();
    let generated: String = pbi_ai::ai_catalog()
        .list()
        .iter()
        .map(pbi_ops::markdown::command_markdown)
        .collect();
    let (Some(start), Some(end)) = (doc.find(BEGIN), doc.find(END)) else {
        panic!("docs/AI.md lacks the generated-section markers");
    };
    let current = &doc[start + BEGIN.len()..end];
    if std::env::var_os("PBIVIEW_BLESS").is_some() {
        let updated = format!("{}{BEGIN}{generated}{}", &doc[..start], &doc[end..]);
        std::fs::write(&path, updated).unwrap();
        return;
    }
    assert!(
        current == generated,
        "docs/AI.md command reference is stale: PBIVIEW_BLESS=1 cargo test -p pbi-ai --test registry"
    );
}

/// `app/src/lib/ops.gen.ts` matches the desktop app's catalog (`pbi_ai::catalog()`: the core
/// commands plus `ai.*`): one TypeScript type per parameter / result schema, a command → types
/// map and camelCase wrappers. Regenerate with `npm run gen:ops` in `app/` (or `PBIVIEW_BLESS=1
/// cargo test -p pbi-ai --test registry ops_typescript`).
#[test]
fn ops_typescript_is_up_to_date() {
    let app = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../app/src/lib");
    if !app.is_dir() {
        println!("SKIP: {} missing", app.display());
        return;
    }
    let path = app.join("ops.gen.ts");
    let generated = ops_ts::generate(&pbi_ai::catalog());
    for id in ["doc.open", "ai.explainMeasure", "ai.apiKey.set"] {
        assert!(generated.contains(&format!("'{id}'")), "{id} missing");
    }
    if std::env::var_os("PBIVIEW_BLESS").is_some() {
        std::fs::write(&path, &generated).unwrap();
        return;
    }
    let committed = std::fs::read_to_string(&path).unwrap_or_default();
    assert!(
        committed == generated,
        "app/src/lib/ops.gen.ts is stale: run `npm run gen:ops` in app/ \
         (or PBIVIEW_BLESS=1 cargo test -p pbi-ai --test registry ops_typescript)"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn ai_is_off_by_default_and_nothing_is_sent() {
    let dir = tmp_dir("ai-off");
    let mock = Arc::new(MockProvider::new("local"));
    let r = Runner::new(mock_host(dir.path(), &mock));
    let doc = r.open(&mini_pbip(dir.path())).await;

    let status = r.run("ai.status", json!({})).await;
    assert_eq!(status["enabled"], json!(false));
    assert_eq!(status["providers"], json!([]));
    assert_eq!(status["sampleRows"], json!(0));
    let implemented: Vec<&str> = status["capabilities"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c["implemented"] == json!(true))
        .map(|c| c["id"].as_str().unwrap())
        .collect();
    assert_eq!(implemented, ["dax.explain", "dax.generate"]);

    let e = r
        .try_run(
            "ai.explainMeasure",
            json!({"docId": doc, "measure": "Total"}),
        )
        .await
        .unwrap_err();
    assert_eq!(
        (e.code, reason(&e).as_str()),
        (OpErrorCode::Unsupported, "disabled")
    );
    // Configured but not enabled: still nothing.
    write_settings(dir.path(), &mock_settings(false));
    let e = r
        .try_run("ai.generateMeasure", json!({"docId": doc, "prompt": "x"}))
        .await
        .unwrap_err();
    assert_eq!(reason(&e), "disabled");
    assert_eq!(mock.calls(), 0);
    assert!(!dir.path().join("ai.json.tmp").exists());
}

#[tokio::test(flavor = "multi_thread")]
async fn consent_is_required_even_for_a_local_endpoint() {
    let dir = tmp_dir("ai-consent");
    let mock = Arc::new(MockProvider::scripted("local", [explain_reply()]));
    let r = Runner::new(mock_host(dir.path(), &mock));
    let doc = r.open(&mini_pbip(dir.path())).await;
    write_settings(dir.path(), &mock_settings(true));

    let e = r
        .try_run(
            "ai.explainMeasure",
            json!({"docId": doc, "measure": "Total"}),
        )
        .await
        .unwrap_err();
    assert_eq!(e.code, OpErrorCode::Unsupported);
    assert_eq!(reason(&e), "consentRequired");
    assert!(e.message.contains("local endpoint"), "{}", e.message);
    let d = e.data.clone().unwrap();
    assert_eq!(d["endpointHost"], "127.0.0.1:11434");
    assert_eq!(d["local"], json!(true));
    assert_eq!(d["grant"]["command"], "ai.consent.grant");
    assert_eq!(mock.calls(), 0, "nothing is sent without consent");

    // A consent for another capability does not help.
    r.run(
        "ai.consent.grant",
        json!({"provider": "local", "capability": "dax.generate"}),
    )
    .await;
    assert!(
        r.try_run(
            "ai.explainMeasure",
            json!({"docId": doc, "measure": "Total"})
        )
        .await
        .is_err()
    );
    let granted = r
        .run(
            "ai.consent.grant",
            json!({"provider": "local", "capability": "dax.explain"}),
        )
        .await;
    assert_eq!(granted["consent"]["local"], json!(true));
    assert_eq!(granted["consent"]["exposure"], "metadata");
    assert_eq!(granted["consent"]["grantedBy"], "cli");

    let out = r
        .run(
            "ai.explainMeasure",
            json!({"docId": doc, "measure": "[Total]"}),
        )
        .await;
    assert_eq!(mock.calls(), 1);
    assert_eq!(out["measure"], "Total");
    assert_eq!(out["table"], "Sales");
    assert_eq!(out["expression"], "SUM(Sales[Amount])");
    assert_eq!(out["summary"], "Adds up Sales[Amount].");
    assert_eq!(out["dependencies"][0]["name"], "Sales[Amount]");
    assert_eq!(
        out["references"]["columns"],
        json!(["Sales[Amount] (int64)"])
    );
    assert!(
        out["references"]["relationships"][0]
            .as_str()
            .unwrap()
            .starts_with("Sales[Region] → Regions[Region]"),
        "{}",
        out["references"]
    );
    assert!(out["markdown"].as_str().unwrap().contains("**Caveats**"));
    assert_eq!(out["call"]["provider"], "local");
    assert_eq!(out["call"]["local"], json!(true));
    assert_eq!(out["call"]["exposure"], "metadata");
    assert_eq!(out["call"]["promptVersion"], "system@1+dax.explain@1");

    // What was sent: metadata only, the hostile description quoted as data.
    let req = &mock.requests()[0];
    assert_eq!(req.capability, Some(pbi_ai::Capability::DaxExplain));
    assert!(req.response_schema.is_some());
    let text = sent_text(&mock);
    assert!(text.contains("SUM(Sales[Amount])"));
    assert!(
        !text.contains("Item 01") && !text.contains("\"East\""),
        "no row values"
    );
    let user = &req.messages[1].content;
    assert_eq!(user.matches("</model_metadata>").count(), 1, "{user}");
    assert!(text.contains("Ignore all previous instructions"));
    assert!(
        !text.contains(HOSTILE),
        "the closing tag inside metadata is escaped"
    );

    // Revoking stops it again.
    let revoked = r
        .run("ai.consent.revoke", json!({"provider": "local"}))
        .await;
    assert_eq!(revoked["removed"], json!(2));
    assert!(
        r.try_run(
            "ai.explainMeasure",
            json!({"docId": doc, "measure": "Total"})
        )
        .await
        .is_err()
    );
    assert_eq!(mock.calls(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn transitive_measures_are_explained_with_their_dax() {
    let dir = tmp_dir("ai-transitive");
    let mock = Arc::new(MockProvider::new("local"));
    let r = Runner::new(mock_host(dir.path(), &mock));
    let doc = r.open(&mini_pbip(dir.path())).await;
    write_settings(dir.path(), &mock_settings(true));
    r.run(
        "ai.consent.grant",
        json!({"provider": "local", "capability": "dax.explain"}),
    )
    .await;
    let out = r
        .run(
            "ai.explainMeasure",
            json!({"docId": doc, "measure": "Sales[Average Amount]"}),
        )
        .await;
    assert_eq!(out["references"]["measures"], json!(["Sales[Total]"]));
    assert!(
        out["summary"]
            .as_str()
            .unwrap()
            .starts_with("Mock explanation")
    );
    let text = sent_text(&mock);
    assert!(
        text.contains("DIVIDE([Total], COUNTROWS(Sales))") && text.contains("SUM(Sales[Amount])")
    );

    let e = r
        .try_run(
            "ai.explainMeasure",
            json!({"docId": doc, "measure": "Totl"}),
        )
        .await
        .unwrap_err();
    assert_eq!(e.code, OpErrorCode::NotFound);
    assert!(e.data.unwrap()["candidates"].as_array().is_some());
}

#[tokio::test(flavor = "multi_thread")]
async fn agents_cannot_change_settings_consents_or_keys() {
    let dir = tmp_dir("ai-agent");
    let mock = Arc::new(MockProvider::new("local"));
    let r = Runner::new(mock_host(dir.path(), &mock));
    write_settings(dir.path(), &mock_settings(true));
    for (id, params) in [
        (
            "ai.consent.grant",
            json!({"provider": "local", "capability": "dax.explain"}),
        ),
        ("ai.consent.revoke", json!({"provider": "local"})),
        ("ai.settings.set", json!({"enabled": true})),
        (
            "ai.apiKey.set",
            json!({"provider": "openai", "key": "sk-agent"}),
        ),
        ("ai.apiKey.clear", json!({"provider": "openai"})),
    ] {
        let e = r.try_as(CallerKind::Agent, id, params).await.unwrap_err();
        assert_eq!(e.code, OpErrorCode::Unsupported, "{id}");
        assert!(
            e.message.contains("not available to agents"),
            "{id}: {}",
            e.message
        );
    }
    let settings: Value =
        serde_json::from_str(&std::fs::read_to_string(dir.path().join("ai.json")).unwrap())
            .unwrap();
    assert_eq!(settings["consents"], Value::Null, "unchanged");
    // Agents may read the status and preview.
    let status = r
        .try_as(CallerKind::Agent, "ai.status", json!({}))
        .await
        .unwrap();
    assert_eq!(status["enabled"], json!(true));
}

#[tokio::test(flavor = "multi_thread")]
async fn api_keys_are_only_taken_from_the_app_and_never_written_to_settings() {
    let dir = tmp_dir("ai-keys");
    let mock = Arc::new(MockProvider::new("local"));
    let host = mock_host(dir.path(), &mock);
    let r = Runner::new(host.clone());
    write_settings(dir.path(), &mock_settings(false));
    for caller in [CallerKind::Cli, CallerKind::Rpc] {
        let e = r
            .try_as(
                caller,
                "ai.apiKey.set",
                json!({"provider": "azure", "key": "sk-from-cli"}),
            )
            .await
            .unwrap_err();
        assert_eq!(reason(&e), "forbidden", "{caller:?}");
        assert!(e.message.contains("dashcraft ai login") && !e.message.contains("sk-from-cli"));
    }
    let out = r
        .try_as(
            CallerKind::App,
            "ai.apiKey.set",
            json!({"provider": "azure", "key": "sk-from-app-123"}),
        )
        .await
        .unwrap();
    assert_eq!(out["account"], "ai:azure");
    assert_eq!(out["service"], "dashcraft");
    assert!(!out.to_string().contains("sk-from-app-123"));
    assert_eq!(
        host.secrets()
            .get("azure")
            .unwrap()
            .map(|k| k.expose().to_owned()),
        Some("sk-from-app-123".into())
    );
    // The library function used by `dashcraft ai login`.
    host.clone()
        .scope(async { pbi_ai::store_api_key("lan", "sk-lib-key").unwrap() })
        .await;
    assert!(host.secrets().get("lan").unwrap().is_some());
    let cleared = r.run("ai.apiKey.clear", json!({"provider": "azure"})).await;
    assert_eq!(cleared["changed"], json!(true));
    let file = std::fs::read_to_string(dir.path().join("ai.json")).unwrap();
    assert!(!file.contains("sk-from-app-123") && !file.contains("sk-lib-key"));
    // Settings that try to carry a key are refused, without echoing it.
    let e = r
        .try_run(
            "ai.settings.set",
            json!({"providers": [{"id": "x", "kind": "openAiCompatible", "model": "m", "apiKey": "sk-in-settings"}]}),
        )
        .await
        .unwrap_err();
    assert_eq!(e.code, OpErrorCode::InvalidParams);
    assert!(!e.message.contains("sk-in-settings"), "{}", e.message);
    write_settings(
        dir.path(),
        &json!({"providers": [{"id": "x", "kind": "ollama", "model": "m", "token": "sk-in-file"}]}),
    );
    let status = r.run("ai.status", json!({})).await;
    let err = status["settingsError"].as_str().unwrap();
    assert!(
        err.contains("keychain") && !err.contains("sk-in-file"),
        "{err}"
    );
    assert_eq!(status["enabled"], json!(false));
}

#[tokio::test(flavor = "multi_thread")]
async fn preview_shows_the_exact_request_without_the_key_and_sends_nothing() {
    let dir = tmp_dir("ai-preview");
    let r = Runner::new(real_host(dir.path()));
    let doc = r.open(&mini_pbip(dir.path())).await;

    // No provider yet: the prompt only.
    let p = r
        .run(
            "ai.preview",
            json!({"capability": "dax.explain", "docId": doc, "measure": "Total"}),
        )
        .await;
    assert_eq!(p["wouldSend"], json!(false));
    assert!(p.get("request").is_none());
    assert_eq!(p["messages"][1]["role"], "user");
    assert_eq!(p["exposure"], "metadata");

    write_settings(dir.path(), &mock_settings(false));
    let p = r
        .run(
            "ai.preview",
            json!({"capability": "dax.explain", "docId": doc, "measure": "Total", "provider": "openai"}),
        )
        .await;
    let text = p.to_string();
    assert!(!text.contains(KEY), "the key never appears in a preview");
    assert_eq!(p["request"]["method"], "POST");
    assert_eq!(
        p["request"]["url"],
        "https://api.example.test/v1/chat/completions"
    );
    let auth = p["request"]["headers"]
        .as_array()
        .unwrap()
        .iter()
        .find(|h| h["name"] == "Authorization")
        .unwrap();
    assert_eq!(auth["value"], "Bearer [redacted]");
    assert_eq!(p["request"]["body"]["model"], "gpt-test");
    assert!(
        p["request"]["body"]["messages"][1]["content"]
            .as_str()
            .unwrap()
            .contains("SUM(Sales[Amount])")
    );
    assert_eq!(p["provider"]["apiKey"], "keychain");
    assert_eq!(p["provider"]["local"], json!(false));
    assert_eq!(p["consent"]["granted"], json!(false));
    assert_eq!(p["consent"]["endpointHost"], "api.example.test");
    assert_eq!(p["wouldSend"], json!(false));
    let blockers = p["blockers"].to_string();
    assert!(
        blockers.contains("disabled") && blockers.contains("consent required"),
        "{blockers}"
    );
    // Agents may preview too.
    let p = r
        .try_as(
            CallerKind::Agent,
            "ai.preview",
            json!({"capability": "dax.generate", "docId": doc, "prompt": "Average amount per item", "table": "Sales"}),
        )
        .await
        .unwrap();
    assert_eq!(p["request"]["url"], "http://127.0.0.1:11434/api/chat");
    assert_eq!(p["provider"]["apiKey"], "notRequired");
    assert_eq!(p["consent"]["local"], json!(true));
    // Contract-only capabilities are not implemented.
    let e = r
        .try_run(
            "ai.preview",
            json!({"capability": "chart.suggest", "docId": doc, "prompt": "x"}),
        )
        .await
        .unwrap_err();
    assert_eq!(
        (e.code, reason(&e).as_str()),
        (OpErrorCode::Unsupported, "notImplemented")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn answers_and_errors_never_contain_the_key() {
    let dir = tmp_dir("ai-redact");
    let mock = Arc::new(MockProvider::scripted(
        "openai",
        [
            MockReply::Json(json!({
                "summary": format!("Your key is {KEY}"),
                "explanation": "x",
            })),
            MockReply::Text(format!("not json, but here is {KEY}")),
        ],
    ));
    let r = Runner::new(mock_host(dir.path(), &mock));
    let doc = r.open(&mini_pbip(dir.path())).await;
    write_settings(dir.path(), &mock_settings(true));
    r.run(
        "ai.consent.grant",
        json!({"provider": "openai", "capability": "dax.explain"}),
    )
    .await;
    let params = json!({"docId": doc, "measure": "Total", "provider": "openai"});
    let out = r.run("ai.explainMeasure", params.clone()).await;
    assert!(!out.to_string().contains(KEY));
    assert_eq!(out["summary"], "Your key is [redacted]");
    let e = r.try_run("ai.explainMeasure", params).await.unwrap_err();
    assert_eq!(
        (e.code, reason(&e).as_str()),
        (OpErrorCode::Query, "invalidOutput")
    );
    let text = serde_json::to_string(&e).unwrap();
    assert!(!text.contains(KEY) && text.contains("[redacted]"), "{text}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_missing_key_stops_remote_requests() {
    let dir = tmp_dir("ai-nokey");
    let mock = Arc::new(MockProvider::new("x"));
    let host = mock_host(dir.path(), &mock);
    let r = Runner::new(host);
    let doc = r.open(&mini_pbip(dir.path())).await;
    write_settings(
        dir.path(),
        &json!({"enabled": true, "providers": [{"id": "azure", "kind": "azureOpenAi", "endpoint": "https://r.openai.azure.com", "deployment": "d"}]}),
    );
    r.run(
        "ai.consent.grant",
        json!({"provider": "azure", "capability": "dax.explain"}),
    )
    .await;
    let e = r
        .try_run(
            "ai.explainMeasure",
            json!({"docId": doc, "measure": "Total"}),
        )
        .await
        .unwrap_err();
    assert_eq!(reason(&e), "apiKeyMissing");
    assert!(e.message.contains("DASHCRAFT_AI_API_KEY"), "{}", e.message);
    assert_eq!(mock.calls(), 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn generated_measures_are_parsed_diagnosed_and_evaluated_but_never_applied() {
    let dir = tmp_dir("ai-generate");
    let mock = Arc::new(MockProvider::scripted(
        "local",
        [
            MockReply::Text(
                "Here it is:\n```json\n{\"name\": \"Half Total\", \"expression\": \"Half Total = DIVIDE([Total], 2)\", \"formatString\": \"0.0\", \"explanation\": \"Half of [Total].\", \"assumptions\": []}\n```"
                    .into(),
            ),
            MockReply::Json(json!({"name": "Broken", "expression": "SUM(Sales[Amount]", "explanation": ""})),
            MockReply::Json(json!({"name": "Total", "expression": "SUM(Sales[Nope]) + MADEUPFUNC(1)", "explanation": ""})),
        ],
    ));
    let r = Runner::new(mock_host(dir.path(), &mock));
    let doc = r.open(&mini_pbip(dir.path())).await;
    write_settings(dir.path(), &mock_settings(true));
    r.run(
        "ai.consent.grant",
        json!({"provider": "local", "capability": "dax.generate"}),
    )
    .await;
    let before = r.run("model.measures", json!({"docId": doc})).await;

    let out = r
        .run(
            "ai.generateMeasure",
            json!({"docId": doc, "prompt": "half of the total", "table": "Sales", "evaluate": true}),
        )
        .await;
    assert_eq!(out["name"], "Half Total");
    assert_eq!(out["table"], "Sales");
    assert_eq!(out["expression"], "DIVIDE([Total], 2)");
    assert_eq!(out["parsed"], json!(true));
    assert_eq!(out["diagnostics"], json!([]));
    assert_eq!(out["applied"], json!(false));
    let value = out["value"]["value"].as_f64().unwrap();
    assert!(
        (value - MINI_TOTAL as f64 / 2.0).abs() < 1e-9,
        "{}",
        out["value"]
    );
    assert_eq!(out["value"]["formatted"], "105.0");
    assert_eq!(out["call"]["promptVersion"], "system@1+dax.generate@1");
    let text = sent_text(&mock);
    assert!(
        text.contains("<user_request>\n\"half of the total\"\n</user_request>"),
        "{text}"
    );

    let out = r
        .run(
            "ai.generateMeasure",
            json!({"docId": doc, "prompt": "total", "evaluate": true}),
        )
        .await;
    assert_eq!(out["parsed"], json!(false));
    assert_eq!(out["diagnostics"][0]["severity"], "error");
    assert_eq!(out["diagnostics"][0]["line"], json!(1));
    assert!(
        out.get("value").is_none(),
        "unparsed suggestions are not evaluated"
    );

    let out = r
        .run(
            "ai.generateMeasure",
            json!({"docId": doc, "prompt": "total"}),
        )
        .await;
    assert_eq!(out["parsed"], json!(true));
    let messages: Vec<&str> = out["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| d["message"].as_str().unwrap())
        .collect();
    assert!(
        messages
            .iter()
            .any(|m| m.contains("unknown column Sales[Nope]")),
        "{messages:?}"
    );
    assert!(
        messages.iter().any(|m| m.contains("MADEUPFUNC")),
        "{messages:?}"
    );
    assert!(
        messages.iter().any(|m| m.contains("already exists")),
        "{messages:?}"
    );
    assert!(out.get("value").is_none(), "no evaluation unless asked");

    let after = r.run("model.measures", json!({"docId": doc})).await;
    assert_eq!(before, after, "nothing was written to the model");
}

#[tokio::test(flavor = "multi_thread")]
async fn sample_rows_need_the_setting_and_a_sample_consent() {
    let dir = tmp_dir("ai-sample");
    let mock = Arc::new(MockProvider::new("local"));
    let r = Runner::new(mock_host(dir.path(), &mock));
    let doc = r.open(&mini_pbip(dir.path())).await;
    write_settings(dir.path(), &mock_settings(true));
    r.run(
        "ai.consent.grant",
        json!({"provider": "local", "capability": "dax.generate"}),
    )
    .await;
    let params =
        json!({"docId": doc, "prompt": "average amount", "table": "Sales", "sampleRows": 3});
    let e = r
        .try_run("ai.generateMeasure", params.clone())
        .await
        .unwrap_err();
    assert_eq!(e.code, OpErrorCode::InvalidParams);
    assert!(e.message.contains("at most 0"), "{}", e.message);

    let set = r.run("ai.settings.set", json!({"sampleRows": 5})).await;
    assert_eq!(set["settings"]["sampleRows"], json!(5));
    let e = r
        .try_run("ai.generateMeasure", params.clone())
        .await
        .unwrap_err();
    assert_eq!(reason(&e), "consentRequired");
    assert_eq!(e.data.unwrap()["exposure"], "sample");
    // Explain can never be granted sample rows; generate can.
    let e = r
        .try_run(
            "ai.consent.grant",
            json!({"provider": "local", "capability": "dax.explain", "exposure": "sample"}),
        )
        .await
        .unwrap_err();
    assert_eq!(e.code, OpErrorCode::InvalidParams);
    let e = r
        .try_run(
            "ai.consent.grant",
            json!({"provider": "local", "capability": "dax.generate", "exposure": "full"}),
        )
        .await
        .unwrap_err();
    assert_eq!(e.code, OpErrorCode::InvalidParams);
    r.run(
        "ai.consent.grant",
        json!({"provider": "local", "capability": "dax.generate", "exposure": "sample"}),
    )
    .await;
    let out = r.run("ai.generateMeasure", params).await;
    assert_eq!(out["call"]["exposure"], "sample");
    let text = sent_text(&mock);
    assert!(
        text.contains("Item 01") && text.contains("Item 03") && !text.contains("Item 04"),
        "{text}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn settings_changes_are_validated_and_drop_stale_consents() {
    let dir = tmp_dir("ai-settings");
    let mock = Arc::new(MockProvider::new("local"));
    let r = Runner::new(mock_host(dir.path(), &mock));
    let set = r
        .run(
            "ai.settings.set",
            json!({"enabled": true, "providers": [{"id": "local", "kind": "ollama", "model": "m"}]}),
        )
        .await;
    assert_eq!(set["settings"]["enabled"], json!(true));
    r.run(
        "ai.consent.grant",
        json!({"provider": "local", "capability": "dax.explain"}),
    )
    .await;
    let e = r
        .try_run(
            "ai.settings.set",
            json!({"providers": [{"id": "remote", "kind": "openAiCompatible", "model": "m", "endpoint": "http://203.0.113.9/v1"}]}),
        )
        .await
        .unwrap_err();
    assert_eq!(e.code, OpErrorCode::InvalidParams);
    assert!(e.message.contains("plain http"), "{}", e.message);
    let set = r
        .run(
            "ai.settings.set",
            json!({"providers": [{"id": "local", "kind": "ollama", "model": "m", "endpoint": "http://localhost:11500"}]}),
        )
        .await;
    assert_eq!(set["consentsDropped"], json!(1));
    let e = r
        .try_run("ai.settings.set", json!({"defaultProvider": "nope"}))
        .await
        .unwrap_err();
    assert_eq!(e.code, OpErrorCode::InvalidParams);
    let status = r.run("ai.status", json!({})).await;
    assert_eq!(status["providers"][0]["endpointHost"], "localhost:11500");
    assert_eq!(status["providers"][0]["isDefault"], json!(true));
    assert_eq!(status["consents"], json!([]));
}

#[tokio::test(flavor = "multi_thread")]
async fn status_settings_round_trip_through_settings_set() {
    let dir = tmp_dir("ai-status-settings");
    let mock = Arc::new(MockProvider::new("local"));
    let r = Runner::new(mock_host(dir.path(), &mock));
    let status = r.run("ai.status", json!({})).await;
    assert_eq!(status["settings"]["enabled"], json!(false));
    assert_eq!(status["settings"]["providers"], json!([]));

    let azure = json!({
        "id": "azure", "kind": "azureOpenAi", "endpoint": "https://contoso.openai.azure.com",
        "deployment": "gpt-4o", "apiVersion": "2024-06-01", "structuredOutput": "jsonObject",
        "label": "Contoso"
    });
    r.run(
        "ai.settings.set",
        json!({"providers": [azure.clone()], "sampleRows": 5}),
    )
    .await;
    let status = r.run("ai.status", json!({})).await;
    // Everything a settings page needs to edit a provider without losing fields; no keys.
    assert_eq!(status["settings"]["providers"][0], azure);
    assert_eq!(status["settings"]["sampleRows"], json!(5));
    assert!(!status.to_string().contains(KEY));
    // Sending the providers back unchanged changes nothing.
    let set = r
        .run(
            "ai.settings.set",
            json!({"providers": status["settings"]["providers"].clone()}),
        )
        .await;
    assert_eq!(set["settings"], status["settings"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn cancellation_stops_a_running_request() {
    let dir = tmp_dir("ai-cancel");
    let mock = Arc::new(MockProvider::scripted(
        "local",
        [MockReply::Delay(
            Duration::from_secs(30),
            Box::new(explain_reply()),
        )],
    ));
    let r = Runner::new(mock_host(dir.path(), &mock));
    let doc = r.open(&mini_pbip(dir.path())).await;
    write_settings(dir.path(), &mock_settings(true));
    r.run(
        "ai.consent.grant",
        json!({"provider": "local", "capability": "dax.explain"}),
    )
    .await;
    let cancel = CancellationToken::new();
    let ctx = Ctx::new(CallerKind::Cli).with_cancel(cancel.clone());
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(150)).await;
        cancel.cancel();
    });
    let started = Instant::now();
    let e = r
        .try_with(
            &ctx,
            "ai.explainMeasure",
            json!({"docId": doc, "measure": "Total"}),
        )
        .await
        .unwrap_err();
    assert_eq!(e.code, OpErrorCode::Cancelled);
    assert!(started.elapsed() < Duration::from_secs(5));
    assert_eq!(mock.calls(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn library_errors_map_to_stable_reasons() {
    let e: pbi_ops::OpError = AiError::Unsupported(pbi_ai::Capability::MGenerate).into();
    assert_eq!(
        (e.code, reason(&e).as_str()),
        (OpErrorCode::Unsupported, "notImplemented")
    );
    let e: pbi_ops::OpError = AiError::Cancelled.into();
    assert_eq!(e.code, OpErrorCode::Cancelled);
}

#[tokio::test(flavor = "multi_thread")]
async fn example_measure_is_explained_with_the_mock() {
    let Some(pbip) = fixture("examples/acme-outfitters/AcmeOutfitters.pbip") else {
        return;
    };
    let dir = tmp_dir("ai-fixture");
    let mock = Arc::new(MockProvider::new("local"));
    let r = Runner::new(mock_host(dir.path(), &mock));
    let doc = r.open(&pbip).await;
    write_settings(dir.path(), &mock_settings(true));
    r.run(
        "ai.consent.grant",
        json!({"provider": "local", "capability": "dax.explain"}),
    )
    .await;
    let out = r
        .run(
            "ai.explainMeasure",
            json!({"docId": doc, "measure": "Sales All Stores"}),
        )
        .await;
    assert_eq!(out["measure"], "Sales All Stores");
    assert_eq!(out["expression"], "CALCULATE([Total Sales], ALL(Stores))");
    assert!(!out["references"]["tables"].as_array().unwrap().is_empty());
    assert_eq!(
        out["references"]["measures"],
        json!(["Orders[Total Sales]"])
    );
    let text = sent_text(&mock);
    assert!(text.contains(out["expression"].as_str().unwrap().trim()));
    assert_eq!(out["call"]["exposure"], "metadata");
}
