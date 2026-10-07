//! Real implementations of the `pg-host` traits for desktop operating systems. Blueprint §3.
//!
//! 0.6 provides [`FsStorage`]; the credential store, HTTPS, dialogs and audio follow in 0.7.

use pg_host::{check_name, BlobInfo, Storage};
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// [`Storage`] over a directory. Writes go to a temp file in the destination directory, are flushed to
/// disk, then renamed into place; on Unix the directory is synced too, so a crash leaves the old blob or
/// the new one.
pub struct FsStorage {
    root: PathBuf,
}

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

impl FsStorage {
    /// Uses `root` as the storage directory, creating it if needed.
    pub fn new(root: impl Into<PathBuf>) -> io::Result<FsStorage> {
        let root = root.into();
        fs::create_dir_all(&root)?;
        Ok(FsStorage { root })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    fn path(&self, name: &str) -> io::Result<PathBuf> {
        check_name(name)?;
        let mut p = self.root.clone();
        p.extend(name.split('/'));
        Ok(p)
    }
}

#[cfg(unix)]
fn sync_dir(dir: &Path) -> io::Result<()> {
    fs::File::open(dir)?.sync_all()
}

#[cfg(not(unix))]
fn sync_dir(_dir: &Path) -> io::Result<()> {
    Ok(()) // Windows cannot open a directory for syncing; the rename itself is journaled
}

impl Storage for FsStorage {
    fn read(&self, name: &str) -> io::Result<Option<Vec<u8>>> {
        match fs::read(self.path(name)?) {
            Ok(b) => Ok(Some(b)),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e),
        }
    }

    fn write_atomic(&self, name: &str, data: &[u8]) -> io::Result<()> {
        let dest = self.path(name)?;
        let dir = dest
            .parent()
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "no parent directory"))?;
        fs::create_dir_all(dir)?;
        let n = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
        let file_name = dest
            .file_name()
            .and_then(|f| f.to_str())
            .unwrap_or("blob");
        let temp = dir.join(format!(".{file_name}.{}.{n}.tmp", std::process::id()));
        let result = (|| {
            let mut f = fs::File::create(&temp)?;
            f.write_all(data)?;
            f.sync_all()?;
            drop(f);
            fs::rename(&temp, &dest)?;
            sync_dir(dir)
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temp);
        }
        result
    }

    fn delete(&self, name: &str) -> io::Result<()> {
        match fs::remove_file(self.path(name)?) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e),
        }
    }

    fn list(&self, prefix: &str) -> io::Result<Vec<BlobInfo>> {
        let mut out = Vec::new();
        let mut stack = vec![(self.root.clone(), String::new())];
        while let Some((dir, rel)) = stack.pop() {
            let entries = match fs::read_dir(&dir) {
                Ok(e) => e,
                Err(e) if e.kind() == io::ErrorKind::NotFound => continue,
                Err(e) => return Err(e),
            };
            for entry in entries {
                let entry = entry?;
                let file_name = entry.file_name().to_string_lossy().into_owned();
                if file_name.starts_with('.') && file_name.ends_with(".tmp") {
                    continue; // an interrupted write's leftover is not a blob
                }
                let name = if rel.is_empty() {
                    file_name.clone()
                } else {
                    format!("{rel}/{file_name}")
                };
                let meta = entry.metadata()?;
                if meta.is_dir() {
                    stack.push((entry.path(), name));
                } else if name.starts_with(prefix) {
                    let modified = meta
                        .modified()
                        .ok()
                        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                        .map_or(0, |d| d.as_secs());
                    out.push(BlobInfo {
                        name,
                        size: meta.len(),
                        modified,
                    });
                }
            }
        }
        out.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(out)
    }

    fn free_space(&self) -> Option<u64> {
        None // a portable query needs a platform crate; added with the 0.7 OS services if wanted
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_root(tag: &str) -> PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!("pg-fs-test-{tag}-{}-{}", std::process::id(), TEMP_COUNTER.fetch_add(1, Ordering::Relaxed)));
        p
    }

    #[test]
    fn blobs_round_trip_through_real_files() {
        let root = temp_root("rt");
        let s = FsStorage::new(&root).unwrap();
        assert_eq!(s.read("w/a.bin").unwrap(), None);
        s.write_atomic("w/a.bin", b"hello").unwrap();
        s.write_atomic("w/b.bin", b"x").unwrap();
        s.write_atomic("top", b"t").unwrap();
        s.write_atomic("w/a.bin", b"replaced").unwrap();
        assert_eq!(s.read("w/a.bin").unwrap().unwrap(), b"replaced");
        let names: Vec<_> = s.list("w/").unwrap().into_iter().map(|b| b.name).collect();
        assert_eq!(names, ["w/a.bin", "w/b.bin"]);
        s.delete("w/b.bin").unwrap();
        s.delete("w/b.bin").unwrap();
        assert_eq!(s.list("").unwrap().len(), 2);
        // No temp files are left behind by successful writes.
        let leftovers = fs::read_dir(root.join("w")).unwrap().count();
        assert_eq!(leftovers, 1);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn bad_names_never_touch_the_disk() {
        let root = temp_root("bad");
        let s = FsStorage::new(&root).unwrap();
        for bad in ["../escape", "/abs", "a/../../b", "C:/x", "a\\b"] {
            assert!(s.write_atomic(bad, b"x").is_err(), "{bad}");
            assert!(s.read(bad).is_err(), "{bad}");
        }
        assert!(!root.parent().unwrap().join("escape").exists());
        let _ = fs::remove_dir_all(&root);
    }
}
