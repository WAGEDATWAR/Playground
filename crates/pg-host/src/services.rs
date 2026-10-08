//! The remaining host-service traits and their in-memory doubles (Blueprint §3): `SecretStore`, `Net`
//! (with an allow-list decorator), `Clock`, `Dialogs` and `Audio`.
//!
//! Every double records what it was asked, so tests can assert both behaviour and what crossed the
//! boundary (for example, that a key was only ever sent to an allow-listed host).

use crate::redact::Secret;
use std::collections::{BTreeMap, VecDeque};
use std::fmt;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

// ---- secrets ---------------------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SecretError {
    /// No credential service is available on this machine.
    Unavailable(String),
    /// The service refused (locked keychain, user cancelled).
    Denied(String),
    Other(String),
}

impl fmt::Display for SecretError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SecretError::Unavailable(e) => write!(f, "no credential store is available: {e}"),
            SecretError::Denied(e) => write!(f, "the credential store refused access: {e}"),
            SecretError::Other(e) => write!(f, "credential store error: {e}"),
        }
    }
}

impl std::error::Error for SecretError {}

/// Provider keys only; backed by the OS credential store, or by memory for the session where none exists.
pub trait SecretStore: Send + Sync {
    fn get(&self, name: &str) -> Result<Option<Secret>, SecretError>;
    fn set(&self, name: &str, value: &Secret) -> Result<(), SecretError>;
    fn delete(&self, name: &str) -> Result<(), SecretError>;
    /// Whether keys survive a restart. `false` means session-only (shown to the player).
    fn is_persistent(&self) -> bool;
}

/// An in-memory [`SecretStore`] for tests and for machines with no credential service.
#[derive(Default)]
pub struct MemSecretStore {
    items: Mutex<BTreeMap<String, String>>,
    session_only: bool,
    fail: Mutex<Option<SecretError>>,
}

impl MemSecretStore {
    /// Behaves like an OS store (persistent).
    pub fn new() -> MemSecretStore {
        MemSecretStore::default()
    }

    /// Behaves like the session-only fallback.
    pub fn session_only() -> MemSecretStore {
        MemSecretStore {
            session_only: true,
            ..MemSecretStore::default()
        }
    }

    /// Makes every call fail with `e` until cleared with `None`.
    pub fn fail_with(&self, e: Option<SecretError>) {
        *lock(&self.fail) = e;
    }

    /// Names stored (never values), for tests.
    pub fn names(&self) -> Vec<String> {
        lock(&self.items).keys().cloned().collect()
    }

    fn check(&self) -> Result<(), SecretError> {
        lock(&self.fail).clone().map_or(Ok(()), Err)
    }
}

impl SecretStore for MemSecretStore {
    fn get(&self, name: &str) -> Result<Option<Secret>, SecretError> {
        self.check()?;
        Ok(lock(&self.items).get(name).map(|v| Secret::new(v.clone())))
    }

    fn set(&self, name: &str, value: &Secret) -> Result<(), SecretError> {
        self.check()?;
        lock(&self.items).insert(name.to_owned(), value.expose().to_owned());
        Ok(())
    }

    fn delete(&self, name: &str) -> Result<(), SecretError> {
        self.check()?;
        lock(&self.items).remove(name);
        Ok(())
    }

    fn is_persistent(&self) -> bool {
        !self.session_only
    }
}

// ---- network ---------------------------------------------------------------------------------------

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Method {
    Get,
    Post,
}

/// An HTTPS request. Its `Debug` output hides credential headers and query values, so even a stray `{:?}`
/// is safe.
#[derive(Clone, PartialEq, Eq)]
pub struct HttpRequest {
    pub method: Method,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Option<String>,
    pub timeout_ms: u64,
}

const SENSITIVE_HEADERS: [&str; 5] = [
    "authorization",
    "x-api-key",
    "api-key",
    "proxy-authorization",
    "x-goog-api-key",
];

impl fmt::Debug for HttpRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let headers: Vec<(String, String)> = self
            .headers
            .iter()
            .map(|(k, v)| {
                let hide = SENSITIVE_HEADERS.contains(&k.to_ascii_lowercase().as_str());
                (k.clone(), if hide { "***".to_owned() } else { v.clone() })
            })
            .collect();
        let url = self.url.split('?').next().unwrap_or("");
        f.debug_struct("HttpRequest")
            .field("method", &self.method)
            .field(
                "url",
                &if url.len() == self.url.len() {
                    url.to_owned()
                } else {
                    format!("{url}?***")
                },
            )
            .field("headers", &headers)
            .field("body_bytes", &self.body.as_ref().map(String::len))
            .field("timeout_ms", &self.timeout_ms)
            .finish()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HttpResponse {
    pub status: u16,
    /// Response headers with lower-cased names (only those the client may need, such as `retry-after`).
    pub headers: Vec<(String, String)>,
    pub body: String,
}

