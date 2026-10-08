//! Developer commands for the AI client skeleton (0.7): `ai providers | key | settings | test | selfcheck`.

use crate::args::{parse, Parsed, Spec};
use pg_ai::client::{AiClient, ClientConfig};
use pg_ai::login::{store_key, DeviceLoginSession, SessionState, CLIENT_ID};
use pg_ai::provider::{adapter_for, allowed_hosts, AiTask, AuthMethod, Provider};
use pg_ai::selfcheck;
use pg_ai::settings::{validate_model_id, DeviceSettings, KeyManager};
use pg_host::{AllowListNet, CancelToken, Clock, RedactingLog, Secret, StderrLog};
use pg_host_os::{FsStorage, KeyringSecretStore, SystemClock, UreqNet};
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

const SPEC: Spec<'static> = Spec {
    values: &[
        "dir",
        "provider",
        "model",
        "from-env",
        "client-id",
        "seed",
        "reply",
        "tone",
        "filter",
    ],
    switches: &["dry-run", "enable", "disable", "default-model"],
    optional: &[],
};

const USAGE: &str = "usage: pg ai providers | key set <provider> [--from-env VAR] | key clear <provider> | key status | settings show|set [--provider P] [--model M | --default-model] [--enable | --disable] | login player2 [--client-id ID] | test [--provider P] [--model M] [--dry-run] | dialogue-test [--seed S] [--reply TEXT] [--tone cozy|standard|mature] [--filter on|off] | selfcheck   (settings flags take --dir <data dir>, default ./pg-data)";

pub fn ai_cmd(args: &[String]) -> Result<ExitCode, String> {
    match args.split_first() {
        Some((c, rest)) => match c.as_str() {
            "providers" => providers(rest),
            "key" => key(rest),
            "settings" => settings(rest),
            "login" => login(rest),
            "test" => test(rest),
            "dialogue-test" => dialogue_test(rest),
            "selfcheck" => Ok(selfcheck_cmd()),
            other => Err(format!("unknown ai command '{other}'\n{USAGE}")),
        },
        None => Err(USAGE.into()),
    }
}

fn data_dir(p: &Parsed) -> String {
    p.one("dir").unwrap_or("pg-data").to_owned()
}

fn provider_arg(text: &str) -> Result<Provider, String> {
    Provider::from_id(text).ok_or_else(|| {
        format!(
            "unknown provider '{text}' (one of: {})",
            Provider::ALL.map(Provider::id).join(", ")
        )
    })
}

fn providers(args: &[String]) -> Result<ExitCode, String> {
    let _ = parse(args, &SPEC)?;
    let store = KeyringSecretStore::new();
    let keys = KeyManager::new(&store);
    println!(
        "{:<11} {:<18} {:<28} key",
        "provider", "host", "recommended model"
    );
    for p in Provider::ALL {
        println!(
            "{:<11} {:<18} {:<28} {}",
            p.id(),
            p.host(),
            if p.chooses_own_model() {
                "(chosen by the provider)"
            } else {
                p.recommended_model()
            },
            if keys.has_key(p) { "stored" } else { "not set" }
        );
    }
    println!(
        "\nkeys are kept in the OS credential store{}",
        if keys.is_persistent() {
            ""
        } else {
            " (unavailable here: session-only)"
        }
    );
    Ok(ExitCode::SUCCESS)
}

fn key(args: &[String]) -> Result<ExitCode, String> {
    let p = parse(args, &SPEC)?;
    let store = KeyringSecretStore::new();
    let keys = KeyManager::new(&store);
    match p.positional.first().map(String::as_str) {
        Some("set") => {
            let provider = provider_arg(p.positional.get(1).ok_or(USAGE)?)?;
            let text = match p.one("from-env") {
                Some(var) => std::env::var(var)
                    .map_err(|_| format!("environment variable {var} is not set"))?,
                None => {
                    eprintln!("paste the {provider} key and press Enter (prefer --from-env to keep it out of your terminal):");
                    let mut line = String::new();
                    std::io::stdin()
                        .read_line(&mut line)
                        .map_err(|e| e.to_string())?;
                    line
                }
            };
            keys.set_key(provider, &text)?;
            println!(
                "stored a key for {provider}{}",
                if keys.is_persistent() {
                    ""
                } else {
                    " (session-only: this machine has no usable credential store, so it is gone when this program exits)"
                }
            );
            Ok(ExitCode::SUCCESS)
        }
        Some("clear") => {
            let provider = provider_arg(p.positional.get(1).ok_or(USAGE)?)?;
            keys.clear_key(provider)?;
            println!("removed the key for {provider}");
            Ok(ExitCode::SUCCESS)
        }
        Some("status") => providers(&[]),
        _ => Err(USAGE.into()),
    }
}

