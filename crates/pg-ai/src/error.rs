//! AI errors (Blueprint §10.1) and the plain sentences the UI shows for them.
//!
//! Every failure is an [`AiError`]; the caller falls back to non-AI behaviour. Errors are shown as a
//! non-blocking status such as "AI unavailable: invalid key", never as a modal that halts the sim.

use std::fmt;
use std::time::Duration;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AiError {
    /// The provider rejected the key (or the key lacks permission).
    Auth,
    /// No key is stored for the selected provider.
    NoKey,
    RateLimit {
        retry_after: Option<Duration>,
    },
    /// The account is out of credit or over its quota.
    Quota,
    Timeout,
    Offline,
    /// The provider answered, but not with a usable reply.
    BadOutput,
    /// Any other provider-side failure; the message is already redacted and length-capped.
    Provider(String),
    /// AI is switched off in settings.
    Disabled,
    /// The circuit breaker is open after repeated failures.
    CircuitOpen {
        retry_in: Duration,
    },
    Cancelled,
    /// The credential store could not be read.
    KeyStore(String),
}

impl AiError {
    /// Worth one automatic retry.
    pub fn is_transient(&self) -> bool {
        matches!(self, AiError::Timeout | AiError::Provider(_))
    }

    /// Whether this failure counts toward opening the circuit breaker. Configuration problems (a bad key,
    /// no credit, a rate limit, AI switched off) do not: retrying them is pointless and the player has to
    /// act, but they also say nothing about whether the provider is healthy.
    pub fn counts_against_breaker(&self) -> bool {
        matches!(
            self,
            AiError::Timeout | AiError::Offline | AiError::BadOutput | AiError::Provider(_)
        )
    }

    /// A short sentence for a status line.
    pub fn user_message(&self) -> String {
        match self {
            AiError::Auth => "AI unavailable: the provider rejected the key".to_owned(),
            AiError::NoKey => "AI unavailable: no key is set for this provider".to_owned(),
            AiError::RateLimit {
                retry_after: Some(d),
            } => {
                format!(
                    "AI is busy: rate limited, try again in {} s",
                    d.as_secs().max(1)
                )
            }
            AiError::RateLimit { retry_after: None } => "AI is busy: rate limited".to_owned(),
            AiError::Quota => {
                "AI unavailable: the account is out of credit or over its quota".to_owned()
            }
            AiError::Timeout => "AI unavailable: the provider did not answer in time".to_owned(),
            AiError::Offline => "AI unavailable: no network connection".to_owned(),
            AiError::BadOutput => {
                "AI unavailable: the provider's reply could not be used".to_owned()
            }
            AiError::Provider(m) => format!("AI unavailable: {m}"),
            AiError::Disabled => "AI is turned off".to_owned(),
            AiError::CircuitOpen { retry_in } => format!(
                "AI paused after repeated failures; trying again in {} s",
                retry_in.as_secs().max(1)
            ),
            AiError::Cancelled => "AI request cancelled".to_owned(),
            AiError::KeyStore(m) => format!("AI unavailable: {m}"),
        }
    }
}

impl fmt::Display for AiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.user_message())
    }
}

impl std::error::Error for AiError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn messages_are_plain_and_distinct() {
        let all = [
            AiError::Auth,
            AiError::NoKey,
            AiError::RateLimit {
                retry_after: Some(Duration::from_secs(7)),
            },
            AiError::RateLimit { retry_after: None },
            AiError::Quota,
            AiError::Timeout,
            AiError::Offline,
            AiError::BadOutput,
            AiError::Provider("model not found".into()),
            AiError::Disabled,
            AiError::CircuitOpen {
                retry_in: Duration::from_secs(20),
            },
            AiError::Cancelled,
            AiError::KeyStore("locked".into()),
        ];
        let texts: Vec<String> = all.iter().map(AiError::user_message).collect();
        let unique: std::collections::BTreeSet<&String> = texts.iter().collect();
        assert_eq!(unique.len(), texts.len(), "{texts:?}");
        assert!(texts[2].contains("7 s"));
        assert_eq!(
            AiError::Auth.to_string(),
            "AI unavailable: the provider rejected the key"
        );
    }

    #[test]
    fn only_provider_health_failures_count_against_the_breaker() {
        assert!(AiError::Timeout.counts_against_breaker());
        assert!(AiError::Provider("x".into()).counts_against_breaker());
        assert!(AiError::Offline.counts_against_breaker());
        assert!(AiError::BadOutput.counts_against_breaker());
        for e in [
            AiError::Auth,
            AiError::NoKey,
            AiError::Quota,
            AiError::Disabled,
            AiError::Cancelled,
            AiError::RateLimit { retry_after: None },
        ] {
            assert!(!e.counts_against_breaker(), "{e:?}");
        }
        assert!(AiError::Timeout.is_transient() && !AiError::Auth.is_transient());
    }
}
