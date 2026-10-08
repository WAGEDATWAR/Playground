//! The app controller (milestone 0.10): performs the effects the UI model asks for, against the real
//! services, and answers with events.
//!
//! The window shell is thin: it feeds input into a `pg_ui_model::AppModel`, hands the model's effects to
//! [`AppController::perform`], feeds the answers back, and calls [`AppController::poll`] every frame for
//! what finished in the background (sign-in, connection tests, the running world). All the logic that
//! touches storage, the credential store, the network and the simulation lives here, behind the host
//! traits, so a full session (create, play, save, quit, relaunch, continue) is tested without a window.

use crate::control::{RunEvent, RunState, Speed};
use crate::keyframes::SimFactory;
use crate::pool::WorkerPool;
use crate::profile::Profiler;
use crate::session::Session;
use crate::settings::{device_registry, DeviceFile};
use crate::sim_loop::{Control, LoopConfig, LoopEvent, LoopServices, SimLoop};
use crate::snapshot::RenderSnapshot;
use crate::thumbnail::thumbnail_png;
use pg_ai::client::{AiClient, ClientConfig};
use pg_ai::login::{is_safe_verification_url, store_key, DeviceLoginSession, SessionState};
use pg_ai::provider::{AuthMethod, Provider};
use pg_ai::settings::{validate_model_id, AiSettings, KeyManager};
use pg_content::schema::FieldSchema;
use pg_content::settings::{SettingsRegistry, SettingsValues};
use pg_content::ContentSet;
use pg_core::canon::Canon;
use pg_core::commands::Command;
use pg_core::id::{EntityId, Kind};
use pg_core::input::SimInput;
use pg_core::world::WorldState;
use pg_host::{CancelToken, Clock, LogSink, Net, SecretStore, Storage};
use pg_persist::compat::ContentRefRecord;
use pg_persist::export::{commit_import, export_world, import_world, ImportOptions};
use pg_persist::store::{valid_world_id, LoadOptions, Recovery, SlotStore};
use pg_script::host::ScriptMeter;
use pg_ui_model::overlay::{EventRow, OverlayData, PackRow, ReasonRow, ScriptRow, SystemRow};
use pg_ui_model::types::*;
use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;
use std::time::Duration;

/// Where exports are written and imports are read, relative to the data folder.
pub const EXPORT_DIR: &str = "exports/";
pub const IMPORT_DIR: &str = "imports/";

/// What the controller needs from the machine. The real app fills these from `pg-host-os`; tests use the
/// in-memory doubles.
pub struct AppServices {
    pub storage: Arc<dyn Storage>,
    pub clock: Arc<dyn Clock>,
    pub secrets: Arc<dyn SecretStore>,
    /// Already restricted to the provider hosts.
    pub net: Arc<dyn Net>,
    pub log: Arc<dyn LogSink>,
    /// Opens a link in the player's browser.
    pub open_url: Arc<dyn Fn(&str) + Send + Sync>,
    /// Shows a file or folder in the system file manager.
    pub reveal_path: Arc<dyn Fn(&std::path::Path) + Send + Sync>,
    /// Sleeps the sign-in worker between polls (tests advance a fixed clock instead).
    pub sleep: Arc<dyn Fn(Duration) + Send + Sync>,
    /// The folder the storage lives in, when it is a real folder (for "show in folder").
    pub data_dir: Option<PathBuf>,
    pub app_version: String,
    /// The id this game is registered under with Player2.
    pub client_id: String,
    /// The system's open and save dialogs; `None` where there are none (tests, headless runs), and the
    /// folder-based import and export are used instead.
    pub dialogs: Option<Arc<dyn pg_host::Dialogs>>,
}

struct ActiveWorld {
    id: String,
    name: String,
    session: Session,
}

pub struct AppController {
    svc: AppServices,
    content: Option<Arc<ContentSet>>,
    registry: SettingsRegistry,
    values: SettingsValues,
    pool: Arc<WorkerPool>,
    tx: Sender<UiEvent>,
    rx: Receiver<UiEvent>,
    world: Option<ActiveWorld>,
    login_cancel: Option<CancelToken>,
    quit: bool,
    profiler: Profiler,
    meter: Arc<ScriptMeter>,
    speed: Speed,
    last_hud: Option<HudInfo>,
    status: Option<(String, Duration)>,
    /// Developer option: re-simulate every keyframe span on this many threads and compare (S-030).
    shadow_threads: Option<usize>,
    shadow_ok: u64,
    shadow_bad: u64,
    /// The developer console's log (milestone 1.3a): app messages, the simulation's events, pack prints.
    console: pg_host::Console,
    /// Lines for conversations the focused resident can hear (milestone 1.5).
    dialogue: crate::dialogue::DialogueHook,
    /// What happened when the packs were chosen at launch (packs left out and why).
    mods_notes: Vec<String>,
    /// The resident the inspector looks at, shared with the simulation loop.
    focus: Arc<std::sync::Mutex<Option<pg_core::id::EntityId>>>,
}

fn slug(name: &str) -> String {
    let mut s: String = name
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    while s.contains("--") {
        s = s.replace("--", "-");
    }
    let s = s.trim_matches('-');
    let s: String = s.chars().take(32).collect();
    if s.is_empty() {
        "world".to_owned()
    } else {
        s
    }
}

fn refs_of(content: &Option<Arc<ContentSet>>) -> Vec<ContentRefRecord> {
    content.as_ref().map_or_else(Vec::new, |c| {
        c.refs()
            .into_iter()
            .map(|r| ContentRefRecord {
                pack_id: r.pack_id.to_string(),
                version: r.version.to_string(),
                hash: r.hash,
            })
            .collect()
    })
}

fn canon_value(c: &Canon) -> Option<SettingValue> {
    match c {
        Canon::Bool(b) => Some(SettingValue::Bool(*b)),
        Canon::Int(i) => i64::try_from(*i).ok().map(SettingValue::Int),
        Canon::Str(s) => Some(SettingValue::Text(s.clone())),
        _ => None,
    }
}

fn to_canon(v: &SettingValue) -> Canon {
    match v {
        SettingValue::Bool(b) => Canon::Bool(*b),
        SettingValue::Int(i) => Canon::Int(i128::from(*i)),
        SettingValue::Text(s) => Canon::Str(s.clone()),
    }
}

