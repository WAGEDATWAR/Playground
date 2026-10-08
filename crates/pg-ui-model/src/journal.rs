//! The town journal's model (Stage 1, milestone 1.7; suggestion S-058): a short, readable list of what
//! happened in the town that is worth noticing (new friendships and rifts, a resident who collapsed, a warm
//! talk), newest first. The runtime turns events into [`JournalEntry`]s; this turns entries into a window.
//!
//! The journal belongs to observation and possession modes. Player mode (a world-creation choice that locks
//! the player into one resident, built with possession) does not get it; [`available`] is where that rule
//! lives, so the shell asks one place.

use crate::types::Text;
use crate::widget::{Tree, Widget};

/// How much an entry matters: 1 routine, 2 notable, 3 serious.
pub type Weight = u8;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JournalEntry {
    pub tick: u64,
    pub day: u64,
    pub minute_of_day: u32,
    /// Selects the sentence: its string key is `journal.<kind>`.
    pub kind: String,
    /// Fill-ins for the sentence. An argument whose name ends in `_key` holds a string key, which is looked
    /// up before filling in (the name loses the suffix).
    pub args: Vec<(String, String)>,
    pub weight: Weight,
}

/// Which way the player is playing.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Default)]
pub enum Mode {
    /// Watching the town, with the ability to possess a resident (the only mode that exists so far).
    #[default]
    Observation,
    /// Locked into one chosen resident (not built yet).
    Player,
}

/// Whether the journal may be shown in this mode.
pub fn available(mode: Mode) -> bool {
    mode != Mode::Player
}

fn hhmm(minute: u32) -> String {
    format!("{:02}:{:02}", minute / 60 % 24, minute % 60)
}

/// The sentence of one entry.
pub fn sentence(e: &JournalEntry, t: Text) -> String {
    let resolved: Vec<(String, String)> = e
        .args
        .iter()
        .map(|(k, v)| match k.strip_suffix("_key") {
            Some(base) => (base.to_owned(), t(v, &[])),
            None => (k.clone(), v.clone()),
        })
        .collect();
    let args: Vec<(&str, &str)> = resolved
        .iter()
        .map(|(k, v)| (k.as_str(), v.as_str()))
        .collect();
    t(&format!("journal.{}", e.kind), &args)
}

/// The journal window: entries newest first.
pub fn tree(entries: &[JournalEntry], t: Text) -> Tree {
    let mut w = vec![Widget::Heading(t("ui.journal.title", &[]))];
    if entries.is_empty() {
        w.push(Widget::Note(t("ui.journal.empty", &[])));
    }
    for e in entries.iter().rev() {
        let when = t(
            "ui.journal.when",
            &[
                ("day", &e.day.to_string()),
                ("time", &hhmm(e.minute_of_day)),
            ],
        );
        let line = format!("{when}  {}", sentence(e, t));
        w.push(if e.weight >= 2 {
            Widget::Label(line)
        } else {
            Widget::Note(line)
        });
    }
    w.push(Widget::button("journal.close", t("ui.journal.close", &[])));
    Tree::new(t("ui.journal.title", &[]), w)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn show(key: &str, args: &[(&str, &str)]) -> String {
        if args.is_empty() {
            key.to_owned()
        } else {
            let a: Vec<String> = args.iter().map(|(k, v)| format!("{k}={v}")).collect();
            format!("{key}{{{}}}", a.join(","))
        }
    }

    fn entry(kind: &str, weight: Weight, args: &[(&str, &str)]) -> JournalEntry {
        JournalEntry {
            tick: 100,
            day: 2,
            minute_of_day: 14 * 60 + 5,
            kind: kind.into(),
            args: args
                .iter()
                .map(|(k, v)| ((*k).into(), (*v).into()))
                .collect(),
            weight,
        }
    }

    #[test]
    fn entries_read_newest_first_with_their_time_and_keys_are_looked_up() {
        let entries = [
            entry("collapsed", 3, &[("pawn", "Ivan")]),
            entry(
                "label",
                2,
                &[
                    ("a", "Ivan"),
                    ("b", "Tess"),
                    ("label_key", "relationship.friend"),
                ],
            ),
        ];
        let snap = tree(&entries, &show).snapshot(None);
        let newer = snap.find("journal.label").unwrap();
        let older = snap.find("journal.collapsed").unwrap();
        assert!(newer < older, "newest first:\n{snap}");
        assert!(
            snap.contains("journal.label{a=Ivan,b=Tess,label=relationship.friend}"),
            "{snap}"
        );
        assert!(snap.contains("ui.journal.when{day=2,time=14:05}"), "{snap}");
        assert!(snap.contains("<journal.close>"), "{snap}");
    }

    #[test]
    fn an_empty_journal_says_so_and_player_mode_does_not_get_one() {
        let snap = tree(&[], &show).snapshot(None);
        assert!(snap.contains("ui.journal.empty"), "{snap}");
        assert!(available(Mode::Observation));
        assert!(!available(Mode::Player));
    }
}
