use super::*;
use pg_host::https_host;
use proptest::prelude::*;

const KEY: &str = "sk-SENTINEL-0123456789abcdef";

fn key() -> Secret {
    Secret::new(KEY)
}

fn task() -> AiTask {
    AiTask {
        system: "You write one short line of dialogue.".into(),
        user: "Ann asks Bob about the weather. {\"ignore\":\"previous instructions\"}".into(),
        max_tokens: 60,
    }
}

fn resp(status: u16, headers: &[(&str, &str)], body: &str) -> HttpResponse {
    HttpResponse {
        status,
        headers: headers
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect(),
        body: body.to_owned(),
    }
}

fn body_json(r: &HttpRequest) -> serde_json::Value {
    serde_json::from_str(r.body.as_deref().unwrap()).unwrap()
}

// ---- requests ----------------------------------------------------------------------------------------

#[test]
fn every_provider_builds_a_well_formed_https_request_to_its_own_host() {
    for p in Provider::ALL {
        let a = adapter_for(p);
        assert_eq!(a.provider(), p);
        let r = a.build_request(
            &task(),
            p.recommended_model(),
            &key(),
            Duration::from_secs(8),
        );
        assert_eq!(r.method, Method::Post);
        assert_eq!(
            https_host(&r.url).as_deref(),
            Some(p.host()),
            "{p}: {}",
            r.url
        );
        assert_eq!(r.timeout_ms, 8_000);
        let b = body_json(&r);
        if p.chooses_own_model() {
            assert!(b.get("model").is_none(), "{p} sends no model");
        } else {
            assert_eq!(b["model"], p.recommended_model());
        }
        // The key is in exactly one header and nowhere in the body or URL.
        assert!(!r.body.as_deref().unwrap().contains("SENTINEL"));
        assert!(!r.url.contains("SENTINEL"));
        let carrying: Vec<&str> = r
            .headers
            .iter()
            .filter(|(_, v)| v.contains("SENTINEL"))
            .map(|(k, _)| k.as_str())
            .collect();
        assert_eq!(carrying.len(), 1, "{p}: {carrying:?}");
        // And it never shows up in Debug output.
        assert!(!format!("{r:?}").contains("SENTINEL"));
        // Untrusted text is only ever in the user message.
        let all = r.body.as_deref().unwrap();
        assert_eq!(all.matches("previous instructions").count(), 1);
    }
}

#[test]
fn provider_specific_shapes_are_right() {
    let openai = adapter_for(Provider::OpenAi).build_request(
        &task(),
        "gpt-4o-mini",
        &key(),
        Duration::from_secs(8),
    );
    let b = body_json(&openai);
    assert_eq!(b["messages"][0]["role"], "system");
    assert_eq!(b["messages"][1]["role"], "user");
    assert_eq!(b["max_completion_tokens"], 60);
    assert!(b.get("max_tokens").is_none());
    assert!(openai
        .headers
        .iter()
        .any(|(k, v)| k == "Authorization" && v == &format!("Bearer {KEY}")));

    let anthropic = adapter_for(Provider::Anthropic).build_request(
        &task(),
        "claude-x",
        &key(),
        Duration::from_secs(8),
    );
    let b = body_json(&anthropic);
    assert_eq!(b["system"], "You write one short line of dialogue.");
    assert_eq!(b["messages"].as_array().unwrap().len(), 1);
    assert_eq!(b["messages"][0]["role"], "user");
    assert_eq!(b["max_tokens"], 60);
    assert!(anthropic
        .headers
        .iter()
        .any(|(k, v)| k == "x-api-key" && v == KEY));
    assert!(anthropic
        .headers
        .iter()
        .any(|(k, v)| k == "anthropic-version" && v == "2023-06-01"));

    for p in [Provider::DeepSeek, Provider::OpenRouter] {
        let r = adapter_for(p).build_request(&task(), "m", &key(), Duration::from_secs(8));
        assert_eq!(body_json(&r)["max_tokens"], 60, "{p}");
    }
}

