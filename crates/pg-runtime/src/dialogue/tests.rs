use super::*;
use pg_ai::client::ClientConfig;
use pg_ai::provider::allowed_hosts;
use pg_ai::provider::Provider;
use pg_ai::settings::KeyManager;
use pg_content::{load_pack, ComponentRegistry, ContentSet, DirPack, Limits};
use pg_core::commands::Command;
use pg_core::input::SimInput;
use pg_core::pipeline::Pipeline;
use pg_core::world::WorldState;
use pg_host::{AllowListNet, FixedClock, MemLog, MemSecretStore, NetError, ScriptedNet};
use std::time::Duration;

const KEY: &str = "sk-SENTINEL-0123456789abcdef";

struct Rig {
    net: Arc<AllowListNet<ScriptedNet>>,
    service: DialogueService,
    settings: AiSettings,
    rules: ContentRules,
}

fn rig(cfg: DialogueConfig, enabled: bool) -> Rig {
    let net = Arc::new(AllowListNet::new(ScriptedNet::new(), &allowed_hosts()));
    let secrets = Arc::new(MemSecretStore::new());
    KeyManager::new(secrets.as_ref())
        .set_key(Provider::OpenAi, KEY)
        .unwrap();
    let client = Arc::new(AiClient::new(
        net.clone(),
        secrets,
        Arc::new(FixedClock::new()),
        Arc::new(MemLog::new()),
        ClientConfig::default(),
    ));
    Rig {
        net,
        service: DialogueService::new(client, Arc::new(WorkerPool::new(2)), cfg),
        settings: AiSettings {
            enabled,
            ..AiSettings::default()
        },
        rules: ContentRules {
            tone_preset: "standard".into(),
            graphic_filter: true,
        },
    }
}

fn content() -> Arc<ContentSet> {
    let dir = format!("{}/../../data/base", env!("CARGO_MANIFEST_DIR"));
    let pack = load_pack(&DirPack::new(dir), &Limits::default()).unwrap_or_else(|r| panic!("{r}"));
    Arc::new(ContentSet::build(vec![pack], ComponentRegistry::builtin()).unwrap())
}

fn town(seed: &str) -> Sim {
    let mut sim =
        Sim::new(WorldState::new("Voices", seed), Pipeline::new()).with_content(content());
    sim.submit(
        0,
        SimInput::Command {
            actor: None,
            cmd: Command::GenerateTown {
                w: 48,
                h: 36,
                water: 15,
                residents: 12,
                tone: "standard".into(),
            },
        },
    )
    .unwrap();
    sim
}

/// Steps until a conversation starts; returns that tick's events and the pair.
fn until_talk(sim: &mut Sim) -> (Vec<Event>, EntityId, EntityId, u64) {
    for _ in 0..30_000 {
        let r = sim.step().unwrap();
        if let Some(e) = r.events.iter().find(|e| e.kind == "conversation.started") {
            let get = |k: &str| {
                e.detail
                    .get(k)
                    .and_then(|c| c.as_str()?.parse::<EntityId>().ok())
                    .unwrap()
            };
            let (a, b, tick) = (get("a"), get("b"), e.tick);
            return (r.events, a, b, tick);
        }
    }
    panic!("nobody talked in two days");
}

fn reply(lines: &[&str]) -> String {
    let list = serde_json::to_string(lines).unwrap();
    serde_json::json!({"choices":[{"message":{"role":"assistant","content": list}}]}).to_string()
}

