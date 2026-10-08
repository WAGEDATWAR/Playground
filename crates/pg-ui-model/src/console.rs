//! The developer console's model (milestone 1.3a): a window of log lines with a text filter and one
//! switch per severity. Pure state: the app feeds it new entries; it decides what is shown.
//!
//! Opening needs developer mode (Options). The type switches start enabled except Debug, and are put back
//! to that every time a world is loaded. The text filter shows only lines that contain the text, matched
//! (without regard to case) against the line as displayed, prefix included.

use crate::types::AppEffect;
use crate::types::Text;
use crate::widget::{Tree, Widget};
use pg_host::console::{matches_filter, Entry, Severity};
use std::collections::VecDeque;

/// Entries the model holds; the oldest are dropped.
pub const KEPT: usize = 3_000;

/// Lines drawn at most (the newest that pass the filters).
pub const SHOWN: usize = 400;

#[derive(Clone, Debug)]
pub struct ConsoleModel {
    visible: bool,
    filter: String,
    enabled: [bool; 5],
    entries: VecDeque<Entry>,
    last_seq: u64,
    /// Remembered filter settings for this session: the type switches and the text.
    presets: Vec<Preset>,
}

/// Most presets kept.
pub const MAX_PRESETS: usize = 5;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Preset {
    pub enabled: [bool; 5],
    pub filter: String,
}

impl Preset {
    /// A short label: the text filter and the types that are on.
    pub fn label(&self) -> String {
        let types: String = Severity::ALL
            .iter()
            .zip(self.enabled)
            .filter(|(_, on)| *on)
            .filter_map(|(s, _)| s.name().chars().next())
            .collect();
        if self.filter.is_empty() {
            types
        } else {
            format!(
                "{types} \"{}\"",
                self.filter.chars().take(12).collect::<String>()
            )
        }
    }
}

impl Default for ConsoleModel {
    fn default() -> Self {
        ConsoleModel {
            visible: false,
            filter: String::new(),
            enabled: ConsoleModel::default_types(),
            entries: VecDeque::new(),
            last_seq: 0,
            presets: Vec::new(),
        }
    }
}

fn slot(s: Severity) -> usize {
    Severity::ALL.iter().position(|x| *x == s).unwrap_or(0)
}

impl ConsoleModel {
    /// Every type on except Debug.
    pub fn default_types() -> [bool; 5] {
        [false, true, true, true, true]
    }

    pub fn visible(&self) -> bool {
        self.visible
    }

    pub fn toggle(&mut self) {
        self.visible = !self.visible;
        if self.visible {
            // Pull everything the buffer still has when it opens.
            self.entries.clear();
            self.last_seq = 0;
        }
    }

    pub fn hide(&mut self) {
        self.visible = false;
    }

    pub fn filter(&self) -> &str {
        &self.filter
    }

    pub fn is_enabled(&self, s: Severity) -> bool {
        self.enabled.get(slot(s)).copied().unwrap_or(false)
    }

    pub fn set_enabled(&mut self, s: Severity, on: bool) {
        if let Some(e) = self.enabled.get_mut(slot(s)) {
            *e = on;
        }
    }

    /// Back to the starting filters: every type but Debug, no text (done on every world load).
    pub fn reset_filters(&mut self) {
        self.enabled = ConsoleModel::default_types();
        self.filter.clear();
    }

    /// The newest sequence number seen; the app asks for entries after it.
    pub fn last_seq(&self) -> u64 {
        self.last_seq
    }

    pub fn append(&mut self, new: Vec<Entry>) {
        for e in new {
            self.last_seq = self.last_seq.max(e.seq);
            self.entries.push_back(e);
        }
        while self.entries.len() > KEPT {
            self.entries.pop_front();
        }
    }

    /// The entries that pass the type switches and the text filter, oldest first, the newest [`SHOWN`].
    pub fn lines(&self) -> Vec<&Entry> {
        let mut v: Vec<&Entry> = self
            .entries
            .iter()
            .filter(|e| self.is_enabled(e.severity) && matches_filter(e, &self.filter))
            .collect();
        if v.len() > SHOWN {
            v.drain(..v.len() - SHOWN);
        }
        v
    }

    pub fn text(&mut self, id: &str, s: String) {
        if id == "console.filter" {
            self.filter = s.chars().take(80).collect();
        }
    }

    pub fn toggle_type(&mut self, id: &str, on: bool) {
        if let Some(sev) = id.strip_prefix("console.type.").and_then(Severity::from_id) {
            self.set_enabled(sev, on);
        }
    }

