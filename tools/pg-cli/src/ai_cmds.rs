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
    values: &["dir", "provider", "model", "from-env", "client-id"],
    switches: &["dry-run", "enable", "disable", "default-model"],
    optional: &[],
};

const USAGE: &str = "usage: pg ai providers | key set <provider> [--from-env VAR] | key clear <provider> | key status | settings show|set [--provider P] [--model M | --default-model] [--enable | --disable] | login player2 [--client-id ID] | test [--provider P] [--model M] [--dry-run] | selfcheck   (settings flags take --dir <data dir>, default ./pg-data)";

pub fn ai_cmd(args: &[String]) -> Result<ExitCode, String> {
    match args.split_first() {
        Some((c, rest)) => match c.as_str() {
            "providers" => providers(rest),
            "key" => key(rest),
            "settings" => settings(rest),
            "login" => login(rest),
            "test" => test(rest),
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
