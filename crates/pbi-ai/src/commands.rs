//! The `ai.*` registry commands. Registered by [`register`](crate::register)
//! (not part of `Registry::default_catalog()`, which stays network-free).
//!
//! | Command | Agent-safe | Network |
//! |---|---|---|
//! | `ai.status`, `ai.preview` | yes | never |
//! | `ai.explainMeasure`, `ai.generateMeasure` | yes | only when enabled and consented |
//! | `ai.consent.grant`, `ai.consent.revoke`, `ai.settings.set`, `ai.apiKey.set`, `ai.apiKey.clear` | no | never |

use std::collections::BTreeMap;

use pbi_ops::{CallerKind, Category, Command, CommandMeta, Ctx, DocEntry, OpError, Workspace};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::capability::{
    Capability, CapabilityInfo, DataExposure, DaxExplainInput, DaxExplainOutput, DaxGenerateInput,
    DaxGenerateOutput, DependencyNote, SampleRows,
};
use crate::context;
use crate::error::AiError;
use crate::host::AiHost;
use crate::provider::{ChatMessage, HttpPreview};
use crate::secrets::{KEYRING_SERVICE, KeySource, keyring_account};
use crate::service::{self, CallInfo, Plan};
use crate::settings::{
    AiSettings, CapabilityLimits, Consent, EffectiveLimits, ProviderKind, ProviderSettings,
};

/// Version that added the AI commands.
pub const SINCE: &str = "0.3.0";

const BASE: CommandMeta = CommandMeta {
    id: "",
    title: "",
    summary: "",
    category: Category::Ai,
    mutates_document: false,
    long_running: false,
    agent_safe: true,
    writes_files: false,
    invokes_other_commands: false,
    since: SINCE,
    examples: &[],
};

/// Settings and consent commands: never for agents; they write the AI
/// settings file chosen by the host (not a path from the parameters).
const ADMIN: CommandMeta = CommandMeta {
    agent_safe: false,
    writes_files: true,
    ..BASE
};

fn refuse_agents(ctx: &Ctx, id: &str) -> Result<(), OpError> {
    if ctx.caller() == CallerKind::Agent {
        return Err(AiError::Forbidden(format!(
            "{id} is not available to agents: only a person can change AI settings, consents \
             or keys"
        ))
        .into());
    }
    Ok(())
}

fn caller_name(ctx: &Ctx) -> &'static str {
    match ctx.caller() {
        CallerKind::Cli => "cli",
        CallerKind::Rpc => "rpc",
        CallerKind::App => "app",
        CallerKind::Agent => "agent",
    }
}

fn path_text(host: &AiHost) -> Option<String> {
    host.settings_path().map(|p| p.display().to_string())
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NoParams {}

// ---------------------------------------------------------------------------
// ai.status

/// A configured provider (never its key).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ProviderStatus {
    pub id: String,
    pub kind: ProviderKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// Base URL (`null` if invalid).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub endpoint: Option<String>,
    /// `host[:port]`: what consents bind to.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub endpoint_host: Option<String>,
    /// The endpoint is this machine (data does not leave it).
    pub local: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Where its API key would come from (never the key).
    pub api_key: KeySource,
    pub is_default: bool,
    /// Capabilities with a consent for this provider and endpoint.
    pub consented: Vec<Capability>,
}

fn provider_status(host: &AiHost, settings: &AiSettings, p: &ProviderSettings) -> ProviderStatus {
    let endpoint = p.endpoint().ok();
    let (_, api_key, _) = host.api_key(p);
    let default_id = settings
        .default_provider
        .as_deref()
        .or(settings.providers.first().map(|f| f.id.as_str()));
    let host_port = endpoint.as_ref().map(crate::Endpoint::host_port);
    ProviderStatus {
        id: p.id.clone(),
        kind: p.kind,
        label: p.label.clone(),
        endpoint: endpoint.as_ref().map(crate::Endpoint::base),
        local: endpoint.as_ref().is_some_and(crate::Endpoint::is_local),
        model: p.model_name().map(str::to_owned),
        api_key,
        is_default: default_id == Some(p.id.as_str()),
        consented: settings
            .consents
            .iter()
            .filter(|c| {
                c.provider == p.id && host_port.as_deref() == Some(c.endpoint_host.as_str())
            })
            .map(|c| c.capability)
            .collect(),
        endpoint_host: host_port,
    }
}

/// `ai.status`.
#[derive(Debug)]
pub struct AiStatus;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct AiStatusOut {
    /// AI features are on. Off by default; nothing is sent while off.
    pub enabled: bool,
    /// The settings file (`null` if no location could be determined).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub settings_path: Option<String>,
    /// The settings file could not be read (AI then stays off).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub settings_error: Option<String>,
    pub providers: Vec<ProviderStatus>,
    /// Recorded consents.
    pub consents: Vec<Consent>,
    /// The capability contract and what is implemented.
    pub capabilities: Vec<CapabilityInfo>,
    /// Most sample rows a request may include (0 = none).
    pub sample_rows: usize,
    /// Effective limits per capability.
    pub limits: BTreeMap<Capability, EffectiveLimits>,
    /// The settings as stored (defaults when the file is missing or unreadable). Never holds
    /// keys. A settings page edits these and sends them back with `ai.settings.set`.
    pub settings: AiSettings,
}

impl Command for AiStatus {
    type Params = NoParams;
    type Output = AiStatusOut;