impl AppController {
    pub fn new(mut svc: AppServices, content: Option<Arc<ContentSet>>) -> AppController {
        let console = pg_host::Console::new();
        // Everything the app and the simulation loop log also shows in the console.
        svc.log = Arc::new(pg_host::ConsoleLog::new(svc.log, console.clone()));
        let registry = device_registry();
        let (values, _) = DeviceFile::new(svc.storage.as_ref(), registry.clone()).load();
        let (tx, rx) = channel();
        let pool = Arc::new(WorkerPool::new(2));
        let dialogue = crate::dialogue::DialogueHook::new(crate::dialogue::DialogueService::new(
            Arc::new(AiClient::new(
                Arc::clone(&svc.net),
                Arc::clone(&svc.secrets),
                Arc::clone(&svc.clock),
                Arc::clone(&svc.log),
                ClientConfig::default(),
            )),
            Arc::clone(&pool),
            crate::dialogue::DialogueConfig::default(),
        ));
        let c = AppController {
            dialogue,
            mods_notes: Vec::new(),
            focus: Arc::new(std::sync::Mutex::new(None)),
            svc,
            content,
            registry,
            values,
            pool,
            tx,
            rx,
            world: None,
            login_cancel: None,
            quit: false,
            profiler: Profiler::new(),
            meter: Arc::new(ScriptMeter::new()),
            speed: Speed::Normal,
            last_hud: None,
            status: None,
            shadow_threads: None,
            shadow_ok: 0,
            shadow_bad: 0,
            console,
        };
        c.sync_console();
        c
    }

    /// Developer mode decides whether Debug entries are produced at all.
    fn sync_console(&self) {
        self.console
            .set_wants_debug(self.values.bool("dev.enabled").unwrap_or(false));
    }

    /// The developer console's log.
    pub fn console(&self) -> &pg_host::Console {
        &self.console
    }

    /// Console entries newer than `seq` (the model asks for what it has not seen).
    pub fn console_since(&self, seq: u64) -> Vec<pg_host::Entry> {
        self.console.since(seq)
    }

    /// Turns shadow determinism verification on for worlds opened from now on (developer option).
    pub fn set_shadow_threads(&mut self, threads: Option<usize>) {
        self.shadow_threads = threads;
    }

    pub fn quit_requested(&self) -> bool {
        self.quit
    }

    pub fn world_open(&self) -> bool {
        self.world.is_some()
    }

    pub fn setting_int(&self, id: &str) -> Option<i64> {
        self.values.int(id)
    }

    pub fn setting_bool(&self, id: &str) -> Option<bool> {
        self.values.bool(id)
    }

    pub fn setting_text(&self, id: &str) -> Option<&str> {
        self.values.text(id)
    }

    fn locale(&self) -> String {
        self.values.text("ui.language").unwrap_or("en").to_owned()
    }

    /// A string from the string table (the base pack's `strings/en.json` and any pack's), `[key]` if missing.
    pub fn text(&self, key: &str, args: &[(&str, &str)]) -> String {
        match &self.content {
            Some(c) => c.strings().text(&self.locale(), key, args),
            None => key.to_owned(),
        }
    }

    // ---- boot ----------------------------------------------------------------------------------------------

    pub fn boot(&mut self) -> UiEvent {
        let (values, report) =
            DeviceFile::new(self.svc.storage.as_ref(), self.registry.clone()).load();
        self.values = values;
        self.sync_console();
        let warnings: Vec<String> = report.issues().iter().map(|i| i.message.clone()).collect();
        UiEvent::Booted(Box::new(BootInfo {
            worlds: self.list_worlds(),
            settings: self.setting_items(),
            ai: self.ai_state(),
            crash_bundle: self.pending_crash_bundle(),
            settings_warnings: warnings,
        }))
    }

    fn pending_crash_bundle(&self) -> Option<String> {
        let bundles = self.svc.storage.list("crash/").ok()?;
        let dismissed: Vec<String> = self
            .svc
            .storage
            .list("crash/dismissed/")
            .ok()?
            .into_iter()
            .map(|b| b.name)
            .collect();
        bundles.into_iter().map(|b| b.name).rfind(|n| {
            n.ends_with(".pgbundle")
                && !dismissed
                    .iter()
                    .any(|d| d.ends_with(n.trim_start_matches("crash/")))
        })
    }

    fn setting_items(&self) -> Vec<SettingItem> {
        self.registry
            .iter()
            .filter_map(|def| {
                let kind = match &def.field.schema {
                    FieldSchema::Bool => SettingKind::Bool,
                    FieldSchema::Int { min, max } => SettingKind::Int {
                        min: *min,
                        max: *max,
                    },
                    FieldSchema::Enum(options) => SettingKind::Enum(
                        options
                            .iter()
                            .map(|o| (o.clone(), format!("{}.{o}", def.label_key)))
                            .collect(),
                    ),
                    FieldSchema::Text { max_len } => SettingKind::Text { max_len: *max_len },
                    _ => return None,
                };
                let value = self.values.get(&def.id).and_then(canon_value)?;
                Some(SettingItem {
                    id: def.id.clone(),
                    label_key: def.label_key.clone(),
                    kind,
                    value,
                    restart_required: def.restart_required,
                })
            })
            .collect()
    }

    fn ai_settings(&self) -> AiSettings {
        AiSettings::from_values(&self.values)
    }

    fn ai_state(&self) -> AiState {
        let keys = KeyManager::new(self.svc.secrets.as_ref());
        let current = self.ai_settings();
        AiState {
            providers: Provider::ALL
                .iter()
                .map(|p| ProviderInfo {
                    id: p.id().to_owned(),
                    label: p.display_name().to_owned(),
                    auth: match p.auth_method() {
                        AuthMethod::PastedKey => Auth::PasteKey,
                        AuthMethod::DeviceLogin => Auth::DeviceCode,
                    },
                    has_key: keys.has_key(*p),
                    model_fixed: p.chooses_own_model(),
                    model: if *p == current.provider {
                        current.effective_model()
                    } else {
                        p.recommended_model().to_owned()
                    },
                    recommended_model: p.recommended_model().to_owned(),
                })
                .collect(),
            selected: current.provider.id().to_owned(),
            persistent_keys: keys.is_persistent(),
        }
    }

    // ---- worlds on disk ------------------------------------------------------------------------------------

