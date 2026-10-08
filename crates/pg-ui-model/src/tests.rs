//! Screen-flow, keyboard-reachability and snapshot tests (suggestion S-027). No window is involved: events
//! go into the model and effects come out.

use crate::app::{AppModel, LoginUi, Screen};
use crate::types::*;
use crate::widget::Widget;
use pg_host::Secret;

/// A translator that shows keys and arguments, so snapshots read the same in every language.
fn show(key: &str, args: &[(&str, &str)]) -> String {
    if args.is_empty() {
        key.to_owned()
    } else {
        let a: Vec<String> = args.iter().map(|(k, v)| format!("{k}={v}")).collect();
        format!("{key}{{{}}}", a.join(","))
    }
}

fn world(id: &str, name: &str, day: u64, damaged: bool) -> WorldEntry {
    WorldEntry {
        id: id.into(),
        name: name.into(),
        day,
        population: 12,
        play_ticks: day * 14_400,
        saved: "2026-10-08 12:00".into(),
        packs: vec!["base".into()],
        thumbnail: None,
        damaged,
    }
}

fn providers() -> AiState {
    let p = |id: &str, auth, has_key, fixed| ProviderInfo {
        id: id.into(),
        label: id.to_uppercase(),
        auth,
        has_key,
        model_fixed: fixed,
        model: if fixed { "chosen".into() } else { "m1".into() },
        recommended_model: "rec".into(),
    };
    AiState {
        providers: vec![
            p("openai", Auth::PasteKey, false, false),
            p("anthropic", Auth::PasteKey, true, false),
            p("player2", Auth::DeviceCode, false, true),
        ],
        selected: "openai".into(),
        persistent_keys: true,
    }
}

fn settings() -> Vec<SettingItem> {
    vec![
        SettingItem {
            id: "ui.scale_percent".into(),
            label_key: "setting.ui.scale_percent".into(),
            kind: SettingKind::Int { min: 50, max: 300 },
            value: SettingValue::Int(100),
            restart_required: false,
        },
        SettingItem {
            id: "ui.window_mode".into(),
            label_key: "setting.ui.window_mode".into(),
            kind: SettingKind::Enum(vec![
                ("windowed".into(), "ui.window.windowed".into()),
                ("fullscreen".into(), "ui.window.fullscreen".into()),
            ]),
            value: SettingValue::Text("windowed".into()),
            restart_required: false,
        },
        SettingItem {
            id: "time.pause_on_focus_loss".into(),
            label_key: "setting.time.pause_on_focus_loss".into(),
            kind: SettingKind::Bool,
            value: SettingValue::Bool(true),
            restart_required: false,
        },
        SettingItem {
            id: "ui.vsync".into(),
            label_key: "setting.ui.vsync".into(),
            kind: SettingKind::Bool,
            value: SettingValue::Bool(true),
            restart_required: true,
        },
        SettingItem {
            id: "dev.enabled".into(),
            label_key: "setting.dev.enabled".into(),
            kind: SettingKind::Bool,
            value: SettingValue::Bool(false),
            restart_required: false,
        },
        // AI settings never appear in the generated screen: they have their own.
        SettingItem {
            id: "ai.provider".into(),
            label_key: "setting.ai.provider".into(),
            kind: SettingKind::Text { max_len: 16 },
            value: SettingValue::Text("openai".into()),
            restart_required: false,
        },
    ]
}

fn boot(worlds: Vec<WorldEntry>, crash: Option<&str>) -> AppModel {
    let mut m = AppModel::new();
    assert!(matches!(m.screen(), Screen::Boot));
    m.update(UiEvent::Booted(Box::new(BootInfo {
        worlds,
        settings: settings(),
        ai: providers(),
        crash_bundle: crash.map(str::to_owned),
        settings_warnings: Vec::new(),
    })));
    m
}

fn press(m: &mut AppModel, k: Key) -> Vec<AppEffect> {
    m.update(UiEvent::Key(k))
}

fn click(m: &mut AppModel, id: &str) -> Vec<AppEffect> {
    m.update(UiEvent::Click(id.into()))
}

fn snap(m: &AppModel) -> String {
    m.tree(&show).snapshot(m.focus())
}

/// Turns developer mode on or off through the pause menu and Options, as a player would, and comes back.
fn dev_mode(m: &mut AppModel, on: bool) {
    press(m, Key::Escape);
    click(m, "pause.options");
    m.update(UiEvent::Toggle("setting.dev.enabled".into(), on));
    press(m, Key::Escape);
    press(m, Key::Escape);
    assert!(matches!(m.screen(), Screen::InGame) && m.dev_mode() == on);
}

fn in_world() -> AppModel {
    let mut m = boot(vec![world("w1", "Town", 3, false)], None);
    click(&mut m, "main.continue");
    m.update(UiEvent::WorldOpened {
        name: "Town".into(),
    });
    assert!(m.in_world() && matches!(m.screen(), Screen::InGame));
    m
}

#[test]
fn booting_leads_to_the_main_menu_with_continue_pointing_at_the_newest_world() {
    let m = boot(
        vec![
            world("w2", "Newer", 9, false),
            world("w1", "Older", 2, false),
        ],
        None,
    );
    assert!(matches!(m.screen(), Screen::MainMenu));
    let s = snap(&m);
    assert!(
        s.contains("ui.main.continue_named{name=Newer,day=9}"),
        "{s}"
    );
    assert_eq!(
        m.focus(),
        Some("main.continue"),
        "focus starts on the first action"
    );
    // With nothing saved, Continue is disabled and focus starts on New world.
    let m = boot(Vec::new(), None);
    assert!(snap(&m).contains("(disabled)"));
    assert_eq!(m.focus(), Some("main.new"));
    // A damaged newest world is skipped for Continue.
    let m = boot(
        vec![
            world("w2", "Broken", 9, true),
            world("w1", "Fine", 2, false),
        ],
        None,
    );
    assert!(snap(&m).contains("name=Fine"));
}

