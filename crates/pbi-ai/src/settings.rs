//! [`AiSettings`]: the opt-in switch, providers, per-capability limits and
//! recorded [`Consent`]s, stored as JSON at a caller-supplied path.
//!
//! The file never holds secrets: provider entries have no key field, and a
//! file or update that tries to add one is refused.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::capability::{Capability, DataExposure};
use crate::endpoint::Endpoint;
use crate::error::{AiError, ConsentScope};

/// Current settings format.
pub const SETTINGS_VERSION: u32 = 1;
/// Largest settings file read.
pub const MAX_SETTINGS_BYTES: u64 = 1 << 20;
/// Hard cap on `sampleRows`.
pub const MAX_SAMPLE_ROWS: usize = 100;
/// Default Ollama endpoint.
pub const OLLAMA_DEFAULT_ENDPOINT: &str = "http://127.0.0.1:11434";
/// Default OpenAI-compatible endpoint.
pub const OPENAI_DEFAULT_ENDPOINT: &str = "https://api.openai.com/v1";
/// Default Azure OpenAI `api-version` (GA, with structured outputs).
pub const AZURE_DEFAULT_API_VERSION: &str = "2024-10-21";

/// The kind of a provider (which adapter talks to it).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum ProviderKind {
    /// `POST {endpoint}/chat/completions` with a Bearer key: OpenAI, GitHub
    /// Models, LM Studio, vLLM, llama.cpp server, …
    OpenAiCompatible,
    /// `POST {endpoint}/openai/deployments/{deployment}/chat/completions`
    /// with an `api-key` header.
    AzureOpenAi,
    /// `POST {endpoint}/api/chat` (default `http://127.0.0.1:11434`), no
    /// key.
    Ollama,
    /// Deterministic canned answers, no network (tests and demos).
    Mock,
}

impl ProviderKind {
    /// `openAiCompatible`, …
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            ProviderKind::OpenAiCompatible => "openAiCompatible",
            ProviderKind::AzureOpenAi => "azureOpenAi",
            ProviderKind::Ollama => "ollama",
            ProviderKind::Mock => "mock",
        }
    }

    /// Requests carry an API key when one is available.
    #[must_use]
    pub fn uses_key(self) -> bool {
        matches!(
            self,
            ProviderKind::OpenAiCompatible | ProviderKind::AzureOpenAi
        )
    }
}

/// How the provider is asked for JSON.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum StructuredOutput {
    /// Send the output JSON Schema (`response_format: json_schema`, Ollama
    /// `format: <schema>`).
    #[default]
    JsonSchema,
    /// Ask for any JSON object (`json_object`, Ollama `format: "json"`).
    JsonObject,
    /// Prompt only; the answer's JSON is extracted (fenced block or first
    /// object).
    None,
}

/// One configured provider. Never holds a key.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProviderSettings {
    /// Stable id (`[a-z0-9][a-z0-9_-]*`, at most 64): consents and the
    /// keychain account (`ai:<id>`) refer to it.
    pub id: String,
    pub kind: ProviderKind,
    /// Base URL. Defaults: OpenAI-compatible `https://api.openai.com/v1`,
    /// Ollama `http://127.0.0.1:11434`; required for Azure
    /// (`https://<resource>.openai.azure.com`). Must be `https` unless the
    /// host is this machine or `allowHttp` is set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub endpoint: Option<String>,
    /// Model name (OpenAI-compatible, Ollama).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Azure deployment name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deployment: Option<String>,
    /// Azure `api-version` (default `2024-10-21`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_version: Option<String>,
    /// How to ask for JSON (default `jsonSchema`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub structured_output: Option<StructuredOutput>,
    /// Allow plain `http` to a host that is not this machine (e.g. a LAN
    /// vLLM server). Requests and keys then travel unencrypted.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub allow_http: bool,
    /// Display name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

