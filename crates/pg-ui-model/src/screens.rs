//! Each screen's widget tree and the effect of each widget (milestone 0.10).
//!
//! `build` is the view: screen state in, [`Tree`] out. `activate`, `text`, `toggle`, `choose` and `slide`
//! are the updates: widget event in, new state and [`AppEffect`]s out. Strings come from the string table
//! through `t`; widget ids are stable and never translated.

use crate::app::{AiForm, AppModel, ConnUi, LoginUi, NewWorldForm, SavedForm, Screen};
use crate::layout::{Align, DrawerLayout};
use crate::types::*;
use crate::widget::{DrawerItem, Tree, Widget};
use pg_host::Secret;

fn hhmm(minute_of_day: u32) -> String {
    format!("{:02}:{:02}", minute_of_day / 60, minute_of_day % 60)
}

/// The world the Continue button opens: the first (newest) one that is not damaged, else the first.
fn continue_target(worlds: &[WorldEntry]) -> Option<&WorldEntry> {
    worlds
        .iter()
        .find(|w| !w.damaged)
        .or_else(|| worlds.first())
}

pub(crate) fn build(m: &AppModel, t: Text) -> Tree {
    let st = m.state();
    match st.screen {
        Screen::Boot => Tree::new(
            t("ui.boot.title", &[]),
            vec![
                Widget::Heading(t("ui.boot.title", &[])),
                Widget::Note(t("ui.boot.loading", &[])),
            ],
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
        Screen::Mods => mods(m, t),
        Screen::InGame => in_game(m, t),
        Screen::Pause => pause(t),
    }
}

fn main_menu(worlds: &[WorldEntry], t: Text) -> Tree {
    let cont = match continue_target(worlds) {
        Some(w) => Widget::button(
            "main.continue",
            t(
                "ui.main.continue_named",
                &[("name", &w.name), ("day", &w.day.to_string())],
            ),
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
            Widget::button("main.mods", t("ui.main.mods", &[])),
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
        Widget::Slider {
            id: "new.water".into(),
            label: t("ui.new.water", &[]),
            min: 0,
            max: 50,
            value: f.water,
        },
        Widget::Choice {
            id: "new.tone".into(),
            label: t("ui.new.tone", &[]),
            options: ["cozy", "standard", "mature"]
                .iter()
                .map(|v| ((*v).to_owned(), t(&format!("ui.new.tone.{v}"), &[])))
                .collect(),
            value: f.tone.clone(),
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
            if let Some(name) = &e.thumbnail {
                w.push(Widget::Thumbnail { name: name.clone() });
            }
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
            w.push(Widget::Label(t(
                "ui.saved.confirm_delete",
                &[("name", &e.name)],
            )));
            w.push(Widget::button(
                "saved.confirm_yes",
                t("ui.saved.delete_yes", &[]),
            ));
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
            (SettingKind::Bool, SettingValue::Bool(v)) => Widget::Toggle {
                id,
                label,
                value: *v,
            },
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
                options: options
                    .iter()
                    .map(|(v, k)| (v.clone(), t(k, &[])))
                    .collect(),
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
    // Developer settings go last, out of the way of the ones every player uses.
    groups.sort_by_key(|(g, _)| g == "dev");
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
            options: ai
                .providers
                .iter()
                .map(|p| (p.id.clone(), p.label.clone()))
                .collect(),
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
                if ai.persistent_keys {
                    "ui.ai.key_stored_os"
                } else {
                    "ui.ai.key_session_only"
                },
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
                w.push(Widget::Label(t(
                    "ui.ai.login.enter_code",
                    &[("code", code)],
                )));
                w.push(Widget::Note(url.clone()));
                w.push(Widget::Note(t("ui.ai.login.waiting", &[])));
                w.push(Widget::button(
                    "ai.open_browser",
                    t("ui.ai.login.open", &[]),
                ));
                w.push(Widget::button("ai.cancel_login", t("ui.cancel", &[])));
            }
        },
    }
    if p.has_key {
        w.push(Widget::button(
            "ai.clear_key",
            t(
                if p.auth == Auth::DeviceCode {
                    "ui.ai.sign_out"
                } else {
                    "ui.ai.clear_key"
                },
                &[],
            ),
        ));
    }
    if p.model_fixed {
        w.push(Widget::Note(t(
            "ui.ai.model_fixed",
            &[("provider", &p.label)],
        )));
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

/// The in-game screen in the three places it is drawn: an information bar along the top, the simulation
/// controls in the bottom-right corner, and, while the window was in the background, a notice in the
/// middle of the screen. The focus order is the notice first, then the controls.
pub struct HudParts {
    pub top: Vec<Widget>,
    pub dialog: Vec<Widget>,
    pub controls: Vec<Widget>,
}

/// The speeds the drawer offers.
pub const SPEEDS: [&str; 4] = ["1x", "3x", "9x", "27x"];

pub(crate) fn hud_parts(m: &AppModel, t: Text) -> HudParts {
    let h = m.hud();
    let mut row = vec![
        Widget::Label(h.world_name.clone()),
        Widget::Label(t(
            "ui.hud.time",
            &[
                ("day", &h.day.to_string()),
                ("time", &hhmm(h.minute_of_day)),
            ],
        )),
        Widget::Label(t("ui.hud.residents", &[("count", &h.pawns.to_string())])),
    ];
    if !h.running {
        row.push(Widget::Label(t("ui.hud.paused", &[])));
    }
    let mut top = vec![Widget::Row(row)];
    if !h.status.is_empty() {
        top.push(Widget::Note(h.status.clone()));
    }
    let mut dialog = Vec::new();
    if h.suspended {
        dialog.push(Widget::Heading(t("ui.hud.suspended_title", &[])));
        dialog.push(Widget::Label(t("ui.hud.suspended", &[])));
        dialog.push(Widget::button("hud.resume", t("ui.hud.resume", &[])));
    }
    let controls = vec![
        Widget::button(
            "hud.pause",
            t(
                if h.running {
                    "ui.hud.pause"
                } else {
                    "ui.hud.play"
                },
                &[],
            ),
        ),
        Widget::Drawer {
            id: "hud.speed".into(),
            label: t("ui.hud.speed", &[("speed", &h.speed)]),
            open: m.open_drawer() == Some("hud.speed"),
            layout: DrawerLayout::List { max_rows: 8 },
            align: Align::End,
            items: SPEEDS
                .iter()
                .map(|s| DrawerItem {
                    id: format!("hud.speed.{s}"),
                    label: (*s).to_owned(),
                    selected: h.speed == *s,
                })
                .collect(),
        },
        Widget::button("hud.journal", t("ui.hud.journal", &[])),
        Widget::button("hud.menu", t("ui.hud.menu", &[])),
    ];
    HudParts {
        top,
        dialog,
        controls,
    }
}

fn in_game(m: &AppModel, t: Text) -> Tree {
    let p = hud_parts(m, t);
    let mut w = p.top;
    w.extend(p.dialog);
    w.extend(p.controls);
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
        Screen::Mods => "mods",
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
        ("main", "main.mods") => {
            m.go_to(Screen::Mods);
            vec![AppEffect::ListMods]
        }
        ("mods", _) => mods_activate(m, id),
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
        ("hud", "hud.menu") => m.open_pause(),
        ("hud", "hud.journal") => {
            m.toggle_journal();
            Vec::new()
        }
        ("hud", "hud.speed") => {
            m.toggle_drawer("hud.speed");
            Vec::new()
        }
        ("hud", _) => match id.strip_prefix("hud.speed.") {
            Some(s) => {
                m.close_drawer();
                vec![AppEffect::SetSpeed(s.to_owned())]
            }
            None => Vec::new(),
        },
        ("pause", "pause.resume") => m.close_pause(),
        ("pause", "pause.save") => vec![AppEffect::SaveNow],
        ("pause", "pause.menu") => {
            m.leave_world();
            vec![
                AppEffect::SaveNow,
                AppEffect::LeaveWorld,
                AppEffect::ListWorlds,
            ]
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
        water: u32::try_from(f.water).unwrap_or(0),
        tone: f.tone.clone(),
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
            Some(key) if !key.is_empty() => vec![AppEffect::SetKey {
                provider: p.id,
                key,
            }],
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
    match screen_kind(m) {
        "options" => return setting_changed(m, id, SettingValue::Bool(v)),
        "mods" => {
            if id == "mods.safe" {
                return vec![AppEffect::SetSafeMode(v)];
            }
            // `mods.cap.<pack>.<capability>`: pack ids never contain a dot, capability names do not either.
            if let Some((pack, cap)) = id.strip_prefix("mods.cap.").and_then(|r| r.split_once('.'))
            {
                return vec![AppEffect::ApproveCapability {
                    id: pack.to_owned(),
                    cap: cap.to_owned(),
                    on: v,
                }];
            }
        }
        _ => {}
    }
    Vec::new()
}

fn mods_activate(m: &mut AppModel, id: &str) -> Vec<AppEffect> {
    match id {
        "mods.back" => {
            m.go_back();
            Vec::new()
        }
        "mods.install" => vec![AppEffect::InstallPack],
        "mods.reveal" => vec![AppEffect::RevealPacks],
        other => {
            if let Some(key) = other.strip_prefix("mods.tile.") {
                m.toggle_mod_tile(key);
                return Vec::new();
            }
            if let Some(pack) = other.strip_prefix("mods.enable.") {
                let now = m
                    .mods()
                    .packs
                    .iter()
                    .find(|p| p.id == pack)
                    .map(|p| p.enabled);
                return match now {
                    Some(on) if !m.mods().packs.iter().any(|p| p.id == pack && p.base) => {
                        vec![AppEffect::SetPackEnabled {
                            id: pack.to_owned(),
                            on: !on,
                        }]
                    }
                    _ => Vec::new(),
                };
            }
            if let Some(key) = other.strip_prefix("mods.open.") {
                return match m
                    .mods()
                    .packs
                    .iter()
                    .find(|p| p.id == key || p.folder == key)
                {
                    Some(p) if !p.path.is_empty() => vec![AppEffect::RevealPath(p.path.clone())],
                    _ => Vec::new(),
                };
            }
            match other.strip_prefix("mods.remove.") {
                Some(folder) => vec![AppEffect::RemovePack(folder.to_owned())],
                None => Vec::new(),
            }
        }
    }
}

/// "3.4 KB", "812 B".
fn size_text(bytes: u64) -> String {
    if bytes >= 1_000_000 {
        format!("{}.{} MB", bytes / 1_000_000, bytes % 1_000_000 / 100_000)
    } else if bytes >= 1_000 {
        format!("{}.{} KB", bytes / 1_000, bytes % 1_000 / 100)
    } else {
        format!("{bytes} B")
    }
}

/// One pack as a tile: rolled up it shows the name, a problem mark, what the pack affects and an On/Off
/// button; unrolled it adds the pack's details, its capability approvals and its own buttons.
fn mod_tile(m: &AppModel, p: &crate::types::PackRow, t: Text) -> Widget {
    let key = if p.id.is_empty() { &p.folder } else { &p.id };
    let open = m.mod_tile_open(key);
    let needs_approval = p.enabled
        && p.capabilities
            .iter()
            .any(|c| c.needs_approval && !c.approved);
    let mut header = vec![Widget::button(
        &format!("mods.tile.{key}"),
        format!("{} {}", if open { "-" } else { "+" }, p.name),
    )];
    if p.error.is_some() {
        header.push(Widget::Label(t("ui.mods.error_mark", &[])));
    } else if needs_approval {
        header.push(Widget::Label(t("ui.mods.approval_mark", &[])));
    }
    if p.base {
        header.push(Widget::Note(t("ui.mods.base_short", &[])));
    } else if p.error.is_none() {
        let affects = p
            .capabilities
            .iter()
            .map(|c| c.name.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        header.push(Widget::Note(t("ui.mods.affects", &[("what", &affects)])));
        header.push(Widget::button(
            &format!("mods.enable.{}", p.id),
            t(
                if p.enabled {
                    "ui.mods.on"
                } else {
                    "ui.mods.off"
                },
                &[],
            ),
        ));
    }
    let mut kids = vec![Widget::Row(header)];
    if open {
        if let Some(e) = &p.error {
            kids.push(Widget::Label(t("ui.mods.invalid", &[])));
            kids.push(Widget::Note(e.chars().take(300).collect()));
        } else {
            let hash: String = p.hash.chars().take(12).collect();
            let details = if p.files > 0 {
                t(
                    "ui.mods.details",
                    &[
                        ("version", &p.version),
                        ("id", &p.id),
                        ("files", &p.files.to_string()),
                        ("size", &size_text(p.size_bytes)),
                        ("hash", &hash),
                    ],
                )
            } else {
                t(
                    "ui.mods.details_short",
                    &[("version", &p.version), ("id", &p.id), ("hash", &hash)],
                )
            };
            kids.push(Widget::Note(details));
            if !p.base {
                let state = match (p.loaded, p.enabled) {
                    (true, true) => "ui.mods.state.loaded",
                    (true, false) => "ui.mods.state.unload",
                    (false, true) => "ui.mods.state.pending",
                    (false, false) => "ui.mods.state.off",
                };
                kids.push(Widget::Label(t(state, &[])));
            }
            if !p.depends.is_empty() {
                kids.push(Widget::Note(t(
                    "ui.mods.depends",
                    &[("packs", &p.depends.join(", "))],
                )));
            }
            for c in &p.capabilities {
                let what = t(&format!("ui.mods.cap.{}", c.name.replace('-', "_")), &[]);
                if c.needs_approval {
                    kids.push(Widget::Toggle {
                        id: format!("mods.cap.{}.{}", p.id, c.name),
                        label: t("ui.mods.allow", &[("what", &what)]),
                        value: c.approved,
                    });
                } else {
                    kids.push(Widget::Note(t("ui.mods.asks", &[("what", &what)])));
                }
            }
        }
        if !p.base {
            let mut buttons = vec![Widget::button(
                &format!("mods.remove.{}", p.folder),
                t("ui.mods.remove", &[]),
            )];
            if p.error.is_none() {
                buttons.push(Widget::button(
                    &format!("mods.enable.{}", p.id),
                    t(
                        if p.enabled {
                            "ui.mods.disable"
                        } else {
                            "ui.mods.enable"
                        },
                        &[],
                    ),
                ));
            }
            buttons.push(Widget::button(
                &format!("mods.open.{key}"),
                t("ui.mods.open", &[]),
            ));
            kids.push(Widget::Row(buttons));
        }
    }
    Widget::Group {
        title: String::new(),
        children: kids,
    }
}

/// The Mods screen: a bounded, scrolling list of every pack as an unrollable tile, then the switches and
/// buttons that belong to the whole screen. A first-stage layout; the tiles have room for more later.
fn mods(m: &AppModel, t: Text) -> Tree {
    let v = m.mods();
    let mut w = vec![Widget::Heading(t("ui.mods.title", &[]))];
    w.push(Widget::Note(t("ui.mods.intro", &[])));
    if v.restart_needed {
        w.push(Widget::Label(t("ui.mods.restart", &[])));
    }
    for n in &v.notes {
        w.push(Widget::Note(n.clone()));
    }
    w.push(Widget::Scroll {
        max_height: 420,
        children: v.packs.iter().map(|p| mod_tile(m, p, t)).collect(),
    });
    w.push(Widget::Toggle {
        id: "mods.safe".into(),
        label: t("ui.mods.safe", &[]),
        value: v.safe_mode,
    });
    w.push(Widget::Row(vec![
        Widget::button("mods.install", t("ui.mods.install", &[])),
        Widget::button("mods.reveal", t("ui.mods.reveal", &[])),
    ]));
    if !v.folder.is_empty() {
        w.push(Widget::Note(t("ui.mods.folder", &[("folder", &v.folder)])));
    }
    w.push(Widget::button("mods.back", t("ui.mods.back", &[])));
    Tree::new(t("ui.mods.title", &[]), w)
}

pub(crate) fn choose(m: &mut AppModel, id: &str, v: &str) -> Vec<AppEffect> {
    match (screen_kind(m), id) {
        ("new", "new.size") => {
            if let (Screen::NewWorld(f), Some(size)) = (m.screen_mut(), MapSize::from_id(v)) {
                f.size = size;
            }
            Vec::new()
        }
        ("new", "new.tone") => {
            if let Screen::NewWorld(f) = m.screen_mut() {
                if ["cozy", "standard", "mature"].contains(&v) {
                    f.tone = v.to_owned();
                }
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
        ("new", "new.water") => {
            if let Screen::NewWorld(f) = m.screen_mut() {
                f.water = v.clamp(0, 50);
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
