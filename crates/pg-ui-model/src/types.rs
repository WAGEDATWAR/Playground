//! The vocabulary between the app shell and the screens: events in, effects out, and the plain data the
//! screens show. Nothing here touches the world, the disk or the network: screens are pure functions from
//! an event to a new state and a list of requests (Blueprint §14.1), so every flow can be tested without a
//! window.

use pg_host::Secret;
use std::fmt;

/// Keys the model understands. The shell maps real key events onto these.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Key {
    Tab,
    BackTab,
    Up,
    Down,
    Left,
    Right,
    Enter,
    Escape,
    Space,
    /// Toggles the developer overlay.
    F3,
    /// The backtick key: toggles the developer console.
    Console,
    /// The J key: toggles the town journal.
    Journal,
}

/// One saved world as the Saved Worlds list shows it (suggestion S-024), built from the manifest alone.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorldEntry {
    pub id: String,
    pub name: String,
    pub day: u64,
    pub population: u32,
    pub play_ticks: u64,
    /// Time of the last save, as text.
    pub saved: String,
    pub packs: Vec<String>,
    /// Name of the thumbnail blob, if one was saved.
    pub thumbnail: Option<String>,
    /// The slot is marked damaged (a load fell back or failed).
    pub damaged: bool,
}

/// What the in-game bar shows. The app builds it from the latest render snapshot.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct HudInfo {
    pub world_name: String,
    pub tick: u64,
    pub day: u64,
    pub minute_of_day: u32,
    pub running: bool,
    /// The world paused itself when the window lost focus and waits for the player.
    pub suspended: bool,
    /// `1x`, `3x`, `9x` or `27x`.
    pub speed: String,
    pub pawns: u32,
    /// A short status such as "Saved" or "Autosaving"; empty for none.
    pub status: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SettingValue {
    Bool(bool),
    Int(i64),
    Text(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SettingKind {
    Bool,
    Int {
        min: i64,
        max: i64,
    },
    /// `(value, label key)`.
    Enum(Vec<(String, String)>),
    Text {
        max_len: usize,
    },
}

/// One entry of the generated Options screen (suggestion S-025): the registry's setting with its current
/// value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SettingItem {
    pub id: String,
    /// String-table key of the label, normally `setting.<id>`.
    pub label_key: String,
    pub kind: SettingKind,
    pub value: SettingValue,
    pub restart_required: bool,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Auth {
    /// The player pastes a key.
    PasteKey,
    /// The player signs in with a code shown on screen (Player2).
    DeviceCode,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProviderInfo {
    pub id: String,
    pub label: String,
    pub auth: Auth,
    pub has_key: bool,
    /// The provider picks the model itself (Player2); the model field is informational.
    pub model_fixed: bool,
    pub model: String,
    pub recommended_model: String,
}

/// The AI options as loaded by the app.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct AiState {
    pub providers: Vec<ProviderInfo>,
    pub selected: String,
    /// `true` when keys survive a restart (an OS credential store is available).
    pub persistent_keys: bool,
}

/// What the Boot screen learned.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct BootInfo {
    pub worlds: Vec<WorldEntry>,
    pub settings: Vec<SettingItem>,
    pub ai: AiState,
    /// A bug bundle written by a crash last time, if any (suggestion S-028).
    pub crash_bundle: Option<String>,
    /// Problems found in the settings file; shown once.
    pub settings_warnings: Vec<String>,
}

/// A request from the player's input or from the app, sent into the model.
#[derive(Clone, PartialEq, Eq)]
pub enum UiEvent {
    // Input.
    Key(Key),
    Click(String),
    Text(String, String),
    Toggle(String, bool),
    Choose(String, String),
    Slide(String, i64),
    // Window.
    CloseRequested,
    FocusLost,
    FocusGained,
    // Results of effects.
    Booted(Box<BootInfo>),
    WorldsListed(Vec<WorldEntry>),
    WorldOpened {
        name: String,
    },
    /// An effect failed; the text is already player-readable.
    Failed(String),
    Hud(HudInfo),
    SettingsLoaded(Vec<SettingItem>),
    AiLoaded(AiState),
    LoginPrompt {
        code: String,
        url: String,
    },
    LoginFinished(Result<(), String>),
    ConnectionResult(Result<String, String>),
    Notice(String),
    Imported(String),
    Exported(String),
}

impl fmt::Debug for UiEvent {
    /// Text typed into fields may be a key: only its length is ever printed.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            UiEvent::Text(id, t) => write!(f, "Text({id}, <{} chars>)", t.chars().count()),
            UiEvent::Key(k) => write!(f, "Key({k:?})"),
            UiEvent::Click(id) => write!(f, "Click({id})"),
            UiEvent::Toggle(id, v) => write!(f, "Toggle({id}, {v})"),
            UiEvent::Choose(id, v) => write!(f, "Choose({id}, {v})"),
            UiEvent::Slide(id, v) => write!(f, "Slide({id}, {v})"),
            UiEvent::CloseRequested => write!(f, "CloseRequested"),
            UiEvent::FocusLost => write!(f, "FocusLost"),
            UiEvent::FocusGained => write!(f, "FocusGained"),
            UiEvent::Booted(_) => write!(f, "Booted"),
            UiEvent::WorldsListed(w) => write!(f, "WorldsListed({})", w.len()),
            UiEvent::WorldOpened { name } => write!(f, "WorldOpened({name})"),
            UiEvent::Failed(m) => write!(f, "Failed({m})"),
            UiEvent::Hud(h) => write!(f, "Hud(tick {})", h.tick),
            UiEvent::SettingsLoaded(s) => write!(f, "SettingsLoaded({})", s.len()),
            UiEvent::AiLoaded(_) => write!(f, "AiLoaded"),
            UiEvent::LoginPrompt { .. } => write!(f, "LoginPrompt"),
            UiEvent::LoginFinished(r) => write!(f, "LoginFinished(ok: {})", r.is_ok()),
            UiEvent::ConnectionResult(r) => write!(f, "ConnectionResult(ok: {})", r.is_ok()),
            UiEvent::Notice(n) => write!(f, "Notice({n})"),
            UiEvent::Imported(n) => write!(f, "Imported({n})"),
            UiEvent::Exported(n) => write!(f, "Exported({n})"),
        }
    }
}