impl ProviderSettings {
    /// The endpoint, with the kind's default.
    ///
    /// # Errors
    /// [`AiError::InvalidSettings`] for a missing (Azure) or invalid URL.
    pub fn endpoint(&self) -> Result<Endpoint, AiError> {
        let url = match (&self.endpoint, self.kind) {
            (_, ProviderKind::Mock) => return Ok(Endpoint::mock(&self.id)),
            (Some(url), _) => url.as_str(),
            (None, ProviderKind::OpenAiCompatible) => OPENAI_DEFAULT_ENDPOINT,
            (None, ProviderKind::Ollama) => OLLAMA_DEFAULT_ENDPOINT,
            (None, ProviderKind::AzureOpenAi) => {
                return Err(AiError::InvalidSettings(format!(
                    "provider '{}': azureOpenAi needs an endpoint (https://<resource>.openai.azure.com)",
                    self.id
                )));
            }
        };
        Endpoint::parse(url)
    }

    /// The model (or Azure deployment) requests name.
    #[must_use]
    pub fn model_name(&self) -> Option<&str> {
        match self.kind {
            ProviderKind::AzureOpenAi => self.deployment.as_deref().or(self.model.as_deref()),
            _ => self.model.as_deref(),
        }
    }

    fn validate(&self) -> Result<(), AiError> {
        let bad = |why: String| AiError::InvalidSettings(format!("provider '{}': {why}", self.id));
        if !valid_id(&self.id) {
            return Err(AiError::InvalidSettings(format!(
                "invalid provider id '{}': use 1–64 of a-z, 0-9, '-' and '_', starting with a \
                 letter or digit",
                self.id
            )));
        }
        let endpoint = self.endpoint()?;
        if endpoint.scheme() == "mock" && self.kind != ProviderKind::Mock {
            return Err(bad(
                "the mock:// scheme is reserved for the mock provider".into()
            ));
        }
        if endpoint.scheme() == "http" && !endpoint.is_loopback() && !self.allow_http {
            return Err(bad(format!(
                "{endpoint} is plain http to another machine; use https, or set \"allowHttp\": \
                 true to accept unencrypted requests"
            )));
        }
        match self.kind {
            ProviderKind::OpenAiCompatible | ProviderKind::Ollama
                if self.model.as_deref().is_none_or(|m| m.trim().is_empty()) =>
            {
                Err(bad(format!("{} needs a model", self.kind.as_str())))
            }
            ProviderKind::AzureOpenAi
                if self
                    .deployment
                    .as_deref()
                    .is_none_or(|d| d.trim().is_empty()) =>
            {
                Err(bad("azureOpenAi needs a deployment".into()))
            }
            _ => Ok(()),
        }?;
        for (name, value) in [
            ("model", &self.model),
            ("deployment", &self.deployment),
            ("apiVersion", &self.api_version),
        ] {
            if value
                .as_deref()
                .is_some_and(|v| v.len() > 200 || v.chars().any(|c| c.is_control() || c == '/'))
            {
                return Err(bad(format!("invalid {name}")));
            }
        }
        Ok(())
    }
}

fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        && id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '-' | '_'))
}

/// Limits of one capability (unset fields take the defaults).
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CapabilityLimits {
    /// Most tokens the model may answer (default 2000, at most 32768).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u32>,
    /// Sampling temperature, 0–2 (default 0.2).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f64>,
    /// Request timeout in seconds, 1–600 (default 120).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_secs: Option<u64>,
    /// Largest prompt in characters (default 60000, at most 1000000).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_input_chars: Option<usize>,
}

/// [`CapabilityLimits`] with the defaults applied.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct EffectiveLimits {
    pub max_tokens: u32,
    pub temperature: f64,
    pub timeout_secs: u64,
    pub max_input_chars: usize,
}

impl CapabilityLimits {
    /// With defaults applied.
    #[must_use]
    pub fn effective(&self) -> EffectiveLimits {
        EffectiveLimits {
            max_tokens: self.max_tokens.unwrap_or(2000),
            temperature: self.temperature.unwrap_or(0.2),
            timeout_secs: self.timeout_secs.unwrap_or(120),
            max_input_chars: self.max_input_chars.unwrap_or(60_000),
        }
    }