fn settings(args: &[String]) -> Result<ExitCode, String> {
    let p = parse(args, &SPEC)?;
    let storage = FsStorage::new(data_dir(&p)).map_err(|e| e.to_string())?;
    let mut s = DeviceSettings::load(&storage).map_err(|e| e.to_string())?;
    match p.positional.first().map(String::as_str) {
        Some("show") | None => {}
        Some("set") => {
            if let Some(t) = p.one("provider") {
                s.ai.provider = provider_arg(t)?;
            }
            if let Some(m) = p.one("model") {
                if s.ai.provider.chooses_own_model() {
                    return Err(format!(
                        "{} chooses its own model; --model is not accepted",
                        s.ai.provider
                    ));
                }
                validate_model_id(m)?;
                s.ai.custom_model = Some(m.to_owned());
            }
            if p.has("default-model") || s.ai.provider.chooses_own_model() {
                s.ai.custom_model = None;
            }
            if p.has("enable") {
                s.ai.enabled = true;
            }
            if p.has("disable") {
                s.ai.enabled = false;
            }
            s.save(&storage).map_err(|e| e.to_string())?;
        }
        Some(other) => return Err(format!("unknown settings command '{other}'\n{USAGE}")),
    }
    println!(
        "AI: {}  provider: {}  model: {}{}",
        if s.ai.enabled { "on" } else { "off" },
        s.ai.provider,
        s.ai.effective_model(),
        if s.ai.provider.chooses_own_model() {
            " (chosen by the provider)"
        } else if s.ai.custom_model.is_some() {
            " (custom)"
        } else {
            " (recommended)"
        }
    );
    Ok(ExitCode::SUCCESS)
}

fn test(args: &[String]) -> Result<ExitCode, String> {
    let p = parse(args, &SPEC)?;
    let storage = FsStorage::new(data_dir(&p)).map_err(|e| e.to_string())?;
    let mut s = DeviceSettings::load(&storage)
        .map_err(|e| e.to_string())?
        .ai;
    if let Some(t) = p.one("provider") {
        s.provider = provider_arg(t)?;
    }
    if let Some(m) = p.one("model") {
        if s.provider.chooses_own_model() {
            return Err(format!(
                "{} chooses its own model; --model is not accepted",
                s.provider
            ));
        }
        validate_model_id(m)?;
        s.custom_model = Some(m.to_owned());
    }
    if s.provider.chooses_own_model() {
        s.custom_model = None;
    }
    println!(
        "provider {} ({}), model {}",
        s.provider,
        s.provider.host(),
        if s.provider.chooses_own_model() {
            "(chosen by the provider)".to_owned()
        } else {
            s.effective_model()
        }
    );
    if p.has("dry-run") {
        // Show exactly what would be sent, with the credential hidden by the request's own Debug output.
        let req = adapter_for(s.provider).build_request(
            &AiTask::connection_test(),
            &s.effective_model(),
            &Secret::new("DRY-RUN-NOT-A-REAL-KEY"),
            Duration::from_secs(8),
        );
        println!("{req:?}");
        println!("body: {}", req.body.as_deref().unwrap_or(""));
        return Ok(ExitCode::SUCCESS);
    }
    let secrets = Arc::new(KeyringSecretStore::new());
    let client = AiClient::new(
        Arc::new(AllowListNet::new(UreqNet, &allowed_hosts())),
        secrets,
        Arc::new(SystemClock::new()),
        Arc::new(RedactingLog::new(StderrLog)),
        ClientConfig::default(),
    );
    match client.test_connection(&s, &CancelToken::new()) {
        Ok(info) => {
            println!("connection test passed");
            if let Some(c) = info.credits {
                println!(
                    "credits: {c}{}",
                    info.tier.map_or(String::new(), |t| format!("  tier: {t}"))
                );
            }
            Ok(ExitCode::SUCCESS)
        }
        Err(e) => {
            println!("{}", e.user_message());
            Ok(ExitCode::FAILURE)
        }
    }
}

