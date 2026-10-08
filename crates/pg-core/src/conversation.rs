//! Conversations between residents (Stage 1, milestone 1.4; Blueprint §9).
//!
//! * [`ConversationSystem`] (every game minute) closes the conversations that are over, calls off the ones
//!   whose two people drifted apart, and lets pairs of nearby, idle or socialising residents start new
//!   ones. A talk is fully decided when it starts (topic and tone from the data's rules and a seeded draw);
//!   it runs for a few game minutes while both stand where they are, and when it closes the outcome is
//!   applied: the pair's affinity moves (daily cap and label hysteresis), both feel less alone, and each
//!   keeps a memory.
//! * [`MemorySystem`] (each new day) fades and forgets memories.
//! * [`dialogue`] chooses the fallback lines a talk would show. They are presentation: nothing in the
//!   simulation reads them.
//!
//! All numbers come from `data/game/conversation.json`, `memory.json` and `relationships.json`. The hook
//! points `relationship.delta_modifier` and `memory.importance_modifier` let packs nudge an outcome within
//! the clamps declared in the API.

use crate::canon::{Canon, ToCanon};
use crate::id::{EntityId, Kind};
use crate::memory;
use crate::pawn::{Pawn, Step};
use crate::pipeline::{Pipeline, System, SystemSlot, TickCtx};
use crate::rng::{Key, Rng, Stream};
use crate::social::{Memory, PairKey, Relationship, Talk, TalkRecord};
use crate::time::TICKS_PER_GAME_MINUTE;
use crate::world::WorldState;
use pg_content::gamedata::{ConversationParams, GameData, ToneDef, TopicDef};
use std::sync::Arc;

/// The activities in which a resident is free to chat (everything else is busy).
pub const FRIENDLY_ACTIONS: [&str; 2] = ["socialise", "idle_at"];

/// The need a talk restores.
const SOCIAL: &str = "social";

fn pawn_ids(world: &WorldState) -> Vec<EntityId> {
    world.pawns.iter().map(|(id, _)| id).collect()
}

/// Whether a resident can start a conversation now: able to talk, standing still and not busy.
fn available(p: &Pawn) -> bool {
    p.talk.is_none()
        && p.capacities.can_act()
        && p.capacities.talking >= 500
        && p.route.is_none()
        && match &p.task {
            None => true,
            Some(t) => {
                FRIENDLY_ACTIONS.contains(&t.action.as_str())
                    && matches!(t.step(), Some(Step::PerformUntil(_)))
            }
        }
}

/// Whether a resident would like company (low on social, or already out to socialise).
fn wants_company(p: &Pawn, params: &ConversationParams) -> bool {
    p.task
        .as_ref()
        .is_some_and(|t| t.action.as_str() == "socialise")
        || p.needs
            .get(SOCIAL)
            .is_some_and(|v| *v < params.social_below)
}

fn mood_matches(moods: &[String], a: &Pawn, b: &Pawn) -> bool {
    moods.is_empty() || moods.iter().any(|m| *m == a.mood || *m == b.mood)
}

fn topic_allowed(t: &TopicDef, affinity: i32, a: &Pawn, b: &Pawn) -> bool {
    t.weight > 0
        && (t.min_affinity..=t.max_affinity).contains(&affinity)
        && mood_matches(&t.moods, a, b)
}

fn tone_applies(t: &ToneDef, affinity: i32, a: &Pawn, b: &Pawn) -> bool {
    (t.min_affinity..=t.max_affinity).contains(&affinity) && mood_matches(&t.moods, a, b)
}

/// The tone a conversation between `a` and `b` takes at `affinity`: the first by priority that holds.
pub fn choose_tone<'p>(
    params: &'p ConversationParams,
    affinity: i32,
    a: &Pawn,
    b: &Pawn,
) -> Option<&'p ToneDef> {
    params
        .ordered_tones()
        .into_iter()
        .find(|t| tone_applies(t, affinity, a, b))
}

/// The affinity two residents start from when they have no relationship yet.
fn known_affinity(world: &WorldState, a: EntityId, b: EntityId, start: i32) -> i32 {
    world.relationships.get(a, b).map_or(start, |r| r.affinity)
}

