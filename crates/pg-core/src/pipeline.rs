//! The fixed tick pipeline (Blueprint §6.2).
//!
//! The thirteen built-in system slots run in a fixed order. Extension systems (from content packs,
//! later) attach `Before` or `After` a named slot and run in a total, deterministic order:
//! `(slot, placement, registration sequence, id)`. Each system declares a [`Cadence`]; the pipeline
//! runs it only when the tick's [`TimeFlags`] say that boundary was crossed.

use crate::canon::Canon;
use crate::hash::StateHash;
use crate::id::EntityId;
use crate::map::MoveCosts;
use crate::movement::MovementSystem;
use crate::path::{BatchExecutor, PathCache, SerialExecutor};
use crate::time::TimeFlags;
use crate::world::WorldState;
use std::collections::BTreeSet;
use std::fmt;

/// The built-in slots, in execution order. The discriminant is the order.
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u8)]
pub enum SystemSlot {
    Needs = 1,
    Mood = 2,
    DayPlanner = 3,
    Commitment = 4,
    ReservationActivator = 5,
    TaskPlanner = 6,
    Movement = 7,
    Activity = 8,
    Conversation = 9,
    Memory = 10,
    Relationship = 11,
    Event = 12,
    Maintenance = 13,
}

impl SystemSlot {
    pub const ALL: [SystemSlot; 13] = [
        SystemSlot::Needs,
        SystemSlot::Mood,
        SystemSlot::DayPlanner,
        SystemSlot::Commitment,
        SystemSlot::ReservationActivator,
        SystemSlot::TaskPlanner,
        SystemSlot::Movement,
        SystemSlot::Activity,
        SystemSlot::Conversation,
        SystemSlot::Memory,
        SystemSlot::Relationship,
        SystemSlot::Event,
        SystemSlot::Maintenance,
    ];

    /// The name used by packs and the dev tools (`NeedsSystem`, …).
    pub const fn name(self) -> &'static str {
        match self {
            SystemSlot::Needs => "NeedsSystem",
            SystemSlot::Mood => "MoodSystem",
            SystemSlot::DayPlanner => "DayPlanner",
            SystemSlot::Commitment => "CommitmentSystem",
            SystemSlot::ReservationActivator => "ReservationActivator",
            SystemSlot::TaskPlanner => "TaskPlanner",
            SystemSlot::Movement => "MovementSystem",
            SystemSlot::Activity => "ActivitySystem",
            SystemSlot::Conversation => "ConversationSystem",
            SystemSlot::Memory => "MemorySystem",
            SystemSlot::Relationship => "RelationshipSystem",
            SystemSlot::Event => "EventSystem",
            SystemSlot::Maintenance => "Maintenance",
        }
    }

    pub fn from_name(name: &str) -> Option<SystemSlot> {
        SystemSlot::ALL.into_iter().find(|s| s.name() == name)
    }

    /// The slot's cadence from the Blueprint's pipeline table. Event-driven slots (Memory,
    /// Relationship) are driven from the day boundary and by events until those exist.
    pub const fn cadence(self) -> Cadence {
        match self {
            SystemSlot::Needs | SystemSlot::Mood => Cadence::Minute,
            SystemSlot::DayPlanner | SystemSlot::Memory => Cadence::Day,
            SystemSlot::Commitment | SystemSlot::ReservationActivator | SystemSlot::Event => {
                Cadence::Slot
            }
            SystemSlot::TaskPlanner
            | SystemSlot::Movement
            | SystemSlot::Activity
            | SystemSlot::Conversation
            | SystemSlot::Relationship
            | SystemSlot::Maintenance => Cadence::Tick,
        }
    }
}

/// How often a system runs.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Cadence {
    Tick,
    Minute,
    Slot,
    Day,
}

impl Cadence {
    pub const fn due(self, flags: &TimeFlags) -> bool {
        match self {
            Cadence::Tick => true,
            Cadence::Minute => flags.minute,
            Cadence::Slot => flags.slot,
            Cadence::Day => flags.new_day,
        }
    }
}

/// Before or after a built-in slot.
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Placement {
    Before,
    After,
}

/// A recorded event (Appendix B). Kinds are text so packs can add `<pack>.<type>`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Event {
    pub tick: u64,
    pub kind: String,
    pub detail: Canon,
}