    fn meta() -> CommandMeta {
        CommandMeta {
            id: "ai.status",
            title: "AI status",
            summary: "Whether AI is enabled, configured providers (never keys), recorded consents, limits and which capabilities are implemented. No network.",
            examples: &[("Status", "{}")],
            ..BASE
        }
    }

    async fn run(_ws: &Workspace, _p: Self::Params, _ctx: &Ctx) -> Result<Self::Output, OpError> {
        let host = AiHost::current();
        let (settings, settings_error) = match host.load_settings() {
            Ok(s) => (s, None),
            Err(e) => (AiSettings::default(), Some(e.to_string())),
        };
        Ok(AiStatusOut {
            enabled: settings.enabled,
            settings_path: path_text(&host),
            settings_error,
            providers: settings
                .providers
                .iter()
                .map(|p| provider_status(&host, &settings, p))
                .collect(),
            consents: settings.consents.clone(),
            capabilities: Capability::ALL.map(CapabilityInfo::of).to_vec(),
            sample_rows: settings.sample_rows,
            limits: Capability::ALL
                .iter()
                .map(|c| (*c, settings.limits_for(*c)))
                .collect(),
            settings,
        })
    }
}

// ---------------------------------------------------------------------------
// Shared planning

fn load_settings(host: &AiHost) -> Result<AiSettings, OpError> {
    Ok(host.load_settings()?)
}

fn model_entry(ws: &Workspace, doc_id: &str) -> Result<std::sync::Arc<DocEntry>, OpError> {
    let entry = ws.get(doc_id)?;
    pbi_ops::resolve::require_model(&entry)?;
    Ok(entry)
}

struct ExplainCtx {
    table: String,
    measure: String,
    expression: String,
    format_string: Option<String>,
    input: DaxExplainInput,
}

fn explain_ctx(
    entry: &DocEntry,
    measure: &str,
    table: Option<&str>,
) -> Result<ExplainCtx, OpError> {
    let model = entry.session().base_model();
    let (t, m) = context::find_measure(&model, measure, table)?;
    Ok(ExplainCtx {
        table: t.name.clone(),
        measure: m.name.clone(),
        expression: m.expression.clone(),
        format_string: m.format_string.clone(),
        input: context::explain_input(&model, t, m),
    })
}

struct GenerateCtx {
    table: String,
    input: DaxGenerateInput,
    exposure: DataExposure,
}

async fn generate_ctx(
    entry: &DocEntry,
    settings: &AiSettings,
    prompt: &str,
    table: Option<&str>,
    sample_rows: Option<usize>,
    ctx: &Ctx,
) -> Result<GenerateCtx, OpError> {
    if prompt.trim().is_empty() {
        return Err(AiError::InvalidInput("the prompt is empty".into()).into());
    }
    if prompt.chars().count() > 4000 {
        return Err(
            AiError::InvalidInput("the prompt is longer than 4000 characters".into()).into(),
        );
    }
    let model = entry.session().base_model();
    let target = table
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .map(|t| {
            model
                .table(t)
                .map(|t| t.name.clone())
                .ok_or_else(|| AiError::NotFound {
                    message: format!("no table '{t}'"),
                    candidates: context::table_names(&model),
                })
        })
        .transpose()?;
    let home = target
        .clone()
        .or_else(|| {
            model
                .tables
                .iter()
                .find(|t| !t.measures.is_empty())
                .or(model.tables.first())
                .map(|t| t.name.clone())
        })
        .ok_or_else(|| AiError::InvalidInput("the model has no tables".into()))?;
    let n = sample_rows.unwrap_or(0);
    let mut sample: Option<SampleRows> = None;
    if n > 0 {
        if n > settings.sample_rows {
            return Err(AiError::InvalidInput(format!(
                "sampleRows is {n}, but the AI settings allow at most {} sample rows (0 = none); \
                 a person can raise sampleRows with ai.settings.set",
                settings.sample_rows
            ))
            .into());
        }
        let Some(t) = &target else {
            return Err(AiError::InvalidInput("sampleRows needs a table".into()).into());
        };
        entry.ensure_loaded(ctx).await?;
        sample = Some(context::sample_rows(entry.session(), t, n)?);
    }
    let exposure = if sample.is_some() {
        DataExposure::Sample
    } else {
        DataExposure::Metadata
    };
    Ok(GenerateCtx {
        table: home,
        input: context::generate_input(&model, prompt, target.as_deref(), sample),
        exposure,
    })
}

// ---------------------------------------------------------------------------
// ai.preview

/// `ai.preview`.
#[derive(Debug)]
pub struct AiPreview;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AiPreviewParams {
    /// `dax.explain` or `dax.generate` (the others are not implemented).
    pub capability: Capability,
    /// The document id from `doc.open` (`d1`, …).
    pub doc_id: String,
    /// `dax.explain`: the measure (`Name`, `[Name]` or `Table[Name]`).
    #[serde(default)]
    pub measure: Option<String>,
    /// `dax.explain`: the measure's table; `dax.generate`: the target table.
    #[serde(default)]
    pub table: Option<String>,
    /// `dax.generate`: what the measure should compute.
    #[serde(default)]
    pub prompt: Option<String>,
    /// Provider id (default: the default provider).
    #[serde(default)]
    pub provider: Option<String>,
    /// `dax.generate`: include this many sample rows of `table`.
    #[serde(default)]
    pub sample_rows: Option<usize>,
}

