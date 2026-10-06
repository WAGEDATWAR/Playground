//! State hashing (Blueprint §5.4): BLAKE3 over the canonical serialization.
//!
//! `hash_state` hashes each table independently and then combines the per-table hashes, so a
//! divergence can be localized to the first differing table (see `docs/SUGGESTIONS.md` S-002).

use crate::canon::{Canon, ToCanon};
use std::fmt;

/// A 256-bit state hash.
#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct StateHash([u8; 32]);

impl StateHash {
    pub const fn from_bytes(bytes: [u8; 32]) -> StateHash {
        StateHash(bytes)
    }

    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    /// Lowercase hex, 64 characters.
    pub fn to_hex(&self) -> String {
        let mut s = String::with_capacity(64);
        for b in self.0 {
            s.push(char::from_digit(u32::from(b >> 4), 16).unwrap_or('0'));
            s.push(char::from_digit(u32::from(b & 0xF), 16).unwrap_or('0'));
        }
        s
    }

    /// First 8 hex characters, for logs.
    pub fn short(&self) -> String {
        self.to_hex().chars().take(8).collect()
    }
}

impl fmt::Display for StateHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_hex())
    }
}

impl fmt::Debug for StateHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "StateHash({})", self.short())
    }
}

/// Hashes raw bytes.
pub fn hash_raw(bytes: &[u8]) -> StateHash {
    StateHash(*blake3::hash(bytes).as_bytes())
}

/// Hashes the canonical text of `value`.
pub fn hash_canon(value: &Canon) -> StateHash {
    hash_raw(value.to_canonical_string().as_bytes())
}

/// Hashes anything with a canonical form.
pub fn hash_value<T: ToCanon + ?Sized>(value: &T) -> StateHash {
    hash_canon(&value.to_canon())
}

/// Combines named per-table hashes into one. The combination is itself a canonical map, so it does
/// not depend on the order the pairs are supplied in.
pub fn combine_table_hashes<'a>(
    tables: impl IntoIterator<Item = (&'a str, StateHash)>,
) -> StateHash {
    hash_canon(&Canon::map(
        tables
            .into_iter()
            .map(|(name, h)| (name, Canon::Str(h.to_hex()))),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blake3_empty_input_matches_the_published_vector() {
        assert_eq!(
            hash_raw(b"").to_hex(),
            "af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262"
        );
    }

    #[test]
    fn equal_values_hash_equal_and_different_values_differ() {
        let a = Canon::map([("x", Canon::Int(1))]);
        let b = Canon::map([("x", Canon::Int(1))]);
        let c = Canon::map([("x", Canon::Int(2))]);
        assert_eq!(hash_canon(&a), hash_canon(&b));
        assert_ne!(hash_canon(&a), hash_canon(&c));
    }

    #[test]
    fn combined_hash_ignores_supply_order_but_not_content() {
        let h1 = hash_raw(b"one");
        let h2 = hash_raw(b"two");
        let a = combine_table_hashes([("pawns", h1), ("objects", h2)]);
        let b = combine_table_hashes([("objects", h2), ("pawns", h1)]);
        let swapped = combine_table_hashes([("pawns", h2), ("objects", h1)]);
        assert_eq!(a, b);
        assert_ne!(a, swapped);
    }

    #[test]
    fn hex_is_64_lowercase_chars() {
        let h = hash_raw(b"x");
        let hex = h.to_hex();
        assert_eq!(hex.len(), 64);
        assert!(hex
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)));
        assert_eq!(h.short(), hex.chars().take(8).collect::<String>());
    }
}
