//! Loading a content pack from files (Blueprint §13.6, §21, §23.3).
//!
//! A pack is a directory (or, later, a `.pgpack` archive) holding `pack.json` and data files. The
//! loader reads it through the [`PackFiles`] trait so tests can use in-memory packs. It enforces:
//!
//! * safe relative paths only (no `..`, absolute paths, backslashes, drive letters);
//! * limits on file count, per-file size and total size;
//! * strict, integer-only JSON for every data file;
//! * a content hash over every file (data, scripts, assets), recorded in a world's `content_refs`.
//!
//! Templates are read from `data/templates/**/*.json`; each file holds one template object or a list
//! of them. Scripts are hashed but not interpreted until milestone 0.9.

use crate::manifest::{is_safe_relative_path, PackManifest};
use crate::report::ValidationReport;
use crate::template::ObjectTemplate;
use pg_canon::json;
use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};

pub const MANIFEST_FILE: &str = "pack.json";
pub const TEMPLATE_DIR: &str = "data/templates/";

/// Resource limits for one pack.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct Limits {
    pub max_files: usize,
    pub max_file_bytes: u64,
    pub max_total_bytes: u64,
}

impl Default for Limits {
    fn default() -> Limits {
        Limits {
            max_files: 2_000,
            max_file_bytes: 1 << 20,
            max_total_bytes: 32 << 20,
        }
    }
}

/// A file-source failure.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PackIoError(pub String);

impl fmt::Display for PackIoError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for PackIoError {}

/// Read access to a pack's files. Paths are `/`-separated and relative to the pack root.
pub trait PackFiles {
    /// Every file path in the pack, sorted. Fails if the pack has more than `max_files` files.
    fn paths(&self, max_files: usize) -> Result<Vec<String>, PackIoError>;
    fn size(&self, path: &str) -> Result<u64, PackIoError>;
    fn read(&self, path: &str) -> Result<Vec<u8>, PackIoError>;
}

/// An in-memory pack (tests, generated packs).
#[derive(Clone, Debug, Default)]
pub struct MemoryPack {
    files: BTreeMap<String, Vec<u8>>,
}

impl MemoryPack {
    pub fn new() -> MemoryPack {
        MemoryPack::default()
    }

    pub fn with(mut self, path: &str, content: &str) -> MemoryPack {
        self.files
            .insert(path.to_owned(), content.as_bytes().to_vec());
        self
    }

    pub fn with_bytes(mut self, path: &str, content: Vec<u8>) -> MemoryPack {
        self.files.insert(path.to_owned(), content);
        self
    }
}

impl PackFiles for MemoryPack {
    fn paths(&self, max_files: usize) -> Result<Vec<String>, PackIoError> {
        if self.files.len() > max_files {
            return Err(PackIoError(format!(
                "the pack has more than {max_files} files"
            )));
        }
        Ok(self.files.keys().cloned().collect())
    }

    fn size(&self, path: &str) -> Result<u64, PackIoError> {
        self.files
            .get(path)
            .map(|b| b.len() as u64)
            .ok_or_else(|| PackIoError(format!("no such file '{path}'")))
    }

    fn read(&self, path: &str) -> Result<Vec<u8>, PackIoError> {
        self.files
            .get(path)
            .cloned()
            .ok_or_else(|| PackIoError(format!("no such file '{path}'")))
    }
}

/// A pack stored as a directory on disk. Symbolic links are refused.
#[derive(Clone, Debug)]
pub struct DirPack {
    root: PathBuf,
}

impl DirPack {
    pub fn new(root: impl Into<PathBuf>) -> DirPack {
        DirPack { root: root.into() }
    }

    fn walk(
        &self,
        dir: &Path,
        prefix: &str,
        max_files: usize,
        out: &mut Vec<String>,
    ) -> Result<(), PackIoError> {
        let entries = std::fs::read_dir(dir)
            .map_err(|e| PackIoError(format!("cannot read {}: {e}", dir.display())))?;
        for entry in entries {
            let entry = entry.map_err(|e| PackIoError(e.to_string()))?;
            let name = entry.file_name().to_string_lossy().into_owned();
            let rel = if prefix.is_empty() {
                name.clone()
            } else {
                format!("{prefix}/{name}")
            };
            let meta =
                std::fs::symlink_metadata(entry.path()).map_err(|e| PackIoError(e.to_string()))?;
            if meta.file_type().is_symlink() {
                return Err(PackIoError(format!(
                    "'{rel}' is a symbolic link; links are not allowed in packs"
                )));
            }
            if meta.is_dir() {
                self.walk(&entry.path(), &rel, max_files, out)?;
            } else if meta.is_file() {
                out.push(rel);
                if out.len() > max_files {
                    return Err(PackIoError(format!(
                        "the pack has more than {max_files} files"
                    )));
                }
            }
        }
        Ok(())
    }

