use super::*;
use crate::settings::KeyManager;
use pg_host::{
    AllowListNet, FixedClock, MemLog, MemSecretStore, NetError, ScriptedNet, SecretError,
};

const KEY: &str = "sk-SENTINEL-0123456789abcdef";

const OK_BODY: &str = r#"{"choices":[{"message":{"role":"assistant","content":"Nice day."}}],"usage":{"total_tokens":9}}"#;

struct Rig {
    net: Arc<AllowListNet<ScriptedNet>>,
    secrets: Arc<MemSecretStore>,
    clock: Arc<FixedClock>,
    log: Arc<MemLog>,
    client: AiClient,
    settings: AiSettings,
}

fn rig_with(cfg: ClientConfig) -> Rig {
    let net = Arc::new(AllowListNet::new(
        ScriptedNet::new(),
        &crate::provider::allowed_hosts(),
    ));
    let secrets = Arc::new(MemSecretStore::new());
    KeyManager::new(secrets.as_ref())
        .set_key(Provider::OpenAi, KEY)
        .unwrap();
    let clock = Arc::new(FixedClock::new());
    let log = Arc::new(MemLog::new());
    let client = AiClient::new(
        net.clone(),
        secrets.clone(),
        clock.clone(),
        log.clone(),
        cfg,
    );
    Rig {
        net,
        secrets,
        clock,
        log,
        client,
        settings: AiSettings {
            enabled: true,
            ..AiSettings::default()
        },
    }
}

fn rig() -> Rig {
    rig_with(ClientConfig::default())
}

fn task(n: u32) -> AiTask {
    AiTask {
        system: "sys".into(),
        user: format!("hello {n}"),
        max_tokens: 40,
    }
}

fn go(r: &Rig, t: &AiTask) -> Result<AiReply, AiError> {
    r.client.generate(&r.settings, t, &CancelToken::new())
}

fn sent(r: &Rig) -> usize {
    r.net.inner().request_count()
}

#[test]
fn a_successful_call_returns_clean_text_and_the_key_goes_only_to_the_provider() {
    let r = rig();
    r.net.inner().push_ok(200, OK_BODY);
    let reply = go(&r, &task(1)).unwrap();
    assert_eq!(
        reply,
        AiReply {
            text: "Nice day.".into(),
            from_cache: false
        }
    );
    let reqs = r.net.inner().requests();
    assert_eq!(reqs.len(), 1);
    assert!(reqs[0].url.starts_with("https://api.openai.com/"));
    assert!(
        reqs[0].headers.iter().any(|(_, v)| v.contains(KEY)),
        "the key is sent to the provider"
    );
    assert!(!reqs[0].body.as_deref().unwrap().contains("SENTINEL"));
    assert!(!r.log.text().contains("SENTINEL"), "{}", r.log.text());
    assert!(r.log.text().contains("ai ok provider=openai"));
}

#[test]
fn identical_requests_are_served_from_the_cache() {
    let r = rig();
    r.net.inner().push_ok(200, OK_BODY);
    assert!(!go(&r, &task(1)).unwrap().from_cache);
    assert!(go(&r, &task(1)).unwrap().from_cache);
    assert_eq!(sent(&r), 1);
    // A different prompt or model is a different entry.
    r.net.inner().push_ok(200, OK_BODY);
    assert!(!go(&r, &task(2)).unwrap().from_cache);
    let other = AiSettings {
        custom_model: Some("other-model".into()),
        ..r.settings.clone()
    };
    r.net.inner().push_ok(200, OK_BODY);
    assert!(
        !r.client
            .generate(&other, &task(1), &CancelToken::new())
            .unwrap()
            .from_cache
    );
}

#[test]
fn the_cache_is_bounded() {
    let r = rig_with(ClientConfig {
        cache_capacity: 2,
        ..ClientConfig::default()
    });
    for n in 0..3 {
        r.net.inner().push_ok(200, OK_BODY);
        go(&r, &task(n)).unwrap();
    }
    // Entry 0 was evicted; entries 1 and 2 remain.
    assert!(go(&r, &task(2)).unwrap().from_cache);
    assert!(go(&r, &task(1)).unwrap().from_cache);
    r.net.inner().push_ok(200, OK_BODY);
    assert!(!go(&r, &task(0)).unwrap().from_cache);
}

