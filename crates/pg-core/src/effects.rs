//! Preconditions and effects of actions (Blueprint §8.7), checked and applied in one place.
//!
//! An [`crate::action::ActionDef`] may declare [`Precondition`]s, which must hold when a task starts, and
//! [`Effect`]s, which are applied when the task finishes. Effects are a **closed set**: scripts and (later)
//! LLM proposals can only name these, never write state themselves. Applying is two-phase and atomic:
//! [`plan_effects`] resolves and validates every effect against the world (entities exist, ids are known,
//! amounts in range) and only when all of them pass does [`apply_planned`] change anything. Each change goes
//! through the same rules the rest of the core uses (need range, the relationship daily cap and hysteresis,
//! the memory bound), and the hook points that packs may adjust (`relationship.delta_modifier`,
//! `memory.importance_modifier`) apply here too, so a pack's influence is attributed the same way.
//!
//! [`adjust_relationship`] and [`add_memory`] are the shared building blocks; commitment outcomes (2.1)
//! use them directly.

use crate::action::ActionDef;
use crate::canon::{Canon, ToCanon};
use crate::id::{EntityId, Kind};
use crate::memory;
use crate::pipeline::TickCtx;
use crate::reason::ReasonCode;
use crate::social::Memory;
use crate::world::WorldState;
use pg_content::gamedata::GameData;

/// The most preconditions or effects one action may declare.
pub const MAX_PER_ACTION: usize = 8;
/// The largest need change one effect may make (needs run 0 to 1000).
pub const MAX_NEED_DELTA: i32 = 1000;
/// The largest affinity or memory-impact change one effect may ask for.
pub const MAX_SOCIAL_DELTA: i32 = 1000;
/// The widest company radius a precondition may use, in tiles.
pub const MAX_RADIUS: u32 = 16;

/// A condition on the acting pawn that must hold when its task starts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Precondition {
    /// The named capacity (consciousness, moving, manipulation, talking, eating, breathing) is at least `min`.
    Capacity { id: String, min: i32 },
    /// The need is at most `max` (for example "only when hungry").
    NeedAtMost { need: String, max: i32 },
    /// The need is at least `min`.
    NeedAtLeast { need: String, min: i32 },
    /// Another pawn on the same map is within `radius` tiles.
    Company { radius: u32 },
}

/// A change an action makes when its task finishes. The acting pawn is "the pawn"; `with` names a
/// parameter of the action that holds another pawn's id.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Effect {
    /// Adds `delta` to one of the pawn's needs, held to 0..=1000.
    AddNeed { need: String, delta: i32 },
    /// Adds `delta` to the affinity between the pawn and the pawn named by parameter `with`, under the
    /// pair's daily cap.
    AdjustRelationship { with: String, delta: i32 },
    /// Gives the pawn a memory of a registered memory type, optionally about the pawn named by `with`.
    AddMemory {
        with: Option<String>,
        memory_type: String,
        impact: i32,
        summary_key: String,
    },
}

impl Precondition {
    /// A one-line statement of the condition, for `pg actions` and failure reasons.
    pub fn describe(&self) -> String {
        match self {
            Precondition::Capacity { id, min } => format!("{id} capacity at least {min}"),
            Precondition::NeedAtMost { need, max } => format!("{need} at most {max}"),
            Precondition::NeedAtLeast { need, min } => format!("{need} at least {min}"),
            Precondition::Company { radius } => format!("someone within {radius} tiles"),
        }
    }

    /// Whether the condition holds for `pawn`; `Err` says why not.
    pub fn check(&self, world: &WorldState, pawn: EntityId) -> Result<(), String> {
        let Some(p) = world.pawns.get(pawn) else {
            return Err("the pawn is gone".into());
        };
        match self {
            Precondition::Capacity { id, min } => match p.capacities.get(id) {
                Some(v) if v >= *min => Ok(()),
                Some(v) => Err(format!("{id} is {v}, needs {min}")),
                None => Err(format!("unknown capacity '{id}'")),
            },
            Precondition::NeedAtMost { need, max } => match p.needs.get(need) {
                Some(v) if v <= max => Ok(()),
                Some(v) => Err(format!("{need} is {v}, must be at most {max}")),
                None => Err(format!("the pawn has no need '{need}'")),
            },
            Precondition::NeedAtLeast { need, min } => match p.needs.get(need) {
                Some(v) if v >= min => Ok(()),
                Some(v) => Err(format!("{need} is {v}, must be at least {min}")),
                None => Err(format!("the pawn has no need '{need}'")),
            },
            Precondition::Company { radius } => {
                let near = world.pawns.iter().any(|(id, other)| {
                    id != pawn
                        && other.position.map == p.position.map
                        && other.position.tile.manhattan(p.position.tile) <= *radius
                });
                if near {
                    Ok(())
                } else {
                    Err(format!("nobody is within {radius} tiles"))
                }
            }
        }
    }
}

