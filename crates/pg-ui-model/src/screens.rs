//! Each screen's widget tree and the effect of each widget (milestone 0.10).
//!
//! `build` is the view: screen state in, [`Tree`] out. `activate`, `text`, `toggle`, `choose` and `slide`
//! are the updates: widget event in, new state and [`AppEffect`]s out. Strings come from the string table
//! through `t`; widget ids are stable and never translated.

use crate::app::{AiForm, AppModel, ConnUi, LoginUi, NewWorldForm, SavedForm, Screen};
use crate::types::*;
use crate::widget::{Tree, Widget};
use pg_host::Secret;

fn hhmm(minute_of_day: u32) -> String {
    format!("{:02}:{:02}", minute_of_day / 60, minute_of_day % 60)
}

/// The world the Continue button opens: the first (newest) one that is not damaged, else the first.
fn continue_target(worlds: &[WorldEntry]) -> Option<&WorldEntry> {
    worlds.iter().find(|w| !w.damaged).or_else(|| worlds.first())
}

pub(crate) fn build(m: &AppModel, t: Text) -> Tree {
    let st = m.state();
    match st.screen {
        Screen::Boot => Tree::new(
            t("ui.boot.title", &[]),
            vec![Widget::Heading(t("ui.boot.title", &[])), Widget::Note(t("ui.boot.loading", &[]))],
        ),
        Screen::CrashPrompt { path } => Tree::new(
            t("ui.crash.title", &[]),
            vec![
                Widget::Heading(t("ui.crash.title", &[])),
                Widget::Label(t("ui.crash.body", &[])),
                Widget::Note(path.clone()),
                Widget::button("crash.reveal", t("ui.crash.reveal", &[])),
                Widget::button("crash.dismiss", t("ui.crash.dismiss", &[])),
            ],
        ),
        Screen::MainMenu => main_menu(st.worlds, t),
        Screen::NewWorld(f) => new_world(f, t),
        Screen::SavedWorlds(f) => saved_worlds(st.worlds, f, t),
        Screen::Options => options(st.settings, t),
        Screen::AiOptions(f) => ai_options(st.ai, f, t),
        Screen::InGame => in_game(st.hud, t),
        Screen::Pause => pause(t),
    }
}

fn main_menu(worlds: &[WorldEntry], t: Text) -> Tree {
    let cont = match continue_target(worlds) {
        Some(w) => Widget::button(
            "main.continue",
            t("ui.main.continue_named", &[("name", &w.name), ("day", &w.day.to_string())]),
        ),
        None => Widget::disabled_button("main.continue", t("ui.main.continue", &[])),
    };
    Tree::new(
        t("ui.main.title", &[]),
        vec![
            Widget::Heading(t("ui.main.title", &[])),
            Widget::Note(t("ui.main.subtitle", &[])),
            Widget::Spacer,
            cont,
            Widget::button("main.new", t("ui.main.new", &[])),
            Widget::button("main.saved", t("ui.main.saved", &[])),
            Widget::button("main.options", t("ui.main.options", &[])),
            Widget::button("main.quit", t("ui.main.quit", &[])),
        ],
    )
}

fn new_world(f: &NewWorldForm, t: Text) -> Tree {
    let mut w = vec![
        Widget::Heading(t("ui.new.title", &[])),
        Widget::TextField {
            id: "new.name".into(),
            label: t("ui.new.name", &[]),
            value: f.name.clone(),
            secret: false,
            hint: t("ui.new.name_hint", &[]),
        },
        Widget::TextField {
            id: "new.seed".into(),
            label: t("ui.new.seed", &[]),
            value: f.seed.clone(),
            secret: false,
            hint: t("ui.new.seed_hint", &[]),
        },
        Widget::Choice {
            id: "new.size".into(),
            label: t("ui.new.size", &[]),
            options: MapSize::ALL
                .iter()
                .map(|s| {
                    let (w, h) = s.tiles();
                    (
                        s.id().to_owned(),
                        t(
                            &format!("ui.new.size.{}", s.id()),
                            &[("w", &w.to_string()), ("h", &h.to_string())],
                        ),
                    )
                })
                .collect(),
            value: f.size.id().to_owned(),
        },
        Widget::Slider {
            id: "new.residents".into(),
            label: t("ui.new.residents", &[]),
            min: 0,
            max: 50,
            value: f.residents,
        },
    ];
    if let Some(e) = f.error {
        w.push(Widget::Label(t(e, &[])));
    }
    w.push(Widget::button("new.create", t("ui.new.create", &[])));
    w.push(Widget::button("new.back", t("ui.back", &[])));
    Tree::new(t("ui.new.title", &[]), w)
}