impl HttpResponse {
    /// The first header named `name` (case-insensitive).
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NetError {
    Timeout,
    Offline,
    Cancelled,
    Tls(String),
    /// The request was refused before it left the machine (not https, host not allow-listed, ...).
    Blocked(String),
    Other(String),
}

impl fmt::Display for NetError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            NetError::Timeout => write!(f, "the request timed out"),
            NetError::Offline => write!(f, "no network connection"),
            NetError::Cancelled => write!(f, "the request was cancelled"),
            NetError::Tls(e) => write!(f, "secure connection failed: {e}"),
            NetError::Blocked(e) => write!(f, "request blocked: {e}"),
            NetError::Other(e) => write!(f, "network error: {e}"),
        }
    }
}

impl std::error::Error for NetError {}

/// Lets the caller abandon a request in flight.
#[derive(Clone, Debug, Default)]
pub struct CancelToken(Arc<AtomicBool>);

impl CancelToken {
    pub fn new() -> CancelToken {
        CancelToken::default()
    }

    pub fn cancel(&self) {
        self.0.store(true, Ordering::SeqCst);
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
}

/// Blocking HTTPS; always called from a worker thread. Implementations never follow redirects (a redirect
/// could carry a key to a host that is not on the allow-list).
pub trait Net: Send + Sync {
    fn request(&self, req: &HttpRequest, cancel: &CancelToken) -> Result<HttpResponse, NetError>;
}

type Handler = Box<dyn Fn(&HttpRequest) -> Result<HttpResponse, NetError> + Send + Sync>;

/// A scripted [`Net`]: replies come from a queue (or a handler) and every request is recorded.
#[derive(Default)]
pub struct ScriptedNet {
    replies: Mutex<VecDeque<Result<HttpResponse, NetError>>>,
    handler: Mutex<Option<Handler>>,
    seen: Mutex<Vec<HttpRequest>>,
}

impl ScriptedNet {
    pub fn new() -> ScriptedNet {
        ScriptedNet::default()
    }

    /// Queues one reply (used in order).
    pub fn push(&self, reply: Result<HttpResponse, NetError>) {
        lock(&self.replies).push_back(reply);
    }

    pub fn push_ok(&self, status: u16, body: &str) {
        self.push(Ok(HttpResponse {
            status,
            headers: Vec::new(),
            body: body.to_owned(),
        }));
    }

    /// Replaces the queue with a function of the request.
    pub fn set_handler(&self, h: Handler) {
        *lock(&self.handler) = Some(h);
    }

    /// Every request received so far.
    pub fn requests(&self) -> Vec<HttpRequest> {
        lock(&self.seen).clone()
    }

    pub fn request_count(&self) -> usize {
        lock(&self.seen).len()
    }
}

impl Net for ScriptedNet {
    fn request(&self, req: &HttpRequest, cancel: &CancelToken) -> Result<HttpResponse, NetError> {
        lock(&self.seen).push(req.clone());
        if cancel.is_cancelled() {
            return Err(NetError::Cancelled);
        }
        if let Some(h) = lock(&self.handler).as_ref() {
            return h(req);
        }
        lock(&self.replies)
            .pop_front()
            .unwrap_or(Err(NetError::Offline))
    }
}

/// The host part of an `https://` URL, lower-cased, or `None` if the URL is not a plain https URL (a
/// different scheme, userinfo, a non-default port, or no host).
pub fn https_host(url: &str) -> Option<String> {
    let rest = url.strip_prefix("https://")?;
    let authority = rest.split(['/', '?', '#']).next()?;
    if authority.is_empty() || authority.contains(['@', ':', '\\', ' ']) {
        return None;
    }
    if !authority
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-')
    {
        return None;
    }
    Some(authority.to_ascii_lowercase())
}

/// Wraps a [`Net`] and refuses anything that is not `https` to an allow-listed host (Blueprint §10.2).
pub struct AllowListNet<N: Net> {
    inner: N,
    hosts: Vec<String>,
}

impl<N: Net> AllowListNet<N> {
    pub fn new(inner: N, hosts: &[&str]) -> AllowListNet<N> {
        AllowListNet {
            inner,
            hosts: hosts.iter().map(|h| h.to_ascii_lowercase()).collect(),
        }
    }