    fn resolve(&self, path: &str) -> Result<PathBuf, PackIoError> {
        if !is_safe_relative_path(path) {
            return Err(PackIoError(format!("'{path}' is not a safe pack path")));
        }
        Ok(self.root.join(path))
    }
}

impl PackFiles for DirPack {
    fn paths(&self, max_files: usize) -> Result<Vec<String>, PackIoError> {
        let mut out = Vec::new();
        self.walk(&self.root, "", max_files, &mut out)?;
        out.sort();
        Ok(out)
    }

    fn size(&self, path: &str) -> Result<u64, PackIoError> {
        std::fs::metadata(self.resolve(path)?)
            .map(|m| m.len())
            .map_err(|e| PackIoError(format!("{path}: {e}")))
    }

    fn read(&self, path: &str) -> Result<Vec<u8>, PackIoError> {
        std::fs::read(self.resolve(path)?).map_err(|e| PackIoError(format!("{path}: {e}")))
    }
}

/// A pack that loaded and validated.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LoadedPack {
    pub manifest: PackManifest,
    /// Templates in file order (path, then position within the file).
    pub templates: Vec<ObjectTemplate>,
    /// Hex BLAKE3 over every file's path and bytes (see [`hash_files`]).
    pub hash: String,
    pub file_count: usize,
    pub total_bytes: u64,
    /// Non-fatal findings.
    pub warnings: ValidationReport,
}

/// Hashes a set of files: BLAKE3 over, for each file in path order, the path and the content, each
/// prefixed with its length so no two different packs can produce the same byte stream.
pub fn hash_files(files: &BTreeMap<String, Vec<u8>>) -> String {
    let mut h = blake3::Hasher::new();
    for (path, content) in files {
        h.update(&(path.len() as u64).to_le_bytes());
        h.update(path.as_bytes());
        h.update(&(content.len() as u64).to_le_bytes());
        h.update(content);
    }
    h.finalize().to_hex().to_string()
}

