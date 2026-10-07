//! The four providers and their adapters (Blueprint §10.1): OpenAI, DeepSeek, Anthropic, OpenRouter.
//!
//! An adapter is a **pure** request builder and response parser: no network, no clock, no key storage. It
//! turns an [`AiTask`] into an [`HttpRequest`], turns a response body into clean text, and classifies error
//! responses. That makes every provider testable from recorded fixtures.
//!
//! Provider replies are untrusted input. They are size-capped before parsing, parsed with a depth limit,
//! reduced to the one text field we need, stripped of control and direction-override characters, and
//! length-capped. Error messages are redacted because some providers echo the offending key.

use crate::error::AiError;
use pg_core::canon::Canon;
use pg_host::{redact, HttpRequest, HttpResponse, Method, Secret};
use std::fmt;
use std::time::Duration;

#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Provider {
    OpenAi,
    DeepSeek,
    Anthropic,
    OpenRouter,
}

impl Provider {
    pub const ALL: [Provider; 4] = [
        Provider::OpenAi,
        Provider::DeepSeek,
        Provider::Anthropic,
        Provider::OpenRouter,
    ];

    /// The stable id used in settings files and secret names.
    pub const fn id(self) -> &'static str {
        match self {
            Provider::OpenAi => "openai",
            Provider::DeepSeek => "deepseek",
            Provider::Anthropic => "anthropic",
            Provider::OpenRouter => "openrouter",
        }
    }

    pub const fn display_name(self) -> &'static str {
        match self {
            Provider::OpenAi => "OpenAI",
            Provider::DeepSeek => "DeepSeek",
            Provider::Anthropic => "Anthropic",
            Provider::OpenRouter => "OpenRouter",
        }
    }

    pub fn from_id(id: &str) -> Option<Provider> {
        Provider::ALL.into_iter().find(|p| p.id() == id)
    }

    /// The only host requests for this provider may go to.
    pub const fn host(self) -> &'static str {
        match self {
            Provider::OpenAi => "api.openai.com",
            Provider::DeepSeek => "api.deepseek.com",
            Provider::Anthropic => "api.anthropic.com",
            Provider::OpenRouter => "openrouter.ai",
        }
    }

    /// A reasonable cheap, fast model to start with. These are data to review as providers change their
    /// line-ups; the player can always enter a custom model id.
    pub const fn recommended_model(self) -> &'static str {
        match self {
            Provider::OpenAi => "gpt-4o-mini",
            Provider::DeepSeek => "deepseek-chat",
            Provider::Anthropic => "claude-haiku-4-5-20251001",
            Provider::OpenRouter => "openrouter/auto",
        }
    }

    /// Where a key for this provider is kept in the credential store.
    pub fn secret_name(self) -> String {
        format!("playground.ai.{}", self.id())
    }
}

impl fmt::Display for Provider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.display_name())
    }
}

/// All the hosts the app may talk to.
pub fn allowed_hosts() -> Vec<&'static str> {
    Provider::ALL.iter().map(|p| p.host()).collect()
}

/// One text-generation request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AiTask {
    /// Trusted instructions written by the game.
    pub system: String,
    /// The untrusted payload: names, topics and anything a pack contributed. It is never placed in `system`.
    pub user: String,
    pub max_tokens: u32,
}

impl AiTask {
    /// The connection test: a tiny, cheap request.
    pub fn connection_test() -> AiTask {
        AiTask {
            system: "You are a connection test. Reply with exactly the word OK.".to_owned(),
            user: "ping".to_owned(),
            max_tokens: 16,
        }
    }
}

/// Largest provider reply body we will parse (1 MiB).
pub const MAX_BODY_BYTES: usize = 1024 * 1024;
/// Longest error message kept from a provider.
const MAX_ERROR_CHARS: usize = 200;

pub trait ProviderAdapter: Send + Sync {
    fn provider(&self) -> Provider;
    fn build_request(
        &self,
        task: &AiTask,
        model: &str,
        key: &Secret,
        timeout: Duration,
    ) -> HttpRequest;
    /// The reply text from a **successful** response body, cleaned and capped at `max_chars`.
    fn parse_response(&self, body: &str, max_chars: usize) -> Result<String, AiError>;
    /// Classifies a non-success response.
    fn classify_error(&self, resp: &HttpResponse) -> AiError {
        classify(resp)
    }
}