/// The consent a request needs and whether it is recorded.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ConsentCheck {
    pub granted: bool,
    pub provider: String,
    pub endpoint_host: String,
    pub capability: Capability,
    pub exposure: DataExposure,
    /// The endpoint is this machine.
    pub local: bool,
    /// The recorded consent that covers it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub consent: Option<Consent>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct AiPreviewOut {
    pub capability: Capability,
    /// What the request contains.
    pub exposure: DataExposure,
    /// The exposure class for people.
    pub exposure_description: String,
    /// AI is enabled.
    pub enabled: bool,
    /// The provider it would go to (`null` when none is configured).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<ProviderStatus>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub consent: Option<ConsentCheck>,
    /// Prompt templates used.
    pub prompt_version: String,
    /// Characters of the prompt messages.
    pub input_chars: usize,
    pub max_input_chars: usize,
    /// The exact HTTP request (key redacted); `null` without a provider.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request: Option<HttpPreview>,
    /// The prompt messages, when there is no provider (otherwise they are in
    /// `request.body`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub messages: Option<Vec<ChatMessage>>,
    /// It would be sent now.
    pub would_send: bool,
    /// Why it would not be sent.
    pub blockers: Vec<String>,
}

fn preview_of(host: &AiHost, plan: &Plan, input_chars: usize) -> Result<AiPreviewOut, OpError> {
    let scope = plan.scope();
    let consent = plan.settings.consent_for(&scope).cloned();
    let mut blockers = Vec::new();
    if !plan.settings.enabled {
        blockers.push(AiError::Disabled.to_string());
    }
    if input_chars > plan.max_input_chars {
        blockers.push(
            AiError::InputTooLarge {
                capability: plan.capability,
                chars: input_chars,
                limit: plan.max_input_chars,
            }
            .to_string(),
        );
    }
    if consent.is_none() {
        blockers.push(AiError::ConsentRequired(Box::new(scope.clone())).to_string());
    }
    if plan.key_required() && plan.key.is_none() {
        blockers.push(
            AiError::MissingApiKey {
                provider: plan.provider_settings.id.clone(),
                detail: None,
            }
            .to_string(),
        );
    }
    let request = plan.provider.http_request(&plan.request)?.preview();
    Ok(AiPreviewOut {
        capability: plan.capability,
        exposure: plan.exposure,
        exposure_description: plan.exposure.description().to_owned(),
        enabled: plan.settings.enabled,
        provider: Some(provider_status(
            host,
            &plan.settings,
            &plan.provider_settings,
        )),
        consent: Some(ConsentCheck {
            granted: consent.is_some(),
            provider: scope.provider,
            endpoint_host: scope.endpoint_host,
            capability: scope.capability,
            exposure: scope.exposure,
            local: scope.local,
            consent,
        }),
        prompt_version: plan.prompt_version.clone(),
        input_chars,
        max_input_chars: plan.max_input_chars,
        request: Some(request),
        messages: None,
        would_send: blockers.is_empty(),
        blockers,
    })
}

/// A preview without a provider: the prompt only.
fn preview_without_provider<I: Serialize, O: JsonSchema>(
    settings: &AiSettings,
    capability: Capability,
    exposure: DataExposure,
    input: &I,
    request: Option<&str>,
) -> Result<AiPreviewOut, OpError> {
    let schema = crate::capability::model_output_schema::<O>();
    let (messages, prompt_version) = crate::prompt::messages(capability, input, request, &schema)?;
    let input_chars = messages.iter().map(|m| m.content.chars().count()).sum();
    Ok(AiPreviewOut {
        capability,
        exposure,
        exposure_description: exposure.description().to_owned(),
        enabled: settings.enabled,
        provider: None,
        consent: None,
        prompt_version,
        input_chars,
        max_input_chars: settings.limits_for(capability).max_input_chars,
        request: None,
        messages: Some(messages),
        would_send: false,
        blockers: vec![AiError::NoProvider.to_string()],
    })
}

impl Command for AiPreview {
    type Params = AiPreviewParams;
    type Output = AiPreviewOut;

    fn meta() -> CommandMeta {
        CommandMeta {
            id: "ai.preview",
            title: "Preview an AI request",
            summary: "The exact request an AI capability would send (key redacted), its data-exposure class and consent status. Never sends anything; works while AI is disabled.",
            examples: &[
                (
                    "Explaining a measure",
                    r#"{"capability": "dax.explain", "docId": "d1", "measure": "Total Sales"}"#,
                ),
                (
                    "Generating a measure",
                    r#"{"capability": "dax.generate", "docId": "d1", "prompt": "Share of completed orders", "table": "Orders"}"#,
                ),
            ],
            ..BASE
        }
    }

