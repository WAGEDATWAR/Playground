//! Compressed replay logs and bug bundles (suggestions S-019 and S-001, Blueprint §20).
//!
//! * A **`.pglog`** is a replay log in the compressed container: the same canonical JSON, a fraction of the
//!   size, with a checksum.
//! * A **`.pgbundle`** is a log trimmed to a point of interest: a snapshot of the world at that tick, the
//!   inputs still queued, the content references and the expected hashes afterwards, plus a note from
//!   whoever made it. `pg bugbundle run` replays it on any machine and says whether the hashes match, so a
//!   bug report is one small file instead of a description.

use crate::codec::{self, CodecError, Container};
use pg_content::ContentSet;
use pg_core::canon::{json, Canon, ToCanon};
use pg_core::replay::{ReplayError, ReplayLog};
use std::fmt;
use std::sync::Arc;

pub const BUNDLE_FORMAT: &str = "playground-bugbundle";
const LIMIT: u64 = 512 * 1024 * 1024;

#[derive(Debug)]
pub enum LogFileError {
    Codec(CodecError),
    Parse(String),
    Replay(ReplayError),
    /// The bytes are not a log, compressed log or bundle.
    Unrecognised,
}

impl fmt::Display for LogFileError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LogFileError::Codec(e) => write!(f, "{e}"),
            LogFileError::Parse(e) => write!(f, "{e}"),
            LogFileError::Replay(e) => write!(f, "{e}"),
            LogFileError::Unrecognised => {
                write!(f, "not a replay log, compressed log or bug bundle")
            }
        }
    }
}

impl std::error::Error for LogFileError {}

impl From<CodecError> for LogFileError {
    fn from(e: CodecError) -> Self {
        LogFileError::Codec(e)
    }
}

/// A compressed replay log.
pub fn encode_log(log: &ReplayLog) -> Vec<u8> {
    codec::encode_as(
        Container::ReplayLog,
        pg_core::replay::REPLAY_VERSION,
        log.to_canon().to_canonical_string().as_bytes(),
    )
}

fn parse_log(payload: &[u8]) -> Result<ReplayLog, LogFileError> {
    let text =
        std::str::from_utf8(payload).map_err(|_| LogFileError::Parse("not UTF-8".to_owned()))?;
    let canon = json::parse(text).map_err(|e| LogFileError::Parse(e.to_string()))?;
    ReplayLog::from_canon(&canon).map_err(|e| LogFileError::Parse(e.to_string()))
}

pub fn decode_log(bytes: &[u8]) -> Result<ReplayLog, LogFileError> {
    let (_, payload) = codec::decode_as(Container::ReplayLog, bytes, LIMIT)?;
    parse_log(&payload)
}

/// A trimmed log plus who made it and why.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Bundle {
    pub note: String,
    pub created_iso: String,
    pub app_version: String,
    pub log: ReplayLog,
}

impl Bundle {
    /// Trims `log` at absolute tick `at` and wraps it.
    pub fn make(
        log: &ReplayLog,
        at: u64,
        content: Option<Arc<ContentSet>>,
        note: &str,
        created_iso: &str,
        app_version: &str,
    ) -> Result<Bundle, LogFileError> {
        Ok(Bundle {
            note: note.to_owned(),
            created_iso: created_iso.to_owned(),
            app_version: app_version.to_owned(),
            log: log.trim(at, content).map_err(LogFileError::Replay)?,
        })
    }

    pub fn encode(&self) -> Vec<u8> {
        let doc = Canon::map([
            ("format", Canon::str(BUNDLE_FORMAT)),
            ("note", Canon::str(self.note.clone())),
            ("created_iso", Canon::str(self.created_iso.clone())),
            ("app_version", Canon::str(self.app_version.clone())),
            ("log", self.log.to_canon()),
        ]);
        codec::encode_as(
            Container::Bundle,
            pg_core::replay::REPLAY_VERSION,
            doc.to_canonical_string().as_bytes(),
        )
    }

    pub fn decode(bytes: &[u8]) -> Result<Bundle, LogFileError> {
        let (_, payload) = codec::decode_as(Container::Bundle, bytes, LIMIT)?;
        let text = std::str::from_utf8(&payload)
            .map_err(|_| LogFileError::Parse("not UTF-8".to_owned()))?;
        let doc = json::parse(text).map_err(|e| LogFileError::Parse(e.to_string()))?;
        let field = |k: &str| doc.field(k).map_err(|e| LogFileError::Parse(e.to_string()));
        if field("format")?.as_str() != Some(BUNDLE_FORMAT) {
            return Err(LogFileError::Parse("not a bug bundle".to_owned()));
        }
        let text_of = |k: &str| -> Result<String, LogFileError> {
            field(k)?
                .as_str()
                .map(str::to_owned)
                .ok_or_else(|| LogFileError::Parse(format!("{k} must be text")))
        };
        Ok(Bundle {
            note: text_of("note")?,
            created_iso: text_of("created_iso")?,
            app_version: text_of("app_version")?,
            log: ReplayLog::from_canon(field("log")?)
                .map_err(|e| LogFileError::Parse(e.to_string()))?,
        })
    }
}