fn wait(service: &mut DialogueService, now: u64) -> Vec<String> {
    let mut notes = Vec::new();
    for _ in 0..400 {
        notes.extend(service.poll(now).notes);
        if service.in_flight() == 0 {
            return notes;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!("the request never finished");
}

fn turns_of(sim: &Sim, a: EntityId) -> usize {
    sim.world()
        .pawns
        .get(a)
        .unwrap()
        .talk
        .as_ref()
        .unwrap()
        .turns as usize
}

#[test]
fn nobody_listening_means_no_lines_and_no_requests() {
    let mut r = rig(DialogueConfig::default(), true);
    let mut sim = town("quiet");
    let (events, a, _b, _t) = until_talk(&mut sim);
    r.service.observe(&events, &sim, &r.settings, &r.rules);
    assert!(r.service.lines(a, 0).is_none());
    assert_eq!(r.service.stats().heard, 0);
    assert_eq!(r.net.inner().request_count(), 0);
}

#[test]
fn with_ai_off_a_heard_conversation_gets_fallback_lines_and_no_request() {
    let mut r = rig(DialogueConfig::default(), false);
    let mut sim = town("off");
    let (events, a, _b, tick) = until_talk(&mut sim);
    r.service.set_focus(Some(a));
    r.service.observe(&events, &sim, &r.settings, &r.rules);
    let turns = turns_of(&sim, a);
    let started = sim
        .world()
        .pawns
        .get(a)
        .unwrap()
        .talk
        .as_ref()
        .unwrap()
        .started;
    assert_eq!(started, tick);
    let lines = r.service.lines(a, started).expect("stored");
    assert_eq!(lines.len(), turns);
    assert!(lines
        .iter()
        .all(|l| l.source == Source::Fallback && !l.text.starts_with('[')));
    assert_eq!(r.net.inner().request_count(), 0);
    assert_eq!(r.service.stats().heard, 1);
}

#[test]
fn good_ai_lines_replace_the_turns_not_yet_spoken_and_late_ones_are_ignored() {
    let mut r = rig(DialogueConfig::default(), true);
    let mut sim = town("good");
    let (events, a, _b, tick) = until_talk(&mut sim);
    let turns = turns_of(&sim, a);
    let said: Vec<String> = (0..turns).map(|i| format!("Line number {i}.")).collect();
    let refs: Vec<&str> = said.iter().map(String::as_str).collect();
    r.net.inner().push_ok(200, &reply(&refs));
    r.service.set_focus(Some(a));
    r.service.observe(&events, &sim, &r.settings, &r.rules);
    assert_eq!(r.service.in_flight(), 1);
    wait(&mut r.service, tick);
    let lines = r.service.lines(a, tick).unwrap();
    assert!(lines.iter().all(|l| l.source == Source::Ai));
    assert_eq!(lines[0].text, "Line number 0.");
    assert_eq!(r.service.stats().used, 1);

    // The same reply arriving after two turns have been spoken only replaces the rest.
    let mut late = rig(DialogueConfig::default(), true);
    late.net.inner().push_ok(200, &reply(&refs));
    late.service.set_focus(Some(a));
    late.service
        .observe(&events, &sim, &late.settings, &late.rules);
    let turn_ticks = TICKS_PER_GAME_MINUTE;
    wait(&mut late.service, tick + turn_ticks);
    let mixed = late.service.lines(a, tick).unwrap();
    assert_eq!(
        mixed[0].source,
        Source::Fallback,
        "turn one was spoken already"
    );
    if turns > 1 {
        assert_eq!(mixed[1].source, Source::Ai);
    }

    // And one that arrives after the conversation is over is not used at all.
    let mut over = rig(DialogueConfig::default(), true);
    over.net.inner().push_ok(200, &reply(&refs));
    over.service.set_focus(Some(a));
    over.service
        .observe(&events, &sim, &over.settings, &over.rules);
    wait(&mut over.service, tick + turn_ticks * 10);
    assert_eq!(over.service.stats().late, 1);
    assert!(over
        .service
        .lines(a, tick)
        .unwrap()
        .iter()
        .all(|l| l.source == Source::Fallback));
}

#[test]
fn hostile_over_long_and_failing_replies_leave_the_fallback_lines() {
    let mut sim = town("hostile");
    let (events, a, _b, tick) = until_talk(&mut sim);
    let turns = turns_of(&sim, a);
    let bad: Vec<String> = vec!["Visit http://evil.example now".into(); turns];
    let bad_refs: Vec<&str> = bad.iter().map(String::as_str).collect();
    let long = vec!["x".repeat(500); turns];
    let long_refs: Vec<&str> = long.iter().map(String::as_str).collect();
    for (name, push) in [
        ("link", reply(&bad_refs)),
        ("too long", reply(&long_refs)),
        (
            "not a list",
            reply(&[]).replace("[]", "\"I am an assistant\""),
        ),
    ] {
        let mut r = rig(DialogueConfig::default(), true);
        r.net.inner().push_ok(200, &push);
        r.service.set_focus(Some(a));
        r.service.observe(&events, &sim, &r.settings, &r.rules);
        let notes = wait(&mut r.service, tick);
        let st = r.service.stats();
        assert_eq!(st.refused + st.failed, 1, "{name}: {notes:?}");
        if name != "not a list" {
            assert_eq!(st.refused, 1, "{name}: {notes:?}");
        }
        assert!(r
            .service
            .lines(a, tick)
            .unwrap()
            .iter()
            .all(|l| l.source == Source::Fallback));
    }
    let mut r = rig(DialogueConfig::default(), true);
    r.net.inner().push(Err(NetError::Offline));
    r.net.inner().push(Err(NetError::Offline));
    r.service.set_focus(Some(a));
    r.service.observe(&events, &sim, &r.settings, &r.rules);
    wait(&mut r.service, tick);
    assert_eq!(r.service.stats().failed, 1);
    assert!(r
        .service
        .lines(a, tick)
        .unwrap()
        .iter()
        .all(|l| l.source == Source::Fallback));
}

#[test]
fn cooldowns_and_the_in_flight_cap_hold_requests_back() {
    let mut r = rig(DialogueConfig::default(), true);
    let mut sim = town("cool");
    let (events, a, _b, tick) = until_talk(&mut sim);
    let turns = turns_of(&sim, a);
    let said: Vec<String> = (0..turns).map(|i| format!("Line {i}.")).collect();
    let refs: Vec<&str> = said.iter().map(String::as_str).collect();
    r.net.inner().push_ok(200, &reply(&refs));
    r.service.set_focus(Some(a));
    r.service.observe(&events, &sim, &r.settings, &r.rules);
    r.service.observe(&events, &sim, &r.settings, &r.rules);
    assert_eq!(r.service.stats().requested, 1);
    assert_eq!(r.service.stats().skipped_cooldown, 1);
    wait(&mut r.service, tick);

    let mut busy = rig(
        DialogueConfig {
            max_in_flight: 0,
            ..DialogueConfig::default()
        },
        true,
    );
    busy.service.set_focus(Some(a));
    busy.service
        .observe(&events, &sim, &busy.settings, &busy.rules);
    assert_eq!(busy.service.stats().skipped_busy, 1);
    assert_eq!(busy.net.inner().request_count(), 0);
}

/// The world with recorded lines taken out, so runs can be compared on everything but the words.
fn without_words(sim: &Sim) -> pg_core::hash::StateHash {
    let mut w = sim.world().clone();
    let ids: Vec<EntityId> = w.pawns.iter().map(|(id, _)| id).collect();
    for id in ids {
        if let Some(p) = w.pawns.get_mut(id) {
            if let Some(t) = p.talk.as_mut() {
                t.lines.clear();
            }
            for m in &mut p.memories {
                if let Some(t) = m.talk.as_mut() {
                    t.lines.clear();
                }
            }
        }
    }
    w.state_hash()
}

#[test]
fn outcomes_are_identical_with_ai_on_off_or_failing_and_only_the_words_differ() {
    let run = |mode: &str| {
        let enabled = mode != "off";
        let r = rig(DialogueConfig::default(), enabled);
        let net = r.net.clone();
        if mode == "failing" {
            for _ in 0..40 {
                net.inner().push(Err(NetError::Offline));
            }
        } else if mode == "good" {
            net.inner().set_handler(Box::new(|_| {
                Ok(pg_host::HttpResponse {
                    status: 200,
                    headers: Vec::new(),
                    body: reply(&[
                        "Hello.", "Hi.", "Fine.", "Good.", "Yes.", "No.", "Ok.", "Bye.",
                    ]),
                })
            }));
        }
        let hook = DialogueHook::new(r.service);
        *hook.settings.lock().unwrap() = (r.settings.clone(), true);
        hook.service
            .lock()
            .unwrap()
            .set_focus(Some(EntityId::new(pg_core::id::Kind::Pawn, 1)));
        let mut sim = town("same-outcomes");
        for _ in 0..28_800 {
            let report = sim.step().unwrap();
            hook.after_tick(&report.events, &mut sim, report.tick);
            // Let a request finish within its conversation, as it would at normal speed.
            while mode == "good" && hook.service.lock().unwrap().in_flight() > 0 {
                std::thread::sleep(Duration::from_millis(1));
                hook.after_tick(&[], &mut sim, report.tick);
            }
        }
        let heard = hook.service.lock().unwrap().stats().heard;
        let written = sim
            .world()
            .pawns
            .iter()
            .flat_map(|(_, p)| p.memories.iter())
            .filter(|m| m.talk.as_ref().is_some_and(|t| !t.lines.is_empty()))
            .count();
        (
            without_words(&sim),
            sim.world().state_hash(),
            heard,
            written,
        )
    };
    let (off, off_full, heard, written_off) = run("off");
    let (good, good_full, _, written_good) = run("good");
    let (failing, failing_full, _, written_failing) = run("failing");
    assert_eq!(good, off, "the words are the only difference");
    assert_eq!(failing, off);
    assert_eq!(failing_full, off_full, "a failing provider records nothing");
    assert!(heard > 0, "the focused resident overheard something");
    assert_eq!((written_off, written_failing), (0, 0));
    if written_good > 0 {
        assert_ne!(
            good_full, off_full,
            "recorded lines are part of the history"
        );
    }
}
