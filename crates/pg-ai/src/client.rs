//! The AI client (Blueprint §10.1-§10.3): one safe-to-fail entry point for every AI call.
//!
//! A call goes through, in order: the on/off switch, the response cache, the circuit breaker, the
//! requests-per-minute cap, the key (read from the credential store at call time), the adapter, the
//! network (with a timeout and one retry on transient errors) and the adapter's parser. Any failure is an
//! [`AiError`]; the caller falls back to non-AI behaviour. Nothing in here logs a request body, a header or
//! a key, and every log line goes through `redact`.

use crate::breaker::{BreakerConfig, BreakerState, CircuitBreaker, Permit};
use crate::error::AiError;
use crate::provider::{adapter_for, AiTask, Provider};
use crate::settings::AiSettings;
use pg_core::canon::Canon;
use pg_core::hash::hash_canon;
use pg_host::{redact, CancelToken, Clock, Level, LogSink, Net, NetError, SecretStore};
use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

#[derive(Clone, Debug)]
pub struct ClientConfig {
    /// Per-request timeout (default 8 s).
    pub timeout: Duration,
    /// Automatic retries after a transient failure (default 1).
    pub retries: u32,
    /// Requests allowed per minute across all providers.
    pub requests_per_minute: usize,
    /// Entries kept in the response cache.
    pub cache_capacity: usize,
    /// Longest reply kept, in characters.
    pub max_reply_chars: usize,
    pub breaker: BreakerConfig,
}