/// `pg ai login player2`: the device-code sign-in. Shows a code and a link, polls until approved, and stores
/// the key in the credential store. Needs a client id registered with the provider.
fn login(args: &[String]) -> Result<ExitCode, String> {
    let p = parse(args, &SPEC)?;
    let provider = provider_arg(p.positional.first().map_or("player2", String::as_str))?;
    if provider.auth_method() != AuthMethod::DeviceLogin {
        return Err(format!(
            "{provider} uses a pasted key: pg ai key set {}",
            provider.id()
        ));
    }
    let client_id = p
        .one("client-id")
        .map(str::to_owned)
        .or_else(|| std::env::var("PG_PLAYER2_CLIENT_ID").ok())
        .unwrap_or_else(|| CLIENT_ID.to_owned());
    let net = AllowListNet::new(UreqNet, &allowed_hosts());
    let clock = SystemClock::new();
    let cancel = CancelToken::new();
    let mut session = DeviceLoginSession::begin(&net, &client_id, clock.now_monotonic(), &cancel)
        .map_err(|e| e.user_message())?;
    let prompt = session.prompt().clone();
    println!(
        "open {}",
        prompt
            .complete_url
            .as_deref()
            .unwrap_or(&prompt.verification_url)
    );
    println!("and confirm the code: {}", prompt.user_code);
    println!(
        "(expires in {} s; Ctrl+C to cancel)",
        prompt.expires_in.as_secs()
    );
    loop {
        match session.poll(&net, clock.now_monotonic(), &cancel) {
            SessionState::Waiting { next_in } => std::thread::sleep(next_in),
            SessionState::Done(key) => {
                store_key(&KeyringSecretStore::new(), provider, &key)?;
                println!("signed in; the key for {provider} is stored in the credential store");
                return Ok(ExitCode::SUCCESS);
            }
            SessionState::Failed(e) => {
                println!("{}", e.user_message());
                return Ok(ExitCode::FAILURE);
            }
        }
    }
}

fn selfcheck_cmd() -> ExitCode {
    let report = selfcheck::run();
    for c in &report.checks {
        println!(
            "  {}  {} ({})",
            if c.ok { "PASS" } else { "FAIL" },
            c.name,
            c.detail
        );
    }
    if report.ok() {
        println!("redaction self-check PASSED: the sentinel key appeared nowhere it should not");
        ExitCode::SUCCESS
    } else {
        println!("redaction self-check FAILED");
        ExitCode::FAILURE
    }
}

/// `pg ai dialogue-test`: finds the first conversation in a generated town and shows what the AI would be
/// sent, the fallback lines the game would use, and (with `--reply`) whether a given reply would be accepted.
/// Makes no network request.
fn dialogue_test(args: &[String]) -> Result<ExitCode, String> {
    use pg_ai::dialogue::{build_task, parse_lines, ContentRules};
    use pg_core::commands::Command as WorldCommand;
    use pg_core::input::SimInput;
    use pg_core::pipeline::Pipeline;
    use pg_core::sim::Sim;
    use pg_core::world::WorldState;
    let p = parse(args, &SPEC)?;
    let seed = p.one("seed").unwrap_or("dialogue");
    let content = crate::shared::load_content(&[crate::shared::DEFAULT_CONTENT_DIR.to_owned()])?;
    let mut sim =
        Sim::new(WorldState::new("Dialogue", seed), Pipeline::new()).with_content(content);
    sim.submit(
        0,
        SimInput::Command {
            actor: None,
            cmd: WorldCommand::GenerateTown {
                w: 48,
                h: 36,
                water: 15,
                residents: 12,
                tone: "standard".into(),
            },
        },
    )
    .map_err(|e| e.to_string())?;
    let rules = ContentRules {
        tone_preset: p.one("tone").unwrap_or("standard").to_owned(),
        graphic_filter: p.one("filter") != Some("off"),
    };
    for _ in 0..30_000 {
        let report = sim.step().map_err(|e| e.to_string())?;
        let Some(e) = report
            .events
            .iter()
            .find(|e| e.kind == "conversation.started")
        else {
            continue;
        };
        let id = |k: &str| {
            e.detail
                .get(k)
                .and_then(|c| c.as_str()?.parse::<pg_core::id::EntityId>().ok())
        };
        let (Some(a), Some(b)) = (id("a"), id("b")) else {
            continue;
        };
        let talk = sim
            .world()
            .pawns
            .get(a)
            .and_then(|p| p.talk.clone())
            .ok_or("the conversation ended at once")?;
        let (fallback, req, _) = pg_runtime::dialogue::describe(&sim, a, b, &talk, e.tick, &rules)
            .ok_or("no conversation data")?;
        let task = build_task(&req);
        println!(
            "{} and {}: {} ({} tone), {} turn(s), tick {}
",
            req.first, req.second, req.topic, req.tone, req.turns, e.tick
        );
        println!(
            "--- system (trusted) ---
{}

--- user (scene data) ---
{}
",
            task.system, task.user
        );
        println!("--- fallback lines the game uses ---");
        for (who, line) in &fallback {
            let name = if *who == a { &req.first } else { &req.second };
            println!("  {name}: {line}");
        }
        if let Some(reply) = p.one("reply") {
            println!(
                "
--- the reply you gave ---"
            );
            match parse_lines(reply, req.turns, &rules) {
                Ok(lines) => {
                    println!("accepted:");
                    for l in lines {
                        println!("  {l}");
                    }
                }
                Err(r) => println!("REFUSED: {} (the game would use the fallback lines)", r.0),
            }
        }
        return Ok(ExitCode::SUCCESS);
    }
    Err("nobody talked in two game days with this seed".into())
}
