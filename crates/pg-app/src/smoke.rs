//! `pg-app --smoke`: a scripted session that exercises everything except the window itself.
//!
//! It boots the app on a throwaway data folder with real files, a real clock and the real UI code under a
//! headless egui context, and walks the main flow: launch to the main menu, new world, play, pause menu,
//! save, back to the menu, continue, quit. Every screen is built and tessellated, so a widget that cannot
//! be drawn fails here rather than on a player's machine. CI runs it on every operating system.

use crate::app::{real_services, App};
use pg_content::ContentSet;
use pg_host::{MemSecretStore, SecretStore};
use pg_runtime::app::AppController;
use pg_ui_model::types::UiEvent;
use pg_ui_model::{Key, Screen};
use std::sync::Arc;
use std::time::{Duration, Instant};

struct Harness {
    ctx: egui::Context,
    started: Instant,
    shapes: usize,
}

impl Harness {
    fn frame(&mut self, app: &mut App) {
        let raw = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1280.0, 800.0),
            )),
            time: Some(self.started.elapsed().as_secs_f64()),
            ..egui::RawInput::default()
        };
        let mut out = self.ctx.run_ui(raw, |ui| app.draw(ui));
        let prims = self.ctx.tessellate(out.shapes, out.pixels_per_point);
        // There is no GPU to upload textures to; they only need to be acknowledged.
        out.textures_delta.clear();
        self.shapes += prims.len();
    }

    /// Runs frames for `ms` of real time (the simulation runs on its own thread).
    fn run_for(&mut self, app: &mut App, ms: u64) {
        let end = Instant::now() + Duration::from_millis(ms);
        while Instant::now() < end {
            self.frame(app);
            std::thread::sleep(Duration::from_millis(8));
        }
    }

    fn until(
        &mut self,
        app: &mut App,
        what: &str,
        mut done: impl FnMut(&App) -> bool,
    ) -> Result<(), String> {
        let end = Instant::now() + Duration::from_secs(10);
        while Instant::now() < end {
            self.frame(app);
            if done(app) {
                return Ok(());
            }
            std::thread::sleep(Duration::from_millis(8));
        }
        Err(format!(
            "timed out waiting for {what}; the screen is {:?}",
            app.model.screen()
        ))
    }
}

fn expect(cond: bool, what: &str) -> Result<(), String> {
    if cond {
        println!("  ok  {what}");
        Ok(())
    } else {
        Err(format!("FAILED: {what}"))
    }
}

