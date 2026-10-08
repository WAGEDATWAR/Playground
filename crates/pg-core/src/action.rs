//! The closed action registry (Blueprint §8.7).
//!
//! All pawn behaviour flows through actions. An [`ActionDef`] names the action, types its parameters, and
//! lists the steps a task performs. The registry is **closed**: built-ins are registered first, packs add
//! theirs during the load phase, then [`ActionRegistry::freeze`] shuts it. Anything that later asks for an
//! unregistered action (a schedule, a script, an LLM proposal) is refused with `unknown_action`.
//!
//! Milestone 0.5 ships the skeleton: `move_to` and `idle_at`, with `MoveTo` and `PerformFor` steps.
//! Preconditions, permissions, costs and effects arrive with the systems that need them (Needs in Stage 1).

use crate::canon::Canon;
use crate::map::Tile;
use crate::pawn::Step;
use crate::reason::ReasonCode;
use pg_content::schema::{Field, FieldSchema, ParamSchema};
use pg_content::{ActionId, Origin, ValidationReport};
use std::collections::BTreeMap;
use std::fmt;

/// How close (Manhattan tiles) a pawn must get to count as having arrived at a gathering. Two pawns cannot
/// stand on one tile, so a meeting place is an area, not a tile.
pub const MEET_RADIUS: u32 = 2;

/// One step of an action, before its parameters are resolved.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StepTemplate {
    /// Walk to the tile in the named `Tile` parameter, stopping once within `within` tiles of it.
    MoveTo { param: String, within: u32 },
    /// Stay put until the end of the reservation that started the task.
    PerformUntilSlotEnd,
}

#[derive(Clone, Debug)]
pub struct ActionDef {
    pub id: ActionId,
    pub origin: Origin,
    pub params: ParamSchema,
    pub steps: Vec<StepTemplate>,
    /// Whether a replan may cut the task short.
    pub interruptible: bool,
    /// Only meaningful with the `ai` capability (Stage 10).
    pub ai_proposable: bool,
    /// One line for `pg actions`.
    pub summary: String,
    /// Needs this action restores while a pawn performs it (Blueprint §8.1).
    pub restores: Vec<Restore>,
}

/// A need an action restores while it is being performed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Restore {
    pub need: String,
    /// Points per game hour.
    pub per_hour: u32,
    /// Only counts while another pawn is within [`COMPANY_RADIUS`] tiles.
    pub company: bool,
}

/// How near another pawn must be for company-dependent restoring (a chat, a shared meal).
pub const COMPANY_RADIUS: u32 = 5;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RegisterError {
    Closed,
    Duplicate(ActionId),
    /// A pack action must be `<pack>.<name>`; a built-in must not contain a dot.
    BadNamespace {
        id: ActionId,
        expected: String,
    },
    /// A `MoveTo` step names a parameter that is not a `Tile` field.
    BadStep {
        id: ActionId,
        param: String,
    },
}

impl fmt::Display for RegisterError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RegisterError::Closed => write!(f, "the action registry is closed"),
            RegisterError::Duplicate(id) => write!(f, "action '{id}' is already registered"),
            RegisterError::BadNamespace { id, expected } => {
                write!(f, "action '{id}' must be named {expected}")
            }
            RegisterError::BadStep { id, param } => write!(
                f,
                "action '{id}' has a MoveTo step on '{param}', which is not a tile parameter"
            ),
        }
    }
}

impl std::error::Error for RegisterError {}

/// A task's steps, resolved from an action and its parameters.
pub struct Instantiated {
    pub steps: Vec<Step>,
    pub interruptible: bool,
}

#[derive(Clone, Debug)]
pub struct ActionRegistry {
    defs: BTreeMap<ActionId, ActionDef>,
    frozen: bool,
}

impl Default for ActionRegistry {
    fn default() -> Self {
        ActionRegistry::new()
    }
}

impl ActionRegistry {
    /// An empty, open registry.
    pub fn new() -> ActionRegistry {
        ActionRegistry {
            defs: BTreeMap::new(),
            frozen: false,
        }
    }