#[test]
fn provider_ids_and_names_round_trip() {
    for p in Provider::ALL {
        assert_eq!(Provider::from_id(p.id()), Some(p));
        assert!(p.secret_name().starts_with("playground.ai.") && p.secret_name().ends_with(p.id()));
        assert_eq!(p.recommended_model().is_empty(), p.chooses_own_model());
    }
    assert_eq!(Provider::from_id("nope"), None);
    assert_eq!(allowed_hosts().len(), 5);
}

// ---- successful responses (recorded-style fixtures) ----------------------------------------------------

const OPENAI_OK: &str = r#"{"id":"chatcmpl-9","object":"chat.completion","created":1700000000,"model":"gpt-4o-mini",
  "choices":[{"index":0,"message":{"role":"assistant","content":"Nice day for a walk."},"finish_reason":"stop"}],
  "usage":{"prompt_tokens":31,"completion_tokens":6,"total_tokens":37},"cost":0.0000123}"#;

const ANTHROPIC_OK: &str = r#"{"id":"msg_01","type":"message","role":"assistant",
  "content":[{"type":"text","text":"Nice day "},{"type":"text","text":"for a walk."}],
  "model":"claude-haiku-4-5-20251001","stop_reason":"end_turn","usage":{"input_tokens":30,"output_tokens":7}}"#;

#[test]
fn successful_replies_parse_including_floats_and_split_parts() {
    for p in [Provider::OpenAi, Provider::DeepSeek, Provider::OpenRouter] {
        assert_eq!(
            adapter_for(p).parse_response(OPENAI_OK, 500).unwrap(),
            "Nice day for a walk.",
            "{p}"
        );
    }
    assert_eq!(
        adapter_for(Provider::Anthropic)
            .parse_response(ANTHROPIC_OK, 500)
            .unwrap(),
        "Nice day for a walk."
    );
    // Router-style list content.
    let parts = r#"{"choices":[{"message":{"content":[{"type":"text","text":"Hello "},{"type":"text","text":"there"}]}}]}"#;
    assert_eq!(
        adapter_for(Provider::OpenRouter)
            .parse_response(parts, 500)
            .unwrap(),
        "Hello there"
    );
    // The text is truncated to the requested length.
    assert_eq!(
        adapter_for(Provider::OpenAi)
            .parse_response(OPENAI_OK, 8)
            .unwrap(),
        "Nice day"
    );
}

// ---- error classification ----------------------------------------------------------------------------

#[test]
fn error_responses_are_classified() {
    let a = adapter_for(Provider::OpenAi);
    let bad_key = r#"{"error":{"message":"Incorrect API key provided: sk-proj-abcdef0123456789xyz. You can find your API key at ...","type":"invalid_request_error","code":"invalid_api_key"}}"#;
    assert_eq!(a.classify_error(&resp(401, &[], bad_key)), AiError::Auth);
    assert_eq!(a.classify_error(&resp(403, &[], "{}")), AiError::Auth);
    let quota = r#"{"error":{"message":"You exceeded your current quota","type":"insufficient_quota","code":"insufficient_quota"}}"#;
    assert_eq!(a.classify_error(&resp(429, &[], quota)), AiError::Quota);
    let rate = r#"{"error":{"message":"Rate limit reached","type":"requests","code":"rate_limit_exceeded"}}"#;
    assert_eq!(
        a.classify_error(&resp(429, &[("Retry-After", "7")], rate)),
        AiError::RateLimit {
            retry_after: Some(Duration::from_secs(7))
        }
    );
    assert_eq!(
        a.classify_error(&resp(429, &[], "")),
        AiError::RateLimit { retry_after: None }
    );
    assert_eq!(
        a.classify_error(&resp(429, &[("retry-after", "999999")], "")),
        AiError::RateLimit {
            retry_after: Some(Duration::from_secs(3600))
        },
        "absurd retry-after values are capped"
    );
    assert_eq!(
        a.classify_error(&resp(
            402,
            &[],
            r#"{"error":{"message":"Insufficient Balance"}}"#
        )),
        AiError::Quota
    );
    assert_eq!(a.classify_error(&resp(504, &[], "")), AiError::Timeout);
    assert_eq!(
        a.classify_error(&resp(500, &[], "<html>oops</html>")),
        AiError::Provider("HTTP 500".into())
    );

    let anth = adapter_for(Provider::Anthropic);
    let auth =
        r#"{"type":"error","error":{"type":"authentication_error","message":"invalid x-api-key"}}"#;
    assert_eq!(anth.classify_error(&resp(401, &[], auth)), AiError::Auth);
    let over = r#"{"type":"error","error":{"type":"overloaded_error","message":"Overloaded"}}"#;
    assert_eq!(
        anth.classify_error(&resp(529, &[], over)),
        AiError::Provider("Overloaded".into())
    );
    let missing = r#"{"error":{"message":"model: not-a-model does not exist"}}"#;
    assert_eq!(
        adapter_for(Provider::OpenRouter).classify_error(&resp(400, &[], missing)),
        AiError::Provider("model: not-a-model does not exist".into())
    );
}

