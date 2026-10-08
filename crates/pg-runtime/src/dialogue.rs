//! Lines for conversations someone could hear (Stage 1, milestone 1.5; Blueprint §9.4, §10.3).
//!
//! The simulation decides what a conversation is and what comes of it (topic, tone, outcome); this service
//! only decides what the residents *say*. It watches `conversation.started` events. When the focused
//! resident is within the hearing range of either speaker it prepares the fallback lines at once (always
//! available, drawn from the data) and, if AI is on, asks the AI client for better ones on the worker pool.
//! A reply that arrives in time replaces the turns that have not been spoken yet; one that arrives late or
//! fails, is refused by the checks or is cut off by the breaker changes nothing. Nothing here touches the
//! world: the text is not in saves, replays or hashes, and outcomes are identical with AI on, off or failing.
//!
//! Limits: one request per conversation, a cooldown per resident, a cap on requests in flight and, in the
//! client, a requests-per-minute cap, a response cache and the circuit breaker.

use crate::pool::{JobHandle, WorkerPool};
use pg_ai::client::{AiClient, AiStatus};
use pg_ai::dialogue::{build_task, parse_lines, ContentRules, DialogueRequest};
use pg_ai::settings::AiSettings;
use pg_core::conversation::dialogue as fallback_dialogue;
use pg_core::id::EntityId;
use pg_core::memory::{select_relevant_memories, Context};
use pg_core::pipeline::Event;
use pg_core::sim::Sim;
use pg_core::time::TICKS_PER_GAME_MINUTE;
use pg_host::CancelToken;
use std::collections::{BTreeMap, VecDeque};
use std::sync::Arc;

#[derive(Clone, Debug)]
pub struct DialogueConfig {
    /// Conversations stored.
    pub keep: usize,
    /// Ticks before the same resident may be in another request.
    pub pawn_cooldown_ticks: u64,
    /// Requests allowed to be running at once.
    pub max_in_flight: usize,
}

impl Default for DialogueConfig {
    fn default() -> Self {
        DialogueConfig {
            keep: 64,
            pawn_cooldown_ticks: 3 * 60 * TICKS_PER_GAME_MINUTE,
            max_in_flight: 2,
        }
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Source {
    Fallback,
    Ai,
}

/// One spoken line for display.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Shown {
    pub speaker: EntityId,
    pub text: String,
    pub source: Source,
}

#[derive(Clone, Debug)]
struct Stored {
    first: EntityId,
    second: EntityId,
    started: u64,
    turn_ticks: u64,
    turns: usize,
    fallback: Vec<(EntityId, String)>,
    /// Lines from the AI and the first turn they apply to.
    ai: Option<(Vec<String>, usize)>,
}

/// Counters for the overlay's AI panel.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DialogueStats {
    pub heard: u64,
    pub requested: u64,
    pub used: u64,
    pub late: u64,
    pub refused: u64,
    pub failed: u64,
    pub skipped_cooldown: u64,
    pub skipped_busy: u64,
}

type Pending = (
    (EntityId, u64),
    JobHandle<Result<Vec<String>, String>>,
    usize,
);

pub struct DialogueService {
    client: Arc<AiClient>,
    pool: Arc<WorkerPool>,
    cfg: DialogueConfig,
    focus: Option<EntityId>,
    stored: VecDeque<Stored>,
    pending: Vec<Pending>,
    last_asked: BTreeMap<EntityId, u64>,
    stats: DialogueStats,
}

fn time_of_day(minute: u32) -> &'static str {
    match minute / 60 {
        5..=11 => "morning",
        12..=16 => "afternoon",
        17..=21 => "evening",
        _ => "night",
    }
}

impl DialogueService {
    pub fn new(
        client: Arc<AiClient>,
        pool: Arc<WorkerPool>,
        cfg: DialogueConfig,
    ) -> DialogueService {
        DialogueService {
            client,
            pool,
            cfg,
            focus: None,
            stored: VecDeque::new(),
            pending: Vec::new(),
            last_asked: BTreeMap::new(),
            stats: DialogueStats::default(),
        }
    }

