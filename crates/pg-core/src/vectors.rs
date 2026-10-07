//! Published and pinned test vectors for the determinism primitives (Blueprint §5.1, §18).
//!
//! `run()` is the single source of truth: unit tests assert every vector passes, and the developer
//! tool `pg selftest` prints them. **Published** vectors come from the algorithms' authors and
//! validate the implementation independently. **Pinned** vectors are regression locks on this
//! project's own constructions; changing one is a breaking change to every seeded world and must be
//! a deliberate, versioned decision.

use crate::canon::Canon;
use crate::hash::{hash_canon, hash_raw};
use crate::id::{EntityId, Kind};
use crate::rng::{fnv1a64, hash_bytes, rand, Key, Seed, SplitMix64, Stream};

/// Where a vector comes from.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Origin {
    /// From the algorithm's authors; independent of this codebase.
    Published,
    /// Locks this project's own construction against accidental change.
    Pinned,
}

/// The outcome of one vector.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VectorResult {
    pub name: &'static str,
    pub origin: Origin,
    pub expected: String,
    pub actual: String,
}

impl VectorResult {
    pub fn passed(&self) -> bool {
        self.expected == self.actual
    }
}

fn v(name: &'static str, origin: Origin, expected: &str, actual: String) -> VectorResult {
    VectorResult {
        name,
        origin,
        expected: expected.to_owned(),
        actual,
    }
}

fn hex64(x: u64) -> String {
    format!("{x:016x}")
}

/// Runs every vector.
pub fn run() -> Vec<VectorResult> {
    let mut out = Vec::new();
    let p = Origin::Published;
    let k = Origin::Pinned;

    // FNV-1a 64: published by the algorithm's authors.
    out.push(v(
        "fnv1a64(\"\")",
        p,
        "cbf29ce484222325",
        hex64(fnv1a64(b"")),
    ));
    out.push(v(
        "fnv1a64(\"a\")",
        p,
        "af63dc4c8601ec8c",
        hex64(fnv1a64(b"a")),
    ));
    out.push(v(
        "fnv1a64(\"foobar\")",
        p,
        "85944171f73967e8",
        hex64(fnv1a64(b"foobar")),
    ));

    // SplitMix64: reference outputs for seed 0 and seed 1234567.
    let mut sm = SplitMix64::new(0);
    out.push(v(
        "splitmix64(0)[0]",
        p,
        "e220a8397b1dcdaf",
        hex64(sm.next_u64()),
    ));
    out.push(v(
        "splitmix64(0)[1]",
        p,
        "6e789e6aa1b965f4",
        hex64(sm.next_u64()),
    ));
    out.push(v(
        "splitmix64(0)[2]",
        p,
        "06c45d188009454f",
        hex64(sm.next_u64()),
    ));
    let mut sm = SplitMix64::new(1_234_567);
    out.push(v(
        "splitmix64(1234567)[0]",
        p,
        &hex64(6_457_827_717_110_365_317),
        hex64(sm.next_u64()),
    ));
    out.push(v(
        "splitmix64(1234567)[1]",
        p,
        &hex64(3_203_168_211_198_807_973),
        hex64(sm.next_u64()),
    ));
    out.push(v(
        "splitmix64(1234567)[2]",
        p,
        &hex64(9_817_491_932_198_370_423),
        hex64(sm.next_u64()),
    ));

    // Pinned: this project's constructions.
    out.push(v(
        "hash_bytes(\"playground\")",
        k,
        "0e80b6efce3abda8",
        hex64(hash_bytes(b"playground")),
    ));
    out.push(v(
        "Seed::from_text(\"playground\")",
        k,
        "0e80b6efce3abda8",
        hex64(Seed::from_text("playground").0),
    ));
    let seed = Seed::from_text("playground");
    out.push(v(
        "rand(playground, worldgen.terrain, [], 0)",
        k,
        "744eb490",
        format!("{:08x}", rand(seed, Stream::WorldgenTerrain, &[], 0)),
    ));
    out.push(v(
        "rand(playground, sched.variation, [pawn_1a, 7], 3)",
        k,
        "da290f7f",
        format!(
            "{:08x}",
            rand(
                seed,
                Stream::SchedVariation,
                &[Key::Id(EntityId::new(Kind::Pawn, 46)), Key::Int(7)],
                3
            )
        ),
    ));
    out.push(v(
        "rand(playground, mod.coffee.roll, [\"k\"], 99)",
        k,
        "039124ef",
        format!(
            "{:08x}",
            rand(
                seed,
                Stream::Pack {
                    pack: "coffee",
                    name: "roll"
                },
                &[Key::Str("k")],
                99
            )
        ),
    ));

    // BLAKE3: published empty-input vector, plus a pinned canonical-form hash.
    out.push(v(
        "blake3(\"\")",
        p,
        "af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262",
        hash_raw(b"").to_hex(),
    ));
    let sample = Canon::map([
        ("name", Canon::str("pawn_1a")),
        (
            "needs",
            Canon::map([("hunger", Canon::Int(700)), ("energy", Canon::Int(450))]),
        ),
        (
            "flags",
            Canon::List(vec![Canon::Bool(true), Canon::Null, Canon::Int(-3)]),
        ),
    ]);
    out.push(v(
        "canonical text of the sample record",
        k,
        r#"{"flags":[true,null,-3],"name":"pawn_1a","needs":{"energy":450,"hunger":700}}"#,
        sample.to_canonical_string(),
    ));
    out.push(v(
        "blake3(canonical sample record)",
        k,
        "5734b1bf26380a3768c48e308eeacc5eb8a04234f5ac48be866ae6b873c65836",
        hash_canon(&sample).to_hex(),
    ));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_vector_passes() {
        let failures: Vec<_> = run().into_iter().filter(|r| !r.passed()).collect();
        assert!(failures.is_empty(), "failing vectors: {failures:#?}");
    }
}