pub struct ConversationSystem {
    data: Arc<GameData>,
}

impl ConversationSystem {
    pub fn new(data: Arc<GameData>) -> ConversationSystem {
        ConversationSystem { data }
    }
}

impl System for ConversationSystem {
    fn id(&self) -> &str {
        "ConversationSystem"
    }

    fn run(&mut self, ctx: &mut TickCtx<'_>) {
        if !ctx.flags.minute {
            return;
        }
        let (Some(params), Some(rel_params)) = (&self.data.conversation, &self.data.relationships)
        else {
            return;
        };
        tidy_and_close(ctx, params, rel_params, self.data.memory.as_ref());
        start_new(ctx, params, rel_params.start_affinity);
    }
}

/// Clears broken links, calls off talks whose people drifted apart and closes the ones that are over.
fn tidy_and_close(
    ctx: &mut TickCtx<'_>,
    params: &ConversationParams,
    rel_params: &pg_content::gamedata::RelationshipParams,
    memory_params: Option<&pg_content::gamedata::MemoryParams>,
) {
    let tick = ctx.flags.tick;
    let ids = pawn_ids(ctx.world);
    // A talk whose other side no longer points back (the partner was removed, or was called off) ends.
    for id in &ids {
        let broken = ctx.world.pawns.get(*id).is_some_and(|p| {
            p.talk.as_ref().is_some_and(|t| {
                ctx.world
                    .pawns
                    .get(t.partner)
                    .and_then(|o| o.talk.as_ref())
                    .is_none_or(|ot| ot.partner != *id || ot.started != t.started)
            })
        });
        if broken {
            if let Some(p) = ctx.world.pawns.get_mut(*id) {
                p.talk = None;
            }
        }
    }
    for id in ids {
        let Some(talk) = ctx
            .world
            .pawns
            .get(id)
            .and_then(|p| p.talk.clone())
            .filter(|t| t.leader)
        else {
            continue;
        };
        let apart = match (ctx.world.pawns.get(id), ctx.world.pawns.get(talk.partner)) {
            (Some(a), Some(b)) => {
                a.position.map != b.position.map
                    || a.position.tile.manhattan(b.position.tile) > params.max_apart
                    || !a.capacities.can_act()
                    || !b.capacities.can_act()
            }
            _ => true,
        };
        if apart {
            for who in [id, talk.partner] {
                if let Some(p) = ctx.world.pawns.get_mut(who) {
                    p.talk = None;
                }
            }
            ctx.emit(
                "conversation.cancelled",
                Canon::map([
                    ("a", id.to_canon()),
                    ("b", talk.partner.to_canon()),
                    ("topic", Canon::str(talk.topic.clone())),
                ]),
            );
        } else if tick >= talk.ends {
            close(ctx, id, &talk, params, rel_params, memory_params);
        }
    }
}

