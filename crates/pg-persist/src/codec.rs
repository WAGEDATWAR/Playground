//! The `.pgsave` container (Blueprint §13.1): a small header followed by zstd-compressed canonical JSON.
//!
//! ```text
//! offset  size  field
//!      0     8  magic "PGSAVE\0\1"
//!      8     4  schema (u32 LE)          -- the world schema the payload was written with
//!     12     8  uncompressed length (u64 LE)
//!     20    32  BLAKE3 of the uncompressed payload
//!     52     .  zstd frame
//! ```
//!
//! Decoding is defensive: the header is checked before any decompression, the declared length is capped by
//! the caller, decompression stops at the declared length (a bomb cannot expand past what it announced),
//! and the checksum is verified last. Every failure is a typed [`CodecError`], never a panic.

use std::fmt;
use std::io::Read;

pub const MAGIC: [u8; 8] = *b"PGSAVE\0\x01";
pub const HEADER_LEN: usize = 52;

/// Default cap on a decompressed save (256 MiB): far above a real town, far below memory trouble.
pub const DEFAULT_MAX_UNCOMPRESSED: u64 = 256 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CodecError {
    /// Shorter than a header, or the magic does not match: not a save file at all.
    NotASave,
    /// The declared uncompressed length exceeds the caller's limit.
    TooLarge { declared: u64, limit: u64 },
    /// The compressed data could not be decoded.
    Corrupt(String),
    /// The data decoded to a different length than the header declared.
    LengthMismatch { declared: u64, actual: u64 },
    /// The checksum does not match: the file was damaged after it was written.
    ChecksumMismatch,
}

impl fmt::Display for CodecError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CodecError::NotASave => write!(f, "not a Playground save file"),
            CodecError::TooLarge { declared, limit } => write!(
                f,
                "the save declares {declared} bytes uncompressed, over the limit of {limit}"
            ),
            CodecError::Corrupt(e) => write!(f, "the save data is corrupt: {e}"),
            CodecError::LengthMismatch { declared, actual } => write!(
                f,
                "the save decoded to {actual} bytes but declared {declared}"
            ),
            CodecError::ChecksumMismatch => write!(f, "the save's checksum does not match"),
        }
    }
}

impl std::error::Error for CodecError {}

/// The parsed header, available without decompressing anything (`pg save inspect`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Header {
    pub schema: u32,
    pub uncompressed_len: u64,
    pub checksum: [u8; 32],
}

pub fn parse_header(bytes: &[u8]) -> Result<Header, CodecError> {
    if bytes.len() < HEADER_LEN || bytes.get(..8) != Some(&MAGIC[..]) {
        return Err(CodecError::NotASave);
    }
    let schema = bytes
        .get(8..12)
        .and_then(|b| b.try_into().ok())
        .map(u32::from_le_bytes)
        .ok_or(CodecError::NotASave)?;
    let uncompressed_len = bytes
        .get(12..20)
        .and_then(|b| b.try_into().ok())
        .map(u64::from_le_bytes)
        .ok_or(CodecError::NotASave)?;
    let checksum: [u8; 32] = bytes
        .get(20..52)
        .and_then(|b| b.try_into().ok())
        .ok_or(CodecError::NotASave)?;
    Ok(Header {
        schema,
        uncompressed_len,
        checksum,
    })
}

/// Compresses `payload` and wraps it in the container.
pub fn encode(schema: u32, payload: &[u8]) -> Vec<u8> {
    let compressed = ruzstd::encoding::compress_to_vec(
        payload,
        ruzstd::encoding::CompressionLevel::Fastest,
    );
    let mut out = Vec::with_capacity(HEADER_LEN + compressed.len());
    out.extend_from_slice(&MAGIC);
    out.extend_from_slice(&schema.to_le_bytes());
    out.extend_from_slice(&(payload.len() as u64).to_le_bytes());
    out.extend_from_slice(blake3::hash(payload).as_bytes());
    out.extend_from_slice(&compressed);
    out
}

