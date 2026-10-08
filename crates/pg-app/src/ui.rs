//! Drawing a widget tree with egui, and turning what the player does into model events.
//!
//! The model owns what is on screen and where keyboard focus is; this module only maps each [`Widget`] to
//! the egui widget that looks and behaves like it, and reports clicks, edits and focus changes back as
//! [`UiEvent`]s. Keyboard navigation (Tab, arrows, Enter, Escape) is not done here: [`collect_keys`] turns
//! key presses into model keys, and the model moves focus, which this module then shows.

use egui::{Align, Color32, Id, Key as EKey, Layout, Modifiers, RichText, Stroke};
use pg_host::console::Severity;
use pg_ui_model::layout::{self, DrawerLayout};
use pg_ui_model::types::{Key, UiEvent};
use pg_ui_model::widget::{DrawerItem, Tree, Widget};
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
    draw_widgets(ui, &tree.widgets, focus, tracker, images)
}

/// Draws a list of widgets one under the other.
pub fn draw_widgets(
    ui: &mut egui::Ui,
    widgets: &[Widget],
    focus: Option<&str>,
    tracker: &FocusTracker,
    images: Images<'_>,
) -> Drawn {
    let mut d = Drawn::default();
    let focus_moved = tracker.last.as_deref() != focus;
    for w in widgets {
        draw_widget(ui, w, focus, focus_moved, false, images, &mut d);
    }
    d
}

/// Draws a list of widgets side by side at their natural size (a control strip).
pub fn draw_inline(
    ui: &mut egui::Ui,
    widgets: &[Widget],
    focus: Option<&str>,
    tracker: &FocusTracker,
    images: Images<'_>,
) -> Drawn {
    let mut d = Drawn::default();
    let focus_moved = tracker.last.as_deref() != focus;
    ui.horizontal(|ui| {
        for w in widgets {
            draw_widget(ui, w, focus, focus_moved, true, images, &mut d);
        }
    });
    d
}

fn to_layout(r: egui::Rect) -> layout::Rect {
    layout::Rect::new(r.min.x, r.min.y, r.width(), r.height())
}

const ITEM_HEIGHT: f32 = 32.0;