    async fn run(ws: &Workspace, p: Self::Params, ctx: &Ctx) -> Result<Self::Output, OpError> {
        let host = AiHost::current();
        let settings = load_settings(&host)?;
        if !p.capability.implemented() {
            return Err(AiError::Unsupported(p.capability).into());
        }
        let entry = model_entry(ws, &p.doc_id)?;
        let no_provider = settings.providers.is_empty()
            && p.provider.as_deref().is_none_or(|s| s.trim().is_empty());
        match p.capability {
            Capability::DaxExplain => {
                let measure = p
                    .measure
                    .as_deref()
                    .ok_or_else(|| AiError::InvalidInput("dax.explain needs a measure".into()))?;
                let ec = explain_ctx(&entry, measure, p.table.as_deref())?;
                if no_provider {
                    return preview_without_provider::<_, DaxExplainOutput>(
                        &settings,
                        p.capability,
                        DataExposure::Metadata,
                        &ec.input,
                        None,
                    );
                }
                let plan = service::plan::<_, DaxExplainOutput>(
                    &host,
                    settings,
                    p.capability,
                    DataExposure::Metadata,
                    p.provider.as_deref(),
                    &ec.input,
                    None,
                )?;
                let chars = plan.request.chars();
                preview_of(&host, &plan, chars)
            }
            _ => {
                let prompt = p
                    .prompt
                    .as_deref()
                    .ok_or_else(|| AiError::InvalidInput("dax.generate needs a prompt".into()))?;
                let gc = generate_ctx(
                    &entry,
                    &settings,
                    prompt,
                    p.table.as_deref(),
                    p.sample_rows,
                    ctx,
                )
                .await?;
                if no_provider {
                    return preview_without_provider::<_, DaxGenerateOutput>(
                        &settings,
                        p.capability,
                        gc.exposure,
                        &gc.input,
                        Some(prompt),
                    );
                }
                let plan = service::plan::<_, DaxGenerateOutput>(
                    &host,
                    settings,
                    p.capability,
                    gc.exposure,
                    p.provider.as_deref(),
                    &gc.input,
                    Some(prompt),
                )?;
                let chars = plan.request.chars();
                preview_of(&host, &plan, chars)
            }
        }
    }
}

// ---------------------------------------------------------------------------
// ai.explainMeasure

/// `ai.explainMeasure`.
#[derive(Debug)]
pub struct AiExplainMeasure;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExplainMeasureParams {
    /// The document id from `doc.open` (`d1`, …).
    pub doc_id: String,
    /// `Name`, `[Name]` or `Table[Name]`.
    pub measure: String,
    /// The measure's table (when names repeat).
    #[serde(default)]
    pub table: Option<String>,
    /// Provider id (default: the default provider).
    #[serde(default)]
    pub provider: Option<String>,
}

/// What a measure references (local analysis, not the model's claims).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct MeasureReferences {
    /// Measures reached, transitively (`Table[Measure]`).
    pub measures: Vec<String>,
    /// Columns with their data types.
    pub columns: Vec<String>,
    pub tables: Vec<String>,
    /// Relationships touching those tables (`from → to (cardinality, filter)`).
    pub relationships: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ExplainMeasureOut {
    pub measure: String,
    pub table: String,
    /// The measure's DAX.
    pub expression: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format_string: Option<String>,
    /// One or two sentences.
    pub summary: String,
    /// The explanation (Markdown).
    pub explanation: String,
    /// Filter context and context-transition notes.
    pub filter_context: Vec<String>,
    /// The model's note per dependency.
    pub dependencies: Vec<DependencyNote>,
    pub caveats: Vec<String>,
    /// What the measure references (computed locally).
    pub references: MeasureReferences,
    /// Everything above as one Markdown document.
    pub markdown: String,
    pub call: CallInfo,
}

fn bullets(out: &mut String, title: &str, items: impl IntoIterator<Item = String>) {
    let items: Vec<String> = items.into_iter().filter(|i| !i.trim().is_empty()).collect();
    if items.is_empty() {
        return;
    }
    out.push_str(&format!("\n**{title}**\n\n"));
    for i in items {
        out.push_str(&format!("- {}\n", i.trim()));
    }
}

fn explain_markdown(ec: &ExplainCtx, o: &DaxExplainOutput, call: &CallInfo) -> String {
    let mut md = format!(
        "### {}\n\n```dax\n{}\n```\n\n{}\n\n{}\n",
        pbi_model::dax_column_ref(&ec.table, &ec.measure),
        ec.expression.trim(),
        o.summary.trim(),
        o.explanation.trim()
    );
    bullets(&mut md, "Filter context", o.filter_context.iter().cloned());
    bullets(
        &mut md,
        "Dependencies",
        o.dependencies
            .iter()
            .map(|d| format!("`{}`: {}", d.name.trim(), d.note.trim())),
    );
    bullets(&mut md, "Caveats", o.caveats.iter().cloned());
    md.push_str(&format!(
        "\n_AI-generated by {} ({}); review before relying on it._\n",
        call.provider,
        call.model.as_deref().unwrap_or(call.kind.as_str())
    ));
    md
}

impl Command for AiExplainMeasure {
    type Params = ExplainMeasureParams;
    type Output = ExplainMeasureOut;

    fn meta() -> CommandMeta {
        CommandMeta {
            id: "ai.explainMeasure",
            title: "Explain a measure (AI)",
            summary: "Explains a measure with an AI model: summary, Markdown explanation, filter-context notes, dependencies, caveats. Sends model metadata only; needs AI enabled and consent.",
            long_running: true,
            examples: &[
                (
                    "Explain a measure",
                    r#"{"docId": "d1", "measure": "Total Sales"}"#,
                ),
                (
                    "With a named provider",
                    r#"{"docId": "d1", "measure": "Orders[Margin %]", "provider": "ollama"}"#,
                ),
            ],
            ..BASE
        }
    }

