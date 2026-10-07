//! Device-code sign-in for providers that issue keys through a website (Player2).
//!
//! The player never pastes a key. The game asks the provider for a short code, shows it with a link, and the
//! player approves it in their browser; the game polls until the provider hands back a key, which goes
//! straight into the credential store. The flow here is pure protocol plus a polling schedule:
//!
//! * [`start_request`] / [`parse_start`] build and read the first call (`POST /login/device/new`);
//! * [`poll_request`] / [`parse_poll`] build and read each poll (`POST /login/device/token`);
//! * [`DeviceLoginSession`] owns the schedule (interval, slow-down, expiry, transient failures) over a
//!   monotonic clock the caller supplies, and **never sleeps**: the UI or CLI thread decides when to call
//!   [`DeviceLoginSession::poll`] again, so the flow is testable without waiting.
//!
//! Everything the provider sends is untrusted. The verification link is checked to be `https` on the
//! provider's own domain before it may be opened, and the user code is cleaned and length-capped, so a
//! hostile or compromised response cannot send the player to another site.

use crate::provider::Provider;
use crate::settings::validate_key;
use pg_host::{redact, CancelToken, HttpRequest, HttpResponse, Method, Net, NetError, Secret};
use std::fmt;
use std::time::Duration;

/// The OAuth grant type for the device flow.
pub const GRANT_TYPE: &str = "urn:ietf:params:oauth:grant-type:device_code";
/// The client id registered with Player2 for this game (a public identifier, not a secret). Tools can
/// override it with `--client-id` or the `PG_PLAYER2_CLIENT_ID` environment variable.
pub const CLIENT_ID: &str = "01a114aa-5569-7b98-96cc-949d6d32191d";

const BASE: &str = "https://api.player2.game/v1";
const MAX_USER_CODE: usize = 32;
const MAX_URI: usize = 300;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LoginError {
    Net(NetError),
    /// The provider refused the client id or the request.
    Rejected(String),
    /// The provider's reply was not what the protocol says.
    BadReply(String),
    /// The player did not approve in time.
    Expired,
    /// The player declined.
    Denied,
    Cancelled,
    /// Too many consecutive failures while polling.
    GaveUp,
}

impl LoginError {
    pub fn user_message(&self) -> String {
        match self {
            LoginError::Net(NetError::Offline) => {
                "Sign-in unavailable: no network connection".to_owned()
            }
            LoginError::Net(NetError::Timeout) => {
                "Sign-in unavailable: the provider did not answer in time".to_owned()
            }
            LoginError::Net(e) => format!("Sign-in unavailable: {e}"),
            LoginError::Rejected(m) => format!("Sign-in was refused: {m}"),
            LoginError::BadReply(m) => format!("Sign-in failed: unexpected reply ({m})"),
            LoginError::Expired => "Sign-in expired before it was approved; start again".to_owned(),
            LoginError::Denied => "Sign-in was declined".to_owned(),
            LoginError::Cancelled => "Sign-in cancelled".to_owned(),
            LoginError::GaveUp => {
                "Sign-in stopped after repeated failures; try again later".to_owned()
            }
        }
    }
}

impl fmt::Display for LoginError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.user_message())
    }
}

impl std::error::Error for LoginError {}

/// What the player is shown while the flow waits for approval.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Prompt {
    /// The short code to confirm in the browser.
    pub user_code: String,
    /// Where to go. Checked to be https on the provider's domain.
    pub verification_url: String,
    /// The same link with the code filled in, when the provider offers it (same checks).
    pub complete_url: Option<String>,
    pub expires_in: Duration,
}

fn json(text: &str) -> Result<serde_json::Value, LoginError> {
    if text.len() > 64 * 1024 {
        return Err(LoginError::BadReply("reply too large".to_owned()));
    }
    serde_json::from_str(text).map_err(|_| LoginError::BadReply("not JSON".to_owned()))
}

fn post(path: &str, body: String) -> HttpRequest {
    HttpRequest {
        method: Method::Post,
        url: format!("{BASE}{path}"),
        headers: vec![
            ("Content-Type".to_owned(), "application/json".to_owned()),
            ("Accept".to_owned(), "application/json".to_owned()),
        ],
        body: Some(body),
        timeout_ms: 8_000,
    }
}

fn quote(s: &str) -> String {
    serde_json::Value::String(s.to_owned()).to_string()
}