impl Default for ClientConfig {
    fn default() -> Self {
        ClientConfig {
            timeout: Duration::from_secs(8),
            retries: 1,
            requests_per_minute: 20,
            cache_capacity: 64,
            max_reply_chars: 2_000,
            breaker: BreakerConfig::default(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AiReply {
    pub text: String,
    pub from_cache: bool,
}

/// What the options screen shows next to the provider choice.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AiStatus {
    Disabled,
    NoKey,
    Ready,
    /// The breaker is open; calls resume after `retry_in`.
    Paused {
        retry_in: Duration,
    },
}

#[derive(Default)]
struct Inner {
    breakers: BTreeMap<Provider, CircuitBreaker>,
    recent: VecDeque<Duration>,
    cache: VecDeque<(String, String)>,
}

pub struct AiClient {
    net: Arc<dyn Net>,
    secrets: Arc<dyn SecretStore>,
    clock: Arc<dyn Clock>,
    log: Arc<dyn LogSink>,
    cfg: ClientConfig,
    inner: Mutex<Inner>,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

impl AiClient {
    pub fn new(
        net: Arc<dyn Net>,
        secrets: Arc<dyn SecretStore>,
        clock: Arc<dyn Clock>,
        log: Arc<dyn LogSink>,
        cfg: ClientConfig,
    ) -> AiClient {
        AiClient {
            net,
            secrets,
            clock,
            log,
            cfg,
            inner: Mutex::new(Inner::default()),
        }
    }

    fn note(&self, level: Level, line: &str) {
        self.log.log(level, &redact(line, &[]));
    }

    /// The state to show for `settings`, without making a request.
    pub fn status(&self, settings: &AiSettings) -> AiStatus {
        if !settings.enabled {
            return AiStatus::Disabled;
        }
        if !matches!(
            self.secrets.get(&settings.provider.secret_name()),
            Ok(Some(_))
        ) {
            return AiStatus::NoKey;
        }
        let now = self.clock.now_monotonic();
        let state = lock(&self.inner)
            .breakers
            .get(&settings.provider)
            .map(CircuitBreaker::state);
        match state {
            Some(BreakerState::Open { until }) if until > now => AiStatus::Paused {
                retry_in: until - now,
            },
            _ => AiStatus::Ready,
        }
    }

    /// A tiny request that proves the key, the model and the connection work. Never cached, and it does not
    /// require AI to be switched on (the player tests before enabling).
    pub fn test_connection(
        &self,
        settings: &AiSettings,
        cancel: &CancelToken,
    ) -> Result<(), AiError> {
        let on = AiSettings {
            enabled: true,
            ..settings.clone()
        };
        self.run(&on, &AiTask::connection_test(), cancel, false)
            .map(|_| ())
    }

    /// Generates text for `task`.
    pub fn generate(
        &self,
        settings: &AiSettings,
        task: &AiTask,
        cancel: &CancelToken,
    ) -> Result<AiReply, AiError> {
        self.run(settings, task, cancel, true)
    }

    fn cache_key(provider: Provider, model: &str, task: &AiTask) -> String {
        hash_canon(&Canon::map([
            ("provider", Canon::str(provider.id())),
            ("model", Canon::str(model)),
            ("system", Canon::str(task.system.clone())),
            ("user", Canon::str(task.user.clone())),
            ("max_tokens", Canon::Int(i128::from(task.max_tokens))),
        ]))
        .to_hex()
    }

    fn run(
        &self,
        settings: &AiSettings,
        task: &AiTask,
        cancel: &CancelToken,
        use_cache: bool,
    ) -> Result<AiReply, AiError> {
        if !settings.enabled {
            return Err(AiError::Disabled);
        }
        let provider = settings.provider;
        let model = settings.effective_model();
        let cache_key = Self::cache_key(provider, &model, task);
        let now = self.clock.now_monotonic();

        // Cache, breaker and rate limit are decided under one lock so concurrent callers see a consistent view.
        {
            let mut g = lock(&self.inner);
            if use_cache {
                if let Some((_, text)) = g.cache.iter().find(|(k, _)| *k == cache_key) {
                    return Ok(AiReply {
                        text: text.clone(),
                        from_cache: true,
                    });
                }
            }
            let breaker = g
                .breakers
                .entry(provider)
                .or_insert_with(|| CircuitBreaker::new(self.cfg.breaker));
            let permit = breaker.permit(now);
            if let Permit::No { retry_in } = permit {
                return Err(AiError::CircuitOpen { retry_in });
            }
            let minute = Duration::from_secs(60);
            while g
                .recent
                .front()
                .is_some_and(|t| now.saturating_sub(*t) >= minute)
            {
                g.recent.pop_front();
            }
            if g.recent.len() >= self.cfg.requests_per_minute {
                let wait = g
                    .recent
                    .front()
                    .map_or(minute, |t| (*t + minute).saturating_sub(now));
                if let Some(b) = g.breakers.get_mut(&provider) {
                    b.record_neutral();
                }
                return Err(AiError::RateLimit {
                    retry_after: Some(wait),
                });
            }
            g.recent.push_back(now);
        }

        let result = self.attempt(provider, &model, task, cancel);
        let mut g = lock(&self.inner);
        let end = self.clock.now_monotonic();
        if let Some(b) = g.breakers.get_mut(&provider) {
            match &result {
                Ok(_) => b.record_success(),
                Err(e) if e.counts_against_breaker() => b.record_failure(end),
                Err(_) => b.record_neutral(),
            }
        }
        match result {
            Ok(text) => {
                if use_cache && self.cfg.cache_capacity > 0 {
                    while g.cache.len() >= self.cfg.cache_capacity {
                        g.cache.pop_front();
                    }
                    g.cache.push_back((cache_key, text.clone()));
                }
                drop(g);
                self.note(
                    Level::Info,
                    &format!("ai ok provider={} model={model}", provider.id()),
                );
                Ok(AiReply {
                    text,
                    from_cache: false,
                })
            }
            Err(e) => {
                drop(g);
                self.note(
                    Level::Warn,
                    &format!("ai failed provider={} model={model}: {e}", provider.id()),
                );
                Err(e)
            }
        }
    }

    fn attempt(
        &self,
        provider: Provider,
        model: &str,
        task: &AiTask,
        cancel: &CancelToken,
    ) -> Result<String, AiError> {
        let key = self
            .secrets
            .get(&provider.secret_name())
            .map_err(|e| AiError::KeyStore(e.to_string()))?
            .ok_or(AiError::NoKey)?;
        let adapter = adapter_for(provider);
        let mut last = AiError::Offline;
        for attempt in 0..=self.cfg.retries {
            if cancel.is_cancelled() {
                return Err(AiError::Cancelled);
            }
            let req = adapter.build_request(task, model, &key, self.cfg.timeout);
            let outcome = match self.net.request(&req, cancel) {
                Ok(resp) if (200..300).contains(&resp.status) => {
                    adapter.parse_response(&resp.body, self.cfg.max_reply_chars)
                }
                Ok(resp) => Err(adapter.classify_error(&resp)),
                Err(NetError::Timeout) => Err(AiError::Timeout),
                Err(NetError::Offline) => Err(AiError::Offline),
                Err(NetError::Cancelled) => Err(AiError::Cancelled),
                Err(NetError::Tls(e)) => Err(AiError::Provider(format!(
                    "secure connection failed: {}",
                    redact(&e, &[])
                ))),
                Err(NetError::Blocked(e)) => Err(AiError::Provider(format!(
                    "request blocked: {}",
                    redact(&e, &[])
                ))),
                Err(NetError::Other(e)) => Err(AiError::Provider(
                    redact(&e, &[]).chars().take(200).collect(),
                )),
            };
            match outcome {
                Ok(text) => return Ok(text),
                Err(e) if e.is_transient() && attempt < self.cfg.retries => last = e,
                Err(e) => return Err(e),
            }
        }
        Err(last)
    }
}

#[cfg(test)]
mod tests;