/// What a step did, for snapshots, logging and the dev tools.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TickReport {
    /// The tick just entered.
    pub tick: u64,
    pub flags: TimeFlags,
    pub inputs_applied: u32,
    pub changed: BTreeSet<EntityId>,
    pub events: Vec<Event>,
    /// Set on the tick a day boundary was crossed.
    pub day_hash: Option<StateHash>,
    /// System ids in the order they ran; filled only when tracing is on.
    pub ran: Vec<String>,
}

/// Derived, shared services for systems: how to solve path batches, the path cache and the movement cost
/// table. None of it is saved or hashed, and none of it can change a result, only how fast it is computed.
pub struct Services {
    pub exec: Box<dyn BatchExecutor>,
    pub paths: PathCache,
    pub costs: MoveCosts,
}

impl Services {
    pub fn new() -> Services {
        Services {
            exec: Box::new(SerialExecutor),
            paths: PathCache::default(),
            costs: MoveCosts::default(),
        }
    }
}

impl Default for Services {
    fn default() -> Self {
        Services::new()
    }
}

/// What a system sees and can report while running.
pub struct TickCtx<'a> {
    pub world: &'a mut WorldState,
    pub flags: &'a TimeFlags,
    pub services: &'a mut Services,
    report: &'a mut TickReport,
}

impl<'a> TickCtx<'a> {
    pub(crate) fn new(
        world: &'a mut WorldState,
        flags: &'a TimeFlags,
        report: &'a mut TickReport,
        services: &'a mut Services,
    ) -> Self {
        TickCtx {
            world,
            flags,
            services,
            report,
        }
    }

    /// Records an event for this tick.
    pub fn emit(&mut self, kind: impl Into<String>, detail: Canon) {
        self.report.events.push(Event {
            tick: self.flags.tick,
            kind: kind.into(),
            detail,
        });
    }

    /// Marks an entity as changed this tick.
    pub fn mark_changed(&mut self, id: EntityId) {
        self.report.changed.insert(id);
    }
}

/// A unit of simulation behavior. State lives in the world, never in the system, so a system can be
/// rebuilt at any time (this is what makes snapshots and the VM-reload test variant possible).
pub trait System: Send {
    fn id(&self) -> &str;
    fn run(&mut self, ctx: &mut TickCtx<'_>);
}

/// A built-in slot with nothing to do yet.
struct NoOp(&'static str);

impl System for NoOp {
    fn id(&self) -> &str {
        self.0
    }
    fn run(&mut self, _ctx: &mut TickCtx<'_>) {}
}

/// Why an extension could not be registered.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RegisterError {
    DuplicateId(String),
}

impl fmt::Display for RegisterError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RegisterError::DuplicateId(id) => {
                write!(f, "a system with id '{id}' is already registered")
            }
        }
    }
}

impl std::error::Error for RegisterError {}

struct Entry {
    slot: SystemSlot,
    /// `None` for the built-in slot system itself.
    placement: Option<Placement>,
    seq: u64,
    cadence: Cadence,
    system: Box<dyn System>,
}

/// The ordered set of systems that make up one tick.
pub struct Pipeline {
    entries: Vec<Entry>,
    order: Vec<usize>,
    next_seq: u64,
}

impl Default for Pipeline {
    fn default() -> Self {
        Pipeline::new()
    }
}

impl Pipeline {
    /// A pipeline with a no-op system in each of the thirteen slots.
    pub fn new() -> Pipeline {
        let entries = SystemSlot::ALL
            .into_iter()
            .map(|slot| Entry {
                slot,
                placement: None,
                seq: 0,
                cadence: slot.cadence(),
                system: if slot == SystemSlot::Movement {
                    Box::new(MovementSystem)
                } else {
                    Box::new(NoOp(slot.name()))
                },
            })
            .collect();
        let mut p = Pipeline {
            entries,
            order: Vec::new(),
            next_seq: 1,
        };
        p.rebuild_order();
        p
    }

    /// Replaces the system in a built-in slot (its cadence stays the slot's).
    pub fn set_builtin(&mut self, slot: SystemSlot, system: Box<dyn System>) {
        if let Some(e) = self
            .entries
            .iter_mut()
            .find(|e| e.placement.is_none() && e.slot == slot)
        {
            e.system = system;
        }
    }

    /// Registers an extension system next to a built-in slot. Registration order is the pack load order.
    pub fn add_extension(
        &mut self,
        slot: SystemSlot,
        placement: Placement,
        cadence: Cadence,
        system: Box<dyn System>,
    ) -> Result<(), RegisterError> {
        if self.entries.iter().any(|e| e.system.id() == system.id()) {
            return Err(RegisterError::DuplicateId(system.id().to_owned()));
        }
        let seq = self.next_seq;
        self.next_seq += 1;
        self.entries.push(Entry {
            slot,
            placement: Some(placement),
            seq,
            cadence,
            system,
        });
        self.rebuild_order();
        Ok(())
    }

