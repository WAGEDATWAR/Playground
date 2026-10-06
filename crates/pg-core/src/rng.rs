//! Counter-based, stateless randomness (Blueprint §5.1).
//!
//! A draw is a pure function of `(world seed, stream name, keys, counter)`. There is no hidden
//! generator state, so results never depend on the order in which systems ask for them, and adding
//! a new stream or key never perturbs an existing one.
//!
//! The construction is specified here and pinned by test vectors (see [`crate::vectors`]); it uses
//! no platform hasher and no external crate:
//!
//! * [`fnv1a64`] hashes bytes (FNV-1a, 64-bit, published constants);
//! * [`mix64`] is the SplitMix64 finalizer;
//! * [`hash_bytes`] = `mix64(fnv1a64(bytes))`;
//! * [`combine`] folds one 64-bit value into an accumulator, order-sensitively;
//! * a draw is `combine(base, counter)` truncated to its high 32 bits, where `base` folds the seed,
//!   the stream name and each key (with a domain-separating tag per key type).

use crate::id::EntityId;
use crate::num::Permille;

/// The SplitMix64 / golden-ratio increment.
pub const GOLDEN: u64 = 0x9E37_79B9_7F4A_7C15;

/// SplitMix64 output finalizer (a bijection on `u64`).
pub const fn mix64(mut z: u64) -> u64 {
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// The reference SplitMix64 generator. Used only to validate [`mix64`] against published vectors
/// and by dev tools; gameplay code uses [`Rng`].
#[derive(Copy, Clone, Debug)]
pub struct SplitMix64 {
    state: u64,
}

impl SplitMix64 {
    pub const fn new(seed: u64) -> SplitMix64 {
        SplitMix64 { state: seed }
    }

    pub fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(GOLDEN);
        mix64(self.state)
    }
}

/// FNV-1a, 64-bit.
pub const fn fnv1a64(mut bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xCBF2_9CE4_8422_2325;
    while let [b, rest @ ..] = bytes {
        hash ^= *b as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01B3);
        bytes = rest;
    }
    hash
}

/// The project's pinned byte hash: FNV-1a followed by the SplitMix64 finalizer.
pub const fn hash_bytes(bytes: &[u8]) -> u64 {
    mix64(fnv1a64(bytes))
}

/// Folds `x` into `acc`. Not commutative: `combine(combine(a, x), y) != combine(combine(a, y), x)`.
pub const fn combine(acc: u64, x: u64) -> u64 {
    mix64(acc.rotate_left(27) ^ x.wrapping_add(GOLDEN))
}

/// The world seed, reduced to 64 bits.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct Seed(pub u64);

impl Seed {
    /// Derives a seed from user-facing text (for example the seed typed into New World).
    pub fn from_text(text: &str) -> Seed {
        Seed(hash_bytes(text.as_bytes()))
    }
}

/// A stable per-draw-site key (pawn id, day index, …).
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Key<'a> {
    Int(i64),
    Str(&'a str),
    Id(EntityId),
}

impl Key<'_> {
    /// Domain-separated hash: an `Int(5)`, `Str("5")` and an id can never collide by construction.
    fn hash(&self) -> u64 {
        match self {
            Key::Int(v) => combine(hash_bytes(&[1]), *v as u64),
            Key::Str(s) => combine(hash_bytes(&[2]), hash_bytes(s.as_bytes())),
            Key::Id(id) => combine(
                combine(hash_bytes(&[3]), u64::from(id.kind() as u8)),
                u64::from(id.counter()),
            ),
        }
    }
}

/// Well-known stream names (Blueprint §5.1). Pack streams are namespaced `mod.<pack>.<name>`.
pub mod streams {
    pub const WORLDGEN_TERRAIN: &str = "worldgen.terrain";
    pub const WORLDGEN_ROADS: &str = "worldgen.roads";
    pub const WORLDGEN_PLOTS: &str = "worldgen.plots";
    pub const WORLDGEN_PEOPLE: &str = "worldgen.people";
    pub const SCHED_VARIATION: &str = "sched.variation";
    pub const SCHED_TIEBREAK: &str = "sched.tiebreak";
    pub const SOCIAL_TOPIC: &str = "social.topic";
    pub const SOCIAL_OUTCOME: &str = "social.outcome";
    pub const PATH_TIEBREAK: &str = "path.tiebreak";

    /// `event.<category>`.
    pub fn event(category: &str) -> String {
        format!("event.{category}")
    }

    /// `mod.<pack>.<name>`: the only streams scripts may draw from.
    pub fn pack(pack_id: &str, name: &str) -> String {
        format!("mod.{pack_id}.{name}")
    }
}

/// A random stream bound to `(seed, stream, keys)`. Draws are pure functions of a counter.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct Rng {
    base: u64,
}

