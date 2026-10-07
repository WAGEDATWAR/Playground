//! OS-backed implementations of the 0.7 host services: credential store, HTTPS and clock.

use pg_host::{
    iso_utc, CancelToken, Clock, HttpRequest, HttpResponse, Method, Net, NetError, Secret,
    SecretError, SecretStore,
};
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant};

const SERVICE: &str = "Playground";

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// Provider keys in the operating system's credential store (Windows Credential Manager, macOS Keychain,
/// the Linux Secret Service). Where the OS store cannot be used (no Secret Service on a minimal Linux
/// install, a locked keychain), keys are kept in memory for the session only and
/// [`SecretStore::is_persistent`] says so; nothing is ever written to disk in plaintext.
#[derive(Default)]
pub struct KeyringSecretStore {
    session: Mutex<BTreeMap<String, String>>,
    fell_back: AtomicBool,
}

impl KeyringSecretStore {
    pub fn new() -> KeyringSecretStore {
        KeyringSecretStore::default()
    }

    fn entry(name: &str) -> Result<keyring::Entry, keyring::Error> {
        keyring::Entry::new(SERVICE, name)
    }

    fn fall_back(&self) {
        self.fell_back.store(true, Ordering::SeqCst);
    }
}

impl SecretStore for KeyringSecretStore {
    fn get(&self, name: &str) -> Result<Option<Secret>, SecretError> {
        if let Some(v) = lock(&self.session).get(name) {
            return Ok(Some(Secret::new(v.clone())));
        }
        match Self::entry(name).and_then(|e| e.get_password()) {
            Ok(v) => Ok(Some(Secret::new(v))),
            Err(keyring::Error::NoEntry) => Ok(None),
            // No usable OS store: behave as session-only, so "nothing stored" rather than an error.
            Err(
                keyring::Error::NoDefaultStore
                | keyring::Error::PlatformFailure(_)
                | keyring::Error::NoStorageAccess(_),
            ) => {
                self.fall_back();
                Ok(None)
            }
            Err(e) => Err(SecretError::Other(e.to_string())),
        }
    }

    fn set(&self, name: &str, value: &Secret) -> Result<(), SecretError> {
        match Self::entry(name).and_then(|e| e.set_password(value.expose())) {
            Ok(()) => {
                lock(&self.session).remove(name);
                Ok(())
            }
            Err(
                keyring::Error::NoDefaultStore
                | keyring::Error::PlatformFailure(_)
                | keyring::Error::NoStorageAccess(_),
            ) => {
                self.fall_back();
                lock(&self.session).insert(name.to_owned(), value.expose().to_owned());
                Ok(())
            }
            Err(e) => Err(SecretError::Other(e.to_string())),
        }
    }

    fn delete(&self, name: &str) -> Result<(), SecretError> {
        lock(&self.session).remove(name);
        match Self::entry(name).and_then(|e| e.delete_credential()) {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(
                keyring::Error::NoDefaultStore
                | keyring::Error::PlatformFailure(_)
                | keyring::Error::NoStorageAccess(_),
            ) => {
                self.fall_back();
                Ok(())
            }
            Err(e) => Err(SecretError::Other(e.to_string())),
        }
    }

    fn is_persistent(&self) -> bool {
        !self.fell_back.load(Ordering::SeqCst) && keyring::Entry::store_status().is_ok()
    }
}

/// Blocking HTTPS through `ureq` (rustls). **Redirects are never followed**, so a key cannot be forwarded to
/// a host that is not on the allow-list; wrap this in `AllowListNet` as well. A request cannot be
/// interrupted once sent; cancellation is checked before it goes out and the timeout bounds the rest.
pub struct UreqNet;

const MAX_RESPONSE_BYTES: u64 = 1024 * 1024;

fn map_error(e: &ureq::Error) -> NetError {
    use ureq::Error as E;
    match e {
        E::Timeout(_) => NetError::Timeout,
        E::HostNotFound | E::ConnectionFailed => NetError::Offline,
        E::Io(io)
            if matches!(
                io.kind(),
                std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
            ) =>
        {
            NetError::Timeout
        }
        E::Io(io)
            if matches!(
                io.kind(),
                std::io::ErrorKind::ConnectionRefused
                    | std::io::ErrorKind::ConnectionReset
                    | std::io::ErrorKind::NotConnected
            ) =>
        {
            NetError::Offline
        }
        other => {
            let text = other.to_string();
            if text.to_ascii_lowercase().contains("tls")
                || text.to_ascii_lowercase().contains("certificate")
            {
                NetError::Tls(text)
            } else {
                NetError::Other(text)
            }
        }
    }
}

