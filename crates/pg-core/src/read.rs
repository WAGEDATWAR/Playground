//! A small typed reader over [`Canon`] for decoding saved state (Blueprint §13.4-§13.5).
//!
//! Every accessor reports the path of the value it was reading ("pawns.pawn_3.route.goal"), so a corrupt or
//! hand-edited save fails with a message that points at the problem instead of a generic parse error.
//! Nothing here panics: saves are untrusted input.

use crate::canon::Canon;
use std::fmt;

/// Why a value could not be decoded.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReadError {
    pub path: String,
    pub message: String,
}

impl fmt::Display for ReadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.path.is_empty() {
            write!(f, "{}", self.message)
        } else {
            write!(f, "{}: {}", self.path, self.message)
        }
    }
}

impl std::error::Error for ReadError {}

/// A value together with where it was found.
#[derive(Copy, Clone)]
pub struct Reader<'a> {
    value: &'a Canon,
    path: &'a str,
}

/// Owned path storage so child readers can borrow it.
pub struct Root {
    value: Canon,
}

impl Root {
    pub fn new(value: Canon) -> Root {
        Root { value }
    }

    pub fn reader(&self) -> Reader<'_> {
        Reader {
            value: &self.value,
            path: "",
        }
    }
}

fn join(path: &str, key: &str) -> String {
    if path.is_empty() {
        key.to_owned()
    } else {
        format!("{path}.{key}")
    }
}

/// A decoded child together with its path string (kept alive by the caller).
pub struct Child {
    value: Canon,
    path: String,
}

impl Child {
    pub fn reader(&self) -> Reader<'_> {
        Reader {
            value: &self.value,
            path: &self.path,
        }
    }
}