    pub fn click(&mut self, id: &str) -> Vec<AppEffect> {
        match id {
            "console.clear" => {
                self.entries.clear();
                vec![AppEffect::ClearConsole]
            }
            "console.filter_clear" => {
                self.filter.clear();
                Vec::new()
            }
            "console.close" => {
                self.visible = false;
                Vec::new()
            }
            "console.copy" => vec![AppEffect::CopyText(self.visible_text())],
            "console.save" => vec![AppEffect::SaveConsoleLog(self.visible_text())],
            "console.preset.save" => {
                let p = Preset {
                    enabled: self.enabled,
                    filter: self.filter.clone(),
                };
                if !self.presets.contains(&p) {
                    self.presets.push(p);
                    if self.presets.len() > MAX_PRESETS {
                        self.presets.remove(0);
                    }
                }
                Vec::new()
            }
            "console.preset.clear" => {
                self.presets.clear();
                Vec::new()
            }
            other => {
                if let Some(p) = other
                    .strip_prefix("console.preset.")
                    .and_then(|n| n.parse::<usize>().ok())
                    .and_then(|i| self.presets.get(i))
                {
                    self.enabled = p.enabled;
                    self.filter = p.filter.clone();
                }
                Vec::new()
            }
        }
    }

    /// The lines that pass the filters, as one block of text (what Copy and Save use).
    pub fn visible_text(&self) -> String {
        let mut out = String::new();
        for e in self.lines() {
            out.push_str(&e.line());
            out.push('\n');
        }
        out
    }

    pub fn presets(&self) -> &[Preset] {
        &self.presets
    }

