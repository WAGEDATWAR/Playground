//! Entity identifiers (Blueprint §4.1).
//!
//! An [`EntityId`] is a `(kind, counter)` pair. Counters are per world and per kind, are only
//! incremented by the core ([`IdCounters::allocate`]) and are **never reused**. The textual form is
//! `<prefix>_<counter in lowercase base36>`, for example `pawn_1a` or `obj_3f2`.
//!
//! Ordering is by `(kind, counter)`, which is the order every table iterates in. It is *not*
//! the lexicographic order of the textual form.

use std::collections::BTreeMap;
use std::fmt;
use std::str::FromStr;

/// What sort of thing an [`EntityId`] names. The `u8` value is part of the ordering and of the
/// canonical hash input, so existing values must never be renumbered.
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u8)]
pub enum Kind {
    Map = 1,
    Pawn = 2,
    Household = 3,
    Object = 4,
    Plot = 5,
    Building = 6,
    Commitment = 7,
    Reservation = 8,
    Conversation = 9,
}

impl Kind {
    /// Every kind, in ascending order.
    pub const ALL: [Kind; 9] = [
        Kind::Map,
        Kind::Pawn,
        Kind::Household,
        Kind::Object,
        Kind::Plot,
        Kind::Building,
        Kind::Commitment,
        Kind::Reservation,
        Kind::Conversation,
    ];

    /// The textual prefix used in the id's display form.
    pub const fn prefix(self) -> &'static str {
        match self {
            Kind::Map => "map",
            Kind::Pawn => "pawn",
            Kind::Household => "hh",
            Kind::Object => "obj",
            Kind::Plot => "plot",
            Kind::Building => "bld",
            Kind::Commitment => "cmt",
            Kind::Reservation => "res",
            Kind::Conversation => "conv",
        }
    }

    /// Inverse of [`Kind::prefix`].
    pub fn from_prefix(prefix: &str) -> Option<Kind> {
        Kind::ALL.into_iter().find(|k| k.prefix() == prefix)
    }
}

/// A stable identifier for a world entity.
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct EntityId {
    kind: Kind,
    n: u32,
}

impl EntityId {
    /// Builds an id directly. Production code allocates through [`IdCounters`]; this exists for
    /// tests, fixtures and import remapping.
    pub const fn new(kind: Kind, n: u32) -> EntityId {
        EntityId { kind, n }
    }

    pub const fn kind(self) -> Kind {
        self.kind
    }

    /// The per-kind counter value.
    pub const fn counter(self) -> u32 {
        self.n
    }
}

const BASE36: &[u8; 36] = b"0123456789abcdefghijklmnopqrstuvwxyz";

fn write_base36(mut n: u32, out: &mut String) {
    if n == 0 {
        out.push('0');
        return;
    }
    let mut digits = [0u8; 7]; // u32::MAX is 7 digits in base 36
    let mut len = 0usize;
    while n > 0 {
        let d = (n % 36) as usize;
        if let (Some(slot), Some(&c)) = (digits.get_mut(len), BASE36.get(d)) {
            *slot = c;
        }
        len += 1;
        n /= 36;
    }
    for &c in digits.iter().take(len).rev() {
        out.push(char::from(c));
    }
}

impl fmt::Display for EntityId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut s = String::with_capacity(12);
        s.push_str(self.kind.prefix());
        s.push('_');
        write_base36(self.n, &mut s);
        f.write_str(&s)
    }
}

/// Why an id string could not be parsed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ParseIdError {
    /// No `_` separator.
    Malformed,
    /// The prefix is not a known [`Kind`].
    UnknownKind(String),
    /// The counter is empty, not lowercase base36, too large, or has leading zeros.
    BadCounter(String),
}

impl fmt::Display for ParseIdError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ParseIdError::Malformed => f.write_str("expected <kind>_<counter>"),
            ParseIdError::UnknownKind(k) => write!(f, "unknown entity kind '{k}'"),
            ParseIdError::BadCounter(c) => write!(f, "bad entity counter '{c}'"),
        }
    }
}

impl std::error::Error for ParseIdError {}

impl FromStr for EntityId {
    type Err = ParseIdError;

    /// Parses only the canonical form: re-formatting the parsed id must reproduce the input, so
    /// `pawn_01`, `pawn_1A` and `pawn_+1` are all rejected.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let (prefix, counter) = s.split_once('_').ok_or(ParseIdError::Malformed)?;
        let kind = Kind::from_prefix(prefix)
            .ok_or_else(|| ParseIdError::UnknownKind(prefix.to_owned()))?;
        let bad = || ParseIdError::BadCounter(counter.to_owned());
        if counter.is_empty()
            || !counter
                .bytes()
                .all(|b| b.is_ascii_digit() || b.is_ascii_lowercase())
        {
            return Err(bad());
        }
        let n = u32::from_str_radix(counter, 36).map_err(|_| bad())?;
        let id = EntityId { kind, n };
        if id.to_string() == s {
            Ok(id)
        } else {
            Err(bad())
        }
    }
}