/// The map sizes the New World screen offers.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum MapSize {
    Small,
    Medium,
    Large,
}

impl MapSize {
    pub const ALL: [MapSize; 3] = [MapSize::Small, MapSize::Medium, MapSize::Large];

    pub const fn id(self) -> &'static str {
        match self {
            MapSize::Small => "small",
            MapSize::Medium => "medium",
            MapSize::Large => "large",
        }
    }

    pub fn from_id(id: &str) -> Option<MapSize> {
        MapSize::ALL.into_iter().find(|m| m.id() == id)
    }

    pub const fn tiles(self) -> (i32, i32) {
        match self {
            MapSize::Small => (48, 36),
            MapSize::Medium => (64, 48),
            MapSize::Large => (96, 72),
        }
    }
}

/// What the model asks the app to do. The app performs it and answers with an event.
#[derive(Clone)]
pub enum AppEffect {
    ListWorlds,
    CreateWorld {
        name: String,
        seed: String,
        size: MapSize,
        residents: u32,
        /// Percent of the map that is water.
        water: u32,
        /// `cozy`, `standard` or `mature`.
        tone: String,
    },
    LoadWorld(String),
    DeleteWorld(String),
    ExportWorld(String),
    ImportWorld,
    /// Saves the running world now.
    SaveNow,
    SetRunning(bool),
    SetSpeed(String),
    /// Stops the running world and returns to the menu.
    LeaveWorld,
    /// The window lost (false) or regained (true) focus while a world is running.
    WindowFocus(bool),
    Rewind {
        tick: u64,
    },
    /// Cuts a bug bundle of the running world (suggestion S-001).
    CutBundle,
    /// Empties the developer console's log.
    ClearConsole,
    /// Puts text on the clipboard (handled by the shell).
    CopyText(String),
    /// Writes the developer console's visible lines to a log file (redacted); the text is what was shown.
    SaveConsoleLog(String),
    SetSetting {
        id: String,
        value: SettingValue,
    },
    SelectProvider(String),
    SetModel {
        provider: String,
        model: String,
    },
    SetKey {
        provider: String,
        key: Secret,
    },
    ClearKey(String),
    TestConnection(String),
    StartLogin(String),
    CancelLogin,
    OpenUrl(String),
    RevealPath(String),
    DismissCrash,
    Quit,
}

