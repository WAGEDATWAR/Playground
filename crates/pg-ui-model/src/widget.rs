//! The widget tree every screen produces (suggestion S-027, Blueprint §14).
//!
//! Screens do not draw. They describe what is on screen as a small tree of labelled widgets with stable
//! ids and a focus order, and the egui layer maps that tree to real widgets. Because the tree is plain
//! data it can be snapshotted as text, checked for keyboard reachability, and handed to the platform's
//! accessibility layer: a button is a button with a name whether it is drawn or not.

use crate::layout::{Align, DrawerLayout};
use pg_host::console::Severity;
use std::fmt::Write;

/// A stable identifier such as `menu.continue`. Ids never contain spaces.
pub type WidgetId = String;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Widget {
    Heading(String),
    Label(String),
    /// A line of smaller, secondary text.
    Note(String),
    Spacer,
    Button {
        id: WidgetId,
        label: String,
        enabled: bool,
    },
    TextField {
        id: WidgetId,
        label: String,
        value: String,
        /// A secret is shown as dots, never in snapshots or logs.
        secret: bool,
        /// Text shown while empty.
        hint: String,
    },
    Toggle {
        id: WidgetId,
        label: String,
        value: bool,
    },
    Choice {
        id: WidgetId,
        label: String,
        /// `(value, label)`.
        options: Vec<(String, String)>,
        value: String,
    },
    Slider {
        id: WidgetId,
        label: String,
        min: i64,
        max: i64,
        value: i64,
    },
    Progress {
        label: String,
        /// Thousandths, 0..=1000.
        permille: u32,
    },
    /// A saved picture, named by the storage blob it lives in; the shell loads and draws it.
    Thumbnail {
        name: String,
    },
    /// A button that opens a panel of choices (a list or a grid) on the side with the most room. The model
    /// keeps which drawer is open; the shell places and scrolls the panel (see [`crate::layout`]).
    Drawer {
        id: WidgetId,
        /// The button's text.
        label: String,
        open: bool,
        layout: DrawerLayout,
        align: Align,
        items: Vec<DrawerItem>,
    },
    /// A switch drawn as a button that stays pressed while on; `tint` colours it (the console's types).
    Chip {
        id: WidgetId,
        label: String,
        on: bool,
        tint: Option<Severity>,
    },
    /// Lines of log text, each at a severity (the shell colours them); scrolls and follows the newest.
    Log {
        lines: Vec<(Severity, String)>,
    },
    /// Widgets laid out side by side.
    Row(Vec<Widget>),
    /// A group with a title; its contents follow vertically.
    Group {
        title: String,
        children: Vec<Widget>,
    },
}

/// One choice in a drawer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DrawerItem {
    pub id: WidgetId,
    pub label: String,
    pub selected: bool,
}

impl Widget {
    pub fn button(id: &str, label: impl Into<String>) -> Widget {
        Widget::Button {
            id: id.to_owned(),
            label: label.into(),
            enabled: true,
        }
    }

    pub fn disabled_button(id: &str, label: impl Into<String>) -> Widget {
        Widget::Button {
            id: id.to_owned(),
            label: label.into(),
            enabled: false,
        }
    }

    pub fn id(&self) -> Option<&str> {
        match self {
            Widget::Button { id, .. }
            | Widget::TextField { id, .. }
            | Widget::Toggle { id, .. }
            | Widget::Choice { id, .. }
            | Widget::Slider { id, .. }
            | Widget::Chip { id, .. }
            | Widget::Drawer { id, .. } => Some(id),
            _ => None,
        }
    }

    /// Whether the widget takes keyboard focus.
    pub fn focusable(&self) -> bool {
        match self {
            Widget::Button { enabled, .. } => *enabled,
            Widget::TextField { .. }
            | Widget::Toggle { .. }
            | Widget::Choice { .. }
            | Widget::Slider { .. }
            | Widget::Chip { .. }
            | Widget::Drawer { .. } => true,
            _ => false,
        }
    }

    fn visit<'a>(&'a self, f: &mut impl FnMut(&'a Widget)) {
        f(self);
        match self {
            Widget::Row(c) | Widget::Group { children: c, .. } => {
                for w in c {
                    w.visit(f);
                }
            }
            _ => {}
        }
    }
}

/// A screen as data.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Tree {
    pub title: String,
    pub widgets: Vec<Widget>,
}

impl Tree {
    pub fn new(title: impl Into<String>, widgets: Vec<Widget>) -> Tree {
        Tree {
            title: title.into(),
            widgets,
        }
    }

    /// Every widget in reading order, containers included.
    pub fn walk(&self) -> Vec<&Widget> {
        let mut out = Vec::new();
        for w in &self.widgets {
            w.visit(&mut |x| out.push(x));
        }
        out
    }

    /// The focus order: ids of the focusable widgets in reading order. An open drawer is followed by its
    /// items, so the keyboard can reach them.
    pub fn focus_order(&self) -> Vec<&str> {
        let mut out = Vec::new();
        for w in self.walk() {
            if !w.focusable() {
                continue;
            }
            if let Some(id) = w.id() {
                out.push(id);
            }
            if let Widget::Drawer {
                open: true, items, ..
            } = w
            {
                out.extend(items.iter().map(|i| i.id.as_str()));
            }
        }
        out
    }

    /// The open drawer holding `item_id`, with the item's position.
    pub fn drawer_of(&self, item_id: &str) -> Option<(&Widget, usize)> {
        self.walk().into_iter().find_map(|w| match w {
            Widget::Drawer {
                open: true, items, ..
            } => items.iter().position(|i| i.id == item_id).map(|p| (w, p)),
            _ => None,
        })
    }