    async fn run(ws: &Workspace, p: Self::Params, ctx: &Ctx) -> Result<Self::Output, OpError> {
        let host = AiHost::current();
        let settings = load_settings(&host)?;
        if !settings.enabled {
            return Err(AiError::Disabled.into());
        }
        let entry = model_entry(ws, &p.doc_id)?;
        let ec = explain_ctx(&entry, &p.measure, p.table.as_deref())?;
        let plan = service::plan::<_, DaxExplainOutput>(
            &host,
            settings,
            Capability::DaxExplain,
            DataExposure::Metadata,
            p.provider.as_deref(),
            &ec.input,
            None,
        )?;
        ctx.progress(
            format!(
                "asking {} to explain {}",
                plan.provider_settings.id, ec.measure
            ),
            None,
        );
        let (o, call) =
            service::run::<DaxExplainOutput>(plan, ctx.cancel_token().child_token()).await?;
        if o.summary.trim().is_empty() && o.explanation.trim().is_empty() {
            return Err(AiError::InvalidOutput {
                capability: Capability::DaxExplain,
                message: "the answer has neither a summary nor an explanation".into(),
                raw: String::new(),
            }
            .into());
        }
        let (measures, columns, tables) = context::reference_names(&ec.input);
        let references = MeasureReferences {
            measures,
            columns,
            tables,
            relationships: ec
                .input
                .relationships
                .iter()
                .map(|r| {
                    format!(
                        "{} → {} ({}, {}{})",
                        r.from,
                        r.to,
                        r.cardinality,
                        r.cross_filter,
                        if r.active { "" } else { ", inactive" }
                    )
                })
                .collect(),
        };
        let markdown = explain_markdown(&ec, &o, &call);
        Ok(ExplainMeasureOut {
            measure: ec.measure,
            table: ec.table,
            expression: ec.expression,
            format_string: ec.format_string,
            summary: o.summary,
            explanation: o.explanation,
            filter_context: o.filter_context,
            dependencies: o.dependencies,
            caveats: o.caveats,
            references,
            markdown,
            call,
        })
    }
}

// ---------------------------------------------------------------------------
// ai.generateMeasure

/// `ai.generateMeasure`.
#[derive(Debug)]
pub struct AiGenerateMeasure;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GenerateMeasureParams {
    /// The document id from `doc.open` (`d1`, …).
    pub doc_id: String,
    /// What the measure should compute (at most 4000 characters).
    pub prompt: String,
    /// Home table of the new measure.
    #[serde(default)]
    pub table: Option<String>,
    /// Provider id (default: the default provider).
    #[serde(default)]
    pub provider: Option<String>,
    /// Evaluate the suggestion locally (unfiltered) when it parses; loads the
    /// data on first use. Nothing is written to the model.
    #[serde(default)]
    pub evaluate: bool,
    /// Include this many rows of `table` (needs `sampleRows` in the settings
    /// and a `sample` consent).
    #[serde(default)]
    pub sample_rows: Option<usize>,
}

/// A problem found locally in the suggestion.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SuggestionDiagnostic {
    /// `error` (does not parse) or `warning`.
    pub severity: String,
    pub message: String,
    /// 1-based position in `expression`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub column: Option<usize>,
    /// What the parser expected there.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub expected: Vec<String>,
}

/// The suggestion's value, evaluated locally.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SuggestionValue {
    /// Raw value (`null` for BLANK or on error).
    pub value: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub formatted: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct GenerateMeasureOut {
    /// Suggested measure name.
    pub name: String,
    /// Suggested home table.
    pub table: String,
    /// The DAX expression.
    pub expression: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format_string: Option<String>,
    /// How it works (Markdown).
    pub explanation: String,
    pub assumptions: Vec<String>,
    /// The expression parses as DAX.
    pub parsed: bool,
    /// Parse errors and unknown references.
    pub diagnostics: Vec<SuggestionDiagnostic>,
    /// With `evaluate`: the value (unfiltered).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<SuggestionValue>,
    /// Always `false`: suggestions are never written to the model.
    pub applied: bool,
    pub call: CallInfo,
}

fn strip_prefix_ci<'a>(s: &'a str, prefix: &str) -> Option<&'a str> {
    let head = s.get(..prefix.len())?;
    head.eq_ignore_ascii_case(prefix)
        .then(|| s.get(prefix.len()..))
        .flatten()
}

/// The expression without code fences and a leading `Name =`.
fn clean_expression(expr: &str, name: &str) -> String {
    let mut e = expr.trim();
    if let Some(rest) = e.strip_prefix("```") {
        let rest = rest.split_once('\n').map_or("", |(_, body)| body);
        e = rest.trim_end().strip_suffix("```").unwrap_or(rest).trim();
    }
    let name = name.trim();
    if !name.is_empty() {
        for prefix in [
            format!("{name} :="),
            format!("{name} ="),
            format!("[{name}] :="),
            format!("[{name}] ="),
        ] {
            if let Some(rest) = strip_prefix_ci(e, &prefix) {
                e = rest.trim();
                break;
            }
        }
    }
    e.to_owned()
}

fn check_references(
    model: &pbi_model::Model,
    expr: &pbi_dax::Expr,
    out: &mut Vec<SuggestionDiagnostic>,
) {
    let warn = |message: String| SuggestionDiagnostic {
        severity: "warning".into(),
        message,
        line: None,
        column: None,
        expected: Vec::new(),
    };
    let any_column = |name: &str| model.tables.iter().any(|t| t.column(name).is_some());
    for r in pbi_dax::references(expr) {
        match r {
            pbi_dax::Reference::Table(t) if model.table(&t).is_none() => {
                out.push(warn(format!("unknown table '{t}'")));
            }
            pbi_dax::Reference::Column { table, column }
                if model.column(&table, &column).is_none()
                    && model.measure(&table, &column).is_none() =>
            {
                out.push(warn(format!(
                    "unknown column {}",
                    pbi_model::dax_column_ref(&table, &column)
                )));
            }
            pbi_dax::Reference::Measure(m)
                if model.find_measure(&m).is_none() && !any_column(&m) =>
            {
                out.push(warn(format!("unknown measure or column [{m}]")));
            }
            pbi_dax::Reference::Function(f) if !pbi_dax::is_supported_function(&f) => {
                out.push(warn(format!(
                    "{f} is not supported by DashCraft's DAX engine yet (it may still be valid DAX)"
                )));
            }
            _ => {}
        }
    }
}