/// Applies a finished conversation's outcome.
fn close(
    ctx: &mut TickCtx<'_>,
    a: EntityId,
    talk: &Talk,
    params: &ConversationParams,
    rel_params: &pg_content::gamedata::RelationshipParams,
    memory_params: Option<&pg_content::gamedata::MemoryParams>,
) {
    let b = talk.partner;
    let tick = ctx.flags.tick;
    let (Some(topic), Some(tone)) = (params.topic(&talk.topic), params.tone(&talk.tone)) else {
        // The data changed under a saved talk; drop it without an outcome.
        for who in [a, b] {
            if let Some(p) = ctx.world.pawns.get_mut(who) {
                p.talk = None;
            }
        }
        return;
    };
    let Some(key) = PairKey::new(a, b) else {
        return;
    };
    if ctx.world.relationships.get(a, b).is_none() {
        let label = rel_params
            .label_for(rel_params.start_affinity)
            .map(|l| l.id.clone())
            .unwrap_or_default();
        ctx.world.relationships.set(Relationship {
            key,
            affinity: rel_params.start_affinity,
            label,
            last_interaction_tick: 0,
            day: ctx.flags.day,
            day_change: 0,
            last_topic: None,
            forgotten: 0,
        });
    }
    // The outcome: the topic's change scaled by the tone, a small seeded wobble, then what packs say.
    let base = i64::from(topic.delta) * i64::from(tone.delta_permille) / 1000;
    let rng = Rng::new(
        ctx.world.seed(),
        Stream::SocialOutcome,
        &[
            Key::Id(a),
            Key::Id(b),
            Key::Int(i64::try_from(talk.started).unwrap_or(0)),
        ],
    );
    let wobble_span = i32::try_from(base.abs() / 5).unwrap_or(0);
    let wobble = rng
        .int_in(0, -wobble_span, wobble_span)
        .map_or(0, i64::from);
    let mut requested = base + wobble;
    if let (Some(point), Some(host)) = (
        crate::hooks::relationship_delta(),
        ctx.services.hooks.as_mut(),
    ) {
        let answers = host.ask(point, &*ctx.world, a);
        if !answers.is_empty() {
            requested = requested * crate::hooks::resolve(point, &answers) / 1000;
        }
    }
    let requested = i32::try_from(requested).unwrap_or(0);
    let day = ctx.flags.day;
    let Some(rel) = ctx.world.relationships.get_mut(a, b) else {
        return;
    };
    let result = rel.apply_delta(rel_params, requested, day, tick);
    rel.last_topic = Some(topic.id.clone());
    let affinity = rel.affinity;
    // Both feel less alone.
    let restore = params.social_restore * tone.restore_permille / 1000;
    for who in [a, b] {
        if let Some(p) = ctx.world.pawns.get_mut(who) {
            if let Some(v) = p.needs.get_mut(SOCIAL) {
                *v = (*v + restore).clamp(0, 1000);
            }
            p.talk = None;
        }
    }
    ctx.emit(
        "conversation.closed",
        Canon::map([
            ("a", a.to_canon()),
            ("b", b.to_canon()),
            ("topic", Canon::str(topic.id.clone())),
            ("tone", Canon::str(tone.id.clone())),
            ("delta", result.applied.to_canon()),
            ("affinity", affinity.to_canon()),
        ]),
    );
    if let Some((from, to)) = result.label_change {
        ctx.emit(
            "relationship.label_changed",
            Canon::map([
                ("a", a.to_canon()),
                ("b", b.to_canon()),
                ("from", Canon::str(from)),
                ("to", Canon::str(to)),
                ("affinity", affinity.to_canon()),
            ]),
        );
    }
    if let Some(mp) = memory_params {
        for (owner, other) in [(a, b), (b, a)] {
            record_memory(ctx, owner, other, topic, result.applied, mp, talk);
        }
    }
}

fn record_memory(
    ctx: &mut TickCtx<'_>,
    owner: EntityId,
    other: EntityId,
    topic: &TopicDef,
    impact: i32,
    mp: &pg_content::gamedata::MemoryParams,
    talk: &Talk,
) {
    let Ok(id) = ctx.world.id_counters.allocate(Kind::Memory) else {
        return;
    };
    let mut m = Memory::new(
        id,
        "conversation",
        ctx.flags.tick,
        vec![other],
        topic.severity,
        impact,
        mp.decay_k,
    );
    m.topic = Some(topic.id.clone());
    m.summary_key = Some(pg_content::gamedata::memory_summary_key(&topic.id));
    m.talk = Some(TalkRecord {
        started: talk.started,
        tone: talk.tone.clone(),
        turns: talk.turns,
        lines: talk.lines.clone(),
    });
    if let (Some(point), Some(host)) = (
        crate::hooks::memory_importance(),
        ctx.services.hooks.as_mut(),
    ) {
        let answers = host.ask(point, &*ctx.world, owner);
        if !answers.is_empty() {
            let f = crate::hooks::resolve(point, &answers);
            m.importance =
                i32::try_from(i64::from(m.importance) * f / 1000).unwrap_or(m.importance);
        }
    }
    let importance = m.importance;
    let gone = memory::remember(ctx.world, owner, m, mp);
    ctx.emit(
        "memory.created",
        Canon::map([
            ("pawn", owner.to_canon()),
            ("memory", id.to_canon()),
            ("other", other.to_canon()),
            ("topic", Canon::str(topic.id.clone())),
            ("importance", importance.to_canon()),
        ]),
    );
    for g in gone {
        emit_forgotten(ctx, &g);
    }
}

