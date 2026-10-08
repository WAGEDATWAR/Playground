//! The typed event catalog (suggestion S-022, Blueprint §6.2).
//!
//! Events stay `(kind, detail)` values and are not part of hashed state, but every kind is declared once
//! here: its category, its detail fields (the same schema machinery as components) and whether it shows by
//! default in the event viewer. Debug builds validate every event a step emits (so a mistyped kind or a
//! forgotten field fails a test immediately), `pg events list` prints the catalog, and the overlay's event
//! viewer reads the same declarations. Packs add events as `<pack>.<kind>` through [`EventCatalog::register`].

use crate::canon::Canon;
use crate::pipeline::Event;
use pg_content::schema::{Field, FieldSchema, ParamSchema};
use pg_content::ValidationReport;
use pg_host::console::Severity;
use std::collections::BTreeMap;
use std::sync::OnceLock;

#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Category {
    Input,
    World,
    Movement,
    Schedule,
    Task,
    Commitment,
    /// Needs, mood and what they do to a pawn.
    Life,
    /// Conversations, memories and relationships.
    Social,
    Dev,
    /// Declared by a content pack.
    Pack,
}

impl Category {
    pub const fn name(self) -> &'static str {
        match self {
            Category::Input => "input",
            Category::World => "world",
            Category::Movement => "movement",
            Category::Schedule => "schedule",
            Category::Task => "task",
            Category::Commitment => "commitment",
            Category::Life => "life",
            Category::Social => "social",
            Category::Dev => "dev",
            Category::Pack => "pack",
        }
    }
}

#[derive(Clone, Debug)]
pub struct EventDef {
    pub kind: String,
    pub category: Category,
    pub fields: ParamSchema,
    /// Shown in the event viewer unless the player filters it out (noisy kinds default to hidden).
    pub default_visible: bool,
    pub summary: String,
    /// How the developer console shows it (milestone 1.3a). Routine detail is Debug; what a developer should
    /// notice is Info, Warn or Error. Every built-in kind is listed in [`SEVERITIES`].
    pub severity: Severity,
}

/// The console severity of every built-in event kind. A new kind must be listed here (a test fails until it
/// is), so nothing the simulation reports is missing from the console or shown at an unconsidered level.
pub const SEVERITIES: &[(&str, Severity)] = &[
    ("input_rejected", Severity::Warn),
    ("script.error", Severity::Error),
    ("script.quarantined", Severity::Error),
    ("script.log", Severity::Info),
    ("setting_changed", Severity::Info),
    ("dev.nudged", Severity::Debug),
    ("dev.day_started", Severity::Debug),
    ("map.created", Severity::Info),
    ("town.generated", Severity::Info),
    ("pawn.spawned", Severity::Debug),
    ("object.spawned", Severity::Debug),
    ("object.contained", Severity::Debug),
    ("world_edited", Severity::Info),
    ("move.requested", Severity::Debug),
    ("move.arrived", Severity::Debug),
    ("move.failed", Severity::Warn),
    ("move.sidestep", Severity::Debug),
    ("schedule.planned", Severity::Debug),
    ("schedule.replanned", Severity::Debug),
    ("need.urgent", Severity::Info),
    ("need.critical", Severity::Warn),
    ("pawn.collapsed", Severity::Warn),
    ("pawn.recovered", Severity::Info),
    ("mood.changed", Severity::Debug),
    ("conversation.started", Severity::Debug),
    ("conversation.closed", Severity::Info),
    ("conversation.cancelled", Severity::Debug),
    ("memory.created", Severity::Debug),
    ("memory.expired", Severity::Debug),
    ("relationship.label_changed", Severity::Info),
    ("task.started", Severity::Debug),
    ("task.done", Severity::Debug),
    ("task.failed", Severity::Warn),
    ("commitment.proposed", Severity::Debug),
    ("commitment.accepted", Severity::Debug),
    ("commitment.declined", Severity::Info),
    ("commitment.expired", Severity::Info),
    ("commitment.active", Severity::Debug),
    ("commitment.completed", Severity::Info),
    ("commitment.failed", Severity::Warn),
    ("commitment.cancelled", Severity::Info),
];

#[derive(Clone, Debug, Default)]
pub struct EventCatalog {
    defs: BTreeMap<String, EventDef>,
}

fn id(kind: &str) -> Field {
    Field::required(FieldSchema::EntityId {
        kind: kind.to_owned(),
    })
}

fn tile() -> Field {
    Field::required(FieldSchema::Tile)
}