    /// The resident whose surroundings the player is watching or possessing (`None`: nobody is listening).
    pub fn set_focus(&mut self, focus: Option<EntityId>) {
        self.focus = focus;
    }

    pub fn stats(&self) -> &DialogueStats {
        &self.stats
    }

    pub fn status(&self, settings: &AiSettings) -> AiStatus {
        self.client.status(settings)
    }

    /// Looks at one tick's events; call it with the simulation after the tick.
    pub fn observe(
        &mut self,
        events: &[Event],
        sim: &Sim,
        settings: &AiSettings,
        rules: &ContentRules,
    ) {
        let Some(content) = sim.content() else {
            return;
        };
        let Some(params) = &content.game().conversation else {
            return;
        };
        let world = sim.world();
        let Some(focus) = self.focus.and_then(|f| world.pawns.get(f)) else {
            return;
        };
        for e in events.iter().filter(|e| e.kind == "conversation.started") {
            let (Some(a), Some(b)) = (
                e.detail
                    .get("a")
                    .and_then(|c| c.as_str()?.parse::<EntityId>().ok()),
                e.detail
                    .get("b")
                    .and_then(|c| c.as_str()?.parse::<EntityId>().ok()),
            ) else {
                continue;
            };
            let (Some(pa), Some(pb)) = (world.pawns.get(a), world.pawns.get(b)) else {
                continue;
            };
            let Some(talk) = pa.talk.clone() else {
                continue;
            };
            // Audible: the focused resident is near enough to either speaker (or is one of them).
            let near = |p: &pg_core::pawn::Pawn| {
                p.position.map == focus.position.map
                    && p.position.tile.manhattan(focus.position.tile) <= params.hear_range
            };
            if !(near(pa) || near(pb)) {
                continue;
            }
            self.stats.heard += 1;
            let Some((fallback, req, turn_ticks)) = describe(sim, a, b, &talk, e.tick, rules)
            else {
                continue;
            };
            self.stored.push_back(Stored {
                first: a,
                second: b,
                started: talk.started,
                turn_ticks,
                turns: talk.turns as usize,
                fallback,
                ai: None,
            });
            while self.stored.len() > self.cfg.keep {
                self.stored.pop_front();
            }
            if !settings.enabled {
                continue;
            }
            let now = e.tick;
            let cooling = [a, b].iter().any(|p| {
                self.last_asked
                    .get(p)
                    .is_some_and(|t| now < t + self.cfg.pawn_cooldown_ticks)
            });
            if cooling {
                self.stats.skipped_cooldown += 1;
                continue;
            }
            if self.pending.len() >= self.cfg.max_in_flight {
                self.stats.skipped_busy += 1;
                continue;
            }
            self.last_asked.insert(a, now);
            self.last_asked.insert(b, now);
            self.stats.requested += 1;
            let client = Arc::clone(&self.client);
            let settings = settings.clone();
            let key = (a, talk.started);
            let turns = req.turns;
            let job = self.pool.submit(move || {
                let task = build_task(&req);
                match client.generate(&settings, &task, &CancelToken::new()) {
                    Ok(reply) => parse_lines(&reply.text, req.turns, &req.rules)
                        .map_err(|r| format!("refused: {}", r.0)),
                    Err(e) => Err(e.user_message()),
                }
            });
            self.pending.push((key, job, turns));
        }
    }

    /// Collects finished requests. `now` is the current tick, used to see which turns are already over.
    /// Lines that were used come back as recordings for the history (see [`Recording`]).
    pub fn poll(&mut self, now: u64) -> Polled {
        let mut notes = Vec::new();
        let mut record = Vec::new();
        let mut still = Vec::new();
        for (key, job, turns) in std::mem::take(&mut self.pending) {
            match job.try_join() {
                None => still.push((key, job, turns)),
                Some(Err(_)) => self.stats.failed += 1,
                Some(Ok(Err(why))) => {
                    if why.starts_with("refused") {
                        self.stats.refused += 1;
                    } else {
                        self.stats.failed += 1;
                    }
                    notes.push(format!("dialogue lines not used: {why}"));
                }
                Some(Ok(Ok(lines))) => {
                    let Some(s) = self.stored.iter_mut().find(|s| (s.first, s.started) == key)
                    else {
                        continue;
                    };
                    let spoken =
                        usize::try_from(now.saturating_sub(s.started) / s.turn_ticks.max(1))
                            .unwrap_or(usize::MAX);
                    if spoken >= turns {
                        self.stats.late += 1;
                    } else {
                        s.ai = Some((lines, spoken));
                        self.stats.used += 1;
                        if let Some(shown) = lines_of(s) {
                            record.push(Recording {
                                a: s.first,
                                b: s.second,
                                started: s.started,
                                lines: shown.into_iter().map(|l| l.text).collect(),
                            });
                        }
                    }
                }
            }
        }
        self.pending = still;
        Polled { notes, record }
    }

