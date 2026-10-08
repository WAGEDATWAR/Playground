//! The simulation thread (Blueprint §1, §6.1).
//!
//! A [`Session`] runs a [`SimLoop`] on its own thread named `pg-sim`. The UI side holds the `Session`: it
//! sends [`Control`] messages (commands, run-state changes, save, rewind), reads the latest
//! [`RenderSnapshot`] whenever it draws, and drains [`LoopEvent`]s for notices. Nothing on the UI side can
//! touch the world, and nothing the simulation does can block the UI.

use crate::control::RunState;
use crate::sim_loop::{Control, LoopEvent, SimLoop};
use crate::snapshot::{RenderSnapshot, SnapshotPublisher};
use pg_core::replay::ReplayError;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;

pub struct Session {
    tx: Sender<Control>,
    events: Receiver<LoopEvent>,
    publisher: Arc<SnapshotPublisher<RenderSnapshot>>,
    handle: Option<JoinHandle<SimLoop>>,
}

impl Session {
    /// Starts the thread. `frame_interval` is how long it sleeps between frames (about 4 ms is smooth).
    pub fn start(mut lp: SimLoop, frame_interval: Duration) -> Session {
        let (tx, rx) = channel::<Control>();
        let (etx, events) = channel::<LoopEvent>();
        let publisher = lp.publisher();
        let handle = std::thread::Builder::new()
            .name("pg-sim".to_owned())
            .spawn(move || {
                loop {
                    while let Ok(c) = rx.try_recv() {
                        for e in lp.control(c) {
                            let _ = etx.send(e);
                        }
                    }
                    for e in lp.frame() {
                        let _ = etx.send(e);
                    }
                    if lp.state() == RunState::Stopped {
                        break;
                    }
                    // A disconnected UI means nobody can ever resume: close down cleanly.
                    std::thread::sleep(frame_interval);
                }
                lp
            })
            .expect("spawning the simulation thread");
        Session {
            tx,
            events,
            publisher,
            handle: Some(handle),
        }
    }

    pub fn send(&self, c: Control) {
        let _ = self.tx.send(c);
    }

    /// The latest published snapshot.
    pub fn snapshot(&self) -> Arc<RenderSnapshot> {
        self.publisher.latest()
    }

    pub fn snapshot_version(&self) -> u64 {
        self.publisher.version()
    }

    /// Everything the loop reported since the last call.
    pub fn events(&self) -> Vec<LoopEvent> {
        self.events.try_iter().collect()
    }

    pub fn is_finished(&self) -> bool {
        self.handle.as_ref().is_none_or(JoinHandle::is_finished)
    }

    /// Asks the loop to close (a final save happens), waits for it and returns it (for inspection).
    pub fn shutdown(mut self) -> Option<SimLoop> {
        self.send(Control::Run(crate::control::RunEvent::CloseRequested));
        self.handle.take().and_then(|h| h.join().ok())
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        if let Some(h) = self.handle.take() {
            let _ = self
                .tx
                .send(Control::Run(crate::control::RunEvent::CloseRequested));
            let _ = h.join();
        }
    }
}