/// Rejection-sampling attempts before falling back to a (negligibly biased) multiply-shift.
const MAX_ATTEMPTS: u32 = 64;

impl Rng {
    pub fn new(seed: Seed, stream: &str, keys: &[Key<'_>]) -> Rng {
        let mut base = combine(seed.0, hash_bytes(stream.as_bytes()));
        for key in keys {
            base = combine(base, key.hash());
        }
        base = combine(base, keys.len() as u64);
        Rng { base }
    }

    fn draw_attempt(&self, counter: u32, attempt: u32) -> u32 {
        let x = u64::from(counter) | (u64::from(attempt) << 32);
        (combine(self.base, x) >> 32) as u32
    }

    /// The raw 32-bit draw for `counter`.
    pub fn draw(&self, counter: u32) -> u32 {
        self.draw_attempt(counter, 0)
    }

    /// A uniform value in `0..n` (Lemire's method with deterministic redraws), or `None` if `n == 0`.
    pub fn range(&self, counter: u32, n: u32) -> Option<u32> {
        if n == 0 {
            return None;
        }
        let threshold = n.wrapping_neg() % n;
        for attempt in 0..MAX_ATTEMPTS {
            let m = u64::from(self.draw_attempt(counter, attempt)) * u64::from(n);
            if (m as u32) >= threshold {
                return Some((m >> 32) as u32);
            }
        }
        let m = u64::from(self.draw_attempt(counter, MAX_ATTEMPTS)) * u64::from(n);
        Some((m >> 32) as u32)
    }

    /// A uniform integer in `min..=max`, or `None` if `min > max` or the span exceeds `u32::MAX`.
    pub fn int_in(&self, counter: u32, min: i32, max: i32) -> Option<i32> {
        if min > max {
            return None;
        }
        let span = i64::from(max) - i64::from(min) + 1;
        let span = u32::try_from(span).ok()?;
        let offset = self.range(counter, span)?;
        i32::try_from(i64::from(min) + i64::from(offset)).ok()
    }

    /// `true` with probability `p / 1000`. `0` never, `1000` always.
    pub fn chance(&self, counter: u32, p: Permille) -> bool {
        self.range(counter, 1000)
            .is_some_and(|r| r < p.get() as u32)
    }

    /// A uniformly chosen element, or `None` for an empty slice.
    pub fn pick<'a, T>(&self, counter: u32, items: &'a [T]) -> Option<&'a T> {
        let n = u32::try_from(items.len()).ok()?;
        let i = self.range(counter, n)?;
        items.get(i as usize)
    }

    /// Fisher–Yates shuffle driven by this stream. The result depends only on the stream and the
    /// input order, never on anything else.
    pub fn shuffle<T>(&self, items: &mut [T]) {
        for i in (1..items.len()).rev() {
            let (Ok(counter), Ok(n)) = (u32::try_from(i), u32::try_from(i + 1)) else {
                return;
            };
            if let Some(j) = self.range(counter, n) {
                items.swap(i, j as usize);
            }
        }
    }
}

