//! Each screen's widget tree and the effect of each widget (milestone 0.10).
//!
//! `build` is the view: screen state in, [`Tree`] out. `activate`, `text`, `toggle`, `choose` and `slide`
//! are the updates: widget event in, new state and [`AppEffect`]s out. Strings come from the string table
//! through `t`; widget ids are stable and never translated.

use crate::app::{AiForm, AppModel, ConnUi, LoginUi, NewWorldForm, SavedForm, Screen};
use crate::layout::{Align, DrawerLayout};
use crate::types::*;
use crate::widget::{BadgeTone, Border, BorderTone, DrawerItem, Tree, Widget, FILL};
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
        Screen::NewWorld(f) => new_world(m, f, t),
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
        Some(w) => Widget::Primary(Box::new(Widget::button(
            "main.continue",
            t(
                "ui.main.continue_named",
                &[("name", &w.name), ("day", &w.day.to_string())],
            ),
        ))),
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

/// The New world screen: a preview slot and the per-world pack list on the left, the settings in category
/// boxes on the right, and Create / Back pinned side by side at the bottom.
fn new_world(m: &AppModel, f: &NewWorldForm, t: Text) -> Tree {
    let size_options = MapSize::ALL
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
        .collect();
    // The category boxes only have a line between neighbours; the column around them has the box.
    let group = |title: &str, children: Vec<Widget>| Widget::Group {
        border: Border::between(1, BorderTone::Normal),
        title: t(title, &[]),
        children,
    };
    // The sample town: a picture of what these settings make (the placeholder until the first one is ready).
    let mut left = vec![match &f.preview {
        Some(name) => Widget::Thumbnail { name: name.clone() },
        None => Widget::Placeholder {
            label: t("ui.new.preview", &[]),
            height: 240,
        },
    }];
    let (tw, th) = f.size.tiles();
    left.push(Widget::Note(t(
        if f.preview_busy {
            "ui.new.preview_busy"
        } else if f.seed.trim().is_empty() {
            "ui.new.preview_sample"
        } else {
            "ui.new.preview_note"
        },
        &[
            ("w", &tw.to_string()),
            ("h", &th.to_string()),
            ("residents", &f.residents.to_string()),
        ],
    )));
    // The packs this world will use: the base game and whatever is enabled under Mods. Choosing packs for each
    // world separately comes later; for now Manage packs opens the Mods screen.
    let mut packs: Vec<Widget> = Vec::new();
    let loaded: Vec<(String, String)> = m
        .mods()
        .packs
        .iter()
        .filter(|p| p.loaded)
        .map(|p| (p.name.clone(), p.version.clone()))
        .collect();
    if !loaded.is_empty() {
        packs.push(Widget::Facts(loaded));
    }
    packs.push(Widget::Note(t("ui.new.packs.note", &[])));
    packs.push(Widget::button("new.packs", t("ui.new.packs.choose", &[])));
    left.push(Widget::Group {
        border: Border::GROUP,
        title: t("ui.new.packs.title", &[]),
        children: packs,
    });
    let settings = vec![
        group(
            "ui.new.group.world",
            vec![
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
                Widget::Row(vec![Widget::button(
                    "new.randomize",
                    t("ui.new.randomize", &[]),
                )]),
                Widget::Choice {
                    id: "new.size".into(),
                    label: t("ui.new.size", &[]),
                    options: size_options,
                    value: f.size.id().to_owned(),
                },
                Widget::Slider {
                    id: "new.water".into(),
                    label: t("ui.new.water", &[]),
                    min: 0,
                    max: 50,
                    value: f.water,
                },
            ],
        ),
        group(
            "ui.new.group.residents",
            vec![Widget::Slider {
                id: "new.residents".into(),
                label: t("ui.new.residents", &[]),
                min: 0,
                max: 50,
                value: f.residents,
            }],
        ),
        group(
            "ui.new.group.content",
            vec![Widget::Choice {
                id: "new.tone".into(),
                label: t("ui.new.tone", &[]),
                options: ["cozy", "standard", "mature"]
                    .iter()
                    .map(|v| ((*v).to_owned(), t(&format!("ui.new.tone.{v}"), &[])))
                    .collect(),
                value: f.tone.clone(),
            }],
        ),
    ];
    // The settings column has a border of its own; the category boxes inside keep their own, apart from it.
    let right = vec![Widget::Bordered {
        border: Border::solid(2, BorderTone::Strong),
        inner: Box::new(Widget::Stack(settings)),
    }];
    let mut w = vec![Widget::Heading(t("ui.new.title", &[]))];
    if let Some(e) = f.error {
        w.push(Widget::Label(t(e, &[])));
    }
    w.push(Widget::Columns { left, right });
    Tree::new(t("ui.new.title", &[]), w).with_footer(
        960,
        vec![Widget::ButtonBar(vec![
            Widget::Primary(Box::new(Widget::button(
                "new.create",
                t("ui.new.create", &[]),
            ))),
            Widget::button("new.back", t("ui.back", &[])),
        ])],
    )
}