    fn store(&self) -> SlotStore<'_> {
        SlotStore::new(self.svc.storage.as_ref())
    }

    pub fn list_worlds(&self) -> Vec<WorldEntry> {
        let store = self.store();
        let mut out: Vec<WorldEntry> = store
            .list_worlds()
            .unwrap_or_default()
            .into_iter()
            .map(|id| {
                let damaged = store.is_marked_damaged(&id);
                match store.manifest(&id) {
                    Some(m) => {
                        let g = m.current();
                        WorldEntry {
                            name: m.name.clone(),
                            day: g.map_or(0, |g| g.day),
                            population: m.summary.as_ref().map_or(0, |s| s.population),
                            play_ticks: g.map_or(0, |g| g.play_ticks),
                            saved: g.map_or_else(String::new, |g| g.saved_iso.clone()),
                            packs: m.content_refs.iter().map(|r| r.pack_id.clone()).collect(),
                            thumbnail: m.summary.as_ref().and_then(|s| s.thumbnail.clone()),
                            damaged,
                            id,
                        }
                    }
                    None => WorldEntry {
                        name: id.clone(),
                        day: 0,
                        population: 0,
                        play_ticks: 0,
                        saved: String::new(),
                        packs: Vec::new(),
                        thumbnail: None,
                        damaged: true,
                        id,
                    },
                }
            })
            .collect();
        // Newest first; ISO times sort as text.
        out.sort_by(|a, b| b.saved.cmp(&a.saved).then_with(|| a.id.cmp(&b.id)));
        out
    }

    /// The picture saved with a world, for the list.
    pub fn thumbnail(&self, name: &str) -> Option<Vec<u8>> {
        self.svc.storage.read(name).ok().flatten()
    }

    /// The same picture as `(width, height, rgb bytes)`, ready for a texture.
    /// The loaded content, for views that build their own small worlds (the main-menu backdrop).
    pub fn content(&self) -> Option<Arc<ContentSet>> {
        self.content.clone()
    }

    pub fn thumbnail_image(&self, name: &str) -> Option<(u32, u32, Vec<u8>)> {
        if !name.starts_with("worlds/") || name.contains("..") {
            return None;
        }
        crate::thumbnail::decode_png_rgb(&self.thumbnail(name)?)
    }

    fn unique_world_id(&self, name: &str) -> String {
        let base = slug(name);
        let existing = self.store().list_worlds().unwrap_or_default();
        if !existing.contains(&base) {
            return base;
        }
        (2..)
            .map(|n| format!("{base}-{n}"))
            .find(|id| !existing.contains(id) && valid_world_id(id))
            .unwrap_or(base)
    }

    fn factory(&self) -> SimFactory {
        let threads = std::thread::available_parallelism().map_or(2, |n| n.get().clamp(1, 4));
        SimFactory::dev(self.content.clone(), threads)
            .with_profiler(Some(self.profiler.clone()))
            .with_meter(Some(Arc::clone(&self.meter)))
    }

    fn create_world(
        &mut self,
        name: &str,
        seed: &str,
        size: MapSize,
        residents: u32,
        water: u32,
        tone: &str,
    ) -> Result<(String, WorldState), String> {
        let seed_text = if seed.is_empty() {
            format!("seed-{}", self.svc.clock.wall_clock_iso())
        } else {
            seed.to_owned()
        };
        let factory = self.factory();
        let mut sim = factory.new_sim(WorldState::new(name, seed_text));
        let (w, h) = size.tiles();
        let cmd = |c| SimInput::Command {
            actor: None,
            cmd: c,
        };
        let generated = self
            .content
            .as_ref()
            .is_some_and(|c| c.game().worldgen.is_some());
        if generated {
            sim.submit(
                0,
                cmd(Command::GenerateTown {
                    w,
                    h,
                    water,
                    residents,
                    tone: tone.to_owned(),
                }),
            )
            .map_err(|e| e.to_string())?;
        } else {
            // Content without game data (a stripped-down test pack): the old scaffolding town.
            sim.submit(0, cmd(Command::DevCreateMap { w, h, style: 1 }))
                .map_err(|e| e.to_string())?;
            for i in 0..residents {
                sim.submit(
                    0,
                    cmd(Command::DevSpawnPawn {
                        map: EntityId::new(Kind::Map, 1),
                        at: None,
                        name: format!("Resident {}", i + 1),
                    }),
                )
                .map_err(|e| e.to_string())?;
            }
        }
        let report = sim.step().map_err(|e| e.to_string())?;
        crate::console::push_events(&self.console, &report.events);
        if let Some(e) = report.events.iter().find(|e| e.kind == "input_rejected") {
            let why = e
                .detail
                .get("reason")
                .and_then(pg_core::canon::Canon::as_str)
                .unwrap_or("the town could not be generated");
            return Err(format!("The town could not be generated: {why}"));
        }
        let world = sim.world().clone();
        let id = self.unique_world_id(name);
        let thumb = thumbnail_png(&world);
        self.store()
            .save_with(
                &id,
                &world,
                &refs_of(&self.content),
                &self.svc.clock.wall_clock_iso(),
                thumb.as_deref(),
            )
            .map_err(|e| format!("The new world could not be saved: {e}"))?;
        Ok((id, world))
    }

    fn load_world(&self, id: &str) -> Result<(WorldState, Option<String>), String> {
        let refs = refs_of(&self.content);
        let opts = LoadOptions {
            content: self.content.as_deref(),
            installed_refs: self.content.as_ref().map(|_| refs),
            ..LoadOptions::default()
        };
        let loaded = self
            .store()
            .load(id, &opts)
            .map_err(|e| format!("The world could not be opened: {e}"))?;
        let note = match loaded.recovery {
            Recovery::Clean => None,
            Recovery::FellBack { ticks_lost, .. } => Some(self.text(
                "ui.notice.recovered",
                &[(
                    "ticks",
                    &ticks_lost.map_or_else(|| "?".to_owned(), |t| t.to_string()),
                )],
            )),
            Recovery::NoManifest { .. } => Some(self.text("ui.notice.recovered_manifest", &[])),
        };
        Ok((loaded.world, note))
    }

    fn open(&mut self, id: &str, world: WorldState) -> UiEvent {
        self.close_world();
        self.profiler.reset();
        self.meter.reset();
        self.last_hud = None;
        let name = world.meta.name.clone();
        let services = LoopServices {
            clock: Arc::clone(&self.svc.clock),
            storage: Arc::clone(&self.svc.storage),
            log: Arc::clone(&self.svc.log),
            pool: Arc::clone(&self.pool),
            thumbnailer: Some(Arc::new(thumbnail_png)),
            console: Some(self.console.clone()),
            dialogue: Some(self.dialogue.clone()),
            focus: Some(Arc::clone(&self.focus)),
        };
        self.console.info("app", &format!("world '{name}' opened"));
        let mut cfg = LoopConfig::new(id);
        cfg.app_version = self.svc.app_version.clone();
        cfg.content_refs = refs_of(&self.content);
        cfg.shadow_threads = self.shadow_threads;
        self.shadow_ok = 0;
        self.shadow_bad = 0;
        cfg.autosave_minutes = self
            .values
            .int("time.autosave_minutes")
            .and_then(|m| u64::try_from(m).ok())
            .unwrap_or(5);
        let lp = SimLoop::new(self.factory(), world, services, cfg);
        self.world = Some(ActiveWorld {
            id: id.to_owned(),
            name: name.clone(),
            session: Session::start(lp, Duration::from_millis(8)),
        });
        UiEvent::WorldOpened { name }
    }

    /// Stops the running world, waiting for its final save.
    fn close_world(&mut self) -> Vec<UiEvent> {
        let Some(w) = self.world.take() else {
            return Vec::new();
        };
        let mut out = Vec::new();
        if let Some(mut lp) = w.session.shutdown() {
            for e in lp.finish_saves() {
                if let LoopEvent::SaveFailed(why) = e {
                    out.push(UiEvent::Failed(
                        self.text("ui.notice.save_failed", &[("reason", &why)]),
                    ));
                }
            }
        }
        self.last_hud = None;
        out
    }

    /// Writes the world and stops everything. Call before the process exits.
    pub fn shutdown(&mut self) {
        if let Some(c) = &self.login_cancel {
            c.cancel();
        }
        let _ = self.close_world();
    }

    // ---- effects -------------------------------------------------------------------------------------------

    pub fn perform(&mut self, effect: AppEffect) -> Vec<UiEvent> {
        let out = self.perform_inner(effect);
        self.log_ui(&out);
        out
    }

    /// What the player is told about failures and notices also goes to the console.
    fn log_ui(&self, events: &[UiEvent]) {
        for e in events {
            match e {
                UiEvent::Failed(m) => self.console.error("app", m),
                UiEvent::Notice(m) => self.console.info("app", m),
                _ => {}
            }
        }
    }

    fn perform_inner(&mut self, effect: AppEffect) -> Vec<UiEvent> {
        match effect {
            AppEffect::ListWorlds => vec![UiEvent::WorldsListed(self.list_worlds())],
            AppEffect::CreateWorld {
                name,
                seed,
                size,
                residents,
                water,
                tone,
            } => match self.create_world(&name, &seed, size, residents, water, &tone) {
                Ok((id, world)) => vec![self.open(&id, world)],
                Err(e) => vec![UiEvent::Failed(e)],
            },
            AppEffect::LoadWorld(id) => match self.load_world(&id) {
                Ok((world, note)) => {
                    let mut out = vec![self.open(&id, world)];
                    out.extend(note.map(UiEvent::Notice));
                    out
                }
                Err(e) => vec![UiEvent::Failed(e)],
            },
            AppEffect::DeleteWorld(id) => match self.store().delete_world(&id) {
                Ok(_) => vec![UiEvent::WorldsListed(self.list_worlds())],
                Err(e) => vec![UiEvent::Failed(format!(
                    "The world could not be deleted: {e}"
                ))],
            },
            AppEffect::ExportWorld(id) => self.export(&id),
            AppEffect::ImportWorld => self.import(),
            AppEffect::SaveNow => {
                self.send(Control::SaveNow);
                self.set_status("ui.status.saving");
                Vec::new()
            }
            AppEffect::SetRunning(run) => {
                let e = if run {
                    RunEvent::Resume(self.speed)
                } else {
                    RunEvent::Pause
                };
                self.send(Control::Run(e));
                Vec::new()
            }
            AppEffect::SetSpeed(s) => {
                if let Some(speed) = Speed::ALL.into_iter().find(|x| x.name() == s) {
                    self.speed = speed;
                    self.send(Control::Run(RunEvent::Resume(speed)));
                }
                Vec::new()
            }
            AppEffect::LeaveWorld => self.close_world(),
            AppEffect::WindowFocus(gained) => {
                let wanted = self.values.bool("time.pause_on_focus_loss").unwrap_or(true);
                if gained {
                    self.send(Control::Run(RunEvent::FocusGained));
                } else if wanted {
                    self.send(Control::Run(RunEvent::FocusLost));
                }
                Vec::new()
            }
            AppEffect::Rewind { tick } => {
                self.send(Control::RewindTo(tick));
                Vec::new()
            }
            AppEffect::ClearConsole => {
                self.console.clear();
                Vec::new()
            }
            AppEffect::CopyText(_) => Vec::new(),
            AppEffect::ListMods => vec![UiEvent::ModsLoaded(self.mods_view())],
            AppEffect::SetPackEnabled { id, on } => self.change_mods(|c| {
                if on {
                    c.enabled.insert(id.clone());
                } else {
                    c.enabled.remove(&id);
                }
            }),
            AppEffect::ApproveCapability { id, cap, on } => {
                self.change_mods(|c| c.set_approved(&id, &cap, on))
            }
            AppEffect::SetSafeMode(on) => self.change_mods(|c| c.safe_mode = on),
            AppEffect::InstallPack => self.install_pack(),
            AppEffect::RemovePack(folder) => self.remove_pack(&folder),
            AppEffect::RevealPacks => {
                if let Some(d) = &self.svc.data_dir {
                    let dir = d.join(crate::mods::PACKS_DIR);
                    let _ = std::fs::create_dir_all(&dir);
                    (self.svc.reveal_path)(&dir);
                }
                Vec::new()
            }
            AppEffect::SaveConsoleLog(text) => {
                // The text is what the console showed, already redacted on its way in; redact again anyway.
                let clean = pg_host::redact(&text, &[]);
                let iso = self.svc.clock.wall_clock_iso();
                let name = format!("{EXPORT_DIR}console-{}.log", iso.replace([':', '.'], "-"));
                match self.svc.storage.write_atomic(&name, clean.as_bytes()) {
                    Ok(()) => {
                        let path = match &self.svc.data_dir {
                            Some(d) => d.join(&name).display().to_string(),
                            None => name,
                        };
                        vec![UiEvent::Notice(
                            self.text("ui.notice.log_saved", &[("path", &path)]),
                        )]
                    }
                    Err(e) => vec![UiEvent::Failed(format!(
                        "The log could not be written: {e}"
                    ))],
                }
            }
            AppEffect::CutBundle => {
                self.send(Control::CutBundle);
                Vec::new()
            }
            AppEffect::SetSetting { id, value } => self.set_setting(&id, &value),
            AppEffect::SelectProvider(p) => self.select_provider(&p),
            AppEffect::SetModel { provider, model } => self.set_model(&provider, &model),
            AppEffect::SetKey { provider, key } => {
                let Some(p) = Provider::from_id(&provider) else {
                    return vec![UiEvent::Failed("Unknown provider".to_owned())];
                };
                match KeyManager::new(self.svc.secrets.as_ref()).set_key(p, key.expose()) {
                    Ok(()) => vec![
                        UiEvent::AiLoaded(self.ai_state()),
                        UiEvent::Notice(self.text("ui.notice.key_saved", &[])),
                    ],
                    Err(e) => vec![UiEvent::Failed(e)],
                }
            }
            AppEffect::ClearKey(provider) => {
                if let Some(p) = Provider::from_id(&provider) {
                    let _ = KeyManager::new(self.svc.secrets.as_ref()).clear_key(p);
                }
                vec![UiEvent::AiLoaded(self.ai_state())]
            }
            AppEffect::TestConnection(provider) => self.test_connection(&provider),
            AppEffect::StartLogin(provider) => self.start_login(&provider),
            AppEffect::CancelLogin => {
                if let Some(c) = self.login_cancel.take() {
                    c.cancel();
                }
                Vec::new()
            }
            AppEffect::OpenUrl(url) => {
                if is_safe_verification_url(&url) {
                    (self.svc.open_url)(&url);
                    Vec::new()
                } else {
                    vec![UiEvent::Failed(self.text("ui.notice.unsafe_link", &[]))]
                }
            }
            AppEffect::RevealPath(name) => {
                if let Some(dir) = &self.svc.data_dir {
                    if name.starts_with("crash/") && !name.contains("..") {
                        (self.svc.reveal_path)(&dir.join(&name));
                    }
                }
                Vec::new()
            }
            AppEffect::DismissCrash => {
                for b in self.svc.storage.list("crash/").unwrap_or_default() {
                    if let Some(n) = b.name.strip_prefix("crash/").filter(|n| !n.contains('/')) {
                        let _ = self
                            .svc
                            .storage
                            .write_atomic(&format!("crash/dismissed/{n}"), b"seen");
                    }
                }
                Vec::new()
            }
            AppEffect::Quit => {
                self.quit = true;
                self.close_world()
            }
        }
    }

    fn send(&self, c: Control) {
        if let Some(w) = &self.world {
            w.session.send(c);
        }
    }

    fn set_status(&mut self, key: &str) {
        let text = self.text(key, &[]);
        let until = self.svc.clock.now_monotonic() + Duration::from_secs(3);
        self.status = Some((text, until));
    }

    fn set_setting(&mut self, id: &str, value: &SettingValue) -> Vec<UiEvent> {
        match self.registry.set(&mut self.values, id, to_canon(value)) {
            Ok(()) => {
                if let Err(e) = DeviceFile::new(self.svc.storage.as_ref(), self.registry.clone())
                    .save(&self.values)
                {
                    return vec![UiEvent::Failed(format!("Settings could not be saved: {e}"))];
                }
                if id == "dev.enabled" {
                    self.sync_console();
                }
                self.console.info("app", &format!("setting {id} changed"));
                if id == "time.autosave_minutes" {
                    if let Some(m) = self.values.int(id).and_then(|m| u64::try_from(m).ok()) {
                        self.send(Control::SetAutosaveMinutes(m));
                    }
                }
                Vec::new()
            }
            Err(e) => vec![
                UiEvent::Failed(e),
                UiEvent::SettingsLoaded(self.setting_items()),
            ],
        }
    }

    fn write_ai(&mut self, ai: &AiSettings) -> Result<(), String> {
        ai.write_into(&self.registry, &mut self.values)?;
        DeviceFile::new(self.svc.storage.as_ref(), self.registry.clone())
            .save(&self.values)
            .map_err(|e| e.to_string())
    }

    fn select_provider(&mut self, id: &str) -> Vec<UiEvent> {
        let Some(p) = Provider::from_id(id) else {
            return vec![UiEvent::Failed("Unknown provider".to_owned())];
        };
        let mut ai = self.ai_settings();
        ai.provider = p;
        if p.chooses_own_model() {
            ai.custom_model = None;
        }
        match self.write_ai(&ai) {
            Ok(()) => vec![UiEvent::AiLoaded(self.ai_state())],
            Err(e) => vec![UiEvent::Failed(e)],
        }
    }

    fn set_model(&mut self, provider: &str, model: &str) -> Vec<UiEvent> {
        let Some(p) = Provider::from_id(provider) else {
            return vec![UiEvent::Failed("Unknown provider".to_owned())];
        };
        if p.chooses_own_model() {
            return vec![UiEvent::Failed(
                self.text("ui.notice.model_fixed", &[("provider", p.display_name())]),
            )];
        }
        let mut ai = self.ai_settings();
        ai.provider = p;
        ai.custom_model = if model.is_empty() || model == p.recommended_model() {
            None
        } else {
            if let Err(e) = validate_model_id(model) {
                return vec![UiEvent::Failed(e)];
            }
            Some(model.to_owned())
        };
        match self.write_ai(&ai) {
            Ok(()) => vec![
                UiEvent::AiLoaded(self.ai_state()),
                UiEvent::Notice(self.text("ui.notice.model_saved", &[])),
            ],
            Err(e) => vec![UiEvent::Failed(e)],
        }
    }

    fn test_connection(&mut self, provider: &str) -> Vec<UiEvent> {
        let Some(p) = Provider::from_id(provider) else {
            return vec![UiEvent::Failed("Unknown provider".to_owned())];
        };
        let mut settings = self.ai_settings();
        settings.provider = p;
        settings.enabled = true;
        let client = AiClient::new(
            Arc::clone(&self.svc.net),
            Arc::clone(&self.svc.secrets),
            Arc::clone(&self.svc.clock),
            Arc::clone(&self.svc.log),
            ClientConfig::default(),
        );
        let tx = self.tx.clone();
        let ok_text = self.text("ui.ai.test_passed", &[]);
        let credits_key = self.text("ui.ai.test_credits", &[]);
        self.pool_spawn(move || {
            let r = client.test_connection(&settings, &CancelToken::new());
            let _ = tx.send(UiEvent::ConnectionResult(match r {
                Ok(info) => Ok(match info.credits {
                    Some(c) => credits_key.replace("{credits}", &c.to_string()),
                    None => ok_text,
                }),
                Err(e) => Err(e.user_message()),
            }));
        });
        Vec::new()
    }

    fn pool_spawn(&self, f: impl FnOnce() + Send + 'static) {
        let _ = self.pool.submit(f);
    }

    fn start_login(&mut self, provider: &str) -> Vec<UiEvent> {
        let Some(p) =
            Provider::from_id(provider).filter(|p| p.auth_method() == AuthMethod::DeviceLogin)
        else {
            return vec![UiEvent::Failed(
                "This provider does not use a sign-in code".to_owned(),
            )];
        };
        if let Some(old) = self.login_cancel.take() {
            old.cancel();
        }
        let cancel = CancelToken::new();
        self.login_cancel = Some(cancel.clone());
        let (net, clock, secrets, sleep, client_id) = (
            Arc::clone(&self.svc.net),
            Arc::clone(&self.svc.clock),
            Arc::clone(&self.svc.secrets),
            Arc::clone(&self.svc.sleep),
            self.svc.client_id.clone(),
        );
        let tx = self.tx.clone();
        // The worker owns the whole flow. It reports through the channel; cancelling stops it at the next
        // poll or sleep. A key that arrives is stored and never leaves this closure except into the store.
        std::thread::spawn(move || {
            let mut session = match DeviceLoginSession::begin(
                net.as_ref(),
                &client_id,
                clock.now_monotonic(),
                &cancel,
            ) {
                Ok(s) => s,
                Err(e) => {
                    if !cancel.is_cancelled() {
                        let _ = tx.send(UiEvent::LoginFinished(Err(e.user_message())));
                    }
                    return;
                }
            };
            let prompt = session.prompt().clone();
            let _ = tx.send(UiEvent::LoginPrompt {
                code: prompt.user_code.clone(),
                url: prompt
                    .complete_url
                    .clone()
                    .unwrap_or(prompt.verification_url.clone()),
            });
            loop {
                if cancel.is_cancelled() {
                    return;
                }
                match session.poll(net.as_ref(), clock.now_monotonic(), &cancel) {
                    SessionState::Waiting { next_in } => {
                        // In slices, so a cancel is noticed quickly.
                        let mut left = next_in;
                        while left > Duration::ZERO && !cancel.is_cancelled() {
                            let slice = left.min(Duration::from_millis(200));
                            sleep(slice);
                            left = left.saturating_sub(slice);
                        }
                    }
                    SessionState::Done(key) => {
                        let r = store_key(secrets.as_ref(), p, &key);
                        let _ = tx.send(UiEvent::LoginFinished(r));
                        return;
                    }
                    SessionState::Failed(e) => {
                        if !cancel.is_cancelled() {
                            let _ = tx.send(UiEvent::LoginFinished(Err(e.user_message())));
                        }
                        return;
                    }
                }
            }
        });
        Vec::new()
    }

    // ---- content packs (the Mods screen) ----------------------------------------------------------------------

    /// Tells the controller what happened when the packs were chosen at launch.
    pub fn set_startup_notes(&mut self, notes: Vec<String>) {
        self.mods_notes = notes;
    }

    fn mods_config(&self) -> (crate::mods::ModsConfig, Option<String>) {
        crate::mods::ModsConfig::load(self.svc.storage.as_ref())
    }

    fn mods_view(&self) -> pg_ui_model::types::ModsView {
        use pg_content::manifest::Capability;
        use pg_ui_model::types::{CapRow, ModsView, PackRow};
        let (config, note) = self.mods_config();
        let installed = self
            .svc
            .data_dir
            .as_deref()
            .map(crate::mods::discover)
            .unwrap_or_default();
        let loaded: Vec<String> = self
            .content
            .as_ref()
            .map(|c| c.load_order().iter().map(|p| p.to_string()).collect())
            .unwrap_or_default();
        let mut packs = Vec::new();
        if let Some(c) = &self.content {
            if let Some(m) = c
                .manifests()
                .find(|m| m.id.to_string() == crate::mods::BASE)
            {
                packs.push(PackRow {
                    id: m.id.to_string(),
                    folder: String::new(),
                    name: m.name.clone(),
                    version: format!(
                        "{}.{}.{}",
                        m.version.major, m.version.minor, m.version.patch
                    ),
                    depends: Vec::new(),
                    enabled: true,
                    loaded: true,
                    base: true,
                    capabilities: Vec::new(),
                    error: None,
                });
            }
        }
        for i in &installed {
            match &i.outcome {
                Err(e) => packs.push(PackRow {
                    id: String::new(),
                    folder: i.folder.clone(),
                    name: i.folder.clone(),
                    version: String::new(),
                    depends: Vec::new(),
                    enabled: false,
                    loaded: false,
                    base: false,
                    capabilities: Vec::new(),
                    error: Some(e.clone()),
                }),
                Ok(s) if s.id == crate::mods::BASE => {}
                Ok(s) => packs.push(PackRow {
                    id: s.id.clone(),
                    folder: i.folder.clone(),
                    name: s.name.clone(),
                    version: s.version.clone(),
                    depends: s
                        .depends
                        .iter()
                        .filter(|d| *d != crate::mods::BASE)
                        .cloned()
                        .collect(),
                    enabled: config.enabled.contains(&s.id),
                    loaded: loaded.contains(&s.id),
                    base: false,
                    capabilities: s
                        .capabilities
                        .iter()
                        .map(|c| {
                            let needs =
                                Capability::from_name(c).is_some_and(crate::mods::needs_approval);
                            CapRow {
                                name: c.clone(),
                                needs_approval: needs,
                                approved: config.is_approved(&s.id, c),
                            }
                        })
                        .collect(),
                    error: None,
                }),
            }
        }
        let will_load: Vec<String> = crate::mods::plan(&installed, &config, false)
            .dirs
            .iter()
            .filter_map(|d| installed.iter().find(|i| &i.path == d))
            .filter_map(|i| i.outcome.as_ref().ok().map(|s| s.id.clone()))
            .collect();
        let now_installed: Vec<String> = installed
            .iter()
            .filter_map(|i| i.outcome.as_ref().ok().map(|s| s.id.clone()))
            .filter(|id| loaded.contains(id) && id != crate::mods::BASE)
            .collect();
        let mut a = will_load.clone();
        let mut b = now_installed.clone();
        a.sort();
        b.sort();
        let mut notes = self.mods_notes.clone();
        notes.extend(note);
        ModsView {
            packs,
            safe_mode: config.safe_mode,
            notes,
            restart_needed: a != b || (config.safe_mode && !b.is_empty()),
            folder: self
                .svc
                .data_dir
                .as_ref()
                .map(|d| d.join(crate::mods::PACKS_DIR).display().to_string())
                .unwrap_or_default(),
        }
    }

    /// Changes the saved choices and shows the screen again.
    fn change_mods(&mut self, f: impl FnOnce(&mut crate::mods::ModsConfig)) -> Vec<UiEvent> {
        let (mut config, _) = self.mods_config();
        f(&mut config);
        match config.save(self.svc.storage.as_ref()) {
            Ok(()) => vec![UiEvent::ModsLoaded(self.mods_view())],
            Err(e) => vec![UiEvent::Failed(format!(
                "The mods settings could not be saved: {e}"
            ))],
        }
    }

    fn install_pack(&mut self) -> Vec<UiEvent> {
        let (Some(dialogs), Some(data_dir)) = (self.svc.dialogs.clone(), self.svc.data_dir.clone())
        else {
            return vec![UiEvent::Failed(
                "Installing a pack needs the system folder dialog, which is not available here. Copy the pack folder into the packs folder instead.".to_owned(),
            )];
        };
        let Some(folder) = dialogs.pick_folder() else {
            return Vec::new();
        };
        match crate::mods::install(&data_dir, &folder) {
            Ok(id) => vec![
                UiEvent::Notice(self.text("ui.notice.pack_installed", &[("id", &id)])),
                UiEvent::ModsLoaded(self.mods_view()),
            ],
            Err(e) => vec![UiEvent::Failed(e)],
        }
    }

    fn remove_pack(&mut self, folder: &str) -> Vec<UiEvent> {
        let Some(data_dir) = self.svc.data_dir.clone() else {
            return Vec::new();
        };
        // Forget the pack's choices too, so a pack installed again later starts from nothing.
        let id = crate::mods::discover(&data_dir)
            .into_iter()
            .find(|i| i.folder == folder)
            .and_then(|i| i.outcome.ok().map(|s| s.id));
        if let Err(e) = crate::mods::remove(&data_dir, folder) {
            return vec![UiEvent::Failed(e)];
        }
        if let Some(id) = id {
            let (mut config, _) = self.mods_config();
            config.enabled.remove(&id);
            config.approved.remove(&id);
            let _ = config.save(self.svc.storage.as_ref());
        }
        vec![UiEvent::ModsLoaded(self.mods_view())]
    }

    // ---- export and import ------------------------------------------------------------------------------------

    fn export(&mut self, id: &str) -> Vec<UiEvent> {
        let (world, _) = match self.load_world(id) {
            Ok(w) => w,
            Err(e) => return vec![UiEvent::Failed(e)],
        };
        let iso = self.svc.clock.wall_clock_iso();
        let bytes = export_world(&world, &refs_of(&self.content), &self.svc.app_version, &iso);
        let name = format!(
            "{EXPORT_DIR}{id}-{}.pgworld.json",
            iso.replace([':', '.'], "-")
        );
        // With system dialogs the player chooses where the file goes; cancelling does nothing.
        if let Some(dialogs) = &self.svc.dialogs {
            let suggested = format!("{id}.pgworld.json");
            let Some(path) = dialogs.pick_file_to_write(&suggested) else {
                return Vec::new();
            };
            return match std::fs::write(&path, &bytes) {
                Ok(()) => vec![UiEvent::Exported(path.display().to_string())],
                Err(e) => vec![UiEvent::Failed(format!(
                    "The export could not be written to {}: {e}",
                    path.display()
                ))],
            };
        }
        match self.svc.storage.write_atomic(&name, &bytes) {
            Ok(()) => vec![UiEvent::Exported(match &self.svc.data_dir {
                Some(d) => d.join(&name).display().to_string(),
                None => name,
            })],
            Err(e) => vec![UiEvent::Failed(format!(
                "The export could not be written: {e}"
            ))],
        }
    }

    fn import(&mut self) -> Vec<UiEvent> {
        // With system dialogs the player picks the file; cancelling does nothing.
        if let Some(dialogs) = self.svc.dialogs.clone() {
            let Some(path) = dialogs.pick_file_to_read(&["json"]) else {
                return Vec::new();
            };
            let label = path.display().to_string();
            let bytes = match std::fs::read(&path) {
                Ok(b) => b,
                Err(e) => {
                    return vec![UiEvent::Failed(format!("{label}: {e}"))];
                }
            };
            let mut out = self.import_bytes(&label, &bytes, None);
            out.push(UiEvent::WorldsListed(self.list_worlds()));
            return out;
        }
        let files: Vec<String> = self
            .svc
            .storage
            .list(IMPORT_DIR)
            .unwrap_or_default()
            .into_iter()
            .map(|b| b.name)
            .filter(|n| n.ends_with(".json"))
            .collect();
        if files.is_empty() {
            let place = match &self.svc.data_dir {
                Some(d) => d.join(IMPORT_DIR).display().to_string(),
                None => IMPORT_DIR.to_owned(),
            };
            return vec![UiEvent::Notice(
                self.text("ui.notice.import_empty", &[("folder", &place)]),
            )];
        }
        let mut out = Vec::new();
        for file in files {
            let bytes = match self.svc.storage.read(&file) {
                Ok(Some(b)) => b,
                _ => continue,
            };
            out.extend(self.import_bytes(&file, &bytes, Some(&file)));
        }
        out.push(UiEvent::WorldsListed(self.list_worlds()));
        out
    }

    /// Imports one export file's bytes as a new world. `consume` names a file in storage to delete once it
    /// has been imported (the folder-based flow); a file the player picked is left where it is.
    fn import_bytes(&mut self, label: &str, bytes: &[u8], consume: Option<&str>) -> Vec<UiEvent> {
        let refs = refs_of(&self.content);
        let opts = ImportOptions {
            content: self.content.as_deref(),
            installed_refs: self.content.as_ref().map(|_| refs.clone()),
            ..ImportOptions::default()
        };
        match import_world(bytes, &opts) {
            Ok(plan) => {
                let id = self.unique_world_id(&plan.world.meta.name);
                let iso = self.svc.clock.wall_clock_iso();
                match commit_import(&self.store(), &id, &plan, &iso, false) {
                    Ok(_) => {
                        if let Some(file) = consume {
                            let _ = self.svc.storage.delete(file);
                        }
                        vec![UiEvent::Imported(plan.world.meta.name.clone())]
                    }
                    Err(e) => vec![UiEvent::Failed(format!("{label}: {e}"))],
                }
            }
            Err(e) => vec![UiEvent::Failed(format!("{label}: {e}"))],
        }
    }

    // ---- frame ----------------------------------------------------------------------------------------------------

    /// Everything that happened since the last call: background results, the running world's events and,
    /// when it changed, the HUD.
    pub fn poll(&mut self) -> Vec<UiEvent> {
        let out = self.poll_inner();
        self.log_ui(&out);
        out
    }

    /// The resident whose surroundings are being watched or possessed; conversations within hearing range of
    /// them get lines prepared (and, with AI on, generated). `None` means nobody is listening.
    pub fn set_dialogue_focus(&self, focus: Option<pg_core::id::EntityId>) {
        if let Ok(mut s) = self.dialogue.service.lock() {
            s.set_focus(focus);
        }
        if let Ok(mut f) = self.focus.lock() {
            *f = focus;
        }
    }

    /// The dialogue service, for the renderer's bubbles and the overlay's AI panel.
    pub fn dialogue(&self) -> &crate::dialogue::DialogueHook {
        &self.dialogue
    }

    fn poll_inner(&mut self) -> Vec<UiEvent> {
        if let Ok(mut s) = self.dialogue.settings.lock() {
            *s = (
                self.ai_settings(),
                self.values.bool("content.graphic_filter").unwrap_or(true),
            );
        }
        let mut out = Vec::new();
        while let Ok(ev) = self.rx.try_recv() {
            let signed_in = matches!(ev, UiEvent::LoginFinished(Ok(())));
            out.push(ev);
            if signed_in {
                out.push(UiEvent::AiLoaded(self.ai_state()));
            }
        }
        let loop_events = self
            .world
            .as_ref()
            .map(|w| w.session.events())
            .unwrap_or_default();
        for e in loop_events {
            match e {
                LoopEvent::Saved { autosave, .. } => {
                    if autosave {
                        self.console.debug("app", "autosaved");
                    } else {
                        self.console.info("app", "saved");
                    }
                    self.set_status(if autosave {
                        "ui.status.autosaved"
                    } else {
                        "ui.status.saved"
                    });
                }
                LoopEvent::SaveFailed(why) => {
                    out.push(UiEvent::Failed(
                        self.text("ui.notice.save_failed", &[("reason", &why)]),
                    ));
                }
                LoopEvent::Crashed { bundle, .. } => {
                    out.push(UiEvent::Failed(self.text(
                        "ui.notice.crashed",
                        &[("bundle", bundle.as_deref().unwrap_or("-"))],
                    )));
                }
                LoopEvent::Diverged { at_tick, .. } => {
                    self.shadow_bad += 1;
                    out.push(UiEvent::Failed(
                        self.text("ui.notice.diverged", &[("tick", &at_tick.to_string())]),
                    ));
                }
                LoopEvent::BundleWritten(Some(name)) => {
                    out.push(UiEvent::Notice(
                        self.text("ui.notice.bundle", &[("name", &name)]),
                    ));
                }
                LoopEvent::BundleWritten(None) => {
                    out.push(UiEvent::Failed(self.text("ui.notice.bundle_failed", &[])));
                }
                LoopEvent::Rewound { to } => {
                    out.push(UiEvent::Notice(
                        self.text("ui.notice.rewound", &[("tick", &to.to_string())]),
                    ));
                }
                LoopEvent::Refused(m) => out.push(UiEvent::Notice(m)),
                LoopEvent::Verified { .. } => self.shadow_ok += 1,
                LoopEvent::State(_) => {}
            }
        }
        if let Some(h) = self.hud() {
            if self.last_hud.as_ref() != Some(&h) {
                self.last_hud = Some(h.clone());
                out.push(UiEvent::Hud(h));
            }
        }
        out
    }

    /// The latest picture of the running world for the renderer.
    pub fn snapshot(&self) -> Option<Arc<RenderSnapshot>> {
        self.world.as_ref().map(|w| w.session.snapshot())
    }

    fn hud(&mut self) -> Option<HudInfo> {
        let snap = self.snapshot()?;
        let now = self.svc.clock.now_monotonic();
        if self.status.as_ref().is_some_and(|(_, until)| now >= *until) {
            self.status = None;
        }
        let (running, suspended, speed) = match snap.run {
            RunState::Running(s) => (true, false, s.name().to_owned()),
            RunState::Suspended => (false, true, self.speed.name().to_owned()),
            _ => (false, false, self.speed.name().to_owned()),
        };
        Some(HudInfo {
            world_name: self
                .world
                .as_ref()
                .map_or_else(String::new, |w| w.name.clone()),
            tick: snap.tick,
            day: snap.day,
            minute_of_day: snap.minute_of_day,
            running,
            suspended,
            speed,
            pawns: u32::try_from(snap.pawns.len()).unwrap_or(u32::MAX),
            status: self
                .status
                .as_ref()
                .map_or_else(String::new, |(t, _)| t.clone()),
        })
    }

    /// The developer overlay's data: from the latest snapshot, the profiler, the script meter and the packs.
    pub fn overlay_data(&self) -> OverlayData {
        let mut d = OverlayData::default();
        if let Some(s) = self.snapshot() {
            d.tick = s.tick;
            d.day = s.day;
            d.hash = if s.keyframe_hash.is_empty() {
                s.day_hash.clone()
            } else {
                s.keyframe_hash.clone()
            };
            d.state = format!("{:?}", s.run);
            d.keyframes = s.keyframes.clone();
            if let Some(c) = &self.content {
                let locale = self.locale();
                d.reasons = s
                    .failures
                    .iter()
                    .map(|(name, code)| ReasonRow {
                        subject: name.clone(),
                        text: code.explain_in(c.strings(), &locale),
                    })
                    .collect();
            }
            d.shadow = match self.shadow_threads {
                Some(n) => format!(
                    "{} verified, {} diverged ({n} thread(s))",
                    self.shadow_ok, self.shadow_bad
                ),
                None => String::new(),
            };
            d.events = s
                .recent_events
                .iter()
                .map(|e| EventRow {
                    tick: e.tick,
                    kind: e.kind.clone(),
                    text: e.detail.to_canonical_string(),
                })
                .collect();
        }
        let report = self.profiler.report();
        let total = report.total.as_secs_f64().max(1e-12);
        d.systems = report
            .rows
            .iter()
            .map(|(name, s)| SystemRow {
                name: name.clone(),
                calls: s.calls,
                avg_micros: (s.total.as_secs_f64() * 1e6 / s.calls.max(1) as f64) as u64,
                share_permille: (s.total.as_secs_f64() / total * 1000.0) as u32,
            })
            .collect();
        if let Some(c) = &self.content {
            let scripted: Vec<String> = c.script_packs().iter().map(|p| p.id.to_owned()).collect();
            d.packs = c
                .manifests()
                .map(|m| PackRow {
                    id: m.id.to_string(),
                    version: m.version.to_string(),
                    status: if scripted.contains(&m.id.to_string()) {
                        "scripts".to_owned()
                    } else {
                        "data".to_owned()
                    },
                })
                .collect();
        }
        d.scripts = self
            .meter
            .rows()
            .into_iter()
            .map(|(pack, point, r)| ScriptRow {
                pack,
                point,
                calls: r.calls,
                fuel: r.fuel,
                errors: r.errors,
            })
            .collect();
        d
    }

    /// The id of the world that is open, if any.
    pub fn open_world_id(&self) -> Option<&str> {
        self.world.as_ref().map(|w| w.id.as_str())
    }
}

impl Drop for AppController {
    fn drop(&mut self) {
        self.shutdown();
    }
}