fn emit_forgotten(ctx: &mut TickCtx<'_>, g: &memory::Forgotten) {
    ctx.emit(
        "memory.expired",
        Canon::map([
            ("pawn", g.pawn.to_canon()),
            ("memory", g.memory.to_canon()),
            ("why", Canon::str(g.why.name())),
        ]),
    );
}

/// Starts conversations among the residents who are free to talk. Pairs are formed greedily in id order,
/// each pawn with the nearest other (ties to the lower id), so the result does not depend on iteration
/// accidents.
fn start_new(ctx: &mut TickCtx<'_>, params: &ConversationParams, start_affinity: i32) {
    let tick = ctx.flags.tick;
    let free: Vec<EntityId> = ctx
        .world
        .pawns
        .iter()
        .filter(|(_, p)| available(p))
        .map(|(id, _)| id)
        .collect();
    let mut used: Vec<EntityId> = Vec::new();
    let cooldown = u64::from(params.cooldown_minutes) * TICKS_PER_GAME_MINUTE;
    for (i, a_id) in free.iter().enumerate() {
        if used.contains(a_id) {
            continue;
        }
        let Some(a) = ctx.world.pawns.get(*a_id) else {
            continue;
        };
        // The nearest candidate within talking range.
        let mut best: Option<(u32, EntityId)> = None;
        for b_id in free.iter().skip(i + 1) {
            if used.contains(b_id) {
                continue;
            }
            let Some(b) = ctx.world.pawns.get(*b_id) else {
                continue;
            };
            if b.position.map != a.position.map {
                continue;
            }
            let d = b.position.tile.manhattan(a.position.tile);
            if d > params.talk_range {
                continue;
            }
            if !(wants_company(a, params) || wants_company(b, params)) {
                continue;
            }
            if let Some(rel) = ctx.world.relationships.get(*a_id, *b_id) {
                if rel.last_interaction_tick > 0
                    && tick < rel.last_interaction_tick.saturating_add(cooldown)
                {
                    continue;
                }
            }
            if best.is_none_or(|(bd, bid)| (d, *b_id) < (bd, bid)) {
                best = Some((d, *b_id));
            }
        }
        let Some((_, b_id)) = best else {
            continue;
        };
        let Some(b) = ctx.world.pawns.get(b_id) else {
            continue;
        };
        let affinity = known_affinity(ctx.world, *a_id, b_id, start_affinity);
        let chance = (i64::from(params.chance_permille) + i64::from(affinity) / 5).clamp(0, 1000);
        let rng = Rng::new(
            ctx.world.seed(),
            Stream::SocialTopic,
            &[
                Key::Id(*a_id),
                Key::Id(b_id),
                Key::Int(i64::try_from(ctx.flags.day).unwrap_or(0)),
                Key::Int(i64::from(ctx.flags.minute_of_day)),
            ],
        );
        if rng.range(0, 1000).is_none_or(|r| i64::from(r) >= chance) {
            continue;
        }
        let weights: Vec<u32> = params
            .topics
            .iter()
            .map(|t| {
                if topic_allowed(t, affinity, a, b) {
                    t.weight
                } else {
                    0
                }
            })
            .collect();
        let Some(topic) = rng
            .weighted_pick(1, &weights)
            .and_then(|i| params.topics.get(i))
        else {
            continue;
        };
        let Some(tone) = choose_tone(params, affinity, a, b) else {
            continue;
        };
        let turns = rng
            .int_in(
                2,
                i32::try_from(params.turns_min).unwrap_or(2),
                i32::try_from(params.turns_max).unwrap_or(4),
            )
            .and_then(|t| u32::try_from(t).ok())
            .unwrap_or(params.turns_min);
        let ends = tick + u64::from(turns) * u64::from(params.turn_minutes) * TICKS_PER_GAME_MINUTE;
        let make = |partner: EntityId, leader: bool| Talk {
            partner,
            topic: topic.id.clone(),
            tone: tone.id.clone(),
            started: tick,
            ends,
            turns,
            leader,
            lines: Vec::new(),
        };
        let (topic_id, tone_id) = (topic.id.clone(), tone.id.clone());
        if let Some(p) = ctx.world.pawns.get_mut(*a_id) {
            p.talk = Some(make(b_id, true));
        }
        if let Some(p) = ctx.world.pawns.get_mut(b_id) {
            p.talk = Some(make(*a_id, false));
        }
        used.push(*a_id);
        used.push(b_id);
        ctx.emit(
            "conversation.started",
            Canon::map([
                ("a", a_id.to_canon()),
                ("b", b_id.to_canon()),
                ("topic", Canon::str(topic_id)),
                ("tone", Canon::str(tone_id)),
                ("turns", turns.to_canon()),
            ]),
        );
    }
}