impl Net for UreqNet {
    fn request(&self, req: &HttpRequest, cancel: &CancelToken) -> Result<HttpResponse, NetError> {
        if cancel.is_cancelled() {
            return Err(NetError::Cancelled);
        }
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_millis(req.timeout_ms.max(1))))
            .max_redirects(0)
            .http_status_as_error(false)
            .https_only(true)
            .build()
            .into();
        let result = match req.method {
            Method::Get => {
                let mut r = agent.get(&req.url);
                for (k, v) in &req.headers {
                    r = r.header(k, v);
                }
                r.call()
            }
            Method::Post => {
                let mut r = agent.post(&req.url);
                for (k, v) in &req.headers {
                    r = r.header(k, v);
                }
                r.send(req.body.as_deref().unwrap_or(""))
            }
        };
        let mut resp = result.map_err(|e| map_error(&e))?;
        let status = resp.status().as_u16();
        let headers = ["retry-after"]
            .iter()
            .filter_map(|n| {
                resp.headers()
                    .get(*n)
                    .and_then(|v| v.to_str().ok())
                    .map(|v| ((*n).to_owned(), v.to_owned()))
            })
            .collect();
        let body = resp
            .body_mut()
            .with_config()
            .limit(MAX_RESPONSE_BYTES)
            .read_to_string()
            .map_err(|e| map_error(&e))?;
        Ok(HttpResponse {
            status,
            headers,
            body,
        })
    }
}

/// The real clock: a monotonic reading since construction, and the wall-clock time as UTC text.
pub struct SystemClock {
    start: Instant,
}

impl SystemClock {
    pub fn new() -> SystemClock {
        SystemClock {
            start: Instant::now(),
        }
    }
}

impl Default for SystemClock {
    fn default() -> Self {
        SystemClock::new()
    }
}

impl Clock for SystemClock {
    fn now_monotonic(&self) -> Duration {
        self.start.elapsed()
    }

    fn wall_clock_iso(&self) -> String {
        let secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        iso_utc(secs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_system_clock_moves_forward_and_formats_utc() {
        let c = SystemClock::new();
        let a = c.now_monotonic();
        std::thread::sleep(Duration::from_millis(5));
        assert!(c.now_monotonic() > a);
        let iso = c.wall_clock_iso();
        assert!(
            iso.len() == 20 && iso.ends_with('Z') && iso.starts_with("20"),
            "{iso}"
        );
    }

    #[test]
    fn a_cancelled_request_never_goes_out() {
        let t = CancelToken::new();
        t.cancel();
        let req = HttpRequest {
            method: Method::Get,
            url: "https://127.0.0.1:1/".into(),
            headers: vec![],
            body: None,
            timeout_ms: 100,
        };
        assert_eq!(UreqNet.request(&req, &t), Err(NetError::Cancelled));
    }

    #[test]
    fn plain_http_and_unreachable_hosts_fail_cleanly() {
        let mk = |url: &str| HttpRequest {
            method: Method::Get,
            url: url.into(),
            headers: vec![],
            body: None,
            timeout_ms: 500,
        };
        let c = CancelToken::new();
        // https_only: plain http is refused before anything is sent.
        assert!(UreqNet.request(&mk("http://127.0.0.1:9/"), &c).is_err());
        // Nothing listens on port 1: an error, never a panic or a hang.
        assert!(UreqNet.request(&mk("https://127.0.0.1:1/"), &c).is_err());
    }

    /// Needs a real OS credential store; run by hand with `cargo test -p pg-host-os -- --ignored`.
    #[test]
    #[ignore = "touches the real OS credential store"]
    fn the_os_credential_store_round_trips() {
        let s = KeyringSecretStore::new();
        let name = format!("playground.test.{}", std::process::id());
        s.set(&name, &Secret::new("sentinel-value-123")).unwrap();
        assert_eq!(
            s.get(&name).unwrap().unwrap().expose(),
            "sentinel-value-123"
        );
        s.delete(&name).unwrap();
        assert_eq!(s.get(&name).unwrap(), None);
        println!("persistent: {}", s.is_persistent());
    }
}
