//! The developer overlay's model (Blueprint §20, milestone 0.10).
//!
//! The overlay is where the developer tools meet the running game: tick and hash, per-system times, the
//! event viewer with the S-009 prefix filter and the S-022 catalog, the reason-code explorer (S-010),
//! pack status and quarantine, script cost per pack and hook (S-036), the keyframe ring with time scrub
//! (S-029) and the bug-bundle button (S-001). The app fills [`OverlayData`] from the runtime each frame
//! while the overlay is visible; this module turns it into a widget tree and turns clicks into effects.

use crate::types::{AppEffect, Text};
use crate::widget::{Tree, Widget};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SystemRow {
    pub name: String,
    pub calls: u64,
    pub avg_micros: u64,
    pub share_permille: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EventRow {
    pub tick: u64,
    pub kind: String,
    pub text: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReasonRow {
    pub subject: String,
    pub text: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PackRow {
    pub id: String,
    pub version: String,
    pub status: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScriptRow {
    pub pack: String,
    pub point: String,
    pub calls: u64,
    pub fuel: u64,
    pub errors: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct OverlayData {
    pub tick: u64,
    pub day: u64,
    pub hash: String,
    pub state: String,
    pub systems: Vec<SystemRow>,
    pub events: Vec<EventRow>,
    pub reasons: Vec<ReasonRow>,
    pub packs: Vec<PackRow>,
    pub scripts: Vec<ScriptRow>,
    pub keyframes: Vec<u64>,
    pub shadow: String,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Tab {
    Time,
    Systems,
    Events,
    Reasons,
    Packs,
    Scripts,
}

impl Tab {
    pub const ALL: [Tab; 6] = [
        Tab::Time,
        Tab::Systems,
        Tab::Events,
        Tab::Reasons,
        Tab::Packs,
        Tab::Scripts,
    ];

    pub const fn id(self) -> &'static str {
        match self {
            Tab::Time => "time",
            Tab::Systems => "systems",
            Tab::Events => "events",
            Tab::Reasons => "reasons",
            Tab::Packs => "packs",
            Tab::Scripts => "scripts",
        }
    }
}

/// Events shown at most.
pub const MAX_EVENT_ROWS: usize = 40;

#[derive(Clone, Debug)]
pub struct Overlay {
    visible: bool,
    tab: Tab,
    filter: String,
    data: OverlayData,
}

impl Default for Overlay {
    fn default() -> Self {
        Overlay {
            visible: false,
            tab: Tab::Time,
            filter: String::new(),
            data: OverlayData::default(),
        }
    }
}

impl Overlay {
    pub fn visible(&self) -> bool {
        self.visible
    }

    pub fn tab(&self) -> Tab {
        self.tab
    }

    pub fn filter(&self) -> &str {
        &self.filter
    }

    pub fn toggle(&mut self) {
        self.visible = !self.visible;
    }

    pub fn set_data(&mut self, d: OverlayData) {
        self.data = d;
    }

    pub fn text(&mut self, id: &str, s: String) {
        if id == "overlay.filter" {
            self.filter = s.chars().take(64).collect();
        }
    }

    pub fn click(&mut self, id: &str) -> Vec<AppEffect> {
        if let Some(tab) = id.strip_prefix("overlay.tab.") {
            if let Some(t) = Tab::ALL.into_iter().find(|t| t.id() == tab) {
                self.tab = t;
            }
            return Vec::new();
        }
        if let Some(tick) = id.strip_prefix("overlay.scrub.") {
            return tick
                .parse::<u64>()
                .map(|tick| vec![AppEffect::Rewind { tick }])
                .unwrap_or_default();
        }
        match id {
            "overlay.bundle" => vec![AppEffect::CutBundle],
            "overlay.close" => {
                self.visible = false;
                Vec::new()
            }
            "overlay.filter_clear" => {
                self.filter.clear();
                Vec::new()
            }
            _ => Vec::new(),
        }
    }

    /// The events that pass the prefix filter (S-009), newest last, at most [`MAX_EVENT_ROWS`].
    pub fn filtered_events(&self) -> Vec<&crate::overlay::EventRow> {
        let mut rows: Vec<&EventRow> = self
            .data
            .events
            .iter()
            .filter(|e| e.kind.starts_with(self.filter.as_str()))
            .collect();
        if rows.len() > MAX_EVENT_ROWS {
            rows.drain(..rows.len() - MAX_EVENT_ROWS);
        }
        rows
    }

    pub fn tree(&self, t: Text) -> Tree {
        let d = &self.data;
        let mut w = vec![Widget::Row(
            Tab::ALL
                .iter()
                .map(|tab| {
                    let label = t(&format!("ui.overlay.tab.{}", tab.id()), &[]);
                    Widget::button(
                        &format!("overlay.tab.{}", tab.id()),
                        if *tab == self.tab { format!("[{label}]") } else { label },
                    )
                })
                .chain([Widget::button("overlay.close", t("ui.overlay.close", &[]))])
                .collect(),
        )];
        match self.tab {
            Tab::Time => {
                w.push(Widget::Label(t(
                    "ui.overlay.time",
                    &[("tick", &d.tick.to_string()), ("day", &d.day.to_string()), ("state", &d.state)],
                )));
                w.push(Widget::Label(t("ui.overlay.hash", &[("hash", &d.hash)])));
                if !d.shadow.is_empty() {
                    w.push(Widget::Label(t("ui.overlay.shadow", &[("status", &d.shadow)])));
                }
                w.push(Widget::Heading(t("ui.overlay.keyframes", &[])));
                w.push(Widget::Row(
                    d.keyframes
                        .iter()
                        .rev()
                        .take(8)
                        .map(|k| Widget::button(&format!("overlay.scrub.{k}"), k.to_string()))
                        .collect(),
                ));
                w.push(Widget::button("overlay.bundle", t("ui.overlay.bundle", &[])));
            }
            Tab::Systems => {
                for s in &d.systems {
                    w.push(Widget::Label(t(
                        "ui.overlay.system",
                        &[
                            ("name", &s.name),
                            ("avg", &s.avg_micros.to_string()),
                            ("calls", &s.calls.to_string()),
                            ("share", &format!("{}.{}", s.share_permille / 10, s.share_permille % 10)),
                        ],
                    )));
                }
            }
            Tab::Events => {
                w.push(Widget::TextField {
                    id: "overlay.filter".into(),
                    label: t("ui.overlay.filter", &[]),
                    value: self.filter.clone(),
                    secret: false,
                    hint: t("ui.overlay.filter_hint", &[]),
                });
                w.push(Widget::button("overlay.filter_clear", t("ui.overlay.filter_clear", &[])));
                for e in self.filtered_events() {
                    w.push(Widget::Label(format!("{}  {}  {}", e.tick, e.kind, e.text)));
                }
            }
            Tab::Reasons => {
                for r in &d.reasons {
                    w.push(Widget::Label(format!("{}: {}", r.subject, r.text)));
                }
            }
            Tab::Packs => {
                for p in &d.packs {
                    w.push(Widget::Label(format!("{} {}  {}", p.id, p.version, p.status)));
                }
            }
            Tab::Scripts => {
                for s in &d.scripts {
                    w.push(Widget::Label(t(
                        "ui.overlay.script",
                        &[
                            ("pack", &s.pack),
                            ("point", &s.point),
                            ("calls", &s.calls.to_string()),
                            ("fuel", &s.fuel.to_string()),
                            ("errors", &s.errors.to_string()),
                        ],
                    )));
                }
            }
        }
        Tree::new(t("ui.overlay.title", &[]), w)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plain(k: &str, _: &[(&str, &str)]) -> String {
        k.to_owned()
    }

    #[test]
    fn the_event_filter_matches_prefixes_and_keeps_the_newest() {
        let mut o = Overlay::default();
        let events: Vec<EventRow> = (0..100)
            .map(|i| EventRow {
                tick: i,
                kind: if i % 2 == 0 { "move.arrived".into() } else { "commitment.accepted".into() },
                text: String::new(),
            })
            .collect();
        o.set_data(OverlayData { events, ..OverlayData::default() });
        assert_eq!(o.filtered_events().len(), MAX_EVENT_ROWS);
        assert_eq!(o.filtered_events().last().unwrap().tick, 99);
        o.text("overlay.filter", "move".into());
        assert!(o.filtered_events().iter().all(|e| e.kind.starts_with("move")));
        assert_eq!(o.filtered_events().len(), 40);
        o.text("overlay.filter", "zzz".into());
        assert!(o.filtered_events().is_empty());
        o.click("overlay.filter_clear");
        assert_eq!(o.filter(), "");
    }

    #[test]
    fn tabs_scrub_and_bundle_become_effects_or_state() {
        let mut o = Overlay::default();
        o.toggle();
        assert!(o.visible());
        for t in Tab::ALL {
            o.click(&format!("overlay.tab.{}", t.id()));
            assert_eq!(o.tab(), t);
            assert!(!o.tree(&plain).widgets.is_empty());
        }
        assert_eq!(o.click("overlay.scrub.1800"), vec![AppEffect::Rewind { tick: 1800 }]);
        assert!(o.click("overlay.scrub.abc").is_empty());
        assert_eq!(o.click("overlay.bundle"), vec![AppEffect::CutBundle]);
        o.click("overlay.close");
        assert!(!o.visible());
    }

    #[test]
    fn script_cost_rows_are_shown() {
        let mut o = Overlay::default();
        o.set_data(OverlayData {
            scripts: vec![ScriptRow { pack: "caffeine".into(), point: "system decay".into(), calls: 12, fuel: 300, errors: 0 }],
            ..OverlayData::default()
        });
        o.click("overlay.tab.scripts");
        let s = o.tree(&plain).snapshot(None);
        assert!(s.contains("ui.overlay.script"), "{s}");
    }
}
