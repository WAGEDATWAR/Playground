//! The application model: which screen is showing, and how every event moves between screens
//! (Blueprint §14.1, milestone 0.10).
//!
//! Everything here is a pure function from an event to a new state and a list of [`AppEffect`]s. The shell
//! feeds in key presses, clicks and the results of effects, draws [`AppModel::tree`], and performs the
//! effects. No screen is a dead end: each has a way back (Escape or a button), keyboard focus always lands on
//! something that does something, and a failed effect shows a notice instead of changing screens.

use crate::console::ConsoleModel;
use crate::overlay::{Overlay, OverlayData};
use crate::types::*;
use crate::widget::{Tree, Widget};
use pg_host::Secret;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Screen {
    Boot,
    /// A crash left a bug bundle (suggestion S-028).
    CrashPrompt {
        path: String,
    },
    MainMenu,
    NewWorld(NewWorldForm),
    SavedWorlds(SavedForm),
    Options,
    AiOptions(AiForm),
    /// Installed content packs.
    Mods,
    InGame,
    Pause,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NewWorldForm {
    pub name: String,
    pub seed: String,
    pub size: MapSize,
    pub residents: i64,
    /// Percent of the map that is water.
    pub water: i64,
    /// `cozy`, `standard` or `mature` (the default).
    pub tone: String,
    /// String-table key of the validation problem, if any.
    pub error: Option<&'static str>,
}

impl Default for NewWorldForm {
    fn default() -> Self {
        NewWorldForm {
            name: "New Town".to_owned(),
            seed: String::new(),
            size: MapSize::Small,
            residents: 14,
            water: 18,
            tone: "standard".to_owned(),
            error: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct SavedForm {
    pub selected: Option<String>,
    pub confirm_delete: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LoginUi {
    Idle,
    Starting,
    Prompt { code: String, url: String },
    Failed(String),
    Done,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConnUi {
    Idle,
    Testing,
    Ok(String),
    Failed(String),
}

/// The AI options screen's own state. The key being typed lives only here, as a [`Secret`], until it is
/// sent; it is never part of a snapshot.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AiForm {
    pub key_input: Option<Secret>,
    pub model_input: String,
    pub login: LoginUi,
    pub conn: ConnUi,
}

impl AiForm {
    fn new(model: &str) -> AiForm {
        AiForm {
            key_input: None,
            model_input: model.to_owned(),
            login: LoginUi::Idle,
            conn: ConnUi::Idle,
        }
    }
}

/// A line shown above the current screen until the next input.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Notice {
    Key(&'static str, Vec<(String, String)>),
    Text(String),
}

#[derive(Clone)]
pub struct AppModel {
    screen: Screen,
    stack: Vec<Screen>,
    worlds: Vec<WorldEntry>,
    settings: Vec<SettingItem>,
    ai: AiState,
    hud: HudInfo,
    notice: Option<Notice>,
    focus: Option<String>,
    in_world: bool,
    /// The pause menu stopped a running world, so closing it should start it again.
    paused_by_menu: bool,
    /// The drawer whose panel is open, if any (at most one).
    open_drawer: Option<String>,
    overlay: Overlay,
    console: ConsoleModel,
    mods: crate::types::ModsView,
    /// The town journal window is open.
    journal_open: bool,
    /// How the world is being played; the journal is for observation and possession modes only.
    mode: crate::journal::Mode,
}

impl Default for AppModel {
    fn default() -> Self {
        AppModel::new()
    }
}

fn plain(k: &str, _: &[(&str, &str)]) -> String {
    k.to_owned()
}

impl AppModel {
    pub fn new() -> AppModel {
        AppModel {
            screen: Screen::Boot,
            stack: Vec::new(),
            worlds: Vec::new(),
            settings: Vec::new(),
            ai: AiState::default(),
            hud: HudInfo::default(),
            notice: None,
            focus: None,
            in_world: false,
            paused_by_menu: false,
            open_drawer: None,
            overlay: Overlay::default(),
            console: ConsoleModel::default(),
            mods: crate::types::ModsView::default(),
            journal_open: false,
            mode: crate::journal::Mode::Observation,
        }
    }

    pub fn screen(&self) -> &Screen {
        &self.screen
    }

    pub fn in_world(&self) -> bool {
        self.in_world
    }

    pub fn focus(&self) -> Option<&str> {
        self.focus.as_deref()
    }

    pub fn hud(&self) -> &HudInfo {
        &self.hud
    }

    pub fn worlds(&self) -> &[WorldEntry] {
        &self.worlds
    }

    pub fn mods(&self) -> &crate::types::ModsView {
        &self.mods
    }

    /// Whether the journal window should be drawn.
    pub fn journal_visible(&self) -> bool {
        self.journal_open && self.in_world && crate::journal::available(self.mode)
    }

    pub fn set_journal_open(&mut self, open: bool) {
        self.journal_open = open;
    }

    pub fn toggle_journal(&mut self) {
        self.journal_open = !self.journal_open;
    }

    pub fn console(&self) -> &ConsoleModel {
        &self.console
    }

    /// New console entries from the app (the shell asks for those after [`ConsoleModel::last_seq`]).
    pub fn append_console(&mut self, entries: Vec<pg_host::console::Entry>) {
        self.console.append(entries);
    }

    /// The console's window while it is open (developer mode only).
    pub fn console_tree(&self, t: Text) -> Option<Tree> {
        self.console.visible().then(|| self.console.tree(t))
    }

    pub fn overlay(&self) -> &Overlay {
        &self.overlay
    }

    pub fn open_drawer(&self) -> Option<&str> {
        self.open_drawer.as_deref()
    }

    /// Developer mode is on (Options, Developer): the overlay and its tools are available.
    pub fn dev_mode(&self) -> bool {
        self.settings
            .iter()
            .any(|s| s.id == "dev.enabled" && s.value == SettingValue::Bool(true))
    }

    pub fn notice(&self) -> Option<&Notice> {
        self.notice.as_ref()
    }

    /// Hands the overlay the latest developer data (the shell does this every frame while it is visible).
    pub fn set_overlay_data(&mut self, data: OverlayData) {
        self.overlay.set_data(data);
    }

    // ---- views -------------------------------------------------------------------------------------------

    /// The widget tree of the current screen, translated with `t`.
    pub fn tree(&self, t: Text) -> Tree {
        let mut tree = crate::screens::build(self, t);
        if let Some(n) = &self.notice {
            let text = match n {
                Notice::Key(k, args) => {
                    let a: Vec<(&str, &str)> =
                        args.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
                    t(k, &a)
                }
                Notice::Text(s) => s.clone(),
            };
            tree.widgets.insert(0, Widget::Label(text));
        }
        tree
    }

    /// The in-game screen in its three places, whichever screen is on top of the world (the pause menu is
    /// drawn over it).
    pub fn hud_parts(&self, t: Text) -> crate::screens::HudParts {
        crate::screens::hud_parts(self, t)
    }

    /// The overlay's tree when it is visible.
    pub fn overlay_tree(&self, t: Text) -> Option<Tree> {
        self.overlay.visible().then(|| self.overlay.tree(t))
    }

    pub(crate) fn state(&self) -> ScreenState<'_> {
        ScreenState {
            screen: &self.screen,
            worlds: &self.worlds,
            settings: &self.settings,
            ai: &self.ai,
        }
    }

    // ---- transitions -------------------------------------------------------------------------------------

    fn go(&mut self, s: Screen) {
        let from = std::mem::replace(&mut self.screen, s);
        self.stack.push(from);
        self.focus = None;
        self.open_drawer = None;
    }

    fn back(&mut self) {
        if let Some(prev) = self.stack.pop() {
            self.screen = prev;
        } else {
            self.screen = if self.in_world {
                Screen::InGame
            } else {
                Screen::MainMenu
            };
        }
        self.focus = None;
        self.open_drawer = None;
    }

    fn reset_to(&mut self, s: Screen) {
        self.screen = s;
        self.stack.clear();
        self.focus = None;
        self.open_drawer = None;
    }

    fn notice_with(&mut self, key: &'static str, args: &[(&str, &str)]) {
        self.notice = Some(Notice::Key(
            key,
            args.iter()
                .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
                .collect(),
        ));
    }

    fn fix_focus(&mut self) {
        let tree = self.tree(&plain);
        let order = tree.focus_order();
        let valid = self.focus.as_deref().is_some_and(|f| order.contains(&f));
        if !valid {
            self.focus = order.first().map(|s| (*s).to_owned());
        }
    }

    /// Moves focus one step through the focus order (wrapping).
    fn move_focus(&mut self, delta: i32) {
        let tree = self.tree(&plain);
        let order: Vec<String> = tree.focus_order().iter().map(|s| (*s).to_owned()).collect();
        if order.is_empty() {
            return;
        }
        let at = self
            .focus
            .as_ref()
            .and_then(|f| order.iter().position(|o| o == f))
            .unwrap_or(0);
        let n = order.len() as i32;
        let next = (at as i32 + delta).rem_euclid(n) as usize;
        self.focus = order.get(next).cloned();
    }

    /// The main entry point: applies `ev` and returns what the app must do.
    pub fn update(&mut self, ev: UiEvent) -> Vec<AppEffect> {
        if matches!(
            ev,
            UiEvent::Key(_)
                | UiEvent::Click(_)
                | UiEvent::Text(..)
                | UiEvent::Toggle(..)
                | UiEvent::Choose(..)
                | UiEvent::Slide(..)
        ) {
            self.notice = None;
        }
        let effects = self.apply(ev);
        if !self.dev_mode() && self.overlay.visible() {
            self.overlay.toggle();
        }
        if !self.dev_mode() {
            self.console.hide();
        }
        self.fix_focus();
        effects
    }

    fn apply(&mut self, ev: UiEvent) -> Vec<AppEffect> {
        match ev {
            UiEvent::Key(k) => self.key(k),
            UiEvent::Click(id) => {
                self.focus = Some(id.clone());
                self.activate(&id)
            }
            UiEvent::Text(id, s) => self.text(&id, s),
            UiEvent::Toggle(id, v) => self.toggle(&id, v),
            UiEvent::Choose(id, v) => self.choose(&id, &v),
            UiEvent::Slide(id, v) => self.slide(&id, v),
            UiEvent::CloseRequested => {
                if self.in_world {
                    vec![AppEffect::SaveNow, AppEffect::Quit]
                } else {
                    vec![AppEffect::Quit]
                }
            }
            UiEvent::FocusLost => {
                if self.in_world {
                    vec![AppEffect::WindowFocus(false)]
                } else {
                    Vec::new()
                }
            }
            UiEvent::FocusGained => {
                if self.in_world {
                    vec![AppEffect::WindowFocus(true)]
                } else {
                    Vec::new()
                }
            }
            UiEvent::Booted(info) => {
                let info = *info;
                self.worlds = info.worlds;
                self.settings = info.settings;
                self.ai = info.ai;
                if let Some(w) = info.settings_warnings.first() {
                    self.notice = Some(Notice::Text(w.clone()));
                }
                match info.crash_bundle {
                    Some(path) => self.reset_to(Screen::CrashPrompt { path }),
                    None => self.reset_to(Screen::MainMenu),
                }
                Vec::new()
            }
            UiEvent::WorldsListed(list) => {
                self.worlds = list;
                if let Screen::SavedWorlds(f) = &mut self.screen {
                    if f.selected
                        .as_ref()
                        .is_some_and(|s| !self.worlds.iter().any(|w| &w.id == s))
                    {
                        f.selected = None;
                        f.confirm_delete = false;
                    }
                }
                Vec::new()
            }
            UiEvent::WorldOpened { name } => {
                self.hud = HudInfo {
                    world_name: name,
                    ..HudInfo::default()
                };
                self.in_world = true;
                // Every world starts with the console's filters at their defaults (Debug off).
                self.console.reset_filters();
                self.journal_open = false;
                self.reset_to(Screen::InGame);
                Vec::new()
            }
            UiEvent::Failed(msg) => {
                self.notice = Some(Notice::Text(msg));
                if let Screen::AiOptions(f) = &mut self.screen {
                    if matches!(f.login, LoginUi::Starting) {
                        f.login = LoginUi::Idle;
                    }
                    if matches!(f.conn, ConnUi::Testing) {
                        f.conn = ConnUi::Idle;
                    }
                }
                Vec::new()
            }
            UiEvent::Hud(h) => {
                if self.in_world {
                    // The notice for a returning player takes focus when it appears, so Enter resumes.
                    let appeared = h.suspended && !self.hud.suspended;
                    self.hud = h;
                    if appeared && matches!(self.screen, Screen::InGame) {
                        self.open_drawer = None;
                        self.focus = Some("hud.resume".to_owned());
                    }
                }
                Vec::new()
            }
            UiEvent::SettingsLoaded(s) => {
                self.settings = s;
                Vec::new()
            }
            UiEvent::AiLoaded(a) => {
                self.ai = a;
                Vec::new()
            }
            UiEvent::ModsLoaded(v) => {
                self.mods = v;
                Vec::new()
            }
            UiEvent::LoginPrompt { code, url } => {
                if let Screen::AiOptions(f) = &mut self.screen {
                    f.login = LoginUi::Prompt { code, url };
                }
                Vec::new()
            }
            UiEvent::LoginFinished(r) => {
                if let Screen::AiOptions(f) = &mut self.screen {
                    f.login = match r {
                        Ok(()) => LoginUi::Done,
                        Err(m) => LoginUi::Failed(m),
                    };
                }
                Vec::new()
            }
            UiEvent::ConnectionResult(r) => {
                if let Screen::AiOptions(f) = &mut self.screen {
                    f.conn = match r {
                        Ok(m) => ConnUi::Ok(m),
                        Err(m) => ConnUi::Failed(m),
                    };
                }
                Vec::new()
            }
            UiEvent::Notice(n) => {
                self.notice = Some(Notice::Text(n));
                Vec::new()
            }
            UiEvent::Imported(name) => {
                self.notice_with("ui.notice.imported", &[("name", &name)]);
                vec![AppEffect::ListWorlds]
            }
            UiEvent::Exported(path) => {
                self.notice_with("ui.notice.exported", &[("path", &path)]);
                Vec::new()
            }
        }
    }

    // ---- keyboard ----------------------------------------------------------------------------------------

    fn key(&mut self, k: Key) -> Vec<AppEffect> {
        if k == Key::F3 {
            if self.dev_mode() {
                self.overlay.toggle();
            }
            return Vec::new();
        }
        if k == Key::Console {
            if self.dev_mode() {
                self.console.toggle();
            }
            return Vec::new();
        }
        if k == Key::Journal {
            if self.in_world
                && crate::journal::available(self.mode)
                && matches!(self.screen, Screen::InGame)
            {
                self.journal_open = !self.journal_open;
            }
            return Vec::new();
        }
        if k == Key::Escape && self.console.visible() {
            self.console.hide();
            return Vec::new();
        }
        if let Some(fx) = self.drawer_key(k) {
            return fx;
        }
        // In the world, Space pauses and resumes whatever has focus.
        if k == Key::Space && matches!(self.screen, Screen::InGame) && self.open_drawer.is_none() {
            return vec![AppEffect::SetRunning(!self.hud.running)];
        }
        match k {
            Key::Tab | Key::Down => {
                self.move_focus(1);
                Vec::new()
            }
            Key::BackTab | Key::Up => {
                self.move_focus(-1);
                Vec::new()
            }
            Key::Escape => self.escape(),
            Key::Enter | Key::Space => match self.focus.clone() {
                Some(id) => self.activate_focused(&id),
                None => Vec::new(),
            },
            Key::Left => self.adjust(-1),
            Key::Right => self.adjust(1),
            Key::F3 | Key::Console | Key::Journal => Vec::new(),
        }
    }

    /// Arrow keys inside an open drawer move between its items (Up and Down jump a row in a grid) and
    /// stop at the ends. `None` when the key is not the drawer's business.
    fn drawer_key(&mut self, k: Key) -> Option<Vec<AppEffect>> {
        self.open_drawer.as_ref()?;
        let focus = self.focus.clone()?;
        let tree = self.tree(&plain);
        let (drawer, at) = tree.drawer_of(&focus)?;
        let Widget::Drawer { items, layout, .. } = drawer else {
            return None;
        };
        let cols = layout.columns(items.len()) as i64;
        let step = match k {
            Key::Left => -1,
            Key::Right => 1,
            Key::Up => -cols,
            Key::Down => cols,
            _ => return None,
        };
        let to = (at as i64 + step).clamp(0, items.len() as i64 - 1) as usize;
        self.focus = items.get(to).map(|i| i.id.clone());
        Some(Vec::new())
    }

    /// Enter or Space on the focused widget.
    fn activate_focused(&mut self, id: &str) -> Vec<AppEffect> {
        let tree = self.tree(&plain);
        if tree.drawer_of(id).is_some() {
            return self.activate(id);
        }
        match tree.find(id) {
            Some(Widget::Button { .. }) | Some(Widget::Drawer { .. }) => self.activate(id),
            Some(Widget::Toggle { value, .. }) => {
                let v = !*value;
                self.toggle(id, v)
            }
            Some(Widget::Choice { .. }) => self.adjust(1),
            Some(Widget::TextField { .. }) => {
                // Enter in a text field moves on, like Tab.
                self.move_focus(1);
                Vec::new()
            }
            _ => Vec::new(),
        }
    }

    /// Left and right on a choice or slider.
    fn adjust(&mut self, dir: i64) -> Vec<AppEffect> {
        let Some(id) = self.focus.clone() else {
            return Vec::new();
        };
        let tree = self.tree(&plain);
        match tree.find(&id) {
            Some(Widget::Choice { options, value, .. }) => {
                let at = options.iter().position(|(v, _)| v == value).unwrap_or(0) as i64;
                let n = options.len() as i64;
                if n == 0 {
                    return Vec::new();
                }
                let next = (at + dir).rem_euclid(n) as usize;
                match options.get(next) {
                    Some((v, _)) => {
                        let v = v.clone();
                        self.choose(&id, &v)
                    }
                    None => Vec::new(),
                }
            }
            Some(Widget::Slider {
                min, max, value, ..
            }) => {
                let step = ((max - min) / 50).max(1);
                let v = (value + dir * step).clamp(*min, *max);
                self.slide(&id, v)
            }
            _ => Vec::new(),
        }
    }

    fn escape(&mut self) -> Vec<AppEffect> {
        match self.screen.clone() {
            Screen::Boot | Screen::MainMenu => Vec::new(),
            Screen::CrashPrompt { .. } => self.activate("crash.dismiss"),
            Screen::InGame if self.open_drawer.is_some() => {
                self.close_drawer();
                Vec::new()
            }
            Screen::InGame => self.open_pause(),
            Screen::Pause => self.close_pause(),
            Screen::SavedWorlds(f) if f.confirm_delete => {
                if let Screen::SavedWorlds(g) = &mut self.screen {
                    g.confirm_delete = false;
                }
                Vec::new()
            }
            Screen::AiOptions(f) => {
                let cancel = matches!(f.login, LoginUi::Starting | LoginUi::Prompt { .. });
                self.back();
                if cancel {
                    vec![AppEffect::CancelLogin]
                } else {
                    Vec::new()
                }
            }
            Screen::NewWorld(_) | Screen::SavedWorlds(_) | Screen::Options | Screen::Mods => {
                self.back();
                Vec::new()
            }
        }
    }

    // ---- widget events -----------------------------------------------------------------------------------

    fn activate(&mut self, id: &str) -> Vec<AppEffect> {
        if id.starts_with("overlay.") {
            return self.overlay.click(id);
        }
        if id.starts_with("console.") {
            return self.console.click(id);
        }
        if id == "journal.close" {
            self.journal_open = false;
            return Vec::new();
        }
        crate::screens::activate(self, id)
    }

    fn text(&mut self, id: &str, s: String) -> Vec<AppEffect> {
        if id.starts_with("overlay.") {
            self.overlay.text(id, s);
            return Vec::new();
        }
        if id.starts_with("console.") {
            self.console.text(id, s);
            return Vec::new();
        }
        crate::screens::text(self, id, s)
    }

    fn toggle(&mut self, id: &str, v: bool) -> Vec<AppEffect> {
        if id.starts_with("console.") {
            self.console.toggle_type(id, v);
            return Vec::new();
        }
        crate::screens::toggle(self, id, v)
    }

    fn choose(&mut self, id: &str, v: &str) -> Vec<AppEffect> {
        crate::screens::choose(self, id, v)
    }

    fn slide(&mut self, id: &str, v: i64) -> Vec<AppEffect> {
        crate::screens::slide(self, id, v)
    }

    // ---- the pieces screens.rs needs to change ---------------------------------------------------------------

    pub(crate) fn screen_mut(&mut self) -> &mut Screen {
        &mut self.screen
    }

    /// Opens the pause menu. A running world is paused while it is open.
    pub(crate) fn open_pause(&mut self) -> Vec<AppEffect> {
        self.paused_by_menu = self.hud.running;
        self.go(Screen::Pause);
        if self.paused_by_menu {
            vec![AppEffect::SetRunning(false)]
        } else {
            Vec::new()
        }
    }

    /// Closes the pause menu, resuming the world if opening it paused it.
    pub(crate) fn close_pause(&mut self) -> Vec<AppEffect> {
        self.back();
        if std::mem::take(&mut self.paused_by_menu) {
            vec![AppEffect::SetRunning(true)]
        } else {
            Vec::new()
        }
    }

    /// Opens the drawer `id` (closing any other) with focus on its selected item, or closes it if it was open.
    pub(crate) fn toggle_drawer(&mut self, id: &str) {
        if self.open_drawer.as_deref() == Some(id) {
            self.close_drawer();
            return;
        }
        self.open_drawer = Some(id.to_owned());
        let tree = self.tree(&plain);
        let first = tree.walk().into_iter().find_map(|w| match w {
            Widget::Drawer { id: d, items, .. } if d == id => items
                .iter()
                .find(|i| i.selected)
                .or(items.first())
                .map(|i| i.id.clone()),
            _ => None,
        });
        if first.is_some() {
            self.focus = first;
        }
    }

    /// Closes the open drawer and returns focus to its button.
    pub(crate) fn close_drawer(&mut self) {
        if let Some(id) = self.open_drawer.take() {
            self.focus = Some(id);
        }
    }

    pub(crate) fn go_to(&mut self, s: Screen) {
        self.go(s);
    }

    pub(crate) fn go_back(&mut self) {
        self.back();
    }

    pub(crate) fn leave_world(&mut self) {
        self.in_world = false;
        self.paused_by_menu = false;
        self.reset_to(Screen::MainMenu);
    }

    pub(crate) fn settings_mut(&mut self) -> &mut Vec<SettingItem> {
        &mut self.settings
    }

    pub(crate) fn ai_mut(&mut self) -> &mut AiState {
        &mut self.ai
    }

    pub(crate) fn ai_state(&self) -> &AiState {
        &self.ai
    }

    pub(crate) fn new_ai_form(&self) -> AiForm {
        let model = self
            .ai
            .providers
            .iter()
            .find(|p| p.id == self.ai.selected)
            .map_or("", |p| p.model.as_str());
        AiForm::new(model)
    }

    pub(crate) fn ai_form_for(&self, provider: &str) -> AiForm {
        let model = self
            .ai
            .providers
            .iter()
            .find(|p| p.id == provider)
            .map_or("", |p| p.model.as_str());
        AiForm::new(model)
    }
}

/// A read-only view for the screen builders.
pub(crate) struct ScreenState<'a> {
    pub screen: &'a Screen,
    pub worlds: &'a [WorldEntry],
    pub settings: &'a [SettingItem],
    pub ai: &'a AiState,
}