    /// The built-in actions, frozen.
    pub fn builtin() -> ActionRegistry {
        let mut r = ActionRegistry::new();
        let tile = || Field::required(FieldSchema::Tile);
        let defs = [
            ActionDef {
                id: id("move_to"),
                origin: Origin::Builtin,
                params: ParamSchema::new().field("to", tile()),
                steps: vec![StepTemplate::MoveTo {
                    param: "to".into(),
                    within: 0,
                }],
                interruptible: true,
                ai_proposable: true,
                summary: "Walk to a tile.".into(),
                restores: Vec::new(),
            },
            ActionDef {
                id: id("idle_at"),
                origin: Origin::Builtin,
                params: ParamSchema::new().field("at", tile()),
                steps: vec![
                    StepTemplate::MoveTo {
                        param: "at".into(),
                        within: 0,
                    },
                    StepTemplate::PerformUntilSlotEnd,
                ],
                interruptible: true,
                ai_proposable: true,
                summary: "Walk to a tile and stay there for the reserved time.".into(),
                restores: Vec::new(),
            },
            ActionDef {
                id: id("meet_at"),
                origin: Origin::Builtin,
                params: ParamSchema::new().field("at", tile()),
                steps: vec![
                    StepTemplate::MoveTo {
                        param: "at".into(),
                        within: MEET_RADIUS,
                    },
                    StepTemplate::PerformUntilSlotEnd,
                ],
                interruptible: true,
                ai_proposable: false,
                summary: "Gather within a couple of tiles of a place (used by commitments).".into(),
                restores: Vec::new(),
            },
        ];
        // Actions that restore a need while they are performed (Blueprint §8.1): walk to a tile and stay.
        let restoring =
            |name: &str, summary: &str, within: u32, need: &str, per_hour: u32, company: bool| {
                ActionDef {
                    id: id(name),
                    origin: Origin::Builtin,
                    params: ParamSchema::new().field("at", tile()),
                    steps: vec![
                        StepTemplate::MoveTo {
                            param: "at".into(),
                            within,
                        },
                        StepTemplate::PerformUntilSlotEnd,
                    ],
                    interruptible: true,
                    ai_proposable: true,
                    summary: summary.to_owned(),
                    restores: vec![Restore {
                        need: need.to_owned(),
                        per_hour,
                        company,
                    }],
                }
            };
        let defs = defs.into_iter().chain([
            restoring(
                "eat",
                "Walk to a tile and have a meal (restores hunger).",
                0,
                "hunger",
                700,
                false,
            ),
            restoring(
                "sleep",
                "Walk to a tile and sleep (restores energy).",
                0,
                "energy",
                180,
                false,
            ),
            restoring(
                "rest",
                "Walk to a tile and rest (restores some energy).",
                0,
                "energy",
                60,
                false,
            ),
            restoring(
                "socialise",
                "Go to a place and spend time with others (restores social while someone is near).",
                MEET_RADIUS + 1,
                "social",
                300,
                true,
            ),
        ]);
        for d in defs {
            // The built-in ids are distinct and valid, and the registry is open here.
            let _ = r.register(d);
        }
        r.freeze();
        r
    }

    /// Registers an action. Refused once the registry is frozen, for a duplicate id, a wrongly namespaced
    /// id, or a step that names a parameter that is not a tile.
    pub fn register(&mut self, def: ActionDef) -> Result<(), RegisterError> {
        if self.frozen {
            return Err(RegisterError::Closed);
        }
        match &def.origin {
            Origin::Builtin if def.id.segments().count() != 1 => {
                return Err(RegisterError::BadNamespace {
                    id: def.id,
                    expected: "a single word".into(),
                })
            }
            Origin::Pack(p) if def.id.segments().next() != Some(p.as_str()) => {
                return Err(RegisterError::BadNamespace {
                    expected: format!("{p}.<name>"),
                    id: def.id,
                })
            }
            _ => {}
        }
        for step in &def.steps {
            if let StepTemplate::MoveTo { param, .. } = step {
                let is_tile = def
                    .params
                    .fields
                    .get(param)
                    .is_some_and(|f| matches!(f.schema, FieldSchema::Tile));
                if !is_tile {
                    return Err(RegisterError::BadStep {
                        id: def.id,
                        param: param.clone(),
                    });
                }
            }
        }
        if self.defs.contains_key(&def.id) {
            return Err(RegisterError::Duplicate(def.id));
        }
        self.defs.insert(def.id.clone(), def);
        Ok(())
    }

