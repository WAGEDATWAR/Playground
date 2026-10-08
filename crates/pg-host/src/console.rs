//! The developer console's log (milestone 1.3a): severities, entries and a bounded, shared buffer.
//!
//! Everything the game wants to tell a developer goes through one [`Console`]: lines the application logs
//! (through [`ConsoleLog`], which wraps the real [`LogSink`]), what the simulation reports (the typed event
//! catalog gives each kind a severity), and what script packs print. The buffer keeps the newest
//! [`CAPACITY`] entries and every line is redacted on the way in, so no key reaches the console any more
//! than it reaches a log file.

use crate::redact::redact;
use crate::services::{Level, LogSink};
use std::collections::VecDeque;
use std::sync::{Arc, Mutex, MutexGuard};

/// Entries kept; the oldest are dropped first.
pub const CAPACITY: usize = 5_000;

/// Longest line kept; longer text is cut (on a character boundary) with an ellipsis.
pub const MAX_LINE: usize = 1_000;

/// How serious a console entry is.
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Severity {
    /// Routine detail, useful to filter out as noise (every task started, every step walked).
    Debug,
    /// What the game and script packs print on purpose.
    Info,
    /// Something went wrong that the game recovered from (a task failed, a command was refused).
    Warn,
    /// An operation failed (a script error, a save that could not be written).
    Error,
    /// The world cannot carry on (a tick panicked and the world was frozen).
    Fatal,
}

impl Severity {
    pub const ALL: [Severity; 5] = [
        Severity::Debug,
        Severity::Info,
        Severity::Warn,
        Severity::Error,
        Severity::Fatal,
    ];

    /// The name printed in the prefix: `[Warn]`.
    pub const fn name(self) -> &'static str {
        match self {
            Severity::Debug => "Debug",
            Severity::Info => "Info",
            Severity::Warn => "Warn",
            Severity::Error => "Error",
            Severity::Fatal => "Fatal",
        }
    }

    /// The lower-case id used in widget ids and command-line flags.
    pub const fn id(self) -> &'static str {
        match self {
            Severity::Debug => "debug",
            Severity::Info => "info",
            Severity::Warn => "warn",
            Severity::Error => "error",
            Severity::Fatal => "fatal",
        }
    }

    pub fn from_id(id: &str) -> Option<Severity> {
        Severity::ALL
            .into_iter()
            .find(|s| s.id() == id.to_ascii_lowercase())
    }

    pub const fn from_level(level: Level) -> Severity {
        match level {
            Level::Debug => Severity::Debug,
            Level::Info => Severity::Info,
            Level::Warn => Severity::Warn,
            Level::Error => Severity::Error,
        }
    }
}

/// One line of the console.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    /// Increases by one per entry for the life of the console; never reused.
    pub seq: u64,
    pub severity: Severity,
    /// Where it came from: `app`, `sim`, `pack:<id>`, ...
    pub source: String,
    /// The game tick it concerns, when it concerns one.
    pub tick: Option<u64>,
    pub text: String,
}

impl Entry {
    /// The line as shown: `[Error]: text`.
    pub fn line(&self) -> String {
        format!("[{}]: {}", self.severity.name(), self.text)
    }
}

fn clip(text: &str) -> String {
    if text.len() <= MAX_LINE {
        return text.to_owned();
    }
    let mut cut = MAX_LINE;
    while !text.is_char_boundary(cut) {
        cut -= 1;
    }
    format!("{}...", &text[..cut])
}

struct Inner {
    entries: VecDeque<Entry>,
    next_seq: u64,
}

/// The shared buffer. Cloning shares it.
#[derive(Clone)]
pub struct Console {
    inner: Arc<Mutex<Inner>>,
    /// Whether Debug entries are worth the cost of formatting (only while a developer is looking).
    wants_debug: Arc<std::sync::atomic::AtomicBool>,
}

impl Default for Console {
    fn default() -> Self {
        Console::new()
    }
}