fn saved_worlds(worlds: &[WorldEntry], f: &SavedForm, t: Text) -> Tree {
    let mut w = vec![Widget::Heading(t("ui.saved.title", &[]))];
    if worlds.is_empty() {
        w.push(Widget::Label(t("ui.saved.empty", &[])));
    }
    for e in worlds {
        let mut label = t(
            "ui.saved.row",
            &[
                ("name", &e.name),
                ("day", &e.day.to_string()),
                ("residents", &e.population.to_string()),
            ],
        );
        if e.damaged {
            label.push_str(&t("ui.saved.damaged_suffix", &[]));
        }
        w.push(Widget::button(&format!("saved.row.{}", e.id), label));
        if f.selected.as_deref() == Some(e.id.as_str()) {
            w.push(Widget::Note(t(
                "ui.saved.details",
                &[
                    ("saved", &e.saved),
                    ("packs", &e.packs.join(", ")),
                    ("ticks", &e.play_ticks.to_string()),
                ],
            )));
        }
    }
    let selected = f
        .selected
        .as_ref()
        .and_then(|id| worlds.iter().find(|e| &e.id == id));
    if f.confirm_delete {
        if let Some(e) = selected {
            w.push(Widget::Label(t("ui.saved.confirm_delete", &[("name", &e.name)])));
            w.push(Widget::button("saved.confirm_yes", t("ui.saved.delete_yes", &[])));
            w.push(Widget::button("saved.cancel", t("ui.cancel", &[])));
        }
    } else {
        let can = selected.is_some();
        let b = |id: &str, key: &str| {
            if can {
                Widget::button(id, t(key, &[]))
            } else {
                Widget::disabled_button(id, t(key, &[]))
            }
        };
        w.push(Widget::Row(vec![
            b("saved.load", "ui.saved.load"),
            b("saved.export", "ui.saved.export"),
            b("saved.delete", "ui.saved.delete"),
        ]));
        w.push(Widget::button("saved.import", t("ui.saved.import", &[])));
        w.push(Widget::button("saved.back", t("ui.back", &[])));
    }
    Tree::new(t("ui.saved.title", &[]), w)
}

/// The generated Options screen (suggestion S-025): one widget per registered device setting.
fn options(items: &[SettingItem], t: Text) -> Tree {
    let mut w = vec![Widget::Heading(t("ui.options.title", &[]))];
    let mut groups: Vec<(String, Vec<Widget>)> = Vec::new();
    for item in items.iter().filter(|i| !i.id.starts_with("ai.")) {
        let group = item.id.split('.').next().unwrap_or("misc").to_owned();
        let label = t(&item.label_key, &[]);
        let id = format!("setting.{}", item.id);
        let widget = match (&item.kind, &item.value) {
            (SettingKind::Bool, SettingValue::Bool(v)) => Widget::Toggle { id, label, value: *v },
            (SettingKind::Int { min, max }, SettingValue::Int(v)) => Widget::Slider {
                id,
                label,
                min: *min,
                max: *max,
                value: *v,
            },
            (SettingKind::Enum(options), SettingValue::Text(v)) => Widget::Choice {
                id,
                label,
                options: options.iter().map(|(v, k)| (v.clone(), t(k, &[]))).collect(),
                value: v.clone(),
            },
            (SettingKind::Text { .. }, SettingValue::Text(v)) => Widget::TextField {
                id,
                label,
                value: v.clone(),
                secret: false,
                hint: String::new(),
            },
            _ => continue,
        };
        let entry = match groups.iter_mut().find(|(g, _)| *g == group) {
            Some(e) => e,
            None => {
                groups.push((group.clone(), Vec::new()));
                match groups.last_mut() {
                    Some(e) => e,
                    None => continue,
                }
            }
        };
        entry.1.push(widget);
        if item.restart_required {
            entry.1.push(Widget::Note(t("ui.options.restart", &[])));
        }
    }
    for (g, children) in groups {
        w.push(Widget::Group {
            title: t(&format!("ui.options.group.{g}"), &[]),
            children,
        });
    }
    w.push(Widget::button("options.ai", t("ui.options.ai", &[])));
    w.push(Widget::button("options.back", t("ui.back", &[])));
    Tree::new(t("ui.options.title", &[]), w)
}