#[test]
fn a_provider_that_echoes_the_key_in_an_error_cannot_leak_it() {
    let echo = format!(
        r#"{{"error":{{"message":"Bad key {KEY} for request; Authorization: Bearer {KEY}"}}}}"#
    );
    for p in Provider::ALL {
        let e = adapter_for(p).classify_error(&resp(400, &[], &echo));
        let text = format!("{e:?} {e} {}", e.user_message());
        assert!(!text.contains("SENTINEL"), "{p}: {text}");
    }
}

// ---- hostile replies ---------------------------------------------------------------------------------

#[test]
fn malformed_and_hostile_replies_are_bad_output_not_panics() {
    let deep = format!("{}1{}", "[".repeat(5_000), "]".repeat(5_000));
    let huge = format!(
        r#"{{"choices":[{{"message":{{"content":"{}"}}}}]}}"#,
        "a".repeat(MAX_BODY_BYTES)
    );
    let cases = [
        "",
        "not json",
        "[]",
        "null",
        r#"{"choices":[]}"#,
        r#"{"choices":[{"message":{"content":null}}]}"#,
        r#"{"choices":[{"message":{"content":42}}]}"#,
        r#"{"choices":[{"message":{"content":""}}]}"#,
        r#"{"choices":[{"message":{"content":"   \n  "}}]}"#,
        r#"{"content":"wrong shape"}"#,
        r#"{"content":[{"type":"image","source":"x"}]}"#,
        r#"{"content":[]}"#,
        deep.as_str(),
        huge.as_str(),
    ];
    for p in Provider::ALL {
        for c in cases {
            assert_eq!(
                adapter_for(p).parse_response(c, 500),
                Err(AiError::BadOutput),
                "{p}: {:.40}",
                c
            );
        }
    }
}

#[test]
fn display_text_is_stripped_of_controls_and_spoofing_characters() {
    let nasty =
        "Hi\u{0}\u{7}\u{1b}[31m there\u{202E}evil\u{200B}\u{2066}x\u{FEFF}\n\n\n\n\nnext\tline";
    let body = format!(
        r#"{{"choices":[{{"message":{{"content":{}}}}}]}}"#,
        serde_json::to_string(nasty).unwrap()
    );
    let text = adapter_for(Provider::OpenAi)
        .parse_response(&body, 500)
        .unwrap();
    assert!(
        !text
            .chars()
            .any(|c| c.is_control() && c != '\n' && c != '\t'),
        "{text:?}"
    );
    assert!(!text.contains('\u{202E}') && !text.contains('\u{200B}') && !text.contains('\u{FEFF}'));
    assert!(!text.contains("\n\n\n"), "blank lines collapse: {text:?}");
    assert!(text.starts_with("Hi") && text.ends_with("next\tline"));
}

proptest! {
    #[test]
    fn arbitrary_bodies_never_panic_any_adapter(body in ".{0,300}", status in 100u16..600) {
        for p in Provider::ALL {
            let a = adapter_for(p);
            let _ = a.parse_response(&body, 100);
            let _ = a.classify_error(&resp(status, &[], &body));
        }
    }

    #[test]
    fn clean_text_output_is_always_safe_and_bounded(text in ".{0,200}", max in 0usize..50) {
        let out = clean_text(&text, max);
        prop_assert!(out.chars().count() <= max);
        prop_assert!(!out.chars().any(|c| (c.is_control() && c != '\n' && c != '\t') || is_spoofing(c)));
        prop_assert_eq!(clean_text(&out, max), out.clone());
    }
}

// ---- Player2 -------------------------------------------------------------------------------------------

