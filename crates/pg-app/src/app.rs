//! The application: the UI model, the controller behind it, and the per-frame glue. Window-system free, so
//! it runs under a headless egui context for the smoke test.

use crate::game_view::GameView;
use crate::os;
use crate::ui::{
    centered_column, collect_keys, draw_inline, draw_tree, draw_widgets, Drawn, FocusTracker,
};
use egui::{Color32, Frame};
use pg_ai::login::CLIENT_ID;
use pg_ai::provider::allowed_hosts;
use pg_content::{load_pack, ComponentRegistry, ContentSet, DirPack, Limits};
use pg_host::{AllowListNet, RedactingLog, SecretStore, StderrLog};
use pg_host_os::{FsStorage, KeyringSecretStore, SystemClock, UreqNet};
use pg_runtime::app::{AppController, AppServices};
use pg_ui_model::types::UiEvent;
use pg_ui_model::{AppEffect, AppModel, Screen};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

/// Loads the shipped base pack, then any extra pack folders (developer use: `--pack <dir>`).
pub fn load_content(base: &Path, extra: &[PathBuf]) -> Result<Arc<ContentSet>, String> {
    let mut packs = Vec::new();
    for dir in std::iter::once(base).chain(extra.iter().map(PathBuf::as_path)) {
        match load_pack(&DirPack::new(dir), &Limits::default()) {
            Ok(p) => packs.push(p),
            Err(r) => return Err(format!("cannot load pack '{}':\n{r}", dir.display())),
        }
    }
    ContentSet::build(packs, ComponentRegistry::builtin())
        .map(Arc::new)
        .map_err(|r| format!("the content did not build:\n{r}"))
}

/// The real services: files under `data_dir`, the OS credential store, HTTPS limited to the providers.
pub fn real_services(data_dir: &Path) -> Result<AppServices, String> {
    let storage = FsStorage::new(data_dir)
        .map_err(|e| format!("cannot open the data folder {}: {e}", data_dir.display()))?;
    let secrets: Arc<dyn SecretStore> = Arc::new(KeyringSecretStore::new());
    Ok(AppServices {
        storage: Arc::new(storage),
        clock: Arc::new(SystemClock::new()),
        secrets,
        net: Arc::new(AllowListNet::new(UreqNet, &allowed_hosts())),
        log: Arc::new(RedactingLog::new(StderrLog)),
        open_url: Arc::new(os::open_url),
        reveal_path: Arc::new(os::reveal),
        sleep: Arc::new(std::thread::sleep),
        data_dir: Some(data_dir.to_path_buf()),
        app_version: env!("CARGO_PKG_VERSION").to_owned(),
        client_id: std::env::var("PG_PLAYER2_CLIENT_ID").unwrap_or_else(|_| CLIENT_ID.to_owned()),
    })
}

pub struct App {
    pub controller: AppController,
    pub model: AppModel,
    view: GameView,
    tracker: FocusTracker,
    overlay_tracker: FocusTracker,
    styled: bool,
    images: HashMap<String, Option<egui::TextureHandle>>,
}

impl App {
    /// Boots the controller and the model.
    pub fn new(mut controller: AppController) -> App {
        let mut model = AppModel::new();
        let boot = controller.boot();
        model.update(boot);
        App {
            controller,
            model,
            view: GameView::default(),
            tracker: FocusTracker::default(),
            overlay_tracker: FocusTracker::default(),
            styled: false,
            images: HashMap::new(),
        }
    }

    /// Feeds an event to the model and performs the effects it asks for, and theirs, until done.
    pub fn dispatch(&mut self, ev: UiEvent) {
        let mut queue = vec![ev];
        while let Some(e) = queue.pop() {
            let was_in_world = self.model.in_world();
            for fx in self.model.update(e) {
                if matches!(fx, AppEffect::LeaveWorld) {
                    self.view.reset();
                }
                queue.extend(self.controller.perform(fx).into_iter().rev());
            }
            if was_in_world != self.model.in_world() {
                self.view.reset();
            }
        }
    }