fn ai_options(ai: &AiState, f: &AiForm, t: Text) -> Tree {
    let mut w = vec![
        Widget::Heading(t("ui.ai.title", &[])),
        Widget::Note(t("ui.ai.optional", &[])),
        Widget::Choice {
            id: "ai.provider".into(),
            label: t("ui.ai.provider", &[]),
            options: ai.providers.iter().map(|p| (p.id.clone(), p.label.clone())).collect(),
            value: ai.selected.clone(),
        },
    ];
    let Some(p) = ai.providers.iter().find(|p| p.id == ai.selected) else {
        w.push(Widget::button("ai.back", t("ui.back", &[])));
        return Tree::new(t("ui.ai.title", &[]), w);
    };
    match p.auth {
        Auth::PasteKey => {
            w.push(Widget::Note(t(
                if ai.persistent_keys { "ui.ai.key_stored_os" } else { "ui.ai.key_session_only" },
                &[],
            )));
            w.push(Widget::TextField {
                id: "ai.key".into(),
                label: t("ui.ai.key", &[]),
                value: f
                    .key_input
                    .as_ref()
                    .map(|k| "\u{2022}".repeat(k.expose().chars().count()))
                    .unwrap_or_default(),
                secret: true,
                hint: t("ui.ai.key_hint", &[]),
            });
            let has_input = f.key_input.as_ref().is_some_and(|k| !k.is_empty());
            w.push(if has_input {
                Widget::button("ai.save_key", t("ui.ai.save_key", &[]))
            } else {
                Widget::disabled_button("ai.save_key", t("ui.ai.save_key", &[]))
            });
        }
        Auth::DeviceCode => match &f.login {
            LoginUi::Idle | LoginUi::Done | LoginUi::Failed(_) => {
                if let LoginUi::Failed(m) = &f.login {
                    w.push(Widget::Label(t("ui.ai.login.failed", &[("reason", m)])));
                }
                if p.has_key {
                    w.push(Widget::Label(t("ui.ai.login.signed_in", &[])));
                } else {
                    w.push(Widget::Note(t("ui.ai.login.explain", &[])));
                    w.push(Widget::button("ai.signin", t("ui.ai.login.start", &[])));
                }
            }
            LoginUi::Starting => {
                w.push(Widget::Note(t("ui.ai.login.contacting", &[])));
                w.push(Widget::button("ai.cancel_login", t("ui.cancel", &[])));
            }
            LoginUi::Prompt { code, url } => {
                w.push(Widget::Label(t("ui.ai.login.enter_code", &[("code", code)])));
                w.push(Widget::Note(url.clone()));
                w.push(Widget::Note(t("ui.ai.login.waiting", &[])));
                w.push(Widget::button("ai.open_browser", t("ui.ai.login.open", &[])));
                w.push(Widget::button("ai.cancel_login", t("ui.cancel", &[])));
            }
        },
    }
    if p.has_key {
        w.push(Widget::button(
            "ai.clear_key",
            t(if p.auth == Auth::DeviceCode { "ui.ai.sign_out" } else { "ui.ai.clear_key" }, &[]),
        ));
    }
    if p.model_fixed {
        w.push(Widget::Note(t("ui.ai.model_fixed", &[("provider", &p.label)])));
    } else {
        w.push(Widget::TextField {
            id: "ai.model".into(),
            label: t("ui.ai.model", &[]),
            value: f.model_input.clone(),
            secret: false,
            hint: p.recommended_model.clone(),
        });
        w.push(Widget::button("ai.save_model", t("ui.ai.save_model", &[])));
    }
    w.push(if p.has_key {
        Widget::button("ai.test", t("ui.ai.test", &[]))
    } else {
        Widget::disabled_button("ai.test", t("ui.ai.test", &[]))
    });
    match &f.conn {
        ConnUi::Idle => {}
        ConnUi::Testing => w.push(Widget::Note(t("ui.ai.testing", &[]))),
        ConnUi::Ok(m) => w.push(Widget::Label(t("ui.ai.test_ok", &[("detail", m)]))),
        ConnUi::Failed(m) => w.push(Widget::Label(t("ui.ai.test_failed", &[("reason", m)]))),
    }
    w.push(Widget::button("ai.back", t("ui.back", &[])));
    Tree::new(t("ui.ai.title", &[]), w)
}