/// A drawer: a button, and while open a panel placed by [`layout::place`] on the side with the most room,
/// scrolling when its items do not fit. Clicking outside both closes it.
#[allow(clippy::too_many_arguments)]
fn draw_drawer(
    ui: &mut egui::Ui,
    id: &str,
    label: &str,
    open: bool,
    layout: &DrawerLayout,
    align: layout::Align,
    items: &[DrawerItem],
    focus: Option<&str>,
    d: &mut Drawn,
) {
    let b = egui::Button::new(RichText::new(format!("{label}    ")).size(17.0))
        .min_size(egui::vec2(0.0, 34.0));
    let r = ui.add(b);
    // A small triangle on the right: up while open.
    let c = egui::pos2(r.rect.right() - 14.0, r.rect.center().y);
    let tri = if open {
        vec![
            c + egui::vec2(-4.5, 2.5),
            c + egui::vec2(4.5, 2.5),
            c + egui::vec2(0.0, -3.5),
        ]
    } else {
        vec![
            c + egui::vec2(-4.5, -2.5),
            c + egui::vec2(4.5, -2.5),
            c + egui::vec2(0.0, 3.5),
        ]
    };
    ui.painter().add(egui::Shape::convex_polygon(
        tri,
        ui.visuals().text_color(),
        Stroke::NONE,
    ));
    ring(ui, &r, id, focus);
    if r.clicked() {
        d.events.push(UiEvent::Click(id.to_owned()));
    }
    d.rects.insert(id.to_owned(), r.rect);
    if !open || items.is_empty() {
        return;
    }

    let ctx = ui.ctx().clone();
    let frame = egui::Frame::popup(ui.style());
    let margin = frame.total_margin().sum();
    let pad = ui.spacing().button_padding;
    let gap = ui.spacing().item_spacing;
    let font = egui::FontId::proportional(17.0);
    // What the panel wants (the visible part: rows beyond the maximum scroll).
    let (cell_w, cell_h, cols, content) = match layout {
        DrawerLayout::List { max_rows } => {
            let widest = items
                .iter()
                .map(|i| {
                    ctx.fonts_mut(|f| {
                        f.layout_no_wrap(i.label.clone(), font.clone(), Color32::WHITE)
                            .size()
                            .x
                    })
                })
                .fold(0.0_f32, f32::max);
            let w = widest + 2.0 * pad.x + 8.0;
            let rows = items.len().min((*max_rows).max(1) as usize) as f32;
            (
                w,
                ITEM_HEIGHT,
                1,
                (w, rows * ITEM_HEIGHT + (rows - 1.0) * gap.y),
            )
        }
        DrawerLayout::Grid(g) => {
            let (w, h) = g.content_size(items.len());
            (
                g.cell_w as f32,
                g.cell_h as f32,
                g.columns(items.len()),
                (w, h),
            )
        }
    };
    let (gx, gy) = match layout {
        DrawerLayout::List { .. } => (gap.x, gap.y),
        DrawerLayout::Grid(g) => (g.gap as f32, g.gap as f32),
    };
    let p = layout::place(
        to_layout(r.rect),
        to_layout(ctx.content_rect()),
        (content.0 + margin.x, content.1 + margin.y),
        align,
        4.0,
        8.0,
    );
    let inner = egui::vec2(
        (p.rect.w - margin.x).max(1.0),
        (p.rect.h - margin.y).max(1.0),
    );
    let area = egui::Area::new(egui_id(id).with("drawer"))
        .order(egui::Order::Foreground)
        .fixed_pos(egui::pos2(p.rect.x, p.rect.y))
        .show(&ctx, |ui| {
            frame.show(ui, |ui| {
                ui.set_width(inner.x);
                egui::ScrollArea::both()
                    .id_salt(egui_id(id).with("scroll"))
                    .max_width(inner.x)
                    .max_height(inner.y)
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        ui.spacing_mut().item_spacing = egui::vec2(gx, gy);
                        let mut cell = |ui: &mut egui::Ui, i: &DrawerItem| {
                            let b = egui::Button::new(RichText::new(&i.label).size(17.0))
                                .selected(i.selected)
                                .min_size(egui::vec2(cell_w, cell_h));
                            let r = ui.add_sized([cell_w, cell_h], b);
                            ring(ui, &r, &i.id, focus);
                            if r.clicked() {
                                d.events.push(UiEvent::Click(i.id.clone()));
                            }
                            d.rects.insert(i.id.clone(), r.rect);
                        };
                        if cols <= 1 {
                            for i in items {
                                cell(ui, i);
                            }
                        } else {
                            for row in items.chunks(cols) {
                                ui.horizontal(|ui| {
                                    for i in row {
                                        cell(ui, i);
                                    }
                                });
                            }
                        }
                    });
            });
        });
    let panel = area.response.rect;
    let pressed_outside = ctx.input(|i| {
        i.pointer.any_pressed()
            && i.pointer
                .interact_pos()
                .is_some_and(|p| !panel.contains(p) && !r.rect.contains(p))
    });
    if pressed_outside {
        d.events.push(UiEvent::Click(id.to_owned()));
    }
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
        Widget::Drawer {
            id,
            label,
            open,
            layout,
            align,
            items,
        } => draw_drawer(ui, id, label, *open, layout, *align, items, focus, d),
        Widget::Chip {
            id,
            label,
            on,
            tint,
        } => {
            let text = RichText::new(label).size(14.0);
            // A switch that is off is dimmed as well as unpressed, so its state reads at a glance.
            let text = match tint {
                Some(s) => {
                    let c = severity_colors(*s).0;
                    text.color(if *on { c } else { c.gamma_multiply(0.4) })
                }
                None => text,
            };
            let r = ui.add(
                egui::Button::new(text)
                    .selected(*on)
                    .min_size(egui::vec2(0.0, 26.0)),
            );
            ring(ui, &r, id, focus);
            if r.clicked() {
                d.events.push(UiEvent::Toggle(id.clone(), !*on));
            }
            d.rects.insert(id.clone(), r.rect);
        }
        Widget::Log { lines } => {
            egui::ScrollArea::vertical()
                .id_salt("pg-log")
                .max_height(280.0)
                .auto_shrink([false, false])
                .stick_to_bottom(true)
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 2.0;
                    for (sev, text) in lines {
                        let (fg, bg) = severity_colors(*sev);
                        let mut t = RichText::new(text).monospace().size(13.0).color(fg);
                        if let Some(bg) = bg {
                            t = t.background_color(bg);
                        }
                        ui.add(egui::Label::new(t).wrap());
                    }
                });
        }
        Widget::Row(children) => {
            ui.horizontal_wrapped(|ui| {
                for c in children {
                    draw_widget(ui, c, focus, focus_moved, true, images, d);
                }
            });
        }
        Widget::Scroll {
            max_height,
            children,
        } => {
            // A bounded region: as tall as asked (or as tall as the room allows, never under a few rows).
            let h = (*max_height as f32)
                .min(ui.ctx().content_rect().height() * 0.6)
                .max(120.0);
            egui::ScrollArea::vertical()
                .min_scrolled_height(h)
                .max_height(h)
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    for c in children {
                        draw_widget(ui, c, focus, focus_moved, compact, images, d);
                    }
                });
        }
        Widget::Group { title, children } => {
            ui.add_space(6.0);
            ui.group(|ui| {
                // Every group is as wide as the room it is in, so a screen's groups line up.
                ui.set_min_width(ui.available_width());
                if !title.is_empty() {
                    ui.label(RichText::new(title).size(18.0).strong());
                }
                for c in children {
                    draw_widget(ui, c, focus, focus_moved, compact, images, d);
                }
            });
        }
    }
}

