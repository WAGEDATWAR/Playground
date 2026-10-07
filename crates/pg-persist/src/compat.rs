//! Content compatibility report (suggestion S-014, Blueprint §13.4 step 3).
//!
//! A save or replay records which content packs (id, version, content hash) it was made with. On load the
//! report compares that list with what is installed and says, pack by pack, what is the same, what
//! changed and what is missing, so a mismatch is explained instead of failing mysteriously.

use pg_core::canon::{Canon, ToCanon};
use pg_core::read::{ReadError, Reader};
pub use pg_core::replay::ContentRefRecord;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CompatEntry {
    Same { pack: String },
    /// The version differs (the hash may too).
    VersionChanged { pack: String, recorded: String, installed: String },
    /// Same version, different content hash: the pack was edited without a version bump.
    ContentChanged { pack: String, recorded: String, installed: String },
    /// The save needs a pack that is not installed.
    Missing { pack: String },
    /// A pack is installed that the save did not use (harmless, but it changes new content).
    Extra { pack: String },
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CompatReport {
    pub entries: Vec<CompatEntry>,
}

fn short(h: &str) -> String {
    h.chars().take(8).collect()
}

/// Compares recorded refs with installed refs. Output order: recorded packs in recorded order, then extras.
pub fn compare(recorded: &[ContentRefRecord], installed: &[ContentRefRecord]) -> CompatReport {
    let mut entries = Vec::new();
    for want in recorded {
        let entry = match installed.iter().find(|h| h.pack_id == want.pack_id) {
            None => CompatEntry::Missing { pack: want.pack_id.clone() },
            Some(h) if h.version != want.version => CompatEntry::VersionChanged {
                pack: want.pack_id.clone(),
                recorded: want.version.clone(),
                installed: h.version.clone(),
            },
            Some(h) if h.hash != want.hash => CompatEntry::ContentChanged {
                pack: want.pack_id.clone(),
                recorded: short(&want.hash),
                installed: short(&h.hash),
            },
            Some(_) => CompatEntry::Same { pack: want.pack_id.clone() },
        };
        entries.push(entry);
    }
    for h in installed {
        if !recorded.iter().any(|w| w.pack_id == h.pack_id) {
            entries.push(CompatEntry::Extra { pack: h.pack_id.clone() });
        }
    }
    CompatReport { entries }
}

impl CompatReport {
    /// Everything matches exactly.
    pub fn is_exact(&self) -> bool {
        self.entries.iter().all(|e| matches!(e, CompatEntry::Same { .. }))
    }

    /// The world cannot be loaded faithfully: a required pack is missing.
    pub fn is_blocking(&self) -> bool {
        self.entries.iter().any(|e| matches!(e, CompatEntry::Missing { .. }))
    }

    /// One plain sentence per difference (nothing for exact matches).
    pub fn explain(&self) -> Vec<String> {
        self.entries
            .iter()
            .filter_map(|e| match e {
                CompatEntry::Same { .. } => None,
                CompatEntry::Missing { pack } => Some(format!("pack '{pack}' is required by this save but is not installed")),
                CompatEntry::VersionChanged { pack, recorded, installed } => Some(format!(
                    "pack '{pack}' is version {installed}; this save was made with {recorded}"
                )),
                CompatEntry::ContentChanged { pack, recorded, installed } => Some(format!(
                    "pack '{pack}' changed without a version change (recorded {recorded}, installed {installed})"
                )),
                CompatEntry::Extra { pack } => Some(format!("pack '{pack}' is installed but this save did not use it")),
            })
            .collect()
    }
}

pub fn refs_to_canon(refs: &[ContentRefRecord]) -> Canon {
    Canon::List(
        refs.iter()
            .map(|r| {
                Canon::map([
                    ("pack_id", Canon::str(r.pack_id.clone())),
                    ("version", Canon::str(r.version.clone())),
                    ("hash", Canon::str(r.hash.clone())),
                ])
            })
            .collect(),
    )
}

pub fn refs_from_reader(r: Reader<'_>) -> Result<Vec<ContentRefRecord>, ReadError> {
    r.list()?
        .iter()
        .map(|c| {
            let c = c.reader();
            c.only(&["pack_id", "version", "hash"])?;
            Ok(ContentRefRecord {
                pack_id: c.child("pack_id")?.reader().str()?.to_owned(),
                version: c.child("version")?.reader().str()?.to_owned(),
                hash: c.child("hash")?.reader().str()?.to_owned(),
            })
        })
        .collect()
}

impl ToCanon for CompatReport {
    fn to_canon(&self) -> Canon {
        Canon::List(self.explain().into_iter().map(Canon::Str).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(id: &str, v: &str, h: &str) -> ContentRefRecord {
        ContentRefRecord { pack_id: id.into(), version: v.into(), hash: h.into() }
    }

    #[test]
    fn identical_content_is_exact_and_silent() {
        let a = vec![r("base", "0.1.0", "aaaaaaaaaa"), r("coffee", "1.0.0", "bbbbbbbbbb")];
        let rep = compare(&a, &a);
        assert!(rep.is_exact() && !rep.is_blocking());
        assert!(rep.explain().is_empty());
    }

    #[test]
    fn every_kind_of_difference_is_named() {
        let recorded = vec![
            r("base", "0.1.0", "aaaaaaaaaaaa"),
            r("coffee", "1.0.0", "bbbbbbbbbbbb"),
            r("gone", "1.0.0", "cccccccccccc"),
            r("edited", "2.0.0", "dddddddddddd"),
        ];
        let installed = vec![
            r("base", "0.2.0", "aaaaaaaaaaaa"),
            r("coffee", "1.0.0", "bbbbbbbbbbbb"),
            r("edited", "2.0.0", "eeeeeeeeeeee"),
            r("new", "1.0.0", "ffffffffffff"),
        ];
        let rep = compare(&recorded, &installed);
        assert!(!rep.is_exact() && rep.is_blocking());
        let text = rep.explain();
        assert_eq!(text.len(), 4);
        assert!(text[0].contains("'base' is version 0.2.0") && text[0].contains("0.1.0"));
        assert!(text[1].contains("'gone'") && text[1].contains("not installed"));
        assert!(text[2].contains("'edited' changed without a version change"));
        assert!(text[3].contains("'new' is installed"));
    }

    #[test]
    fn extras_alone_are_not_blocking() {
        let rep = compare(&[], &[r("new", "1.0.0", "x")]);
        assert!(!rep.is_exact() && !rep.is_blocking());
    }

    #[test]
    fn refs_round_trip_through_canon() {
        let a = vec![r("base", "0.1.0", "aa"), r("c", "1.0.0", "bb")];
        let c = refs_to_canon(&a);
        let root = pg_core::read::Root::new(c);
        assert_eq!(refs_from_reader(root.reader()).unwrap(), a);
    }
}