/// The adapter for a provider.
pub fn adapter_for(p: Provider) -> &'static dyn ProviderAdapter {
    match p {
        Provider::OpenAi => &OpenAiLike {
            provider: Provider::OpenAi,
        },
        Provider::DeepSeek => &OpenAiLike {
            provider: Provider::DeepSeek,
        },
        Provider::OpenRouter => &OpenAiLike {
            provider: Provider::OpenRouter,
        },
        Provider::Anthropic => &AnthropicAdapter,
    }
}

struct OpenAiLike {
    provider: Provider,
}

fn json_request(
    url: &str,
    headers: Vec<(String, String)>,
    body: &Canon,
    timeout: Duration,
) -> HttpRequest {
    HttpRequest {
        method: Method::Post,
        url: url.to_owned(),
        headers,
        body: Some(body.to_canonical_string()),
        timeout_ms: u64::try_from(timeout.as_millis()).unwrap_or(u64::MAX),
    }
}

impl ProviderAdapter for OpenAiLike {
    fn provider(&self) -> Provider {
        self.provider
    }

    fn build_request(
        &self,
        task: &AiTask,
        model: &str,
        key: &Secret,
        timeout: Duration,
    ) -> HttpRequest {
        let (url, tokens_field) = match self.provider {
            Provider::OpenAi => (
                "https://api.openai.com/v1/chat/completions",
                "max_completion_tokens",
            ),
            Provider::DeepSeek => ("https://api.deepseek.com/chat/completions", "max_tokens"),
            _ => (
                "https://openrouter.ai/api/v1/chat/completions",
                "max_tokens",
            ),
        };
        let body = Canon::map([
            ("model", Canon::str(model)),
            (
                "messages",
                Canon::List(vec![
                    Canon::map([
                        ("role", Canon::str("system")),
                        ("content", Canon::str(task.system.clone())),
                    ]),
                    Canon::map([
                        ("role", Canon::str("user")),
                        ("content", Canon::str(task.user.clone())),
                    ]),
                ]),
            ),
            (tokens_field, Canon::Int(i128::from(task.max_tokens))),
        ]);
        json_request(
            url,
            vec![
                (
                    "Authorization".to_owned(),
                    format!("Bearer {}", key.expose()),
                ),
                ("Content-Type".to_owned(), "application/json".to_owned()),
            ],
            &body,
            timeout,
        )
    }

    fn parse_response(&self, body: &str, max_chars: usize) -> Result<String, AiError> {
        let v = parse_json(body)?;
        let content = v
            .pointer("/choices/0/message/content")
            .ok_or(AiError::BadOutput)?;
        // Content is normally a string; some routers return a list of typed parts.
        let text = match content {
            serde_json::Value::String(s) => s.clone(),
            serde_json::Value::Array(parts) => parts
                .iter()
                .filter_map(|p| p.get("text").and_then(serde_json::Value::as_str))
                .collect::<Vec<_>>()
                .join(""),
            _ => return Err(AiError::BadOutput),
        };
        finish(&text, max_chars)
    }
}

struct AnthropicAdapter;

impl ProviderAdapter for AnthropicAdapter {
    fn provider(&self) -> Provider {
        Provider::Anthropic
    }

    fn build_request(
        &self,
        task: &AiTask,
        model: &str,
        key: &Secret,
        timeout: Duration,
    ) -> HttpRequest {
        let body = Canon::map([
            ("model", Canon::str(model)),
            ("max_tokens", Canon::Int(i128::from(task.max_tokens))),
            ("system", Canon::str(task.system.clone())),
            (
                "messages",
                Canon::List(vec![Canon::map([
                    ("role", Canon::str("user")),
                    ("content", Canon::str(task.user.clone())),
                ])]),
            ),
        ]);
        json_request(
            "https://api.anthropic.com/v1/messages",
            vec![
                ("x-api-key".to_owned(), key.expose().to_owned()),
                ("anthropic-version".to_owned(), "2023-06-01".to_owned()),
                ("Content-Type".to_owned(), "application/json".to_owned()),
            ],
            &body,
            timeout,
        )
    }

