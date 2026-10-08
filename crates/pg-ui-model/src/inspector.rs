//! The resident inspector's model (Stage 1, milestone 1.7; Blueprint section 14): what a focused resident is
//! doing and feeling, what they remember (with the reason each memory matters now), how they stand with
//! others and what they said lately. Pure data in, a widget tree out; the runtime fills [`ResidentView`]
//! from the world, the shell draws the tree.

use crate::types::Text;
use crate::widget::{Tree, Widget};
use pg_core::id::EntityId;

/// One line of a remembered conversation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Said {
    /// Text that was generated or recorded.
    Written(String),
    /// A fallback line: a string-table key with the speaker's and listener's first names to fill in.
    Key {
        key: String,
        name: String,
        other: String,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MemoryRow {
    /// String key of the one-line summary (`{other}` is filled in), if the memory has one.
    pub summary_key: Option<String>,
    pub other: String,
    pub importance: i32,
    pub persistent: bool,
    /// Why it matters now ("important (60)", "recent", ...).
    pub reasons: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RelationshipRow {
    pub other: String,
    /// The label id (`friend`, `wary`, ...); its string key is `relationship.<id>`.
    pub label: String,
    pub affinity: i32,
    pub last_topic: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConversationRow {
    pub summary_key: Option<String>,
    pub other: String,
    pub tone: String,
    /// Whether the words were recorded (generated); otherwise they are the fallback lines.
    pub recorded: bool,
    /// `(speaker's name, what was said)`.
    pub lines: Vec<(String, Said)>,
}

/// Everything the inspector shows about one resident.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResidentView {
    pub id: EntityId,
    pub name: String,
    /// The occupation template id (`barista`); its string key is `occupation.<id>`.
    pub occupation: Option<String>,
    pub mood: String,
    /// What they are doing (an action id, or `free`).
    pub activity: String,
    pub talking_with: Option<String>,
    /// `(need id, level 0..=1000)`; its string key is `need.<id>`.
    pub needs: Vec<(String, i32)>,
    pub memories: Vec<MemoryRow>,
    pub relationships: Vec<RelationshipRow>,
    pub conversations: Vec<ConversationRow>,
}

/// The inspector's window contents.
pub fn tree(v: &ResidentView, t: Text) -> Tree {
    let mut w = vec![Widget::Heading(v.name.clone())];
    let job = v
        .occupation
        .as_ref()
        .map_or_else(String::new, |o| t(&format!("occupation.{o}"), &[]));
    w.push(Widget::Label(t(
        "ui.inspect.summary",
        &[
            ("job", &job),
            ("mood", &t(&format!("mood.{}", v.mood), &[])),
        ],
    )));
    w.push(Widget::Note(match &v.talking_with {
        Some(other) => t("ui.inspect.talking", &[("other", other)]),
        None => t("ui.inspect.doing", &[("activity", &v.activity)]),
    }));
    w.push(Widget::Group {
        title: t("ui.inspect.needs", &[]),
        children: v
            .needs
            .iter()
            .map(|(id, level)| Widget::Progress {
                label: t(&format!("need.{id}"), &[]),
                permille: u32::try_from((*level).clamp(0, 1000)).unwrap_or(0),
            })
            .collect(),
    });
    w.push(Widget::Group {
        title: t("ui.inspect.memories", &[]),
        children: if v.memories.is_empty() {
            vec![Widget::Note(t("ui.inspect.none", &[]))]
        } else {
            v.memories
                .iter()
                .map(|m| {
                    let what = m
                        .summary_key
                        .as_ref()
                        .map_or_else(|| m.other.clone(), |k| t(k, &[("other", &m.other)]));
                    let why = if m.reasons.is_empty() {
                        String::new()
                    } else {
                        format!(" ({})", m.reasons.join(", "))
                    };
                    Widget::Note(format!("{what}{why}"))
                })
                .collect()
        },
    });
    w.push(Widget::Group {
        title: t("ui.inspect.relationships", &[]),
        children: if v.relationships.is_empty() {
            vec![Widget::Note(t("ui.inspect.none", &[]))]
        } else {
            v.relationships
                .iter()
                .map(|r| {
                    Widget::Note(t(
                        "ui.inspect.relationship",
                        &[
                            ("other", &r.other),
                            ("label", &t(&format!("relationship.{}", r.label), &[])),
                            ("affinity", &r.affinity.to_string()),
                        ],
                    ))
                })
                .collect()
        },
    });
    let mut talks = Vec::new();
    for c in &v.conversations {
        let head = c
            .summary_key
            .as_ref()
            .map_or_else(|| c.other.clone(), |k| t(k, &[("other", &c.other)]));
        talks.push(Widget::Label(format!(
            "{head} [{}]",
            t(
                if c.recorded {
                    "ui.inspect.recorded"
                } else {
                    "ui.inspect.rebuilt"
                },
                &[]
            )
        )));
        for (speaker, said) in &c.lines {
            let text = match said {
                Said::Written(s) => s.clone(),
                Said::Key { key, name, other } => t(key, &[("name", name), ("other", other)]),
            };
            talks.push(Widget::Note(format!("{speaker}: {text}")));
        }
    }
    if talks.is_empty() {
        talks.push(Widget::Note(t("ui.inspect.none", &[])));
    }
    w.push(Widget::Group {
        title: t("ui.inspect.conversations", &[]),
        children: talks,
    });
    Tree::new(t("ui.inspect.title", &[]), w)
}

#[cfg(test)]
mod tests {
    use super::*;
    use pg_core::id::Kind;

    fn show(key: &str, args: &[(&str, &str)]) -> String {
        if args.is_empty() {
            key.to_owned()
        } else {
            let a: Vec<String> = args.iter().map(|(k, v)| format!("{k}={v}")).collect();
            format!("{key}{{{}}}", a.join(","))
        }
    }

    fn view() -> ResidentView {
        ResidentView {
            id: EntityId::new(Kind::Pawn, 1),
            name: "Ivan Bauer".into(),
            occupation: Some("gardener".into()),
            mood: "content".into(),
            activity: "idle_at".into(),
            talking_with: None,
            needs: vec![("hunger".into(), 640), ("social".into(), 2000)],
            memories: vec![MemoryRow {
                summary_key: Some("memory.conversation.food".into()),
                other: "Tess".into(),
                importance: 40,
                persistent: false,
                reasons: vec!["recent".into(), "involves them".into()],
            }],
            relationships: vec![RelationshipRow {
                other: "Tess Fischer".into(),
                label: "friendly".into(),
                affinity: 320,
                last_topic: Some("food".into()),
            }],
            conversations: vec![ConversationRow {
                summary_key: Some("memory.conversation.food".into()),
                other: "Tess".into(),
                tone: "friendly".into(),
                recorded: false,
                lines: vec![
                    (
                        "Ivan".into(),
                        Said::Key {
                            key: "dialogue.food.1".into(),
                            name: "Ivan".into(),
                            other: "Tess".into(),
                        },
                    ),
                    ("Tess".into(), Said::Written("Same here.".into())),
                ],
            }],
        }
    }

    #[test]
    fn the_inspector_shows_state_memories_with_reasons_relationships_and_lines() {
        let snap = tree(&view(), &show).snapshot(None);
        for want in [
            "Ivan Bauer",
            "ui.inspect.summary{job=occupation.gardener,mood=mood.content}",
            "ui.inspect.doing{activity=idle_at}",
            "need.hunger",
            "memory.conversation.food{other=Tess} (recent, involves them)",
            "ui.inspect.relationship{other=Tess Fischer,label=relationship.friendly,affinity=320}",
            "dialogue.food.1{name=Ivan,other=Tess}",
            "Tess: Same here.",
            "ui.inspect.rebuilt",
        ] {
            assert!(snap.contains(want), "missing {want:?} in\n{snap}");
        }
    }

    #[test]
    fn levels_are_clamped_and_empty_sections_say_so_and_talking_replaces_the_activity() {
        let mut v = view();
        v.memories.clear();
        v.relationships.clear();
        v.conversations.clear();
        v.talking_with = Some("Tess".into());
        let snap = tree(&v, &show).snapshot(None);
        assert_eq!(snap.matches("ui.inspect.none").count(), 3, "{snap}");
        assert!(
            snap.contains("ui.inspect.talking{other=Tess}") && !snap.contains("ui.inspect.doing")
        );
        assert!(
            snap.contains("need.social: 100%"),
            "an out-of-range level is held at full: {snap}"
        );
    }
}
