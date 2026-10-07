//! A threaded [`BatchExecutor`] (Blueprint §6.5, §7.3).
//!
//! The core only knows the executor trait and ships a serial implementation; this crate supplies the
//! multi-threaded one. Jobs are split into contiguous chunks, each chunk runs on a scoped thread, and the
//! results are joined **in job order**. Since every job is a pure function of its inputs, the output is
//! identical to the serial executor's for any thread count; only the time changes.
//!
//! Milestone 0.8 replaces the per-batch scoped threads with the long-lived worker pool.

use pg_core::path::{BatchExecutor, PathJob, PathOutcome};

#[derive(Copy, Clone, Debug)]
pub struct ScopedThreads {
    threads: usize,
}

impl ScopedThreads {
    pub fn new(threads: usize) -> ScopedThreads {
        ScopedThreads {
            threads: threads.max(1),
        }
    }

    /// One thread per core, leaving two for the simulation and UI threads (at least one).
    pub fn auto() -> ScopedThreads {
        let cores = std::thread::available_parallelism().map_or(1, std::num::NonZeroUsize::get);
        ScopedThreads::new(cores.saturating_sub(2).max(1))
    }
}

impl BatchExecutor for ScopedThreads {
    fn solve(&self, jobs: &[PathJob<'_>]) -> Vec<PathOutcome> {
        // Spawning threads costs more than a handful of searches: small batches run inline.
        if self.threads == 1 || jobs.len() < 4 {
            return jobs.iter().map(PathJob::solve).collect();
        }
        let chunk = jobs.len().div_ceil(self.threads).max(1);
        std::thread::scope(|scope| {
            let handles: Vec<_> = jobs
                .chunks(chunk)
                .map(|part| {
                    scope.spawn(move || part.iter().map(PathJob::solve).collect::<Vec<_>>())
                })
                .collect();
            let mut out = Vec::with_capacity(jobs.len());
            for (handle, part) in handles.into_iter().zip(jobs.chunks(chunk)) {
                match handle.join() {
                    Ok(results) => out.extend(results),
                    // A worker panicked (a bug, not bad data): redo its chunk here so the batch still
                    // returns one result per job, in order.
                    Err(_) => out.extend(part.iter().map(PathJob::solve)),
                }
            }
            out
        })
    }

    fn threads(&self) -> usize {
        self.threads
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pg_core::id::{EntityId, Kind};
    use pg_core::map::{MapData, MapKind, MoveCosts, Tile, WATER};
    use pg_core::path::SerialExecutor;

    fn map() -> MapData {
        let mut m = MapData::new(EntityId::new(Kind::Map, 1), MapKind::Overworld, 40, 40).unwrap();
        for i in 0..32 {
            m.set_blocked(Tile::new(10, i), true).unwrap();
            m.set_blocked(Tile::new(20, 39 - i), true).unwrap();
            m.set_terrain(Tile::new(30, i), WATER).unwrap();
        }
        m
    }

    fn jobs<'a>(m: &'a MapData, c: &'a MoveCosts, n: i32) -> Vec<PathJob<'a>> {
        (0..n)
            .map(|i| PathJob {
                map: m,
                costs: c,
                from: Tile::new(i % 9, (i * 5) % 40),
                to: Tile::new(39 - (i * 3) % 9, (i * 7) % 40),
                cap: 20_000,
            })
            .collect()
    }

    #[test]
    fn every_thread_count_matches_the_serial_executor() {
        let (m, c) = (map(), MoveCosts::default());
        for n in [0, 1, 3, 4, 5, 17, 64] {
            let js = jobs(&m, &c, n);
            let serial = SerialExecutor.solve(&js);
            for t in [1, 2, 3, 4, 8, 16, 100] {
                assert_eq!(
                    ScopedThreads::new(t).solve(&js),
                    serial,
                    "{n} jobs on {t} threads"
                );
            }
        }
    }

    #[test]
    fn results_come_back_in_job_order() {
        let (m, c) = (map(), MoveCosts::default());
        let js = jobs(&m, &c, 24);
        let out = ScopedThreads::new(6).solve(&js);
        for (job, result) in js.iter().zip(&out) {
            assert_eq!(
                *result,
                job.solve(),
                "the result at each index belongs to the job at that index"
            );
        }
    }

    #[test]
    fn thread_counts_are_clamped_and_reported() {
        assert_eq!(ScopedThreads::new(0).threads(), 1);
        assert_eq!(ScopedThreads::new(6).threads(), 6);
        assert!(ScopedThreads::auto().threads() >= 1);
    }
}