    pub fn inner(&self) -> &N {
        &self.inner
    }
}

impl<N: Net> Net for AllowListNet<N> {
    fn request(&self, req: &HttpRequest, cancel: &CancelToken) -> Result<HttpResponse, NetError> {
        let host = https_host(&req.url)
            .ok_or_else(|| NetError::Blocked("only plain https URLs are allowed".to_owned()))?;
        if !self.hosts.contains(&host) {
            return Err(NetError::Blocked(format!(
                "host '{host}' is not on the allow-list"
            )));
        }
        self.inner.request(req, cancel)
    }
}

// ---- logging ---------------------------------------------------------------------------------------

#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    Debug,
    Info,
    Warn,
    Error,
}

/// Where log lines go. Every path that can carry text from outside (provider errors, file paths, user
/// input) must go through [`RedactingLog`].
pub trait LogSink: Send + Sync {
    fn log(&self, level: Level, line: &str);
}

/// Wraps a sink so that nothing reaches it without being redacted first.
pub struct RedactingLog<L: LogSink> {
    inner: L,
}

impl<L: LogSink> RedactingLog<L> {
    pub fn new(inner: L) -> RedactingLog<L> {
        RedactingLog { inner }
    }

    pub fn inner(&self) -> &L {
        &self.inner
    }
}

impl<L: LogSink> LogSink for RedactingLog<L> {
    fn log(&self, level: Level, line: &str) {
        self.inner.log(level, &crate::redact::redact(line, &[]));
    }
}

/// Collects log lines in memory.
#[derive(Default)]
pub struct MemLog {
    lines: Mutex<Vec<(Level, String)>>,
}

impl MemLog {
    pub fn new() -> MemLog {
        MemLog::default()
    }

    pub fn lines(&self) -> Vec<(Level, String)> {
        lock(&self.lines).clone()
    }

    pub fn text(&self) -> String {
        lock(&self.lines)
            .iter()
            .map(|(l, s)| format!("{l:?}: {s}"))
            .collect::<Vec<_>>()
            .join(
                "
",
            )
    }
}

impl LogSink for MemLog {
    fn log(&self, level: Level, line: &str) {
        lock(&self.lines).push((level, line.to_owned()));
    }
}

/// Writes to standard error.
pub struct StderrLog;

impl LogSink for StderrLog {
    fn log(&self, level: Level, line: &str) {
        eprintln!("[{level:?}] {line}");
    }
}

// ---- clock -----------------------------------------------------------------------------------------

/// Formats seconds since the Unix epoch as `YYYY-MM-DDTHH:MM:SSZ` (UTC), without a calendar crate.
pub fn iso_utc(secs: u64) -> String {
    let days = i64::try_from(secs / 86_400).unwrap_or(0);
    let rem = secs % 86_400;
    // Days since 1970-01-01 to a civil date (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    )
}

/// Monotonic time for timers; wall-clock text for save metadata only (never inside the simulation).
pub trait Clock: Send + Sync {
    fn now_monotonic(&self) -> Duration;
    fn wall_clock_iso(&self) -> String;
}

/// A clock tests move by hand.
#[derive(Default)]
pub struct FixedClock {
    now: Mutex<Duration>,
}

impl FixedClock {
    pub fn new() -> FixedClock {
        FixedClock::default()
    }

    pub fn advance(&self, by: Duration) {
        let mut g = lock(&self.now);
        *g += by;
    }
}

impl Clock for FixedClock {
    fn now_monotonic(&self) -> Duration {
        *lock(&self.now)
    }

    fn wall_clock_iso(&self) -> String {
        "2026-01-01T00:00:00Z".to_owned()
    }
}

// ---- dialogs ---------------------------------------------------------------------------------------

/// Native file pickers.
pub trait Dialogs: Send + Sync {
    fn pick_file_to_read(&self, extensions: &[&str]) -> Option<PathBuf>;
    fn pick_file_to_write(&self, suggested: &str) -> Option<PathBuf>;
    /// Chooses a folder (for installing a content pack). The default is a cancelled dialog.
    fn pick_folder(&self) -> Option<PathBuf> {
        None
    }
}

/// Returns scripted answers (and `None` once they run out, like a cancelled dialog).
#[derive(Default)]
pub struct ScriptedDialogs {
    reads: Mutex<VecDeque<Option<PathBuf>>>,
    folders: Mutex<VecDeque<Option<PathBuf>>>,
    writes: Mutex<VecDeque<Option<PathBuf>>>,
    asked: Mutex<Vec<String>>,
}