    /// Closes the registry. Called once the load phase ends.
    pub fn freeze(&mut self) {
        self.frozen = true;
    }

    pub fn is_frozen(&self) -> bool {
        self.frozen
    }

    pub fn get(&self, id: &ActionId) -> Option<&ActionDef> {
        self.defs.get(id)
    }

    /// Actions in id order.
    pub fn iter(&self) -> impl Iterator<Item = &ActionDef> {
        self.defs.values()
    }

    /// Checks `params` against the action's schema and returns the normalised parameters.
    pub fn validate(&self, action: &ActionId, params: &Canon) -> Result<Canon, ReasonCode> {
        let def = self.get(action).ok_or_else(|| {
            ReasonCode::builtin("unknown_action", [("action", Canon::str(action.as_str()))])
        })?;
        let mut report = ValidationReport::new();
        let clean = def.params.check(params, "params", &mut report);
        if report.is_ok() {
            Ok(clean)
        } else {
            let why = report
                .errors()
                .map(|i| format!("{}: {}", i.path, i.message))
                .collect::<Vec<_>>()
                .join("; ");
            Err(ReasonCode::builtin(
                "bad_params",
                [("why", Canon::str(why))],
            ))
        }
    }

    /// Resolves an action into concrete steps. `end_tick` is the tick at which the reservation ends.
    pub fn instantiate(
        &self,
        action: &ActionId,
        params: &Canon,
        end_tick: u64,
    ) -> Result<Instantiated, ReasonCode> {
        let clean = self.validate(action, params)?;
        let def = self.get(action).ok_or_else(|| {
            ReasonCode::builtin("unknown_action", [("action", Canon::str(action.as_str()))])
        })?;
        let mut steps = Vec::new();
        for t in &def.steps {
            match t {
                StepTemplate::MoveTo { param, within } => {
                    let tile = clean.get(param).and_then(Tile::from_canon).ok_or_else(|| {
                        ReasonCode::builtin(
                            "bad_params",
                            [("why", Canon::str(format!("'{param}' is not a tile")))],
                        )
                    })?;
                    steps.push(Step::MoveTo {
                        goal: tile,
                        within: *within,
                    });
                }
                StepTemplate::PerformUntilSlotEnd => steps.push(Step::PerformUntil(end_tick)),
            }
        }
        Ok(Instantiated {
            steps,
            interruptible: def.interruptible,
        })
    }
}