    /// Developer aid (`--demo <screen>`): drives the app to a screen so it can be looked at or captured
    /// without clicking there by hand. Names: new, options, ai, saved, game, drawer, focus, pause, overlay.
    pub fn apply_demo(&mut self, name: &str) {
        let click = |a: &mut App, id: &str| a.dispatch(UiEvent::Click(id.to_owned()));
        let start_world = |a: &mut App| {
            click(a, "main.new");
            a.dispatch(UiEvent::Text("new.name".into(), "Demo Town".into()));
            a.dispatch(UiEvent::Text("new.seed".into(), "demo".into()));
            a.dispatch(UiEvent::Choose("new.size".into(), "medium".into()));
            a.dispatch(UiEvent::Slide("new.residents".into(), 14));
            click(a, "new.create");
            click(a, "hud.speed.9x");
        };
        match name {
            "new" => click(self, "main.new"),
            "options" => click(self, "main.options"),
            "ai" => {
                click(self, "main.options");
                click(self, "options.ai");
                self.dispatch(UiEvent::Choose("ai.provider".into(), "player2".into()));
            }
            "saved" => {
                start_world(self);
                self.dispatch(UiEvent::Key(pg_ui_model::Key::Escape));
                click(self, "pause.menu");
                click(self, "main.saved");
                click(self, "saved.row.demo-town");
            }
            "game" => start_world(self),
            "drawer" => {
                start_world(self);
                click(self, "hud.speed");
            }
            "focus" => {
                start_world(self);
                self.dispatch(UiEvent::FocusLost);
            }
            "pause" => {
                start_world(self);
                self.dispatch(UiEvent::Key(pg_ui_model::Key::Escape));
            }
            "overlay" => {
                start_world(self);
                click(self, "hud.speed.27x");
                self.dispatch(UiEvent::Key(pg_ui_model::Key::Escape));
                click(self, "pause.options");
                self.dispatch(UiEvent::Toggle("setting.dev.enabled".into(), true));
                self.dispatch(UiEvent::Key(pg_ui_model::Key::Escape));
                self.dispatch(UiEvent::Key(pg_ui_model::Key::Escape));
                self.dispatch(UiEvent::Key(pg_ui_model::Key::F3));
            }
            other => eprintln!("unknown demo '{other}'"),
        }
    }

    pub fn wants_exit(&self) -> bool {
        self.controller.quit_requested()
    }

    /// Saves and stops everything (call before the process exits).
    pub fn shutdown(&mut self) {
        self.controller.shutdown();
    }

