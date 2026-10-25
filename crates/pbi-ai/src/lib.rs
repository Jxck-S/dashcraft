//! Opt-in AI layer for DashCraft: a capability contract,
//! provider adapters, settings, consent and key handling, exposed as
//! registry commands (`ai.*`). See ADR-0008 (`docs/adr/0008-ai-layer.md`)
//! and the user guide `docs/AI.md`.
//!
//! - **Off by default.** Nothing is sent until a person enables AI
//!   ([`AiSettings::enabled`]), configures a provider and records a
//!   [`Consent`] for the provider endpoint, capability and
//!   [`DataExposure`] class. A local endpoint (Ollama on loopback) also
//!   needs consent.
//! - **Capabilities** ([`Capability`]): `dax.explain` and `dax.generate`
//!   are implemented; `m.generate`, `report.lint` and `chart.suggest` have
//!   typed contracts and answer [`AiError::Unsupported`].
//! - **Data exposure.** v1 requests carry model metadata (names, types,
//!   descriptions, relationships, DAX/M); sample rows only when the
//!   settings allow them and a `sample` consent exists; never full tables.
//! - **Providers** ([`Provider`]): [`OpenAiCompatible`], [`AzureOpenAi`],
//!   [`Ollama`] (blocking `ureq` + rustls off the async runtime, cancellable) and
//!   [`MockProvider`].
//! - **Keys** only in the OS keychain (service `dashcraft`, account
//!   `ai:<provider-id>`, [`store_api_key`]) or `DASHCRAFT_AI_API_KEY`;
//!   never in settings, logs, errors, previews or
//!   outputs ([`Redactor`]).
//! - **Untrusted metadata.** Prompts are versioned templates
//!   (`prompts/*.md`); model metadata is JSON-escaped inside delimited
//!   blocks and the model is told to ignore instructions in it. Outputs are
//!   validated and never executed or applied.
//! - **Registry.** [`register`] adds the commands to a
//!   [`Registry`](pbi_ops::Registry); `Registry::default_catalog()` stays
//!   network-free. Settings, consents and keys are changed only by
//!   non-agent commands (`agent_safe: false`) or library calls.
//!
//! ```no_run
//! # async fn demo() -> Result<(), pbi_ops::OpError> {
//! use pbi_ops::{CallerKind, Ctx, Registry, Workspace};
//! use serde_json::json;
//!
//! let mut registry = Registry::default_catalog();
//! pbi_ai::register(&mut registry);
//! let ws = Workspace::new();
//! let ctx = Ctx::new(CallerKind::Cli);
//! let doc = registry.execute(&ws, "doc.open", json!({"path": "model.pbip"}), &ctx).await?;
//! let preview = registry
//!     .execute(&ws, "ai.preview", json!({
//!         "capability": "dax.explain", "docId": doc["docId"], "measure": "Total Sales"
//!     }), &ctx)
//!     .await?;
//! println!("{}", preview["request"]["body"]);
//! # Ok(()) }
//! ```

pub mod capability;
pub mod commands;
mod context;
mod endpoint;
mod error;
mod host;
pub mod prompt;
pub mod provider;
pub mod secrets;
mod service;
pub mod settings;

pub use capability::{Capability, CapabilityContract, CapabilityInfo, DataExposure};
pub use endpoint::Endpoint;
pub use error::{AiError, ConsentScope, truncate};
pub use host::{AiHost, clear_api_key, store_api_key};
pub use provider::{
    AzureOpenAi, ChatMessage, ChatRequest, ChatResponse, DefaultProviderFactory, MockProvider,
    MockReply, Ollama, OpenAiCompatible, Provider, ProviderFactory, ResponseSchema, Role, Usage,
};
pub use secrets::{
    API_KEY_ENV, KEYRING_SERVICE, KeySource, KeychainStore, MemorySecretStore, Redactor, Secret,
    SecretStore,
};
pub use service::CallInfo;
pub use settings::{
    AiSettings, CapabilityLimits, Consent, ProviderKind, ProviderSettings, StructuredOutput,
    default_settings_path,
};

use pbi_ops::Registry;

/// Adds the `ai.*` commands to `registry`.
pub fn register(registry: &mut Registry) {
    registry
        .register::<commands::AiStatus>()
        .register::<commands::AiPreview>()
        .register::<commands::AiExplainMeasure>()
        .register::<commands::AiGenerateMeasure>()
        .register::<commands::AiConsentGrant>()
        .register::<commands::AiConsentRevoke>()
        .register::<commands::AiSettingsSet>()
        .register::<commands::AiApiKeySet>()
        .register::<commands::AiApiKeyClear>();
}

/// `Registry::default_catalog()` plus the `ai.*` commands.
#[must_use]
pub fn catalog() -> Registry {
    let mut registry = Registry::default_catalog();
    register(&mut registry);
    registry
}

/// Only the `ai.*` commands (documentation, tests).
#[must_use]
pub fn ai_catalog() -> Registry {
    let mut registry = Registry::new();
    register(&mut registry);
    registry
}