#[test]
fn disabled_missing_key_and_unreadable_store_fail_without_touching_the_network() {
    let r = rig();
    let off = AiSettings {
        enabled: false,
        ..r.settings.clone()
    };
    assert_eq!(
        r.client.generate(&off, &task(1), &CancelToken::new()),
        Err(AiError::Disabled)
    );
    assert_eq!(r.client.status(&off), AiStatus::Disabled);

    let other = AiSettings {
        provider: Provider::Anthropic,
        ..r.settings.clone()
    };
    assert_eq!(
        r.client.generate(&other, &task(1), &CancelToken::new()),
        Err(AiError::NoKey)
    );
    assert_eq!(r.client.status(&other), AiStatus::NoKey);

    r.secrets
        .fail_with(Some(SecretError::Denied("keychain locked".into())));
    assert!(matches!(go(&r, &task(1)), Err(AiError::KeyStore(_))));
    assert_eq!(sent(&r), 0);
}

#[test]
fn connection_tests_work_before_ai_is_switched_on_and_skip_the_cache() {
    let r = rig();
    let off = AiSettings {
        enabled: false,
        ..r.settings.clone()
    };
    r.net
        .inner()
        .push_ok(200, r#"{"choices":[{"message":{"content":"OK"}}]}"#);
    r.client.test_connection(&off, &CancelToken::new()).unwrap();
    r.net.inner().push_ok(
        401,
        r#"{"error":{"code":"invalid_api_key","message":"nope"}}"#,
    );
    assert_eq!(
        r.client.test_connection(&off, &CancelToken::new()),
        Err(AiError::Auth)
    );
    assert_eq!(sent(&r), 2);
}

#[test]
fn configuration_errors_are_not_retried_and_never_open_the_breaker() {
    let r = rig();
    for n in 0..10 {
        r.net.inner().push_ok(
            401,
            r#"{"error":{"code":"invalid_api_key","message":"bad key"}}"#,
        );
        assert_eq!(go(&r, &task(n)), Err(AiError::Auth));
    }
    assert_eq!(sent(&r), 10, "one request per call, no retries");
    assert_eq!(r.client.status(&r.settings), AiStatus::Ready);
    r.net.inner().push_ok(402, "{}");
    assert_eq!(go(&r, &task(99)), Err(AiError::Quota));
}

#[test]
fn transient_failures_are_retried_once() {
    let r = rig();
    r.net.inner().push(Err(NetError::Timeout));
    r.net.inner().push_ok(200, OK_BODY);
    assert!(go(&r, &task(1)).is_ok());
    assert_eq!(sent(&r), 2);
    r.net.inner().push(Err(NetError::Timeout));
    r.net.inner().push(Err(NetError::Timeout));
    assert_eq!(go(&r, &task(2)), Err(AiError::Timeout));
    assert_eq!(sent(&r), 4);
    r.net.inner().push_ok(500, "oops");
    r.net.inner().push_ok(200, OK_BODY);
    assert!(go(&r, &task(3)).is_ok(), "a 5xx is retried too");
    // Offline is not retried: it will not fix itself in a moment.
    r.net.inner().push(Err(NetError::Offline));
    assert_eq!(go(&r, &task(4)), Err(AiError::Offline));
    assert_eq!(sent(&r), 7);
}

#[test]
fn repeated_provider_failures_open_the_breaker_then_a_probe_closes_it() {
    let r = rig();
    for n in 0..3 {
        r.net.inner().push(Err(NetError::Offline));
        assert_eq!(go(&r, &task(n)), Err(AiError::Offline));
    }
    assert_eq!(sent(&r), 3);
    // Open: calls fail instantly without a request.
    match go(&r, &task(10)) {
        Err(AiError::CircuitOpen { retry_in }) => assert!(retry_in <= Duration::from_secs(30)),
        other => panic!("{other:?}"),
    }
    assert_eq!(sent(&r), 3);
    assert!(matches!(
        r.client.status(&r.settings),
        AiStatus::Paused { .. }
    ));
    // After the delay a single probe goes out; success closes the breaker.
    r.clock.advance(Duration::from_secs(31));
    r.net.inner().push_ok(200, OK_BODY);
    assert!(go(&r, &task(11)).is_ok());
    assert_eq!(r.client.status(&r.settings), AiStatus::Ready);
    r.net.inner().push_ok(200, OK_BODY);
    assert!(go(&r, &task(12)).is_ok());
}

#[test]
fn a_failed_probe_reopens_the_breaker() {
    let r = rig();
    for n in 0..3 {
        r.net.inner().push(Err(NetError::Offline));
        let _ = go(&r, &task(n));
    }
    r.clock.advance(Duration::from_secs(31));
    r.net.inner().push(Err(NetError::Offline));
    assert_eq!(go(&r, &task(10)), Err(AiError::Offline));
    assert!(matches!(
        go(&r, &task(11)),
        Err(AiError::CircuitOpen { .. })
    ));
}

#[test]
fn unusable_replies_count_as_failures() {
    let r = rig();
    for n in 0..3 {
        r.net.inner().push_ok(200, r#"{"choices":[]}"#);
        assert_eq!(go(&r, &task(n)), Err(AiError::BadOutput));
    }
    assert!(matches!(go(&r, &task(9)), Err(AiError::CircuitOpen { .. })));
}

#[test]
fn the_request_rate_is_capped_per_minute() {
    let r = rig_with(ClientConfig {
        requests_per_minute: 5,
        ..ClientConfig::default()
    });
    for n in 0..5 {
        r.net.inner().push_ok(200, OK_BODY);
        go(&r, &task(n)).unwrap();
        r.clock.advance(Duration::from_secs(1));
    }
    match go(&r, &task(50)) {
        Err(AiError::RateLimit {
            retry_after: Some(d),
        }) => assert!(d > Duration::ZERO && d <= Duration::from_secs(60), "{d:?}"),
        other => panic!("{other:?}"),
    }
    assert_eq!(sent(&r), 5);
    r.clock.advance(Duration::from_secs(61));
    r.net.inner().push_ok(200, OK_BODY);
    assert!(go(&r, &task(51)).is_ok());
}

#[test]
fn a_cancelled_request_is_neutral() {
    let r = rig();
    let c = CancelToken::new();
    c.cancel();
    assert_eq!(
        r.client.generate(&r.settings, &task(1), &c),
        Err(AiError::Cancelled)
    );
    assert_eq!(r.client.status(&r.settings), AiStatus::Ready);
    for n in 0..5 {
        let c = CancelToken::new();
        c.cancel();
        let _ = r.client.generate(&r.settings, &task(n + 10), &c);
    }
    assert_eq!(
        r.client.status(&r.settings),
        AiStatus::Ready,
        "cancels never open the breaker"
    );
}

#[test]
fn nothing_a_provider_or_the_network_says_can_leak_the_key_through_errors_or_logs() {
    let r = rig();
    let echo = format!(
        r#"{{"error":{{"message":"Incorrect API key provided: {KEY}","code":"invalid_api_key"}}}}"#
    );
    r.net.inner().push_ok(401, &echo);
    r.net.inner().push_ok(400, &echo);
    r.net.inner().push(Err(NetError::Other(format!(
        "proxy rejected credentials {KEY}"
    ))));
    r.net.inner().push(Err(NetError::Tls(format!(
        "handshake failed for Bearer {KEY}"
    ))));
    r.net.inner().push_ok(500, &echo);
    r.net.inner().push_ok(500, &echo);
    let mut texts = Vec::new();
    for n in 0..4 {
        let e = go(&r, &task(n)).unwrap_err();
        texts.push(format!("{e:?} | {e} | {}", e.user_message()));
    }
    for t in &texts {
        assert!(!t.contains("SENTINEL"), "{t}");
    }
    assert!(!r.log.text().contains("SENTINEL"), "{}", r.log.text());
}

#[test]
fn requests_to_unlisted_hosts_never_leave_the_machine() {
    // A net that refuses everything not on the allow-list, fed an adapter URL we corrupt on purpose, shows the
    // decorator is what stands between a bug and a leaked key.
    let r = rig();
    let req = pg_host::HttpRequest {
        method: pg_host::Method::Post,
        url: "https://evil.example/v1/chat".into(),
        headers: vec![("Authorization".into(), format!("Bearer {KEY}"))],
        body: None,
        timeout_ms: 1000,
    };
    assert!(matches!(
        r.net.request(&req, &CancelToken::new()),
        Err(NetError::Blocked(_))
    ));
    assert_eq!(sent(&r), 0);
}

#[test]
fn the_client_is_usable_from_several_threads() {
    let r = rig();
    for _ in 0..8 {
        r.net.inner().push_ok(200, OK_BODY);
    }
    let r = Arc::new(r);
    let handles: Vec<_> = (0..8)
        .map(|n| {
            let r = Arc::clone(&r);
            std::thread::spawn(move || go(&r, &task(n)).is_ok())
        })
        .collect();
    let ok = handles
        .into_iter()
        .map(|h| h.join().unwrap())
        .filter(|b| *b)
        .count();
    assert_eq!(ok, 8);
}
