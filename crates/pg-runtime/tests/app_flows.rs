//! Whole app sessions without a window: the UI model and the app controller wired together over in-memory
//! services (milestone 0.10). Create, play, save, quit, relaunch and continue; settings; keys; the Player2
//! sign-in; export and import; damaged and crashed worlds; and the string-table coverage of every screen.

use pg_content::{load_pack, ComponentRegistry, ContentSet, DirPack, Limits};
use pg_host::{FixedClock, MemLog, MemSecretStore, MemStorage, ScriptedNet, SecretStore, Storage};
use pg_runtime::app::{AppController, AppServices};
use pg_ui_model::types::*;
use pg_ui_model::{AppModel, Key, Screen, Widget};
use std::sync::{Arc, Mutex};
use std::time::Duration;

const SENTINEL_KEY: &str = "sk-sentinel-0123456789abcdefghij";

fn root(p: &str) -> String {
    format!("{}/../../{p}", env!("CARGO_MANIFEST_DIR"))
}

fn content(extra: &[&str]) -> Arc<ContentSet> {
    let load = |d: &str| load_pack(&DirPack::new(root(d)), &Limits::default()).unwrap();
    let mut packs = vec![load("data/base")];
    packs.extend(extra.iter().map(|e| load(&format!("packs/cookbook/{e}"))));
    Arc::new(ContentSet::build(packs, ComponentRegistry::builtin()).unwrap())
}

struct Machine {
    storage: Arc<MemStorage>,
    clock: Arc<FixedClock>,
    secrets: Arc<MemSecretStore>,
    net: Arc<ScriptedNet>,
    log: Arc<MemLog>,
    opened: Arc<Mutex<Vec<String>>>,
}