fn int() -> Field {
    Field::required_int(0, i64::MAX)
}

/// A signed whole number (a change that can be negative).
fn signed() -> Field {
    Field::required_int(-1_000_000_000, 1_000_000_000)
}

fn text(max: usize) -> Field {
    Field::required_text(max)
}

fn maybe_text(max: usize) -> Field {
    Field::maybe(FieldSchema::Text { max_len: max })
}

fn schema(fields: &[(&str, Field)]) -> ParamSchema {
    fields
        .iter()
        .fold(ParamSchema::new(), |s, (n, f)| s.field(n, f.clone()))
}

impl EventCatalog {
    pub fn new() -> EventCatalog {
        EventCatalog::default()
    }

    /// The engine's own events.
    pub fn builtin() -> EventCatalog {
        let mut c = EventCatalog::new();
        let mut add =
            |kind: &str, category, visible: bool, summary: &str, fields: &[(&str, Field)]| {
                c.defs.insert(
                    kind.to_owned(),
                    EventDef {
                        kind: kind.to_owned(),
                        category,
                        fields: schema(fields),
                        default_visible: visible,
                        summary: summary.to_owned(),
                        severity: SEVERITIES
                            .iter()
                            .find(|(k, _)| *k == kind)
                            .map_or(Severity::Debug, |(_, s)| *s),
                    },
                );
            };
        use Category::*;
        add(
            "input_rejected",
            Input,
            true,
            "A command or setting change was refused.",
            &[("command", maybe_text(64)), ("reason", text(400))],
        );
        add(
            "script.error",
            Pack,
            true,
            "A pack's script failed; the call was discarded.",
            &[
                ("pack", text(64)),
                ("point", text(96)),
                ("message", text(400)),
            ],
        );
        add(
            "script.quarantined",
            Pack,
            true,
            "A pack was switched off for the session after repeated script errors.",
            &[("pack", text(64)), ("reason", text(200))],
        );
        add(
            "script.log",
            Pack,
            true,
            "A script pack printed a line with pg.log.info or pg.log.warn.",
            &[("pack", text(64)), ("level", text(8)), ("text", text(400))],
        );
        add(
            "setting_changed",
            Input,
            true,
            "A setting changed.",
            &[("key", text(64)), ("value", int())],
        );
        add(
            "dev.nudged",
            Dev,
            true,
            "Developer command: the probe value changed.",
            &[(
                "amount",
                Field::required_int(i64::from(i32::MIN), i64::from(i32::MAX)),
            )],
        );
        add(
            "dev.day_started",
            Dev,
            true,
            "Developer scaffolding: a day began.",
            &[("day", int())],
        );
        add(
            "map.created",
            World,
            true,
            "A map was created.",
            &[
                ("map", id("map")),
                ("w", int()),
                ("h", int()),
                ("style", Field::required_int(0, 255)),
            ],
        );
        add(
            "pawn.spawned",
            World,
            true,
            "A pawn was created.",
            &[("pawn", id("pawn")), ("tile", tile())],
        );
        add(
            "object.spawned",
            World,
            true,
            "An object was created.",
            &[("object", id("obj")), ("tile", tile())],
        );
        add(
            "object.contained",
            World,
            true,
            "An object was put in a container.",
            &[
                ("object", id("obj")),
                ("owner", id("obj")),
                ("container", text(64)),
            ],
        );
        add(
            "world_edited",
            World,
            true,
            "A map tile was edited.",
            &[
                ("map", id("map")),
                ("tile", tile()),
                ("blocked", Field::required(FieldSchema::Bool)),
            ],
        );
        add(
            "move.requested",
            Movement,
            true,
            "A pawn was sent somewhere.",
            &[("pawn", id("pawn")), ("to", tile())],
        );
        add(
            "move.arrived",
            Movement,
            false,
            "A pawn reached its destination.",
            &[("pawn", id("pawn")), ("tile", tile())],
        );
        add(
            "move.failed",
            Movement,
            true,
            "A pawn's route failed.",
            &[("pawn", id("pawn")), ("reason", text(64))],
        );
        add(
            "move.sidestep",
            Movement,
            false,
            "A pawn stepped aside to get past another.",
            &[("pawn", id("pawn")), ("tile", tile())],
        );
        add(
            "schedule.planned",
            Schedule,
            true,
            "A pawn's day was planned.",
            &[
                ("pawn", id("pawn")),
                ("day", int()),
                ("reservations", int()),
                ("dropped", int()),
            ],
        );
        add(
            "schedule.replanned",
            Schedule,
            true,
            "The rest of a pawn's day was replanned.",
            &[("pawn", id("pawn")), ("from", int()), ("why", text(400))],
        );
        add(
            "town.generated",
            World,
            true,
            "The town was generated from the world's seed.",
            &[
                ("map", id("map")),
                ("districts", int()),
                ("buildings", int()),
                ("residents", int()),
                ("attempt", int()),
                ("hash", text(64)),
            ],
        );
        add(
            "need.urgent",
            Life,
            true,
            "A need fell below its urgent level.",
            &[("pawn", id("pawn")), ("need", text(24)), ("value", int())],
        );
        add(
            "need.critical",
            Life,
            true,
            "A need fell below its critical level.",
            &[("pawn", id("pawn")), ("need", text(24)), ("value", int())],
        );
        add(
            "pawn.collapsed",
            Life,
            true,
            "A pawn was too exhausted to carry on and collapsed where it stood.",
            &[("pawn", id("pawn"))],
        );
        add(
            "pawn.recovered",
            Life,
            true,
            "A collapsed pawn recovered enough to act again.",
            &[("pawn", id("pawn"))],
        );
        add(
            "mood.changed",
            Life,
            false,
            "A pawn's mood changed.",
            &[
                ("pawn", id("pawn")),
                ("from", text(24)),
                ("to", text(24)),
                ("rule", text(48)),
            ],
        );
        add(
            "conversation.started",
            Social,
            false,
            "Two residents began a conversation.",
            &[
                ("a", id("pawn")),
                ("b", id("pawn")),
                ("topic", text(48)),
                ("tone", text(48)),
                ("turns", int()),
            ],
        );
        add(
            "conversation.closed",
            Social,
            true,
            "A conversation ended and changed how the two feel about each other.",
            &[
                ("a", id("pawn")),
                ("b", id("pawn")),
                ("topic", text(48)),
                ("tone", text(48)),
                ("delta", signed()),
                ("affinity", signed()),
            ],
        );
        add(
            "conversation.cancelled",
            Social,
            false,
            "A conversation was called off because the two moved apart or one could not carry on.",
            &[("a", id("pawn")), ("b", id("pawn")), ("topic", text(48))],
        );
        add(
            "memory.created",
            Social,
            false,
            "A pawn formed a memory.",
            &[
                ("pawn", id("pawn")),
                ("memory", id("mem")),
                ("other", id("pawn")),
                ("topic", text(48)),
                ("importance", int()),
            ],
        );
        add(
            "memory.expired",
            Social,
            false,
            "A pawn forgot a memory (it faded, or was crowded out by more important ones).",
            &[
                ("pawn", id("pawn")),
                ("memory", id("mem")),
                ("why", text(24)),
            ],
        );
        add(
            "relationship.label_changed",
            Social,
            true,
            "Two residents' relationship moved to a different label (friendly, friend, wary, ...).",
            &[
                ("a", id("pawn")),
                ("b", id("pawn")),
                ("from", text(48)),
                ("to", text(48)),
                ("affinity", signed()),
            ],
        );
        add(
            "task.started",
            Task,
            false,
            "A pawn began a task.",
            &[
                ("pawn", id("pawn")),
                ("reservation", int()),
                ("action", text(64)),
            ],
        );
        add(
            "task.done",
            Task,
            false,
            "A pawn finished a task.",
            &[
                ("pawn", id("pawn")),
                ("reservation", int()),
                ("action", text(64)),
            ],
        );
        add(
            "task.failed",
            Task,
            true,
            "A task ended in failure, with the reason.",
            &[
                ("pawn", id("pawn")),
                (
                    "reservation",
                    Field::maybe(FieldSchema::Int {
                        min: 0,
                        max: i64::MAX,
                    }),
                ),
                ("reason", Field::required(FieldSchema::Any)),
                ("explain", text(400)),
            ],
        );
        add(
            "commitment.proposed",
            Commitment,
            true,
            "One pawn asked another to meet.",
            &[
                ("commitment", id("cmt")),
                ("proposer", id("pawn")),
                ("invitee", id("pawn")),
                ("start", int()),
                ("len", int()),
            ],
        );
        for (state, summary) in [
            (
                "accepted",
                "The invitee accepted; both schedules reserved the time.",
            ),
            ("declined", "The invitee declined."),
            ("expired", "The proposal expired unanswered."),
            ("active", "The commitment began."),
            ("completed", "Both pawns kept the commitment."),
            ("failed", "The commitment failed."),
            ("cancelled", "The commitment was cancelled."),
        ] {
            add(
                &format!("commitment.{state}"),
                Commitment,
                true,
                summary,
                &[("commitment", id("cmt")), ("explain", maybe_text(400))],
            );
        }
        c
    }