    pub fn find(&self, id: &str) -> Option<&Widget> {
        self.walk().into_iter().find(|w| w.id() == Some(id))
    }

    /// A plain-text rendering used by snapshot tests and the accessibility dump. Secrets show as dots.
    pub fn snapshot(&self, focus: Option<&str>) -> String {
        let mut out = String::new();
        let _ = writeln!(out, "[{}]", self.title);
        for w in &self.widgets {
            snapshot_widget(&mut out, w, 1, focus);
        }
        out
    }
}

fn snapshot_widget(out: &mut String, w: &Widget, depth: usize, focus: Option<&str>) {
    let pad = "  ".repeat(depth);
    let mark = |id: &str| if focus == Some(id) { ">" } else { " " };
    match w {
        Widget::Heading(t) => {
            let _ = writeln!(out, "{pad}# {t}");
        }
        Widget::Label(t) => {
            let _ = writeln!(out, "{pad}{t}");
        }
        Widget::Note(t) => {
            let _ = writeln!(out, "{pad}({t})");
        }
        Widget::Spacer => {}
        Widget::Button { id, label, enabled } => {
            let _ = writeln!(
                out,
                "{pad}{}[{label}]{} <{id}>",
                mark(id),
                if *enabled { "" } else { " (disabled)" }
            );
        }
        Widget::TextField {
            id,
            label,
            value,
            secret,
            hint,
        } => {
            let shown = if *secret {
                "\u{2022}".repeat(value.chars().count())
            } else {
                value.clone()
            };
            let shown = if shown.is_empty() {
                format!("<{hint}>")
            } else {
                shown
            };
            let _ = writeln!(out, "{pad}{}{label}: [{shown}] <{id}>", mark(id));
        }
        Widget::Toggle { id, label, value } => {
            let _ = writeln!(
                out,
                "{pad}{}[{}] {label} <{id}>",
                mark(id),
                if *value { "x" } else { " " }
            );
        }
        Widget::Choice {
            id,
            label,
            options,
            value,
        } => {
            let shown = options
                .iter()
                .find(|(v, _)| v == value)
                .map_or(value.as_str(), |(_, l)| l.as_str());
            let _ = writeln!(out, "{pad}{}{label}: < {shown} > <{id}>", mark(id));
        }
        Widget::Slider {
            id,
            label,
            min,
            max,
            value,
        } => {
            let _ = writeln!(
                out,
                "{pad}{}{label}: {value} ({min}..{max}) <{id}>",
                mark(id)
            );
        }
        Widget::Progress { label, permille } => {
            let _ = writeln!(out, "{pad}{label}: {}%", permille / 10);
        }
        Widget::Thumbnail { name } => {
            let _ = writeln!(out, "{pad}(picture {name})");
        }
        Widget::Chip { id, label, on, .. } => {
            let _ = writeln!(
                out,
                "{pad}{}({}) {label} <{id}>",
                mark(id),
                if *on { "on" } else { "off" }
            );
        }
        Widget::Log { lines } => {
            for (_, text) in lines {
                let _ = writeln!(out, "{pad}{text}");
            }
        }
        Widget::Drawer {
            id,
            label,
            open,
            items,
            ..
        } => {
            let _ = writeln!(
                out,
                "{pad}{}[{label} {}] <{id}>",
                mark(id),
                if *open { "^" } else { "v" }
            );
            if *open {
                for i in items {
                    let _ = writeln!(
                        out,
                        "{pad}  {}{}[{}] <{}>",
                        mark(&i.id),
                        if i.selected { "*" } else { " " },
                        i.label,
                        i.id
                    );
                }
            }
        }
        Widget::Row(c) => {
            let _ = writeln!(out, "{pad}row:");
            for x in c {
                snapshot_widget(out, x, depth + 1, focus);
            }
        }
        Widget::Group { title, children } => {
            let _ = writeln!(out, "{pad}== {title} ==");
            for x in children {
                snapshot_widget(out, x, depth + 1, focus);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Tree {
        Tree::new(
            "Sample",
            vec![
                Widget::Heading("Hello".into()),
                Widget::Row(vec![
                    Widget::button("a", "A"),
                    Widget::disabled_button("b", "B"),
                ]),
                Widget::Group {
                    title: "G".into(),
                    children: vec![
                        Widget::Toggle {
                            id: "t".into(),
                            label: "T".into(),
                            value: true,
                        },
                        Widget::TextField {
                            id: "k".into(),
                            label: "Key".into(),
                            value: "secret".into(),
                            secret: true,
                            hint: "paste".into(),
                        },
                    ],
                },
            ],
        )
    }

    #[test]
    fn focus_order_is_reading_order_and_skips_disabled_widgets() {
        assert_eq!(sample().focus_order(), ["a", "t", "k"]);
        assert!(sample().find("b").is_some());
        assert!(sample().find("zz").is_none());
    }

    #[test]
    fn the_snapshot_marks_focus_and_hides_secrets() {
        let s = sample().snapshot(Some("t"));
        assert!(s.contains(">[x] T <t>"), "{s}");
        assert!(s.contains("[A] <a>"), "{s}");
        assert!(s.contains("(disabled)"), "{s}");
        assert!(
            s.contains("Key: [\u{2022}\u{2022}\u{2022}\u{2022}\u{2022}\u{2022}]"),
            "{s}"
        );
        assert!(!s.contains("secret"), "{s}");
    }
}
