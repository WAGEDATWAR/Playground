//! Canonical serialization (Blueprint §5.4).
//!
//! The canonical form of a value is JSON with **sorted keys, no whitespace and integers only**:
//! there is no way to express a float, so authoritative state cannot smuggle one into a hash.
//! Keys sort by their UTF-8 bytes. Strings escape `"`, `\`, and control characters (< 0x20) and are
//! otherwise written as raw UTF-8. Two equal values always have byte-identical canonical forms.

use crate::id::{EntityId, IdCounters};
use crate::table::Table;
use std::collections::BTreeMap;

/// A tree of canonical values.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Canon {
    Null,
    Bool(bool),
    /// `i128` so that every `i64` and `u64` fits exactly.
    Int(i128),
    Str(String),
    List(Vec<Canon>),
    Map(BTreeMap<String, Canon>),
}

impl Canon {
    pub fn str(s: impl Into<String>) -> Canon {
        Canon::Str(s.into())
    }

    /// Builds a map from `(key, value)` pairs. Later duplicates replace earlier ones.
    pub fn map<K: Into<String>>(pairs: impl IntoIterator<Item = (K, Canon)>) -> Canon {
        Canon::Map(pairs.into_iter().map(|(k, v)| (k.into(), v)).collect())
    }

    /// The value under `key`, if this is a map that has it.
    pub fn get(&self, key: &str) -> Option<&Canon> {
        match self {
            Canon::Map(m) => m.get(key),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Canon::Str(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Canon::Bool(b) => Some(*b),
            _ => None,
        }
    }

    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Canon::Int(v) => i64::try_from(*v).ok(),
            _ => None,
        }
    }

    pub fn as_u64(&self) -> Option<u64> {
        match self {
            Canon::Int(v) => u64::try_from(*v).ok(),
            _ => None,
        }
    }

    pub fn as_list(&self) -> Option<&[Canon]> {
        match self {
            Canon::List(l) => Some(l),
            _ => None,
        }
    }

    /// The canonical text.
    pub fn to_canonical_string(&self) -> String {
        let mut out = String::new();
        self.write(&mut out);
        out
    }

    /// Appends the canonical text to `out`.
    pub fn write(&self, out: &mut String) {
        match self {
            Canon::Null => out.push_str("null"),
            Canon::Bool(true) => out.push_str("true"),
            Canon::Bool(false) => out.push_str("false"),
            Canon::Int(v) => out.push_str(&v.to_string()),
            Canon::Str(s) => write_string(s, out),
            Canon::List(items) => {
                out.push('[');
                for (i, item) in items.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    item.write(out);
                }
                out.push(']');
            }
            Canon::Map(map) => {
                out.push('{');
                for (i, (k, v)) in map.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    write_string(k, out);
                    out.push(':');
                    v.write(out);
                }
                out.push('}');
            }
        }
    }
}