/// One-shot form matching the Blueprint signature: `rand(world_seed, stream, keys, counter) -> u32`.
pub fn rand(seed: Seed, stream: &str, keys: &[Key<'_>], counter: u32) -> u32 {
    Rng::new(seed, stream, keys).draw(counter)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::id::Kind;
    use proptest::prelude::*;
    // Explicit import wins over the two glob imports, which both export an `Rng`.
    use super::Rng;

    fn rng(stream: &str) -> Rng {
        Rng::new(Seed::from_text("town-1"), stream, &[Key::Int(7)])
    }

    #[test]
    fn same_inputs_same_draw() {
        assert_eq!(rng("a").draw(5), rng("a").draw(5));
        assert_eq!(
            rand(Seed(1), "s", &[Key::Str("k")], 3),
            rand(Seed(1), "s", &[Key::Str("k")], 3)
        );
    }

    #[test]
    fn every_input_changes_the_draw() {
        let base = rand(Seed(1), "s", &[Key::Int(1)], 0);
        assert_ne!(base, rand(Seed(2), "s", &[Key::Int(1)], 0), "seed");
        assert_ne!(base, rand(Seed(1), "t", &[Key::Int(1)], 0), "stream");
        assert_ne!(base, rand(Seed(1), "s", &[Key::Int(2)], 0), "key");
        assert_ne!(base, rand(Seed(1), "s", &[Key::Int(1)], 1), "counter");
        assert_ne!(
            base,
            rand(Seed(1), "s", &[Key::Int(1), Key::Int(1)], 0),
            "key count"
        );
    }

    #[test]
    fn key_types_are_domain_separated() {
        let a = rand(Seed(1), "s", &[Key::Int(5)], 0);
        let b = rand(Seed(1), "s", &[Key::Str("5")], 0);
        let c = rand(Seed(1), "s", &[Key::Id(EntityId::new(Kind::Pawn, 5))], 0);
        assert!(a != b && b != c && a != c);
    }

    #[test]
    fn key_order_matters() {
        let a = rand(Seed(1), "s", &[Key::Int(1), Key::Int(2)], 0);
        let b = rand(Seed(1), "s", &[Key::Int(2), Key::Int(1)], 0);
        assert_ne!(a, b);
    }

    #[test]
    fn draws_do_not_depend_on_call_order() {
        let r = rng("order");
        let forward: Vec<u32> = (0..16).map(|c| r.draw(c)).collect();
        let mut backward: Vec<u32> = (0..16).rev().map(|c| r.draw(c)).collect();
        backward.reverse();
        assert_eq!(forward, backward);
    }

    #[test]
    fn range_edge_cases() {
        let r = rng("edge");
        assert_eq!(r.range(0, 0), None);
        assert_eq!(r.range(0, 1), Some(0));
        assert!(r.range(0, u32::MAX).is_some());
        assert_eq!(r.int_in(0, 5, 4), None);
        assert_eq!(
            r.int_in(0, i32::MIN, i32::MAX),
            None,
            "span 2^32 does not fit u32"
        );
        assert_eq!(r.int_in(0, 3, 3), Some(3));
        assert!(r.int_in(0, i32::MIN, i32::MAX - 1).is_some());
    }

    #[test]
    fn chance_extremes() {
        let r = rng("chance");
        for c in 0..200 {
            assert!(!r.chance(c, Permille::new(0).unwrap()));
            assert!(r.chance(c, Permille::new(1000).unwrap()));
        }
    }

    #[test]
    fn chance_is_roughly_calibrated() {
        let r = rng("calibration");
        let p = Permille::new(300).unwrap();
        let hits = (0..20_000u32).filter(|&c| r.chance(c, p)).count();
        // Expect ~6000; 5-sigma is about 330.
        assert!((5600..6400).contains(&hits), "hits = {hits}");
    }

    #[test]
    fn range_is_roughly_uniform() {
        let r = rng("uniform");
        let mut buckets = [0u32; 10];
        for c in 0..50_000u32 {
            if let Some(slot) = r.range(c, 10).and_then(|v| buckets.get_mut(v as usize)) {
                *slot += 1;
            }
        }
        for (i, &b) in buckets.iter().enumerate() {
            assert!((4500..5500).contains(&b), "bucket {i} = {b}");
        }
    }

    #[test]
    fn pick_and_shuffle_on_empty_and_singleton() {
        let r = rng("pick");
        let empty: [u8; 0] = [];
        assert_eq!(r.pick(0, &empty), None);
        assert_eq!(r.pick(0, &[42]), Some(&42));
        let mut one = [1];
        r.shuffle(&mut one);
        assert_eq!(one, [1]);
    }

    proptest! {
        #[test]
        fn range_is_always_below_n(seed in any::<u64>(), counter in any::<u32>(), n in 1u32..) {
            let v = Rng::new(Seed(seed), "p", &[]).range(counter, n).unwrap();
            prop_assert!(v < n);
        }

        #[test]
        fn int_in_stays_in_bounds(seed in any::<u64>(), counter in any::<u32>(), a in any::<i32>(), b in any::<i32>()) {
            let (min, max) = if a <= b { (a, b) } else { (b, a) };
            if let Some(v) = Rng::new(Seed(seed), "p", &[]).int_in(counter, min, max) {
                prop_assert!((min..=max).contains(&v));
            }
        }

        #[test]
        fn shuffle_is_a_permutation(seed in any::<u64>(), len in 0usize..64) {
            let mut v: Vec<usize> = (0..len).collect();
            Rng::new(Seed(seed), "shuf", &[]).shuffle(&mut v);
            let mut sorted = v.clone();
            sorted.sort_unstable();
            prop_assert_eq!(sorted, (0..len).collect::<Vec<_>>());
        }

        #[test]
        fn shuffle_is_deterministic(seed in any::<u64>(), len in 0usize..64) {
            let mut a: Vec<usize> = (0..len).collect();
            let mut b = a.clone();
            Rng::new(Seed(seed), "shuf", &[]).shuffle(&mut a);
            Rng::new(Seed(seed), "shuf", &[]).shuffle(&mut b);
            prop_assert_eq!(a, b);
        }

        #[test]
        fn streams_are_independent_of_unrelated_streams(seed in any::<u64>(), counter in any::<u32>()) {
            // Drawing from another stream first (or not at all) cannot change this stream's draw.
            let target = Rng::new(Seed(seed), "target", &[]);
            let before = target.draw(counter);
            let _ = Rng::new(Seed(seed), "noise", &[]).draw(counter);
            prop_assert_eq!(before, target.draw(counter));
        }
    }
}