    fn validate(&self, capability: Capability) -> Result<(), AiError> {
        let bad = |why: &str| AiError::InvalidSettings(format!("limits.{capability}: {why}"));
        if self.max_tokens.is_some_and(|t| t == 0 || t > 32_768) {
            return Err(bad("maxTokens must be 1–32768"));
        }
        if self
            .temperature
            .is_some_and(|t| !t.is_finite() || !(0.0..=2.0).contains(&t))
        {
            return Err(bad("temperature must be 0–2"));
        }
        if self.timeout_secs.is_some_and(|t| t == 0 || t > 600) {
            return Err(bad("timeoutSecs must be 1–600"));
        }
        if self
            .max_input_chars
            .is_some_and(|c| !(1000..=1_000_000).contains(&c))
        {
            return Err(bad("maxInputChars must be 1000–1000000"));
        }
        Ok(())
    }
}

/// A recorded permission to send one capability's data of an exposure class
/// to one provider endpoint.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Consent {
    /// Provider id.
    pub provider: String,
    /// `host[:port]` the consent was granted for; a changed endpoint needs
    /// a new consent.
    pub endpoint_host: String,
    pub capability: Capability,
    /// The most the provider may receive for this capability.
    pub exposure: DataExposure,
    /// The endpoint is this machine (data does not leave it).
    pub local: bool,
    /// When it was granted (RFC 3339, UTC).
    pub granted_at: String,
    /// Who granted it (`cli`, `app`, `rpc`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub granted_by: Option<String>,
}

impl Consent {
    /// Allows `scope`.
    #[must_use]
    pub fn allows(&self, scope: &ConsentScope) -> bool {
        self.provider == scope.provider
            && self
                .endpoint_host
                .eq_ignore_ascii_case(&scope.endpoint_host)
            && self.capability == scope.capability
            && self.exposure.covers(scope.exposure)
    }
}

/// The AI settings file (`ai.json`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AiSettings {
    /// Format version (1).
    #[serde(default = "settings_version")]
    pub version: u32,
    /// Master switch (default `false`): nothing is sent while off.
    #[serde(default)]
    pub enabled: bool,
    /// Provider used when a command names none (default: the first).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_provider: Option<String>,
    #[serde(default)]
    pub providers: Vec<ProviderSettings>,
    /// Per-capability limits, keyed by capability id.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub limits: BTreeMap<Capability, CapabilityLimits>,
    /// Most sample rows a request may include (default 0: none; at most
    /// 100). Sample rows also need a `sample` consent.
    #[serde(default)]
    pub sample_rows: usize,
    /// Recorded consents (changed only through `ai.consent.*`).
    #[serde(default)]
    pub consents: Vec<Consent>,
}

fn settings_version() -> u32 {
    SETTINGS_VERSION
}

impl Default for AiSettings {
    fn default() -> Self {
        Self {
            version: SETTINGS_VERSION,
            enabled: false,
            default_provider: None,
            providers: Vec::new(),
            limits: BTreeMap::new(),
            sample_rows: 0,
            consents: Vec::new(),
        }
    }
}

/// Names that look like secrets; refused in the settings file.
const SECRET_KEYS: &[&str] = &[
    "apikey",
    "api_key",
    "api-key",
    "key",
    "secret",
    "token",
    "password",
    "authorization",
    "bearer",
];

fn secret_field(value: &Value) -> Option<String> {
    match value {
        Value::Object(map) => map.iter().find_map(|(k, v)| {
            let lower = k.to_ascii_lowercase();
            if SECRET_KEYS.contains(&lower.as_str()) || lower.ends_with("apikey") {
                Some(k.clone())
            } else {
                secret_field(v)
            }
        }),
        Value::Array(items) => items.iter().find_map(secret_field),
        _ => None,
    }
}