#[test]
fn the_main_menu_snapshot_is_stable() {
    let m = boot(vec![world("w1", "Town", 3, false)], None);
    let expected = "\
[ui.main.title]
  # ui.main.title
  (ui.main.subtitle)
  >[ui.main.continue_named{name=Town,day=3}] <main.continue>
   [ui.main.new] <main.new>
   [ui.main.saved] <main.saved>
   [ui.main.options] <main.options>
   [ui.main.quit] <main.quit>
";
    assert_eq!(snap(&m), expected);
}

#[test]
fn keyboard_only_new_world_flow_creates_a_world() {
    let mut m = boot(Vec::new(), None);
    assert!(press(&mut m, Key::Enter).is_empty());
    assert!(matches!(m.screen(), Screen::NewWorld(_)));
    // Name field, seed field, size, residents, water, tone, create, back.
    assert_eq!(m.focus(), Some("new.name"));
    m.update(UiEvent::Text("new.name".into(), "  Maple Creek ".into()));
    m.update(UiEvent::Text("new.seed".into(), "abc".into()));
    press(&mut m, Key::Tab);
    press(&mut m, Key::Tab);
    assert_eq!(m.focus(), Some("new.size"));
    press(&mut m, Key::Right); // small -> medium
    press(&mut m, Key::Tab);
    assert_eq!(m.focus(), Some("new.residents"));
    for _ in 0..5 {
        press(&mut m, Key::Right);
    }
    press(&mut m, Key::Tab);
    assert_eq!(m.focus(), Some("new.water"));
    press(&mut m, Key::Tab);
    assert_eq!(m.focus(), Some("new.tone"));
    press(&mut m, Key::Tab);
    assert_eq!(m.focus(), Some("new.create"));
    let fx = press(&mut m, Key::Enter);
    assert_eq!(
        fx,
        vec![AppEffect::CreateWorld {
            name: "Maple Creek".into(),
            seed: "abc".into(),
            size: MapSize::Medium,
            residents: 19,
            water: 18,
            tone: "standard".into(),
        }]
    );
    m.update(UiEvent::WorldOpened {
        name: "Maple Creek".into(),
    });
    assert!(matches!(m.screen(), Screen::InGame));
}

#[test]
fn new_world_validates_names_and_seeds_and_escape_goes_back() {
    let mut m = boot(Vec::new(), None);
    click(&mut m, "main.new");
    m.update(UiEvent::Text("new.name".into(), "   ".into()));
    assert!(click(&mut m, "new.create").is_empty());
    assert!(snap(&m).contains("ui.new.error.name"));
    m.update(UiEvent::Text("new.name".into(), "bad\nname".into()));
    assert!(click(&mut m, "new.create").is_empty());
    m.update(UiEvent::Text("new.name".into(), "Ok".into()));
    m.update(UiEvent::Text("new.seed".into(), "x".repeat(70)));
    assert!(click(&mut m, "new.create").is_empty());
    assert!(snap(&m).contains("ui.new.error.seed"));
    press(&mut m, Key::Escape);
    assert!(matches!(m.screen(), Screen::MainMenu));
}

#[test]
fn saved_worlds_select_load_export_delete_import() {
    let mut m = boot(
        vec![world("a", "Alpha", 1, false), world("b", "Beta", 4, true)],
        None,
    );
    let fx = click(&mut m, "main.saved");
    assert_eq!(fx, vec![AppEffect::ListWorlds]);
    let s = snap(&m);
    assert!(
        s.contains("ui.saved.damaged_suffix"),
        "damaged slots are marked: {s}"
    );
    assert!(
        s.contains("(disabled)"),
        "no actions until a world is selected"
    );
    assert!(click(&mut m, "saved.load").is_empty());
    click(&mut m, "saved.row.a");
    assert!(snap(&m).contains("ui.saved.details"));
    assert_eq!(
        click(&mut m, "saved.load"),
        vec![AppEffect::LoadWorld("a".into())]
    );
    assert_eq!(
        click(&mut m, "saved.export"),
        vec![AppEffect::ExportWorld("a".into())]
    );
    // Delete asks first, and Escape cancels the question instead of leaving the screen.
    assert!(click(&mut m, "saved.delete").is_empty());
    assert!(snap(&m).contains("ui.saved.confirm_delete{name=Alpha}"));
    press(&mut m, Key::Escape);
    assert!(matches!(m.screen(), Screen::SavedWorlds(_)));
    assert!(!snap(&m).contains("confirm_delete"));
    click(&mut m, "saved.delete");
    assert_eq!(
        click(&mut m, "saved.confirm_yes"),
        vec![AppEffect::DeleteWorld("a".into())]
    );
    m.update(UiEvent::WorldsListed(vec![world("b", "Beta", 4, true)]));
    assert!(!snap(&m).contains("Alpha"));
    assert_eq!(click(&mut m, "saved.import"), vec![AppEffect::ImportWorld]);
    assert_eq!(
        m.update(UiEvent::Imported("Gamma".into())),
        vec![AppEffect::ListWorlds]
    );
    assert!(matches!(
        m.notice(),
        Some(crate::app::Notice::Key("ui.notice.imported", _))
    ));
    click(&mut m, "saved.back");
    assert!(matches!(m.screen(), Screen::MainMenu));
}

