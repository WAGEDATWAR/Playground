//! What the UI thread sees of the world (Blueprint §6.1, §14.1).
//!
//! The simulation thread builds an immutable [`RenderSnapshot`] after each batch of ticks and publishes it
//! through a [`SnapshotPublisher`]; the UI thread takes the latest one whenever it draws. The UI never
//! touches `WorldState`, so it can neither stall the simulation nor change it. Map tiles are the bulk of
//! the data, so a map is shared (`Arc`) between consecutive snapshots until it is edited.

use crate::control::RunState;
use pg_core::id::EntityId;
use pg_core::map::{Dir4, MapData, Tile};
use pg_core::pawn::Intent;
use pg_core::pipeline::Event;
use pg_core::world::WorldState;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PawnView {
    pub id: EntityId,
    pub name: String,
    pub map: EntityId,
    pub tile: Tile,
    pub facing: Dir4,
    /// A short label of what the pawn is doing ("idle_at", "free").
    pub activity: String,
    pub walking: bool,
}

#[derive(Clone, Debug)]
pub struct RenderSnapshot {
    pub tick: u64,
    pub day: u64,
    pub minute_of_day: u32,
    pub slot_of_day: u32,
    pub run: RunState,
    pub pawns: Vec<PawnView>,
    /// Maps in id order; unchanged maps are shared with the previous snapshot.
    pub maps: Vec<Arc<MapData>>,
    /// The most recent events (oldest first), for the overlay's event viewer.
    pub recent_events: Vec<Event>,
    /// First 8 hex characters of the state hash at the last day boundary, for the overlay.
    pub day_hash: String,
    /// Ticks of the keyframes the runtime holds, oldest first (the overlay's time scrub).
    pub keyframes: Vec<u64>,
    /// The newest keyframe's state hash, first 8 hex characters.
    pub keyframe_hash: String,
    /// The latest planning failure of each pawn that has one, for the reason explorer (suggestion S-010).
    pub failures: Vec<(String, pg_core::reason::ReasonCode)>,
}

/// How many recent events a snapshot carries.
pub const EVENT_TAIL: usize = 200;

impl RenderSnapshot {
    /// Builds a snapshot of `world`. `prev` lets unchanged maps be shared; `events` are appended to the
    /// previous snapshot's tail (bounded to [`EVENT_TAIL`]).
    pub fn build(
        world: &WorldState,
        run: RunState,
        prev: Option<&RenderSnapshot>,
        events: &[Event],
        day_hash: &str,
    ) -> RenderSnapshot {
        let slot = world.settings.slot_minutes;
        let clock = world.clock;
        let maps = world
            .maps
            .iter()
            .map(|(id, m)| {
                prev.and_then(|p| {
                    p.maps
                        .iter()
                        .find(|pm| pm.id == id && pm.edit_version() == m.edit_version())
                })
                .map_or_else(|| Arc::new(m.clone()), Arc::clone)
            })
            .collect();
        let pawns = world
            .pawns
            .iter()
            .map(|(id, p)| PawnView {
                id,
                name: p.name.clone(),
                map: p.position.map,
                tile: p.position.tile,
                facing: p.facing,
                activity: match (&p.task, p.intent) {
                    (Some(t), _) => t.action.to_string(),
                    (None, Intent::Reservation(_)) => "starting".to_owned(),
                    (None, Intent::Free) => "free".to_owned(),
                },
                walking: p.route.is_some(),
            })
            .collect();
        let mut recent: Vec<Event> = prev.map_or_else(Vec::new, |p| p.recent_events.clone());
        recent.extend_from_slice(events);
        if recent.len() > EVENT_TAIL {
            recent.drain(..recent.len() - EVENT_TAIL);
        }
        RenderSnapshot {
            tick: clock.tick(),
            day: clock.day(),
            minute_of_day: clock.minute_of_day(),
            slot_of_day: clock.slot_of_day(slot),
            run,
            pawns,
            maps,
            recent_events: recent,
            day_hash: day_hash.to_owned(),
            keyframes: prev.map_or_else(Vec::new, |p| p.keyframes.clone()),
            keyframe_hash: prev.map_or_else(String::new, |p| p.keyframe_hash.clone()),
            failures: world
                .pawns
                .iter()
                .filter_map(|(_, p)| p.last_failure.clone().map(|f| (p.name.clone(), f)))
                .collect(),
        }
    }
}

/// Hands the latest snapshot from the simulation thread to any number of readers. Publishing swaps one
/// `Arc` under a short lock; readers clone the `Arc` and then work without any lock.
pub struct SnapshotPublisher<T> {
    slot: Mutex<Arc<T>>,
    version: AtomicU64,
}

