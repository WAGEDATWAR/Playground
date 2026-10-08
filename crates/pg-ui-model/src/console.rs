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
}

impl Default for ConsoleModel {
    fn default() -> Self {
        ConsoleModel {
            visible: false,
            filter: String::new(),
            enabled: ConsoleModel::default_types(),
            entries: VecDeque::new(),
            last_seq: 0,
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
            _ => Vec::new(),
        }
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
                Widget::button("console.close", t("ui.console.close", &[])),
            ]),
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
                "console.close"
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
}