#[test]
fn an_empty_saved_worlds_list_says_so_and_still_has_a_way_out() {
    let mut m = boot(Vec::new(), None);
    click(&mut m, "main.saved");
    assert!(snap(&m).contains("ui.saved.empty"));
    assert_eq!(m.focus(), Some("saved.import"));
    press(&mut m, Key::Escape);
    assert!(matches!(m.screen(), Screen::MainMenu));
}

#[test]
fn the_options_screen_is_generated_from_the_settings() {
    let mut m = boot(Vec::new(), None);
    click(&mut m, "main.options");
    let s = snap(&m);
    assert!(s.contains("== ui.options.group.ui =="), "{s}");
    assert!(s.contains("== ui.options.group.time =="), "{s}");
    assert!(s.contains("setting.ui.scale_percent: 100 (50..300)"), "{s}");
    assert!(s.contains("< ui.window.windowed >"), "{s}");
    assert!(
        s.contains("ui.options.restart"),
        "restart-required settings say so: {s}"
    );
    assert!(
        !s.contains("setting.ai.provider"),
        "AI settings have their own screen: {s}"
    );
    // Each kind of change becomes a SetSetting effect and updates the shown value.
    assert_eq!(
        m.update(UiEvent::Slide("setting.ui.scale_percent".into(), 150)),
        vec![AppEffect::SetSetting {
            id: "ui.scale_percent".into(),
            value: SettingValue::Int(150)
        }]
    );
    assert_eq!(
        m.update(UiEvent::Choose(
            "setting.ui.window_mode".into(),
            "fullscreen".into()
        )),
        vec![AppEffect::SetSetting {
            id: "ui.window_mode".into(),
            value: SettingValue::Text("fullscreen".into())
        }]
    );
    assert_eq!(
        m.update(UiEvent::Toggle(
            "setting.time.pause_on_focus_loss".into(),
            false
        )),
        vec![AppEffect::SetSetting {
            id: "time.pause_on_focus_loss".into(),
            value: SettingValue::Bool(false)
        }]
    );
    let s = snap(&m);
    assert!(
        s.contains("150 (50..300)")
            && s.contains("< ui.window.fullscreen >")
            && s.contains("[ ] setting.time"),
        "{s}"
    );
    // Out-of-range or unknown values are not forwarded.
    assert!(m
        .update(UiEvent::Slide("setting.ui.scale_percent".into(), 9999))
        .is_empty());
    assert!(m
        .update(UiEvent::Choose(
            "setting.ui.window_mode".into(),
            "nonsense".into()
        ))
        .is_empty());
    assert!(m
        .update(UiEvent::Toggle("setting.nothing".into(), true))
        .is_empty());
    // Keyboard: arrows adjust the focused slider.
    while m.focus() != Some("setting.ui.scale_percent") {
        press(&mut m, Key::Tab);
    }
    press(&mut m, Key::Left);
    assert!(
        snap(&m).contains("setting.ui.scale_percent: 145"),
        "{}",
        snap(&m)
    );
    press(&mut m, Key::Escape);
    assert!(matches!(m.screen(), Screen::MainMenu));
}

#[test]
fn pasting_a_key_never_shows_it_or_prints_it() {
    let mut m = boot(Vec::new(), None);
    click(&mut m, "main.options");
    click(&mut m, "options.ai");
    assert!(matches!(m.screen(), Screen::AiOptions(_)));
    let secret = "sk-test-0123456789abcdef";
    let ev = UiEvent::Text("ai.key".into(), secret.into());
    assert!(
        !format!("{ev:?}").contains(secret),
        "events print only a length"
    );
    m.update(ev);
    let s = snap(&m);
    assert!(!s.contains(secret) && s.contains("\u{2022}"), "{s}");
    let fx = click(&mut m, "ai.save_key");
    assert!(
        !format!("{fx:?}").contains(secret),
        "effects redact keys: {fx:?}"
    );
    assert_eq!(
        fx,
        vec![AppEffect::SetKey {
            provider: "openai".into(),
            key: Secret::new("anything")
        }],
        "equality compares the redacted form"
    );
    match fx.first() {
        Some(AppEffect::SetKey { key, provider }) => {
            assert_eq!(key.expose(), secret);
            assert_eq!(provider, "openai");
        }
        other => panic!("{other:?}"),
    }
    // The model no longer holds the key: saving again does nothing.
    assert!(click(&mut m, "ai.save_key").is_empty());
    assert!(!format!("{:?}", m.screen()).contains(secret));
}

#[test]
fn the_player2_sign_in_flow_shows_the_code_and_can_be_cancelled() {
    let mut m = boot(Vec::new(), None);
    click(&mut m, "main.options");
    click(&mut m, "options.ai");
    assert_eq!(
        m.update(UiEvent::Choose("ai.provider".into(), "player2".into())),
        vec![AppEffect::SelectProvider("player2".into())]
    );
    let s = snap(&m);
    assert!(
        s.contains("ai.signin") && s.contains("ui.ai.model_fixed"),
        "{s}"
    );
    assert!(
        s.contains("ai.test") && s.contains("(disabled)"),
        "no key, no test: {s}"
    );
    assert_eq!(
        click(&mut m, "ai.signin"),
        vec![AppEffect::StartLogin("player2".into())]
    );
    assert!(snap(&m).contains("ui.ai.login.contacting"));
    m.update(UiEvent::LoginPrompt {
        code: "ABCD-1234".into(),
        url: "https://player2.game/link".into(),
    });
    let s = snap(&m);
    assert!(
        s.contains("ui.ai.login.enter_code{code=ABCD-1234}")
            && s.contains("https://player2.game/link"),
        "{s}"
    );
    assert_eq!(
        click(&mut m, "ai.open_browser"),
        vec![AppEffect::OpenUrl("https://player2.game/link".into())]
    );
    // Escape on this screen cancels the pending sign-in as it leaves.
    let fx = press(&mut m, Key::Escape);
    assert_eq!(fx, vec![AppEffect::CancelLogin]);
    assert!(matches!(m.screen(), Screen::Options));
}

