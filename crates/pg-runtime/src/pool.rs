//! A long-lived worker pool (Blueprint §6.5).
//!
//! The pool runs **owned** jobs (`'static`): encoding a save, re-simulating a span for shadow
//! verification, and later conversation, AI and worldgen work. Results come back through [`JobHandle`]s,
//! and [`WorkerPool::map_ordered`] applies a function to a list and returns the results **in input order**,
//! so using the pool can never reorder anything the simulation sees. A job that panics yields an error to
//! its caller instead of killing a worker.
//!
//! Path batches borrow map data and so keep using the scoped fan-out in [`crate::exec`]: a persistent
//! thread cannot hold a borrow without `unsafe`, which the project does not use.

use std::any::Any;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

type Job = Box<dyn FnOnce() + Send + 'static>;

/// A job panicked; the message is the panic payload when it was text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JobPanic(pub String);

impl std::fmt::Display for JobPanic {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "a worker job panicked: {}", self.0)
    }
}

impl std::error::Error for JobPanic {}

/// What a submitted job will produce.
pub struct JobHandle<R> {
    rx: Receiver<Result<R, JobPanic>>,
}

impl<R> JobHandle<R> {
    /// Waits for the result.
    pub fn join(self) -> Result<R, JobPanic> {
        self.rx
            .recv()
            .unwrap_or_else(|_| Err(JobPanic("the pool shut down before the job ran".to_owned())))
    }

    /// The result if it is ready, without waiting.
    pub fn try_join(&self) -> Option<Result<R, JobPanic>> {
        self.rx.try_recv().ok()
    }
}

pub(crate) fn panic_text(p: &(dyn Any + Send)) -> String {
    p.downcast_ref::<&str>()
        .map(|s| (*s).to_owned())
        .or_else(|| p.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "(non-text panic)".to_owned())
}

pub struct WorkerPool {
    tx: Option<Sender<Job>>,
    workers: Vec<JoinHandle<()>>,
}

impl WorkerPool {
    /// A pool with `threads` workers (at least one).
    pub fn new(threads: usize) -> WorkerPool {
        let (tx, rx) = channel::<Job>();
        let rx = Arc::new(Mutex::new(rx));
        let workers = (0..threads.max(1))
            .map(|i| {
                let rx = Arc::clone(&rx);
                std::thread::Builder::new()
                    .name(format!("pg-worker-{i}"))
                    .spawn(move || loop {
                        // Hold the lock only while taking a job, not while running it.
                        let job = rx.lock().unwrap_or_else(|e| e.into_inner()).recv();
                        match job {
                            Ok(job) => job(),
                            Err(_) => break, // the pool was dropped
                        }
                    })
                    .expect("spawning a worker thread")
            })
            .collect();
        WorkerPool {
            tx: Some(tx),
            workers,
        }
    }

    pub fn threads(&self) -> usize {
        self.workers.len()
    }

    /// Queues `f`; the handle yields its result (or its panic).
    pub fn submit<R: Send + 'static>(
        &self,
        f: impl FnOnce() -> R + Send + 'static,
    ) -> JobHandle<R> {
        let (rtx, rrx) = channel();
        let job: Job = Box::new(move || {
            let r = catch_unwind(AssertUnwindSafe(f)).map_err(|p| JobPanic(panic_text(&*p)));
            let _ = rtx.send(r);
        });
        if let Some(tx) = &self.tx {
            // If the pool is shutting down the job is dropped and the handle reports it.
            let _ = tx.send(job);
        }
        JobHandle { rx: rrx }
    }

    /// Applies `f` to every item on the pool and returns the results in the order of `items`.
    pub fn map_ordered<I, R>(
        &self,
        items: Vec<I>,
        f: impl Fn(I) -> R + Send + Sync + 'static,
    ) -> Vec<Result<R, JobPanic>>
    where
        I: Send + 'static,
        R: Send + 'static,
    {
        let f = Arc::new(f);
        let handles: Vec<JobHandle<R>> = items
            .into_iter()
            .map(|item| {
                let f = Arc::clone(&f);
                self.submit(move || f(item))
            })
            .collect();
        handles.into_iter().map(JobHandle::join).collect()
    }
}

impl Drop for WorkerPool {
    fn drop(&mut self) {
        self.tx.take(); // closes the channel; workers finish their current job and exit
        for w in self.workers.drain(..) {
            let _ = w.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    #[test]
    fn jobs_run_and_return_their_results() {
        let pool = WorkerPool::new(3);
        assert_eq!(pool.threads(), 3);
        let hs: Vec<_> = (0..20u64).map(|n| pool.submit(move || n * n)).collect();
        let got: Vec<u64> = hs.into_iter().map(|h| h.join().unwrap()).collect();
        assert_eq!(got, (0..20u64).map(|n| n * n).collect::<Vec<_>>());
    }

    #[test]
    fn map_ordered_keeps_input_order_whatever_the_finish_order() {
        let pool = WorkerPool::new(4);
        // Earlier items sleep longer, so they finish last.
        let out = pool.map_ordered((0..8u64).collect(), |n| {
            std::thread::sleep(Duration::from_millis((8 - n) * 3));
            n * 10
        });
        assert_eq!(
            out.into_iter().map(Result::unwrap).collect::<Vec<_>>(),
            [0, 10, 20, 30, 40, 50, 60, 70]
        );
    }

    #[test]
    fn results_do_not_depend_on_the_worker_count() {
        let work = |n: u64| (0..1000).fold(n, |a, b| a.wrapping_mul(31).wrapping_add(b));
        let one: Vec<u64> = WorkerPool::new(1)
            .map_ordered((0..50).collect(), work)
            .into_iter()
            .map(Result::unwrap)
            .collect();
        let many: Vec<u64> = WorkerPool::new(7)
            .map_ordered((0..50).collect(), work)
            .into_iter()
            .map(Result::unwrap)
            .collect();
        assert_eq!(one, many);
    }

    #[test]
    fn a_panicking_job_is_reported_and_the_pool_keeps_working() {
        let pool = WorkerPool::new(2);
        let bad = pool.submit(|| -> u32 { panic!("boom {}", 7) });
        assert_eq!(bad.join(), Err(JobPanic("boom 7".to_owned())));
        let bad2 = pool.submit(|| -> u32 { panic!("static text") });
        assert_eq!(bad2.join(), Err(JobPanic("static text".to_owned())));
        for _ in 0..10 {
            assert_eq!(pool.submit(|| 5).join(), Ok(5));
        }
    }

    #[test]
    fn dropping_the_pool_finishes_queued_work_and_joins_the_threads() {
        let counter = Arc::new(AtomicUsize::new(0));
        {
            let pool = WorkerPool::new(2);
            for _ in 0..30 {
                let c = Arc::clone(&counter);
                let _ = pool.submit(move || {
                    c.fetch_add(1, Ordering::SeqCst);
                });
            }
        }
        assert_eq!(counter.load(Ordering::SeqCst), 30);
    }

    #[test]
    fn try_join_does_not_block() {
        let pool = WorkerPool::new(1);
        let h = pool.submit(|| {
            std::thread::sleep(Duration::from_millis(50));
            1
        });
        assert!(h.try_join().is_none());
        assert_eq!(h.join(), Ok(1));
    }
}
