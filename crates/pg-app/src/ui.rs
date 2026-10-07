//! Drawing a widget tree with egui, and turning what the player does into model events.
//!
//! The model owns what is on screen and where keyboard focus is; this module only maps each [`Widget`] to
//! the egui widget that looks and behaves like it, and reports clicks, edits and focus changes back as
//! [`UiEvent`]s. Keyboard navigation (Tab, arrows, Enter, Escape) is not done here: [`collect_keys`] turns
//! key presses into model keys, and the model moves focus, which this module then shows.

use egui::{Align, Color32, Id, Key as EKey, Layout, Modifiers, RichText, Stroke};
use pg_ui_model::types::{Key, UiEvent};
use pg_ui_model::widget::{Tree, Widget};
use std::collections::BTreeMap;

/// What one drawn tree produced.
#[derive(Default)]
pub struct Drawn {
    pub events: Vec<UiEvent>,
    /// Where each interactive widget ended up (for tests and for the focus ring).
    pub rects: BTreeMap<String, egui::Rect>,
}

/// Remembers the model's focus between frames, so egui focus is moved only when the model moved it.
#[derive(Default)]
pub struct FocusTracker {
    last: Option<String>,
}

impl FocusTracker {
    pub fn update(&mut self, focus: Option<&str>) {
        self.last = focus.map(str::to_owned);
    }
}

fn egui_id(id: &str) -> Id {
    Id::new(("pg-widget", id))
}

/// Finds the texture for a saved picture by its storage name, with the size to draw it at.
pub type Images<'a> = &'a mut dyn FnMut(&str) -> Option<(egui::TextureId, egui::Vec2)>;

/// Draws `tree`. `focus` is the model's focused widget id.
pub fn draw_tree(
    ui: &mut egui::Ui,
    tree: &Tree,
    focus: Option<&str>,
    tracker: &FocusTracker,
    images: Images<'_>,
) -> Drawn {
    let mut d = Drawn::default();
    let focus_moved = tracker.last.as_deref() != focus;
    for w in &tree.widgets {
        draw_widget(ui, w, focus, focus_moved, false, images, &mut d);
    }
    d
}

fn ring(ui: &egui::Ui, r: &egui::Response, id: &str, focus: Option<&str>) {
    if focus == Some(id) {
        ui.painter().rect_stroke(
            r.rect.expand(3.0),
            4.0,
            Stroke::new(2.0, Color32::from_rgb(250, 205, 90)),
            egui::StrokeKind::Outside,
        );
    }
}

