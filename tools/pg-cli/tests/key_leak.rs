//! The sentinel-key leak test (Blueprint §10.2 item 3, Roadmap Stage 0 "Done when").
//!
//! The whole application surface runs with a recognisable key in the credential store: AI calls that
//! succeed and fail (with providers that echo the key back), settings, a world save, an export, a replay
//! log, a bug bundle, a crash report and the log. Then every byte that could leave the program, in raw and
//! decompressed form, is searched for the key. The one place the key legitimately appears is the
//! authorization header of a request to an allow-listed provider host.

use pg_ai::client::{AiClient, ClientConfig};
use pg_ai::provider::{allowed_hosts, AiTask, Provider};
use pg_ai::selfcheck::{NEEDLE, SENTINEL_KEY};
use pg_ai::settings::{AiSettings, DeviceSettings, KeyManager};
use pg_core::commands::Command;
use pg_core::id::{EntityId, Kind};
use pg_core::input::SimInput;
use pg_core::replay::ReplayLog;
use pg_core::sim::Sim;
use pg_core::world::WorldState;
use pg_host::{
    AllowListNet, CancelToken, FixedClock, Level, LogSink, MemLog, MemSecretStore, MemStorage,
    NetError, RedactingLog, ScriptedNet, Secret, Storage,
};
use pg_persist::codec::{self, Container};
use pg_persist::crash::{write_report, CrashInfo};
use pg_persist::export::export_world;
use pg_persist::logfile::{encode_log, Bundle};
use pg_persist::store::SlotStore;
use std::sync::Arc;

fn cmd(c: Command) -> SimInput {
    SimInput::Command {
        actor: None,
        cmd: c,
    }
}

fn world_and_log() -> (WorldState, ReplayLog) {
    let mut sim = Sim::with_dev_systems(WorldState::new("Leak Town", "leak-seed"));
    sim.submit(
        0,
        cmd(Command::DevCreateMap {
            w: 24,
            h: 18,
            style: 1,
        }),
    )
    .unwrap();
    for i in 0..4 {
        sim.submit(
            0,
            cmd(Command::DevSpawnPawn {
                map: EntityId::new(Kind::Map, 1),
                at: None,
                name: format!("P{i}"),
            }),
        )
        .unwrap();
    }
    sim.run_ticks(20_000).unwrap();
    (sim.world().clone(), ReplayLog::record(&sim))
}

/// Raw bytes plus, for containers, their decompressed payloads.
fn expand(bytes: &[u8]) -> Vec<Vec<u8>> {
    let mut out = vec![bytes.to_vec()];
    for kind in Container::ALL {
        if let Ok((_, payload)) = codec::decode_as(kind, bytes, 1 << 30) {
            out.push(payload);
        }
    }
    out
}

fn contains(haystack: &[u8], needle: &str) -> bool {
    haystack
        .windows(needle.len())
        .any(|w| w == needle.as_bytes())
}