    /// The lines of a stored conversation as they should be shown at `now`: each turn from the AI if its
    /// lines were in before that turn began, otherwise from the fallback.
    pub fn lines(&self, first: EntityId, started: u64) -> Option<Vec<Shown>> {
        let s = self
            .stored
            .iter()
            .find(|s| s.first == first && s.started == started)?;
        lines_of(s)
    }

    /// What is being said at tick `now`: for each conversation under way that was heard, the current turn's
    /// speaker and line.
    pub fn bubbles(&self, now: u64) -> Vec<(EntityId, String)> {
        self.stored
            .iter()
            .filter_map(|s| {
                let span = s.turn_ticks.max(1) * s.turns as u64;
                if now < s.started || now >= s.started + span {
                    return None;
                }
                let turn = usize::try_from((now - s.started) / s.turn_ticks.max(1)).ok()?;
                let line = lines_of(s)?.into_iter().nth(turn)?;
                Some((line.speaker, line.text))
            })
            .collect()
    }

    pub fn in_flight(&self) -> usize {
        self.pending.len()
    }
}

/// Lines to record for the history: what was shown, turn by turn.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Recording {
    pub a: EntityId,
    pub b: EntityId,
    pub started: u64,
    pub lines: Vec<String>,
}

/// What a poll found.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Polled {
    pub notes: Vec<String>,
    pub record: Vec<Recording>,
}

fn lines_of(s: &Stored) -> Option<Vec<Shown>> {
    {
        Some(
            (0..s.turns)
                .filter_map(|i| {
                    let (speaker, fb) = s.fallback.get(i)?;
                    if let Some((ai, from)) = &s.ai {
                        if i >= *from {
                            if let Some(t) = ai.get(i) {
                                return Some(Shown {
                                    speaker: *speaker,
                                    text: t.clone(),
                                    source: Source::Ai,
                                });
                            }
                        }
                    }
                    Some(Shown {
                        speaker: *speaker,
                        text: fb.clone(),
                        source: Source::Fallback,
                    })
                })
                .collect(),
        )
    }
}

/// Fallback lines (speaker and text), the request an AI would be sent, and the length of a turn in ticks.
pub type Described = (Vec<(EntityId, String)>, DialogueRequest, u64);