/// `POST /login/device/new`.
pub fn start_request(client_id: &str) -> HttpRequest {
    post(
        "/login/device/new",
        format!(r#"{{"client_id":{}}}"#, quote(client_id)),
    )
}

/// `POST /login/device/token`.
pub fn poll_request(client_id: &str, device_code: &Secret) -> HttpRequest {
    post(
        "/login/device/token",
        format!(
            r#"{{"client_id":{},"device_code":{},"grant_type":{}}}"#,
            quote(client_id),
            quote(device_code.expose()),
            quote(GRANT_TYPE)
        ),
    )
}

/// Whether `url` may be opened for the player: https, no credentials, and the provider's own domain.
pub fn is_safe_verification_url(url: &str) -> bool {
    if url.len() > MAX_URI || url.chars().any(|c| c.is_control() || c == ' ' || c == '\\') {
        return false;
    }
    let Some(rest) = url.strip_prefix("https://") else {
        return false;
    };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    if authority.contains(['@', ':']) {
        return false;
    }
    let host = authority.to_ascii_lowercase();
    host == "player2.game" || host.ends_with(".player2.game")
}

fn clean_code(code: &str) -> Option<String> {
    let c = code.trim();
    let ok = !c.is_empty()
        && c.len() <= MAX_USER_CODE
        && c.chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == ' ');
    ok.then(|| c.to_owned())
}

/// The first reply: the device code (kept secret), what to show, and the polling schedule.
pub struct Started {
    pub device_code: Secret,
    pub prompt: Prompt,
    pub interval: Duration,
}

pub fn parse_start(resp: &HttpResponse) -> Result<Started, LoginError> {
    if !(200..300).contains(&resp.status) {
        return Err(rejection(resp));
    }
    let v = json(&resp.body)?;
    let text = |k: &str| v.get(k).and_then(serde_json::Value::as_str);
    let device_code = text("deviceCode")
        .filter(|c| !c.is_empty() && c.len() <= 512 && c.chars().all(|ch| ch.is_ascii_graphic()))
        .ok_or_else(|| LoginError::BadReply("no device code".to_owned()))?;
    let user_code = text("userCode")
        .and_then(clean_code)
        .ok_or_else(|| LoginError::BadReply("no usable user code".to_owned()))?;
    let verification_url = text("verificationUri")
        .filter(|u| is_safe_verification_url(u))
        .ok_or_else(|| {
            LoginError::BadReply("the verification link is not on the provider's site".to_owned())
        })?
        .to_owned();
    let complete_url = text("verificationUriComplete")
        .filter(|u| is_safe_verification_url(u))
        .map(str::to_owned);
    let secs = |k: &str, default: u64, lo: u64, hi: u64| {
        v.get(k)
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(default)
            .clamp(lo, hi)
    };
    Ok(Started {
        device_code: Secret::new(device_code),
        prompt: Prompt {
            user_code,
            verification_url,
            complete_url,
            expires_in: Duration::from_secs(secs("expiresIn", 600, 30, 1800)),
        },
        interval: Duration::from_secs(secs("interval", 5, 1, 30)),
    })
}

fn rejection(resp: &HttpResponse) -> LoginError {
    let msg = serde_json::from_str::<serde_json::Value>(&resp.body)
        .ok()
        .and_then(|v| {
            v.get("message")
                .or_else(|| v.get("error_description"))
                .or_else(|| v.get("error"))
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned)
        })
        .unwrap_or_else(|| format!("HTTP {}", resp.status));
    LoginError::Rejected(crate::provider::clean_text(&redact(&msg, &[]), 160))
}

/// What one poll said.
pub enum PollOutcome {
    /// Not approved yet; poll again after the interval.
    Pending,
    /// Poll less often: the interval grows.
    SlowDown,
    Done(Secret),
    Expired,
    Denied,
    /// A server-side hiccup; try again next time (the session counts these).
    Transient,
    Failed(LoginError),
}

pub fn parse_poll(resp: &HttpResponse) -> PollOutcome {
    if (200..300).contains(&resp.status) {
        let key = serde_json::from_str::<serde_json::Value>(&resp.body)
            .ok()
            .and_then(|v| {
                v.get("p2Key")
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_owned)
            });
        return match key.map(|k| validate_key(&k)) {
            Some(Ok(secret)) => PollOutcome::Done(secret),
            _ => PollOutcome::Failed(LoginError::BadReply(
                "no usable key in the reply".to_owned(),
            )),
        };
    }
    if resp.status >= 500 {
        return PollOutcome::Transient;
    }
    if resp.status == 429 {
        return PollOutcome::SlowDown;
    }
    let code = serde_json::from_str::<serde_json::Value>(&resp.body)
        .ok()
        .and_then(|v| {
            v.get("error")
                .and_then(serde_json::Value::as_str)
                .map(str::to_ascii_lowercase)
        })
        .unwrap_or_default();
    match code.as_str() {
        "authorization_pending" => PollOutcome::Pending,
        "slow_down" => PollOutcome::SlowDown,
        "expired_token" | "expired" => PollOutcome::Expired,
        "access_denied" | "denied" => PollOutcome::Denied,
        _ => PollOutcome::Failed(rejection(resp)),
    }
}