    /// One frame: background results in, input in, the screen out. Returns after dispatching what the
    /// player did.
    pub fn draw(&mut self, ui: &mut egui::Ui) {
        for e in self.controller.poll() {
            self.dispatch(e);
        }
        for k in collect_keys(ui.ctx()) {
            self.dispatch(UiEvent::Key(k));
        }
        if self.model.overlay().visible() {
            let data = self.controller.overlay_data();
            self.model.set_overlay_data(data);
        }
        let ctx = ui.ctx().clone();
        if !self.styled {
            self.styled = true;
            ctx.all_styles_mut(|s| {
                s.spacing.item_spacing = egui::vec2(10.0, 9.0);
                s.spacing.button_padding = egui::vec2(10.0, 5.0);
            });
        }
        let scale = self
            .controller
            .setting_int("ui.scale_percent")
            .unwrap_or(100) as f32
            / 100.0;
        ctx.set_zoom_factor(scale);

        let t = |k: &str, a: &[(&str, &str)]| self.controller.text(k, a);
        let tree = self.model.tree(&t);
        let hud = self.model.hud_parts(&t);
        let overlay = self.model.overlay_tree(&t);
        let focus = self.model.focus().map(str::to_owned);
        let screen = self.model.screen().clone();
        let snapshot = self.controller.snapshot();
        let controller = &self.controller;
        let image_cache = &mut self.images;
        let image_ctx = ctx.clone();
        let mut images = |name: &str| {
            let entry = image_cache.entry(name.to_owned()).or_insert_with(|| {
                controller.thumbnail_image(name).map(|(w, h, rgb)| {
                    image_ctx.load_texture(
                        name,
                        egui::ColorImage::from_rgb([w as usize, h as usize], &rgb),
                        egui::TextureOptions::NEAREST,
                    )
                })
            });
            entry.as_ref().map(|t| (t.id(), t.size_vec2() * 3.0))
        };

        let mut events: Vec<UiEvent> = Vec::new();
        match screen {
            Screen::InGame | Screen::Pause => {
                let in_game = matches!(screen, Screen::InGame);
                let f = if in_game { focus.as_deref() } else { None };
                egui::Panel::top("hud").show(ui, |ui| {
                    let drawn = draw_widgets(ui, &hud.top, f, &self.tracker, &mut images);
                    events.extend(drawn.events);
                });
                egui::CentralPanel::default()
                    .frame(Frame::new())
                    .show(ui, |ui| match &snapshot {
                        Some(s) => self.view.show(ui, s),
                        None => {
                            ui.centered_and_justified(|ui| ui.label("…"));
                        }
                    });
                // The simulation controls, bottom right; shown but dead while the pause menu is up.
                egui::Area::new(egui::Id::new("hud-controls"))
                    .anchor(egui::Align2::RIGHT_BOTTOM, egui::vec2(-12.0, -12.0))
                    .show(&ctx, |ui| {
                        egui::Frame::window(ui.style()).show(ui, |ui| {
                            ui.add_enabled_ui(in_game, |ui| {
                                let drawn =
                                    draw_inline(ui, &hud.controls, f, &self.tracker, &mut images);
                                events.extend(drawn.events);
                            });
                        });
                    });
                if in_game && !hud.dialog.is_empty() {
                    egui::Window::new("welcome-back")
                        .title_bar(false)
                        .resizable(false)
                        .collapsible(false)
                        .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
                        .show(&ctx, |ui| {
                            let drawn =
                                draw_widgets(ui, &hud.dialog, f, &self.tracker, &mut images);
                            events.extend(drawn.events);
                        });
                }
                if !in_game {
                    egui::Window::new("pause")
                        .title_bar(false)
                        .resizable(false)
                        .collapsible(false)
                        .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
                        .show(&ctx, |ui| {
                            let drawn =
                                draw_tree(ui, &tree, focus.as_deref(), &self.tracker, &mut images);
                            events.extend(drawn.events);
                        });
                }
            }
            _ => {
                egui::CentralPanel::default()
                    .frame(Frame::new().fill(Color32::from_rgb(26, 30, 38)))
                    .show(ui, |ui| {
                        egui::ScrollArea::vertical().show(ui, |ui| {
                            ui.add_space(36.0);
                            centered_column(ui, 420.0, |ui| {
                                let drawn: Drawn = draw_tree(
                                    ui,
                                    &tree,
                                    focus.as_deref(),
                                    &self.tracker,
                                    &mut images,
                                );
                                events.extend(drawn.events);
                            });
                        });
                    });
            }
        }
        if let Some(o) = overlay {
            egui::Window::new("Developer overlay")
                .default_width(420.0)
                .anchor(egui::Align2::RIGHT_TOP, egui::vec2(-8.0, 40.0))
                .show(&ctx, |ui| {
                    egui::ScrollArea::vertical()
                        .max_height(480.0)
                        .show(ui, |ui| {
                            let drawn = draw_tree(ui, &o, None, &self.overlay_tracker, &mut images);
                            events.extend(drawn.events);
                        });
                });
        }
        self.tracker.update(self.model.focus());
        for e in events {
            self.dispatch(e);
        }
        ctx.request_repaint_after(Duration::from_millis(33));
    }
}