fn id(s: &str) -> ActionId {
    // Only called with literals that are valid ids; the unit tests prove it.
    ActionId::new(s).unwrap_or_else(|_| ActionId::new("move_to").unwrap_or_else(|_| unreachable!()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::canon::ToCanon;
    use pg_content::PackId;

    fn tile(x: i32, y: i32) -> Canon {
        Tile::new(x, y).to_canon()
    }

    fn params(key: &str, v: Canon) -> Canon {
        Canon::map([(key, v)])
    }

    fn pack_action(pack: &str, name: &str) -> ActionDef {
        ActionDef {
            id: ActionId::new(&format!("{pack}.{name}")).unwrap(),
            origin: Origin::Pack(PackId::new(pack).unwrap()),
            params: ParamSchema::new().field("to", Field::required(FieldSchema::Tile)),
            steps: vec![StepTemplate::MoveTo {
                param: "to".into(),
                within: 0,
            }],
            interruptible: true,
            ai_proposable: false,
            summary: "test".into(),
            restores: Vec::new(),
        }
    }

    #[test]
    fn builtin_registry_has_the_skeleton_actions_and_is_closed() {
        let r = ActionRegistry::builtin();
        assert!(r.is_frozen());
        let ids: Vec<&str> = r.iter().map(|d| d.id.as_str()).collect();
        assert_eq!(
            ids,
            [
                "eat",
                "idle_at",
                "meet_at",
                "move_to",
                "rest",
                "sleep",
                "socialise"
            ]
        );
        let mut r = r;
        assert_eq!(
            r.register(pack_action("coffee", "brew")),
            Err(RegisterError::Closed)
        );
    }

    #[test]
    fn open_registry_accepts_namespaced_pack_actions_only() {
        let mut r = ActionRegistry::new();
        r.register(pack_action("coffee", "brew")).unwrap();
        assert_eq!(
            r.register(pack_action("coffee", "brew")),
            Err(RegisterError::Duplicate(
                ActionId::new("coffee.brew").unwrap()
            ))
        );
        // Wrong namespace: the action id's first segment is not the registering pack.
        let mut bad = pack_action("coffee", "brew2");
        bad.origin = Origin::Pack(PackId::new("other").unwrap());
        assert!(matches!(
            r.register(bad),
            Err(RegisterError::BadNamespace { .. })
        ));
        // A pack may not take a built-in name.
        let mut squatter = pack_action("coffee", "x");
        squatter.id = ActionId::new("move_to").unwrap();
        assert!(matches!(
            r.register(squatter),
            Err(RegisterError::BadNamespace { .. })
        ));
        // A built-in id with a dot is refused too.
        let mut dotted = pack_action("coffee", "x");
        dotted.origin = Origin::Builtin;
        assert!(matches!(
            r.register(dotted),
            Err(RegisterError::BadNamespace { .. })
        ));
        r.freeze();
        assert_eq!(
            r.register(pack_action("coffee", "later")),
            Err(RegisterError::Closed)
        );
    }

    #[test]
    fn a_move_step_must_name_a_tile_parameter() {
        let mut r = ActionRegistry::new();
        let mut d = pack_action("coffee", "brew");
        d.steps = vec![StepTemplate::MoveTo {
            param: "nope".into(),
            within: 0,
        }];
        assert!(matches!(r.register(d), Err(RegisterError::BadStep { .. })));
        let mut d = pack_action("coffee", "brew");
        d.params = ParamSchema::new().field("to", Field::required_int(0, 5));
        assert!(matches!(r.register(d), Err(RegisterError::BadStep { .. })));
    }

    #[test]
    fn instantiating_resolves_parameters_into_steps() {
        let r = ActionRegistry::builtin();
        let got = r
            .instantiate(&id("idle_at"), &params("at", tile(3, 4)), 900)
            .unwrap();
        assert_eq!(
            got.steps,
            [
                Step::MoveTo {
                    goal: Tile::new(3, 4),
                    within: 0
                },
                Step::PerformUntil(900)
            ]
        );
        let got = r
            .instantiate(&id("move_to"), &params("to", tile(0, 0)), 900)
            .unwrap();
        assert_eq!(
            got.steps,
            [Step::MoveTo {
                goal: Tile::new(0, 0),
                within: 0
            }]
        );
        let got = r
            .instantiate(&id("meet_at"), &params("at", tile(1, 1)), 5)
            .unwrap();
        assert!(matches!(
            got.steps[0],
            Step::MoveTo {
                within: MEET_RADIUS,
                ..
            }
        ));
    }

    #[test]
    fn bad_requests_become_reason_codes() {
        let r = ActionRegistry::builtin();
        let unknown = r.instantiate(&id("fly"), &Canon::Null, 0).err().unwrap();
        assert_eq!(unknown.code, "unknown_action");
        assert_eq!(unknown.explain(), "No action named fly is registered.");
        let bad = r
            .instantiate(&id("move_to"), &params("to", Canon::str("x")), 0)
            .err()
            .unwrap();
        assert_eq!(bad.code, "bad_params");
        assert!(bad.explain().contains("params.to"), "{}", bad.explain());
        let missing = r
            .validate(&id("move_to"), &Canon::map::<&str>([]))
            .unwrap_err();
        assert_eq!(missing.code, "bad_params");
    }

    #[test]
    fn builtin_ids_are_valid() {
        // `id()` falls back silently, so prove the literals parse.
        for s in ["move_to", "idle_at", "meet_at"] {
            assert!(ActionId::new(s).is_ok());
        }
    }
}