fn write_string(s: &str, out: &mut String) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\u{08}' => out.push_str("\\b"),
            '\u{0c}' => out.push_str("\\f"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                out.push_str("\\u00");
                let v = c as u32;
                out.push(char::from_digit((v >> 4) & 0xF, 16).unwrap_or('0'));
                out.push(char::from_digit(v & 0xF, 16).unwrap_or('0'));
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

/// A canonical value did not have the expected shape when decoding.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CanonError(pub String);

impl CanonError {
    pub fn new(msg: impl Into<String>) -> CanonError {
        CanonError(msg.into())
    }
}

impl std::fmt::Display for CanonError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for CanonError {}

impl Canon {
    /// `map[key]` or an error naming the missing key.
    pub fn field(&self, key: &str) -> Result<&Canon, CanonError> {
        self.get(key)
            .ok_or_else(|| CanonError(format!("missing field '{key}'")))
    }
}

/// Types that have a canonical form.
pub trait ToCanon {
    fn to_canon(&self) -> Canon;
}

macro_rules! int_to_canon {
    ($($t:ty),*) => {$(
        impl ToCanon for $t {
            fn to_canon(&self) -> Canon {
                Canon::Int(i128::from(*self))
            }
        }
    )*};
}
int_to_canon!(i8, i16, i32, i64, u8, u16, u32, u64);

impl ToCanon for bool {
    fn to_canon(&self) -> Canon {
        Canon::Bool(*self)
    }
}

impl ToCanon for str {
    fn to_canon(&self) -> Canon {
        Canon::Str(self.to_owned())
    }
}

impl ToCanon for String {
    fn to_canon(&self) -> Canon {
        Canon::Str(self.clone())
    }
}

impl<T: ToCanon> ToCanon for Option<T> {
    fn to_canon(&self) -> Canon {
        self.as_ref().map_or(Canon::Null, ToCanon::to_canon)
    }
}

impl<T: ToCanon> ToCanon for Vec<T> {
    fn to_canon(&self) -> Canon {
        Canon::List(self.iter().map(ToCanon::to_canon).collect())
    }
}

impl<T: ToCanon> ToCanon for [T] {
    fn to_canon(&self) -> Canon {
        Canon::List(self.iter().map(ToCanon::to_canon).collect())
    }
}

impl ToCanon for EntityId {
    fn to_canon(&self) -> Canon {
        Canon::Str(self.to_string())
    }
}

impl<T: ToCanon> ToCanon for Table<T> {
    /// A map from the id's text to the row. The text form is unique per id, so the canonical form
    /// does not depend on the (separate) iteration order of the table.
    fn to_canon(&self) -> Canon {
        Canon::Map(
            self.iter()
                .map(|(id, row)| (id.to_string(), row.to_canon()))
                .collect(),
        )
    }
}

impl ToCanon for IdCounters {
    fn to_canon(&self) -> Canon {
        Canon::Map(
            self.iter()
                .map(|(kind, n)| (kind.prefix().to_owned(), n.to_canon()))
                .collect(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::id::Kind;

    #[test]
    fn scalars() {
        assert_eq!(Canon::Null.to_canonical_string(), "null");
        assert_eq!(Canon::Bool(true).to_canonical_string(), "true");
        assert_eq!(Canon::Int(-42).to_canonical_string(), "-42");
        assert_eq!(
            Canon::Int(i128::from(u64::MAX)).to_canonical_string(),
            "18446744073709551615"
        );
        assert_eq!(Canon::str("hi").to_canonical_string(), "\"hi\"");
    }

    #[test]
    fn keys_are_sorted_with_no_whitespace() {
        let c = Canon::map([
            ("b", Canon::Int(2)),
            ("a", Canon::Int(1)),
            ("aa", Canon::Int(3)),
        ]);
        assert_eq!(c.to_canonical_string(), r#"{"a":1,"aa":3,"b":2}"#);
    }

    #[test]
    fn construction_order_never_matters() {
        let a = Canon::map([("x", Canon::Int(1)), ("y", Canon::Int(2))]);
        let b = Canon::map([("y", Canon::Int(2)), ("x", Canon::Int(1))]);
        assert_eq!(a.to_canonical_string(), b.to_canonical_string());
    }

    #[test]
    fn string_escaping() {
        let s = "a\"b\\c\nd\te\u{01}f\u{7f}é";
        assert_eq!(
            Canon::str(s).to_canonical_string(),
            "\"a\\\"b\\\\c\\nd\\te\\u0001f\u{7f}é\""
        );
        assert_eq!(
            Canon::str("\u{08}\u{0c}\r").to_canonical_string(),
            r#""\b\f\r""#
        );
    }

    #[test]
    fn nested_structures() {
        let c = Canon::map([
            (
                "list",
                Canon::List(vec![Canon::Int(1), Canon::Null, Canon::Bool(false)]),
            ),
            ("empty", Canon::Map(BTreeMap::new())),
            ("none", Option::<i32>::None.to_canon()),
        ]);
        assert_eq!(
            c.to_canonical_string(),
            r#"{"empty":{},"list":[1,null,false],"none":null}"#
        );
    }

    #[test]
    fn tables_and_counters_serialize_by_id_text() {
        let mut t = Table::new();
        t.insert(EntityId::new(Kind::Pawn, 11), 7i32).unwrap();
        t.insert(EntityId::new(Kind::Pawn, 2), 3i32).unwrap();
        assert_eq!(
            t.to_canon().to_canonical_string(),
            r#"{"pawn_2":3,"pawn_b":7}"#
        );

        let mut c = IdCounters::new();
        c.allocate(Kind::Pawn).unwrap();
        c.allocate(Kind::Pawn).unwrap();
        c.allocate(Kind::Object).unwrap();
        assert_eq!(c.to_canon().to_canonical_string(), r#"{"obj":2,"pawn":3}"#);
    }
}
