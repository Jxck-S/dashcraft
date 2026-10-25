//! API keys: the [`Secret`] wrapper, [`SecretStore`]s (OS keychain, memory)
//! and the [`Redactor`] that keeps keys out of errors, outputs and previews.
//!
//! Keys live only in the OS keychain (service [`KEYRING_SERVICE`], account
//! `ai:<provider-id>`) or, for CI and headless use, in the environment
//! variable [`API_KEY_ENV`]. They are never
//! written to the settings, logs, errors, previews or command outputs.

use std::collections::HashMap;
use std::fmt;
use std::sync::{Mutex, PoisonError};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::error::AiError;

/// Keychain service name.
pub const KEYRING_SERVICE: &str = "dashcraft";
/// Environment variable with an API key (CI, headless).
pub const API_KEY_ENV: &str = "DASHCRAFT_AI_API_KEY";
/// What replaces a secret in text.
pub const REDACTED: &str = "[redacted]";

/// The keychain account of a provider: `ai:<provider-id>`.
#[must_use]
pub fn keyring_account(provider_id: &str) -> String {
    format!("ai:{provider_id}")
}

/// A secret string. `Debug` and `Display` print [`REDACTED`]; it is not
/// `Serialize`. The buffer is overwritten (best effort) when dropped.
#[derive(Clone, PartialEq, Eq)]
pub struct Secret(String);

impl Secret {
    /// Wraps `value` (surrounding whitespace removed).
    #[must_use]
    pub fn new(value: impl Into<String>) -> Self {
        let value = value.into();
        let trimmed = value.trim();
        if trimmed.len() == value.len() {
            Self(value)
        } else {
            Self(trimmed.to_owned())
        }
    }

    /// The secret itself: only for building the request that carries it.
    #[must_use]
    pub fn expose(&self) -> &str {
        &self.0
    }

    /// Empty after trimming.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(REDACTED)
    }
}

impl fmt::Display for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(REDACTED)
    }
}

impl Drop for Secret {
    fn drop(&mut self) {
        let mut bytes = std::mem::take(&mut self.0).into_bytes();
        bytes.fill(0);
        std::hint::black_box(&bytes);
    }
}

/// Where an API key came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum KeySource {
    /// The provider kind sends no key.
    NotRequired,
    /// The OS keychain (or the host's secret store).
    Keychain,
    /// `DASHCRAFT_AI_API_KEY`.
    Environment,
    /// No key found.
    Missing,
}

/// Stores API keys per provider id.
pub trait SecretStore: Send + Sync + fmt::Debug {
    /// `keychain`, `memory`, …
    fn name(&self) -> &'static str;

    /// The key of `provider_id`, if stored.
    ///
    /// # Errors
    /// [`AiError::Keychain`] when the store is unavailable.
    fn get(&self, provider_id: &str) -> Result<Option<Secret>, AiError>;

    /// Stores (replaces) the key of `provider_id`.
    ///
    /// # Errors
    /// [`AiError::Keychain`].
    fn set(&self, provider_id: &str, key: &Secret) -> Result<(), AiError>;

    /// Deletes the key; `false` when there was none.
    ///
    /// # Errors
    /// [`AiError::Keychain`].
    fn delete(&self, provider_id: &str) -> Result<bool, AiError>;
}

/// The OS keychain (macOS Keychain, Windows Credential Manager, Secret
/// Service on Linux) through `keyring`.
#[derive(Debug, Default, Clone, Copy)]
pub struct KeychainStore;

fn keychain_error(e: &keyring::Error) -> AiError {
    AiError::Keychain(e.to_string())
}

impl KeychainStore {
    fn entry(provider_id: &str) -> Result<keyring::Entry, AiError> {
        keyring::Entry::new(KEYRING_SERVICE, &keyring_account(provider_id))
            .map_err(|e| keychain_error(&e))
    }
}

impl SecretStore for KeychainStore {
    fn name(&self) -> &'static str {
        "keychain"
    }

    fn get(&self, provider_id: &str) -> Result<Option<Secret>, AiError> {
        match Self::entry(provider_id)?.get_password() {
            Ok(key) => Ok(Some(Secret::new(key)).filter(|k| !k.is_empty())),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(keychain_error(&e)),
        }
    }

    fn set(&self, provider_id: &str, key: &Secret) -> Result<(), AiError> {
        Self::entry(provider_id)?
            .set_password(key.expose())
            .map_err(|e| keychain_error(&e))
    }

    fn delete(&self, provider_id: &str) -> Result<bool, AiError> {
        match Self::entry(provider_id)?.delete_credential() {
            Ok(()) => Ok(true),
            Err(keyring::Error::NoEntry) => Ok(false),
            Err(e) => Err(keychain_error(&e)),
        }
    }
}

/// An in-memory store (tests, embedding).
#[derive(Debug, Default)]
pub struct MemorySecretStore {
    keys: Mutex<HashMap<String, Secret>>,
}

impl MemorySecretStore {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// With `key` stored for `provider_id`.
    #[must_use]
    pub fn with(self, provider_id: &str, key: &str) -> Self {
        self.keys
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(provider_id.to_owned(), Secret::new(key));
        self
    }
}

impl SecretStore for MemorySecretStore {
    fn name(&self) -> &'static str {
        "memory"
    }

    fn get(&self, provider_id: &str) -> Result<Option<Secret>, AiError> {
        Ok(self
            .keys
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(provider_id)
            .cloned())
    }

    fn set(&self, provider_id: &str, key: &Secret) -> Result<(), AiError> {
        self.keys
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(provider_id.to_owned(), key.clone());
        Ok(())
    }

    fn delete(&self, provider_id: &str) -> Result<bool, AiError> {
        Ok(self
            .keys
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(provider_id)
            .is_some())
    }
}