impl Effect {
    /// A one-line statement of the effect, for `pg actions`.
    pub fn describe(&self) -> String {
        match self {
            Effect::AddNeed { need, delta } => format!("{need} {delta:+}"),
            Effect::AdjustRelationship { with, delta } => {
                format!("affinity with <{with}> {delta:+}")
            }
            Effect::AddMemory {
                with,
                memory_type,
                impact,
                ..
            } => match with {
                Some(w) => format!("a {memory_type} memory of <{w}> (impact {impact:+})"),
                None => format!("a {memory_type} memory (impact {impact:+})"),
            },
        }
    }

    /// The parameter this effect reads another pawn from, if any.
    pub fn param(&self) -> Option<&str> {
        match self {
            Effect::AdjustRelationship { with, .. } => Some(with),
            Effect::AddMemory { with, .. } => with.as_deref(),
            Effect::AddNeed { .. } => None,
        }
    }

    /// Static limits every effect must respect, whoever declared it.
    pub fn check_shape(&self) -> Result<(), String> {
        match self {
            Effect::AddNeed { delta, .. } if delta.abs() > MAX_NEED_DELTA => {
                Err(format!("a need change must be within +-{MAX_NEED_DELTA}"))
            }
            Effect::AdjustRelationship { delta, .. } if delta.abs() > MAX_SOCIAL_DELTA => Err(
                format!("an affinity change must be within +-{MAX_SOCIAL_DELTA}"),
            ),
            Effect::AddMemory { impact, .. } if impact.abs() > MAX_SOCIAL_DELTA => Err(format!(
                "a memory impact must be within +-{MAX_SOCIAL_DELTA}"
            )),
            _ => Ok(()),
        }
    }
}

impl Precondition {
    /// Static limits every precondition must respect, whoever declared it.
    pub fn check_shape(&self) -> Result<(), String> {
        match self {
            Precondition::Company { radius } if *radius > MAX_RADIUS => {
                Err(format!("a company radius is at most {MAX_RADIUS} tiles"))
            }
            Precondition::NeedAtMost { max: v, .. } | Precondition::NeedAtLeast { min: v, .. }
                if !(0..=1000).contains(v) =>
            {
                Err("a need threshold is within 0..=1000".into())
            }
            Precondition::Capacity { min, .. } if !(0..=1000).contains(min) => {
                Err("a capacity threshold is within 0..=1000".into())
            }
            _ => Ok(()),
        }
    }
}

fn refused(action: &ActionDef, why: impl Into<String>) -> ReasonCode {
    ReasonCode::builtin(
        "precondition_failed",
        [
            ("action", Canon::str(action.id.as_str())),
            ("why", Canon::str(why)),
        ],
    )
}

/// Checks every precondition of `def` for `pawn`; the first that fails becomes the reason.
pub fn check_preconditions(
    world: &WorldState,
    pawn: EntityId,
    def: &ActionDef,
) -> Result<(), ReasonCode> {
    for pre in &def.preconditions {
        pre.check(world, pawn).map_err(|why| refused(def, why))?;
    }
    Ok(())
}

/// An effect with its parameters resolved and validated against the world.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Planned {
    Need {
        need: String,
        delta: i32,
    },
    Relationship {
        other: EntityId,
        delta: i32,
    },
    Memory {
        other: Option<EntityId>,
        memory_type: String,
        impact: i32,
        summary_key: String,
    },
}

fn other_pawn(
    world: &WorldState,
    pawn: EntityId,
    def: &ActionDef,
    params: &Canon,
    name: &str,
) -> Result<EntityId, ReasonCode> {
    let id: EntityId = params
        .get(name)
        .and_then(Canon::as_str)
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| refused(def, format!("'{name}' does not name a pawn")))?;
    if id == pawn {
        return Err(refused(def, format!("'{name}' is the acting pawn")));
    }
    if world.pawns.get(id).is_none() {
        return Err(refused(def, format!("{id} is no longer here")));
    }
    Ok(id)
}