#[test]
fn sign_in_failures_and_success_and_the_connection_test() {
    let mut m = boot(Vec::new(), None);
    click(&mut m, "main.options");
    click(&mut m, "options.ai");
    m.update(UiEvent::Choose("ai.provider".into(), "player2".into()));
    click(&mut m, "ai.signin");
    m.update(UiEvent::LoginFinished(Err("expired".into())));
    let s = snap(&m);
    assert!(
        s.contains("ui.ai.login.failed{reason=expired}") && s.contains("ai.signin"),
        "retry is offered: {s}"
    );
    click(&mut m, "ai.signin");
    click(&mut m, "ai.cancel_login");
    assert!(snap(&m).contains("ai.signin"));
    // Success: the app reloads the AI state with a key present.
    click(&mut m, "ai.signin");
    m.update(UiEvent::LoginPrompt {
        code: "X".into(),
        url: "u".into(),
    });
    m.update(UiEvent::LoginFinished(Ok(())));
    let mut ai = providers();
    ai.selected = "player2".into();
    if let Some(p) = ai.providers.iter_mut().find(|p| p.id == "player2") {
        p.has_key = true;
    }
    m.update(UiEvent::AiLoaded(ai));
    let s = snap(&m);
    assert!(
        s.contains("ui.ai.login.signed_in") && s.contains("ui.ai.sign_out"),
        "{s}"
    );
    assert_eq!(
        click(&mut m, "ai.test"),
        vec![AppEffect::TestConnection("player2".into())]
    );
    assert!(snap(&m).contains("ui.ai.testing"));
    m.update(UiEvent::ConnectionResult(Ok("12 credits".into())));
    assert!(snap(&m).contains("ui.ai.test_ok{detail=12 credits}"));
    click(&mut m, "ai.test");
    m.update(UiEvent::ConnectionResult(Err("offline".into())));
    assert!(snap(&m).contains("ui.ai.test_failed{reason=offline}"));
    assert_eq!(
        click(&mut m, "ai.clear_key"),
        vec![AppEffect::ClearKey("player2".into())]
    );
    assert!(matches!(m.screen(), Screen::AiOptions(f) if f.login == LoginUi::Idle));
}

#[test]
fn model_and_provider_choices_become_effects() {
    let mut m = boot(Vec::new(), None);
    click(&mut m, "main.options");
    click(&mut m, "options.ai");
    m.update(UiEvent::Text("ai.model".into(), " gpt-x ".into()));
    assert_eq!(
        click(&mut m, "ai.save_model"),
        vec![AppEffect::SetModel {
            provider: "openai".into(),
            model: "gpt-x".into()
        }]
    );
    assert!(m
        .update(UiEvent::Choose("ai.provider".into(), "nope".into()))
        .is_empty());
    m.update(UiEvent::Choose("ai.provider".into(), "anthropic".into()));
    assert!(
        snap(&m).contains("ai.clear_key"),
        "a provider with a key offers to clear it"
    );
    assert!(
        snap(&m).contains("hint") || snap(&m).contains("<rec>"),
        "{}",
        snap(&m)
    );
}

#[test]
fn in_game_controls_and_the_pause_menu() {
    let mut m = in_world();
    m.update(UiEvent::Hud(HudInfo {
        world_name: "Town".into(),
        tick: 1500,
        day: 0,
        minute_of_day: 150,
        running: true,
        suspended: false,
        speed: "3x".into(),
        pawns: 12,
        status: String::new(),
    }));
    let s = snap(&m);
    assert!(
        s.contains("ui.hud.time{day=0,time=02:30}") && s.contains("ui.hud.speed{speed=3x}"),
        "{s}"
    );
    assert_eq!(
        click(&mut m, "hud.pause"),
        vec![AppEffect::SetRunning(false)]
    );
    assert_eq!(
        press(&mut m, Key::Space),
        vec![AppEffect::SetRunning(false)],
        "space toggles whatever has focus"
    );
    assert_eq!(
        click(&mut m, "hud.speed.9x"),
        vec![AppEffect::SetSpeed("9x".into())]
    );
    assert!(
        !s.contains("hud.save"),
        "saving lives in the pause menu: {s}"
    );
    // Escape opens the pause menu and closes it again; a running world is paused while the menu is open
    // and resumed when it closes.
    assert_eq!(
        press(&mut m, Key::Escape),
        vec![AppEffect::SetRunning(false)]
    );
    assert!(matches!(m.screen(), Screen::Pause));
    assert_eq!(
        press(&mut m, Key::Escape),
        vec![AppEffect::SetRunning(true)]
    );
    assert!(matches!(m.screen(), Screen::InGame));
    // A world that was already paused stays paused.
    m.update(UiEvent::Hud(HudInfo {
        running: false,
        ..m.hud().clone()
    }));
    assert!(press(&mut m, Key::Escape).is_empty());
    assert!(press(&mut m, Key::Escape).is_empty());
    m.update(UiEvent::Hud(HudInfo {
        running: true,
        ..m.hud().clone()
    }));
    // Options from the pause menu return to the pause menu.
    press(&mut m, Key::Escape);
    click(&mut m, "pause.options");
    assert!(matches!(m.screen(), Screen::Options));
    press(&mut m, Key::Escape);
    assert!(matches!(m.screen(), Screen::Pause));
    assert_eq!(click(&mut m, "pause.save"), vec![AppEffect::SaveNow]);
    let fx = click(&mut m, "pause.menu");
    assert_eq!(
        fx,
        vec![
            AppEffect::SaveNow,
            AppEffect::LeaveWorld,
            AppEffect::ListWorlds
        ]
    );
    assert!(matches!(m.screen(), Screen::MainMenu) && !m.in_world());
}