    fn rebuild_order(&mut self) {
        let mut idx: Vec<usize> = (0..self.entries.len()).collect();
        // Total order: slot; then Before-extensions, the built-in, After-extensions; then
        // registration sequence; then id.
        let rank = |e: &Entry| match e.placement {
            Some(Placement::Before) => 0u8,
            None => 1,
            Some(Placement::After) => 2,
        };
        idx.sort_by(|&a, &b| {
            let (ea, eb) = match (self.entries.get(a), self.entries.get(b)) {
                (Some(x), Some(y)) => (x, y),
                _ => return std::cmp::Ordering::Equal,
            };
            ea.slot
                .cmp(&eb.slot)
                .then(rank(ea).cmp(&rank(eb)))
                .then(ea.seq.cmp(&eb.seq))
                .then_with(|| ea.system.id().cmp(eb.system.id()))
        });
        self.order = idx;
    }

    /// System ids in execution order (the dev tools print this).
    pub fn order(&self) -> Vec<String> {
        self.order
            .iter()
            .filter_map(|&i| self.entries.get(i))
            .map(|e| e.system.id().to_owned())
            .collect()
    }

    /// `(system id, placement label, cadence)` in execution order, for the dev tools.
    pub fn describe(&self) -> Vec<(String, &'static str, Cadence)> {
        self.order
            .iter()
            .filter_map(|&i| self.entries.get(i))
            .map(|e| {
                let label = match e.placement {
                    Some(Placement::Before) => "before",
                    None => "builtin",
                    Some(Placement::After) => "after",
                };
                (e.system.id().to_owned(), label, e.cadence)
            })
            .collect()
    }

    pub(crate) fn run(
        &mut self,
        world: &mut WorldState,
        flags: &TimeFlags,
        report: &mut TickReport,
        services: &mut Services,
        trace: bool,
    ) {
        let order = self.order.clone();
        for i in order {
            if let Some(entry) = self.entries.get_mut(i) {
                if !entry.cadence.due(flags) {
                    continue;
                }
                if trace {
                    report.ran.push(entry.system.id().to_owned());
                }
                let mut ctx = TickCtx::new(world, flags, report, services);
                entry.system.run(&mut ctx);
            }
        }
    }
}

