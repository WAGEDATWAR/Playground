//! Feeding the developer console from the simulation (milestone 1.3a).

use pg_core::events::EventCatalog;
use pg_core::pipeline::Event;
use pg_host::{Console, Severity};

/// Writes `events` to the console at their catalog severities, as source `sim`. Debug-level events are
/// skipped (and not even formatted) unless the console wants them.
pub fn push_events(console: &Console, events: &[Event]) {
    let catalog = EventCatalog::shared();
    for e in events {
        if catalog.severity_of(&e.kind) == Severity::Debug && !console.wants_debug() {
            continue;
        }
        let (sev, text) = catalog.console_line(e);
        console.push(sev, "sim", Some(e.tick), &text);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pg_core::canon::Canon;

    fn ev(kind: &str) -> Event {
        Event {
            tick: 5,
            kind: kind.into(),
            detail: Canon::map::<String>([]),
        }
    }

    #[test]
    fn events_arrive_at_their_severity_and_debug_waits_for_a_developer() {
        let c = Console::new();
        push_events(&c, &[ev("task.started"), ev("input_rejected")]);
        let got = c.since(0);
        assert_eq!(got.len(), 1);
        assert_eq!((got[0].severity, got[0].tick), (Severity::Warn, Some(5)));
        c.set_wants_debug(true);
        push_events(&c, &[ev("task.started")]);
        assert_eq!(c.since(0).pop().unwrap().line(), "[Debug]: task.started");
    }
}
