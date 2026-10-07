//! The last-resort tick guard (Blueprint §17, §24, suggestion S-028).
//!
//! A bug in a system must not take the whole application down with the player's unsaved session. The sim
//! thread runs each tick through [`guarded`]: a panic is caught, its message, location and backtrace are
//! captured by a process-wide hook, and the caller freezes the world, writes a redacted crash report and a
//! replayable bundle, and keeps the UI alive. Panics outside a guard still reach the previous hook (the
//! default one prints them), so other threads and tests behave as usual.

use std::cell::Cell;
use std::panic::{catch_unwind, AssertUnwindSafe, PanicHookInfo};
use std::sync::{Mutex, Once};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PanicReport {
    pub message: String,
    pub location: Option<String>,
    pub backtrace: String,
}

type Hook = Box<dyn Fn(&PanicHookInfo<'_>) + Sync + Send + 'static>;

static INSTALL: Once = Once::new();
static LAST: Mutex<Option<PanicReport>> = Mutex::new(None);
static PREVIOUS: Mutex<Option<Hook>> = Mutex::new(None);

thread_local! {
    static GUARDED: Cell<u32> = const { Cell::new(0) };
}

fn payload_text(info: &PanicHookInfo<'_>) -> String {
    let p = info.payload();
    p.downcast_ref::<&str>()
        .map(|s| (*s).to_owned())
        .or_else(|| p.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "(non-text panic)".to_owned())
}

/// Installs the capturing hook (once per process; calling again is harmless).
pub fn install_hook() {
    INSTALL.call_once(|| {
        let previous = std::panic::take_hook();
        *PREVIOUS.lock().unwrap_or_else(|e| e.into_inner()) = Some(previous);
        std::panic::set_hook(Box::new(|info| {
            if GUARDED.with(Cell::get) > 0 {
                let report = PanicReport {
                    message: payload_text(info),
                    location: info
                        .location()
                        .map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column())),
                    backtrace: std::backtrace::Backtrace::force_capture().to_string(),
                };
                *LAST.lock().unwrap_or_else(|e| e.into_inner()) = Some(report);
            } else if let Some(prev) = PREVIOUS.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
                prev(info);
            }
        }));
    });
}

/// Runs `f`, turning a panic into a [`PanicReport`].
pub fn guarded<R>(f: impl FnOnce() -> R) -> Result<R, PanicReport> {
    install_hook();
    GUARDED.with(|g| g.set(g.get() + 1));
    let result = catch_unwind(AssertUnwindSafe(f));
    GUARDED.with(|g| g.set(g.get().saturating_sub(1)));
    result.map_err(|payload| {
        LAST.lock()
            .unwrap_or_else(|e| e.into_inner())
            .take()
            .unwrap_or_else(|| PanicReport {
                message: crate::pool::panic_text(&*payload),
                location: None,
                backtrace: String::new(),
            })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_normal_call_returns_its_value() {
        assert_eq!(guarded(|| 41 + 1).unwrap(), 42);
    }

    #[test]
    fn a_panic_becomes_a_report_with_message_location_and_backtrace() {
        let r = guarded(|| -> u32 { panic!("tick exploded at {}", 99) }).unwrap_err();
        assert_eq!(r.message, "tick exploded at 99");
        assert!(
            r.location
                .as_deref()
                .is_some_and(|l| l.contains("guard.rs")),
            "{:?}",
            r.location
        );
        assert!(!r.backtrace.is_empty());
        let s = guarded(|| -> u32 { panic!("static") }).unwrap_err();
        assert_eq!(s.message, "static");
    }

    #[test]
    fn guards_nest_and_recover_for_later_calls() {
        let outer = guarded(|| {
            let inner = guarded(|| -> u32 { panic!("inner") });
            assert!(inner.is_err());
            7
        });
        assert_eq!(outer.unwrap(), 7);
        assert_eq!(guarded(|| 1).unwrap(), 1);
        assert!(guarded(|| -> u32 { panic!("again") }).is_err());
    }

    #[test]
    fn guarded_panics_on_many_threads_do_not_get_mixed_up() {
        let hs: Vec<_> = (0..8)
            .map(|i| {
                std::thread::spawn(move || {
                    for _ in 0..20 {
                        let r = guarded(|| -> u32 { panic!("thread {i}") }).unwrap_err();
                        assert_eq!(r.message, format!("thread {i}"));
                    }
                })
            })
            .collect();
        for h in hs {
            h.join().unwrap();
        }
    }
}
