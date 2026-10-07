//! Host-service traits (Storage, SecretStore, Net, Clock, Dialogs, Audio) and in-memory test doubles. Blueprint §3.
//!
//! `Storage` arrived in 0.6; `SecretStore`, `Net`, `Clock`, `Dialogs`, `Audio` and the redaction utility in 0.7. Every trait has an in-memory
//! double so persistence, the AI client and the simulation run headless in tests and in the CLI.

pub mod redact;
pub mod services;

pub use redact::{redact, redact_plain, Secret, REDACTED};
pub use services::{
    https_host, iso_utc, AllowListNet, Audio, Bus, CancelToken, Clock, Dialogs, FixedClock,
    HttpRequest, HttpResponse, Level, LogSink, MemLog, MemSecretStore, Method, Net, NetError,
    NullAudio, RedactingLog, ScriptedDialogs, ScriptedNet, SecretError, SecretStore, StderrLog,
};

use std::collections::BTreeMap;
use std::io;
use std::sync::Mutex;

/// A stored blob's name, size and last-modified time (seconds since the Unix epoch; 0 when unknown).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BlobInfo {
    pub name: String,
    pub size: u64,
    pub modified: u64,
}

/// Atomic named blobs under the user-data directory (Blueprint §3).
///
/// Names are relative, `/`-separated and may not contain `..`, a leading `/`, `\`, `:` or empty
/// segments ([`check_name`]); implementations refuse anything else with `InvalidInput`.
pub trait Storage: Send + Sync {
    /// The blob's bytes, or `None` if it does not exist.
    fn read(&self, name: &str) -> io::Result<Option<Vec<u8>>>;
    /// Replaces the blob in one step: readers see the old bytes or the new bytes, never a mixture. The real
    /// implementation writes a temp file in the same directory, `fsync`s it, then renames it into place.
    fn write_atomic(&self, name: &str, data: &[u8]) -> io::Result<()>;
    /// Deletes the blob; deleting a missing blob is not an error.
    fn delete(&self, name: &str) -> io::Result<()>;
    /// Blobs whose names start with `prefix`, in name order.
    fn list(&self, prefix: &str) -> io::Result<Vec<BlobInfo>>;
    /// Free bytes on the storage volume, if known.
    fn free_space(&self) -> Option<u64>;
}

/// Validates a blob name.
pub fn check_name(name: &str) -> io::Result<()> {
    let bad = |why: &str| {
        Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("bad storage name '{name}': {why}"),
        ))
    };
    if name.is_empty() {
        return bad("empty");
    }
    if name.len() > 240 {
        return bad("too long");
    }
    if name.contains(['\\', ':', '\0']) || name.chars().any(char::is_control) {
        return bad("contains a forbidden character");
    }
    for seg in name.split('/') {
        if seg.is_empty() || seg == "." || seg == ".." {
            return bad("empty, '.' or '..' segment");
        }
    }
    Ok(())
}

/// What the in-memory store should do wrong, for fault-injection tests.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct Faults {
    /// Every operation numbered `n` or higher fails with an I/O error and changes nothing (a crash or a
    /// vanished disk: nothing after that point happens).
    pub fail_from_op: Option<u64>,
    /// The write numbered `n` (counting writes only) stores half of its bytes and then reports an error: a
    /// torn write on a file system that is not actually atomic.
    pub torn_write: Option<u64>,
    /// Pretend the disk is full: every write fails.
    pub disk_full: bool,
}

#[derive(Default)]
struct MemInner {
    blobs: BTreeMap<String, Vec<u8>>,
    ops: u64,
    writes: u64,
    faults: Faults,
}

/// An in-memory [`Storage`] with operation counting and fault injection.
#[derive(Default)]
pub struct MemStorage {
    inner: Mutex<MemInner>,
}