impl Console {
    pub fn new() -> Console {
        Console {
            inner: Arc::new(Mutex::new(Inner {
                entries: VecDeque::new(),
                next_seq: 1,
            })),
            wants_debug: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        }
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// Debug entries are produced only while someone can see them (developer mode); the producers ask here
    /// before they spend time formatting one.
    pub fn set_wants_debug(&self, on: bool) {
        self.wants_debug
            .store(on, std::sync::atomic::Ordering::Relaxed);
    }

    pub fn wants_debug(&self) -> bool {
        self.wants_debug.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Adds an entry. The text is redacted and clipped. Returns its sequence number.
    pub fn push(&self, severity: Severity, source: &str, tick: Option<u64>, text: &str) -> u64 {
        let text = clip(&redact(text, &[]));
        let mut g = self.lock();
        let seq = g.next_seq;
        g.next_seq += 1;
        g.entries.push_back(Entry {
            seq,
            severity,
            source: source.to_owned(),
            tick,
            text,
        });
        while g.entries.len() > CAPACITY {
            g.entries.pop_front();
        }
        seq
    }

    pub fn debug(&self, source: &str, text: &str) {
        if self.wants_debug() {
            self.push(Severity::Debug, source, None, text);
        }
    }

    pub fn info(&self, source: &str, text: &str) {
        self.push(Severity::Info, source, None, text);
    }

    pub fn warn(&self, source: &str, text: &str) {
        self.push(Severity::Warn, source, None, text);
    }

    pub fn error(&self, source: &str, text: &str) {
        self.push(Severity::Error, source, None, text);
    }

    pub fn fatal(&self, source: &str, text: &str) {
        self.push(Severity::Fatal, source, None, text);
    }

    /// Entries with a sequence number above `seq` (0 for all kept), oldest first.
    pub fn since(&self, seq: u64) -> Vec<Entry> {
        self.lock()
            .entries
            .iter()
            .filter(|e| e.seq > seq)
            .cloned()
            .collect()
    }

    pub fn len(&self) -> usize {
        self.lock().entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.lock().entries.is_empty()
    }

    pub fn clear(&self) {
        self.lock().entries.clear();
    }
}

/// A [`LogSink`] that also writes every line to the console. Wrap the application's log with it and
/// everything the app and the simulation loop log shows up in the console at the matching severity.
pub struct ConsoleLog {
    inner: Arc<dyn LogSink>,
    console: Console,
}

impl ConsoleLog {
    pub fn new(inner: Arc<dyn LogSink>, console: Console) -> ConsoleLog {
        ConsoleLog { inner, console }
    }
}

impl LogSink for ConsoleLog {
    fn log(&self, level: Level, line: &str) {
        self.inner.log(level, line);
        let sev = Severity::from_level(level);
        if sev != Severity::Debug || self.console.wants_debug() {
            self.console.push(sev, "app", None, line);
        }
    }
}

/// Whether `line` passes a developer's filter box: case-insensitive, matched against the line as shown
/// (prefix included, so typing `error` finds the errors). An empty filter passes everything.
pub fn matches_filter(entry: &Entry, filter: &str) -> bool {
    let f = filter.trim();
    f.is_empty() || entry.line().to_lowercase().contains(&f.to_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::MemLog;

    #[test]
    fn severities_are_ordered_named_and_parsed() {
        assert!(Severity::Debug < Severity::Info && Severity::Error < Severity::Fatal);
        let names: Vec<&str> = Severity::ALL.iter().map(|s| s.name()).collect();
        assert_eq!(names, ["Debug", "Info", "Warn", "Error", "Fatal"]);
        for s in Severity::ALL {
            assert_eq!(Severity::from_id(s.id()), Some(s));
        }
        assert_eq!(Severity::from_id("WARN"), Some(Severity::Warn));
        assert_eq!(Severity::from_id("loud"), None);
        assert_eq!(Severity::from_level(Level::Error), Severity::Error);
    }

    #[test]
    fn entries_are_numbered_prefixed_and_read_back_incrementally() {
        let c = Console::new();
        c.info("app", "hello");
        c.warn("sim", "careful");
        c.error("pack:x", "broke");
        let all = c.since(0);
        assert_eq!(all.iter().map(|e| e.seq).collect::<Vec<_>>(), [1, 2, 3]);
        assert_eq!(all[0].line(), "[Info]: hello");
        assert_eq!(all[2].line(), "[Error]: broke");
        assert_eq!(c.since(2).len(), 1);
        assert_eq!(c.since(3).len(), 0);
        c.clear();
        assert!(c.is_empty());
        c.info("app", "again");
        assert_eq!(c.since(0)[0].seq, 4, "sequence numbers are never reused");
    }

    #[test]
    fn the_buffer_keeps_only_the_newest_and_clips_long_lines() {
        let c = Console::new();
        for i in 0..(CAPACITY + 10) {
            c.info("app", &format!("line {i}"));
        }
        assert_eq!(c.len(), CAPACITY);
        assert_eq!(c.since(0)[0].text, "line 10");
        c.info("app", &"é".repeat(MAX_LINE));
        let last = c.since(0).pop().unwrap();
        assert!(last.text.ends_with("...") && last.text.len() <= MAX_LINE + 3);
    }

    #[test]
    fn debug_is_only_produced_when_asked_for_and_text_is_redacted() {
        let c = Console::new();
        c.debug("sim", "noise");
        assert!(c.is_empty(), "nobody is looking");
        c.set_wants_debug(true);
        c.debug("sim", "noise");
        assert_eq!(c.len(), 1);
        // Redaction runs on every entry, like every other log path.
        c.error(
            "app",
            "the key was sk-ant-api03-abcdefghijklmnopqrstuvwxyz0123456789 ok",
        );
        let line = c.since(0).pop().unwrap().line();
        assert!(!line.contains("abcdefghijklmnop"), "{line}");
    }

    #[test]
    fn the_console_log_forwards_and_mirrors() {
        let mem = Arc::new(MemLog::new());
        let c = Console::new();
        let log = ConsoleLog::new(mem.clone(), c.clone());
        log.log(Level::Warn, "disk is slow");
        log.log(Level::Debug, "quiet");
        assert_eq!(mem.lines().len(), 2, "the real sink still gets everything");
        let got = c.since(0);
        assert_eq!(got.len(), 1, "debug stays out unless wanted");
        assert_eq!(
            (got[0].severity, got[0].source.as_str()),
            (Severity::Warn, "app")
        );
        c.set_wants_debug(true);
        log.log(Level::Debug, "loud now");
        assert_eq!(c.len(), 2);
    }

    #[test]
    fn the_filter_matches_the_displayed_line_ignoring_case() {
        let e = Entry {
            seq: 1,
            severity: Severity::Error,
            source: "app".into(),
            tick: None,
            text: "Save failed".into(),
        };
        assert!(matches_filter(&e, ""));
        assert!(matches_filter(&e, "  "));
        assert!(matches_filter(&e, "save"));
        assert!(matches_filter(&e, "[error]"));
        assert!(matches_filter(&e, "ERROR"));
        assert!(!matches_filter(&e, "load"));
    }
}