/// How a console line of each severity is drawn: text colour and an optional background. Debug is blue, Info
/// white, Warn yellow, Error red; Fatal is black on red.
pub fn severity_colors(s: Severity) -> (Color32, Option<Color32>) {
    match s {
        Severity::Debug => (Color32::from_rgb(100, 165, 255), None),
        Severity::Info => (Color32::WHITE, None),
        Severity::Warn => (Color32::from_rgb(245, 215, 70), None),
        Severity::Error => (Color32::from_rgb(240, 80, 80), None),
        Severity::Fatal => (Color32::BLACK, Some(Color32::from_rgb(225, 45, 45))),
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
        // The backtick opens the console; while a text field has focus it types a backtick instead.
        take(EKey::Backtick, Modifiers::NONE, Key::Console, false);
        // J opens the town journal (not while typing in a field).
        take(EKey::J, Modifiers::NONE, Key::Journal, false);
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

    fn drawer(open: bool, layout: DrawerLayout, n: usize) -> Tree {
        Tree::new(
            "t",
            vec![Widget::Drawer {
                id: "d".into(),
                label: "Speed 3x".into(),
                open,
                layout,
                align: layout::Align::End,
                items: (0..n)
                    .map(|i| DrawerItem {
                        id: format!("d.{i}"),
                        label: format!("Entry number {i}"),
                        selected: i == 1,
                    })
                    .collect(),
            }],
        )
    }

    /// A frame with the drawer's button in the bottom-right corner of a 900x700 screen.
    fn corner_frame(ctx: &egui::Context, tree: &Tree, events: Vec<egui::Event>) -> Drawn {
        let tracker = FocusTracker::default();
        let mut drawn = Drawn::default();
        let mut out = ctx.run_ui(raw(events), |ui| {
            egui::Area::new(Id::new("corner"))
                .anchor(egui::Align2::RIGHT_BOTTOM, egui::vec2(-12.0, -12.0))
                .show(ui.ctx(), |ui| {
                    drawn = draw_inline(ui, &tree.widgets, None, &tracker, &mut |_| None);
                });
        });
        out.textures_delta.clear();
        drawn
    }

    #[test]
    fn a_list_drawer_opens_up_and_left_inside_the_screen_and_sizes_to_its_longest_entry() {
        let ctx = egui::Context::default();
        let tree = drawer(true, DrawerLayout::List { max_rows: 8 }, 4);
        corner_frame(&ctx, &tree, vec![]);
        let d = corner_frame(&ctx, &tree, vec![]);
        let button = d.rects["d"];
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(900.0, 700.0));
        let widths: Vec<f32> = (0..4).map(|i| d.rects[&format!("d.{i}")].width()).collect();
        assert!(
            widths.iter().all(|w| (w - widths[0]).abs() < 0.5),
            "{widths:?}"
        );
        for i in 0..4 {
            let r = d.rects[&format!("d.{i}")];
            assert!(screen.contains_rect(r), "{r:?} inside {screen:?}");
            assert!(r.max.y < button.min.y, "items sit above the button");
        }
        let (a, b) = (d.rects["d.0"], d.rects["d.1"]);
        assert!(a.min.y < b.min.y, "one column, in order");
        assert!(
            (a.max.x - button.max.x).abs() < 20.0,
            "end-aligned with the button"
        );
    }

    #[test]
    fn a_grid_drawer_is_as_big_as_its_items_up_to_the_maximum_and_scrolls_past_it() {
        let ctx = egui::Context::default();
        let grid = |n| {
            drawer(
                true,
                DrawerLayout::Grid(layout::GridSpec {
                    max_cols: 3,
                    max_rows: 2,
                    cell_w: 70,
                    cell_h: 30,
                    gap: 6,
                }),
                n,
            )
        };
        for (n, cols, rows) in [(2, 2, 1), (5, 3, 2), (6, 3, 2)] {
            let tree = grid(n);
            corner_frame(&ctx, &tree, vec![]);
            let d = corner_frame(&ctx, &tree, vec![]);
            let xs: std::collections::BTreeSet<i32> = (0..n)
                .map(|i| d.rects[&format!("d.{i}")].min.x.round() as i32)
                .collect();
            let ys: std::collections::BTreeSet<i32> = (0..n)
                .map(|i| d.rects[&format!("d.{i}")].min.y.round() as i32)
                .collect();
            assert_eq!((xs.len(), ys.len()), (cols, rows), "{n} items");
        }
        // Twenty items: only the maximum rows are visible, the rest scroll.
        let tree = grid(20);
        corner_frame(&ctx, &tree, vec![]);
        let d = corner_frame(&ctx, &tree, vec![]);
        let top = d.rects["d.0"].min.y;
        let visible = (0..20)
            .filter(|i| {
                let r = d.rects[&format!("d.{i}")];
                r.min.y >= top && r.max.y <= top + 2.0 * 30.0 + 6.0 + 1.0
            })
            .count();
        assert_eq!(visible, 6);
    }

    #[test]
    fn clicking_an_item_or_outside_a_drawer_is_reported() {
        let ctx = egui::Context::default();
        let tree = drawer(true, DrawerLayout::List { max_rows: 8 }, 3);
        corner_frame(&ctx, &tree, vec![]);
        let d = corner_frame(&ctx, &tree, vec![]);
        let item = d.rects["d.2"].center();
        corner_frame(&ctx, &tree, vec![egui::Event::PointerMoved(item)]);
        corner_frame(&ctx, &tree, vec![click_at(item, true)]);
        let up = corner_frame(&ctx, &tree, vec![click_at(item, false)]);
        assert_eq!(up.events, vec![UiEvent::Click("d.2".into())]);
        // A press far from both the button and the panel asks to toggle the drawer closed.
        let away = egui::pos2(40.0, 40.0);
        corner_frame(&ctx, &tree, vec![egui::Event::PointerMoved(away)]);
        let down = corner_frame(&ctx, &tree, vec![click_at(away, true)]);
        assert_eq!(down.events, vec![UiEvent::Click("d".into())]);
        // A closed drawer draws only its button.
        let closed = drawer(false, DrawerLayout::List { max_rows: 8 }, 3);
        let d = corner_frame(&ctx, &closed, vec![]);
        assert!(d.rects.contains_key("d") && !d.rects.contains_key("d.0"));
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
    fn console_lines_are_coloured_by_severity_as_specified() {
        // Debug blue, Info white, Warn yellow, Error red, Fatal black text on a red background.
        let (debug, none) = severity_colors(Severity::Debug);
        assert!(none.is_none() && debug.b() > debug.r() && debug.b() > 200);
        assert_eq!(severity_colors(Severity::Info), (Color32::WHITE, None));
        let (warn, none) = severity_colors(Severity::Warn);
        assert!(none.is_none() && warn.r() > 200 && warn.g() > 180 && warn.b() < 120);
        let (error, none) = severity_colors(Severity::Error);
        assert!(none.is_none() && error.r() > 200 && error.g() < 120 && error.b() < 120);
        let (fatal, background) = severity_colors(Severity::Fatal);
        assert_eq!(fatal, Color32::BLACK);
        let bg = background.expect("fatal has a background");
        assert!(bg.r() > 200 && bg.g() < 90 && bg.b() < 90);
    }

    #[test]
    fn a_type_chip_reports_a_toggle_and_a_log_draws_every_line() {
        let ctx = egui::Context::default();
        let tree = Tree::new(
            "console",
            vec![
                Widget::Chip {
                    id: "console.type.warn".into(),
                    label: "Warn".into(),
                    on: true,
                    tint: Some(Severity::Warn),
                },
                Widget::Log {
                    lines: vec![
                        (Severity::Info, "[Info]: one".into()),
                        (Severity::Fatal, "[Fatal]: two".into()),
                    ],
                },
            ],
        );
        let first = run_frame(&ctx, &tree, raw(vec![]), None);
        let centre = first.rects["console.type.warn"].center();
        run_frame(
            &ctx,
            &tree,
            raw(vec![egui::Event::PointerMoved(centre)]),
            None,
        );
        run_frame(&ctx, &tree, raw(vec![click_at(centre, true)]), None);
        let up = run_frame(&ctx, &tree, raw(vec![click_at(centre, false)]), None);
        assert_eq!(
            up.events,
            vec![UiEvent::Toggle("console.type.warn".into(), false)],
            "a pressed chip turns off"
        );
        // The same widgets under a screen-sized frame tessellate without trouble.
        let mut out = ctx.run_ui(raw(vec![]), |ui| {
            egui::CentralPanel::default().show(ui, |ui| {
                draw_tree(ui, &tree, None, &FocusTracker::default(), &mut |_| None);
            });
        });
        out.textures_delta.clear();
        assert!(!out.shapes.is_empty());
    }

    #[test]
    fn the_backtick_is_the_console_key_unless_a_text_field_is_being_typed_in() {
        let ctx = egui::Context::default();
        let press = |k: EKey| egui::Event::Key {
            key: k,
            physical_key: Some(k),
            pressed: true,
            repeat: false,
            modifiers: Modifiers::NONE,
        };
        let mut keys = Vec::new();
        let mut out = ctx.run_ui(raw(vec![press(EKey::Backtick)]), |ui| {
            keys = collect_keys(ui.ctx());
        });
        out.textures_delta.clear();
        assert_eq!(keys, vec![Key::Console]);
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
