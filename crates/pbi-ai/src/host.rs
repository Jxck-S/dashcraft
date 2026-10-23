//! [`AiHost`]: where the AI layer keeps its settings and keys and how it
//! builds providers. Registry commands are plain functions without state,
//! so the host is process-wide ([`AiHost::install`]) with a per-task
//! override ([`AiHost::scope`], for tests and embedders). Without either,
//! the default host uses [`default_settings_path`], the OS keychain, the
//! process environment and the built-in adapters.

use std::fmt;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError, RwLock};

use crate::capability::{Capability, DataExposure};
use crate::error::AiError;
use crate::provider::{DefaultProviderFactory, ProviderFactory};
use crate::secrets::{EnvReader, KeySource, KeychainStore, Secret, SecretStore, resolve_key};
use crate::settings::{AiSettings, Consent, ProviderSettings, default_settings_path};

struct HostInner {
    settings_path: Option<PathBuf>,
    secrets: Arc<dyn SecretStore>,
    providers: Arc<dyn ProviderFactory>,
    env: EnvReader,
}

/// Serializes read-modify-write of settings files in this process.
static WRITE_LOCK: Mutex<()> = Mutex::new(());

/// The AI layer's environment. Cheap to clone.
#[derive(Clone)]
pub struct AiHost {
    inner: Arc<HostInner>,
}

impl fmt::Debug for AiHost {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AiHost")
            .field("settings_path", &self.inner.settings_path)
            .field("secrets", &self.inner.secrets.name())
            .field("providers", &self.inner.providers)
            .finish_non_exhaustive()
    }
}

static GLOBAL: RwLock<Option<AiHost>> = RwLock::new(None);

tokio::task_local! {
    static SCOPED: AiHost;
}

impl AiHost {
    /// Settings at `settings_path`; OS keychain, process environment,
    /// built-in adapters.
    #[must_use]
    pub fn new(settings_path: impl Into<PathBuf>) -> Self {
        Self::build(Some(settings_path.into()))
    }

    /// Settings at [`default_settings_path`].
    #[must_use]
    pub fn from_default_path() -> Self {
        Self::build(default_settings_path())
    }

    fn build(settings_path: Option<PathBuf>) -> Self {
        Self {
            inner: Arc::new(HostInner {
                settings_path,
                secrets: Arc::new(KeychainStore),
                providers: Arc::new(DefaultProviderFactory),
                env: Arc::new(|name| std::env::var(name).ok()),
            }),
        }
    }

    fn map(self, f: impl FnOnce(&mut HostInner)) -> Self {
        let mut inner = HostInner {
            settings_path: self.inner.settings_path.clone(),
            secrets: Arc::clone(&self.inner.secrets),
            providers: Arc::clone(&self.inner.providers),
            env: Arc::clone(&self.inner.env),
        };
        f(&mut inner);
        Self {
            inner: Arc::new(inner),
        }
    }

    /// Uses `store` for API keys instead of the OS keychain.
    #[must_use]
    pub fn with_secret_store(self, store: impl SecretStore + 'static) -> Self {
        self.map(|i| i.secrets = Arc::new(store))
    }

    /// Builds providers with `factory` (e.g. one returning a
    /// [`MockProvider`](crate::MockProvider)).
    #[must_use]
    pub fn with_provider_factory(self, factory: impl ProviderFactory + 'static) -> Self {
        self.map(|i| i.providers = Arc::new(factory))
    }

    /// Reads environment variables through `env` (tests).
    #[must_use]
    pub fn with_env(self, env: impl Fn(&str) -> Option<String> + Send + Sync + 'static) -> Self {
        self.map(|i| i.env = Arc::new(env))
    }

    /// Makes `self` the process-wide host (front ends call this once at
    /// start-up, e.g. with the app's configuration folder).
    pub fn install(self) {
        *GLOBAL.write().unwrap_or_else(PoisonError::into_inner) = Some(self);
    }

    /// Runs `fut` with `self` as the host of the current task (commands
    /// awaited inside see it; spawned tasks do not).
    pub async fn scope<F: Future>(self, fut: F) -> F::Output {
        SCOPED.scope(self, fut).await
    }

    /// The host in effect: the task's, else the installed one, else a
    /// default host.
    #[must_use]
    pub fn current() -> Self {
        if let Ok(host) = SCOPED.try_with(Clone::clone) {
            return host;
        }
        if let Some(host) = GLOBAL
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .as_ref()
        {
            return host.clone();
        }
        let host = Self::from_default_path();
        let mut global = GLOBAL.write().unwrap_or_else(PoisonError::into_inner);
        global.get_or_insert(host).clone()
    }

    /// The settings file, if one could be determined.
    #[must_use]
    pub fn settings_path(&self) -> Option<&Path> {
        self.inner.settings_path.as_deref()
    }

    fn require_path(&self) -> Result<&Path, AiError> {
        self.settings_path().ok_or(AiError::NoSettingsPath)
    }