/// The Saved worlds screen: the worlds as a list on the left, and everything about the chosen one (its picture,
/// facts and what can be done with it) in a pane on the right. The newest world is chosen when the screen opens.
fn saved_worlds(worlds: &[WorldEntry], f: &SavedForm, t: Text) -> Tree {
    let chosen = chosen_world(worlds, f.selected.as_deref());
    let rows: Vec<Widget> = worlds
        .iter()
        .map(|e| {
            let mut badges = Vec::new();
            if e.damaged {
                badges.push((t("ui.saved.badge.damaged", &[]), BadgeTone::Bad));
            }
            Widget::ListRow {
                id: format!("saved.row.{}", e.id),
                title: e.name.clone(),
                subtitle: t(
                    "ui.saved.row_sub",
                    &[
                        ("day", &e.day.to_string()),
                        ("residents", &e.population.to_string()),
                    ],
                ),
                badges,
                selected: chosen.is_some_and(|c| c.id == e.id),
            }
        })
        .collect();
    let left = if rows.is_empty() {
        vec![Widget::Note(t("ui.saved.empty", &[]))]
    } else {
        vec![Widget::Scroll {
            max_height: FILL,
            children: rows,
        }]
    };
    let between = |title: String, children: Vec<Widget>| Widget::Group {
        border: Border::between(1, BorderTone::Normal),
        title,
        children,
    };
    let mut sections = Vec::new();
    if let Some(e) = chosen {
        let mut head = vec![Widget::Heading(e.name.clone())];
        let mut badges = vec![
            Widget::Badge {
                label: t("ui.saved.badge.day", &[("day", &e.day.to_string())]),
                tone: BadgeTone::Neutral,
            },
            Widget::Badge {
                label: t(
                    "ui.saved.badge.residents",
                    &[("count", &e.population.to_string())],
                ),
                tone: BadgeTone::Neutral,
            },
        ];
        if e.damaged {
            badges.push(Widget::Badge {
                label: t("ui.saved.badge.damaged", &[]),
                tone: BadgeTone::Bad,
            });
        }
        head.push(Widget::Row(badges));
        if let Some(name) = &e.thumbnail {
            head.push(Widget::Thumbnail { name: name.clone() });
        }
        sections.push(between(String::new(), head));
        sections.push(between(
            t("ui.saved.group.details", &[]),
            vec![Widget::Facts(vec![
                (t("ui.saved.fact.saved", &[]), e.saved.clone()),
                (
                    t("ui.saved.fact.played", &[]),
                    t("ui.saved.played", &[("ticks", &e.play_ticks.to_string())]),
                ),
                (t("ui.saved.fact.packs", &[]), e.packs.join(", ")),
            ])],
        ));
        sections.push(between(
            String::new(),
            if f.confirm_delete {
                vec![
                    Widget::Label(t("ui.saved.confirm_delete", &[("name", &e.name)])),
                    Widget::Row(vec![
                        Widget::button("saved.confirm_yes", t("ui.saved.delete_yes", &[])),
                        Widget::button("saved.cancel", t("ui.cancel", &[])),
                    ]),
                ]
            } else {
                vec![Widget::Row(vec![
                    Widget::Primary(Box::new(Widget::button(
                        "saved.load",
                        t("ui.saved.load", &[]),
                    ))),
                    Widget::button("saved.export", t("ui.saved.export", &[])),
                    Widget::button("saved.delete", t("ui.saved.delete", &[])),
                ])]
            },
        ));
    } else {
        sections.push(between(
            String::new(),
            vec![Widget::Note(t("ui.saved.none_chosen", &[]))],
        ));
    }
    Tree::new(
        t("ui.saved.title", &[]),
        vec![
            Widget::Heading(t("ui.saved.title", &[])),
            Widget::Columns {
                left,
                right: vec![detail_pane(sections)],
            },
        ],
    )
    .with_footer(
        1100,
        vec![Widget::ButtonBar(vec![
            Widget::button("saved.import", t("ui.saved.import", &[])),
            Widget::button("saved.back", t("ui.back", &[])),
        ])],
    )
}

