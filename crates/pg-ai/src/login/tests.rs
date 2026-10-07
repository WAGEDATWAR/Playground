use super::*;
use pg_host::{AllowListNet, FixedClock, MemSecretStore, ScriptedNet, SecretStore};
use proptest::prelude::*;

const START_OK: &str = r#"{"deviceCode":"dev-abc123","userCode":"AB12-CD34","verificationUri":"https://player2.game/device","verificationUriComplete":"https://player2.game/device?user_code=AB12-CD34","expiresIn":600,"interval":5}"#;
const PENDING: &str = r#"{"error":"authorization_pending"}"#;
const KEY_OK: &str = r#"{"p2Key":"p2-key-0123456789abcdef"}"#;

fn net() -> AllowListNet<ScriptedNet> {
    AllowListNet::new(ScriptedNet::new(), &crate::provider::allowed_hosts())
}

fn secs(n: u64) -> Duration {
    Duration::from_secs(n)
}

fn started(n: &AllowListNet<ScriptedNet>) -> DeviceLoginSession {
    n.inner().push_ok(200, START_OK);
    DeviceLoginSession::begin(n, "my-client", secs(100), &CancelToken::new()).unwrap()
}

fn resp(status: u16, body: &str) -> HttpResponse {
    HttpResponse {
        status,
        headers: vec![],
        body: body.to_owned(),
    }
}

// ---- requests ----------------------------------------------------------------------------------------