/// Fades and forgets memories at each new day.
pub struct MemorySystem {
    data: Arc<GameData>,
}

impl MemorySystem {
    pub fn new(data: Arc<GameData>) -> MemorySystem {
        MemorySystem { data }
    }
}

impl System for MemorySystem {
    fn id(&self) -> &str {
        "MemorySystem"
    }

    fn run(&mut self, ctx: &mut TickCtx<'_>) {
        if !ctx.flags.new_day {
            return;
        }
        let Some(params) = &self.data.memory else {
            return;
        };
        for id in pawn_ids(ctx.world) {
            for g in memory::age_memories(ctx.world, id, params) {
                emit_forgotten(ctx, &g);
            }
        }
    }
}

pub fn install(pipeline: &mut Pipeline, data: Arc<GameData>) {
    pipeline.set_builtin(
        SystemSlot::Conversation,
        Box::new(ConversationSystem::new(Arc::clone(&data))),
    );
    pipeline.set_builtin(SystemSlot::Memory, Box::new(MemorySystem::new(data)));
}

/// The most lines a conversation can carry (a turn is one line).
pub const MAX_RECORDED_LINES: usize = 8;

/// Why recorded lines are refused (shown in `input_rejected`).
fn refuse_lines(lines: &[String], turns: Option<u32>) -> Option<&'static str> {
    if lines.is_empty() || lines.len() > MAX_RECORDED_LINES {
        return Some("a conversation has 1 to 8 lines");
    }
    if turns.is_some_and(|t| t as usize != lines.len()) {
        return Some("the number of lines must equal the number of turns");
    }
    if lines.iter().any(|l| {
        l.trim().is_empty()
            || l.chars().count() > pg_content::gamedata::MAX_DIALOGUE_CHARS
            || l.chars().any(char::is_control)
    }) {
        return Some("lines must be 1 to 140 characters with no control characters");
    }
    None
}

fn same_talk(m: &Memory, other: EntityId, started: u64) -> bool {
    m.participants.contains(&other) && m.talk.as_ref().is_some_and(|t| t.started == started)
}

/// Records what was said in a conversation (the `RecordDialogue` command). While the talk is under way the
/// lines wait on the two pawns' talks; once it has closed they go into both pawns' memories of it. Either
/// way they are state, entered as a logged input, so a replay reproduces them. Returns when they were
/// recorded, or why they were refused.
pub fn record_dialogue(
    world: &mut WorldState,
    a: EntityId,
    b: EntityId,
    started: u64,
    lines: &[String],
) -> Result<&'static str, &'static str> {
    let live_turns = world
        .pawns
        .get(a)
        .and_then(|p| p.talk.as_ref())
        .filter(|t| t.partner == b && t.started == started)
        .map(|t| t.turns);
    if let Some(turns) = live_turns {
        if let Some(why) = refuse_lines(lines, Some(turns)) {
            return Err(why);
        }
        for (who, other) in [(a, b), (b, a)] {
            if let Some(t) = world
                .pawns
                .get_mut(who)
                .and_then(|p| p.talk.as_mut())
                .filter(|t| t.partner == other && t.started == started)
            {
                t.lines = lines.to_vec();
            }
        }
        return Ok("while it was happening");
    }
    // Afterwards: the memory each of them kept.
    let mut found = false;
    for (who, other) in [(a, b), (b, a)] {
        let turns = world.pawns.get(who).and_then(|p| {
            p.memories
                .iter()
                .find(|m| same_talk(m, other, started))
                .and_then(|m| m.talk.as_ref().map(|t| t.turns))
        });
        if let Some(turns) = turns {
            if let Some(why) = refuse_lines(lines, Some(turns)) {
                return Err(why);
            }
            found = true;
        }
    }
    if !found {
        return Err("no such conversation is remembered");
    }
    for (who, other) in [(a, b), (b, a)] {
        if let Some(t) = world.pawns.get_mut(who).and_then(|p| {
            p.memories
                .iter_mut()
                .find(|m| same_talk(m, other, started))
                .and_then(|m| m.talk.as_mut())
        }) {
            t.lines = lines.to_vec();
        }
    }
    Ok("afterwards")
}