    /// The shared engine catalog.
    pub fn shared() -> &'static EventCatalog {
        static CATALOG: OnceLock<EventCatalog> = OnceLock::new();
        CATALOG.get_or_init(EventCatalog::builtin)
    }

    /// Declares a pack event. The kind must be `<pack>.<name>`.
    pub fn register(
        &mut self,
        pack: &str,
        name: &str,
        fields: ParamSchema,
        default_visible: bool,
        summary: &str,
    ) -> Result<(), String> {
        let ok_part = |s: &str| {
            !s.is_empty()
                && s.chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
        };
        if !ok_part(pack) || !ok_part(name) {
            return Err(format!("'{pack}.{name}' is not a valid event kind"));
        }
        let kind = format!("{pack}.{name}");
        if self.defs.contains_key(&kind) {
            return Err(format!("event '{kind}' is already declared"));
        }
        self.defs.insert(
            kind.clone(),
            EventDef {
                kind,
                category: Category::Pack,
                fields,
                default_visible,
                summary: summary.to_owned(),
                severity: Severity::Info,
            },
        );
        Ok(())
    }

    /// What the developer console shows for an event: its severity and one line of text, `kind key=value ...`
    /// with the fields in name order. A `script.log` line shows the pack's own text at the level it chose;
    /// an unknown kind shows as Warn so it is noticed rather than lost.
    pub fn console_line(&self, e: &Event) -> (Severity, String) {
        let Some(def) = self.get(&e.kind) else {
            // Packs declare their own events; the engine's are all in the catalog (and checked in debug builds).
            return (
                Severity::Info,
                format!("{} {}", e.kind, e.detail.to_canonical_string()),
            );
        };
        let text = |k: &str| e.detail.get(k).and_then(Canon::as_str).map(str::to_owned);
        if e.kind == "script.log" {
            let sev = if text("level").as_deref() == Some("warn") {
                Severity::Warn
            } else {
                Severity::Info
            };
            return (
                sev,
                format!(
                    "[{}] {}",
                    text("pack").unwrap_or_default(),
                    text("text").unwrap_or_default()
                ),
            );
        }
        let mut line = e.kind.clone();
        let explain = text("explain");
        if let Canon::Map(m) = &e.detail {
            for (k, v) in m {
                // An explanation replaces the machine-readable reason it comes with.
                if explain.is_some() && (k == "explain" || k == "reason") {
                    continue;
                }
                let shown = match v {
                    Canon::Str(s) => s.clone(),
                    other => other.to_canonical_string(),
                };
                line.push_str(&format!(" {k}={shown}"));
            }
        }
        if let Some(why) = explain {
            line.push_str(&format!(": {why}"));
        }
        (def.severity, line)
    }

    /// The console severity of `kind` without formatting anything; kinds this catalog does not know (a
    /// pack's) are Info.
    pub fn severity_of(&self, kind: &str) -> Severity {
        self.get(kind).map_or(Severity::Info, |d| d.severity)
    }

    pub fn get(&self, kind: &str) -> Option<&EventDef> {
        self.defs.get(kind)
    }

    /// Declarations in kind order.
    pub fn iter(&self) -> impl Iterator<Item = &EventDef> {
        self.defs.values()
    }

    /// Checks an event against its declaration.
    pub fn validate(&self, e: &Event) -> Result<(), String> {
        let def = self.defs.get(&e.kind).ok_or_else(|| {
            format!(
                "event kind '{}' is not in the catalog{}",
                e.kind,
                pg_content::hints::hint(&e.kind, self.defs.keys().map(String::as_str))
            )
        })?;
        let mut report = ValidationReport::new();
        def.fields.check(&e.detail, &e.kind, &mut report);
        let first = report.errors().next().map(|i| {
            format!(
                "event '{}' has a bad detail: {}: {}",
                e.kind, i.path, i.message
            )
        });
        first.map_or(Ok(()), Err)
    }

    /// Validates a batch, returning every problem.
    pub fn validate_all<'a>(&self, events: impl IntoIterator<Item = &'a Event>) -> Vec<String> {
        events
            .into_iter()
            .filter_map(|e| self.validate(e).err())
            .collect()
    }
}

#[cfg(test)]
mod tests;