const SUGGESTION_MEASURE: &str = "__DashCraftAiSuggestion";

async fn evaluate_suggestion(
    entry: &DocEntry,
    table: &str,
    expression: &str,
    format_string: Option<&str>,
    ctx: &Ctx,
) -> Result<SuggestionValue, OpError> {
    let failed = |error: String| SuggestionValue {
        value: Value::Null,
        formatted: None,
        error: Some(error),
    };
    match entry.ensure_loaded(ctx).await {
        Ok(_) => {}
        Err(e) if e.code == pbi_ops::OpErrorCode::Cancelled => return Err(e),
        Err(e) => {
            return Ok(failed(format!(
                "the data could not be loaded: {}",
                e.message
            )));
        }
    }
    let measure = pbi_model::dax_column_ref(table, SUGGESTION_MEASURE);
    let query = format!(
        "DEFINE MEASURE {measure} =\n{expression}\nEVALUATE ROW(\"value\", {})",
        pbi_model::dax_measure_ref(SUGGESTION_MEASURE)
    );
    let session = entry.session();
    let batches = match session
        .dax_query(
            &query,
            &pbi_session::FilterSpec::new(),
            ctx.cancel_token().child_token(),
        )
        .await
    {
        Ok(b) => b,
        Err(pbi_session::SessionError::Cancelled) => return Err(OpError::cancelled()),
        Err(e) => return Ok(failed(e.to_string())),
    };
    let Some(batch) = batches
        .first()
        .filter(|b| b.num_rows() > 0 && b.num_columns() > 0)
    else {
        return Ok(failed("the query returned no value".into()));
    };
    match pbi_engine::value::value_from_array(batch.column(0), 0) {
        Ok(v) => Ok(SuggestionValue {
            value: pbi_session::value_to_json(&v),
            formatted: Some(pbi_dax::format_value(&v, format_string, &session.culture())),
            error: None,
        }),
        Err(e) => Ok(failed(e.to_string())),
    }
}

impl Command for AiGenerateMeasure {
    type Params = GenerateMeasureParams;
    type Output = GenerateMeasureOut;

    fn meta() -> CommandMeta {
        CommandMeta {
            id: "ai.generateMeasure",
            title: "Suggest a measure (AI)",
            summary: "Natural language → a DAX measure suggestion, parsed locally with diagnostics and optionally evaluated; never written to the model. Needs AI enabled and consent.",
            long_running: true,
            examples: &[
                (
                    "Suggest a measure",
                    r#"{"docId": "d1", "prompt": "Share of orders that are completed", "table": "Orders"}"#,
                ),
                (
                    "Suggest and evaluate",
                    r#"{"docId": "d1", "prompt": "Average number of returns per store", "evaluate": true}"#,
                ),
            ],
            ..BASE
        }
    }

    async fn run(ws: &Workspace, p: Self::Params, ctx: &Ctx) -> Result<Self::Output, OpError> {
        let host = AiHost::current();
        let settings = load_settings(&host)?;
        if !settings.enabled {
            return Err(AiError::Disabled.into());
        }
        let entry = model_entry(ws, &p.doc_id)?;
        let gc = generate_ctx(
            &entry,
            &settings,
            &p.prompt,
            p.table.as_deref(),
            p.sample_rows,
            ctx,
        )
        .await?;
        let plan = service::plan::<_, DaxGenerateOutput>(
            &host,
            settings,
            Capability::DaxGenerate,
            gc.exposure,
            p.provider.as_deref(),
            &gc.input,
            Some(&p.prompt),
        )?;
        ctx.progress(
            format!("asking {} for a measure", plan.provider_settings.id),
            None,
        );
        let (o, call) =
            service::run::<DaxGenerateOutput>(plan, ctx.cancel_token().child_token()).await?;
        let expression = clean_expression(&o.expression, &o.name);
        if expression.is_empty() {
            return Err(AiError::InvalidOutput {
                capability: Capability::DaxGenerate,
                message: "the answer has no expression".into(),
                raw: String::new(),
            }
            .into());
        }
        let model = entry.session().base_model();
        let mut diagnostics = Vec::new();
        let parsed = match pbi_dax::parse_expression(&expression) {
            Ok(expr) => {
                check_references(&model, &expr, &mut diagnostics);
                true
            }
            Err(e) => {
                let (line, column) = e.line_col(&expression);
                diagnostics.push(SuggestionDiagnostic {
                    severity: "error".into(),
                    message: e.to_string(),
                    line: Some(line),
                    column: Some(column),
                    expected: e.expected.clone(),
                });
                false
            }
        };
        let name = if o.name.trim().is_empty() {
            "New measure".to_owned()
        } else {
            o.name.trim().to_owned()
        };
        if let Some((t, _)) = model.find_measure(&name) {
            diagnostics.push(SuggestionDiagnostic {
                severity: "warning".into(),
                message: format!("a measure named '{name}' already exists in '{}'", t.name),
                line: None,
                column: None,
                expected: Vec::new(),
            });
        }
        let format_string = o.format_string.filter(|f| !f.trim().is_empty());
        let value = if p.evaluate && parsed {
            Some(
                evaluate_suggestion(
                    &entry,
                    &gc.table,
                    &expression,
                    format_string.as_deref(),
                    ctx,
                )
                .await?,
            )
        } else {
            None
        };
        Ok(GenerateMeasureOut {
            name,
            table: gc.table,
            expression,
            format_string,
            explanation: o.explanation,
            assumptions: o.assumptions,
            parsed,
            diagnostics,
            value,
            applied: false,
            call,
        })
    }
}

