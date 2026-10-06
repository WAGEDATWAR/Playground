//! Integer-only numerics (Blueprint §4.6, §5.2): bounded domain newtypes and specified rounding.
//!
//! Authoritative state never contains floating point. Fractional quantities are fixed-point with a
//! documented scale (needs 0..=1000, rates in permille). Division truncates toward zero (Rust's `/`);
//! the named [`round_half_up_div`] rounds halves toward positive infinity.

use std::fmt;

/// A value fell outside a bounded type's range.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OutOfRange {
    pub value: i32,
    pub min: i32,
    pub max: i32,
}

impl fmt::Display for OutOfRange {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} is outside {}..={}", self.value, self.min, self.max)
    }
}

impl std::error::Error for OutOfRange {}

macro_rules! bounded_int {
    ($(#[$meta:meta])* $name:ident, $min:expr, $max:expr) => {
        $(#[$meta])*
        #[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(i32);

        impl $name {
            pub const MIN: i32 = $min;
            pub const MAX: i32 = $max;

            /// Checked constructor.
            pub const fn new(value: i32) -> Result<Self, OutOfRange> {
                if value >= Self::MIN && value <= Self::MAX {
                    Ok(Self(value))
                } else {
                    Err(OutOfRange { value, min: Self::MIN, max: Self::MAX })
                }
            }

            /// Clamps into range instead of failing.
            pub const fn saturating(value: i32) -> Self {
                if value < Self::MIN {
                    Self(Self::MIN)
                } else if value > Self::MAX {
                    Self(Self::MAX)
                } else {
                    Self(value)
                }
            }

            pub const fn get(self) -> i32 {
                self.0
            }

            /// Adds `delta`, clamping to the type's range (no overflow, no panic).
            pub const fn saturating_add(self, delta: i32) -> Self {
                Self::saturating(self.0.saturating_add(delta))
            }

            /// Adds `delta`, or `None` if the result would leave the range.
            pub const fn checked_add(self, delta: i32) -> Option<Self> {
                match self.0.checked_add(delta) {
                    Some(v) => match Self::new(v) {
                        Ok(s) => Some(s),
                        Err(_) => None,
                    },
                    None => None,
                }
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.fmt(f)
            }
        }
    };
}

bounded_int!(
    /// A rate or probability in thousandths: `0..=1000`.
    Permille, 0, 1000
);
bounded_int!(
    /// A need level: `0..=1000`, where 1000 is fully satisfied.
    NeedValue, 0, 1000
);
bounded_int!(
    /// A relationship affinity: `-1000..=1000`.
    Affinity, -1000, 1000
);

/// `n / d` rounded to nearest, with exact halves rounded toward positive infinity.
/// `-5 / 2 = -2.5 -> -2`, `5 / 2 = 2.5 -> 3`. Returns `None` if `d <= 0` or on overflow.
pub fn round_half_up_div(n: i64, d: i64) -> Option<i64> {
    if d <= 0 {
        return None;
    }
    let numerator = n.checked_mul(2)?.checked_add(d)?;
    Some(numerator.div_euclid(d.checked_mul(2)?))
}

/// `a * b / c` truncating toward zero, or `None` on overflow or `c == 0`.
pub fn muldiv_trunc(a: i64, b: i64, c: i64) -> Option<i64> {
    a.checked_mul(b)?.checked_div(c)
}

/// Scales `value` by a permille factor, rounding half up. The result never exceeds `|value|`.
pub fn scale_permille(value: i32, p: Permille) -> i32 {
    let scaled = round_half_up_div(i64::from(value) * i64::from(p.get()), 1000).unwrap_or(0);
    i32::try_from(scaled).unwrap_or(if scaled < 0 { i32::MIN } else { i32::MAX })
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn bounds_are_enforced_by_new() {
        assert!(Permille::new(0).is_ok());
        assert!(Permille::new(1000).is_ok());
        assert_eq!(
            Permille::new(1001),
            Err(OutOfRange {
                value: 1001,
                min: 0,
                max: 1000
            })
        );
        assert!(Permille::new(-1).is_err());
        assert!(Affinity::new(-1000).is_ok());
        assert!(Affinity::new(-1001).is_err());
    }

    #[test]
    fn saturating_clamps() {
        assert_eq!(NeedValue::saturating(5000).get(), 1000);
        assert_eq!(NeedValue::saturating(-5).get(), 0);
        assert_eq!(Affinity::saturating(i32::MIN).get(), -1000);
        assert_eq!(
            NeedValue::saturating(500).saturating_add(i32::MAX).get(),
            1000
        );
        assert_eq!(NeedValue::saturating(500).saturating_add(i32::MIN).get(), 0);
    }

    #[test]
    fn checked_add_reports_range_exit() {
        let n = NeedValue::new(990).unwrap();
        assert_eq!(n.checked_add(10), NeedValue::new(1000).ok());
        assert_eq!(n.checked_add(11), None);
        assert_eq!(n.checked_add(i32::MAX), None);
    }

    #[test]
    fn round_half_up_table() {
        let cases: [(i64, i64, i64); 10] = [
            (5, 2, 3),
            (-5, 2, -2),
            (4, 2, 2),
            (-4, 2, -2),
            (1, 3, 0),
            (2, 3, 1),
            (-1, 3, 0),
            (-2, 3, -1),
            (0, 7, 0),
            (7, 1, 7),
        ];
        for (n, d, want) in cases {
            assert_eq!(round_half_up_div(n, d), Some(want), "{n}/{d}");
        }
        assert_eq!(round_half_up_div(1, 0), None);
        assert_eq!(round_half_up_div(1, -3), None);
        assert_eq!(round_half_up_div(i64::MAX, 2), None);
    }

    #[test]
    fn division_truncates_toward_zero() {
        assert_eq!(muldiv_trunc(7, 1, 2), Some(3));
        assert_eq!(muldiv_trunc(-7, 1, 2), Some(-3));
        assert_eq!(muldiv_trunc(1, 1, 0), None);
        assert_eq!(muldiv_trunc(i64::MAX, 2, 1), None);
    }

    #[test]
    fn scale_permille_examples() {
        let p = |v| Permille::new(v).unwrap();
        assert_eq!(scale_permille(1000, p(500)), 500);
        assert_eq!(scale_permille(5, p(500)), 3); // 2.5 -> 3
        assert_eq!(scale_permille(-5, p(500)), -2); // -2.5 -> -2
        assert_eq!(scale_permille(i32::MAX, p(1000)), i32::MAX);
        assert_eq!(scale_permille(i32::MIN, p(1000)), i32::MIN);
        assert_eq!(scale_permille(12345, p(0)), 0);
    }

    proptest! {
        #[test]
        fn saturating_always_in_range(v in any::<i32>(), d in any::<i32>()) {
            let n = NeedValue::saturating(v).saturating_add(d).get();
            prop_assert!((NeedValue::MIN..=NeedValue::MAX).contains(&n));
        }

        #[test]
        fn round_half_up_is_within_half_of_the_true_quotient(n in -1_000_000i64..1_000_000, d in 1i64..10_000) {
            let r = round_half_up_div(n, d).unwrap();
            // |r - n/d| <= 1/2  <=>  |2*d*r - 2*n| <= d
            prop_assert!((2 * d * r - 2 * n).abs() <= d);
        }

        #[test]
        fn scale_permille_never_grows_magnitude(v in any::<i32>(), p in 0i32..=1000) {
            let out = scale_permille(v, Permille::new(p).unwrap());
            prop_assert!(i64::from(out).abs() <= i64::from(v).abs());
        }
    }
}
