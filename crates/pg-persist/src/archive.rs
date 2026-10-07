//! Hardened archive reader for `.pgpack` content packs (Blueprint §13.6, §23).
//!
//! Archives are untrusted input from strangers. The reader decides everything from the archive's central
//! directory *before* decompressing, then enforces the limits again on the bytes actually produced, because
//! a hostile archive can lie about sizes. It rejects:
//!
//! * path traversal (`..`, absolute paths, drive letters, backslashes, empty segments, control characters);
//! * names Windows cannot represent safely (reserved device names, trailing dots or spaces);
//! * symlinks and encrypted entries;
//! * duplicate names, including names that differ only by case;
//! * too many entries, oversized files, an oversized total, and absurd compression ratios (zip bombs).
//!
//! Nothing is ever written to disk here: the result is a list of `(path, bytes)` that the pack loader
//! validates. Installation (after the manifest validates and the player approves capabilities) is Stage 11.

use std::collections::BTreeSet;
use std::fmt;
use std::io::{Cursor, Read};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ArchiveLimits {
    pub max_archive_bytes: u64,
    pub max_entries: usize,
    pub max_file_bytes: u64,
    pub max_total_bytes: u64,
    /// Largest allowed uncompressed/compressed ratio (applies to entries over 1 KiB).
    pub max_ratio: u64,
    pub max_path_len: usize,
}

impl Default for ArchiveLimits {
    fn default() -> Self {
        ArchiveLimits {
            max_archive_bytes: 32 * 1024 * 1024,
            max_entries: 2_000,
            max_file_bytes: 16 * 1024 * 1024,
            max_total_bytes: 64 * 1024 * 1024,
            max_ratio: 100,
            max_path_len: 200,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ArchiveError {
    ArchiveTooLarge {
        bytes: u64,
        limit: u64,
    },
    NotAnArchive(String),
    TooManyEntries {
        count: usize,
        limit: usize,
    },
    BadPath {
        path: String,
        why: &'static str,
    },
    Symlink(String),
    Encrypted(String),
    Duplicate(String),
    FileTooLarge {
        path: String,
        bytes: u64,
        limit: u64,
    },
    TotalTooLarge {
        limit: u64,
    },
    SuspiciousRatio {
        path: String,
        ratio: u64,
        limit: u64,
    },
    /// The entry produced a different number of bytes than its header declared.
    Corrupt {
        path: String,
        why: String,
    },
}

impl fmt::Display for ArchiveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ArchiveError::ArchiveTooLarge { bytes, limit } => {
                write!(f, "the archive is {bytes} bytes; the limit is {limit}")
            }
            ArchiveError::NotAnArchive(e) => write!(f, "not a readable archive: {e}"),
            ArchiveError::TooManyEntries { count, limit } => {
                write!(f, "the archive has {count} entries; the limit is {limit}")
            }
            ArchiveError::BadPath { path, why } => write!(f, "unsafe path '{path}': {why}"),
            ArchiveError::Symlink(p) => {
                write!(f, "'{p}' is a symbolic link, which packs may not contain")
            }
            ArchiveError::Encrypted(p) => write!(f, "'{p}' is encrypted, which packs may not be"),
            ArchiveError::Duplicate(p) => write!(
                f,
                "'{p}' appears more than once (names are compared ignoring case)"
            ),
            ArchiveError::FileTooLarge { path, bytes, limit } => {
                write!(
                    f,
                    "'{path}' is {bytes} bytes; the per-file limit is {limit}"
                )
            }
            ArchiveError::TotalTooLarge { limit } => {
                write!(
                    f,
                    "the archive expands past the total limit of {limit} bytes"
                )
            }
            ArchiveError::SuspiciousRatio { path, ratio, limit } => write!(
                f,
                "'{path}' expands {ratio}:1 (limit {limit}:1); this looks like a compression bomb"
            ),
            ArchiveError::Corrupt { path, why } => write!(f, "'{path}' is corrupt: {why}"),
        }
    }
}

impl std::error::Error for ArchiveError {}

const RESERVED: [&str; 22] = [
    "con", "prn", "aux", "nul", "com1", "com2", "com3", "com4", "com5", "com6", "com7", "com8",
    "com9", "lpt1", "lpt2", "lpt3", "lpt4", "lpt5", "lpt6", "lpt7", "lpt8", "lpt9",
];

