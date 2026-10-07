//! The redaction self-check (Roadmap Stage 0 "no key appears in logs, exports or crash reports").
//!
//! [`run`] drives the AI client through success, authentication failure, provider errors that **echo the
//! key**, network errors that echo the key, retries and a tripped circuit breaker, all with a recognisable
//! sentinel key, and then scans everything that could carry text out of the program: every returned error
//! and reply, every log line, the saved settings file, the `Debug` output of the request and settings
//! types, and the whole in-memory storage. The only place the sentinel may appear is the one header of the
//! one request that is sent to the provider. `pg ai selfcheck` runs it; `pg check` and CI call that.

use crate::client::{AiClient, ClientConfig};
use crate::provider::{allowed_hosts, AiTask, Provider};
use crate::settings::{AiSettings, DeviceSettings, KeyManager};
use pg_host::{
    AllowListNet, CancelToken, FixedClock, HttpResponse, MemLog, MemSecretStore, MemStorage,
    NetError, ScriptedNet, Storage,
};
use std::sync::Arc;

/// A recognisable key. Its shape (`sk-...`) is also one the scrubber knows, so the check also proves the
/// shape-based layer; `SENTINEL` is what the scan greps for.
pub const SENTINEL_KEY: &str = "sk-SENTINEL-9f8e7d6c5b4a39281706";
pub const NEEDLE: &str = "SENTINEL";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Check {
    pub name: String,
    pub ok: bool,
    pub detail: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SelfCheckReport {
    pub checks: Vec<Check>,
}

impl SelfCheckReport {
    pub fn ok(&self) -> bool {
        self.checks.iter().all(|c| c.ok)
    }

    fn add(&mut self, name: &str, ok: bool, detail: impl Into<String>) {
        self.checks.push(Check {
            name: name.to_owned(),
            ok,
            detail: detail.into(),
        });
    }
}

fn echo_body() -> String {
    format!(
        r#"{{"error":{{"message":"Incorrect API key provided: {SENTINEL_KEY}. Authorization: Bearer {SENTINEL_KEY}","code":"invalid_api_key"}}}}"#
    )
}

/// Runs the whole check and reports each scan separately.
pub fn run() -> SelfCheckReport {
    let mut report = SelfCheckReport::default();
    let storage = MemStorage::new();
    let secrets = Arc::new(MemSecretStore::new());
    let log = Arc::new(MemLog::new());
    let net = Arc::new(AllowListNet::new(ScriptedNet::new(), &allowed_hosts()));
    let clock = Arc::new(FixedClock::new());
    let client = AiClient::new(
        net.clone(),
        secrets.clone(),
        clock.clone(),
        log.clone(),
        ClientConfig::default(),
    );

    // Setup: the key goes into the credential store; settings go to storage.
    let keys = KeyManager::new(secrets.as_ref());
    let mut results: Vec<String> = Vec::new();
    for p in Provider::ALL {
        if let Err(e) = keys.set_key(p, SENTINEL_KEY) {
            results.push(e);
        }
    }
    let settings = AiSettings {
        enabled: true,
        ..AiSettings::default()
    };
    let device = DeviceSettings {
        ai: settings.clone(),
    };
    if let Err(e) = device.save(&storage) {
        results.push(e.to_string());
    }

    // Scenario traffic: every call below can produce text.
    let ok_body = r#"{"choices":[{"message":{"content":"Fine."}}]}"#;
    let task = |n: u32| AiTask {
        system: "sys".into(),
        user: format!("u{n}"),
        max_tokens: 20,
    };
    net.inner().push_ok(200, ok_body);
    net.inner().push_ok(401, &echo_body());
    net.inner().push_ok(400, &echo_body());
    net.inner().push(Err(NetError::Other(format!(
        "proxy rejected {SENTINEL_KEY}"
    ))));
    net.inner().push(Err(NetError::Tls(format!(
        "handshake failed, Authorization: Bearer {SENTINEL_KEY}"
    ))));
    net.inner().push_ok(500, &echo_body());
    net.inner().push_ok(500, &echo_body());
    net.inner().push_ok(429, &echo_body());
    for provider in Provider::ALL {
        let s = AiSettings {
            provider,
            ..settings.clone()
        };
        for n in 0..8 {
            match client.generate(&s, &task(n), &CancelToken::new()) {
                Ok(r) => results.push(r.text),
                Err(e) => results.push(format!("{e:?} | {e} | {}", e.user_message())),
            }
        }
        results.push(format!("{:?}", client.status(&s)));
    }
    // The breaker has opened by now; its message must be clean too.
    clock.advance(std::time::Duration::from_secs(1));
    if let Err(e) = client.generate(&settings, &task(99), &CancelToken::new()) {
        results.push(e.user_message());
    }

    // 1. Everything the client returned.
    let leaked: Vec<&String> = results.iter().filter(|t| t.contains(NEEDLE)).collect();
    report.add(
        "returned errors, replies and statuses",
        leaked.is_empty(),
        format!("{} item(s) scanned", results.len()),
    );

    // 2. Log lines.
    let log_text = log.text();
    report.add(
        "log lines",
        !log_text.contains(NEEDLE),
        format!("{} line(s) scanned", log.lines().len()),
    );

    // 3. Debug output of the types that could be printed by accident.
    let requests = net.inner().requests();
    let debug: String = requests
        .iter()
        .map(|r| format!("{r:?}"))
        .chain(std::iter::once(format!("{settings:?} {device:?}")))
        .chain(std::iter::once(format!(
            "{:?}",
            pg_host::Secret::new(SENTINEL_KEY)
        )))
        .collect::<Vec<_>>()
        .join("\n");
    report.add(
        "Debug output of requests, settings and secrets",
        !debug.contains(NEEDLE),
        format!("{} request(s) printed", requests.len()),
    );

    // 4. Storage: nothing the program wrote contains the key.
    let mut stored = String::new();
    if let Ok(blobs) = storage.list("") {
        for b in blobs {
            if let Ok(Some(bytes)) = storage.read(&b.name) {
                stored.push_str(&b.name);
                stored.push_str(&String::from_utf8_lossy(&bytes));
            }
        }
    }
    report.add(
        "saved settings and storage",
        !stored.contains(NEEDLE),
        format!("{} file(s) scanned", storage.names().len()),
    );

    // 5. The key reached the network only where it should: the right header of requests to allow-listed hosts.
    let mut misplaced = Vec::new();
    for r in &requests {
        let in_body = r.body.as_deref().is_some_and(|b| b.contains(NEEDLE));
        let in_url = r.url.contains(NEEDLE);
        let headers = r.headers.iter().filter(|(_, v)| v.contains(NEEDLE)).count();
        if in_body || in_url || headers > 1 {
            misplaced.push(r.url.clone());
        }
    }
    report.add(
        "key appears only in one header per request",
        misplaced.is_empty(),
        format!("{} request(s) checked", requests.len()),
    );

    // 6. Requests to hosts outside the allow-list never reach the network.
    let before = net.inner().request_count();
    let evil = pg_host::HttpRequest {
        method: pg_host::Method::Post,
        url: "https://evil.example/v1".into(),
        headers: vec![("Authorization".into(), format!("Bearer {SENTINEL_KEY}"))],
        body: None,
        timeout_ms: 1000,
    };
    let blocked = matches!(
        pg_host::Net::request(net.as_ref(), &evil, &CancelToken::new()),
        Err(NetError::Blocked(_))
    );
    report.add(
        "requests to other hosts are blocked",
        blocked && net.inner().request_count() == before,
        "evil.example refused",
    );

    // 7. A bare redaction of a response that echoes the key.
    let echoed = HttpResponse {
        status: 401,
        headers: vec![],
        body: echo_body(),
    };
    let cleaned = pg_host::redact(&echoed.body, &[]);
    report.add(
        "redact() removes echoed keys by shape",
        !cleaned.contains(NEEDLE),
        "provider echo scrubbed",
    );
    report
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_self_check_passes() {
        let r = run();
        for c in &r.checks {
            assert!(c.ok, "{}: {}", c.name, c.detail);
        }
        assert!(r.checks.len() >= 7);
        assert!(r.ok());
    }

    /// Mutation check: the scan really detects a leak. If the scrubber were bypassed the same scan fails.
    #[test]
    fn the_scan_would_notice_a_leak() {
        let log = MemLog::new();
        pg_host::LogSink::log(&log, pg_host::Level::Info, &format!("oops {SENTINEL_KEY}"));
        assert!(
            log.text().contains(NEEDLE),
            "the raw sink keeps what it is given, so redaction must happen before it"
        );
        // Through the redacting wrapper it is gone.
        let safe = pg_host::RedactingLog::new(MemLog::new());
        pg_host::LogSink::log(&safe, pg_host::Level::Info, &format!("oops {SENTINEL_KEY}"));
        assert!(!safe.inner().text().contains(NEEDLE));
    }
}