    pub fn tree(&self, t: Text) -> Tree {
        let shown = self.lines();
        let mut w = vec![
            Widget::Heading(t("ui.console.title", &[])),
            Widget::TextField {
                id: "console.filter".into(),
                label: t("ui.console.filter", &[]),
                value: self.filter.clone(),
                secret: false,
                hint: t("ui.console.filter_hint", &[]),
            },
            Widget::Row(
                Severity::ALL
                    .iter()
                    .map(|s| Widget::Chip {
                        id: format!("console.type.{}", s.id()),
                        label: s.name().to_owned(),
                        on: self.is_enabled(*s),
                        tint: Some(*s),
                    })
                    .collect(),
            ),
            Widget::Row(vec![
                Widget::button("console.clear", t("ui.console.clear", &[])),
                Widget::button("console.copy", t("ui.console.copy", &[])),
                Widget::button("console.save", t("ui.console.save", &[])),
                Widget::button("console.close", t("ui.console.close", &[])),
            ]),
            Widget::Row(
                std::iter::once(Widget::button(
                    "console.preset.save",
                    t("ui.console.preset_save", &[]),
                ))
                .chain(
                    self.presets
                        .iter()
                        .enumerate()
                        .map(|(i, p)| Widget::button(&format!("console.preset.{i}"), p.label())),
                )
                .chain((!self.presets.is_empty()).then(|| {
                    Widget::button("console.preset.clear", t("ui.console.preset_clear", &[]))
                }))
                .collect(),
            ),
            Widget::Note(t(
                "ui.console.count",
                &[
                    ("shown", &shown.len().to_string()),
                    ("total", &self.entries.len().to_string()),
                ],
            )),
        ];
        w.push(Widget::Log {
            lines: shown.iter().map(|e| (e.severity, e.line())).collect(),
        });
        Tree::new(t("ui.console.title", &[]), w)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(seq: u64, severity: Severity, text: &str) -> Entry {
        Entry {
            seq,
            severity,
            source: "test".into(),
            tick: None,
            text: text.into(),
        }
    }

    fn show(key: &str, args: &[(&str, &str)]) -> String {
        if args.is_empty() {
            key.to_owned()
        } else {
            let a: Vec<String> = args.iter().map(|(k, v)| format!("{k}={v}")).collect();
            format!("{key}{{{}}}", a.join(","))
        }
    }

    fn sample() -> ConsoleModel {
        let mut c = ConsoleModel::default();
        c.toggle();
        c.append(vec![
            entry(1, Severity::Debug, "tick noise"),
            entry(2, Severity::Info, "world opened"),
            entry(3, Severity::Warn, "task failed: blocked"),
            entry(4, Severity::Error, "save failed"),
            entry(5, Severity::Fatal, "the world was frozen"),
        ]);
        c
    }

    #[test]
    fn every_type_starts_on_except_debug_and_the_filters_reset_to_that() {
        let mut c = sample();
        let on: Vec<bool> = Severity::ALL.iter().map(|s| c.is_enabled(*s)).collect();
        assert_eq!(on, [false, true, true, true, true]);
        assert_eq!(c.lines().len(), 4, "debug is hidden");
        c.set_enabled(Severity::Debug, true);
        c.set_enabled(Severity::Warn, false);
        c.text("console.filter", "world".into());
        assert_eq!(c.lines().len(), 2);
        c.reset_filters();
        assert_eq!(c.lines().len(), 4);
        assert_eq!(c.filter(), "");
        assert!(!c.is_enabled(Severity::Debug) && c.is_enabled(Severity::Warn));
    }

    #[test]
    fn disabled_types_do_not_show_and_the_text_filter_keeps_only_matching_lines() {
        let mut c = sample();
        c.toggle_type("console.type.debug", true);
        assert_eq!(c.lines().len(), 5);
        c.toggle_type("console.type.info", false);
        assert!(c.lines().iter().all(|e| e.severity != Severity::Info));
        c.text("console.filter", "FAILED".into());
        let shown: Vec<String> = c.lines().iter().map(|e| e.line()).collect();
        assert_eq!(
            shown,
            ["[Warn]: task failed: blocked", "[Error]: save failed"]
        );
        // The prefix counts: typing a type name finds that type.
        c.text("console.filter", "[error]".into());
        assert_eq!(c.lines().len(), 1);
        c.text("console.filter", "nothing like this".into());
        assert!(c.lines().is_empty());
        assert!(c.click("console.filter_clear").is_empty());
        assert_eq!(c.lines().len(), 4, "debug on, info off");
        // Unknown ids change nothing.
        c.toggle_type("console.type.loud", false);
        c.toggle_type("other", false);
        assert_eq!(c.lines().len(), 4);
    }

    #[test]
    fn the_tree_has_a_filter_box_five_type_buttons_a_clear_button_and_prefixed_lines() {
        let c = sample();
        let tree = c.tree(&show);
        let snap = tree.snapshot(None);
        for name in ["Debug", "Info", "Warn", "Error", "Fatal"] {
            assert!(snap.contains(name), "{snap}");
        }
        assert!(
            snap.contains("<console.filter>") && snap.contains("<console.clear>"),
            "{snap}"
        );
        assert!(snap.contains("[Warn]: task failed: blocked"), "{snap}");
        assert!(!snap.contains("tick noise"), "debug starts hidden: {snap}");
        assert_eq!(
            tree.focus_order(),
            [
                "console.filter",
                "console.type.debug",
                "console.type.info",
                "console.type.warn",
                "console.type.error",
                "console.type.fatal",
                "console.clear",
                "console.copy",
                "console.save",
                "console.close",
                "console.preset.save"
            ]
        );
    }

    #[test]
    fn opening_pulls_everything_again_and_clear_asks_the_app_to_clear_its_buffer() {
        let mut c = sample();
        assert_eq!(c.last_seq(), 5);
        c.toggle();
        c.toggle();
        assert_eq!(
            c.last_seq(),
            0,
            "a fresh open starts from the start of the buffer"
        );
        c.append(vec![entry(9, Severity::Info, "x")]);
        assert_eq!(c.last_seq(), 9);
        assert_eq!(c.click("console.clear"), vec![AppEffect::ClearConsole]);
        assert!(c.lines().is_empty());
        assert_eq!(c.last_seq(), 9, "what was cleared is not fetched again");
    }

    #[test]
    fn the_model_keeps_a_bounded_history_and_draws_a_bounded_window() {
        let mut c = ConsoleModel::default();
        c.append(
            (1..=(KEPT as u64 + 50))
                .map(|i| entry(i, Severity::Info, &format!("line {i}")))
                .collect(),
        );
        assert_eq!(c.entries.len(), KEPT);
        assert_eq!(c.lines().len(), SHOWN);
        assert_eq!(
            c.lines().last().unwrap().text,
            format!("line {}", KEPT + 50)
        );
    }

    #[test]
    fn copy_and_save_carry_exactly_the_visible_lines() {
        let mut c = sample();
        c.text("console.filter", "failed".into());
        let want = "[Warn]: task failed: blocked
[Error]: save failed
"
        .to_owned();
        assert_eq!(c.visible_text(), want);
        assert_eq!(
            c.click("console.copy"),
            vec![AppEffect::CopyText(want.clone())]
        );
        assert_eq!(
            c.click("console.save"),
            vec![AppEffect::SaveConsoleLog(want)]
        );
    }

    #[test]
    fn filter_presets_are_remembered_applied_deduplicated_bounded_and_survive_a_world_load() {
        let mut c = sample();
        c.set_enabled(Severity::Debug, true);
        c.set_enabled(Severity::Info, false);
        c.text("console.filter", "save".into());
        c.click("console.preset.save");
        c.click("console.preset.save");
        assert_eq!(c.presets().len(), 1, "the same filter is not saved twice");
        assert_eq!(c.presets()[0].label(), "DWEF \"save\"");
        c.reset_filters();
        assert!(!c.is_enabled(Severity::Debug) && c.filter().is_empty());
        assert_eq!(c.presets().len(), 1, "presets last the session");
        c.click("console.preset.0");
        assert!(c.is_enabled(Severity::Debug) && !c.is_enabled(Severity::Info));
        assert_eq!(c.filter(), "save");
        for i in 0..8 {
            c.text("console.filter", format!("f{i}"));
            c.click("console.preset.save");
        }
        assert_eq!(c.presets().len(), MAX_PRESETS);
        assert_eq!(c.presets().last().unwrap().filter, "f7");
        let snap = c.tree(&show).snapshot(None);
        assert!(
            snap.contains("<console.preset.4>") && snap.contains("<console.preset.clear>"),
            "{snap}"
        );
        c.click("console.preset.99");
        c.click("console.preset.clear");
        assert!(c.presets().is_empty());
    }
}