/// Checks one entry name. Directory names may end in `/`.
pub fn check_path(path: &str, max_len: usize) -> Result<(), &'static str> {
    if path.is_empty() {
        return Err("empty name");
    }
    if path.len() > max_len {
        return Err("name too long");
    }
    if path.starts_with('/') {
        return Err("absolute path");
    }
    if path.contains('\\') {
        return Err("backslash in name");
    }
    if path.contains(':') {
        return Err("drive letter or stream separator");
    }
    if path.chars().any(char::is_control) {
        return Err("control character in name");
    }
    let trimmed = path.strip_suffix('/').unwrap_or(path);
    for seg in trimmed.split('/') {
        if seg.is_empty() {
            return Err("empty path segment");
        }
        if seg == "." || seg == ".." {
            return Err("'.' or '..' segment");
        }
        if seg.ends_with('.') || seg.ends_with(' ') {
            return Err("segment ends with a dot or space");
        }
        let stem = seg.split('.').next().unwrap_or(seg).to_ascii_lowercase();
        if RESERVED.contains(&stem.as_str()) {
            return Err("reserved device name");
        }
    }
    Ok(())
}

/// Reads every file of a zip archive into memory, enforcing the limits. Returns `(path, bytes)` in archive
/// order; directories are validated but not returned.
pub fn read_archive(
    bytes: &[u8],
    limits: &ArchiveLimits,
) -> Result<Vec<(String, Vec<u8>)>, ArchiveError> {
    if bytes.len() as u64 > limits.max_archive_bytes {
        return Err(ArchiveError::ArchiveTooLarge {
            bytes: bytes.len() as u64,
            limit: limits.max_archive_bytes,
        });
    }
    let mut zip = zip::ZipArchive::new(Cursor::new(bytes))
        .map_err(|e| ArchiveError::NotAnArchive(e.to_string()))?;
    if zip.len() > limits.max_entries {
        return Err(ArchiveError::TooManyEntries {
            count: zip.len(),
            limit: limits.max_entries,
        });
    }

    // Pass 1: judge every entry from the central directory, before decompressing anything.
    let mut seen = BTreeSet::new();
    let mut declared_total: u64 = 0;
    for i in 0..zip.len() {
        let entry = zip
            .by_index_raw(i)
            .map_err(|e| ArchiveError::NotAnArchive(e.to_string()))?;
        let name = entry.name().to_owned();
        check_path(&name, limits.max_path_len).map_err(|why| ArchiveError::BadPath {
            path: name.clone(),
            why,
        })?;
        if entry.encrypted() {
            return Err(ArchiveError::Encrypted(name));
        }
        if entry
            .unix_mode()
            .is_some_and(|m| m & 0o170_000 == 0o120_000)
        {
            return Err(ArchiveError::Symlink(name));
        }
        if !seen.insert(name.trim_end_matches('/').to_ascii_lowercase()) {
            return Err(ArchiveError::Duplicate(name));
        }
        if entry.is_dir() {
            continue;
        }
        let (size, packed) = (entry.size(), entry.compressed_size());
        if size > limits.max_file_bytes {
            return Err(ArchiveError::FileTooLarge {
                path: name,
                bytes: size,
                limit: limits.max_file_bytes,
            });
        }
        if size > 1024 && size / packed.max(1) > limits.max_ratio {
            return Err(ArchiveError::SuspiciousRatio {
                path: name,
                ratio: size / packed.max(1),
                limit: limits.max_ratio,
            });
        }
        declared_total = declared_total.saturating_add(size);
        if declared_total > limits.max_total_bytes {
            return Err(ArchiveError::TotalTooLarge {
                limit: limits.max_total_bytes,
            });
        }
    }

    // Pass 2: decompress, never trusting the declared size.
    let mut out = Vec::new();
    let mut actual_total: u64 = 0;
    for i in 0..zip.len() {
        let entry = zip
            .by_index(i)
            .map_err(|e| ArchiveError::NotAnArchive(e.to_string()))?;
        if entry.is_dir() {
            continue;
        }
        let name = entry.name().to_owned();
        let declared = entry.size();
        let mut data = Vec::new();
        entry
            .take(declared.saturating_add(1))
            .read_to_end(&mut data)
            .map_err(|e| ArchiveError::Corrupt {
                path: name.clone(),
                why: e.to_string(),
            })?;
        if data.len() as u64 != declared {
            return Err(ArchiveError::Corrupt {
                path: name,
                why: format!("declared {declared} bytes, produced {}", data.len()),
            });
        }
        actual_total = actual_total.saturating_add(data.len() as u64);
        if actual_total > limits.max_total_bytes {
            return Err(ArchiveError::TotalTooLarge {
                limit: limits.max_total_bytes,
            });
        }
        out.push((name, data));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use zip::write::SimpleFileOptions;
    use zip::{CompressionMethod, ZipWriter};

    fn build(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut w = ZipWriter::new(Cursor::new(Vec::new()));
        for (name, data) in entries {
            let opts = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
            w.start_file(*name, opts).unwrap();
            w.write_all(data).unwrap();
        }
        w.finish().unwrap().into_inner()
    }

    fn read(entries: &[(&str, &[u8])]) -> Result<Vec<(String, Vec<u8>)>, ArchiveError> {
        read_archive(&build(entries), &ArchiveLimits::default())
    }

    #[test]
    fn a_normal_pack_reads_back() {
        let got = read(&[
            ("pack.json", br#"{"id":"x"}"#),
            ("data/templates/a.json", b"[]"),
            ("scripts/main.luau", b"return 1"),
        ])
        .unwrap();
        assert_eq!(got.len(), 3);
        assert_eq!(got[0], ("pack.json".to_owned(), br#"{"id":"x"}"#.to_vec()));
    }

    #[test]
    fn traversal_and_unsafe_names_are_rejected() {
        for bad in [
            "../evil.txt",
            "a/../../evil.txt",
            "/etc/passwd",
            "C:/Windows/x",
            "a\\b.txt",
            "a//b.txt",
            "./a.txt",
            "con.txt",
            "dir/NUL",
            "dir/aux.json",
            "trailing.",
            "space ",
            "ctrl\u{7}.txt",
            "",
        ] {
            let r = read(&[(bad, b"x")]);
            assert!(
                matches!(r, Err(ArchiveError::BadPath { .. })),
                "{bad:?} -> {r:?}"
            );
        }
        assert!(check_path(&"a/".repeat(300), 200).is_err());
        assert!(check_path("fine/name.json", 200).is_ok());
        assert!(check_path("dir/", 200).is_ok());
    }

    #[test]
    fn duplicates_including_case_variants_are_rejected() {
        assert!(matches!(
            read(&[("Readme.md", b"1"), ("README.MD", b"2")]),
            Err(ArchiveError::Duplicate(_))
        ));
    }

    #[test]
    fn symlinks_are_rejected() {
        let mut w = ZipWriter::new(Cursor::new(Vec::new()));
        w.add_symlink("link", "/etc/passwd", SimpleFileOptions::default())
            .unwrap();
        let bytes = w.finish().unwrap().into_inner();
        assert!(matches!(
            read_archive(&bytes, &ArchiveLimits::default()),
            Err(ArchiveError::Symlink(_))
        ));
    }

    #[test]
    fn a_compression_bomb_is_stopped_by_the_ratio_and_size_limits() {
        // 8 MiB of zeros deflates to a few KiB.
        let zeros = vec![0u8; 8 * 1024 * 1024];
        let r = read(&[("bomb.bin", &zeros)]);
        assert!(
            matches!(r, Err(ArchiveError::SuspiciousRatio { .. })),
            "{r:?}"
        );
        // With the ratio check relaxed, the per-file and total limits still hold.
        let lenient = ArchiveLimits {
            max_ratio: u64::MAX,
            max_file_bytes: 1024 * 1024,
            ..ArchiveLimits::default()
        };
        assert!(matches!(
            read_archive(&build(&[("bomb.bin", &zeros)]), &lenient),
            Err(ArchiveError::FileTooLarge { .. })
        ));
        let total = ArchiveLimits {
            max_ratio: u64::MAX,
            max_total_bytes: 12 * 1024 * 1024,
            ..ArchiveLimits::default()
        };
        let two = build(&[("a.bin", &zeros), ("b.bin", &zeros)]);
        assert!(matches!(
            read_archive(&two, &total),
            Err(ArchiveError::TotalTooLarge { .. })
        ));
    }

    #[test]
    fn too_many_entries_and_oversized_archives_are_rejected() {
        let names: Vec<String> = (0..60).map(|i| format!("f{i}.txt")).collect();
        let entries: Vec<(&str, &[u8])> = names.iter().map(|n| (n.as_str(), &b"x"[..])).collect();
        let few = ArchiveLimits {
            max_entries: 50,
            ..ArchiveLimits::default()
        };
        assert!(matches!(
            read_archive(&build(&entries), &few),
            Err(ArchiveError::TooManyEntries {
                count: 60,
                limit: 50
            })
        ));
        let tiny = ArchiveLimits {
            max_archive_bytes: 10,
            ..ArchiveLimits::default()
        };
        assert!(matches!(
            read_archive(&build(&entries), &tiny),
            Err(ArchiveError::ArchiveTooLarge { .. })
        ));
    }

    #[test]
    fn garbage_and_truncated_archives_do_not_panic() {
        assert!(matches!(
            read_archive(b"", &ArchiveLimits::default()),
            Err(ArchiveError::NotAnArchive(_))
        ));
        assert!(read_archive(b"PK\x03\x04 not really", &ArchiveLimits::default()).is_err());
        let good = build(&[("a.txt", b"hello world"), ("b.txt", b"more")]);
        for cut in [good.len() - 1, good.len() / 2, 30, 4] {
            assert!(
                read_archive(&good[..cut], &ArchiveLimits::default()).is_err(),
                "cut {cut}"
            );
        }
    }

    #[test]
    fn mutated_archives_never_panic() {
        let good = build(&[("a.txt", b"hello world"), ("dir/b.json", br#"{"k":1}"#)]);
        for i in 0..good.len() {
            let mut bad = good.clone();
            bad[i] ^= 0xA5;
            let _ = read_archive(&bad, &ArchiveLimits::default());
        }
    }
}