fn in_game(h: &HudInfo, t: Text) -> Tree {
    let mut w = vec![Widget::Row(vec![
        Widget::Label(h.world_name.clone()),
        Widget::Label(t(
            "ui.hud.time",
            &[("day", &h.day.to_string()), ("time", &hhmm(h.minute_of_day))],
        )),
        Widget::Label(if h.running {
            t("ui.hud.speed", &[("speed", &h.speed)])
        } else {
            t("ui.hud.paused", &[])
        }),
        Widget::Label(t("ui.hud.residents", &[("count", &h.pawns.to_string())])),
    ])];
    if h.suspended {
        w.push(Widget::Label(t("ui.hud.suspended", &[])));
        w.push(Widget::button("hud.resume", t("ui.hud.resume", &[])));
    }
    let mut controls = vec![Widget::button(
        "hud.pause",
        t(if h.running { "ui.hud.pause" } else { "ui.hud.play" }, &[]),
    )];
    for s in ["1x", "3x", "9x", "27x"] {
        controls.push(Widget::button(&format!("hud.speed.{s}"), s));
    }
    controls.push(Widget::button("hud.save", t("ui.hud.save", &[])));
    controls.push(Widget::button("hud.menu", t("ui.hud.menu", &[])));
    w.push(Widget::Row(controls));
    if !h.status.is_empty() {
        w.push(Widget::Note(h.status.clone()));
    }
    Tree::new(t("ui.hud.title", &[]), w)
}

fn pause(t: Text) -> Tree {
    Tree::new(
        t("ui.pause.title", &[]),
        vec![
            Widget::Heading(t("ui.pause.title", &[])),
            Widget::button("pause.resume", t("ui.pause.resume", &[])),
            Widget::button("pause.save", t("ui.pause.save", &[])),
            Widget::button("pause.options", t("ui.pause.options", &[])),
            Widget::button("pause.menu", t("ui.pause.menu", &[])),
            Widget::button("pause.quit", t("ui.pause.quit", &[])),
        ],
    )
}

// ---- updates -----------------------------------------------------------------------------------------------

fn screen_kind(m: &AppModel) -> &'static str {
    match m.screen() {
        Screen::Boot => "boot",
        Screen::CrashPrompt { .. } => "crash",
        Screen::MainMenu => "main",
        Screen::NewWorld(_) => "new",
        Screen::SavedWorlds(_) => "saved",
        Screen::Options => "options",
        Screen::AiOptions(_) => "ai",
        Screen::InGame => "hud",
        Screen::Pause => "pause",
    }
}

fn valid_name(name: &str) -> bool {
    let n = name.trim();
    !n.is_empty() && n.chars().count() <= 40 && !n.chars().any(char::is_control)
}

pub(crate) fn activate(m: &mut AppModel, id: &str) -> Vec<AppEffect> {
    match (screen_kind(m), id) {
        ("main", "main.continue") => match continue_target(m.worlds()) {
            Some(w) => vec![AppEffect::LoadWorld(w.id.clone())],
            None => Vec::new(),
        },
        ("main", "main.new") => {
            m.go_to(Screen::NewWorld(NewWorldForm::default()));
            Vec::new()
        }
        ("main", "main.saved") => {
            m.go_to(Screen::SavedWorlds(SavedForm::default()));
            vec![AppEffect::ListWorlds]
        }
        ("main", "main.options") | ("pause", "pause.options") => {
            m.go_to(Screen::Options);
            Vec::new()
        }
        ("main", "main.quit") => vec![AppEffect::Quit],
        ("crash", "crash.reveal") => match m.screen() {
            Screen::CrashPrompt { path } => vec![AppEffect::RevealPath(path.clone())],
            _ => Vec::new(),
        },
        ("crash", "crash.dismiss") => {
            m.leave_world();
            vec![AppEffect::DismissCrash]
        }
        ("new", "new.back") | ("saved", "saved.back") | ("options", "options.back") => {
            m.go_back();
            Vec::new()
        }
        ("new", "new.create") => new_create(m),
        ("saved", _) => saved_activate(m, id),
        ("options", "options.ai") => {
            let f = m.new_ai_form();
            m.go_to(Screen::AiOptions(f));
            Vec::new()
        }
        ("ai", _) => ai_activate(m, id),
        ("hud", "hud.pause") => vec![AppEffect::SetRunning(!m.hud().running)],
        ("hud", "hud.resume") => vec![AppEffect::SetRunning(true)],
        ("hud", "hud.save") => vec![AppEffect::SaveNow],
        ("hud", "hud.menu") => {
            m.go_to(Screen::Pause);
            Vec::new()
        }
        ("hud", _) => match id.strip_prefix("hud.speed.") {
            Some(s) => vec![AppEffect::SetSpeed(s.to_owned())],
            None => Vec::new(),
        },
        ("pause", "pause.resume") => {
            m.go_back();
            Vec::new()
        }
        ("pause", "pause.save") => vec![AppEffect::SaveNow],
        ("pause", "pause.menu") => {
            m.leave_world();
            vec![AppEffect::SaveNow, AppEffect::LeaveWorld, AppEffect::ListWorlds]
        }
        ("pause", "pause.quit") => vec![AppEffect::SaveNow, AppEffect::Quit],
        _ => Vec::new(),
    }
}

