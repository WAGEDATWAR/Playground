//! Building the resident inspector's data from the world (milestone 1.7). Read-only: the inspector looks at
//! the simulation, never changes it.

use pg_core::conversation::{recall, Spoken};
use pg_core::id::EntityId;
use pg_core::memory::{is_persistent, select_relevant_memories, Context};
use pg_core::sim::Sim;
use pg_ui_model::inspector::{ConversationRow, MemoryRow, RelationshipRow, ResidentView, Said};

/// Memories shown, best first.
const MEMORIES: usize = 6;
/// Relationships shown, strongest feelings first.
const RELATIONSHIPS: usize = 8;
/// Remembered conversations shown, newest first.
const CONVERSATIONS: usize = 3;

fn given(full: &str) -> String {
    full.split_whitespace().next().unwrap_or(full).to_owned()
}

/// What the inspector shows for `id`, or `None` if there is no such resident.
pub fn resident_view(sim: &Sim, id: EntityId) -> Option<ResidentView> {
    let world = sim.world();
    let p = world.pawns.get(id)?;
    let content = sim.content();
    let data = content.map(|c| c.game());
    let name = |who: EntityId| {
        world
            .pawns
            .get(who)
            .map_or_else(|| who.to_string(), |o| o.name.clone())
    };
    let now = world.clock.tick();
    let persistent = |m: &pg_core::social::Memory| {
        data.and_then(|d| d.memory.as_ref())
            .is_some_and(|mp| is_persistent(m, mp))
    };
    let memories = select_relevant_memories(
        &p.memories,
        &Context {
            now,
            with: p.talk.as_ref().map(|t| t.partner),
            topic: p.talk.as_ref().map(|t| t.topic.as_str()),
        },
        MEMORIES,
    )
    .into_iter()
    .map(|r| MemoryRow {
        summary_key: r.memory.summary_key.clone(),
        other: r
            .memory
            .participants
            .first()
            .map_or_else(String::new, |o| given(&name(*o))),
        importance: r.memory.importance,
        persistent: persistent(r.memory),
        reasons: r.reasons.iter().map(|x| x.text()).collect(),
    })
    .collect();
    let mut rels: Vec<_> = world.relationships.of(id).collect();
    rels.sort_by(|a, b| {
        b.affinity
            .abs()
            .cmp(&a.affinity.abs())
            .then(a.key.cmp(&b.key))
    });
    let relationships = rels
        .into_iter()
        .take(RELATIONSHIPS)
        .map(|r| RelationshipRow {
            other: r.key.other(id).map_or_else(String::new, name),
            label: r.label.clone(),
            affinity: r.affinity,
            last_topic: r.last_topic.clone(),
        })
        .collect();
    let mut conversations = Vec::new();
    if let Some(params) = data.and_then(|d| d.conversation.as_ref()) {
        let mut talks: Vec<_> = p.memories.iter().filter(|m| m.talk.is_some()).collect();
        talks.sort_by(|a, b| b.tick.cmp(&a.tick).then(a.id.cmp(&b.id)));
        for m in talks.into_iter().take(CONVERSATIONS) {
            let Some(lines) = recall(world, params, id, m) else {
                continue;
            };
            let record = m.talk.as_ref()?;
            conversations.push(ConversationRow {
                summary_key: m.summary_key.clone(),
                other: m
                    .participants
                    .first()
                    .map_or_else(String::new, |o| given(&name(*o))),
                tone: record.tone.clone(),
                recorded: !record.lines.is_empty(),
                lines: lines
                    .into_iter()
                    .map(|l| {
                        let said = match l.said {
                            Spoken::Written(t) => Said::Written(t),
                            Spoken::Key { key, memory } => Said::Key {
                                key,
                                name: given(&name(l.speaker)),
                                other: given(&name(l.listener)),
                                memory,
                            },
                        };
                        (given(&name(l.speaker)), said)
                    })
                    .collect(),
            });
        }
    }
    Some(ResidentView {
        id,
        name: p.name.clone(),
        occupation: p.occupation.as_ref().map(|o| o.template.clone()),
        mood: p.mood.clone(),
        outgoing: p.outgoing,
        activity: p
            .task
            .as_ref()
            .map_or_else(|| "free".to_owned(), |t| t.action.to_string()),
        talking_with: p.talk.as_ref().map(|t| given(&name(t.partner))),
        needs: p.needs.iter().map(|(k, v)| (k.clone(), *v)).collect(),
        memories,
        relationships,
        conversations,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use pg_content::{load_pack, ComponentRegistry, ContentSet, DirPack, Limits};
    use pg_core::commands::Command;
    use pg_core::input::SimInput;
    use pg_core::pipeline::Pipeline;
    use pg_core::world::WorldState;
    use std::sync::Arc;

    fn town() -> Sim {
        let dir = format!("{}/../../data/base", env!("CARGO_MANIFEST_DIR"));
        let pack = load_pack(&DirPack::new(dir), &Limits::default()).unwrap();
        let content =
            Arc::new(ContentSet::build(vec![pack], ComponentRegistry::builtin()).unwrap());
        let mut sim =
            Sim::new(WorldState::new("Look", "inspect"), Pipeline::new()).with_content(content);
        sim.submit(
            0,
            SimInput::Command {
                actor: None,
                cmd: Command::GenerateTown {
                    w: 48,
                    h: 36,
                    water: 15,
                    residents: 12,
                    tone: "standard".into(),
                },
            },
        )
        .unwrap();
        sim
    }

    #[test]
    fn a_resident_view_has_needs_memories_with_reasons_relationships_and_remembered_talks() {
        let mut sim = town();
        sim.run_ticks(20_000).unwrap();
        let id = EntityId::new(pg_core::id::Kind::Pawn, 1);
        let v = resident_view(&sim, id).unwrap();
        assert_eq!(v.id, id);
        assert!(v.needs.len() >= 3 && v.occupation.is_some());
        assert!(!v.memories.is_empty() && v.memories.len() <= MEMORIES);
        assert!(
            v.memories.iter().all(|m| !m.other.contains(' ')),
            "first names"
        );
        assert!(v.relationships.len() <= RELATIONSHIPS);
        assert!(v
            .relationships
            .windows(2)
            .all(|w| w[0].affinity.abs() >= w[1].affinity.abs()));
        assert!(
            !v.conversations.is_empty(),
            "the resident has talked by now"
        );
        assert!(v.conversations.len() <= CONVERSATIONS);
        assert!(v.conversations.iter().all(|c| !c.lines.is_empty()));
        assert!(resident_view(&sim, EntityId::new(pg_core::id::Kind::Pawn, 999)).is_none());
    }
}