/// What was said, as remembered.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Spoken {
    /// Text that was generated or recorded.
    Written(String),
    /// A fallback line: a string-table key with `{name}` and `{other}` to fill in.
    Key(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Recalled {
    pub speaker: EntityId,
    pub listener: EntityId,
    pub said: Spoken,
}

/// Recalls the conversation a memory is about: the recorded lines if there are any, otherwise the fallback
/// lines rebuilt from the data. `owner` is the pawn who holds the memory. `None` for a memory that is not
/// of a conversation.
pub fn recall(
    world: &WorldState,
    params: &ConversationParams,
    owner: EntityId,
    memory: &Memory,
) -> Option<Vec<Recalled>> {
    let record = memory.talk.as_ref()?;
    let other = *memory.participants.first()?;
    let (first, second) = (owner.min(other), owner.max(other));
    if !record.lines.is_empty() {
        return Some(
            record
                .lines
                .iter()
                .enumerate()
                .map(|(i, l)| {
                    let (speaker, listener) = if i % 2 == 0 {
                        (first, second)
                    } else {
                        (second, first)
                    };
                    Recalled {
                        speaker,
                        listener,
                        said: Spoken::Written(l.clone()),
                    }
                })
                .collect(),
        );
    }
    let talk = Talk {
        partner: second,
        topic: memory.topic.clone()?,
        tone: record.tone.clone(),
        started: record.started,
        ends: record.started,
        turns: record.turns,
        leader: true,
        lines: Vec::new(),
    };
    Some(
        dialogue(params, world.seed(), first, &talk)
            .into_iter()
            .map(|l| Recalled {
                speaker: l.speaker,
                listener: l.listener,
                said: Spoken::Key(l.key),
            })
            .collect(),
    )
}

/// One line of fallback dialogue: who says it and the string-table key (with `{name}` the speaker and
/// `{other}` the listener to fill in).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DialogueLine {
    pub speaker: EntityId,
    pub listener: EntityId,
    pub key: String,
}

/// The lines a talk would show, one per turn, alternating speakers starting with `first`. Presentation
/// only: the same talk always gives the same lines, and nothing reads them back.
pub fn dialogue(
    params: &ConversationParams,
    seed: crate::rng::Seed,
    first: EntityId,
    talk: &Talk,
) -> Vec<DialogueLine> {
    let second = talk.partner;
    let keys = params.line_keys(&talk.topic, &talk.tone);
    let rng = Rng::new(
        seed,
        Stream::SocialTopic,
        &[
            Key::Str("lines"),
            Key::Id(first.min(second)),
            Key::Id(first.max(second)),
            Key::Int(i64::try_from(talk.started).unwrap_or(0)),
        ],
    );
    // Start at a seeded line and walk the set, so no line repeats before the set is used up.
    let start = rng
        .range(0, u32::try_from(keys.len()).unwrap_or(1))
        .unwrap_or(0) as usize;
    (0..talk.turns)
        .filter_map(|i| {
            let key = keys.get((start + i as usize) % keys.len().max(1))?.clone();
            let (speaker, listener) = if i % 2 == 0 {
                (first, second)
            } else {
                (second, first)
            };
            Some(DialogueLine {
                speaker,
                listener,
                key,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests;