/// Opens any replay-shaped file: a plain JSON log, a `.pglog`, or a `.pgbundle` (its log).
pub fn read_any(bytes: &[u8]) -> Result<ReplayLog, LogFileError> {
    match Container::detect(bytes) {
        Some(Container::ReplayLog) => decode_log(bytes),
        Some(Container::Bundle) => Ok(Bundle::decode(bytes)?.log),
        Some(Container::Save) => Err(LogFileError::Unrecognised),
        None => parse_log(bytes).map_err(|_| LogFileError::Unrecognised),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testkit::town_sim;
    use pg_core::replay::replay;

    fn log(ticks: u64) -> ReplayLog {
        let mut sim = town_sim();
        sim.run_ticks(ticks).unwrap();
        ReplayLog::record(&sim)
    }

    #[test]
    fn compressed_logs_are_much_smaller_and_identical_after_decoding() {
        let l = log(30_000);
        let plain = l.to_canon().to_canonical_string();
        let packed = encode_log(&l);
        assert!(
            packed.len() * 2 < plain.len(),
            "{} vs {}",
            packed.len(),
            plain.len()
        );
        let back = decode_log(&packed).unwrap();
        assert_eq!(back, l);
        assert!(replay(&back, None).unwrap().ok());
    }

    #[test]
    fn a_bundle_replays_the_tail_and_matches_the_original_end_state() {
        let l = log(40_000);
        let b = Bundle::make(
            &l,
            20_000,
            None,
            "pawns stuck at noon",
            "2026-10-06T00:00:00Z",
            "0.0.1",
        )
        .unwrap();
        let bytes = b.encode();
        let back = Bundle::decode(&bytes).unwrap();
        assert_eq!(back, b);
        assert_eq!(back.note, "pawns stuck at noon");
        let out = replay(&back.log, None).unwrap();
        assert!(out.ok(), "{:?}", out.mismatches);
        assert_eq!(out.final_hash.to_hex(), l.final_hash);
        // A bundle carries a world snapshot, so it is bigger than a bare log but still compressed.
        let plain_bundle = back.log.to_canon().to_canonical_string().len();
        assert!(
            bytes.len() * 2 < plain_bundle,
            "{} vs {plain_bundle}",
            bytes.len()
        );
    }

    #[test]
    fn containers_are_not_interchangeable() {
        let l = log(2_000);
        let packed = encode_log(&l);
        assert!(Bundle::decode(&packed).is_err());
        let bundle = Bundle::make(&l, 1_000, None, "n", "t", "v")
            .unwrap()
            .encode();
        assert!(decode_log(&bundle).is_err());
    }

    #[test]
    fn read_any_takes_all_three_shapes_and_refuses_the_rest() {
        let l = log(3_000);
        let plain = l.to_canon().to_canonical_string().into_bytes();
        assert_eq!(read_any(&plain).unwrap(), l);
        assert_eq!(read_any(&encode_log(&l)).unwrap(), l);
        let b = Bundle::make(&l, 1_500, None, "n", "t", "v").unwrap();
        assert_eq!(read_any(&b.encode()).unwrap(), b.log);
        assert!(matches!(
            read_any(b"hello"),
            Err(LogFileError::Unrecognised)
        ));
        assert!(matches!(
            read_any(&codec::encode(3, b"{}")),
            Err(LogFileError::Unrecognised)
        ));
    }

    #[test]
    fn damaged_files_are_reported_not_trusted() {
        let l = log(2_000);
        let mut packed = encode_log(&l);
        let mid = packed.len() / 2;
        packed[mid] ^= 0xFF;
        assert!(decode_log(&packed).is_err());
        let mut bundle = Bundle::make(&l, 1_000, None, "n", "t", "v")
            .unwrap()
            .encode();
        bundle.truncate(bundle.len() - 10);
        assert!(Bundle::decode(&bundle).is_err());
    }

    #[test]
    fn trimming_out_of_range_is_an_error() {
        let l = log(1_000);
        assert!(Bundle::make(&l, 5_000, None, "n", "t", "v").is_err());
    }
}
