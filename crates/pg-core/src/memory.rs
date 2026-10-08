//! What residents remember (Stage 1, milestone 1.4; Blueprint §8.3).
//!
//! * [`remember`] adds a memory and, when a pawn is over its bound, forgets the least valuable ordinary
//!   memory (importance times retention, ties by age then id). Persistent memories (importance at or above
//!   the data's threshold) are never forgotten, so the bound applies to ordinary memories only.
//! * [`age_memories`] runs at each day boundary: retention drops by each memory's own rate and ordinary
//!   memories that reach zero are forgotten.
//! * Whatever is forgotten is rolled up into the relationship it concerned (`last_topic`, `forgotten`), so a
//!   pair does not lose all sense of its history.
//! * [`select_relevant_memories`] picks the memories that matter for a situation and says why.

use crate::id::EntityId;
use crate::social::Memory;
use crate::world::WorldState;
use pg_content::gamedata::MemoryParams;

/// Why a memory left a pawn's mind.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Why {
    /// Its retention ran out.
    Faded,
    /// The pawn had too many and this was the least valuable.
    Crowded,
}

impl Why {
    pub const fn name(self) -> &'static str {
        match self {
            Why::Faded => "faded",
            Why::Crowded => "crowded_out",
        }
    }
}

/// A memory that was forgotten.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Forgotten {
    pub pawn: EntityId,
    pub memory: EntityId,
    pub why: Why,
}

/// Whether the memory never fades.
pub fn is_persistent(m: &Memory, params: &MemoryParams) -> bool {
    m.importance >= params.persist_threshold
}

fn roll_up(world: &mut WorldState, pawn: EntityId, m: &Memory) {
    for other in &m.participants {
        if let Some(rel) = world.relationships.get_mut(pawn, *other) {
            rel.forgotten = rel.forgotten.saturating_add(1);
            if m.topic.is_some() {
                rel.last_topic = m.topic.clone();
            }
        }
    }
}

/// Adds `memory` to `pawn`. Returns what had to be forgotten to stay within the bound.
pub fn remember(
    world: &mut WorldState,
    pawn: EntityId,
    memory: Memory,
    params: &MemoryParams,
) -> Vec<Forgotten> {
    let Some(p) = world.pawns.get_mut(pawn) else {
        return Vec::new();
    };
    p.memories.push(memory);
    let mut gone = Vec::new();
    let mut rolled = Vec::new();
    let max = usize::try_from(params.max_memories).unwrap_or(usize::MAX);
    while p.memories.len() > max {
        // The least valuable ordinary memory: lowest importance x retention, then oldest, then lowest id.
        let victim = p
            .memories
            .iter()
            .enumerate()
            .filter(|(_, m)| !is_persistent(m, params))
            .min_by_key(|(_, m)| {
                (
                    i64::from(m.importance) * i64::from(m.retention),
                    m.tick,
                    m.id,
                )
            })
            .map(|(i, _)| i);
        let Some(i) = victim else {
            break;
        };
        let m = p.memories.remove(i);
        gone.push(Forgotten {
            pawn,
            memory: m.id,
            why: Why::Crowded,
        });
        rolled.push(m);
    }
    for m in &rolled {
        roll_up(world, pawn, m);
    }
    gone
}

/// One day passes for `pawn`'s memories.
pub fn age_memories(
    world: &mut WorldState,
    pawn: EntityId,
    params: &MemoryParams,
) -> Vec<Forgotten> {
    let Some(p) = world.pawns.get_mut(pawn) else {
        return Vec::new();
    };
    let mut gone = Vec::new();
    let mut rolled = Vec::new();
    let mut kept = Vec::with_capacity(p.memories.len());
    for mut m in std::mem::take(&mut p.memories) {
        if is_persistent(&m, params) {
            kept.push(m);
            continue;
        }
        m.retention = m.retention.saturating_sub(m.decay_per_day);
        if m.retention <= 0 {
            gone.push(Forgotten {
                pawn,
                memory: m.id,
                why: Why::Faded,
            });
            rolled.push(m);
        } else {
            kept.push(m);
        }
    }
    p.memories = kept;
    for m in &rolled {
        roll_up(world, pawn, m);
    }
    gone
}

/// What a caller wants to remember about.
#[derive(Clone, Debug, Default)]
pub struct Context<'a> {
    pub now: u64,
    /// Someone the situation involves.
    pub with: Option<EntityId>,
    pub topic: Option<&'a str>,
}

/// Why a memory was picked.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Reason {
    Important(i32),
    Recent,
    Participant,
    Topic,
}

impl Reason {
    pub fn text(&self) -> String {
        match self {
            Reason::Important(i) => format!("important ({i})"),
            Reason::Recent => "recent".to_owned(),
            Reason::Participant => "involves them".to_owned(),
            Reason::Topic => "same topic".to_owned(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct Relevant<'a> {
    pub memory: &'a Memory,
    pub score: i64,
    pub reasons: Vec<Reason>,
}

/// How many ticks count as recent for the recency bonus (two game days).
const RECENT_TICKS: u64 = 2 * 1440 * crate::time::TICKS_PER_GAME_MINUTE;

/// The `n` most relevant memories for `ctx`, best first. The score is the memory's importance, a bonus that
/// shrinks with age over two days, 200 for involving the person in question and 150 for the same topic;
/// ties go to the newer memory, then the lower id.
pub fn select_relevant_memories<'a>(
    memories: &'a [Memory],
    ctx: &Context<'_>,
    n: usize,
) -> Vec<Relevant<'a>> {
    let mut out: Vec<Relevant<'a>> = memories
        .iter()
        .map(|m| {
            let mut score = i64::from(m.importance);
            let mut reasons = Vec::new();
            if m.importance >= 40 {
                reasons.push(Reason::Important(m.importance));
            }
            let age = ctx.now.saturating_sub(m.tick);
            if m.tick > 0 && age < RECENT_TICKS {
                score += i64::try_from((RECENT_TICKS - age) * 300 / RECENT_TICKS).unwrap_or(0);
                reasons.push(Reason::Recent);
            }
            if ctx.with.is_some_and(|w| m.participants.contains(&w)) {
                score += 200;
                reasons.push(Reason::Participant);
            }
            if ctx.topic.is_some() && m.topic.as_deref() == ctx.topic {
                score += 150;
                reasons.push(Reason::Topic);
            }
            Relevant {
                memory: m,
                score,
                reasons,
            }
        })
        .collect();
    out.sort_by(|a, b| {
        b.score
            .cmp(&a.score)
            .then(b.memory.tick.cmp(&a.memory.tick))
            .then(a.memory.id.cmp(&b.memory.id))
    });
    out.truncate(n);
    out
}

#[cfg(test)]
mod tests;