impl<T> SnapshotPublisher<T> {
    pub fn new(initial: T) -> SnapshotPublisher<T> {
        SnapshotPublisher {
            slot: Mutex::new(Arc::new(initial)),
            version: AtomicU64::new(0),
        }
    }

    pub fn publish(&self, value: T) {
        *self.slot.lock().unwrap_or_else(|e| e.into_inner()) = Arc::new(value);
        self.version.fetch_add(1, Ordering::SeqCst);
    }

    pub fn latest(&self) -> Arc<T> {
        Arc::clone(&self.slot.lock().unwrap_or_else(|e| e.into_inner()))
    }

    /// Counts publishes; a reader can skip redrawing when it has not changed.
    pub fn version(&self) -> u64 {
        self.version.load(Ordering::SeqCst)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pg_core::map::MapKind;
    use pg_core::pipeline::Event;

    fn world() -> WorldState {
        let mut w = WorldState::new("Snap", "s");
        let m = w.create_map(MapKind::Overworld, 10, 10).unwrap();
        w.spawn_pawn("Ann", m, Tile::new(1, 1)).unwrap();
        w.spawn_pawn("Bob", m, Tile::new(2, 1)).unwrap();
        w
    }

    fn ev(n: u64) -> Event {
        Event {
            tick: n,
            kind: "dev.nudged".into(),
            detail: pg_core::canon::Canon::map([("amount", pg_core::canon::Canon::Int(1))]),
        }
    }

    #[test]
    fn a_snapshot_mirrors_the_world() {
        let w = world();
        let s = RenderSnapshot::build(&w, RunState::Paused, None, &[ev(1)], "abcd1234");
        assert_eq!((s.tick, s.day, s.pawns.len(), s.maps.len()), (0, 0, 2, 1));
        assert_eq!(s.pawns[0].name, "Ann");
        assert_eq!(s.pawns[0].activity, "free");
        assert_eq!(s.run, RunState::Paused);
        assert_eq!(s.recent_events.len(), 1);
        assert_eq!(s.day_hash, "abcd1234");
    }

    #[test]
    fn unchanged_maps_are_shared_and_edited_maps_are_copied() {
        let mut w = world();
        let s1 = RenderSnapshot::build(&w, RunState::Paused, None, &[], "");
        let s2 = RenderSnapshot::build(&w, RunState::Paused, Some(&s1), &[], "");
        assert!(
            Arc::ptr_eq(&s1.maps[0], &s2.maps[0]),
            "no edit, same allocation"
        );
        let m = EntityId::new(pg_core::id::Kind::Map, 1);
        w.maps
            .get_mut(m)
            .unwrap()
            .set_blocked(Tile::new(5, 5), true)
            .unwrap();
        let s3 = RenderSnapshot::build(&w, RunState::Paused, Some(&s2), &[], "");
        assert!(!Arc::ptr_eq(&s2.maps[0], &s3.maps[0]));
        assert!(
            s3.maps[0].is_blocked(Tile::new(5, 5)) && !s2.maps[0].is_blocked(Tile::new(5, 5)),
            "old snapshots are immutable"
        );
    }

    #[test]
    fn the_event_tail_is_bounded_and_keeps_the_newest() {
        let w = world();
        let mut prev: Option<RenderSnapshot> = None;
        for batch in 0..5u64 {
            let events: Vec<Event> = (0..100).map(|i| ev(batch * 100 + i)).collect();
            prev = Some(RenderSnapshot::build(
                &w,
                RunState::Paused,
                prev.as_ref(),
                &events,
                "",
            ));
        }
        let s = prev.unwrap();
        assert_eq!(s.recent_events.len(), EVENT_TAIL);
        assert_eq!(s.recent_events.last().unwrap().tick, 499);
        assert_eq!(s.recent_events.first().unwrap().tick, 300);
    }

    #[test]
    fn the_publisher_hands_over_the_latest_value_to_other_threads() {
        let p = Arc::new(SnapshotPublisher::new(0u64));
        assert_eq!((*p.latest(), p.version()), (0, 0));
        let reader = {
            let p = Arc::clone(&p);
            std::thread::spawn(move || {
                // Spin until the writer's last value shows up; values never go backwards.
                let mut last = 0;
                while last < 1000 {
                    let v = *p.latest();
                    assert!(v >= last);
                    last = v;
                }
            })
        };
        for v in 1..=1000u64 {
            p.publish(v);
        }
        reader.join().unwrap();
        assert_eq!((*p.latest(), p.version()), (1000, 1000));
    }
}