/// Resolves and validates every effect of `def` for `pawn`. Nothing is changed; `Err` means none of the
/// effects may be applied.
pub fn plan_effects(
    world: &WorldState,
    pawn: EntityId,
    def: &ActionDef,
    params: &Canon,
    data: &GameData,
) -> Result<Vec<Planned>, ReasonCode> {
    let mut out = Vec::new();
    for effect in &def.effects {
        effect.check_shape().map_err(|why| refused(def, why))?;
        out.push(match effect {
            Effect::AddNeed { need, delta } => {
                let has = world
                    .pawns
                    .get(pawn)
                    .is_some_and(|p| p.needs.contains_key(need));
                if !has {
                    return Err(refused(def, format!("the pawn has no need '{need}'")));
                }
                Planned::Need {
                    need: need.clone(),
                    delta: *delta,
                }
            }
            Effect::AdjustRelationship { with, delta } => {
                let other = other_pawn(world, pawn, def, params, with)?;
                if data.relationships.is_none() {
                    return Err(refused(def, "this world has no relationship rules"));
                }
                if world.relationships.get(pawn, other).is_none() {
                    return Err(refused(
                        def,
                        format!("{pawn} and {other} do not know each other"),
                    ));
                }
                Planned::Relationship {
                    other,
                    delta: *delta,
                }
            }
            Effect::AddMemory {
                with,
                memory_type,
                impact,
                summary_key,
            } => {
                let other = match with {
                    Some(w) => Some(other_pawn(world, pawn, def, params, w)?),
                    None => None,
                };
                let known = data
                    .memory
                    .as_ref()
                    .is_some_and(|m| m.types.iter().any(|t| &t.id == memory_type));
                if !known {
                    return Err(refused(def, format!("unknown memory type '{memory_type}'")));
                }
                Planned::Memory {
                    other,
                    memory_type: memory_type.clone(),
                    impact: *impact,
                    summary_key: summary_key.clone(),
                }
            }
        });
    }
    Ok(out)
}

/// Applies planned effects to `pawn`, in declaration order.
pub fn apply_planned(ctx: &mut TickCtx<'_>, pawn: EntityId, planned: &[Planned], data: &GameData) {
    for step in planned {
        match step {
            Planned::Need { need, delta } => {
                if let Some(v) = ctx
                    .world
                    .pawns
                    .get_mut(pawn)
                    .and_then(|p| p.needs.get_mut(need))
                {
                    *v = v.saturating_add(*delta).clamp(0, 1000);
                }
                ctx.mark_changed(pawn);
            }
            Planned::Relationship { other, delta } => {
                adjust_relationship(ctx, pawn, *other, *delta, data);
            }
            Planned::Memory {
                other,
                memory_type,
                impact,
                summary_key,
            } => {
                add_memory(ctx, pawn, *other, memory_type, *impact, summary_key, data);
            }
        }
    }
}

/// Changes the affinity between `a` and `b` by `requested`, through the pair's daily cap and the label
/// hysteresis, after the `relationship.delta_modifier` hook. Returns the change that was applied.
pub fn adjust_relationship(
    ctx: &mut TickCtx<'_>,
    a: EntityId,
    b: EntityId,
    requested: i32,
    data: &GameData,
) -> i32 {
    let Some(params) = &data.relationships else {
        return 0;
    };
    let mut requested = i64::from(requested);
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
    let (day, tick) = (ctx.flags.day, ctx.flags.tick);
    let Some(rel) = ctx.world.relationships.get_mut(a, b) else {
        return 0;
    };
    let result = rel.apply_delta(params, requested, day, tick);
    let affinity = rel.affinity;
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
    result.applied
}

/// Gives `owner` a memory of `memory_type` (about `other`, when there is one) with the type's severity and
/// the requested impact, after the `memory.importance_modifier` hook. Returns the new memory's id, or
/// `None` when the type is unknown or ids are exhausted.
pub fn add_memory(
    ctx: &mut TickCtx<'_>,
    owner: EntityId,
    other: Option<EntityId>,
    memory_type: &str,
    impact: i32,
    summary_key: &str,
    data: &GameData,
) -> Option<EntityId> {
    let params = data.memory.as_ref()?;
    let severity = params.types.iter().find(|t| t.id == memory_type)?.severity;
    let id = ctx.world.id_counters.allocate(Kind::Memory).ok()?;
    let mut m = Memory::new(
        id,
        memory_type,
        ctx.flags.tick,
        other.into_iter().collect(),
        severity,
        impact,
        params.decay_k,
    );
    m.summary_key = Some(summary_key.to_owned());
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
    let gone = memory::remember(ctx.world, owner, m, params);
    ctx.emit(
        "memory.created",
        Canon::map([
            ("pawn", owner.to_canon()),
            ("memory", id.to_canon()),
            ("other", other.map_or(Canon::Null, |o| o.to_canon())),
            ("topic", Canon::str(memory_type)),
            ("importance", importance.to_canon()),
        ]),
    );
    for g in gone {
        ctx.emit(
            "memory.expired",
            Canon::map([
                ("pawn", g.pawn.to_canon()),
                ("memory", g.memory.to_canon()),
                ("why", Canon::str(g.why.name())),
            ]),
        );
    }
    ctx.mark_changed(owner);
    Some(id)
}

#[cfg(test)]
mod tests;