/// Verifies and unwraps a container, returning the schema and the payload.
pub fn decode(bytes: &[u8], max_uncompressed: u64) -> Result<(u32, Vec<u8>), CodecError> {
    let header = parse_header(bytes)?;
    if header.uncompressed_len > max_uncompressed {
        return Err(CodecError::TooLarge {
            declared: header.uncompressed_len,
            limit: max_uncompressed,
        });
    }
    let body = bytes.get(HEADER_LEN..).ok_or(CodecError::NotASave)?;
    let mut decoder = ruzstd::decoding::StreamingDecoder::new(body)
        .map_err(|e| CodecError::Corrupt(e.to_string()))?;
    // Read one byte past the declared length so an over-long stream is detected, not truncated silently.
    let mut payload = Vec::new();
    (&mut decoder)
        .take(header.uncompressed_len.saturating_add(1))
        .read_to_end(&mut payload)
        .map_err(|e| CodecError::Corrupt(e.to_string()))?;
    if payload.len() as u64 != header.uncompressed_len {
        return Err(CodecError::LengthMismatch {
            declared: header.uncompressed_len,
            actual: payload.len() as u64,
        });
    }
    if blake3::hash(&payload).as_bytes() != &header.checksum {
        return Err(CodecError::ChecksumMismatch);
    }
    Ok((header.schema, payload))
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn sample() -> Vec<u8> {
        br#"{"hello":"world","n":[1,2,3,4,5,6,7,8,9,10]}"#.repeat(200)
    }

    #[test]
    fn round_trip_and_compression() {
        let payload = sample();
        let blob = encode(3, &payload);
        assert!(blob.len() < payload.len() / 4, "repetitive JSON compresses well");
        let (schema, back) = decode(&blob, DEFAULT_MAX_UNCOMPRESSED).unwrap();
        assert_eq!((schema, back), (3, payload));
        let h = parse_header(&blob).unwrap();
        assert_eq!((h.schema, h.uncompressed_len), (3, sample().len() as u64));
    }

    #[test]
    fn empty_and_tiny_payloads_round_trip() {
        for p in [&b""[..], b"x", b"{}"] {
            let (s, back) = decode(&encode(1, p), 1024).unwrap();
            assert_eq!((s, back.as_slice()), (1, p));
        }
    }

    #[test]
    fn non_saves_are_recognised() {
        assert_eq!(decode(b"", 10), Err(CodecError::NotASave));
        assert_eq!(decode(b"PGSAVE", 10), Err(CodecError::NotASave));
        assert_eq!(decode(&[0u8; 100], 10), Err(CodecError::NotASave));
        let mut blob = encode(1, b"abc");
        blob[0] ^= 0xFF;
        assert_eq!(decode(&blob, 10), Err(CodecError::NotASave));
    }

    #[test]
    fn a_flipped_byte_never_yields_a_wrong_payload() {
        let payload = sample();
        let blob = encode(2, &payload);
        let mut caught = 0;
        for i in 0..blob.len() {
            let mut bad = blob.clone();
            bad[i] ^= 0x55;
            match decode(&bad, DEFAULT_MAX_UNCOMPRESSED) {
                Err(_) => caught += 1,
                // Some bits (the schema field, unused frame bits) change nothing about the payload; what
                // must never happen is a different payload being accepted.
                Ok((_, p)) => assert_eq!(p, payload, "byte {i} flipped and a wrong payload was accepted"),
            }
        }
        assert!(caught * 10 > blob.len() * 9, "almost every flip is detected: {caught}/{}", blob.len());
    }

    #[test]
    fn truncation_is_detected() {
        let blob = encode(2, &sample());
        for cut in [HEADER_LEN, HEADER_LEN + 1, blob.len() / 2, blob.len() - 1] {
            assert!(decode(&blob[..cut], DEFAULT_MAX_UNCOMPRESSED).is_err(), "cut at {cut}");
        }
    }

    #[test]
    fn a_declared_size_over_the_limit_is_refused_before_decompressing() {
        let blob = encode(1, &sample());
        assert!(matches!(decode(&blob, 100), Err(CodecError::TooLarge { .. })));
    }

    #[test]
    fn a_lying_length_field_is_caught_both_ways() {
        let payload = sample();
        let blob = encode(1, &payload);
        // Claims less than the real size: the stream is longer than announced.
        let mut small = blob.clone();
        small[12..20].copy_from_slice(&10u64.to_le_bytes());
        assert!(matches!(decode(&small, 1 << 20), Err(CodecError::LengthMismatch { .. })));
        // Claims more: the stream ends early.
        let mut big = blob;
        big[12..20].copy_from_slice(&(payload.len() as u64 + 5).to_le_bytes());
        assert!(matches!(decode(&big, 1 << 20), Err(CodecError::LengthMismatch { .. })));
    }

    proptest! {
        #[test]
        fn arbitrary_bytes_never_panic(data in prop::collection::vec(any::<u8>(), 0..400)) {
            let _ = decode(&data, 1 << 16);
            // Also with a valid header in front of garbage.
            let mut framed = encode(1, b"seed");
            framed.truncate(HEADER_LEN);
            framed.extend_from_slice(&data);
            let _ = decode(&framed, 1 << 16);
        }

        #[test]
        fn any_payload_round_trips(payload in prop::collection::vec(any::<u8>(), 0..2000)) {
            let (_, back) = decode(&encode(9, &payload), 1 << 20).unwrap();
            prop_assert_eq!(back, payload);
        }
    }
}