#[test]
fn the_two_requests_have_the_documented_shape_and_go_to_the_provider_host() {
    let r = start_request("my-client");
    assert_eq!(r.method, Method::Post);
    assert_eq!(r.url, "https://api.player2.game/v1/login/device/new");
    assert_eq!(r.body.as_deref(), Some(r#"{"client_id":"my-client"}"#));
    let p = poll_request("my-client", &Secret::new("dev-abc123"));
    assert_eq!(p.url, "https://api.player2.game/v1/login/device/token");
    let b: serde_json::Value = serde_json::from_str(p.body.as_deref().unwrap()).unwrap();
    assert_eq!(b["client_id"], "my-client");
    assert_eq!(b["device_code"], "dev-abc123");
    assert_eq!(b["grant_type"], GRANT_TYPE);
    for req in [&r, &p] {
        assert_eq!(
            pg_host::https_host(&req.url).as_deref(),
            Some(Provider::Player2.host())
        );
    }
    // A client id with quotes cannot break out of the JSON.
    let evil = start_request("a\",\"x\":\"y");
    assert!(serde_json::from_str::<serde_json::Value>(evil.body.as_deref().unwrap()).is_ok());
}

// ---- the happy path ----------------------------------------------------------------------------------

#[test]
fn a_full_sign_in_polls_on_schedule_and_returns_the_key() {
    let n = net();
    let mut s = started(&n);
    assert_eq!(s.prompt().user_code, "AB12-CD34");
    assert_eq!(s.prompt().verification_url, "https://player2.game/device");
    assert!(s
        .prompt()
        .complete_url
        .as_deref()
        .unwrap()
        .contains("AB12-CD34"));
    assert_eq!(s.prompt().expires_in, secs(600));
    let cancel = CancelToken::new();

    // Too early: no request is made.
    assert!(
        matches!(s.poll(&n, secs(102), &cancel), SessionState::Waiting { next_in } if next_in == secs(3))
    );
    assert_eq!(n.inner().request_count(), 1);
    // On time but not approved yet.
    n.inner().push_ok(400, PENDING);
    assert!(
        matches!(s.poll(&n, secs(105), &cancel), SessionState::Waiting { next_in } if next_in == secs(5))
    );
    // The provider asks us to slow down: the interval grows.
    n.inner().push_ok(400, r#"{"error":"slow_down"}"#);
    assert!(
        matches!(s.poll(&n, secs(110), &cancel), SessionState::Waiting { next_in } if next_in == secs(10))
    );
    assert_eq!(s.next_poll_at(), secs(120));
    // Approved.
    n.inner().push_ok(200, KEY_OK);
    match s.poll(&n, secs(120), &cancel) {
        SessionState::Done(k) => assert_eq!(k.expose(), "p2-key-0123456789abcdef"),
        other => panic!("{other:?}"),
    }
    assert_eq!(n.inner().request_count(), 4);
}

#[test]
fn the_key_is_stored_under_the_providers_secret_name() {
    let store = MemSecretStore::new();
    store_key(
        &store,
        Provider::Player2,
        &Secret::new("p2-key-0123456789abcdef"),
    )
    .unwrap();
    assert_eq!(store.names(), ["playground.ai.player2"]);
    assert_eq!(
        store
            .get("playground.ai.player2")
            .unwrap()
            .unwrap()
            .expose(),
        "p2-key-0123456789abcdef"
    );
}

// ---- ways it ends ------------------------------------------------------------------------------------

#[test]
fn expiry_denial_cancel_and_provider_errors_end_the_flow_with_a_reason() {
    let cancel = CancelToken::new();
    // Expires by the clock, without a request.
    let n = net();
    let mut s = started(&n);
    assert!(matches!(
        s.poll(&n, secs(700), &cancel),
        SessionState::Failed(LoginError::Expired)
    ));
    assert_eq!(n.inner().request_count(), 1);
    // Expired and denied as reported by the provider.
    for (body, want) in [
        (r#"{"error":"expired_token"}"#, LoginError::Expired),
        (r#"{"error":"access_denied"}"#, LoginError::Denied),
    ] {
        let n = net();
        let mut s = started(&n);
        n.inner().push_ok(400, body);
        match s.poll(&n, secs(105), &cancel) {
            SessionState::Failed(e) => assert_eq!(e, want),
            other => panic!("{other:?}"),
        }
    }
    // An unknown error ends the flow rather than looping until expiry.
    let n = net();
    let mut s = started(&n);
    n.inner().push_ok(
        400,
        r#"{"error":"invalid_client","error_description":"unknown client id"}"#,
    );
    match s.poll(&n, secs(105), &cancel) {
        SessionState::Failed(LoginError::Rejected(m)) => {
            assert!(m.contains("unknown client id"), "{m}")
        }
        other => panic!("{other:?}"),
    }
    // Cancelling stops it.
    let n = net();
    let mut s = started(&n);
    let c = CancelToken::new();
    c.cancel();
    assert!(matches!(
        s.poll(&n, secs(105), &c),
        SessionState::Failed(LoginError::Cancelled)
    ));
}

#[test]
fn transient_failures_are_tolerated_a_few_times_then_give_up() {
    let n = net();
    let mut s = started(&n);
    let cancel = CancelToken::new();
    let mut now = 105;
    for _ in 0..5 {
        n.inner().push_ok(503, "busy");
        assert!(
            matches!(s.poll(&n, secs(now), &cancel), SessionState::Waiting { .. }),
            "at {now}"
        );
        now += 5;
    }
    n.inner().push(Err(NetError::Offline));
    assert!(matches!(
        s.poll(&n, secs(now), &cancel),
        SessionState::Failed(LoginError::GaveUp)
    ));
    // A good poll in between resets the count.
    let n = net();
    let mut s = started(&n);
    n.inner().push_ok(503, "busy");
    n.inner().push_ok(400, PENDING);
    n.inner().push_ok(200, KEY_OK);
    assert!(matches!(
        s.poll(&n, secs(105), &cancel),
        SessionState::Waiting { .. }
    ));
    assert!(matches!(
        s.poll(&n, secs(110), &cancel),
        SessionState::Waiting { .. }
    ));
    assert!(matches!(
        s.poll(&n, secs(115), &cancel),
        SessionState::Done(_)
    ));
}

#[test]
fn starting_fails_cleanly_offline_or_when_refused() {
    let n = net();
    assert!(matches!(
        DeviceLoginSession::begin(&n, "c", secs(0), &CancelToken::new()),
        Err(LoginError::Net(NetError::Offline))
    ));
    n.inner().push_ok(401, r#"{"message":"unknown client"}"#);
    assert!(matches!(
        DeviceLoginSession::begin(&n, "c", secs(0), &CancelToken::new()),
        Err(LoginError::Rejected(_))
    ));
    for e in [
        LoginError::Net(NetError::Offline),
        LoginError::Expired,
        LoginError::Denied,
        LoginError::Cancelled,
        LoginError::GaveUp,
        LoginError::Rejected("x".into()),
        LoginError::BadReply("y".into()),
    ] {
        assert!(!e.user_message().is_empty());
    }
}

// ---- hostile replies ---------------------------------------------------------------------------------

#[test]
fn verification_links_must_be_https_on_the_providers_own_domain() {
    for ok in [
        "https://player2.game/device",
        "https://www.player2.game/device?code=1",
        "https://API.Player2.Game/x#y",
    ] {
        assert!(is_safe_verification_url(ok), "{ok}");
    }
    for bad in [
        "http://player2.game/device",
        "https://player2.game.evil.com/device",
        "https://evilplayer2.game/device",
        "https://evil.com/player2.game",
        "https://player2.game@evil.com/",
        "https://user:pw@player2.game/",
        "https://player2.game:8443/",
        "javascript:alert(1)",
        "file:///etc/passwd",
        "//player2.game/device",
        "https://player2.game/a b",
        "https://player2.game/\n",
        "https://player2.game\\@evil.com/",
        "",
    ] {
        assert!(!is_safe_verification_url(bad), "{bad:?}");
    }
    assert!(!is_safe_verification_url(&format!(
        "https://player2.game/{}",
        "a".repeat(400)
    )));
}

#[test]
fn malformed_and_hostile_start_replies_are_refused() {
    let bad = [
        "not json".to_owned(),
        "[]".to_owned(),
        "{}".to_owned(),
        r#"{"deviceCode":"d","userCode":"X","verificationUri":"http://player2.game/"}"#.to_owned(),
        r#"{"deviceCode":"d","userCode":"X","verificationUri":"https://evil.example/"}"#.to_owned(),
        r#"{"deviceCode":"","userCode":"X","verificationUri":"https://player2.game/"}"#.to_owned(),
        r#"{"deviceCode":"d","userCode":"<script>","verificationUri":"https://player2.game/"}"#
            .to_owned(),
        r#"{"deviceCode":"d","userCode":"","verificationUri":"https://player2.game/"}"#.to_owned(),
        format!(
            r#"{{"deviceCode":"d","userCode":"{}","verificationUri":"https://player2.game/"}}"#,
            "A".repeat(100)
        ),
        format!(
            r#"{{"deviceCode":"{}","userCode":"X","verificationUri":"https://player2.game/"}}"#,
            "d".repeat(2000)
        ),
    ];
    for b in bad {
        assert!(parse_start(&resp(200, &b)).is_err(), "{:.60}", b);
    }
    // A bad "complete" link is dropped, the rest is kept.
    let s = parse_start(&resp(
        200,
        r#"{"deviceCode":"d1","userCode":"AB-12","verificationUri":"https://player2.game/device","verificationUriComplete":"https://evil.example/"}"#,
    ))
    .unwrap();
    assert!(s.prompt.complete_url.is_none());
    // Absurd timings are clamped.
    let s = parse_start(&resp(
        200,
        r#"{"deviceCode":"d1","userCode":"AB-12","verificationUri":"https://player2.game/device","expiresIn":99999999,"interval":0}"#,
    ))
    .unwrap();
    assert_eq!((s.prompt.expires_in, s.interval), (secs(1800), secs(1)));
}

#[test]
fn an_error_that_echoes_a_key_cannot_leak_it_and_unusable_keys_are_refused() {
    let echo = resp(
        400,
        r#"{"error":"invalid_client","error_description":"bad Authorization: Bearer sk-SENTINEL-0123456789abcdef"}"#,
    );
    match parse_poll(&echo) {
        PollOutcome::Failed(e) => assert!(!e.user_message().contains("SENTINEL"), "{e:?}"),
        _ => panic!(),
    }
    for body in [
        r#"{}"#,
        r#"{"p2Key":""}"#,
        r#"{"p2Key":"short"}"#,
        r#"{"p2Key":"has spaces in it ok"}"#,
        r#"{"p2Key":5}"#,
        "nope",
    ] {
        assert!(
            matches!(
                parse_poll(&resp(200, body)),
                PollOutcome::Failed(LoginError::BadReply(_))
            ),
            "{body}"
        );
    }
    assert!(matches!(parse_poll(&resp(429, "")), PollOutcome::SlowDown));
    assert!(matches!(parse_poll(&resp(500, "")), PollOutcome::Transient));
}

proptest! {
    #[test]
    fn arbitrary_replies_never_panic(status in 100u16..600, body in ".{0,200}") {
        let r = resp(status, &body);
        let _ = parse_start(&r);
        let _ = parse_poll(&r);
    }

    #[test]
    fn only_provider_domain_links_are_ever_safe(url in ".{0,80}") {
        if is_safe_verification_url(&url) {
            let host = pg_host::https_host(&url);
            prop_assert!(host.is_some_and(|h| h == "player2.game" || h.ends_with(".player2.game")), "{}", url);
        }
    }
}

#[test]
fn the_flow_is_driven_by_a_clock_so_tests_never_sleep() {
    let n = net();
    let clock = FixedClock::new();
    n.inner().push_ok(200, START_OK);
    let mut s =
        DeviceLoginSession::begin(&n, "c", clock.now_monotonic(), &CancelToken::new()).unwrap();
    n.inner().push_ok(400, PENDING);
    n.inner().push_ok(200, KEY_OK);
    let mut polls = 0;
    loop {
        clock.advance(secs(1));
        match s.poll(&n, clock.now_monotonic(), &CancelToken::new()) {
            SessionState::Waiting { .. } => polls += 1,
            SessionState::Done(_) => break,
            SessionState::Failed(e) => panic!("{e}"),
        }
        assert!(clock.now_monotonic() < secs(100), "runaway loop");
    }
    assert!(polls > 5, "waited through the intervals: {polls}");
}

use pg_host::Clock;