/// The chosen world: the one picked, else the newest that is not damaged, else the first.
fn chosen_world<'a>(worlds: &'a [WorldEntry], picked: Option<&str>) -> Option<&'a WorldEntry> {
    picked
        .and_then(|id| worlds.iter().find(|w| w.id == id))
        .or_else(|| continue_target(worlds))
}

/// A bordered pane that holds a detail view: its sections scroll inside the border when they are too tall.
fn detail_pane(sections: Vec<Widget>) -> Widget {
    Widget::Bordered {
        border: Border::solid(2, BorderTone::Strong),
        inner: Box::new(Widget::Scroll {
            max_height: FILL,
            children: vec![Widget::Stack(sections)],
        }),
    }
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
    let boxes = groups
        .into_iter()
        .map(|(g, children)| Widget::Group {
            border: Border::between(1, BorderTone::Normal),
            title: t(&format!("ui.options.group.{g}"), &[]),
            children,
        })
        .collect();
    w.push(column(boxes));
    Tree::new(t("ui.options.title", &[]), w).with_footer(
        640,
        vec![Widget::ButtonBar(vec![
            Widget::button("options.ai", t("ui.options.ai", &[])),
            Widget::button("options.back", t("ui.back", &[])),
        ])],
    )
}

/// A screen's settings as one bordered column whose category boxes only have lines between them.
fn column(boxes: Vec<Widget>) -> Widget {
    Widget::Bordered {
        border: Border::solid(2, BorderTone::Strong),
        inner: Box::new(Widget::Stack(boxes)),
    }
}