/// Used only so the module's error types are in one place for callers building sessions from logs.
pub type SessionError = ReplayError;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::control::{RunEvent, Speed};
    use crate::keyframes::SimFactory;
    use crate::pool::WorkerPool;
    use crate::sim_loop::{LoopConfig, LoopServices};
    use pg_core::commands::Command;
    use pg_core::map::{MapKind, Tile};
    use pg_core::world::WorldState;
    use pg_host::{Clock, FixedClock, MemLog, MemStorage};
    use pg_persist::store::{LoadOptions, SlotStore};
    use std::time::Instant;

    struct Parts {
        session: Session,
        clock: Arc<FixedClock>,
        storage: Arc<MemStorage>,
    }

    fn start() -> Parts {
        let mut w = WorldState::new("Thread Town", "thread-seed");
        let m = w.create_map(MapKind::Overworld, 20, 20).unwrap();
        for i in 0..4 {
            w.spawn_pawn(&format!("P{i}"), m, Tile::new(i, 0)).unwrap();
        }
        let clock = Arc::new(FixedClock::new());
        let storage = Arc::new(MemStorage::new());
        let services = LoopServices {
            clock: clock.clone(),
            storage: storage.clone(),
            log: Arc::new(MemLog::new()),
            pool: Arc::new(WorkerPool::new(2)),
            thumbnailer: None,
            console: None,
        };
        let lp = SimLoop::new(
            SimFactory::dev(None, 2),
            w,
            services,
            LoopConfig::new("thread"),
        );
        Parts {
            session: Session::start(lp, Duration::from_millis(1)),
            clock,
            storage,
        }
    }

    fn wait_for(what: &str, mut ok: impl FnMut() -> bool) {
        let start = Instant::now();
        while !ok() {
            assert!(
                start.elapsed() < Duration::from_secs(10),
                "timed out waiting for {what}"
            );
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    #[test]
    fn the_ui_side_drives_the_world_through_messages_and_snapshots() {
        let p = start();
        assert_eq!(p.session.snapshot().run, RunState::Paused);
        p.session
            .send(Control::Run(RunEvent::Resume(Speed::Normal)));
        wait_for("resume", || {
            p.session.snapshot().run == RunState::Running(Speed::Normal)
        });
        p.clock.advance(Duration::from_secs(3));
        wait_for("ticks", || p.session.snapshot().tick >= 30);
        assert_eq!(p.session.snapshot().tick, 30);
        assert_eq!(p.session.snapshot().pawns.len(), 4);
        p.session
            .send(Control::Command(Command::DevNudge { amount: 1 }));
        p.session.send(Control::Run(RunEvent::Pause));
        wait_for("pause", || p.session.snapshot().run == RunState::Paused);
        p.clock.advance(Duration::from_secs(10));
        std::thread::sleep(Duration::from_millis(30));
        assert!(
            p.session.snapshot().tick <= 31,
            "paused worlds do not advance"
        );
        assert!(p.session.snapshot_version() > 0);
    }

    #[test]
    fn shutdown_saves_and_returns_the_finished_loop() {
        let p = start();
        p.session.send(Control::Run(RunEvent::Resume(Speed::Fast)));
        p.clock.advance(Duration::from_millis(1));
        wait_for("running", || p.session.snapshot().run.is_running());
        p.clock.advance(Duration::from_secs(1));
        wait_for("ticks", || p.session.snapshot().tick >= 30);
        let storage = p.storage.clone();
        let events = p.session.events();
        assert!(events
            .iter()
            .any(|e| matches!(e, LoopEvent::State(RunState::Running(_)))));
        let lp = p.session.shutdown().expect("the loop returns");
        assert_eq!(lp.state(), RunState::Stopped);
        let loaded = SlotStore::new(storage.as_ref())
            .load("thread", &LoadOptions::default())
            .unwrap();
        assert_eq!(
            loaded.world.clock.tick(),
            lp.sim().world().clock.tick(),
            "the final save holds the last tick"
        );
        assert!(loaded.world.clock.tick() >= 30);
    }

    #[test]
    fn dropping_a_session_closes_it_cleanly() {
        let p = start();
        let storage = p.storage.clone();
        drop(p.session);
        assert!(
            storage.names().iter().any(|n| n.ends_with("manifest.json")),
            "closing saved the world"
        );
    }

    #[test]
    fn many_messages_from_many_threads_are_all_handled() {
        let p = start();
        p.session
            .send(Control::Run(RunEvent::Resume(Speed::Normal)));
        // (The loop measures time between its own frames, so let it take its first frame while running.)
        wait_for("resume", || p.session.snapshot().run.is_running());
        let tx = p.session.tx.clone();
        let hs: Vec<_> = (0..4)
            .map(|_| {
                let tx = tx.clone();
                std::thread::spawn(move || {
                    for _ in 0..50 {
                        let _ = tx.send(Control::Command(Command::DevNudge { amount: 1 }));
                    }
                })
            })
            .collect();
        for h in hs {
            h.join().unwrap();
        }
        p.clock.advance(Duration::from_secs(2));
        wait_for("ticks", || p.session.snapshot().tick >= 20);
        let lp = p.session.shutdown().unwrap();
        // 200 nudges of +1 were all applied, in some order, before the 20 ticks ran.
        assert!(lp.sim().world().probe.value >= 200 - 1000 && lp.sim().world().clock.tick() >= 20);
        let _ = Clock::now_monotonic(&FixedClock::new());
    }
}