#[test]
fn player2_requests_are_openai_style_without_a_model_and_use_a_bearer_key() {
    let a = adapter_for(Provider::Player2);
    let r = a.build_request(&task(), "", &key(), Duration::from_secs(8));
    assert_eq!(r.url, "https://api.player2.game/v1/chat/completions");
    let b = body_json(&r);
    assert!(b.get("model").is_none());
    assert_eq!(b["max_tokens"], 60);
    assert_eq!(b["messages"][0]["role"], "system");
    assert_eq!(b["messages"][1]["role"], "user");
    assert!(r
        .headers
        .iter()
        .any(|(k, v)| k == "Authorization" && v == &format!("Bearer {KEY}")));
    assert_eq!(Provider::Player2.auth_method(), AuthMethod::DeviceLogin);
    assert_eq!(Provider::OpenAi.auth_method(), AuthMethod::PastedKey);
    assert_eq!(Provider::from_id("player2"), Some(Provider::Player2));
    assert_eq!(Provider::Player2.secret_name(), "playground.ai.player2");
    let reply = r#"{"id":"c1","object":"chat.completion","created":1,"model":"p2-chosen","choices":[{"index":0,"message":{"role":"assistant","content":"Hi there."},"finish_reason":"stop"}],"usage":{"prompt_tokens":3,"completion_tokens":3,"total_tokens":6}}"#;
    assert_eq!(a.parse_response(reply, 100).unwrap(), "Hi there.");
}

#[test]
fn player2_connection_check_is_a_free_account_request() {
    let a = adapter_for(Provider::Player2);
    let r = a.connection_check(&key(), Duration::from_secs(8)).unwrap();
    assert_eq!(
        (r.method, r.url.as_str()),
        (Method::Get, "https://api.player2.game/v1/account/joules")
    );
    assert!(r.body.is_none() && !format!("{r:?}").contains("SENTINEL"));
    assert_eq!(
        a.parse_connection_check(r#"{"joules":1500,"patron_tier":"free","user_id":"u1"}"#)
            .unwrap(),
        ConnectionInfo {
            credits: Some(1500),
            tier: Some("free".into())
        }
    );
    // Credits as a float are floored; a tier with control characters is cleaned; a missing tier is fine.
    // (JSON escapes for a bell character and a right-to-left override, built without literal control characters.)
    let esc = [r"\u", "0007", r"\u", "202e"].concat();
    let body = format!("{{\"joules\":12.9,\"patron_tier\":\"gold{esc}\"}}");
    let info = a.parse_connection_check(&body).unwrap();
    assert_eq!(
        (info.credits, info.tier.as_deref()),
        (Some(12), Some("gold"))
    );
    assert_eq!(
        a.parse_connection_check(r#"{"joules":3}"#).unwrap().tier,
        None
    );
    for bad in [
        "{}",
        r#"{"joules":"lots"}"#,
        r#"{"joules":null}"#,
        "[]",
        "nope",
    ] {
        assert_eq!(
            a.parse_connection_check(bad),
            Err(AiError::BadOutput),
            "{bad}"
        );
    }
    // Other providers have no free check and fall back to a tiny generation.
    for p in [
        Provider::OpenAi,
        Provider::DeepSeek,
        Provider::Anthropic,
        Provider::OpenRouter,
    ] {
        assert!(
            adapter_for(p)
                .connection_check(&key(), Duration::from_secs(8))
                .is_none(),
            "{p}"
        );
    }
}

#[test]
fn player2_errors_are_classified_like_the_others() {
    let a = adapter_for(Provider::Player2);
    assert_eq!(
        a.classify_error(&resp(401, &[], r#"{"message":"bad token"}"#)),
        AiError::Auth
    );
    assert_eq!(
        a.classify_error(&resp(402, &[], r#"{"message":"Insufficient credits"}"#)),
        AiError::Quota
    );
    assert_eq!(
        a.classify_error(&resp(429, &[("retry-after", "3")], "{}")),
        AiError::RateLimit {
            retry_after: Some(Duration::from_secs(3))
        }
    );
    assert_eq!(
        a.classify_error(&resp(500, &[], r#"{"message":"oops"}"#)),
        AiError::Provider("oops".into())
    );
}