impl MemStorage {
    pub fn new() -> MemStorage {
        MemStorage::default()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, MemInner> {
        // A panic while holding the lock cannot leave the map half-updated, so poison is safe to ignore.
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Operations performed so far (reads, writes, deletes and lists each count one).
    pub fn ops(&self) -> u64 {
        self.lock().ops
    }

    pub fn set_faults(&self, faults: Faults) {
        self.lock().faults = faults;
    }

    /// Resets the operation and write counters (not the contents), so a test can count one save.
    pub fn reset_counters(&self) {
        let mut g = self.lock();
        g.ops = 0;
        g.writes = 0;
    }

    /// Direct access for tests that corrupt a blob on purpose.
    pub fn put_raw(&self, name: &str, data: Vec<u8>) {
        self.lock().blobs.insert(name.to_owned(), data);
    }

    pub fn get_raw(&self, name: &str) -> Option<Vec<u8>> {
        self.lock().blobs.get(name).cloned()
    }

    pub fn names(&self) -> Vec<String> {
        self.lock().blobs.keys().cloned().collect()
    }

    /// A copy of the whole store (used to retry the same save under different faults).
    pub fn duplicate(&self) -> MemStorage {
        let g = self.lock();
        MemStorage {
            inner: Mutex::new(MemInner {
                blobs: g.blobs.clone(),
                ..MemInner::default()
            }),
        }
    }
}

fn crash() -> io::Error {
    io::Error::other("injected fault: the operation did not happen")
}

impl MemInner {
    /// Counts an operation; fails if the fault plan says this one (or any later one) must not happen.
    fn begin(&mut self) -> io::Result<()> {
        let n = self.ops;
        self.ops += 1;
        match self.faults.fail_from_op {
            Some(from) if n >= from => Err(crash()),
            _ => Ok(()),
        }
    }
}

impl Storage for MemStorage {
    fn read(&self, name: &str) -> io::Result<Option<Vec<u8>>> {
        check_name(name)?;
        let mut g = self.lock();
        g.begin()?;
        Ok(g.blobs.get(name).cloned())
    }

    fn write_atomic(&self, name: &str, data: &[u8]) -> io::Result<()> {
        check_name(name)?;
        let mut g = self.lock();
        g.begin()?;
        let w = g.writes;
        g.writes += 1;
        if g.faults.disk_full {
            return Err(io::Error::other("injected fault: disk full"));
        }
        if g.faults.torn_write == Some(w) {
            let half = data.len() / 2;
            g.blobs.insert(name.to_owned(), data[..half].to_vec());
            return Err(io::Error::other("injected fault: torn write"));
        }
        g.blobs.insert(name.to_owned(), data.to_vec());
        Ok(())
    }

    fn delete(&self, name: &str) -> io::Result<()> {
        check_name(name)?;
        let mut g = self.lock();
        g.begin()?;
        g.blobs.remove(name);
        Ok(())
    }

    fn list(&self, prefix: &str) -> io::Result<Vec<BlobInfo>> {
        let mut g = self.lock();
        g.begin()?;
        Ok(g.blobs
            .iter()
            .filter(|(k, _)| k.starts_with(prefix))
            .map(|(k, v)| BlobInfo {
                name: k.clone(),
                size: v.len() as u64,
                modified: 0,
            })
            .collect())
    }

    fn free_space(&self) -> Option<u64> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_validated() {
        for ok in ["a", "worlds/w1/manifest.json", "a/b/c.pgsave"] {
            assert!(check_name(ok).is_ok(), "{ok}");
        }
        for bad in [
            "", "/abs", "a//b", "a/../b", "..", "./a", "a/", "a\\b", "C:/x", "a\0b", "a\nb",
        ] {
            assert!(check_name(bad).is_err(), "{bad:?}");
        }
        assert!(check_name(&"x".repeat(241)).is_err());
    }

    #[test]
    fn read_write_list_delete() {
        let s = MemStorage::new();
        assert_eq!(s.read("a/b").unwrap(), None);
        s.write_atomic("a/b", b"one").unwrap();
        s.write_atomic("a/c", b"three").unwrap();
        s.write_atomic("z", b"z").unwrap();
        assert_eq!(s.read("a/b").unwrap().unwrap(), b"one");
        s.write_atomic("a/b", b"uno").unwrap();
        assert_eq!(s.read("a/b").unwrap().unwrap(), b"uno");
        let names: Vec<_> = s
            .list("a/")
            .unwrap()
            .into_iter()
            .map(|b| (b.name, b.size))
            .collect();
        assert_eq!(names, [("a/b".to_owned(), 3), ("a/c".to_owned(), 5)]);
        s.delete("a/b").unwrap();
        s.delete("a/b").unwrap(); // deleting twice is fine
        assert_eq!(s.read("a/b").unwrap(), None);
        assert!(s.write_atomic("../x", b"").is_err());
    }

    #[test]
    fn a_crash_stops_everything_from_that_operation_on() {
        let s = MemStorage::new();
        s.set_faults(Faults {
            fail_from_op: Some(2),
            ..Faults::default()
        });
        s.write_atomic("a", b"1").unwrap(); // op 0
        s.write_atomic("b", b"2").unwrap(); // op 1
        assert!(s.write_atomic("c", b"3").is_err()); // op 2
        assert!(s.read("a").is_err()); // op 3: still crashed
        s.set_faults(Faults::default());
        assert_eq!(
            s.names(),
            ["a", "b"],
            "the failed write left nothing behind"
        );
    }

    #[test]
    fn a_torn_write_leaves_half_a_blob_and_an_error() {
        let s = MemStorage::new();
        s.set_faults(Faults {
            torn_write: Some(1),
            ..Faults::default()
        });
        s.write_atomic("a", b"abcdef").unwrap();
        assert!(s.write_atomic("b", b"abcdef").is_err());
        assert_eq!(s.get_raw("b").unwrap(), b"abc");
        s.write_atomic("c", b"abcdef").unwrap();
    }

    #[test]
    fn disk_full_fails_writes_but_not_reads() {
        let s = MemStorage::new();
        s.write_atomic("a", b"1").unwrap();
        s.set_faults(Faults {
            disk_full: true,
            ..Faults::default()
        });
        assert!(s.write_atomic("a", b"2").is_err());
        assert_eq!(s.read("a").unwrap().unwrap(), b"1");
    }

    #[test]
    fn counters_and_duplicates() {
        let s = MemStorage::new();
        s.write_atomic("a", b"1").unwrap();
        s.read("a").unwrap();
        assert_eq!(s.ops(), 2);
        let d = s.duplicate();
        d.write_atomic("b", b"2").unwrap();
        assert_eq!(s.names(), ["a"]);
        assert_eq!(d.names(), ["a", "b"]);
        s.reset_counters();
        assert_eq!(s.ops(), 0);
    }
}