impl TickReport {
    pub(crate) fn new(flags: TimeFlags) -> TickReport {
        TickReport {
            tick: flags.tick,
            flags,
            inputs_applied: 0,
            changed: BTreeSet::new(),
            events: Vec::new(),
            day_hash: None,
            ran: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::time::{flags_for, SlotMinutes};
    use std::sync::{Arc, Mutex};

    struct Recorder {
        id: String,
        log: Arc<Mutex<Vec<String>>>,
    }

    impl System for Recorder {
        fn id(&self) -> &str {
            &self.id
        }
        fn run(&mut self, _ctx: &mut TickCtx<'_>) {
            self.log.lock().unwrap().push(self.id.clone());
        }
    }

    fn rec(id: &str, log: &Arc<Mutex<Vec<String>>>) -> Box<dyn System> {
        Box::new(Recorder {
            id: id.to_owned(),
            log: Arc::clone(log),
        })
    }

    fn run_at(p: &mut Pipeline, tick: u64) -> TickReport {
        let flags = flags_for(tick, SlotMinutes::DEFAULT);
        let mut world = WorldState::new("t", "s");
        let mut report = TickReport::new(flags);
        p.run(&mut world, &flags, &mut report, &mut Services::new(), true);
        report
    }

    #[test]
    fn builtin_slots_are_in_the_blueprint_order() {
        let names: Vec<_> = Pipeline::new().order();
        assert_eq!(
            names,
            [
                "NeedsSystem",
                "MoodSystem",
                "DayPlanner",
                "CommitmentSystem",
                "ReservationActivator",
                "TaskPlanner",
                "MovementSystem",
                "ActivitySystem",
                "ConversationSystem",
                "MemorySystem",
                "RelationshipSystem",
                "EventSystem",
                "Maintenance",
            ]
        );
    }

    #[test]
    fn slot_names_round_trip() {
        for s in SystemSlot::ALL {
            assert_eq!(SystemSlot::from_name(s.name()), Some(s));
        }
        assert_eq!(SystemSlot::from_name("Nope"), None);
    }

    #[test]
    fn cadence_gates_what_runs() {
        let mut p = Pipeline::new();
        let ran = |r: &TickReport| r.ran.clone();
        // An ordinary tick: only per-tick slots.
        assert_eq!(
            ran(&run_at(&mut p, 7)),
            [
                "TaskPlanner",
                "MovementSystem",
                "ActivitySystem",
                "ConversationSystem",
                "RelationshipSystem",
                "Maintenance"
            ]
        );
        // A minute boundary adds Needs and Mood.
        let r = ran(&run_at(&mut p, 10));
        assert!(r.starts_with(&["NeedsSystem".to_owned(), "MoodSystem".to_owned()]));
        assert!(!r.contains(&"CommitmentSystem".to_owned()));
        // A slot boundary adds the slot systems; a day boundary adds the day systems.
        assert!(ran(&run_at(&mut p, 300)).contains(&"CommitmentSystem".to_owned()));
        let day = ran(&run_at(&mut p, 14_400));
        for needed in ["DayPlanner", "MemorySystem", "EventSystem", "NeedsSystem"] {
            assert!(day.contains(&needed.to_owned()), "{needed}");
        }
    }

    #[test]
    fn extensions_run_before_and_after_their_anchor() {
        let log = Arc::new(Mutex::new(Vec::new()));
        let mut p = Pipeline::new();
        p.add_extension(
            SystemSlot::Needs,
            Placement::After,
            Cadence::Minute,
            rec("pack.after_needs", &log),
        )
        .unwrap();
        p.add_extension(
            SystemSlot::Needs,
            Placement::Before,
            Cadence::Minute,
            rec("pack.before_needs", &log),
        )
        .unwrap();
        let order = p.order();
        let pos = |n: &str| order.iter().position(|x| x == n).unwrap();
        assert!(pos("pack.before_needs") < pos("NeedsSystem"));
        assert!(pos("NeedsSystem") < pos("pack.after_needs"));
        assert!(pos("pack.after_needs") < pos("MoodSystem"));
        run_at(&mut p, 10);
        assert_eq!(
            *log.lock().unwrap(),
            ["pack.before_needs", "pack.after_needs"]
        );
    }

    #[test]
    fn same_anchor_orders_by_registration_then_never_by_insertion_accident() {
        let log = Arc::new(Mutex::new(Vec::new()));
        let mut p = Pipeline::new();
        for id in ["pack.c", "pack.a", "pack.b"] {
            p.add_extension(
                SystemSlot::Mood,
                Placement::After,
                Cadence::Tick,
                rec(id, &log),
            )
            .unwrap();
        }
        let order = p.order();
        let after_mood: Vec<_> = order
            .iter()
            .skip_while(|n| *n != "MoodSystem")
            .skip(1)
            .take(3)
            .cloned()
            .collect();
        // Registration order (the pack load order), not alphabetical.
        assert_eq!(after_mood, ["pack.c", "pack.a", "pack.b"]);
    }

    #[test]
    fn duplicate_ids_are_rejected() {
        let log = Arc::new(Mutex::new(Vec::new()));
        let mut p = Pipeline::new();
        p.add_extension(
            SystemSlot::Needs,
            Placement::After,
            Cadence::Tick,
            rec("pack.x", &log),
        )
        .unwrap();
        let err = p
            .add_extension(
                SystemSlot::Mood,
                Placement::After,
                Cadence::Tick,
                rec("pack.x", &log),
            )
            .unwrap_err();
        assert_eq!(err, RegisterError::DuplicateId("pack.x".into()));
        let err = p
            .add_extension(
                SystemSlot::Mood,
                Placement::After,
                Cadence::Tick,
                rec("NeedsSystem", &log),
            )
            .unwrap_err();
        assert_eq!(err, RegisterError::DuplicateId("NeedsSystem".into()));
    }

    #[test]
    fn a_builtin_slot_can_be_replaced_and_keeps_its_cadence() {
        let log = Arc::new(Mutex::new(Vec::new()));
        let mut p = Pipeline::new();
        p.set_builtin(SystemSlot::Needs, rec("needs.real", &log));
        run_at(&mut p, 7);
        assert!(log.lock().unwrap().is_empty(), "Needs is minute-cadence");
        run_at(&mut p, 10);
        assert_eq!(*log.lock().unwrap(), ["needs.real"]);
    }
}