/// Everything about one conversation that the lines depend on: the fallback lines as text, the request an
/// AI would be sent, and the length of a turn in ticks. `None` if the game has no conversation data or a
/// speaker is gone. Pure: shared by the service and `pg ai dialogue-test`.
pub fn describe(
    sim: &Sim,
    a: EntityId,
    b: EntityId,
    talk: &pg_core::social::Talk,
    now: u64,
    rules: &ContentRules,
) -> Option<Described> {
    let content = sim.content()?;
    let params = content.game().conversation.as_ref()?;
    let world = sim.world();
    let (pa, pb) = (world.pawns.get(a)?, world.pawns.get(b)?);
    let strings = content.strings();
    let fallback = fallback_dialogue(params, world.seed(), a, talk)
        .into_iter()
        .map(|l| {
            let (speaker, listener) = if l.speaker == a {
                (given(&pa.name), given(&pb.name))
            } else {
                (given(&pb.name), given(&pa.name))
            };
            let phrase = l.memory.as_ref().map_or_else(String::new, |m| {
                strings.text("en", &pg_content::gamedata::memory_phrase_key(m), &[])
            });
            (
                l.speaker,
                strings.text(
                    "en",
                    &l.key,
                    &[("name", speaker), ("other", listener), ("memory", &phrase)],
                ),
            )
        })
        .collect();
    let relationship = world.relationships.get(a, b).map_or_else(
        || "stranger".to_owned(),
        |r| game_label(content.game(), strings, &r.label),
    );
    let memories = select_relevant_memories(
        &pa.memories,
        &Context {
            now,
            with: Some(b),
            topic: Some(&talk.topic),
        },
        pg_ai::dialogue::MAX_MEMORIES,
    )
    .into_iter()
    .map(|r| memory_text(r.memory, &pb.name, strings))
    .collect();
    let req = DialogueRequest {
        first: pa.name.clone(),
        second: pb.name.clone(),
        relationship,
        first_mood: pa.mood.clone(),
        second_mood: pb.mood.clone(),
        topic: talk.topic.clone(),
        tone: talk.tone.clone(),
        turns: talk.turns as usize,
        time_of_day: time_of_day(world.clock.minute_of_day()).to_owned(),
        memories,
        rules: rules.clone(),
    };
    Some((
        fallback,
        req,
        u64::from(params.turn_minutes) * TICKS_PER_GAME_MINUTE,
    ))
}

/// The first name (lines address people the way neighbours do).
fn given(full: &str) -> &str {
    full.split_whitespace().next().unwrap_or(full)
}

fn game_label(
    data: &pg_content::gamedata::GameData,
    strings: &pg_content::strings::Strings,
    label_id: &str,
) -> String {
    data.relationships
        .as_ref()
        .and_then(|r| r.labels.iter().find(|l| l.id == label_id))
        .map_or_else(
            || label_id.to_owned(),
            |l| strings.text("en", &l.label_key, &[]),
        )
}

fn memory_text(
    m: &pg_core::social::Memory,
    other_name: &str,
    strings: &pg_content::strings::Strings,
) -> String {
    match &m.summary_key {
        Some(k) => strings.text("en", k, &[("other", other_name)]),
        None => m.ty.clone(),
    }
}

/// What the simulation loop needs to feed a [`DialogueService`]: the service, and the AI and filter settings
/// as the controller last saw them (the loop runs on its own thread and cannot read the settings itself).
#[derive(Clone)]
pub struct DialogueHook {
    pub service: std::sync::Arc<std::sync::Mutex<DialogueService>>,
    pub settings: std::sync::Arc<std::sync::Mutex<(AiSettings, bool)>>,
}

impl DialogueHook {
    pub fn new(service: DialogueService) -> DialogueHook {
        DialogueHook {
            service: std::sync::Arc::new(std::sync::Mutex::new(service)),
            settings: std::sync::Arc::new(std::sync::Mutex::new((AiSettings::default(), true))),
        }
    }

    /// What residents are saying at `tick` (see [`DialogueService::bubbles`]).
    pub fn bubbles(&self, tick: u64) -> Vec<(EntityId, String)> {
        self.service
            .lock()
            .map(|s| s.bubbles(tick))
            .unwrap_or_default()
    }

    /// Called by the loop after each tick. Lines that were used are submitted as `RecordDialogue` inputs, so
    /// they become part of the logged, replayable history.
    pub fn after_tick(&self, events: &[Event], sim: &mut Sim, tick: u64) {
        let (settings, graphic_filter) = self
            .settings
            .lock()
            .map(|g| g.clone())
            .unwrap_or_else(|e| e.into_inner().clone());
        let rules = ContentRules {
            tone_preset: sim.world().settings.tone.name().to_owned(),
            graphic_filter,
        };
        let polled = {
            let mut svc = self.service.lock().unwrap_or_else(|e| e.into_inner());
            svc.observe(events, sim, &settings, &rules);
            svc.poll(tick)
        };
        for r in polled.record {
            let _ = sim.submit_now(pg_core::input::SimInput::Command {
                actor: None,
                cmd: pg_core::commands::Command::RecordDialogue {
                    a: r.a,
                    b: r.b,
                    started: r.started,
                    lines: r.lines,
                },
            });
        }
    }
}

#[cfg(test)]
mod tests;
