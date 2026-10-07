//! The application model: which screen is showing, and how every event moves between screens
//! (Blueprint §14.1, milestone 0.10).
//!
//! Everything here is a pure function from an event to a new state and a list of [`AppEffect`]s. The shell
//! feeds in key presses, clicks and the results of effects, draws [`AppModel::tree`], and performs the
//! effects. No screen is a dead end: each has a way back (Escape or a button), keyboard focus always lands on
//! something that does something, and a failed effect shows a notice instead of changing screens.

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
    InGame,
    Pause,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NewWorldForm {
    pub name: String,
    pub seed: String,
    pub size: MapSize,
    pub residents: i64,
    /// String-table key of the validation problem, if any.
    pub error: Option<&'static str>,
}

impl Default for NewWorldForm {
    fn default() -> Self {
        NewWorldForm {
            name: "New Town".to_owned(),
            seed: String::new(),
            size: MapSize::Small,
            residents: 10,
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
    overlay: Overlay,
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
            overlay: Overlay::default(),
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

    pub fn overlay(&self) -> &Overlay {
        &self.overlay
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
            hud: &self.hud,
        }
    }

    // ---- transitions -------------------------------------------------------------------------------------

    fn go(&mut self, s: Screen) {
        let from = std::mem::replace(&mut self.screen, s);
        self.stack.push(from);
        self.focus = None;
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
    }

    fn reset_to(&mut self, s: Screen) {
        self.screen = s;
        self.stack.clear();
        self.focus = None;
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
                    self.hud = h;
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
            self.overlay.toggle();
            return Vec::new();
        }
        // In the world, Space pauses and resumes whatever has focus.
        if k == Key::Space && matches!(self.screen, Screen::InGame) {
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
            Key::F3 => Vec::new(),
        }
    }

    /// Enter or Space on the focused widget.
    fn activate_focused(&mut self, id: &str) -> Vec<AppEffect> {
        let tree = self.tree(&plain);
        match tree.find(id) {
            Some(Widget::Button { .. }) => self.activate(id),
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
            Screen::InGame => {
                self.go(Screen::Pause);
                Vec::new()
            }
            Screen::Pause => {
                self.back();
                Vec::new()
            }
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
            Screen::NewWorld(_) | Screen::SavedWorlds(_) | Screen::Options => {
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
        crate::screens::activate(self, id)
    }

    fn text(&mut self, id: &str, s: String) -> Vec<AppEffect> {
        if id.starts_with("overlay.") {
            self.overlay.text(id, s);
            return Vec::new();
        }
        crate::screens::text(self, id, s)
    }

    fn toggle(&mut self, id: &str, v: bool) -> Vec<AppEffect> {
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

    pub(crate) fn go_to(&mut self, s: Screen) {
        self.go(s);
    }

    pub(crate) fn go_back(&mut self) {
        self.back();
    }

    pub(crate) fn leave_world(&mut self) {
        self.in_world = false;
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
    pub hud: &'a HudInfo,
}
