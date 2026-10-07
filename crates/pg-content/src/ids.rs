//! Validated identifier types for content (Blueprint §4.1).
//!
//! `TemplateId` is a dotted lower-case slug (`furniture.drawer`, `occupation.barista`). Pack ids,
//! tags and component names use the same slug grammar. Validation happens once, at construction, so
//! the rest of the code can treat these as trusted.

use pg_canon::{Canon, ToCanon};
use std::fmt;
use std::str::FromStr;

/// An identifier failed validation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IdError {
    pub kind: &'static str,
    pub text: String,
    pub reason: String,
}

impl fmt::Display for IdError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "invalid {} '{}': {}", self.kind, self.text, self.reason)
    }
}

impl std::error::Error for IdError {}

const MAX_SEGMENT_LEN: usize = 32;

fn check_segment(segment: &str) -> Result<(), &'static str> {
    let mut chars = segment.chars();
    match chars.next() {
        None => return Err("empty segment"),
        Some(c) if c.is_ascii_lowercase() => {}
        Some(_) => return Err("each segment must start with a lowercase letter a-z"),
    }
    if segment.len() > MAX_SEGMENT_LEN {
        return Err("a segment may be at most 32 characters");
    }
    if !chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_') {
        return Err("only a-z, 0-9 and _ are allowed");
    }
    Ok(())
}

fn check_dotted(
    kind: &'static str,
    text: &str,
    min_segments: usize,
    max_segments: usize,
    max_len: usize,
) -> Result<(), IdError> {
    let fail = |reason: &str| IdError {
        kind,
        text: text.to_owned(),
        reason: reason.to_owned(),
    };
    if text.len() > max_len {
        return Err(fail(&format!("longer than {max_len} characters")));
    }
    let count = text.split('.').count();
    if count < min_segments || count > max_segments {
        return Err(fail(&format!(
            "must have {min_segments} to {max_segments} dot-separated segments"
        )));
    }
    for segment in text.split('.') {
        check_segment(segment).map_err(fail)?;
    }
    Ok(())
}

macro_rules! slug_type {
    ($(#[$meta:meta])* $name:ident, $kind:expr, $min:expr, $max:expr, $maxlen:expr) => {
        $(#[$meta])*
        #[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(String);

        impl $name {
            pub fn new(text: &str) -> Result<$name, IdError> {
                check_dotted($kind, text, $min, $max, $maxlen)?;
                Ok($name(text.to_owned()))
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }

            /// The dot-separated segments.
            pub fn segments(&self) -> impl Iterator<Item = &str> {
                self.0.split('.')
            }
        }

        impl FromStr for $name {
            type Err = IdError;
            fn from_str(s: &str) -> Result<$name, IdError> {
                $name::new(s)
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }

        impl AsRef<str> for $name {
            fn as_ref(&self) -> &str {
                &self.0
            }
        }

        impl ToCanon for $name {
            fn to_canon(&self) -> Canon {
                Canon::Str(self.0.clone())
            }
        }
    };
}

slug_type!(
    /// A content pack id: one segment, for example `base` or `coffee_shop`.
    PackId, "pack id", 1, 1, 32
);
slug_type!(
    /// A template tag: one segment, for example `furniture`.
    Tag, "tag", 1, 1, 32
);
slug_type!(
    /// A template id: two to six segments, for example `furniture.drawer`.
    TemplateId, "template id", 2, 6, 96
);
slug_type!(
    /// A component name: one to four segments. Built-ins are single segment (`physical`); pack
    /// components are `<pack>.<name>`.
    ComponentName, "component name", 1, 4, 64
);

slug_type!(
    /// An action id: one or two segments. Built-ins are single segment (`move_to`); pack actions are
    /// `<pack>.<name>`.
    ActionId, "action id", 1, 2, 64
);

impl TemplateId {
    /// The root every inheritance chain must end at.
    pub const ROOT: &'static str = "base.object";

    pub fn is_root(&self) -> bool {
        self.0 == TemplateId::ROOT
    }

    /// Whether the id is namespaced by `pack` (its first segment equals the pack id).
    pub fn in_namespace(&self, pack: &PackId) -> bool {
        self.segments().next() == Some(pack.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_ids_round_trip() {
        for t in [
            "base.object",
            "furniture.drawer",
            "occupation.barista",
            "a.b",
            "coffee_shop.machine2",
        ] {
            assert_eq!(t.parse::<TemplateId>().unwrap().to_string(), t);
        }
        assert!("base".parse::<PackId>().is_ok());
        assert!("coffee_shop".parse::<PackId>().is_ok());
        assert!("physical".parse::<ComponentName>().is_ok());
        assert!("coffee_shop.caffeine".parse::<ComponentName>().is_ok());
        assert!("furniture".parse::<Tag>().is_ok());
    }

    #[test]
    fn invalid_ids_are_rejected_with_a_reason() {
        for bad in [
            "",
            ".",
            "a.",
            ".a",
            "a..b",
            "Base.object",
            "base.Object",
            "base object",
            "base-object",
            "1a.b",
            "a.1b",
            "_a.b",
            "a.b!",
            "é.b",
        ] {
            assert!(bad.parse::<TemplateId>().is_err(), "{bad:?}");
        }
        assert!(
            "single".parse::<TemplateId>().is_err(),
            "template ids need at least two segments"
        );
        assert!(
            "a.b.c.d.e.f.g".parse::<TemplateId>().is_err(),
            "too many segments"
        );
        assert!("a.b".repeat(40).parse::<TemplateId>().is_err());
        assert!(
            "two.parts".parse::<PackId>().is_err(),
            "pack ids are one segment"
        );
        assert!("two.parts".parse::<Tag>().is_err());
        let long = "x".repeat(33);
        assert!(long.parse::<PackId>().is_err());
        let e = "Bad".parse::<PackId>().unwrap_err();
        assert!(e.to_string().contains("invalid pack id 'Bad'"), "{e}");
    }

    #[test]
    fn namespace_and_root_helpers() {
        let id: TemplateId = "coffee_shop.machine".parse().unwrap();
        assert!(id.in_namespace(&"coffee_shop".parse().unwrap()));
        assert!(!id.in_namespace(&"base".parse().unwrap()));
        assert!("base.object".parse::<TemplateId>().unwrap().is_root());
        assert!(!"base.item".parse::<TemplateId>().unwrap().is_root());
        // A pack named like another pack's prefix must not match by substring.
        let id: TemplateId = "coffee_shop2.machine".parse().unwrap();
        assert!(!id.in_namespace(&"coffee_shop".parse().unwrap()));
    }

    #[test]
    fn ids_order_lexicographically_and_hash_canonically() {
        let mut v: Vec<TemplateId> = ["b.x", "a.y", "a.x"]
            .iter()
            .map(|s| s.parse().unwrap())
            .collect();
        v.sort();
        assert_eq!(
            v.iter().map(ToString::to_string).collect::<Vec<_>>(),
            ["a.x", "a.y", "b.x"]
        );
        assert_eq!(v[0].to_canon(), Canon::str("a.x"));
    }
}