/// Loads and validates a pack. On any error the whole report is returned.
pub fn load_pack(source: &dyn PackFiles, limits: &Limits) -> Result<LoadedPack, ValidationReport> {
    let mut report = ValidationReport::new();

    let paths = match source.paths(limits.max_files) {
        Ok(p) => p,
        Err(e) => {
            report.error("pack_io", "", e.to_string());
            return Err(report);
        }
    };

    let mut files: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    let mut total: u64 = 0;
    for path in &paths {
        if !is_safe_relative_path(path) {
            report.error(
                "unsafe_path",
                path.clone(),
                "file path is not a safe relative path",
            );
            continue;
        }
        match source.size(path) {
            Ok(size) if size > limits.max_file_bytes => {
                report.error(
                    "file_too_large",
                    path.clone(),
                    format!(
                        "{size} bytes exceeds the {} byte file limit",
                        limits.max_file_bytes
                    ),
                );
                continue;
            }
            Ok(size) => {
                total = total.saturating_add(size);
                if total > limits.max_total_bytes {
                    report.error(
                        "pack_too_large",
                        "",
                        format!(
                            "the pack exceeds the {} byte total limit",
                            limits.max_total_bytes
                        ),
                    );
                    return Err(report);
                }
            }
            Err(e) => {
                report.error("pack_io", path.clone(), e.to_string());
                continue;
            }
        }
        match source.read(path) {
            // The size can lie (a file growing between stat and read); check what was actually read.
            Ok(bytes) if bytes.len() as u64 > limits.max_file_bytes => {
                report.error(
                    "file_too_large",
                    path.clone(),
                    "file is larger than its reported size",
                );
            }
            Ok(bytes) => {
                files.insert(path.clone(), bytes);
            }
            Err(e) => report.error("pack_io", path.clone(), e.to_string()),
        }
    }
    if !report.is_ok() {
        return Err(report);
    }

    let manifest = match files.get(MANIFEST_FILE) {
        None => {
            report.error(
                "missing_manifest",
                MANIFEST_FILE,
                "the pack has no pack.json",
            );
            None
        }
        Some(bytes) => parse_json_file(MANIFEST_FILE, bytes, &mut report)
            .and_then(|v| PackManifest::from_canon(&v, MANIFEST_FILE, &mut report)),
    };

    let mut templates = Vec::new();
    let mut seen: BTreeMap<String, String> = BTreeMap::new();
    for (path, bytes) in &files {
        if !(path.starts_with(TEMPLATE_DIR) && path.ends_with(".json")) {
            continue;
        }
        let Some(value) = parse_json_file(path, bytes, &mut report) else {
            continue;
        };
        let items: Vec<(&pg_canon::Canon, String)> = match &value {
            pg_canon::Canon::List(list) => list
                .iter()
                .enumerate()
                .map(|(i, v)| (v, format!("{path}[{i}]")))
                .collect(),
            other => vec![(other, path.clone())],
        };
        for (item, ipath) in items {
            if let Some(t) = ObjectTemplate::from_canon(item, &ipath, &mut report) {
                if let Some(first) = seen.insert(t.id.to_string(), ipath.clone()) {
                    report.error(
                        "duplicate_template",
                        ipath,
                        format!("template '{}' is already defined in {first}", t.id),
                    );
                } else {
                    templates.push(t);
                }
            }
        }
    }

    if !report.is_ok() {
        return Err(report);
    }
    let Some(manifest) = manifest else {
        return Err(report);
    };
    Ok(LoadedPack {
        manifest,
        templates,
        hash: hash_files(&files),
        file_count: files.len(),
        total_bytes: total,
        warnings: report,
    })
}