/// `compact`: inside a row, where buttons take their natural width instead of filling the column.
fn draw_widget(
    ui: &mut egui::Ui,
    w: &Widget,
    focus: Option<&str>,
    focus_moved: bool,
    compact: bool,
    images: Images<'_>,
    d: &mut Drawn,
) {
    match w {
        Widget::Heading(t) => {
            ui.add_space(4.0);
            ui.heading(RichText::new(t).size(28.0).strong());
        }
        Widget::Label(t) => {
            ui.label(RichText::new(t).size(16.0));
        }
        Widget::Note(t) => {
            ui.label(RichText::new(t).size(13.0).weak());
        }
        Widget::Spacer => {
            ui.add_space(14.0);
        }
        Widget::Button { id, label, enabled } => {
            let width = if compact {
                0.0
            } else {
                ui.available_width().min(420.0)
            };
            let b = egui::Button::new(RichText::new(label).size(17.0))
                .min_size(egui::vec2(width, 34.0));
            let r = ui.add_enabled(*enabled, b);
            ring(ui, &r, id, focus);
            if r.clicked() {
                d.events.push(UiEvent::Click(id.clone()));
            }
            d.rects.insert(id.clone(), r.rect);
        }
        Widget::TextField {
            id,
            label,
            value,
            secret,
            hint,
        } => {
            ui.label(RichText::new(label).size(14.0));
            let eid = egui_id(id);
            // A secret's real text lives only here, in egui's temporary memory; the model's copy is dots.
            let mut text = if *secret {
                let mut buf: String = ui.data_mut(|m| m.get_temp(eid).unwrap_or_default());
                if value.is_empty() && !buf.is_empty() {
                    buf.clear();
                    ui.data_mut(|m| m.insert_temp(eid, buf.clone()));
                }
                buf
            } else {
                value.clone()
            };
            let r = ui.add(
                egui::TextEdit::singleline(&mut text)
                    .id(eid)
                    .password(*secret)
                    .hint_text(hint)
                    .desired_width(320.0),
            );
            if focus == Some(id.as_str()) && focus_moved && !r.has_focus() {
                r.request_focus();
            }
            if r.gained_focus() && focus != Some(id.as_str()) {
                d.events.push(UiEvent::Click(id.clone()));
            }
            if r.changed() {
                if *secret {
                    ui.data_mut(|m| m.insert_temp(eid, text.clone()));
                }
                d.events.push(UiEvent::Text(id.clone(), text));
            }
            ring(ui, &r, id, focus);
            d.rects.insert(id.clone(), r.rect);
        }
        Widget::Toggle { id, label, value } => {
            let mut v = *value;
            let r = ui.checkbox(&mut v, RichText::new(label).size(16.0));
            ring(ui, &r, id, focus);
            if r.changed() {
                d.events.push(UiEvent::Toggle(id.clone(), v));
            }
            d.rects.insert(id.clone(), r.rect);
        }
        Widget::Choice {
            id,
            label,
            options,
            value,
        } => {
            let shown = options
                .iter()
                .find(|(v, _)| v == value)
                .map_or(value.as_str(), |(_, l)| l.as_str());
            let mut rect = None;
            ui.horizontal(|ui| {
                ui.label(RichText::new(label).size(16.0));
                let inner = egui::ComboBox::from_id_salt(egui_id(id))
                    .selected_text(shown)
                    .show_ui(ui, |ui| {
                        for (v, l) in options {
                            if ui.selectable_label(v == value, l).clicked() {
                                d.events.push(UiEvent::Choose(id.clone(), v.clone()));
                            }
                        }
                    });
                ring(ui, &inner.response, id, focus);
                rect = Some(inner.response.rect);
            });
            if let Some(r) = rect {
                d.rects.insert(id.clone(), r);
            }
        }
        Widget::Slider {
            id,
            label,
            min,
            max,
            value,
        } => {
            let mut v = *value;
            let r = ui.add(egui::Slider::new(&mut v, *min..=*max).text(label));
            ring(ui, &r, id, focus);
            if r.changed() && v != *value {
                d.events.push(UiEvent::Slide(id.clone(), v));
            }
            d.rects.insert(id.clone(), r.rect);
        }
        Widget::Thumbnail { name } => {
            if let Some((id, size)) = images(name) {
                ui.image(egui::load::SizedTexture::new(id, size));
            }
        }
        Widget::Progress { label, permille } => {
            ui.add(egui::ProgressBar::new(*permille as f32 / 1000.0).text(label));
        }
        Widget::Row(children) => {
            ui.horizontal_wrapped(|ui| {
                for c in children {
                    draw_widget(ui, c, focus, focus_moved, true, images, d);
                }
            });
        }
        Widget::Group { title, children } => {
            ui.add_space(6.0);
            ui.group(|ui| {
                ui.label(RichText::new(title).size(18.0).strong());
                for c in children {
                    draw_widget(ui, c, focus, focus_moved, compact, images, d);
                }
            });
        }
    }
}

/// Key presses this frame, as model keys. Arrows, Space and Enter are left to a focused text field.
pub fn collect_keys(ctx: &egui::Context) -> Vec<Key> {
    let typing = ctx.egui_wants_keyboard_input();
    let mut out = Vec::new();
    ctx.input_mut(|i| {
        let mut take = |key: EKey, mods: Modifiers, k: Key, even_when_typing: bool| {
            if (even_when_typing || !typing) && i.consume_key(mods, key) {
                out.push(k);
            }
        };
        take(EKey::Tab, Modifiers::SHIFT, Key::BackTab, true);
        take(EKey::Tab, Modifiers::NONE, Key::Tab, true);
        take(EKey::Escape, Modifiers::NONE, Key::Escape, true);
        take(EKey::Enter, Modifiers::NONE, Key::Enter, true);
        take(EKey::F3, Modifiers::NONE, Key::F3, true);
        take(EKey::ArrowUp, Modifiers::NONE, Key::Up, false);
        take(EKey::ArrowDown, Modifiers::NONE, Key::Down, false);
        take(EKey::ArrowLeft, Modifiers::NONE, Key::Left, false);
        take(EKey::ArrowRight, Modifiers::NONE, Key::Right, false);
        take(EKey::Space, Modifiers::NONE, Key::Space, false);
    });
    out
}

/// A column centred in the available space, for menu screens.
pub fn centered_column<R>(
    ui: &mut egui::Ui,
    width: f32,
    add: impl FnOnce(&mut egui::Ui) -> R,
) -> R {
    let avail = ui.available_width();
    let side = ((avail - width) / 2.0).max(0.0);
    ui.horizontal_top(|ui| {
        ui.add_space(side);
        ui.allocate_ui_with_layout(
            egui::vec2(width.min(avail), 0.0),
            Layout::top_down(Align::Min),
            add,
        )
        .inner
    })
    .inner
}

#[cfg(test)]
mod tests {
    use super::*;
    use pg_ui_model::widget::Widget;