/// Reads an environment variable (replaceable in tests).
pub type EnvReader = std::sync::Arc<dyn Fn(&str) -> Option<String> + Send + Sync>;

/// The key of `provider_id`: the store first, then [`API_KEY_ENV`]. A failing store (e.g. no Secret Service on a
/// headless Linux box) falls back to the environment; its error is returned
/// as the detail when no key is found.
pub(crate) fn resolve_key(
    store: &dyn SecretStore,
    env: &EnvReader,
    provider_id: &str,
) -> (Option<Secret>, KeySource, Option<String>) {
    let mut detail = None;
    match store.get(provider_id) {
        Ok(Some(key)) => return (Some(key), KeySource::Keychain, None),
        Ok(None) => {}
        Err(e) => {
            tracing::debug!("AI key lookup in the {} failed: {e}", store.name());
            detail = Some(e.to_string());
        }
    }
    if let Some(key) = env(API_KEY_ENV).map(Secret::new).filter(|k| !k.is_empty()) {
        return (Some(key), KeySource::Environment, None);
    }
    (None, KeySource::Missing, detail)
}

/// Replaces known secrets (and anything after `Bearer ` or `api-key:`) in
/// text.
#[derive(Debug, Clone, Default)]
pub struct Redactor {
    secrets: Vec<Secret>,
}

impl Redactor {
    /// Redacts `secret` (ignored when shorter than 4 characters, which
    /// would redact ordinary text).
    #[must_use]
    pub fn with(mut self, secret: Option<&Secret>) -> Self {
        if let Some(s) = secret
            && s.expose().len() >= 4
        {
            self.secrets.push(s.clone());
        }
        self
    }

    /// `text` with every known secret and every `Bearer`/`api-key` value
    /// replaced (for errors and HTTP bodies).
    #[must_use]
    pub fn redact(&self, text: &str) -> String {
        redact_auth_values(&self.redact_known(text))
    }

    /// `text` with every known secret replaced (for model answers, where
    /// prose may mention "Bearer" legitimately).
    #[must_use]
    pub fn redact_known(&self, text: &str) -> String {
        let mut out = text.to_owned();
        for s in &self.secrets {
            if out.contains(s.expose()) {
                out = out.replace(s.expose(), REDACTED);
            }
        }
        out
    }
}

/// Masks the value after `Bearer ` and `api-key:` / `"api-key": "` (header
/// echoes in error bodies).
fn redact_auth_values(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    loop {
        let lower = rest.to_ascii_lowercase();
        let hit = ["bearer ", "api-key: ", "api-key\":\"", "api-key\": \""]
            .iter()
            .filter_map(|m| lower.find(m).map(|i| (i, m.len())))
            .min();
        let Some((i, len)) = hit else {
            out.push_str(rest);
            return out;
        };
        let start = i + len;
        out.push_str(rest.get(..start).unwrap_or_default());
        let tail = rest.get(start..).unwrap_or_default();
        let value_len = tail
            .find(|c: char| c.is_whitespace() || matches!(c, '"' | '\'' | ',' | '}'))
            .unwrap_or(tail.len());
        if value_len > 0 && tail.get(..value_len) != Some(REDACTED) {
            out.push_str(REDACTED);
        } else {
            out.push_str(tail.get(..value_len).unwrap_or_default());
        }
        rest = tail.get(value_len..).unwrap_or_default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn secrets_never_print() {
        let s = Secret::new("  sk-live-ABCDEF  ");
        assert_eq!(s.expose(), "sk-live-ABCDEF");
        assert_eq!(format!("{s:?} {s}"), "[redacted] [redacted]");
        assert_eq!(keyring_account("openai"), "ai:openai");
    }

    #[test]
    fn redactor_masks_keys_and_auth_headers() {
        let key = Secret::new("sk-live-ABCDEF");
        let r = Redactor::default().with(Some(&key));
        let text = "bad key sk-live-ABCDEF; Authorization: Bearer xyz123, {\"api-key\": \"abc\"} api-key: q1";
        let red = r.redact(text);
        assert!(!red.contains("sk-live-ABCDEF"), "{red}");
        assert!(
            !red.contains("xyz123") && !red.contains("abc\"") && !red.contains("q1"),
            "{red}"
        );
        assert!(red.contains("Bearer [redacted]"), "{red}");
        // Short "secrets" are not redacted (they would mangle ordinary text).
        let r = Redactor::default().with(Some(&Secret::new("a")));
        assert_eq!(r.redact("a cat"), "a cat");
        assert_eq!(
            Redactor::default().redact("Bearer [redacted]"),
            "Bearer [redacted]"
        );
    }

    #[test]
    fn keys_come_from_the_store_then_the_environment() {
        let store = MemorySecretStore::new().with("openai", "sk-store");
        let env: EnvReader = Arc::new(|name| (name == API_KEY_ENV).then(|| "sk-env".to_owned()));
        let (k, src, _) = resolve_key(&store, &env, "openai");
        assert_eq!(
            (k.map(|k| k.expose().to_owned()), src),
            (Some("sk-store".into()), KeySource::Keychain)
        );
        let (k, src, _) = resolve_key(&store, &env, "other");
        assert_eq!(
            (k.map(|k| k.expose().to_owned()), src),
            (Some("sk-env".into()), KeySource::Environment)
        );
        let none: EnvReader = Arc::new(|_| None);
        let (k, src, _) = resolve_key(&store, &none, "other");
        assert!(k.is_none());
        assert_eq!(src, KeySource::Missing);
        assert!(store.delete("openai").unwrap());
        assert!(!store.delete("openai").unwrap());
    }
}