impl<'a> Reader<'a> {
    pub fn new(value: &'a Canon, path: &'a str) -> Reader<'a> {
        Reader { value, path }
    }

    pub fn value(&self) -> &'a Canon {
        self.value
    }

    pub fn path(&self) -> &str {
        self.path
    }

    pub fn err(&self, message: impl Into<String>) -> ReadError {
        ReadError {
            path: self.path.to_owned(),
            message: message.into(),
        }
    }

    /// The child under `key` as an owned (cloned) reader source. Cloning keeps lifetimes simple; saves are
    /// decoded once per load.
    pub fn child(&self, key: &str) -> Result<Child, ReadError> {
        match self.value.get(key) {
            Some(v) => Ok(Child {
                value: v.clone(),
                path: join(self.path, key),
            }),
            None => Err(self.err(format!("missing field '{key}'"))),
        }
    }

    /// Like [`Reader::child`], but a missing field or `null` is `None`.
    pub fn maybe(&self, key: &str) -> Result<Option<Child>, ReadError> {
        match self.value.get(key) {
            None | Some(Canon::Null) => Ok(None),
            Some(v) => Ok(Some(Child {
                value: v.clone(),
                path: join(self.path, key),
            })),
        }
    }

    pub fn is_null(&self) -> bool {
        matches!(self.value, Canon::Null)
    }

    pub fn str(&self) -> Result<&'a str, ReadError> {
        self.value.as_str().ok_or_else(|| self.err("expected text"))
    }

    pub fn bool(&self) -> Result<bool, ReadError> {
        self.value
            .as_bool()
            .ok_or_else(|| self.err("expected true or false"))
    }

    fn int(&self) -> Result<i128, ReadError> {
        self.value
            .as_i128()
            .ok_or_else(|| self.err("expected an integer"))
    }

    pub fn i64(&self) -> Result<i64, ReadError> {
        i64::try_from(self.int()?).map_err(|_| self.err("integer does not fit i64"))
    }

    pub fn i32(&self) -> Result<i32, ReadError> {
        i32::try_from(self.int()?).map_err(|_| self.err("integer does not fit i32"))
    }

    pub fn u64(&self) -> Result<u64, ReadError> {
        u64::try_from(self.int()?).map_err(|_| self.err("expected a non-negative integer"))
    }

    pub fn u32(&self) -> Result<u32, ReadError> {
        u32::try_from(self.int()?).map_err(|_| self.err("expected an integer 0..=4294967295"))
    }

    pub fn u8(&self) -> Result<u8, ReadError> {
        u8::try_from(self.int()?).map_err(|_| self.err("expected an integer 0..=255"))
    }

    pub fn usize(&self) -> Result<usize, ReadError> {
        usize::try_from(self.int()?).map_err(|_| self.err("expected a non-negative integer"))
    }

    /// A list's items, each with its own path.
    pub fn list(&self) -> Result<Vec<Child>, ReadError> {
        match self.value {
            Canon::List(items) => Ok(items
                .iter()
                .enumerate()
                .map(|(i, v)| Child {
                    value: v.clone(),
                    path: format!("{}[{i}]", self.path),
                })
                .collect()),
            _ => Err(self.err("expected a list")),
        }
    }

    /// A map's entries in key order, each with its own path.
    pub fn entries(&self) -> Result<Vec<(String, Child)>, ReadError> {
        match self.value {
            Canon::Map(m) => Ok(m
                .iter()
                .map(|(k, v)| {
                    (
                        k.clone(),
                        Child {
                            value: v.clone(),
                            path: join(self.path, k),
                        },
                    )
                })
                .collect()),
            _ => Err(self.err("expected an object")),
        }
    }

    /// Fails if the object has a key outside `allowed` (unknown fields are corruption, not extensions).
    pub fn only(&self, allowed: &[&str]) -> Result<(), ReadError> {
        match self.value {
            Canon::Map(m) => match m.keys().find(|k| !allowed.contains(&k.as_str())) {
                Some(k) => Err(self.err(format!("unknown field '{k}'"))),
                None => Ok(()),
            },
            _ => Err(self.err("expected an object")),
        }
    }

    /// Parses text with `FromStr`, reporting the parse error at this path.
    pub fn parse<T: std::str::FromStr>(&self) -> Result<T, ReadError>
    where
        T::Err: fmt::Display,
    {
        self.str()?
            .parse()
            .map_err(|e: T::Err| self.err(e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::canon::json::parse;

    fn root(json: &str) -> Root {
        Root::new(parse(json).unwrap())
    }

    #[test]
    fn paths_point_at_the_problem() {
        let r = root(r#"{"a":{"b":[1,"x"]}}"#);
        let a = r.reader().child("a").unwrap();
        let b = a.reader().child("b").unwrap();
        let items = b.reader().list().unwrap();
        assert_eq!(items[0].reader().u32().unwrap(), 1);
        let e = items[1].reader().u32().unwrap_err();
        assert_eq!(e.path, "a.b[1]");
        assert_eq!(e.to_string(), "a.b[1]: expected an integer");
        let e = a.reader().child("zzz").map(|_| ()).unwrap_err();
        assert_eq!(e.to_string(), "a: missing field 'zzz'");
    }

    #[test]
    fn integer_ranges_are_checked() {
        let r = root(r#"{"big":99999999999,"neg":-1,"ok":7}"#);
        let rd = r.reader();
        assert!(rd.child("big").unwrap().reader().u32().is_err());
        assert!(rd.child("big").unwrap().reader().i64().is_ok());
        assert!(rd.child("neg").unwrap().reader().u64().is_err());
        assert_eq!(rd.child("ok").unwrap().reader().u8().unwrap(), 7);
    }

    #[test]
    fn maybe_treats_null_and_missing_alike_and_only_rejects_strangers() {
        let r = root(r#"{"a":null,"b":1}"#);
        let rd = r.reader();
        assert!(rd.maybe("a").unwrap().is_none());
        assert!(rd.maybe("zz").unwrap().is_none());
        assert!(rd.maybe("b").unwrap().is_some());
        assert!(rd.only(&["a", "b"]).is_ok());
        assert_eq!(rd.only(&["a"]).unwrap_err().message, "unknown field 'b'");
    }

    #[test]
    fn entries_and_parse() {
        let r = root(r#"{"pawn_1":"pawn_1","x":"nope"}"#);
        let entries = r.reader().entries().unwrap();
        assert_eq!(entries.len(), 2);
        let id: crate::id::EntityId = entries[0].1.reader().parse().unwrap();
        assert_eq!(id.to_string(), "pawn_1");
        assert!(entries[1]
            .1
            .reader()
            .parse::<crate::id::EntityId>()
            .is_err());
    }
}