impl ScriptedDialogs {
    pub fn new() -> ScriptedDialogs {
        ScriptedDialogs::default()
    }

    pub fn answer_read(&self, p: Option<PathBuf>) {
        lock(&self.reads).push_back(p);
    }

    pub fn answer_folder(&self, p: Option<PathBuf>) {
        lock(&self.folders).push_back(p);
    }

    pub fn answer_write(&self, p: Option<PathBuf>) {
        lock(&self.writes).push_back(p);
    }

    pub fn asked(&self) -> Vec<String> {
        lock(&self.asked).clone()
    }
}

impl Dialogs for ScriptedDialogs {
    fn pick_file_to_read(&self, extensions: &[&str]) -> Option<PathBuf> {
        lock(&self.asked).push(format!("read {}", extensions.join(",")));
        lock(&self.reads).pop_front().flatten()
    }

    fn pick_file_to_write(&self, suggested: &str) -> Option<PathBuf> {
        lock(&self.asked).push(format!("write {suggested}"));
        lock(&self.writes).pop_front().flatten()
    }

    fn pick_folder(&self) -> Option<PathBuf> {
        lock(&self.asked).push("folder".to_owned());
        lock(&self.folders).pop_front().flatten()
    }
}

// ---- audio -----------------------------------------------------------------------------------------

#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Bus {
    Master,
    Music,
    Sfx,
    Ui,
    Ambient,
}

pub trait Audio: Send + Sync {
    fn play(&self, id: &str, bus: Bus);
    fn set_volume(&self, bus: Bus, volume: f32);
    fn stop(&self, bus: Bus);
}

/// Plays nothing and remembers what it was asked.
#[derive(Default)]
pub struct NullAudio {
    log: Mutex<Vec<String>>,
}

impl NullAudio {
    pub fn new() -> NullAudio {
        NullAudio::default()
    }

    pub fn log(&self) -> Vec<String> {
        lock(&self.log).clone()
    }
}

impl Audio for NullAudio {
    fn play(&self, id: &str, bus: Bus) {
        lock(&self.log).push(format!("play {id} on {bus:?}"));
    }

    fn set_volume(&self, bus: Bus, volume: f32) {
        lock(&self.log).push(format!("volume {bus:?} {volume:.2}"));
    }