impl AiSettings {
    /// Parses settings JSON and validates it.
    ///
    /// # Errors
    /// [`AiError::InvalidSettings`]; the message never repeats values.
    pub fn from_json(value: Value) -> Result<Self, AiError> {
        if let Some(field) = secret_field(&value) {
            return Err(AiError::InvalidSettings(format!(
                "the AI settings may not contain secrets (found a '{field}' field): API keys go \
                 in the OS keychain (`dashcraft ai login`, the app) or DASHCRAFT_AI_API_KEY"
            )));
        }
        let settings: AiSettings = serde_json::from_value(value)
            .map_err(|e| AiError::InvalidSettings(format!("invalid AI settings: {e}")))?;
        settings.validate()?;
        Ok(settings)
    }

    /// Checks ids, endpoints, limits and references.
    ///
    /// # Errors
    /// [`AiError::InvalidSettings`].
    pub fn validate(&self) -> Result<(), AiError> {
        if self.version != SETTINGS_VERSION {
            return Err(AiError::InvalidSettings(format!(
                "unsupported AI settings version {} (expected {SETTINGS_VERSION})",
                self.version
            )));
        }
        let mut seen = std::collections::HashSet::new();
        for p in &self.providers {
            p.validate()?;
            if !seen.insert(p.id.as_str()) {
                return Err(AiError::InvalidSettings(format!(
                    "provider id '{}' is used twice",
                    p.id
                )));
            }
        }
        if let Some(d) = &self.default_provider
            && !seen.contains(d.as_str())
        {
            return Err(AiError::InvalidSettings(format!(
                "defaultProvider '{d}' is not a configured provider"
            )));
        }
        for (c, l) in &self.limits {
            l.validate(*c)?;
        }
        if self.sample_rows > MAX_SAMPLE_ROWS {
            return Err(AiError::InvalidSettings(format!(
                "sampleRows is {}; the maximum is {MAX_SAMPLE_ROWS}",
                self.sample_rows
            )));
        }
        Ok(())
    }

    /// The limits of `capability` with defaults applied.
    #[must_use]
    pub fn limits_for(&self, capability: Capability) -> EffectiveLimits {
        self.limits
            .get(&capability)
            .copied()
            .unwrap_or_default()
            .effective()
    }

    /// The provider `id`, or the default one.
    ///
    /// # Errors
    /// [`AiError::NoProvider`], [`AiError::UnknownProvider`].
    pub fn provider(&self, id: Option<&str>) -> Result<&ProviderSettings, AiError> {
        let wanted = id.map(str::trim).filter(|s| !s.is_empty());
        let wanted = wanted.or(self.default_provider.as_deref());
        match wanted {
            Some(w) => {
                self.providers
                    .iter()
                    .find(|p| p.id == w)
                    .ok_or_else(|| AiError::UnknownProvider {
                        name: w.to_owned(),
                        known: self.provider_ids(),
                    })
            }
            None => self.providers.first().ok_or(AiError::NoProvider),
        }
    }