#[test]
fn quitting_and_closing_the_window_save_a_running_world() {
    let mut m = in_world();
    press(&mut m, Key::Escape);
    assert_eq!(
        click(&mut m, "pause.quit"),
        vec![AppEffect::SaveNow, AppEffect::Quit]
    );
    let mut m = in_world();
    assert_eq!(
        m.update(UiEvent::CloseRequested),
        vec![AppEffect::SaveNow, AppEffect::Quit]
    );
    let mut m = boot(Vec::new(), None);
    assert_eq!(m.update(UiEvent::CloseRequested), vec![AppEffect::Quit]);
    assert_eq!(click(&mut m, "main.quit"), vec![AppEffect::Quit]);
}

#[test]
fn losing_focus_in_a_world_asks_for_the_pause_flow_and_waits_for_the_player() {
    let mut m = in_world();
    assert_eq!(
        m.update(UiEvent::FocusLost),
        vec![AppEffect::WindowFocus(false)]
    );
    assert_eq!(
        m.update(UiEvent::FocusGained),
        vec![AppEffect::WindowFocus(true)]
    );
    let mut h = m.hud().clone();
    h.suspended = true;
    m.update(UiEvent::Hud(h));
    let s = snap(&m);
    assert!(
        s.contains("ui.hud.suspended") && s.contains("hud.resume"),
        "{s}"
    );
    assert_eq!(
        click(&mut m, "hud.resume"),
        vec![AppEffect::SetRunning(true)]
    );
    // In the menus focus changes mean nothing.
    let mut m = boot(Vec::new(), None);
    assert!(m.update(UiEvent::FocusLost).is_empty());
}

#[test]
fn a_crash_bundle_is_offered_once_at_launch() {
    let mut m = boot(
        vec![world("w1", "Town", 3, false)],
        Some("crash/crash-1.pgbundle"),
    );
    assert!(matches!(m.screen(), Screen::CrashPrompt { .. }));
    assert!(snap(&m).contains("crash/crash-1.pgbundle"));
    assert_eq!(
        click(&mut m, "crash.reveal"),
        vec![AppEffect::RevealPath("crash/crash-1.pgbundle".into())]
    );
    assert_eq!(press(&mut m, Key::Escape), vec![AppEffect::DismissCrash]);
    assert!(matches!(m.screen(), Screen::MainMenu));
}

#[test]
fn failures_show_a_notice_and_keep_the_screen() {
    let mut m = boot(vec![world("w1", "Town", 3, false)], None);
    let fx = click(&mut m, "main.continue");
    assert_eq!(fx, vec![AppEffect::LoadWorld("w1".into())]);
    m.update(UiEvent::Failed("The save is damaged".into()));
    assert!(matches!(m.screen(), Screen::MainMenu));
    assert!(snap(&m).contains("The save is damaged"));
    // The next input clears it.
    press(&mut m, Key::Tab);
    assert!(!snap(&m).contains("The save is damaged"));
}

#[test]
fn the_overlay_toggles_with_f3_and_its_clicks_become_effects() {
    let mut m = in_world();
    dev_mode(&mut m, true);
    assert!(m.overlay_tree(&show).is_none());
    press(&mut m, Key::F3);
    assert!(m.overlay_tree(&show).is_some());
    assert!(click(&mut m, "overlay.tab.events").is_empty());
    m.update(UiEvent::Text("overlay.filter".into(), "move".into()));
    assert_eq!(m.overlay().filter(), "move");
    assert_eq!(click(&mut m, "overlay.bundle"), vec![AppEffect::CutBundle]);
    assert_eq!(
        click(&mut m, "overlay.scrub.1800"),
        vec![AppEffect::Rewind { tick: 1800 }]
    );
    press(&mut m, Key::F3);
    assert!(m.overlay_tree(&show).is_none());
}

// ---- no dead ends, everything reachable by keyboard ---------------------------------------------------------------

/// Plays the app: answers an effect with the event the real app would send.
fn respond(m: &mut AppModel, fx: &[AppEffect]) {
    for e in fx {
        let replies: Vec<UiEvent> = match e {
            AppEffect::ListWorlds => vec![UiEvent::WorldsListed(vec![
                world("w1", "Town", 3, false),
                world("w2", "Old", 1, true),
            ])],
            AppEffect::LoadWorld(_) | AppEffect::CreateWorld { .. } => vec![UiEvent::WorldOpened {
                name: "Town".into(),
            }],
            AppEffect::StartLogin(_) => vec![UiEvent::LoginPrompt {
                code: "C".into(),
                url: "u".into(),
            }],
            AppEffect::TestConnection(_) => vec![UiEvent::ConnectionResult(Ok("ok".into()))],
            AppEffect::ImportWorld => vec![UiEvent::Imported("X".into())],
            AppEffect::ExportWorld(_) => vec![UiEvent::Exported("p".into())],
            _ => Vec::new(),
        };
        for r in replies {
            let more = m.update(r);
            respond(m, &more);
        }
    }
}