/// Returned when a kind has handed out every counter value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IdsExhausted(pub Kind);

impl fmt::Display for IdsExhausted {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ids exhausted for kind '{}'", self.0.prefix())
    }
}

impl std::error::Error for IdsExhausted {}

/// Per-world, per-kind id counters (`WorldState.id_counters`). Counters start at 1 and only grow.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct IdCounters {
    next: BTreeMap<Kind, u32>,
}

impl IdCounters {
    pub fn new() -> IdCounters {
        IdCounters::default()
    }

    /// Hands out the next id of `kind`. The id is never handed out again.
    pub fn allocate(&mut self, kind: Kind) -> Result<EntityId, IdsExhausted> {
        let n = self.next.get(&kind).copied().unwrap_or(1);
        let following = n.checked_add(1).ok_or(IdsExhausted(kind))?;
        self.next.insert(kind, following);
        Ok(EntityId { kind, n })
    }

    /// The counter value the next allocation of `kind` would use.
    pub fn peek(&self, kind: Kind) -> u32 {
        self.next.get(&kind).copied().unwrap_or(1)
    }

    /// Counters in ascending kind order, for canonical serialization.
    pub fn iter(&self) -> impl Iterator<Item = (Kind, u32)> + '_ {
        self.next.iter().map(|(k, n)| (*k, *n))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn display_matches_spec_examples() {
        assert_eq!(
            EntityId::new(Kind::Pawn, 10 * 36 + 10).to_string(),
            "pawn_aa"
        );
        assert_eq!(EntityId::new(Kind::Pawn, 36 + 10).to_string(), "pawn_1a");
        assert_eq!(
            EntityId::new(Kind::Object, 3 * 1296 + 15 * 36 + 2).to_string(),
            "obj_3f2"
        );
        assert_eq!(EntityId::new(Kind::Plot, 9).to_string(), "plot_9");
        assert_eq!(EntityId::new(Kind::Map, 0).to_string(), "map_0");
    }

    #[test]
    fn parse_round_trips_every_kind() {
        for kind in Kind::ALL {
            for n in [0, 1, 35, 36, 1295, 1296, u32::MAX] {
                let id = EntityId::new(kind, n);
                assert_eq!(id.to_string().parse::<EntityId>(), Ok(id));
            }
        }
    }

    #[test]
    fn parse_rejects_non_canonical_and_garbage() {
        for bad in [
            "",
            "pawn",
            "pawn_",
            "pawn_01",
            "pawn_1A",
            "pawn_+1",
            "pawn_-1",
            "dog_1",
            "pawn_zzzzzzzz",
            "_1",
            "pawn__1",
            "pawn_1 ",
        ] {
            assert!(
                bad.parse::<EntityId>().is_err(),
                "{bad:?} should be rejected"
            );
        }
    }

    #[test]
    fn ordering_is_kind_then_counter() {
        let a = EntityId::new(Kind::Map, 99);
        let b = EntityId::new(Kind::Pawn, 1);
        let c = EntityId::new(Kind::Pawn, 2);
        assert!(a < b && b < c);
        // Textual order differs from id order: "pawn_a" (10) sorts after "pawn_1z" textually only
        // by coincidence of digits, so we never rely on it.
        assert!(EntityId::new(Kind::Pawn, 36) > EntityId::new(Kind::Pawn, 35));
    }

    #[test]
    fn counters_start_at_one_never_repeat_and_are_per_kind() {
        let mut c = IdCounters::new();
        assert_eq!(c.peek(Kind::Pawn), 1);
        let p1 = c.allocate(Kind::Pawn).unwrap();
        let p2 = c.allocate(Kind::Pawn).unwrap();
        let o1 = c.allocate(Kind::Object).unwrap();
        assert_eq!((p1.counter(), p2.counter(), o1.counter()), (1, 2, 1));
        assert_ne!(p1, p2);
        assert_eq!(c.peek(Kind::Pawn), 3);
    }

    #[test]
    fn counters_report_exhaustion_instead_of_wrapping() {
        let mut c = IdCounters::new();
        c.next.insert(Kind::Plot, u32::MAX);
        assert_eq!(c.allocate(Kind::Plot), Err(IdsExhausted(Kind::Plot)));
        // The failed allocation must not have changed anything.
        assert_eq!(c.peek(Kind::Plot), u32::MAX);
    }

    proptest! {
        #[test]
        fn any_id_round_trips(kind_idx in 0usize..9, n in any::<u32>()) {
            let kind = Kind::ALL[kind_idx];
            let id = EntityId::new(kind, n);
            prop_assert_eq!(id.to_string().parse::<EntityId>(), Ok(id));
        }

        #[test]
        fn distinct_ids_have_distinct_text(k1 in 0usize..9, n1 in any::<u32>(), k2 in 0usize..9, n2 in any::<u32>()) {
            let a = EntityId::new(Kind::ALL[k1], n1);
            let b = EntityId::new(Kind::ALL[k2], n2);
            prop_assert_eq!(a == b, a.to_string() == b.to_string());
        }
    }
}