    fn parse_response(&self, body: &str, max_chars: usize) -> Result<String, AiError> {
        let v = parse_json(body)?;
        let parts = v
            .get("content")
            .and_then(serde_json::Value::as_array)
            .ok_or(AiError::BadOutput)?;
        let text: String = parts
            .iter()
            .filter(|p| p.get("type").and_then(serde_json::Value::as_str) == Some("text"))
            .filter_map(|p| p.get("text").and_then(serde_json::Value::as_str))
            .collect::<Vec<_>>()
            .join("");
        finish(&text, max_chars)
    }
}

fn parse_json(body: &str) -> Result<serde_json::Value, AiError> {
    if body.len() > MAX_BODY_BYTES {
        return Err(AiError::BadOutput);
    }
    // serde_json refuses nesting deeper than 128 levels, so hostile depth cannot overflow the stack.
    serde_json::from_str(body).map_err(|_| AiError::BadOutput)
}

fn finish(text: &str, max_chars: usize) -> Result<String, AiError> {
    let clean = clean_text(text, max_chars);
    if clean.is_empty() {
        Err(AiError::BadOutput)
    } else {
        Ok(clean)
    }
}

/// Characters that can disguise text on screen: bidirectional overrides and isolates, zero-width
/// characters and the byte-order mark.
fn is_spoofing(c: char) -> bool {
    matches!(c, '\u{200B}'..='\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2060}'..='\u{2064}' | '\u{2066}'..='\u{2069}' | '\u{FEFF}')
}

/// Makes untrusted text safe to display: drops control characters (keeping newlines and tabs) and
/// direction-spoofing characters, collapses runs of blank lines, trims, and truncates to `max_chars`
/// characters.
pub fn clean_text(text: &str, max_chars: usize) -> String {
    let mut out = String::new();
    let mut newlines = 0;
    for c in text.chars() {
        if is_spoofing(c) || (c.is_control() && c != '\n' && c != '\t') {
            continue;
        }
        if c == '\n' {
            newlines += 1;
            if newlines > 2 {
                continue;
            }
        } else {
            newlines = 0;
        }
        out.push(c);
    }
    let cut: String = out.trim().chars().take(max_chars).collect();
    cut.trim_end().to_owned()
}

/// The message inside a provider's error JSON, if any, redacted and capped.
fn error_message(body: &str) -> Option<String> {
    if body.len() > MAX_BODY_BYTES {
        return None;
    }
    let v: serde_json::Value = serde_json::from_str(body).ok()?;
    let msg = v
        .pointer("/error/message")
        .or_else(|| v.get("message"))
        .or_else(|| v.get("error").filter(|e| e.is_string()))
        .and_then(serde_json::Value::as_str)?;
    Some(clean_text(&redact(msg, &[]), MAX_ERROR_CHARS))
}

fn error_code(body: &str) -> String {
    serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .map(|v| {
            [
                v.pointer("/error/code"),
                v.pointer("/error/type"),
                v.get("type"),
            ]
            .into_iter()
            .flatten()
            .filter_map(serde_json::Value::as_str)
            .collect::<Vec<_>>()
            .join(" ")
            .to_ascii_lowercase()
        })
        .unwrap_or_default()
}

fn classify(resp: &HttpResponse) -> AiError {
    let code = error_code(&resp.body);
    let retry_after = resp
        .header("retry-after")
        .and_then(|v| v.trim().parse::<u64>().ok())
        .map(|s| Duration::from_secs(s.min(3600)));
    let quota_words = [
        "insufficient_quota",
        "billing",
        "credit",
        "exceeded_current_quota",
    ];
    let message = || error_message(&resp.body).unwrap_or_else(|| format!("HTTP {}", resp.status));
    match resp.status {
        401 | 403 => AiError::Auth,
        402 => AiError::Quota,
        429 if quota_words.iter().any(|w| code.contains(w)) => AiError::Quota,
        429 => AiError::RateLimit { retry_after },
        408 | 504 => AiError::Timeout,
        _ if code.contains("authentication") || code.contains("invalid_api_key") => AiError::Auth,
        _ if code.contains("rate_limit") => AiError::RateLimit { retry_after },
        _ => AiError::Provider(message()),
    }
}

#[cfg(test)]
mod tests;