fn new_create(m: &mut AppModel) -> Vec<AppEffect> {
    let Screen::NewWorld(f) = m.screen_mut() else {
        return Vec::new();
    };
    if !valid_name(&f.name) {
        f.error = Some("ui.new.error.name");
        return Vec::new();
    }
    if f.seed.chars().count() > 64 || f.seed.chars().any(char::is_control) {
        f.error = Some("ui.new.error.seed");
        return Vec::new();
    }
    f.error = None;
    vec![AppEffect::CreateWorld {
        name: f.name.trim().to_owned(),
        seed: f.seed.trim().to_owned(),
        size: f.size,
        residents: u32::try_from(f.residents).unwrap_or(0),
    }]
}

fn saved_activate(m: &mut AppModel, id: &str) -> Vec<AppEffect> {
    let Screen::SavedWorlds(f) = m.screen_mut() else {
        return Vec::new();
    };
    if let Some(world) = id.strip_prefix("saved.row.") {
        f.selected = Some(world.to_owned());
        f.confirm_delete = false;
        return Vec::new();
    }
    let selected = f.selected.clone();
    match (id, selected) {
        ("saved.load", Some(w)) => vec![AppEffect::LoadWorld(w)],
        ("saved.export", Some(w)) => vec![AppEffect::ExportWorld(w)],
        ("saved.delete", Some(_)) => {
            f.confirm_delete = true;
            Vec::new()
        }
        ("saved.confirm_yes", Some(w)) => {
            f.confirm_delete = false;
            f.selected = None;
            vec![AppEffect::DeleteWorld(w)]
        }
        ("saved.cancel", _) => {
            f.confirm_delete = false;
            Vec::new()
        }
        ("saved.import", _) => vec![AppEffect::ImportWorld],
        ("saved.back", _) => {
            m.go_back();
            Vec::new()
        }
        _ => Vec::new(),
    }
}

fn selected_provider(m: &AppModel) -> Option<ProviderInfo> {
    let ai = m.ai_state();
    ai.providers.iter().find(|p| p.id == ai.selected).cloned()
}

fn ai_activate(m: &mut AppModel, id: &str) -> Vec<AppEffect> {
    let Some(p) = selected_provider(m) else {
        if id == "ai.back" {
            m.go_back();
        }
        return Vec::new();
    };
    let Screen::AiOptions(f) = m.screen_mut() else {
        return Vec::new();
    };
    match id {
        "ai.save_key" => match f.key_input.take() {
            Some(key) if !key.is_empty() => vec![AppEffect::SetKey { provider: p.id, key }],
            other => {
                f.key_input = other;
                Vec::new()
            }
        },
        "ai.clear_key" => {
            f.login = LoginUi::Idle;
            f.conn = ConnUi::Idle;
            vec![AppEffect::ClearKey(p.id)]
        }
        "ai.save_model" => vec![AppEffect::SetModel {
            provider: p.id,
            model: f.model_input.trim().to_owned(),
        }],
        "ai.test" => {
            f.conn = ConnUi::Testing;
            vec![AppEffect::TestConnection(p.id)]
        }
        "ai.signin" => {
            f.login = LoginUi::Starting;
            vec![AppEffect::StartLogin(p.id)]
        }
        "ai.cancel_login" => {
            f.login = LoginUi::Idle;
            vec![AppEffect::CancelLogin]
        }
        "ai.open_browser" => match &f.login {
            LoginUi::Prompt { url, .. } => vec![AppEffect::OpenUrl(url.clone())],
            _ => Vec::new(),
        },
        "ai.back" => {
            let cancel = matches!(f.login, LoginUi::Starting | LoginUi::Prompt { .. });
            m.go_back();
            if cancel {
                vec![AppEffect::CancelLogin]
            } else {
                Vec::new()
            }
        }
        _ => Vec::new(),
    }
}