    fn stop(&self, bus: Bus) {
        lock(&self.log).push(format!("stop {bus:?}"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req(url: &str) -> HttpRequest {
        HttpRequest {
            method: Method::Post,
            url: url.to_owned(),
            headers: vec![
                (
                    "Authorization".into(),
                    "Bearer sk-SENTINEL-123456789".into(),
                ),
                ("Content-Type".into(), "application/json".into()),
            ],
            body: Some("{}".into()),
            timeout_ms: 8_000,
        }
    }

    #[test]
    fn secret_stores_round_trip_and_report_persistence() {
        let s = MemSecretStore::new();
        assert!(s.is_persistent());
        assert_eq!(s.get("k").unwrap(), None);
        s.set("k", &Secret::new("v1")).unwrap();
        assert_eq!(s.get("k").unwrap().unwrap().expose(), "v1");
        assert_eq!(s.names(), ["k"]);
        s.delete("k").unwrap();
        s.delete("k").unwrap();
        assert_eq!(s.get("k").unwrap(), None);
        assert!(!MemSecretStore::session_only().is_persistent());
        s.fail_with(Some(SecretError::Denied("locked".into())));
        assert!(matches!(s.get("k"), Err(SecretError::Denied(_))));
        assert!(s.set("k", &Secret::new("x")).is_err());
        s.fail_with(None);
        assert!(s.get("k").is_ok());
    }

    #[test]
    fn a_request_never_shows_its_credentials_in_debug_output() {
        let r = req("https://api.example.com/v1/x?key=SUPERSECRETKEY&alt=json");
        let text = format!("{r:?}");
        assert!(
            !text.contains("SENTINEL") && !text.contains("SUPERSECRETKEY"),
            "{text}"
        );
        assert!(text.contains("application/json") && text.contains("api.example.com"));
    }

    #[test]
    fn scripted_net_replies_in_order_records_requests_and_honours_cancel() {
        let n = ScriptedNet::new();
        n.push_ok(200, "one");
        n.push(Err(NetError::Timeout));
        let c = CancelToken::new();
        assert_eq!(
            n.request(&req("https://a.example/x"), &c).unwrap().body,
            "one"
        );
        assert_eq!(
            n.request(&req("https://a.example/y"), &c),
            Err(NetError::Timeout)
        );
        assert_eq!(
            n.request(&req("https://a.example/z"), &c),
            Err(NetError::Offline),
            "empty queue is offline"
        );
        assert_eq!(n.request_count(), 3);
        c.cancel();
        assert_eq!(
            n.request(&req("https://a.example/w"), &c),
            Err(NetError::Cancelled)
        );
        n.set_handler(Box::new(|r| {
            Ok(HttpResponse {
                status: 200,
                headers: Vec::new(),
                body: r.url.clone(),
            })
        }));
        assert_eq!(
            n.request(&req("https://a.example/h"), &CancelToken::new())
                .unwrap()
                .body,
            "https://a.example/h"
        );
    }

    #[test]
    fn the_allow_list_blocks_everything_but_https_to_listed_hosts() {
        let n = AllowListNet::new(ScriptedNet::new(), &["api.openai.com", "API.anthropic.com"]);
        n.inner().push_ok(200, "ok");
        let c = CancelToken::new();
        assert!(n
            .request(&req("https://api.openai.com/v1/chat"), &c)
            .is_ok());
        for bad in [
            "http://api.openai.com/v1",
            "https://evil.example/v1",
            "https://api.openai.com.evil.example/v1",
            "https://user:pw@api.openai.com/v1",
            "https://api.openai.com:8443/v1",
            "https://api.openai.com@evil.example/v1",
            "ftp://api.openai.com/",
            "https:///nohost",
            "//api.openai.com/v1",
            "https://api.openai.com\\@evil.example/",
            "",
        ] {
            assert!(
                matches!(n.request(&req(bad), &c), Err(NetError::Blocked(_))),
                "{bad}"
            );
        }
        assert_eq!(
            n.inner().request_count(),
            1,
            "blocked requests never reach the network"
        );
        n.inner().push_ok(200, "ok");
        assert!(
            n.request(&req("https://api.anthropic.com/v1/messages"), &c)
                .is_ok(),
            "host match ignores case"
        );
    }

    #[test]
    fn https_host_extracts_only_clean_hosts() {
        assert_eq!(
            https_host("https://Api.Example.com/a?b#c").as_deref(),
            Some("api.example.com")
        );
        assert_eq!(https_host("https://x.y"), Some("x.y".to_owned()));
        for bad in [
            "http://x",
            "https://",
            "https://a b/",
            "https://a:1/",
            "https://a@b/",
            "https://é.com/",
        ] {
            assert_eq!(https_host(bad), None, "{bad}");
        }
    }

    #[test]
    fn iso_formatting_matches_known_dates() {
        assert_eq!(iso_utc(0), "1970-01-01T00:00:00Z");
        assert_eq!(iso_utc(951_782_400), "2000-02-29T00:00:00Z", "leap day");
        assert_eq!(iso_utc(1_709_210_096), "2024-02-29T12:34:56Z");
        assert_eq!(iso_utc(4_102_444_799), "2099-12-31T23:59:59Z");
        assert_eq!(iso_utc(1_000_000_000), "2001-09-09T01:46:40Z");
    }

    #[test]
    fn clock_dialogs_and_audio_doubles_behave() {
        let c = FixedClock::new();
        assert_eq!(c.now_monotonic(), Duration::ZERO);
        c.advance(Duration::from_secs(5));
        c.advance(Duration::from_millis(500));
        assert_eq!(c.now_monotonic(), Duration::from_millis(5_500));
        assert!(c.wall_clock_iso().ends_with('Z'));

        let d = ScriptedDialogs::new();
        d.answer_read(Some(PathBuf::from("a.json")));
        assert_eq!(
            d.pick_file_to_read(&["json"]),
            Some(PathBuf::from("a.json"))
        );
        assert_eq!(
            d.pick_file_to_read(&["json"]),
            None,
            "out of answers is a cancelled dialog"
        );
        assert_eq!(d.pick_file_to_write("x.json"), None);
        assert_eq!(d.asked(), ["read json", "read json", "write x.json"]);

        let a = NullAudio::new();
        a.play("click", Bus::Ui);
        a.set_volume(Bus::Music, 0.5);
        a.stop(Bus::Ui);
        assert_eq!(
            a.log(),
            ["play click on Ui", "volume Music 0.50", "stop Ui"]
        );
    }
}