/// Every widget id on screen, activated one at a time from the same starting point.
fn explore(
    start: &AppModel,
    depth: usize,
    seen: &mut std::collections::BTreeSet<String>,
    walk: &dyn Fn(&AppModel),
) {
    let tree = start.tree(&show);
    let key = format!(
        "{:?}|{}",
        std::mem::discriminant(start.screen()),
        tree.snapshot(None)
    );
    if !seen.insert(key) || depth == 0 {
        return;
    }
    walk(start);
    for id in tree.focus_order() {
        let mut next = start.clone();
        let widget = tree.find(id).cloned();
        let fx = match widget {
            Some(Widget::Button { .. } | Widget::Drawer { .. }) => {
                next.update(UiEvent::Click(id.to_owned()))
            }
            None if tree.drawer_of(id).is_some() => next.update(UiEvent::Click(id.to_owned())),
            Some(Widget::Toggle { value, .. }) => {
                next.update(UiEvent::Toggle(id.to_owned(), !value))
            }
            Some(Widget::Choice { options, value, .. }) => {
                let other = options
                    .iter()
                    .find(|(v, _)| *v != value)
                    .map(|(v, _)| v.clone())
                    .unwrap_or(value);
                next.update(UiEvent::Choose(id.to_owned(), other))
            }
            Some(Widget::Slider { min, value, .. }) => next.update(UiEvent::Slide(
                id.to_owned(),
                if value == min { min + 1 } else { min },
            )),
            Some(Widget::TextField { .. }) => {
                next.update(UiEvent::Text(id.to_owned(), "abc".into()))
            }
            _ => Vec::new(),
        };
        if fx.iter().any(|e| matches!(e, AppEffect::Quit)) {
            continue;
        }
        respond(&mut next, &fx);
        explore(&next, depth - 1, seen, walk);
    }
}

/// From any state the keyboard alone gets back to the main menu: Escape (and, in a world, the pause menu's
/// "main menu" button).
fn escapes_to_main_menu(start: &AppModel) -> bool {
    let mut m = start.clone();
    for _ in 0..10 {
        match m.screen() {
            Screen::MainMenu => return true,
            Screen::InGame => {
                press(&mut m, Key::Escape);
            }
            Screen::Pause => {
                // Tab to "pause.menu" and press Enter, as a keyboard-only player would.
                for _ in 0..8 {
                    if m.focus() == Some("pause.menu") {
                        break;
                    }
                    press(&mut m, Key::Tab);
                }
                let fx = press(&mut m, Key::Enter);
                respond(&mut m, &fx);
            }
            _ => {
                let fx = press(&mut m, Key::Escape);
                respond(&mut m, &fx);
            }
        }
    }
    matches!(m.screen(), Screen::MainMenu)
}

#[test]
fn every_reachable_screen_has_focusable_widgets_unique_ids_and_a_way_back() {
    let start = boot(
        vec![world("w1", "Town", 3, false), world("w2", "Old", 1, true)],
        None,
    );
    let mut seen = std::collections::BTreeSet::new();
    let mut checked = 0;
    let walk = |m: &AppModel| {
        let tree = m.tree(&show);
        let order = tree.focus_order();
        assert!(
            !order.is_empty(),
            "a screen with nothing focusable is a dead end:\n{}",
            tree.snapshot(m.focus())
        );
        let mut ids: Vec<&str> = order.clone();
        ids.sort_unstable();
        let n = ids.len();
        ids.dedup();
        assert_eq!(
            ids.len(),
            n,
            "duplicate widget ids:\n{}",
            tree.snapshot(m.focus())
        );
        assert!(
            m.focus().is_some_and(|f| order.contains(&f)),
            "focus is on a real widget:\n{}",
            tree.snapshot(m.focus())
        );
        assert!(
            escapes_to_main_menu(m),
            "no keyboard way back from:\n{}",
            tree.snapshot(m.focus())
        );
        // Tab visits every focusable widget and comes back round.
        let mut walker = m.clone();
        let mut visited = std::collections::BTreeSet::new();
        for _ in 0..=order.len() {
            if let Some(f) = walker.focus() {
                visited.insert(f.to_owned());
            }
            press(&mut walker, Key::Tab);
        }
        assert_eq!(
            visited.len(),
            order.len(),
            "Tab must reach every widget:\n{}",
            tree.snapshot(m.focus())
        );
    };
    // Counted through a cell so the closure can stay `Fn`.
    let counter = std::cell::Cell::new(0);
    let counted = |m: &AppModel| {
        counter.set(counter.get() + 1);
        walk(m);
    };
    explore(&start, 6, &mut seen, &counted);
    checked += counter.get();
    assert!(
        checked > 25,
        "the exploration reached only {checked} states"
    );
}

#[test]
fn the_pseudo_text_of_every_screen_is_not_empty() {
    // Every label comes from the translator, so a screen built with a blank translator shows nothing but
    // ids: the check that no widget carries hard-coded English.
    let blank = |_: &str, _: &[(&str, &str)]| String::new();
    let m = boot(vec![world("w1", "Town", 3, false)], None);
    let tree = m.tree(&blank);
    for w in tree.walk() {
        if let Widget::Button { label, .. } = w {
            assert!(
                label.is_empty(),
                "button text must come from the string table: {label}"
            );
        }
    }
}