fn parse_json_file(
    path: &str,
    bytes: &[u8],
    report: &mut ValidationReport,
) -> Option<pg_canon::Canon> {
    let text = match std::str::from_utf8(bytes) {
        Ok(t) => t,
        Err(e) => {
            report.error(
                "bad_encoding",
                path,
                format!("file is not valid UTF-8 (at byte {})", e.valid_up_to()),
            );
            return None;
        }
    };
    match json::parse(text) {
        Ok(v) => Some(v),
        Err(e) => {
            report.error("bad_json", path, e.to_string());
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MANIFEST: &str = r#"{"id":"base","name":"Base","version":"0.1.0"}"#;
    const ROOT: &str = r#"{"id":"base.object","schema":1}"#;

    fn pack() -> MemoryPack {
        MemoryPack::new()
            .with("pack.json", MANIFEST)
            .with("data/templates/a.json", ROOT)
    }

    fn load(p: &MemoryPack) -> Result<LoadedPack, ValidationReport> {
        load_pack(p, &Limits::default())
    }

    #[test]
    fn a_minimal_pack_loads() {
        let lp = load(&pack()).unwrap();
        assert_eq!(lp.manifest.id.as_str(), "base");
        assert_eq!(lp.templates.len(), 1);
        assert_eq!((lp.file_count, lp.hash.len()), (2, 64));
    }

    #[test]
    fn files_may_hold_one_template_or_a_list() {
        let p = MemoryPack::new().with("pack.json", MANIFEST).with(
            "data/templates/many.json",
            r#"[{"id":"base.object","schema":1},{"id":"base.item","schema":1,"extends":"base.object"}]"#,
        );
        assert_eq!(load(&p).unwrap().templates.len(), 2);
    }

    #[test]
    fn non_template_files_are_hashed_but_not_parsed() {
        let a = pack().with("assets/x.png", "not json at all");
        let b = pack().with("assets/x.png", "different bytes");
        let (la, lb) = (load(&a).unwrap(), load(&b).unwrap());
        assert_ne!(
            la.hash, lb.hash,
            "assets and scripts are part of the content hash"
        );
        assert_eq!(la.templates, lb.templates);
    }

    #[test]
    fn the_hash_is_stable_and_sensitive_to_content_and_names() {
        assert_eq!(load(&pack()).unwrap().hash, load(&pack()).unwrap().hash);
        let renamed = MemoryPack::new()
            .with("pack.json", MANIFEST)
            .with("data/templates/b.json", ROOT);
        assert_ne!(load(&pack()).unwrap().hash, load(&renamed).unwrap().hash);
        // Length prefixes keep ("ab","c") distinct from ("a","bc").
        let mut x = BTreeMap::new();
        x.insert("ab".to_owned(), b"c".to_vec());
        let mut y = BTreeMap::new();
        y.insert("a".to_owned(), b"bc".to_vec());
        assert_ne!(hash_files(&x), hash_files(&y));
    }

    #[test]
    fn a_missing_manifest_is_an_error() {
        let p = MemoryPack::new().with("data/templates/a.json", ROOT);
        assert!(load(&p).unwrap_err().has_code("missing_manifest"));
    }

    #[test]
    fn bad_json_and_bad_encoding_name_the_file() {
        let p = pack().with(
            "data/templates/broken.json",
            r#"{"id": "a.b", "schema": 1.5}"#,
        );
        let r = load(&p).unwrap_err();
        let issue = r.errors().find(|i| i.code == "bad_json").unwrap();
        assert_eq!(issue.path, "data/templates/broken.json");
        assert!(issue.message.contains("non-integer"), "{}", issue.message);

        let p = pack().with_bytes("data/templates/bin.json", vec![0xff, 0xfe, 0x00]);
        assert!(load(&p).unwrap_err().has_code("bad_encoding"));
    }

    #[test]
    fn duplicate_template_ids_in_one_pack_are_errors() {
        let p = pack().with("data/templates/b.json", ROOT);
        let r = load(&p).unwrap_err();
        assert!(r.has_code("duplicate_template"), "{r}");
    }

    #[test]
    fn errors_in_every_file_are_collected() {
        let p = pack()
            .with("data/templates/b.json", r#"{"id":"x"}"#)
            .with("data/templates/c.json", r#"{"id":"Y.z","schema":1}"#);
        let r = load(&p).unwrap_err();
        assert!(r.error_count() >= 2, "{r}");
    }

    #[test]
    fn unsafe_paths_are_rejected() {
        for bad in [
            "../evil.json",
            "a\\b.json",
            "/abs.json",
            "C:/x.json",
            "data//x.json",
        ] {
            let p = pack().with(bad, "{}");
            assert!(load(&p).unwrap_err().has_code("unsafe_path"), "{bad}");
        }
    }

    #[test]
    fn limits_are_enforced() {
        let p = pack();
        let tiny_files = Limits {
            max_files: 1,
            ..Limits::default()
        };
        assert!(load_pack(&p, &tiny_files).unwrap_err().has_code("pack_io"));
        let tiny_file = Limits {
            max_file_bytes: 10,
            ..Limits::default()
        };
        assert!(load_pack(&p, &tiny_file)
            .unwrap_err()
            .has_code("file_too_large"));
        let tiny_total = Limits {
            max_total_bytes: 40,
            ..Limits::default()
        };
        assert!(load_pack(&p, &tiny_total)
            .unwrap_err()
            .has_code("pack_too_large"));
    }

    #[test]
    fn warnings_do_not_block_loading() {
        let p = MemoryPack::new()
            .with(
                "pack.json",
                r#"{"id":"x","name":"X","version":"1","entry":"main.luau"}"#,
            )
            .with("data/templates/a.json", ROOT);
        let lp = load(&p).unwrap();
        assert!(lp.warnings.has_code("script_without_capabilities"));
    }

    #[test]
    fn the_directory_loader_matches_the_memory_loader_and_refuses_oversized_trees() {
        let dir = std::env::temp_dir().join(format!("pg_content_test_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("data/templates")).unwrap();
        std::fs::write(dir.join("pack.json"), MANIFEST).unwrap();
        std::fs::write(dir.join("data/templates/a.json"), ROOT).unwrap();
        let from_disk = load_pack(&DirPack::new(&dir), &Limits::default()).unwrap();
        let from_memory = load(&pack()).unwrap();
        assert_eq!(
            from_disk.hash, from_memory.hash,
            "same files, same hash, wherever they live"
        );
        assert!(load_pack(
            &DirPack::new(&dir),
            &Limits {
                max_files: 1,
                ..Limits::default()
            }
        )
        .is_err());
        assert!(
            DirPack::new(&dir).read("../outside").is_err(),
            "traversal is refused"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