pub fn run(content: Arc<ContentSet>) -> Result<(), String> {
    let dir = std::env::temp_dir().join(format!("pg-smoke-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let mut services = real_services(&dir)?;
    // The smoke test must not touch the player's credential store.
    let secrets: Arc<dyn SecretStore> = Arc::new(MemSecretStore::new());
    services.secrets = secrets;
    println!("smoke test, data in {}", dir.display());

    let mut h = Harness {
        ctx: egui::Context::default(),
        started: Instant::now(),
        shapes: 0,
    };
    let mut app = App::new(AppController::new(services, Some(Arc::clone(&content))));
    h.frame(&mut app);
    expect(
        matches!(app.model.screen(), Screen::MainMenu),
        "launch shows the main menu",
    )?;

    app.dispatch(UiEvent::Click("main.options".into()));
    h.frame(&mut app);
    app.dispatch(UiEvent::Click("options.ai".into()));
    h.frame(&mut app);
    app.dispatch(UiEvent::Key(Key::Escape));
    app.dispatch(UiEvent::Key(Key::Escape));
    expect(
        matches!(app.model.screen(), Screen::MainMenu),
        "options and AI settings open and close",
    )?;

    app.dispatch(UiEvent::Click("main.new".into()));
    h.frame(&mut app);
    app.dispatch(UiEvent::Text("new.name".into(), "Smoke Town".into()));
    app.dispatch(UiEvent::Text("new.seed".into(), "smoke".into()));
    app.dispatch(UiEvent::Click("new.create".into()));
    h.until(&mut app, "the new world to open", |a| {
        matches!(a.model.screen(), Screen::InGame) && a.model.hud().pawns > 0
    })?;
    expect(
        app.controller.world_open(),
        "a world opens and has residents",
    )?;

    app.dispatch(UiEvent::Click("hud.speed.27x".into()));
    h.until(&mut app, "time to pass", |a| a.model.hud().tick > 200)?;
    expect(true, "time passes in the running world")?;
    app.dispatch(UiEvent::Key(Key::Escape));
    app.dispatch(UiEvent::Click("pause.options".into()));
    app.dispatch(UiEvent::Toggle("setting.dev.enabled".into(), true));
    app.dispatch(UiEvent::Key(Key::Escape));
    app.dispatch(UiEvent::Key(Key::Escape));
    app.dispatch(UiEvent::Key(Key::F3));
    expect(
        app.model.overlay().visible(),
        "developer mode opens the overlay",
    )?;
    h.run_for(&mut app, 150);
    app.dispatch(UiEvent::Key(Key::F3));
    app.dispatch(UiEvent::Key(Key::Escape));
    h.frame(&mut app);
    expect(
        matches!(app.model.screen(), Screen::Pause),
        "Escape opens the pause menu",
    )?;
    app.dispatch(UiEvent::Click("pause.save".into()));
    h.until(&mut app, "the save", |a| {
        a.model.hud().status.to_lowercase().contains("saved")
    })?;
    app.dispatch(UiEvent::Click("pause.menu".into()));
    h.frame(&mut app);
    expect(
        matches!(app.model.screen(), Screen::MainMenu),
        "the pause menu returns to the main menu",
    )?;
    expect(
        app.model.worlds().iter().any(|w| w.name == "Smoke Town"),
        "the world is in the saved list",
    )?;

    app.dispatch(UiEvent::Click("main.saved".into()));
    h.frame(&mut app);
    app.dispatch(UiEvent::Key(Key::Escape));
    app.dispatch(UiEvent::Click("main.continue".into()));
    h.until(&mut app, "the world to load again", |a| {
        matches!(a.model.screen(), Screen::InGame) && a.model.hud().pawns > 0
    })?;
    expect(
        app.model.hud().tick > 0,
        "Continue reopens the saved world where it was",
    )?;

    app.dispatch(UiEvent::CloseRequested);
    expect(
        app.wants_exit() && !app.controller.world_open(),
        "closing the window saves and stops the world",
    )?;
    expect(
        h.shapes > 100,
        &format!("{} shapes were tessellated across all screens", h.shapes),
    )?;
    let played = app.model.hud().tick;
    drop(app);

    // Relaunch: a brand-new app on the same folder offers the world and continues it.
    let mut services = real_services(&dir)?;
    let secrets: Arc<dyn SecretStore> = Arc::new(MemSecretStore::new());
    services.secrets = secrets;
    let mut app = App::new(AppController::new(services, Some(content)));
    h.frame(&mut app);
    expect(
        matches!(app.model.screen(), Screen::MainMenu)
            && app.model.worlds().iter().any(|w| w.name == "Smoke Town"),
        "a relaunch shows the main menu with the saved world",
    )?;
    app.dispatch(UiEvent::Click("main.continue".into()));
    h.until(&mut app, "the relaunched world to load", |a| {
        matches!(a.model.screen(), Screen::InGame) && a.model.hud().pawns > 0
    })?;
    expect(
        app.model.hud().tick >= played && !app.model.hud().running,
        "Continue after a relaunch resumes at the saved moment, paused",
    )?;
    app.dispatch(UiEvent::CloseRequested);
    drop(app);
    let _ = std::fs::remove_dir_all(&dir);
    println!("smoke test passed");
    Ok(())
}