#[test]
fn the_speeds_live_in_one_drawer_that_the_keyboard_can_open_choose_from_and_close() {
    let mut m = in_world();
    m.update(UiEvent::Hud(HudInfo {
        running: true,
        speed: "3x".into(),
        ..m.hud().clone()
    }));
    let s = snap(&m);
    assert!(
        s.contains("[ui.hud.speed{speed=3x} v] <hud.speed>") && !s.contains("<hud.speed.9x>"),
        "closed, one button showing the speed: {s}"
    );
    // Opening it puts focus on the current speed and lists every speed after the button.
    assert!(click(&mut m, "hud.speed").is_empty());
    assert_eq!(m.open_drawer(), Some("hud.speed"));
    assert_eq!(m.focus(), Some("hud.speed.3x"));
    let order: Vec<String> = m
        .tree(&show)
        .focus_order()
        .iter()
        .map(|s| (*s).to_owned())
        .collect();
    let at = order.iter().position(|i| i == "hud.speed").unwrap();
    assert_eq!(
        &order[at..at + 5],
        [
            "hud.speed",
            "hud.speed.1x",
            "hud.speed.3x",
            "hud.speed.9x",
            "hud.speed.27x"
        ]
    );
    assert!(snap(&m).contains("*[3x] <hud.speed.3x>"), "{}", snap(&m));
    // Arrows move through the items and stop at the ends; Space does not pause while it is open.
    press(&mut m, Key::Down);
    assert_eq!(m.focus(), Some("hud.speed.9x"));
    press(&mut m, Key::Down);
    press(&mut m, Key::Down);
    assert_eq!(m.focus(), Some("hud.speed.27x"));
    press(&mut m, Key::Up);
    assert_eq!(
        press(&mut m, Key::Enter),
        vec![AppEffect::SetSpeed("9x".into())]
    );
    assert_eq!(m.open_drawer(), None);
    assert_eq!(m.focus(), Some("hud.speed"), "focus returns to the button");
    // Escape closes an open drawer before it opens the pause menu.
    click(&mut m, "hud.speed");
    assert_eq!(
        press(&mut m, Key::Space),
        vec![AppEffect::SetSpeed("3x".into())],
        "Space picks the focused item instead of pausing"
    );
    assert_eq!(m.open_drawer(), None);
    click(&mut m, "hud.speed");
    assert_eq!(m.open_drawer(), Some("hud.speed"));
    assert!(press(&mut m, Key::Escape).is_empty());
    assert!(matches!(m.screen(), Screen::InGame) && m.open_drawer().is_none());
    // Clicking the button again closes it, and the pause menu never starts with one open.
    click(&mut m, "hud.speed");
    click(&mut m, "hud.speed");
    assert_eq!(m.open_drawer(), None);
    click(&mut m, "hud.speed");
    press(&mut m, Key::Tab);
    click(&mut m, "hud.menu");
    assert!(matches!(m.screen(), Screen::Pause) && m.open_drawer().is_none());
}

#[test]
fn saving_is_in_the_pause_menu_only() {
    let mut m = in_world();
    let hud: Vec<String> = m
        .tree(&show)
        .focus_order()
        .iter()
        .map(|s| (*s).to_owned())
        .collect();
    assert!(!hud.iter().any(|i| i.contains("save")), "{hud:?}");
    press(&mut m, Key::Escape);
    assert!(m.tree(&show).focus_order().contains(&"pause.save"));
    assert_eq!(click(&mut m, "pause.save"), vec![AppEffect::SaveNow]);
}

#[test]
fn the_in_game_screen_has_a_top_bar_bottom_right_controls_and_a_notice_for_a_returning_player() {
    let mut m = in_world();
    let p = m.hud_parts(&show);
    let ids = |ws: &[Widget]| -> Vec<String> {
        crate::widget::Tree::new("t", ws.to_vec())
            .focus_order()
            .iter()
            .map(|s| (*s).to_owned())
            .collect()
    };
    assert!(ids(&p.top).is_empty(), "the top bar is information only");
    assert_eq!(
        ids(&p.controls),
        ["hud.pause", "hud.speed", "hud.journal", "hud.menu"]
    );
    assert!(p.dialog.is_empty());
    m.update(UiEvent::Hud(HudInfo {
        suspended: true,
        ..m.hud().clone()
    }));
    let p = m.hud_parts(&show);
    assert_eq!(ids(&p.dialog), ["hud.resume"]);
    // The notice comes first in the focus order, so Enter resumes.
    assert_eq!(m.tree(&show).focus_order()[0], "hud.resume");
    assert_eq!(m.focus(), Some("hud.resume"), "focus lands on the notice");
    assert_eq!(press(&mut m, Key::Enter), vec![AppEffect::SetRunning(true)]);
}

#[test]
fn developer_mode_gates_the_overlay_and_turning_it_off_closes_it() {
    let mut m = in_world();
    assert!(!m.dev_mode());
    press(&mut m, Key::F3);
    assert!(
        m.overlay_tree(&show).is_none(),
        "F3 does nothing outside developer mode"
    );
    dev_mode(&mut m, true);
    press(&mut m, Key::F3);
    assert!(m.overlay_tree(&show).is_some());
    press(&mut m, Key::Escape);
    click(&mut m, "pause.options");
    let fx = m.update(UiEvent::Toggle("setting.dev.enabled".into(), false));
    assert!(matches!(fx.as_slice(), [AppEffect::SetSetting { .. }]));
    assert!(
        m.overlay_tree(&show).is_none(),
        "the overlay closes with developer mode"
    );
    // The toggle is in the generated Options screen under its own group.
    let s = snap(&m);
    assert!(
        s.contains("ui.options.group.dev") && s.contains("setting.dev.enabled"),
        "{s}"
    );
    assert!(s.contains("setting.time.pause_on_focus_loss"), "{s}");
}

// ---- the developer console (milestone 1.3a) -------------------------------------------------------------

use pg_host::console::{Entry, Severity};

fn log_entry(seq: u64, severity: Severity, text: &str) -> Entry {
    Entry {
        seq,
        severity,
        source: "test".into(),
        tick: None,
        text: text.into(),
    }
}