// ---------------------------------------------------------------------------
// ai.consent.grant / ai.consent.revoke

/// `ai.consent.grant`.
#[derive(Debug)]
pub struct AiConsentGrant;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConsentGrantParams {
    /// Provider id.
    pub provider: String,
    pub capability: Capability,
    /// The most that may be sent (default `metadata`; `full` is refused).
    #[serde(default)]
    pub exposure: DataExposure,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ConsentGrantOut {
    pub consent: Consent,
    /// What was allowed, for people.
    pub note: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub settings_path: Option<String>,
}

impl Command for AiConsentGrant {
    type Params = ConsentGrantParams;
    type Output = ConsentGrantOut;

    fn meta() -> CommandMeta {
        CommandMeta {
            id: "ai.consent.grant",
            title: "Grant AI consent",
            summary: "Records a person's consent to send one capability's data (metadata or sample) to one provider endpoint. Not available to agents.",
            examples: &[
                (
                    "Explain with a local Ollama",
                    r#"{"provider": "ollama", "capability": "dax.explain"}"#,
                ),
                (
                    "Generate with sample rows",
                    r#"{"provider": "openai", "capability": "dax.generate", "exposure": "sample"}"#,
                ),
            ],
            ..ADMIN
        }
    }

    async fn run(_ws: &Workspace, p: Self::Params, ctx: &Ctx) -> Result<Self::Output, OpError> {
        refuse_agents(ctx, "ai.consent.grant")?;
        let host = AiHost::current();
        let by = caller_name(ctx);
        let consent = host.grant_consent(&p.provider, p.capability, p.exposure, Some(by))?;
        let note = format!(
            "{} may now receive {} for {}{}",
            consent.endpoint_host,
            consent.exposure.description(),
            consent.capability,
            if consent.local {
                " (a local endpoint: data stays on this machine)"
            } else {
                " (a remote endpoint: data leaves this machine)"
            }
        );
        Ok(ConsentGrantOut {
            consent,
            note,
            settings_path: path_text(&host),
        })
    }
}

/// `ai.consent.revoke`.
#[derive(Debug)]
pub struct AiConsentRevoke;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConsentRevokeParams {
    /// Provider id.
    pub provider: String,
    /// Only this capability (default: all of the provider's consents).
    #[serde(default)]
    pub capability: Option<Capability>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ConsentRevokeOut {
    /// Consents removed.
    pub removed: usize,
    /// Consents left.
    pub consents: Vec<Consent>,
}

impl Command for AiConsentRevoke {
    type Params = ConsentRevokeParams;
    type Output = ConsentRevokeOut;

    fn meta() -> CommandMeta {
        CommandMeta {
            id: "ai.consent.revoke",
            title: "Revoke AI consent",
            summary: "Removes recorded consents of a provider (one capability or all). Not available to agents.",
            examples: &[
                ("All of a provider", r#"{"provider": "openai"}"#),
                (
                    "One capability",
                    r#"{"provider": "openai", "capability": "dax.generate"}"#,
                ),
            ],
            ..ADMIN
        }
    }

    async fn run(_ws: &Workspace, p: Self::Params, ctx: &Ctx) -> Result<Self::Output, OpError> {
        refuse_agents(ctx, "ai.consent.revoke")?;
        let host = AiHost::current();
        let removed = host.revoke_consent(&p.provider, p.capability)?;
        Ok(ConsentRevokeOut {
            removed,
            consents: host.load_settings()?.consents,
        })
    }
}

// ---------------------------------------------------------------------------
// ai.settings.set

/// `ai.settings.set`.
#[derive(Debug)]
pub struct AiSettingsSet;

/// Fields to change; absent fields keep their value.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SettingsSetParams {
    /// Turn AI on or off.
    #[serde(default)]
    pub enabled: Option<bool>,
    /// Replace the provider list (keys are never part of it). Consents of
    /// removed providers or changed endpoints are dropped.
    #[serde(default)]
    pub providers: Option<Vec<ProviderSettings>>,
    /// Default provider id (`""` clears it).
    #[serde(default)]
    pub default_provider: Option<String>,
    /// Replace the per-capability limits.
    #[serde(default)]
    pub limits: Option<BTreeMap<Capability, CapabilityLimits>>,
    /// Most sample rows per request (0 = none, at most 100).
    #[serde(default)]
    pub sample_rows: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SettingsSetOut {
    pub settings: AiSettings,
    /// Consents dropped because their provider or endpoint changed.
    pub consents_dropped: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub settings_path: Option<String>,
}

impl Command for AiSettingsSet {
    type Params = SettingsSetParams;
    type Output = SettingsSetOut;