    fn provider_ids(&self) -> String {
        if self.providers.is_empty() {
            "none".to_owned()
        } else {
            self.providers
                .iter()
                .map(|p| p.id.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        }
    }

    /// The consent that allows `scope`, if any.
    #[must_use]
    pub fn consent_for(&self, scope: &ConsentScope) -> Option<&Consent> {
        self.consents.iter().find(|c| c.allows(scope))
    }

    /// Records a consent (replacing one for the same provider, host and
    /// capability).
    ///
    /// # Errors
    /// [`AiError::UnknownProvider`]; [`AiError::InvalidInput`] for
    /// [`DataExposure::Full`] or an exposure above the capability's maximum.
    pub fn grant(
        &mut self,
        provider: &str,
        capability: Capability,
        exposure: DataExposure,
        granted_by: Option<&str>,
    ) -> Result<Consent, AiError> {
        if exposure > capability.max_exposure() {
            return Err(AiError::InvalidInput(format!(
                "{capability} never sends more than {}; refusing to grant {}",
                capability.max_exposure(),
                exposure
            )));
        }
        let p = self.provider(Some(provider))?;
        let endpoint = p.endpoint()?;
        let consent = Consent {
            provider: p.id.clone(),
            endpoint_host: endpoint.host_port(),
            capability,
            exposure,
            local: endpoint.is_local(),
            granted_at: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
            granted_by: granted_by.map(str::to_owned),
        };
        self.consents.retain(|c| {
            !(c.provider == consent.provider
                && c.endpoint_host == consent.endpoint_host
                && c.capability == capability)
        });
        self.consents.push(consent.clone());
        Ok(consent)
    }

    /// Removes the consents of `provider` (for `capability`, or all).
    /// Returns how many were removed.
    pub fn revoke(&mut self, provider: &str, capability: Option<Capability>) -> usize {
        let before = self.consents.len();
        self.consents.retain(|c| {
            !(c.provider == provider && capability.is_none_or(|cap| c.capability == cap))
        });
        before - self.consents.len()
    }

    /// Drops consents whose provider is gone or whose endpoint host changed.
    /// Returns how many were dropped.
    pub fn prune_consents(&mut self) -> usize {
        let before = self.consents.len();
        let providers = self.providers.clone();
        self.consents.retain(|c| {
            providers.iter().any(|p| {
                p.id == c.provider
                    && p.endpoint()
                        .is_ok_and(|e| e.host_port().eq_ignore_ascii_case(&c.endpoint_host))
            })
        });
        before - self.consents.len()
    }

    /// Reads `path`; a missing file gives the defaults (disabled).
    ///
    /// # Errors
    /// [`AiError::SettingsIo`] (unreadable, too large),
    /// [`AiError::InvalidSettings`].
    pub fn load(path: &Path) -> Result<Self, AiError> {
        let meta = match fs::metadata(path) {
            Ok(m) => m,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(e) => {
                return Err(AiError::SettingsIo(format!(
                    "cannot read {}: {e}",
                    path.display()
                )));
            }
        };
        if meta.len() > MAX_SETTINGS_BYTES {
            return Err(AiError::SettingsIo(format!(
                "{} is larger than {MAX_SETTINGS_BYTES} bytes",
                path.display()
            )));
        }
        let text = fs::read_to_string(path)
            .map_err(|e| AiError::SettingsIo(format!("cannot read {}: {e}", path.display())))?;
        let value: Value = serde_json::from_str(&text).map_err(|e| {
            AiError::InvalidSettings(format!("{} is not valid JSON: {e}", path.display()))
        })?;
        Self::from_json(value).map_err(|e| match e {
            AiError::InvalidSettings(m) => {
                AiError::InvalidSettings(format!("{}: {m}", path.display()))
            }
            other => other,
        })
    }

    /// Validates and writes `path` atomically (temporary file + rename),
    /// creating its folder; owner-only permissions on Unix.
    ///
    /// # Errors
    /// [`AiError::InvalidSettings`], [`AiError::SettingsIo`].
    pub fn save(&self, path: &Path) -> Result<(), AiError> {
        self.validate()?;
        let io = |what: &str, e: std::io::Error| {
            AiError::SettingsIo(format!("cannot {what} {}: {e}", path.display()))
        };
        let dir = path
            .parent()
            .filter(|d| !d.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        fs::create_dir_all(dir).map_err(|e| io("create the folder of", e))?;
        let mut text = serde_json::to_string_pretty(self)
            .map_err(|e| AiError::Internal(format!("cannot serialize the AI settings: {e}")))?;
        text.push('\n');
        let file_name = path
            .file_name()
            .map_or_else(|| OsString::from("ai.json"), ToOwned::to_owned);
        let mut tmp_name = OsString::from(".");
        tmp_name.push(&file_name);
        tmp_name.push(format!(".{}.tmp", std::process::id()));
        let tmp = dir.join(tmp_name);
        let write = || -> std::io::Result<()> {
            let mut f = fs::File::create(&tmp)?;
            restrict_permissions(&f)?;
            f.write_all(text.as_bytes())?;
            f.sync_all()?;
            fs::rename(&tmp, path)
        };
        write().map_err(|e| {
            let _ = fs::remove_file(&tmp);
            io("write", e)
        })
    }
}

#[cfg(unix)]
fn restrict_permissions(f: &fs::File) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    f.set_permissions(fs::Permissions::from_mode(0o600))
}

#[cfg(not(unix))]
fn restrict_permissions(_: &fs::File) -> std::io::Result<()> {
    Ok(())
}

/// The default settings file: `$DASHCRAFT_CONFIG_DIR/ai.json`, else `dashcraft/ai.json` under the
/// platform configuration folder (`$XDG_CONFIG_HOME` or `~/.config` on
/// Linux, `~/Library/Application Support` on macOS, `%APPDATA%` on Windows).
/// `None` when no folder can be found.
#[must_use]
pub fn default_settings_path() -> Option<PathBuf> {
    settings_path_from(|name| std::env::var_os(name), std::env::consts::OS)
}

/// [`default_settings_path`] with an explicit environment and OS (testable).
#[must_use]
pub fn settings_path_from(env: impl Fn(&str) -> Option<OsString>, os: &str) -> Option<PathBuf> {
    let var = |name: &str| env(name).filter(|v| !v.is_empty()).map(PathBuf::from);
    if let Some(dir) = var("DASHCRAFT_CONFIG_DIR") {
        return Some(dir.join("ai.json"));
    }
    let base = match os {
        "windows" => var("APPDATA"),
        "macos" => var("HOME").map(|h| h.join("Library").join("Application Support")),
        _ => var("XDG_CONFIG_HOME").or_else(|| var("HOME").map(|h| h.join(".config"))),
    }?;
    Some(base.join("dashcraft").join("ai.json"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn provider(id: &str, kind: ProviderKind, endpoint: Option<&str>) -> ProviderSettings {
        ProviderSettings {
            id: id.into(),
            kind,
            endpoint: endpoint.map(Into::into),
            model: Some("m".into()),
            deployment: Some("d".into()),
            api_version: None,
            structured_output: None,
            allow_http: false,
            label: None,
        }
    }

    #[test]
    fn defaults_are_off_and_empty() {
        let s = AiSettings::default();
        assert!(!s.enabled);
        assert_eq!(s.sample_rows, 0);
        assert!(s.providers.is_empty() && s.consents.is_empty());
        let parsed = AiSettings::from_json(json!({})).unwrap();
        assert_eq!(parsed, s);
        let l = s.limits_for(Capability::DaxExplain);
        assert_eq!((l.max_tokens, l.timeout_secs), (2000, 120));
    }

    #[test]
    fn secrets_are_refused_without_echoing_them() {
        let e = AiSettings::from_json(json!({
            "providers": [{"id": "openai", "kind": "openAiCompatible", "model": "x", "apiKey": "sk-SECRET-123"}]
        }))
        .unwrap_err();
        let text = e.to_string();
        assert!(
            text.contains("apiKey") && text.contains("keychain"),
            "{text}"
        );
        assert!(!text.contains("sk-SECRET-123"));
    }

    #[test]
    fn providers_are_validated() {
        let ok = |p: ProviderSettings| {
            AiSettings {
                providers: vec![p],
                ..AiSettings::default()
            }
            .validate()
        };
        assert!(ok(provider("ollama", ProviderKind::Ollama, None)).is_ok());
        assert!(
            ok(provider(
                "lan",
                ProviderKind::OpenAiCompatible,
                Some("http://10.0.0.5/v1")
            ))
            .is_err()
        );
        let mut lan = provider(
            "lan",
            ProviderKind::OpenAiCompatible,
            Some("http://10.0.0.5/v1"),
        );
        lan.allow_http = true;
        assert!(ok(lan).is_ok());
        assert!(ok(provider("Bad Id", ProviderKind::Ollama, None)).is_err());
        assert!(ok(provider("az", ProviderKind::AzureOpenAi, None)).is_err());
        assert!(
            ok(provider(
                "az",
                ProviderKind::AzureOpenAi,
                Some("https://r.openai.azure.com")
            ))
            .is_ok()
        );
        let dup = AiSettings {
            providers: vec![
                provider("a", ProviderKind::Ollama, None),
                provider("a", ProviderKind::Mock, None),
            ],
            ..AiSettings::default()
        };
        assert!(dup.validate().is_err());
    }

    #[test]
    fn consents_bind_provider_host_capability_and_exposure() {
        let mut s = AiSettings {
            providers: vec![provider("local", ProviderKind::Ollama, None)],
            ..AiSettings::default()
        };
        let scope = ConsentScope {
            provider: "local".into(),
            endpoint_host: "127.0.0.1:11434".into(),
            capability: Capability::DaxExplain,
            exposure: DataExposure::Metadata,
            local: true,
        };
        assert!(s.consent_for(&scope).is_none());
        let c = s
            .grant(
                "local",
                Capability::DaxExplain,
                DataExposure::Metadata,
                Some("cli"),
            )
            .unwrap();
        assert!(c.local);
        assert!(s.consent_for(&scope).is_some());
        // Another capability, a higher exposure or another host is not covered.
        let other = ConsentScope {
            capability: Capability::DaxGenerate,
            ..scope.clone()
        };
        assert!(s.consent_for(&other).is_none());
        let sample = ConsentScope {
            capability: Capability::DaxGenerate,
            exposure: DataExposure::Sample,
            ..scope.clone()
        };
        s.grant(
            "local",
            Capability::DaxGenerate,
            DataExposure::Metadata,
            None,
        )
        .unwrap();
        assert!(s.consent_for(&sample).is_none());
        let moved = ConsentScope {
            endpoint_host: "127.0.0.1:9999".into(),
            ..scope.clone()
        };
        assert!(s.consent_for(&moved).is_none());
        // Full can never be granted; explain never sends samples.
        assert!(
            s.grant("local", Capability::DaxExplain, DataExposure::Full, None)
                .is_err()
        );
        assert!(
            s.grant("local", Capability::DaxExplain, DataExposure::Sample, None)
                .is_err()
        );
        // Granting again replaces; revoke removes.
        s.grant(
            "local",
            Capability::DaxExplain,
            DataExposure::Metadata,
            None,
        )
        .unwrap();
        assert_eq!(s.consents.len(), 2);
        assert_eq!(s.revoke("local", Some(Capability::DaxExplain)), 1);
        assert_eq!(s.revoke("local", None), 1);
        assert!(s.consents.is_empty());
    }

    #[test]
    fn endpoint_changes_prune_consents() {
        let mut s = AiSettings {
            providers: vec![provider("local", ProviderKind::Ollama, None)],
            ..AiSettings::default()
        };
        s.grant(
            "local",
            Capability::DaxExplain,
            DataExposure::Metadata,
            None,
        )
        .unwrap();
        assert_eq!(s.prune_consents(), 0);
        s.providers[0].endpoint = Some("http://localhost:11500".into());
        assert_eq!(s.prune_consents(), 1);
    }

    #[test]
    fn default_path_follows_the_platform() {
        let env = |pairs: &'static [(&'static str, &'static str)]| {
            move |name: &str| {
                pairs
                    .iter()
                    .find(|(k, _)| *k == name)
                    .map(|(_, v)| OsString::from(v))
            }
        };
        let p = |pairs, os| settings_path_from(env(pairs), os);
        assert_eq!(
            p(&[("DASHCRAFT_CONFIG_DIR", "/cfg"), ("HOME", "/h")], "linux"),
            Some(PathBuf::from("/cfg/ai.json"))
        );
        assert_eq!(
            p(&[("HOME", "/h")], "macos"),
            Some(PathBuf::from(
                "/h/Library/Application Support/dashcraft/ai.json"
            ))
        );
        assert_eq!(
            p(&[("HOME", "/h"), ("XDG_CONFIG_HOME", "/x")], "linux"),
            Some(PathBuf::from("/x/dashcraft/ai.json"))
        );
        assert_eq!(
            p(&[("HOME", "/h")], "linux"),
            Some(PathBuf::from("/h/.config/dashcraft/ai.json"))
        );
        assert_eq!(
            p(&[("APPDATA", "C:\\Users\\u\\AppData\\Roaming")], "windows"),
            Some(
                PathBuf::from("C:\\Users\\u\\AppData\\Roaming")
                    .join("dashcraft")
                    .join("ai.json")
            )
        );
        assert_eq!(p(&[], "linux"), None);
    }
}