#[test]
fn the_sentinel_key_appears_nowhere_it_must_not() {
    // --- the running application, with the key in the credential store ---
    let storage = MemStorage::new();
    let secrets = Arc::new(MemSecretStore::new());
    let raw_log = Arc::new(MemLog::new());
    // Every log path goes through the redacting wrapper, as in the real app.
    struct Shared(Arc<MemLog>);
    impl LogSink for Shared {
        fn log(&self, level: Level, line: &str) {
            self.0.log(level, line);
        }
    }
    let log: Arc<RedactingLog<Shared>> = Arc::new(RedactingLog::new(Shared(raw_log.clone())));
    let net = Arc::new(AllowListNet::new(ScriptedNet::new(), &allowed_hosts()));
    let client = AiClient::new(
        net.clone(),
        secrets.clone(),
        Arc::new(FixedClock::new()),
        log.clone(),
        ClientConfig::default(),
    );

    let keys = KeyManager::new(secrets.as_ref());
    keys.set_key(Provider::OpenAi, SENTINEL_KEY).unwrap();
    let settings = AiSettings {
        enabled: true,
        ..AiSettings::default()
    };
    DeviceSettings {
        ai: settings.clone(),
    }
    .save(&storage)
    .unwrap();

    let echo =
        format!(r#"{{"error":{{"message":"bad key {SENTINEL_KEY}","code":"invalid_api_key"}}}}"#);
    net.inner()
        .push_ok(200, r#"{"choices":[{"message":{"content":"Hello."}}]}"#);
    net.inner().push_ok(401, &echo);
    net.inner()
        .push(Err(NetError::Other(format!("proxy said {SENTINEL_KEY}"))));
    net.inner().push_ok(500, &echo);
    net.inner().push_ok(500, &echo);
    let mut texts = Vec::new();
    for n in 0..4 {
        let task = AiTask {
            system: "s".into(),
            user: format!("u{n}"),
            max_tokens: 10,
        };
        match client.generate(&settings, &task, &CancelToken::new()) {
            Ok(r) => texts.push(r.text),
            Err(e) => texts.push(format!("{e:?}{e}{}", e.user_message())),
        }
    }
    // Something deliberately careless: a log line and a panic message that contain the key.
    log.log(Level::Error, &format!("debugging: key is {SENTINEL_KEY}"));
    let panic_message =
        format!("assertion failed while calling with Authorization: Bearer {SENTINEL_KEY}");

    // --- everything that persists or leaves the program ---
    let (world, replay) = world_and_log();
    SlotStore::new(&storage)
        .save("town", &world, &[], "2026-10-07T00:00:00Z")
        .unwrap();
    let export = export_world(&world, &[], "test", "2026-10-07T00:00:00Z");
    let packed_log = encode_log(&replay);
    let bundle = Bundle::make(&replay, 10_000, None, "leak test", "t", "v")
        .unwrap()
        .encode();
    let crash_name = write_report(
        &storage,
        &CrashInfo {
            message: &panic_message,
            location: Some("somewhere.rs:1:1"),
            backtrace: Some("0: main"),
            app_version: "test",
            os: "test",
            time_iso: "2026-10-07T00:00:00Z",
            recent_log: &raw_log
                .lines()
                .iter()
                .map(|(_, l)| l.clone())
                .collect::<Vec<_>>(),
            context: &[],
        },
        &[&Secret::new(SENTINEL_KEY)],
    )
    .unwrap();

    // --- the scan ---
    let mut surfaces: Vec<(String, Vec<Vec<u8>>)> = vec![
        ("export".into(), expand(&export)),
        ("compressed replay log".into(), expand(&packed_log)),
        ("bug bundle".into(), expand(&bundle)),
        ("log".into(), vec![raw_log.text().into_bytes()]),
        (
            "AI errors and replies".into(),
            vec![texts.join("\n").into_bytes()],
        ),
        (
            "debug output".into(),
            vec![format!(
                "{settings:?} {:?} {:?}",
                net.inner().requests(),
                Secret::new(SENTINEL_KEY)
            )
            .into_bytes()],
        ),
    ];
    for blob in storage.list("").unwrap() {
        surfaces.push((
            format!("storage {}", blob.name),
            expand(&storage.read(&blob.name).unwrap().unwrap()),
        ));
    }
    assert!(
        storage.names().iter().any(|n| n == &crash_name)
            && storage.names().iter().any(|n| n.ends_with("manifest.json"))
    );
    for (name, variants) in &surfaces {
        for v in variants {
            assert!(!contains(v, NEEDLE), "the key leaked into: {name}");
        }
    }

    // The key did reach the network, once per request, in a header, to an allow-listed host.
    let reqs = net.inner().requests();
    assert!(!reqs.is_empty());
    for r in &reqs {
        assert!(pg_host::https_host(&r.url).is_some_and(|h| allowed_hosts().contains(&h.as_str())));
        assert!(r.headers.iter().filter(|(_, v)| v.contains(NEEDLE)).count() == 1);
        assert!(!r.body.as_deref().unwrap_or("").contains(NEEDLE) && !r.url.contains(NEEDLE));
    }
    // And the scan itself is not vacuous: the raw log sink received the careless line, redacted.
    assert!(
        raw_log.text().contains("debugging: key is [redacted]"),
        "{}",
        raw_log.text()
    );
}