    fn raw(events: Vec<egui::Event>) -> egui::RawInput {
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(900.0, 700.0),
            )),
            events,
            ..egui::RawInput::default()
        }
    }

    fn click_at(pos: egui::Pos2, pressed: bool) -> egui::Event {
        egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        }
    }

    fn run_frame(
        ctx: &egui::Context,
        tree: &Tree,
        input: egui::RawInput,
        focus: Option<&str>,
    ) -> Drawn {
        let tracker = FocusTracker::default();
        let mut drawn = Drawn::default();
        let mut out = ctx.run_ui(input, |ui| {
            egui::CentralPanel::default().show(ui, |ui| {
                drawn = draw_tree(ui, tree, focus, &tracker, &mut |_| None);
            });
        });
        out.textures_delta.clear();
        drawn
    }

    fn sample() -> Tree {
        Tree::new(
            "t",
            vec![
                Widget::Heading("Title".into()),
                Widget::button("a.go", "Go"),
                Widget::Toggle {
                    id: "a.flag".into(),
                    label: "Flag".into(),
                    value: false,
                },
            ],
        )
    }

    #[test]
    fn clicking_a_button_and_a_toggle_reports_events_and_disabled_buttons_stay_quiet() {
        let ctx = egui::Context::default();
        let tree = sample();
        let first = run_frame(&ctx, &tree, raw(vec![]), None);
        for id in ["a.go", "a.flag"] {
            let r = first.rects[id];
            let centre = r.center();
            run_frame(
                &ctx,
                &tree,
                raw(vec![egui::Event::PointerMoved(centre)]),
                None,
            );
            run_frame(&ctx, &tree, raw(vec![click_at(centre, true)]), None);
            let released = run_frame(&ctx, &tree, raw(vec![click_at(centre, false)]), None);
            match id {
                "a.go" => assert_eq!(released.events, vec![UiEvent::Click("a.go".into())]),
                _ => assert_eq!(
                    released.events,
                    vec![UiEvent::Toggle("a.flag".into(), true)]
                ),
            }
        }
        // A disabled button cannot be clicked.
        let off = Tree::new("t", vec![Widget::disabled_button("a.off", "Off")]);
        let first = run_frame(&ctx, &off, raw(vec![]), None);
        let centre = first.rects["a.off"].center();
        run_frame(
            &ctx,
            &off,
            raw(vec![egui::Event::PointerMoved(centre)]),
            None,
        );
        run_frame(&ctx, &off, raw(vec![click_at(centre, true)]), None);
        assert!(
            run_frame(&ctx, &off, raw(vec![click_at(centre, false)]), None)
                .events
                .is_empty()
        );
    }

    #[test]
    fn typing_into_a_secret_field_sends_the_real_text_and_the_tree_never_holds_it() {
        let ctx = egui::Context::default();
        let field = |value: &str| {
            Tree::new(
                "t",
                vec![Widget::TextField {
                    id: "ai.key".into(),
                    label: "Key".into(),
                    value: value.into(),
                    secret: true,
                    hint: "paste".into(),
                }],
            )
        };
        let first = run_frame(&ctx, &field(""), raw(vec![]), Some("ai.key"));
        let centre = first.rects["ai.key"].center();
        // Focus it (the model asked for focus), then type.
        let tracker = FocusTracker::default();
        let _ = tracker;
        run_frame(
            &ctx,
            &field(""),
            raw(vec![egui::Event::PointerMoved(centre)]),
            Some("ai.key"),
        );
        run_frame(
            &ctx,
            &field(""),
            raw(vec![click_at(centre, true)]),
            Some("ai.key"),
        );
        run_frame(
            &ctx,
            &field(""),
            raw(vec![click_at(centre, false)]),
            Some("ai.key"),
        );
        let typed = run_frame(
            &ctx,
            &field(""),
            raw(vec![egui::Event::Text("sk-abc".into())]),
            Some("ai.key"),
        );
        let sent: Vec<&UiEvent> = typed
            .events
            .iter()
            .filter(|e| matches!(e, UiEvent::Text(..)))
            .collect();
        assert_eq!(sent, vec![&UiEvent::Text("ai.key".into(), "sk-abc".into())]);
        // The model's copy is dots; typing more extends the real text, not the dots.
        let more = run_frame(
            &ctx,
            &field("\u{2022}\u{2022}\u{2022}\u{2022}\u{2022}\u{2022}"),
            raw(vec![egui::Event::Text("d".into())]),
            Some("ai.key"),
        );
        assert!(
            more.events
                .contains(&UiEvent::Text("ai.key".into(), "sk-abcd".into())),
            "{:?}",
            more.events
        );
    }

    #[test]
    fn keys_are_mapped_and_arrows_leave_text_fields_alone() {
        let ctx = egui::Context::default();
        let press = |k: EKey| egui::Event::Key {
            key: k,
            physical_key: Some(k),
            pressed: true,
            repeat: false,
            modifiers: Modifiers::NONE,
        };
        let mut keys = Vec::new();
        let mut out = ctx.run_ui(
            raw(vec![
                press(EKey::Tab),
                press(EKey::Escape),
                press(EKey::ArrowDown),
                press(EKey::F3),
            ]),
            |ui| {
                keys = collect_keys(ui.ctx());
            },
        );
        out.textures_delta.clear();
        assert_eq!(keys, vec![Key::Tab, Key::Escape, Key::F3, Key::Down]);
    }
}