pub(crate) fn text(m: &mut AppModel, id: &str, s: String) -> Vec<AppEffect> {
    match (screen_kind(m), id) {
        ("new", "new.name") => {
            if let Screen::NewWorld(f) = m.screen_mut() {
                f.name = s.chars().take(60).collect();
                f.error = None;
            }
            Vec::new()
        }
        ("new", "new.seed") => {
            if let Screen::NewWorld(f) = m.screen_mut() {
                f.seed = s.chars().take(80).collect();
                f.error = None;
            }
            Vec::new()
        }
        ("ai", "ai.key") => {
            if let Screen::AiOptions(f) = m.screen_mut() {
                // The text goes straight into a Secret: it is never held as a plain string again.
                f.key_input = (!s.is_empty()).then(|| Secret::new(s));
            }
            Vec::new()
        }
        ("ai", "ai.model") => {
            if let Screen::AiOptions(f) = m.screen_mut() {
                f.model_input = s.chars().take(100).collect();
            }
            Vec::new()
        }
        ("options", _) => setting_changed(m, id, SettingValue::Text(s)),
        _ => Vec::new(),
    }
}

pub(crate) fn toggle(m: &mut AppModel, id: &str, v: bool) -> Vec<AppEffect> {
    if screen_kind(m) == "options" {
        return setting_changed(m, id, SettingValue::Bool(v));
    }
    Vec::new()
}

pub(crate) fn choose(m: &mut AppModel, id: &str, v: &str) -> Vec<AppEffect> {
    match (screen_kind(m), id) {
        ("new", "new.size") => {
            if let (Screen::NewWorld(f), Some(size)) = (m.screen_mut(), MapSize::from_id(v)) {
                f.size = size;
            }
            Vec::new()
        }
        ("ai", "ai.provider") => {
            if !m.ai_state().providers.iter().any(|p| p.id == v) {
                return Vec::new();
            }
            let cancel = matches!(
                m.screen(),
                Screen::AiOptions(f) if matches!(f.login, LoginUi::Starting | LoginUi::Prompt { .. })
            );
            m.ai_mut().selected = v.to_owned();
            let form = m.ai_form_for(v);
            *m.screen_mut() = Screen::AiOptions(form);
            let mut out = Vec::new();
            if cancel {
                out.push(AppEffect::CancelLogin);
            }
            out.push(AppEffect::SelectProvider(v.to_owned()));
            out
        }
        ("options", _) => setting_changed(m, id, SettingValue::Text(v.to_owned())),
        _ => Vec::new(),
    }
}

pub(crate) fn slide(m: &mut AppModel, id: &str, v: i64) -> Vec<AppEffect> {
    match (screen_kind(m), id) {
        ("new", "new.residents") => {
            if let Screen::NewWorld(f) = m.screen_mut() {
                f.residents = v.clamp(0, 50);
            }
            Vec::new()
        }
        ("options", _) => setting_changed(m, id, SettingValue::Int(v)),
        _ => Vec::new(),
    }
}

/// A generated setting changed: update the shown value and ask the app to validate and save it.
fn setting_changed(m: &mut AppModel, id: &str, value: SettingValue) -> Vec<AppEffect> {
    let Some(setting) = id.strip_prefix("setting.") else {
        return Vec::new();
    };
    let Some(item) = m.settings_mut().iter_mut().find(|i| i.id == setting) else {
        return Vec::new();
    };
    let ok = match (&item.kind, &value) {
        (SettingKind::Bool, SettingValue::Bool(_)) => true,
        (SettingKind::Int { min, max }, SettingValue::Int(v)) => (*min..=*max).contains(v),
        (SettingKind::Enum(options), SettingValue::Text(v)) => options.iter().any(|(o, _)| o == v),
        (SettingKind::Text { max_len }, SettingValue::Text(v)) => v.chars().count() <= *max_len,
        _ => false,
    };
    if !ok {
        return Vec::new();
    }
    item.value = value.clone();
    vec![AppEffect::SetSetting {
        id: setting.to_owned(),
        value,
    }]
}