fn console_snap(m: &AppModel) -> String {
    m.console_tree(&show)
        .map(|t| t.snapshot(None))
        .unwrap_or_default()
}

#[test]
fn the_backtick_opens_and_closes_the_console_only_in_developer_mode_and_escape_closes_it() {
    let mut m = in_world();
    assert!(press(&mut m, Key::Console).is_empty());
    assert!(!m.console().visible(), "developer mode is off");
    dev_mode(&mut m, true);
    press(&mut m, Key::Console);
    assert!(m.console().visible() && m.console_tree(&show).is_some());
    press(&mut m, Key::Console);
    assert!(!m.console().visible());
    // Escape closes it first, without opening the pause menu.
    press(&mut m, Key::Console);
    press(&mut m, Key::Escape);
    assert!(!m.console().visible() && matches!(m.screen(), Screen::InGame));
    // Turning developer mode off while it is open closes it.
    press(&mut m, Key::Console);
    assert!(m.console().visible());
    click(&mut m, "hud.menu");
    click(&mut m, "pause.options");
    m.update(UiEvent::Toggle("setting.dev.enabled".into(), false));
    assert!(!m.console().visible());
}

#[test]
fn the_console_works_on_every_screen_including_the_main_menu() {
    let mut m = boot(Vec::new(), None);
    // Developer mode from Options on the main menu.
    click(&mut m, "main.options");
    m.update(UiEvent::Toggle("setting.dev.enabled".into(), true));
    press(&mut m, Key::Escape);
    assert!(matches!(m.screen(), Screen::MainMenu));
    press(&mut m, Key::Console);
    assert!(m.console().visible());
    // Its widgets are clickable by id and do not disturb the screen under it.
    m.append_console(vec![log_entry(1, Severity::Error, "boot problem")]);
    assert!(console_snap(&m).contains("[Error]: boot problem"));
    assert!(matches!(m.screen(), Screen::MainMenu));
}

#[test]
fn the_filter_box_the_type_buttons_and_clear_work_through_events_and_a_new_world_resets_them() {
    let mut m = in_world();
    dev_mode(&mut m, true);
    press(&mut m, Key::Console);
    m.append_console(vec![
        log_entry(1, Severity::Debug, "noise"),
        log_entry(2, Severity::Info, "world opened"),
        log_entry(3, Severity::Warn, "task failed"),
        log_entry(4, Severity::Error, "save failed"),
    ]);
    let snap = console_snap(&m);
    assert!(snap.contains("(off) Debug <console.type.debug>"), "{snap}");
    assert!(snap.contains("(on) Info <console.type.info>"), "{snap}");
    assert!(
        !snap.contains("noise") && snap.contains("[Info]: world opened"),
        "{snap}"
    );
    // Turn Debug on, Warn off.
    m.update(UiEvent::Toggle("console.type.debug".into(), true));
    m.update(UiEvent::Toggle("console.type.warn".into(), false));
    let snap = console_snap(&m);
    assert!(
        snap.contains("[Debug]: noise") && !snap.contains("task failed"),
        "{snap}"
    );
    // The text box keeps only lines that contain the text.
    m.update(UiEvent::Text("console.filter".into(), "FAILED".into()));
    let snap = console_snap(&m);
    assert!(
        snap.contains("[Error]: save failed") && !snap.contains("world opened"),
        "{snap}"
    );
    // Clear asks the app to clear its buffer.
    assert_eq!(
        click(&mut m, "console.clear"),
        vec![AppEffect::ClearConsole]
    );
    assert!(!console_snap(&m).contains("save failed"));
    // A world being opened puts the filters back: Debug off, every other type on, no text.
    m.update(UiEvent::WorldOpened {
        name: "Again".into(),
    });
    assert!(!m.console().is_enabled(Severity::Debug));
    assert!(m.console().is_enabled(Severity::Warn) && m.console().filter().is_empty());
    // Typing into the console never moves screen focus or triggers screen widgets.
    assert!(m
        .update(UiEvent::Text("console.filter".into(), "x".into()))
        .is_empty());
}

#[test]
fn the_console_is_not_part_of_the_keyboard_focus_order_of_the_screen() {
    let mut m = in_world();
    dev_mode(&mut m, true);
    let before = m.tree(&show).focus_order().len();
    press(&mut m, Key::Console);
    m.append_console(vec![log_entry(1, Severity::Info, "x")]);
    assert_eq!(m.tree(&show).focus_order().len(), before);
}

#[test]
fn the_journal_opens_from_its_button_or_the_j_key_in_a_world_and_closes_again() {
    let mut m = boot(Vec::new(), None);
    press(&mut m, Key::Journal);
    assert!(!m.journal_visible(), "there is no world yet");
    let mut m = in_world();
    assert!(!m.journal_visible());
    click(&mut m, "hud.journal");
    assert!(m.journal_visible());
    click(&mut m, "journal.close");
    assert!(!m.journal_visible());
    press(&mut m, Key::Journal);
    assert!(m.journal_visible());
    press(&mut m, Key::Journal);
    assert!(!m.journal_visible());
    // The pause menu is not the journal's place: the key does nothing there.
    click(&mut m, "hud.menu");
    press(&mut m, Key::Journal);
    assert!(!m.journal_visible());
    // A new world starts with it closed.
    press(&mut m, Key::Escape);
    press(&mut m, Key::Journal);
    assert!(m.journal_visible());
    m.update(UiEvent::Hud(HudInfo {
        world_name: "Other".into(),
        ..HudInfo::default()
    }));
    assert!(m.journal_visible(), "the same world carries on");
}