    /// The settings (defaults when the file does not exist).
    ///
    /// # Errors
    /// [`AiError::SettingsIo`], [`AiError::InvalidSettings`]; none without a
    /// settings path (then AI stays disabled).
    pub fn load_settings(&self) -> Result<AiSettings, AiError> {
        match self.settings_path() {
            Some(p) => AiSettings::load(p),
            None => Ok(AiSettings::default()),
        }
    }

    /// Loads, changes and saves the settings under a process-wide lock.
    ///
    /// # Errors
    /// `f`'s error, [`AiError::NoSettingsPath`], load and save errors.
    pub fn update_settings<T>(
        &self,
        f: impl FnOnce(&mut AiSettings) -> Result<T, AiError>,
    ) -> Result<T, AiError> {
        let path = self.require_path()?;
        let _guard = WRITE_LOCK.lock().unwrap_or_else(PoisonError::into_inner);
        let mut settings = AiSettings::load(path)?;
        let out = f(&mut settings)?;
        settings.save(path)?;
        Ok(out)
    }

    /// Records a consent (see [`AiSettings::grant`]); `granted_by` names
    /// the surface (`cli`, `app`, …). For people only: never call this on
    /// an agent's behalf.
    ///
    /// # Errors
    /// [`AiError::UnknownProvider`], [`AiError::InvalidInput`] (exposure not
    /// allowed for the capability), settings errors.
    pub fn grant_consent(
        &self,
        provider: &str,
        capability: Capability,
        exposure: DataExposure,
        granted_by: Option<&str>,
    ) -> Result<Consent, AiError> {
        self.update_settings(|s| s.grant(provider, capability, exposure, granted_by))
    }

    /// Removes the consents of `provider` (for `capability`, or all);
    /// returns how many were removed.
    ///
    /// # Errors
    /// Settings errors.
    pub fn revoke_consent(
        &self,
        provider: &str,
        capability: Option<Capability>,
    ) -> Result<usize, AiError> {
        self.update_settings(|s| Ok(s.revoke(provider, capability)))
    }

    /// Turns AI on or off.
    ///
    /// # Errors
    /// Settings errors.
    pub fn set_enabled(&self, enabled: bool) -> Result<(), AiError> {
        self.update_settings(|s| {
            s.enabled = enabled;
            Ok(())
        })
    }

    /// The secret store.
    #[must_use]
    pub fn secrets(&self) -> &dyn SecretStore {
        self.inner.secrets.as_ref()
    }

    /// The provider factory.
    #[must_use]
    pub fn providers(&self) -> &dyn ProviderFactory {
        self.inner.providers.as_ref()
    }

    /// The API key of `provider` and where it came from (`None` when its
    /// kind sends no key).
    pub(crate) fn api_key(
        &self,
        provider: &ProviderSettings,
    ) -> (Option<Secret>, KeySource, Option<String>) {
        if !provider.kind.uses_key() {
            return (None, KeySource::NotRequired, None);
        }
        resolve_key(self.secrets(), &self.inner.env, &provider.id)
    }

    /// Stores the API key of `provider_id` in this host's secret store (the
    /// OS keychain by default). The provider does not have to be
    /// configured yet.
    ///
    /// # Errors
    /// [`AiError::InvalidInput`] for an empty key or a bad id,
    /// [`AiError::Keychain`].
    pub fn store_api_key(&self, provider_id: &str, key: &str) -> Result<(), AiError> {
        check_provider_id(provider_id)?;
        let key = Secret::new(key);
        if key.is_empty() {
            return Err(AiError::InvalidInput("the API key is empty".into()));
        }
        if key.expose().chars().any(char::is_control) {
            return Err(AiError::InvalidInput(
                "the API key contains control characters".into(),
            ));
        }
        self.secrets().set(provider_id, &key)
    }

    /// Deletes the stored key of `provider_id`; `false` when there was none.
    ///
    /// # Errors
    /// [`AiError::InvalidInput`] for a bad id, [`AiError::Keychain`].
    pub fn clear_api_key(&self, provider_id: &str) -> Result<bool, AiError> {
        check_provider_id(provider_id)?;
        self.secrets().delete(provider_id)
    }
}

fn check_provider_id(id: &str) -> Result<(), AiError> {
    let ok = !id.is_empty()
        && id.len() <= 64
        && id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '-' | '_'));
    if ok {
        Ok(())
    } else {
        Err(AiError::InvalidInput(format!("invalid provider id '{id}'")))
    }
}

/// Stores the API key of `provider_id` in the current host's secret store
/// (the OS keychain: service `dashcraft`, account `ai:<provider-id>`). For
/// `dashcraft ai login` and the app's settings page; never exposed to agents.
///
/// # Errors
/// See [`AiHost::store_api_key`].
pub fn store_api_key(provider_id: &str, key: &str) -> Result<(), AiError> {
    AiHost::current().store_api_key(provider_id, key)
}

/// Deletes the stored API key of `provider_id`.
///
/// # Errors
/// See [`AiHost::clear_api_key`].
pub fn clear_api_key(provider_id: &str) -> Result<bool, AiError> {
    AiHost::current().clear_api_key(provider_id)
}