fn ai_options(ai: &AiState, f: &AiForm, t: Text) -> Tree {
    let boxed = |title: &str, children: Vec<Widget>| Widget::Group {
        border: Border::between(1, BorderTone::Normal),
        title: t(title, &[]),
        children,
    };
    let footer = |back: Widget| vec![Widget::ButtonBar(vec![back])];
    let head = vec![
        Widget::Heading(t("ui.ai.title", &[])),
        Widget::Note(t("ui.ai.optional", &[])),
    ];
    let provider = vec![Widget::Choice {
        id: "ai.provider".into(),
        label: t("ui.ai.provider", &[]),
        options: ai
            .providers
            .iter()
            .map(|p| (p.id.clone(), p.label.clone()))
            .collect(),
        value: ai.selected.clone(),
    }];
    let back = || Widget::button("ai.back", t("ui.back", &[]));
    let Some(p) = ai.providers.iter().find(|p| p.id == ai.selected) else {
        let mut w = head;
        w.push(column(vec![boxed("ui.ai.group.provider", provider)]));
        return Tree::new(t("ui.ai.title", &[]), w).with_footer(640, footer(back()));
    };
    let mut acct: Vec<Widget> = Vec::new();
    let mut model: Vec<Widget> = Vec::new();
    let mut conn: Vec<Widget> = Vec::new();
    match p.auth {
        Auth::PasteKey => {
            acct.push(Widget::Note(t(
                if ai.persistent_keys {
                    "ui.ai.key_stored_os"
                } else {
                    "ui.ai.key_session_only"
                },
                &[],
            )));
            acct.push(Widget::TextField {
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
            acct.push(if has_input {
                Widget::button("ai.save_key", t("ui.ai.save_key", &[]))
            } else {
                Widget::disabled_button("ai.save_key", t("ui.ai.save_key", &[]))
            });
        }
        Auth::DeviceCode => match &f.login {
            LoginUi::Idle | LoginUi::Done | LoginUi::Failed(_) => {
                if let LoginUi::Failed(m) = &f.login {
                    acct.push(Widget::Label(t("ui.ai.login.failed", &[("reason", m)])));
                }
                if p.has_key {
                    acct.push(Widget::Label(t("ui.ai.login.signed_in", &[])));
                } else {
                    acct.push(Widget::Note(t("ui.ai.login.explain", &[])));
                    acct.push(Widget::button("ai.signin", t("ui.ai.login.start", &[])));
                }
            }
            LoginUi::Starting => {
                acct.push(Widget::Note(t("ui.ai.login.contacting", &[])));
                acct.push(Widget::button("ai.cancel_login", t("ui.cancel", &[])));
            }
            LoginUi::Prompt { code, url } => {
                acct.push(Widget::Label(t(
                    "ui.ai.login.enter_code",
                    &[("code", code)],
                )));
                acct.push(Widget::Note(url.clone()));
                acct.push(Widget::Note(t("ui.ai.login.waiting", &[])));
                acct.push(Widget::button(
                    "ai.open_browser",
                    t("ui.ai.login.open", &[]),
                ));
                acct.push(Widget::button("ai.cancel_login", t("ui.cancel", &[])));
            }
        },
    }
    if p.has_key {
        acct.push(Widget::button(
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
        model.push(Widget::Note(t(
            "ui.ai.model_fixed",
            &[("provider", &p.label)],
        )));
    } else {
        model.push(Widget::TextField {
            id: "ai.model".into(),
            label: t("ui.ai.model", &[]),
            value: f.model_input.clone(),
            secret: false,
            hint: p.recommended_model.clone(),
        });
        model.push(Widget::button("ai.save_model", t("ui.ai.save_model", &[])));
    }
    conn.push(if p.has_key {
        Widget::button("ai.test", t("ui.ai.test", &[]))
    } else {
        Widget::disabled_button("ai.test", t("ui.ai.test", &[]))
    });
    match &f.conn {
        ConnUi::Idle => {}
        ConnUi::Testing => conn.push(Widget::Note(t("ui.ai.testing", &[]))),
        ConnUi::Ok(m) => conn.push(Widget::Label(t("ui.ai.test_ok", &[("detail", m)]))),
        ConnUi::Failed(m) => conn.push(Widget::Label(t("ui.ai.test_failed", &[("reason", m)]))),
    }
    let mut w = head;
    w.push(column(vec![
        boxed("ui.ai.group.provider", provider),
        boxed("ui.ai.group.account", acct),
        boxed("ui.ai.group.model", model),
        boxed("ui.ai.group.connection", conn),
    ]));
    Tree::new(t("ui.ai.title", &[]), w).with_footer(640, footer(back()))
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
    // The world's name, then small status chips: the time, the residents, and a Paused tag when stopped.
    let badge = |label: String, tone| Widget::Badge { label, tone };
    let mut row = vec![
        Widget::Label(h.world_name.clone()),
        badge(
            t(
                "ui.hud.time",
                &[
                    ("day", &h.day.to_string()),
                    ("time", &hhmm(h.minute_of_day)),
                ],
            ),
            BadgeTone::Neutral,
        ),
        badge(
            t("ui.hud.residents", &[("count", &h.pawns.to_string())]),
            BadgeTone::Neutral,
        ),
    ];
    if !h.running {
        row.push(badge(t("ui.hud.paused", &[]), BadgeTone::Accent));
    }
    let mut top = vec![Widget::Row(row)];
    if !h.status.is_empty() {
        top.push(Widget::Note(h.status.clone()));
    }
    let mut dialog = Vec::new();
    if h.suspended {
        dialog.push(Widget::Heading(t("ui.hud.suspended_title", &[])));
        dialog.push(Widget::Label(t("ui.hud.suspended", &[])));
        dialog.push(Widget::Primary(Box::new(Widget::button(
            "hud.resume",
            t("ui.hud.resume", &[]),
        ))));
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

#[cfg(test)]
pub(crate) fn build_for_test_pause(t: Text) -> Tree {
    pause(t)
}

fn pause(t: Text) -> Tree {
    // Three boxes with lines between them: carry on, keep or change things, leave.
    let boxed = |children: Vec<Widget>| Widget::Group {
        border: Border::between(1, BorderTone::Normal),
        title: String::new(),
        children,
    };
    Tree::new(
        t("ui.pause.title", &[]),
        vec![
            Widget::Heading(t("ui.pause.title", &[])),
            Widget::Stack(vec![
                boxed(vec![Widget::Primary(Box::new(Widget::button(
                    "pause.resume",
                    t("ui.pause.resume", &[]),
                )))]),
                boxed(vec![
                    Widget::button("pause.save", t("ui.pause.save", &[])),
                    Widget::button("pause.options", t("ui.pause.options", &[])),
                ]),
                boxed(vec![
                    Widget::button("pause.menu", t("ui.pause.menu", &[])),
                    Widget::button("pause.quit", t("ui.pause.quit", &[])),
                ]),
            ]),
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
            let f = NewWorldForm {
                preview_busy: true,
                ..NewWorldForm::default()
            };
            let preview = f.preview_effect();
            m.go_to(Screen::NewWorld(f));
            // Typing starts at the name, not at the pack button that comes first on the left.
            m.focus_on("new.name");
            // The packs this world will use come from the Mods list; the sample town is made in the background.
            vec![AppEffect::ListMods, preview]
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
        ("new", "new.randomize") => vec![AppEffect::RandomSeed],
        ("new", "new.packs") => {
            m.go_to(Screen::Mods);
            vec![AppEffect::ListMods]
        }
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
    // The world chosen when nothing was picked is the newest, so Load works as soon as the screen opens.
    let default = continue_target(m.worlds()).map(|w| w.id.clone());
    let known: Vec<String> = m.worlds().iter().map(|w| w.id.clone()).collect();
    let Screen::SavedWorlds(f) = m.screen_mut() else {
        return Vec::new();
    };
    if let Some(world) = id.strip_prefix("saved.row.") {
        f.selected = Some(world.to_owned());
        f.confirm_delete = false;
        return Vec::new();
    }
    let selected = f
        .selected
        .clone()
        .filter(|id| known.contains(id))
        .or(default);
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
                f.preview_busy = true;
                return vec![f.preview_effect()];
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
            if let Some(key) = other.strip_prefix("mods.row.") {
                m.select_mod(key);
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
            if let Some(folder) = other.strip_prefix("mods.remove_yes.") {
                m.set_mods_confirm(None);
                return vec![AppEffect::RemovePack(folder.to_owned())];
            }
            if other == "mods.remove_cancel" {
                m.set_mods_confirm(None);
                return Vec::new();
            }
            if let Some(folder) = other.strip_prefix("mods.remove.") {
                // Removing deletes the pack's folder, so it asks first.
                m.set_mods_confirm(Some(folder.to_owned()));
            }
            Vec::new()
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

fn pack_key(p: &crate::types::PackRow) -> &str {
    if p.id.is_empty() {
        &p.folder
    } else {
        &p.id
    }
}

/// The packs in the order the list shows them: the base game first, then installed packs by name, and the ones
/// that could not be loaded last.
fn ordered_packs(m: &AppModel) -> Vec<&crate::types::PackRow> {
    let mut v: Vec<&crate::types::PackRow> = m.mods().packs.iter().collect();
    v.sort_by_key(|p| (!p.base, p.error.is_some(), p.name.to_lowercase()));
    v
}

/// The chosen pack: the one picked, else the first installed pack that loads, else the first in the list.
fn chosen_pack(m: &AppModel) -> Option<&crate::types::PackRow> {
    let packs = ordered_packs(m);
    m.mods_selected()
        .and_then(|k| packs.iter().copied().find(|p| pack_key(p) == k))
        .or_else(|| packs.iter().copied().find(|p| !p.base && p.error.is_none()))
        .or_else(|| packs.first().copied())
}

/// One pack as a list row: its name, version and what it affects, and badges for how it stands.
fn mod_row(p: &crate::types::PackRow, chosen: bool, t: Text) -> Widget {
    let needs_approval = p.enabled
        && p.capabilities
            .iter()
            .any(|c| c.needs_approval && !c.approved);
    let mut badges = Vec::new();
    let subtitle = if p.base {
        badges.push((t("ui.mods.badge.base", &[]), BadgeTone::Neutral));
        t("ui.mods.base_short", &[])
    } else if p.error.is_some() {
        badges.push((t("ui.mods.badge.error", &[]), BadgeTone::Bad));
        t("ui.mods.unreadable", &[])
    } else {
        badges.push(if p.enabled {
            (t("ui.mods.on", &[]), BadgeTone::Info)
        } else {
            (t("ui.mods.off", &[]), BadgeTone::Neutral)
        });
        if needs_approval {
            badges.push((t("ui.mods.badge.approval", &[]), BadgeTone::Accent));
        }
        let affects = p
            .capabilities
            .iter()
            .map(|c| c.name.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        t(
            "ui.mods.row_sub",
            &[("version", &p.version), ("what", &affects)],
        )
    };
    Widget::ListRow {
        id: format!("mods.row.{}", pack_key(p)),
        title: p.name.clone(),
        subtitle,
        badges,
        selected: chosen,
    }
}

/// Everything about the chosen pack, as sections for the detail pane.
fn mod_detail(m: &AppModel, p: &crate::types::PackRow, t: Text) -> Vec<Widget> {
    let between = |title: String, children: Vec<Widget>| Widget::Group {
        border: Border::between(1, BorderTone::Normal),
        title,
        children,
    };
    let badge = |label: String, tone| Widget::Badge { label, tone };
    // Header: the name and how the pack stands.
    let mut chips = Vec::new();
    if p.base {
        chips.push(badge(t("ui.mods.badge.base", &[]), BadgeTone::Neutral));
    } else if p.error.is_some() {
        chips.push(badge(t("ui.mods.badge.error", &[]), BadgeTone::Bad));
    } else {
        chips.push(badge(
            t("ui.mods.version", &[("version", &p.version)]),
            BadgeTone::Neutral,
        ));
        chips.push(if p.enabled {
            badge(t("ui.mods.on", &[]), BadgeTone::Info)
        } else {
            badge(t("ui.mods.off", &[]), BadgeTone::Neutral)
        });
        if p.enabled
            && p.capabilities
                .iter()
                .any(|c| c.needs_approval && !c.approved)
        {
            chips.push(badge(t("ui.mods.badge.approval", &[]), BadgeTone::Accent));
        }
    }
    let mut out = vec![between(
        String::new(),
        vec![Widget::Heading(p.name.clone()), Widget::Row(chips)],
    )];
    if let Some(e) = &p.error {
        out.push(between(
            t("ui.mods.group.problem", &[]),
            vec![
                Widget::Label(t("ui.mods.invalid", &[])),
                Widget::Note(e.chars().take(300).collect()),
            ],
        ));
    } else {
        let mut facts = Vec::new();
        if !p.id.is_empty() {
            facts.push((t("ui.mods.fact.id", &[]), p.id.clone()));
        }
        facts.push((t("ui.mods.fact.version", &[]), p.version.clone()));
        if p.files > 0 {
            facts.push((t("ui.mods.fact.files", &[]), p.files.to_string()));
            facts.push((t("ui.mods.fact.size", &[]), size_text(p.size_bytes)));
        }
        facts.push((
            t("ui.mods.fact.hash", &[]),
            p.hash.chars().take(12).collect(),
        ));
        if !p.base {
            let state = match (p.loaded, p.enabled) {
                (true, true) => "ui.mods.state.loaded",
                (true, false) => "ui.mods.state.unload",
                (false, true) => "ui.mods.state.pending",
                (false, false) => "ui.mods.state.off",
            };
            facts.push((t("ui.mods.fact.status", &[]), t(state, &[])));
        }
        if !p.depends.is_empty() {
            facts.push((t("ui.mods.fact.needs", &[]), p.depends.join(", ")));
        }
        out.push(between(
            t("ui.mods.group.details", &[]),
            vec![Widget::Facts(facts)],
        ));
        if !p.capabilities.is_empty() {
            let rows = p
                .capabilities
                .iter()
                .map(|c| {
                    let what = t(&format!("ui.mods.cap.{}", c.name.replace('-', "_")), &[]);
                    if c.needs_approval {
                        Widget::Toggle {
                            id: format!("mods.cap.{}.{}", p.id, c.name),
                            label: t("ui.mods.allow", &[("what", &what)]),
                            value: c.approved,
                        }
                    } else {
                        Widget::Note(t("ui.mods.asks", &[("what", &what)]))
                    }
                })
                .collect();
            out.push(between(t("ui.mods.group.permissions", &[]), rows));
        }
    }
    if !p.base {
        let mut actions = Vec::new();
        if m.mods_confirm() == Some(p.folder.as_str()) {
            actions.push(Widget::Label(t(
                "ui.mods.confirm_remove",
                &[("name", &p.name)],
            )));
            actions.push(Widget::Row(vec![
                Widget::button(
                    &format!("mods.remove_yes.{}", p.folder),
                    t("ui.mods.remove_yes", &[]),
                ),
                Widget::button("mods.remove_cancel", t("ui.cancel", &[])),
            ]));
        } else {
            let mut buttons = Vec::new();
            if p.error.is_none() {
                let toggle = Widget::button(
                    &format!("mods.enable.{}", p.id),
                    t(
                        if p.enabled {
                            "ui.mods.disable"
                        } else {
                            "ui.mods.enable"
                        },
                        &[],
                    ),
                );
                // Turning a pack on is the main thing to do with one that is off.
                buttons.push(if p.enabled {
                    toggle
                } else {
                    Widget::Primary(Box::new(toggle))
                });
            }
            buttons.push(Widget::button(
                &format!("mods.open.{}", pack_key(p)),
                t("ui.mods.open", &[]),
            ));
            buttons.push(Widget::button(
                &format!("mods.remove.{}", p.folder),
                t("ui.mods.remove", &[]),
            ));
            actions.push(Widget::Row(buttons));
        }
        out.push(between(String::new(), actions));
    }
    out
}

/// The Mods screen: every pack as a list row on the left, with the safe-mode switch under it; the chosen pack's
/// facts, permissions and actions in a pane on the right. Install, the packs folder and Back are pinned below.
fn mods(m: &AppModel, t: Text) -> Tree {
    let v = m.mods();
    let chosen = chosen_pack(m);
    let mut w = vec![Widget::Heading(t("ui.mods.title", &[]))];
    w.push(Widget::Note(t("ui.mods.intro", &[])));
    if v.restart_needed {
        w.push(Widget::Row(vec![Widget::Badge {
            label: t("ui.mods.restart", &[]),
            tone: BadgeTone::Accent,
        }]));
    }
    for n in &v.notes {
        w.push(Widget::Note(n.clone()));
    }
    let rows: Vec<Widget> = ordered_packs(m)
        .into_iter()
        .map(|p| mod_row(p, chosen.is_some_and(|c| pack_key(c) == pack_key(p)), t))
        .collect();
    let mut left = vec![Widget::Scroll {
        max_height: FILL,
        children: rows,
    }];
    let mut options = vec![Widget::Toggle {
        id: "mods.safe".into(),
        label: t("ui.mods.safe", &[]),
        value: v.safe_mode,
    }];
    if !v.folder.is_empty() {
        options.push(Widget::Note(t("ui.mods.folder", &[("folder", &v.folder)])));
    }
    left.push(Widget::Group {
        border: Border::GROUP,
        title: String::new(),
        children: options,
    });
    let right = match chosen {
        Some(p) => vec![detail_pane(mod_detail(m, p, t))],
        None => vec![detail_pane(vec![Widget::Note(t(
            "ui.mods.none_chosen",
            &[],
        ))])],
    };
    w.push(Widget::Columns { left, right });
    Tree::new(t("ui.mods.title", &[]), w).with_footer(
        1100,
        vec![Widget::ButtonBar(vec![
            Widget::button("mods.install", t("ui.mods.install", &[])),
            Widget::button("mods.reveal", t("ui.mods.reveal", &[])),
            Widget::button("mods.back", t("ui.mods.back", &[])),
        ])],
    )
}

pub(crate) fn choose(m: &mut AppModel, id: &str, v: &str) -> Vec<AppEffect> {
    match (screen_kind(m), id) {
        ("new", "new.size") => {
            if let (Screen::NewWorld(f), Some(size)) = (m.screen_mut(), MapSize::from_id(v)) {
                f.size = size;
                f.preview_busy = true;
                return vec![f.preview_effect()];
            }
            Vec::new()
        }
        ("new", "new.tone") => {
            if let Screen::NewWorld(f) = m.screen_mut() {
                if ["cozy", "standard", "mature"].contains(&v) {
                    f.tone = v.to_owned();
                    f.preview_busy = true;
                    return vec![f.preview_effect()];
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
                f.preview_busy = true;
                return vec![f.preview_effect()];
            }
            Vec::new()
        }
        ("new", "new.water") => {
            if let Screen::NewWorld(f) = m.screen_mut() {
                f.water = v.clamp(0, 50);
                f.preview_busy = true;
                return vec![f.preview_effect()];
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