    fn meta() -> CommandMeta {
        CommandMeta {
            id: "ai.settings.set",
            title: "Change AI settings",
            summary: "Changes the AI settings (enable, providers, default provider, limits, sample rows); never holds keys. Not available to agents.",
            examples: &[
                (
                    "Enable with a local Ollama",
                    r#"{"enabled": true, "providers": [{"id": "ollama", "kind": "ollama", "model": "qwen2.5-coder:7b"}]}"#,
                ),
                (
                    "Azure OpenAI",
                    r#"{"providers": [{"id": "azure", "kind": "azureOpenAi", "endpoint": "https://contoso.openai.azure.com", "deployment": "gpt-4o"}], "defaultProvider": "azure"}"#,
                ),
                ("Disable", r#"{"enabled": false}"#),
            ],
            ..ADMIN
        }
    }

    async fn run(_ws: &Workspace, p: Self::Params, ctx: &Ctx) -> Result<Self::Output, OpError> {
        refuse_agents(ctx, "ai.settings.set")?;
        let host = AiHost::current();
        let (settings, dropped) = host.update_settings(|s| {
            if let Some(v) = p.enabled {
                s.enabled = v;
            }
            if let Some(v) = p.providers {
                s.providers = v;
                // A default that no longer exists falls back to the first
                // provider, unless a new default is given below.
                if s.default_provider
                    .as_deref()
                    .is_some_and(|d| !s.providers.iter().any(|p| p.id == d))
                {
                    s.default_provider = None;
                }
            }
            if let Some(v) = p.default_provider {
                s.default_provider = Some(v.trim().to_owned()).filter(|d| !d.is_empty());
            }
            if let Some(v) = p.limits {
                s.limits = v;
            }
            if let Some(v) = p.sample_rows {
                s.sample_rows = v;
            }
            s.validate()?;
            let dropped = s.prune_consents();
            Ok((s.clone(), dropped))
        })?;
        Ok(SettingsSetOut {
            settings,
            consents_dropped: dropped,
            settings_path: path_text(&host),
        })
    }
}

// ---------------------------------------------------------------------------
// ai.apiKey.set / ai.apiKey.clear

/// `ai.apiKey.set`.
#[derive(Debug)]
pub struct AiApiKeySet;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ApiKeySetParams {
    /// Provider id.
    pub provider: String,
    /// The API key. Only accepted from the desktop app.
    pub key: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ApiKeyOut {
    pub provider: String,
    /// A key was stored (`set`) or removed (`clear`).
    pub changed: bool,
    /// `keychain` (or the host's store).
    pub store: String,
    /// Keychain service and account.
    pub service: String,
    pub account: String,
}

impl Command for AiApiKeySet {
    type Params = ApiKeySetParams;
    type Output = ApiKeyOut;

    fn meta() -> CommandMeta {
        CommandMeta {
            id: "ai.apiKey.set",
            title: "Store an AI API key",
            summary: "Stores a provider's API key in the OS keychain. Desktop app only (the key travels in the parameters); use `dashcraft ai login` on the command line.",
            examples: &[(
                "From the app's settings page",
                r#"{"provider": "openai", "key": "<key typed by the user>"}"#,
            )],
            ..ADMIN
        }
    }

    async fn run(_ws: &Workspace, p: Self::Params, ctx: &Ctx) -> Result<Self::Output, OpError> {
        if ctx.caller() != CallerKind::App {
            return Err(AiError::Forbidden(
                "ai.apiKey.set only accepts keys from the desktop app; on the command line use \
                 `dashcraft ai login` (reads the key without echo) or DASHCRAFT_AI_API_KEY. If you \
                 typed a real key on a command line, it may be in your shell history: rotate it"
                    .into(),
            )
            .into());
        }
        let host = AiHost::current();
        host.store_api_key(&p.provider, &p.key)?;
        Ok(ApiKeyOut {
            changed: true,
            store: host.secrets().name().to_owned(),
            service: KEYRING_SERVICE.to_owned(),
            account: keyring_account(&p.provider),
            provider: p.provider,
        })
    }
}

/// `ai.apiKey.clear`.
#[derive(Debug)]
pub struct AiApiKeyClear;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ApiKeyClearParams {
    /// Provider id.
    pub provider: String,
}

impl Command for AiApiKeyClear {
    type Params = ApiKeyClearParams;
    type Output = ApiKeyOut;

    fn meta() -> CommandMeta {
        CommandMeta {
            id: "ai.apiKey.clear",
            title: "Remove an AI API key",
            summary: "Deletes a provider's API key from the OS keychain (DASHCRAFT_AI_API_KEY is not affected). Not available to agents.",
            examples: &[("Remove the OpenAI key", r#"{"provider": "openai"}"#)],
            ..ADMIN
        }
    }

    async fn run(_ws: &Workspace, p: Self::Params, ctx: &Ctx) -> Result<Self::Output, OpError> {
        refuse_agents(ctx, "ai.apiKey.clear")?;
        let host = AiHost::current();
        let changed = host.clear_api_key(&p.provider)?;
        Ok(ApiKeyOut {
            changed,
            store: host.secrets().name().to_owned(),
            service: KEYRING_SERVICE.to_owned(),
            account: keyring_account(&p.provider),
            provider: p.provider,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expressions_are_cleaned() {
        assert_eq!(clean_expression("```dax\nSUM(T[x])\n```", "X"), "SUM(T[x])");
        assert_eq!(
            clean_expression("Total Sales = SUM(T[x])", "Total Sales"),
            "SUM(T[x])"
        );
        assert_eq!(clean_expression("[total sales] := 1", "Total Sales"), "1");
        assert_eq!(clean_expression("  DIVIDE(1, 2) ", "R"), "DIVIDE(1, 2)");
        assert_eq!(clean_expression("```\n1\n```", ""), "1");
    }
}