impl fmt::Debug for AppEffect {
    /// Keys never print: `SetKey` shows the provider only.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AppEffect::SetKey { provider, .. } => write!(f, "SetKey({provider}, <redacted>)"),
            AppEffect::CreateWorld {
                name,
                seed,
                size,
                residents,
                water,
                tone,
            } => write!(
                f,
                "CreateWorld({name}, {seed}, {size:?}, {residents}, water {water}, {tone})"
            ),
            AppEffect::LoadWorld(i) => write!(f, "LoadWorld({i})"),
            AppEffect::DeleteWorld(i) => write!(f, "DeleteWorld({i})"),
            AppEffect::ExportWorld(i) => write!(f, "ExportWorld({i})"),
            AppEffect::SetSetting { id, value } => write!(f, "SetSetting({id}, {value:?})"),
            AppEffect::SetModel { provider, model } => write!(f, "SetModel({provider}, {model})"),
            AppEffect::ListWorlds => write!(f, "ListWorlds"),
            AppEffect::ImportWorld => write!(f, "ImportWorld"),
            AppEffect::SaveNow => write!(f, "SaveNow"),
            AppEffect::SetRunning(r) => write!(f, "SetRunning({r})"),
            AppEffect::SetSpeed(s) => write!(f, "SetSpeed({s})"),
            AppEffect::LeaveWorld => write!(f, "LeaveWorld"),
            AppEffect::WindowFocus(g) => write!(f, "WindowFocus({g})"),
            AppEffect::Rewind { tick } => write!(f, "Rewind({tick})"),
            AppEffect::CutBundle => write!(f, "CutBundle"),
            AppEffect::ClearConsole => write!(f, "ClearConsole"),
            AppEffect::CopyText(t) => write!(f, "CopyText({} chars)", t.len()),
            AppEffect::SaveConsoleLog(t) => write!(f, "SaveConsoleLog({} chars)", t.len()),
            AppEffect::SelectProvider(p) => write!(f, "SelectProvider({p})"),
            AppEffect::ClearKey(p) => write!(f, "ClearKey({p})"),
            AppEffect::TestConnection(p) => write!(f, "TestConnection({p})"),
            AppEffect::StartLogin(p) => write!(f, "StartLogin({p})"),
            AppEffect::CancelLogin => write!(f, "CancelLogin"),
            AppEffect::OpenUrl(u) => write!(f, "OpenUrl({u})"),
            AppEffect::RevealPath(p) => write!(f, "RevealPath({p})"),
            AppEffect::DismissCrash => write!(f, "DismissCrash"),
            AppEffect::Quit => write!(f, "Quit"),
        }
    }
}

impl PartialEq for AppEffect {
    fn eq(&self, other: &Self) -> bool {
        format!("{self:?}") == format!("{other:?}")
    }
}

impl Eq for AppEffect {}

/// Translates a string-table key, with `{name}` arguments. Supplied by the shell (the real string table, or
/// a stub in tests).
pub type Text<'a> = &'a dyn Fn(&str, &[(&str, &str)]) -> String;