/// Where a session stands after a poll.
#[derive(Debug)]
pub enum SessionState {
    /// Waiting for the player; poll again after `next_in`.
    Waiting {
        next_in: Duration,
    },
    /// Approved: store this key.
    Done(Secret),
    Failed(LoginError),
}

/// Consecutive transient failures tolerated.
const MAX_TRANSIENT: u32 = 5;

/// One sign-in attempt: the schedule over a caller-supplied monotonic clock.
pub struct DeviceLoginSession {
    client_id: String,
    device_code: Secret,
    prompt: Prompt,
    interval: Duration,
    deadline: Duration,
    next_poll: Duration,
    transient: u32,
}

impl DeviceLoginSession {
    /// Starts the flow: one request, then the player is shown [`DeviceLoginSession::prompt`].
    pub fn begin(
        net: &dyn Net,
        client_id: &str,
        now: Duration,
        cancel: &CancelToken,
    ) -> Result<DeviceLoginSession, LoginError> {
        let resp = net
            .request(&start_request(client_id), cancel)
            .map_err(|e| match e {
                NetError::Cancelled => LoginError::Cancelled,
                other => LoginError::Net(other),
            })?;
        let s = parse_start(&resp)?;
        Ok(DeviceLoginSession {
            client_id: client_id.to_owned(),
            device_code: s.device_code,
            deadline: now + s.prompt.expires_in,
            next_poll: now + s.interval,
            interval: s.interval,
            prompt: s.prompt,
            transient: 0,
        })
    }

    pub fn prompt(&self) -> &Prompt {
        &self.prompt
    }

    /// The monotonic time at which the next poll is due.
    pub fn next_poll_at(&self) -> Duration {
        self.next_poll
    }

    /// Polls if it is time (otherwise reports how long to wait). The caller sleeps; this never does.
    pub fn poll(&mut self, net: &dyn Net, now: Duration, cancel: &CancelToken) -> SessionState {
        if cancel.is_cancelled() {
            return SessionState::Failed(LoginError::Cancelled);
        }
        if now >= self.deadline {
            return SessionState::Failed(LoginError::Expired);
        }
        if now < self.next_poll {
            return SessionState::Waiting {
                next_in: self.next_poll - now,
            };
        }
        let outcome = match net.request(&poll_request(&self.client_id, &self.device_code), cancel) {
            Ok(resp) => parse_poll(&resp),
            Err(NetError::Cancelled) => return SessionState::Failed(LoginError::Cancelled),
            // A network blip is like a server hiccup: keep trying within the budget.
            Err(NetError::Timeout | NetError::Offline) => PollOutcome::Transient,
            Err(e) => return SessionState::Failed(LoginError::Net(e)),
        };
        match outcome {
            PollOutcome::Done(key) => SessionState::Done(key),
            PollOutcome::Expired => SessionState::Failed(LoginError::Expired),
            PollOutcome::Denied => SessionState::Failed(LoginError::Denied),
            PollOutcome::Failed(e) => SessionState::Failed(e),
            PollOutcome::Pending => self.wait(0),
            PollOutcome::SlowDown => {
                self.interval =
                    (self.interval + Duration::from_secs(5)).min(Duration::from_secs(60));
                self.wait(0)
            }
            PollOutcome::Transient => {
                self.transient += 1;
                if self.transient > MAX_TRANSIENT {
                    return SessionState::Failed(LoginError::GaveUp);
                }
                self.wait(self.transient)
            }
        }
        .at(now, self)
    }

    fn wait(&self, _transient: u32) -> SessionState {
        SessionState::Waiting {
            next_in: self.interval,
        }
    }
}

/// Small helper so `poll` can schedule the next poll after computing the wait.
trait Schedule {
    fn at(self, now: Duration, s: &mut DeviceLoginSession) -> SessionState;
}

impl Schedule for SessionState {
    fn at(self, now: Duration, s: &mut DeviceLoginSession) -> SessionState {
        if let SessionState::Waiting { next_in } = &self {
            s.next_poll = now + *next_in;
        }
        self
    }
}

/// The key a finished sign-in produced is stored like any other: under the provider's secret name.
pub fn store_key(
    store: &dyn pg_host::SecretStore,
    provider: Provider,
    key: &Secret,
) -> Result<(), String> {
    store
        .set(&provider.secret_name(), key)
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests;
