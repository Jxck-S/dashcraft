//! Running a capability: plan the request (provider, key, prompt), then —
//! only when AI is enabled, a consent covers the request and a key is
//! available — send it and validate the answer.

use std::sync::Arc;
use std::time::{Duration, Instant};

use schemars::JsonSchema;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;

use crate::capability::{Capability, DataExposure, model_output_schema};
use crate::endpoint::Endpoint;
use crate::error::{AiError, ConsentScope};
use crate::host::AiHost;
use crate::prompt;
use crate::provider::{ChatRequest, Provider, ResponseSchema, Usage};
use crate::secrets::{Redactor, Secret};
use crate::settings::{AiSettings, ProviderKind, ProviderSettings};

/// What was called (part of every capability result).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CallInfo {
    pub capability: Capability,
    /// Provider id.
    pub provider: String,
    pub kind: ProviderKind,
    /// The model that answered (as reported, else as configured).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// `host[:port]` the request went to.
    pub endpoint_host: String,
    /// The endpoint is this machine.
    pub local: bool,
    /// What the request contained.
    pub exposure: DataExposure,
    /// Prompt templates used (`system@1+dax.explain@1`).
    pub prompt_version: String,
    pub duration_ms: u64,
    pub usage: Usage,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finish_reason: Option<String>,
}

/// A request ready to send (or to preview).
pub(crate) struct Plan {
    pub capability: Capability,
    pub exposure: DataExposure,
    pub settings: AiSettings,
    pub provider_settings: ProviderSettings,
    pub endpoint: Endpoint,
    pub provider: Arc<dyn Provider>,
    pub request: ChatRequest,
    pub prompt_version: String,
    pub key: Option<Secret>,
    pub key_detail: Option<String>,
    pub max_input_chars: usize,
}

impl Plan {
    pub(crate) fn scope(&self) -> ConsentScope {
        ConsentScope {
            provider: self.provider_settings.id.clone(),
            endpoint_host: self.endpoint.host_port(),
            capability: self.capability,
            exposure: self.exposure,
            local: self.endpoint.is_local(),
        }
    }

    /// The provider will refuse (or should not get) the request without a
    /// key: Azure always, OpenAI-compatible unless the server is local.
    pub(crate) fn key_required(&self) -> bool {
        match self.provider_settings.kind {
            ProviderKind::AzureOpenAi => true,
            ProviderKind::OpenAiCompatible => !self.endpoint.is_loopback(),
            ProviderKind::Ollama | ProviderKind::Mock => false,
        }
    }
}

/// Plans `capability` for `input` with the provider `provider_id` (or the
/// default one).
///
/// # Errors
/// Provider lookup and construction errors; prompt errors.
pub(crate) fn plan<I: Serialize, O: JsonSchema>(
    host: &AiHost,
    settings: AiSettings,
    capability: Capability,
    exposure: DataExposure,
    provider_id: Option<&str>,
    input: &I,
    user_request: Option<&str>,
) -> Result<Plan, AiError> {
    let provider_settings = settings.provider(provider_id)?.clone();
    let endpoint = provider_settings.endpoint()?;
    let (key, _, key_detail) = host.api_key(&provider_settings);
    let provider = host.providers().create(&provider_settings, key.clone())?;
    let limits = settings.limits_for(capability);
    let schema = model_output_schema::<O>();
    let (messages, prompt_version) = prompt::messages(capability, input, user_request, &schema)?;
    let request = ChatRequest {
        messages,
        temperature: Some(limits.temperature),
        max_tokens: Some(limits.max_tokens),
        response_schema: Some(ResponseSchema {
            name: capability.id().replace('.', "_"),
            schema,
        }),
        timeout: Duration::from_secs(limits.timeout_secs),
        capability: Some(capability),
    };
    Ok(Plan {
        capability,
        exposure,
        settings,
        provider_settings,
        endpoint,
        provider,
        request,
        prompt_version,
        key,
        key_detail,
        max_input_chars: limits.max_input_chars,
    })
}

/// Sends a planned request and parses the answer into `O`.
///
/// Refuses before any network access when AI is disabled, the request is
/// too large, no consent covers it, or a required key is missing.
///
/// # Errors
/// [`AiError::Disabled`], [`AiError::InputTooLarge`],
/// [`AiError::ConsentRequired`], [`AiError::MissingApiKey`], provider and
/// output errors, [`AiError::Cancelled`].
pub(crate) async fn run<O: DeserializeOwned>(
    plan: Plan,
    cancel: CancellationToken,
) -> Result<(O, CallInfo), AiError> {
    if !plan.settings.enabled {
        return Err(AiError::Disabled);
    }
    let chars = plan.request.chars();
    if chars > plan.max_input_chars {
        return Err(AiError::InputTooLarge {
            capability: plan.capability,
            chars,
            limit: plan.max_input_chars,
        });
    }
    let scope = plan.scope();
    if plan.settings.consent_for(&scope).is_none() {
        return Err(AiError::ConsentRequired(Box::new(scope)));
    }
    if plan.key_required() && plan.key.is_none() {
        return Err(AiError::MissingApiKey {
            provider: plan.provider_settings.id.clone(),
            detail: plan.key_detail.clone(),
        });
    }
    tracing::info!(
        capability = %plan.capability,
        provider = %plan.provider_settings.id,
        endpoint = %scope.endpoint_host,
        local = scope.local,
        exposure = %plan.exposure,
        chars,
        "sending an AI request (consented)"
    );
    let started = Instant::now();
    let response = plan.provider.complete(plan.request, cancel).await?;
    let duration_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
    let redactor = Redactor::default().with(plan.key.as_ref());
    let content = redactor.redact_known(&response.content);
    let output: O = crate::provider::parse_output(plan.capability, &content)?;
    Ok((
        output,
        CallInfo {
            capability: plan.capability,
            provider: plan.provider_settings.id.clone(),
            kind: plan.provider_settings.kind,
            model: response
                .model
                .or_else(|| plan.provider_settings.model_name().map(str::to_owned)),
            endpoint_host: scope.endpoint_host,
            local: scope.local,
            exposure: plan.exposure,
            prompt_version: plan.prompt_version,
            duration_ms,
            usage: response.usage,
            finish_reason: response.finish_reason,
        },
    ))
}