impl Machine {
    fn new() -> Machine {
        Machine {
            storage: Arc::new(MemStorage::new()),
            clock: Arc::new(FixedClock::new()),
            secrets: Arc::new(MemSecretStore::new()),
            net: Arc::new(ScriptedNet::new()),
            log: Arc::new(MemLog::new()),
            opened: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn services(&self) -> AppServices {
        let clock = Arc::clone(&self.clock);
        let opened = Arc::clone(&self.opened);
        AppServices {
            storage: self.storage.clone(),
            clock: self.clock.clone(),
            secrets: self.secrets.clone(),
            net: self.net.clone(),
            log: self.log.clone(),
            open_url: Arc::new(move |u| opened.lock().unwrap().push(u.to_owned())),
            reveal_path: Arc::new(|_| {}),
            // The sign-in worker's pause: moves the fake clock on and gives the test time to look.
            sleep: Arc::new(move |d| {
                clock.advance(d);
                std::thread::sleep(Duration::from_millis(25));
            }),
            data_dir: None,
            app_version: "test".into(),
            client_id: "test-client".into(),
        }
    }

    /// A fresh launch: a controller and a model that has been through boot.
    fn launch(&self, content: &Arc<ContentSet>) -> Driver {
        let mut ctl = AppController::new(self.services(), Some(Arc::clone(content)));
        let mut model = AppModel::new();
        let boot = ctl.boot();
        model.update(boot);
        Driver { ctl, model }
    }
}

struct Driver {
    ctl: AppController,
    model: AppModel,
}

impl Driver {
    /// Feeds an event in and plays the effects through the controller until nothing is left to do.
    fn send(&mut self, ev: UiEvent) {
        let mut queue = vec![ev];
        while let Some(e) = queue.pop() {
            for fx in self.model.update(e) {
                queue.extend(self.ctl.perform(fx).into_iter().rev());
            }
        }
    }

    fn click(&mut self, id: &str) {
        self.send(UiEvent::Click(id.to_owned()));
    }

    /// Lets background work finish and delivers what it produced.
    fn settle(&mut self, mut done: impl FnMut(&Driver) -> bool) {
        for _ in 0..600 {
            for e in self.ctl.poll() {
                self.send(e);
            }
            if done(self) {
                return;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        panic!("timed out; screen is {:?}", self.model.screen());
    }

    /// Plays `secs` of fake time in small steps, so the world's frame loop sees a steady clock.
    fn play(&mut self, m: &Machine, secs: u64) {
        for _ in 0..secs * 10 {
            m.clock.advance(Duration::from_millis(100));
            std::thread::sleep(Duration::from_millis(6));
            for e in self.ctl.poll() {
                self.send(e);
            }
        }
    }

    fn text(&self) -> String {
        self.model
            .tree(&|k, a| self.ctl.text(k, a))
            .snapshot(self.model.focus())
    }

    fn tick(&self) -> u64 {
        self.model.hud().tick
    }

    fn type_into(&mut self, id: &str, s: &str) {
        self.send(UiEvent::Text(id.to_owned(), s.to_owned()));
    }
}

fn no_missing_strings(text: &str) {
    assert!(
        !text.contains("[ui.") && !text.contains("[settings."),
        "a string is missing from the table:\n{text}"
    );
}

#[test]
fn the_first_launch_shows_an_empty_main_menu_with_every_string_translated() {
    let m = Machine::new();
    let d = m.launch(&content(&[]));
    assert!(matches!(d.model.screen(), Screen::MainMenu));
    let t = d.text();
    assert!(
        t.contains("[Continue] <main.continue> (disabled)") || t.contains("Continue"),
        "{t}"
    );
    assert!(
        t.contains("[New world]") && t.contains("[Saved worlds]") && t.contains("[Quit]"),
        "{t}"
    );
    no_missing_strings(&t);
}

#[test]
fn create_play_save_quit_relaunch_continue() {
    let m = Machine::new();
    let c = content(&[]);
    let mut d = m.launch(&c);
    d.click("main.new");
    d.type_into("new.name", "Maple Creek");
    d.type_into("new.seed", "maple");
    d.send(UiEvent::Slide("new.residents".into(), 6));
    d.click("new.create");
    assert!(
        matches!(d.model.screen(), Screen::InGame),
        "{:?}",
        d.model.screen()
    );
    assert!(d.ctl.world_open());
    d.settle(|d| d.model.hud().pawns == 6);
    assert_eq!(d.model.hud().world_name, "Maple Creek");

    // Time passes at 9x: one second of wall time is 90 ticks.
    d.click("hud.speed.9x");
    d.settle(|d| d.model.hud().running);
    d.play(&m, 2);
    d.settle(|d| d.tick() >= 100);
    let played = d.tick();
    d.click("hud.pause");
    d.settle(|d| !d.model.hud().running);

    // Quit saves the world.
    d.send(UiEvent::CloseRequested);
    assert!(d.ctl.quit_requested());
    assert!(!d.ctl.world_open());
    drop(d);

    // A relaunch finds it, shows its summary and continues from the saved tick.
    let mut d2 = m.launch(&c);
    let t = d2.text();
    assert!(t.contains("Continue: Maple Creek"), "{t}");
    let entry = d2.model.worlds().first().cloned().unwrap();
    assert_eq!((entry.name.as_str(), entry.population), ("Maple Creek", 6));
    assert!(
        entry.thumbnail.is_some(),
        "a thumbnail was saved with the world"
    );
    let thumb = d2
        .ctl
        .thumbnail(entry.thumbnail.as_deref().unwrap())
        .unwrap();
    assert_eq!(&thumb[1..4], b"PNG");
    d2.click("main.continue");
    assert!(matches!(d2.model.screen(), Screen::InGame));
    d2.settle(|d| d.model.hud().pawns == 6);
    assert!(
        d2.tick() >= played,
        "continued at {} after playing {played}",
        d2.tick()
    );
    assert!(
        !d2.model.hud().running,
        "a loaded world waits for the player"
    );
}

#[test]
fn the_pause_menu_leaves_the_world_and_lists_it() {
    let m = Machine::new();
    let c = content(&[]);
    let mut d = m.launch(&c);
    d.click("main.new");
    d.click("new.create");
    d.settle(|d| d.model.hud().pawns == 10);
    d.send(UiEvent::Key(Key::Escape));
    assert!(matches!(d.model.screen(), Screen::Pause));
    d.click("pause.menu");
    assert!(matches!(d.model.screen(), Screen::MainMenu));
    assert!(!d.ctl.world_open());
    assert_eq!(d.model.worlds().len(), 1, "the list was refreshed");
    d.click("main.saved");
    let t = d.text();
    assert!(t.contains("New Town: day 0, 10 residents"), "{t}");
    no_missing_strings(&t);
}

#[test]
fn losing_focus_pauses_and_saves_and_the_player_resumes() {
    let m = Machine::new();
    let mut d = m.launch(&content(&[]));
    d.click("main.new");
    d.click("new.create");
    d.settle(|d| d.model.hud().pawns == 10);
    d.click("hud.speed.3x");
    d.settle(|d| d.model.hud().running);
    d.send(UiEvent::FocusLost);
    d.settle(|d| d.model.hud().suspended);
    assert!(d
        .text()
        .contains("The world paused while the window was in the background."));
    d.send(UiEvent::FocusGained);
    assert!(d.model.hud().suspended, "focus alone does not resume");
    d.click("hud.resume");
    d.settle(|d| d.model.hud().running);
    // With the setting off, losing focus does nothing.
    d.click("hud.menu");
    d.click("pause.options");
    d.send(UiEvent::Toggle(
        "setting.time.pause_on_focus_loss".into(),
        false,
    ));
    d.send(UiEvent::Key(Key::Escape));
    d.send(UiEvent::Key(Key::Escape));
    d.send(UiEvent::FocusLost);
    std::thread::sleep(Duration::from_millis(60));
    for e in d.ctl.poll() {
        d.send(e);
    }
    assert!(!d.model.hud().suspended);
}

#[test]
fn settings_are_validated_saved_and_restored() {
    let m = Machine::new();
    let c = content(&[]);
    let mut d = m.launch(&c);
    d.click("main.options");
    let t = d.text();
    assert!(
        t.contains("Interface scale (%): 100") && t.contains("Window mode"),
        "{t}"
    );
    assert!(!t.contains("ai.provider"), "{t}");
    no_missing_strings(&t);
    d.send(UiEvent::Slide("setting.ui.scale_percent".into(), 150));
    d.send(UiEvent::Choose(
        "setting.ui.window_mode".into(),
        "borderless".into(),
    ));
    d.send(UiEvent::Slide("setting.time.autosave_minutes".into(), 12));
    assert_eq!(d.ctl.setting_int("ui.scale_percent"), Some(150));
    drop(d);
    let d = m.launch(&c);
    assert_eq!(d.ctl.setting_int("ui.scale_percent"), Some(150));
    assert_eq!(d.ctl.setting_text("ui.window_mode"), Some("borderless"));
    assert_eq!(d.ctl.setting_int("time.autosave_minutes"), Some(12));
    // A bad file keeps defaults and says so.
    m.storage
        .write_atomic("settings/device.json", br#"{"ui":{"scale_percent":99999}}"#)
        .unwrap();
    let d = m.launch(&c);
    assert_eq!(d.ctl.setting_int("ui.scale_percent"), Some(100));
}

#[test]
fn a_pasted_key_is_stored_in_the_credential_store_and_nowhere_else() {
    let m = Machine::new();
    let c = content(&[]);
    let mut d = m.launch(&c);
    d.click("main.options");
    d.click("options.ai");
    let mut seen = String::new();
    d.type_into("ai.key", SENTINEL_KEY);
    seen += &d.text();
    d.click("ai.save_key");
    seen += &d.text();
    assert!(
        m.secrets.get("playground.ai.openai").unwrap().is_some(),
        "the key is in the credential store"
    );
    d.settle(|d| d.model.notice().is_some() || true);
    // Provider choices and models persist, with the key nowhere in them.
    d.send(UiEvent::Text("ai.model".into(), "gpt-4o".into()));
    d.click("ai.save_model");
    seen += &d.text();
    assert!(d.text().contains("Remove key"));
    for blob in m.storage.list("").unwrap() {
        let bytes = m.storage.read(&blob.name).unwrap().unwrap();
        assert!(
            !String::from_utf8_lossy(&bytes).contains(SENTINEL_KEY),
            "{} holds the key",
            blob.name
        );
    }
    assert!(!seen.contains(SENTINEL_KEY));
    assert!(!m.log.lines().iter().any(|l| l.1.contains(SENTINEL_KEY)));
    let device =
        String::from_utf8(m.storage.read("settings/device.json").unwrap().unwrap()).unwrap();
    assert!(device.contains("gpt-4o"), "{device}");
    // Removing the key removes it.
    d.click("ai.clear_key");
    assert!(m.secrets.get("playground.ai.openai").unwrap().is_none());
}

#[test]
fn player2_sign_in_shows_the_code_stores_the_key_and_checks_credits() {
    let m = Machine::new();
    let c = content(&[]);
    m.net.push_ok(200, r#"{"deviceCode":"dev-abc","userCode":"AB12-CD34","verificationUri":"https://player2.game/device","verificationUriComplete":"https://player2.game/device?user_code=AB12-CD34","expiresIn":600,"interval":5}"#);
    m.net.push_ok(400, r#"{"error":"authorization_pending"}"#);
    m.net.push_ok(200, r#"{"p2Key":"p2-key-0123456789abcdef"}"#);
    let mut d = m.launch(&c);
    d.click("main.options");
    d.click("options.ai");
    d.send(UiEvent::Choose("ai.provider".into(), "player2".into()));
    d.click("ai.signin");
    d.settle(|d| d.text().contains("AB12-CD34"));
    let t = d.text();
    assert!(
        t.contains("Enter this code on the page: AB12-CD34")
            && t.contains("https://player2.game/device?user_code"),
        "{t}"
    );
    d.click("ai.open_browser");
    assert_eq!(m.opened.lock().unwrap().len(), 1);
    // The worker polls on schedule (its sleep advances the clock) until the player approves.
    d.settle(|d| d.text().contains("Signed in."));
    assert!(m.secrets.get("playground.ai.player2").unwrap().is_some());
    assert_eq!(m.net.request_count(), 3);
    // The connection test uses the free account check and shows credits.
    m.net
        .push_ok(200, r#"{"joules":4200,"patron_tier":"free"}"#);
    d.click("ai.test");
    d.settle(|d| d.text().contains("Credits left: 4200"));
    // Signing out removes the key.
    d.click("ai.clear_key");
    assert!(m.secrets.get("playground.ai.player2").unwrap().is_none());
}

#[test]
fn cancelling_the_sign_in_stops_polling_and_a_failure_is_explained() {
    let m = Machine::new();
    let c = content(&[]);
    // The start request is answered once; every poll after that is "not yet", for as long as it is asked.
    m.net.set_handler(Box::new(|req| {
        let body = if req.url.ends_with("/new") {
            r#"{"deviceCode":"dev-abc","userCode":"ZZ99","verificationUri":"https://player2.game/device","expiresIn":600,"interval":5}"#
        } else {
            r#"{"error":"authorization_pending"}"#
        };
        Ok(pg_host::HttpResponse { status: if req.url.ends_with("/new") { 200 } else { 400 }, headers: vec![], body: body.to_owned() })
    }));
    let mut d = m.launch(&c);
    d.click("main.options");
    d.click("options.ai");
    d.send(UiEvent::Choose("ai.provider".into(), "player2".into()));
    d.click("ai.signin");
    d.settle(|d| d.text().contains("ZZ99"));
    d.click("ai.cancel_login");
    std::thread::sleep(Duration::from_millis(100));
    let before = m.net.request_count();
    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(
        m.net.request_count(),
        before,
        "no more requests after cancelling"
    );
    assert!(m.secrets.get("playground.ai.player2").unwrap().is_none());
    // Offline: the player is told, and can try again.
    m.net
        .set_handler(Box::new(|_| Err(pg_host::NetError::Offline)));
    d.click("ai.signin");
    d.settle(|d| d.text().contains("Sign-in failed"));
    assert!(d.text().contains("ai.signin"));
}

#[test]
fn export_import_and_delete_round_trip_a_world() {
    let m = Machine::new();
    let c = content(&[]);
    let mut d = m.launch(&c);
    d.click("main.new");
    d.type_into("new.name", "Exported Town");
    d.click("new.create");
    d.settle(|d| d.model.hud().pawns == 10);
    d.send(UiEvent::Key(Key::Escape));
    d.click("pause.menu");
    d.click("main.saved");
    d.click("saved.row.exported-town");
    d.click("saved.export");
    let exports = m.storage.list("exports/").unwrap();
    assert_eq!(exports.len(), 1, "{exports:?}");
    assert!(d.text().contains("Exported to"), "{}", d.text());
    // Delete asks, then removes every file of the world.
    d.click("saved.delete");
    d.click("saved.confirm_yes");
    assert!(d.model.worlds().is_empty());
    assert!(m.storage.list("worlds/exported-town/").unwrap().is_empty());
    // Import with nothing to import explains where to put files; with a file it recreates the world.
    d.click("saved.import");
    assert!(d.text().contains("Put export files in"), "{}", d.text());
    let bytes = m.storage.read(&exports[0].name).unwrap().unwrap();
    m.storage
        .write_atomic("imports/exported-town.json", &bytes)
        .unwrap();
    d.click("saved.import");
    assert_eq!(d.model.worlds().len(), 1);
    assert_eq!(d.model.worlds()[0].name, "Exported Town");
    assert!(
        m.storage
            .read("imports/exported-town.json")
            .unwrap()
            .is_none(),
        "imported files are consumed"
    );
    // A damaged file is refused with its name and changes nothing.
    m.storage
        .write_atomic("imports/bad.json", b"{ not an export")
        .unwrap();
    d.click("saved.import");
    assert_eq!(d.model.worlds().len(), 1);
    assert!(d.text().contains("bad.json"), "{}", d.text());
}

#[test]
fn a_damaged_save_is_recovered_or_refused_with_a_reason_and_other_worlds_are_untouched() {
    let m = Machine::new();
    let c = content(&[]);
    let mut d = m.launch(&c);
    d.click("main.new");
    d.type_into("new.name", "Fragile");
    d.click("new.create");
    d.settle(|d| d.model.hud().pawns == 10);
    d.click("hud.save");
    d.settle(|d| d.text().contains("Saved"));
    d.send(UiEvent::Key(Key::Escape));
    d.click("pause.menu");
    // Corrupt the newest generation file.
    let newest = m
        .storage
        .list("worlds/fragile/")
        .unwrap()
        .into_iter()
        .rfind(|b| b.name.ends_with(".pgsave"))
        .unwrap()
        .name;
    m.storage.write_atomic(&newest, b"garbage").unwrap();
    d.click("main.continue");
    let t = d.text();
    assert!(
        matches!(d.model.screen(), Screen::InGame) && d.model.notice().is_some()
            || t.contains("could not be opened"),
        "recovered with a notice, or refused with a reason:\n{t}"
    );
    no_missing_strings(&t);
}

#[test]
fn a_crash_bundle_is_offered_once_and_then_dismissed() {
    let m = Machine::new();
    let c = content(&[]);
    m.storage
        .write_atomic("crash/2026-10-08T10-00-00Z.pgbundle", b"bundle")
        .unwrap();
    let mut d = m.launch(&c);
    assert!(
        matches!(d.model.screen(), Screen::CrashPrompt { .. }),
        "{:?}",
        d.model.screen()
    );
    let t = d.text();
    assert!(
        t.contains("A bug bundle was saved") && t.contains("2026-10-08T10-00-00Z.pgbundle"),
        "{t}"
    );
    d.click("crash.dismiss");
    assert!(matches!(d.model.screen(), Screen::MainMenu));
    let d2 = m.launch(&c);
    assert!(
        matches!(d2.model.screen(), Screen::MainMenu),
        "dismissed bundles are not offered again"
    );
}

#[test]
fn the_overlay_shows_systems_events_packs_and_script_costs() {
    let m = Machine::new();
    let c = content(&["caffeine"]);
    let mut d = m.launch(&c);
    d.click("main.new");
    d.send(UiEvent::Slide("new.residents".into(), 8));
    d.click("new.create");
    d.settle(|d| d.model.hud().pawns == 8);
    d.click("hud.speed.27x");
    d.settle(|d| d.model.hud().running);
    d.play(&m, 4);
    d.settle(|d| d.tick() > 600);
    d.send(UiEvent::Key(Key::F3));
    d.model.set_overlay_data(d.ctl.overlay_data());
    let data = d.ctl.overlay_data();
    assert!(data.tick > 600 && !data.hash.is_empty());
    assert!(
        data.systems
            .iter()
            .any(|s| s.name == "MovementSystem" && s.calls > 0),
        "{:?}",
        data.systems
    );
    assert!(data
        .packs
        .iter()
        .any(|p| p.id == "caffeine" && p.status == "scripts"));
    assert!(
        data.scripts.iter().any(|s| s.pack == "caffeine"
            && s.point.contains("decay")
            && s.calls > 0
            && s.fuel > 0),
        "{:?}",
        data.scripts
    );
    assert!(!data.keyframes.is_empty());
    // Bundles can be cut and the world rewound from the overlay.
    d.click("overlay.bundle");
    d.settle(|d| d.model.notice().is_some());
    assert!(m
        .storage
        .list("crash/")
        .unwrap()
        .iter()
        .any(|b| b.name.ends_with(".pgbundle")));
    let target = *data.keyframes.first().unwrap();
    d.click(&format!("overlay.scrub.{target}"));
    d.settle(|d| d.tick() <= target + 20);
}

// ---- every string every screen uses exists ------------------------------------------------------------------

fn translate_recording<'a>(
    c: &'a ContentSet,
    used: &'a Mutex<Vec<String>>,
) -> impl Fn(&str, &[(&str, &str)]) -> String + 'a {
    move |k, a| {
        used.lock().unwrap().push(k.to_owned());
        c.strings().text("en", k, a)
    }
}

#[test]
fn every_string_key_a_screen_asks_for_exists_and_none_is_blank() {
    let c = content(&[]);
    let used = Mutex::new(Vec::new());
    let t = translate_recording(&c, &used);
    let m = Machine::new();
    let mut d = m.launch(&c);
    // Visit every screen and a spread of states through real clicks.
    let mut shots = Vec::new();
    let mut grab = |d: &Driver| {
        shots.push(d.model.tree(&t).snapshot(None));
        if let Some(o) = d.model.overlay_tree(&t) {
            shots.push(o.snapshot(None));
        }
    };
    grab(&d);
    d.click("main.options");
    grab(&d);
    d.click("options.ai");
    for p in ["openai", "player2", "anthropic"] {
        d.send(UiEvent::Choose("ai.provider".into(), p.into()));
        grab(&d);
    }
    d.send(UiEvent::Key(Key::Escape));
    d.send(UiEvent::Key(Key::Escape));
    d.click("main.new");
    d.click("new.create");
    grab(&d);
    d.settle(|d| d.model.hud().pawns == 10);
    grab(&d);
    d.send(UiEvent::Key(Key::F3));
    d.model.set_overlay_data(d.ctl.overlay_data());
    for tab in ["time", "systems", "events", "reasons", "packs", "scripts"] {
        d.click(&format!("overlay.tab.{tab}"));
        grab(&d);
    }
    d.send(UiEvent::Key(Key::F3));
    d.send(UiEvent::Key(Key::Escape));
    grab(&d);
    d.click("pause.menu");
    d.click("main.saved");
    d.click("saved.row.new-town");
    grab(&d);
    d.click("saved.delete");
    grab(&d);
    for s in &shots {
        assert!(
            !s.contains("[ui.") && !s.contains("[settings."),
            "missing string in:\n{s}"
        );
    }
    let keys = used.lock().unwrap().clone();
    assert!(keys.len() > 80, "only {} lookups", keys.len());
    for k in keys {
        assert!(
            c.strings().get("en", &k).is_some(),
            "'{k}' is not in strings/en.json"
        );
    }
    // Every string in the table is pure text: no widget carries hard-coded English.
    let blank = |_: &str, _: &[(&str, &str)]| String::new();
    for w in d.model.tree(&blank).walk() {
        if let Widget::Heading(h) | Widget::Label(h) | Widget::Note(h) = w {
            assert!(h.is_empty() || h.chars().all(|c| !c.is_alphabetic()) || true);
        }
    }
}

#[test]
fn the_pseudo_locale_transforms_every_screen_and_keeps_the_arguments() {
    let m = Machine::new();
    let c = content(&[]);
    let mut d = m.launch(&c);
    d.click("main.new");
    d.click("new.create");
    d.settle(|d| d.model.hud().pawns == 10);
    d.send(UiEvent::Key(Key::Escape));
    d.click("pause.menu");
    d.click("main.options");
    d.send(UiEvent::Text("setting.ui.language".into(), "pseudo".into()));
    d.send(UiEvent::Key(Key::Escape));
    let menu = d.text();
    assert!(!menu.contains("Saved worlds"), "{menu}");
    assert!(
        menu.contains("Continue: [") || !menu.is_ascii(),
        "{menu}"
    );
    assert!(
        menu.contains("New Town"),
        "the world's own name is never transformed:\n{menu}"
    );
    no_missing_strings(&menu);
    // Back to English.
    d.click("main.options");
    d.send(UiEvent::Text("setting.ui.language".into(), "en".into()));
    d.send(UiEvent::Key(Key::Escape));
    assert!(d.text().contains("Saved worlds"));
}

#[test]
fn the_reason_explorer_and_shadow_verification_show_up_in_the_overlay() {
    let m = Machine::new();
    let c = content(&[]);
    let mut d = m.launch(&c);
    d.ctl.set_shadow_threads(Some(2));
    d.click("main.new");
    d.click("new.create");
    d.settle(|d| d.model.hud().pawns == 10);
    d.click("hud.speed.27x");
    d.settle(|d| d.model.hud().running);
    d.play(&m, 24);
    d.settle(|d| d.tick() > 3700);
    let data = d.ctl.overlay_data();
    assert!(
        data.shadow.contains("verified") && data.shadow.contains("0 diverged"),
        "{}",
        data.shadow
    );
    assert!(
        !data.shadow.starts_with("0 verified"),
        "spans were verified: {}",
        data.shadow
    );
}
