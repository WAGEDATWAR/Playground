//! A tiny flag parser shared by the `pg` subcommands.
//!
//! Flags are `--name value` (may repeat), `--switch` (no value), or `--name [value]` (optional value, taken
//! only if the next token is not itself a flag). Anything else is a positional argument.

use std::collections::BTreeMap;

pub struct Spec<'a> {
    /// Flags that require a value.
    pub values: &'a [&'a str],
    /// Flags with no value.
    pub switches: &'a [&'a str],
    /// Flags whose value is optional.
    pub optional: &'a [&'a str],
}

#[derive(Debug, Default)]
pub struct Parsed {
    pub positional: Vec<String>,
    flags: BTreeMap<String, Vec<Option<String>>>,
}

impl Parsed {
    /// The last value given for `name`.
    pub fn one(&self, name: &str) -> Option<&str> {
        self.flags
            .get(name)
            .and_then(|v| v.last())
            .and_then(|v| v.as_deref())
    }

    /// Every value given for `name`, in order.
    pub fn all(&self, name: &str) -> Vec<&str> {
        self.flags
            .get(name)
            .map(|v| v.iter().filter_map(|x| x.as_deref()).collect())
            .unwrap_or_default()
    }

    /// Whether the flag appeared at all (with or without a value).
    pub fn has(&self, name: &str) -> bool {
        self.flags.contains_key(name)
    }

    /// Parses `name`'s value as `T`.
    pub fn parse<T: std::str::FromStr>(&self, name: &str) -> Result<Option<T>, String>
    where
        T::Err: std::fmt::Display,
    {
        self.one(name)
            .map(|v| v.parse::<T>().map_err(|e| format!("--{name} '{v}': {e}")))
            .transpose()
    }
}

pub fn parse(args: &[String], spec: &Spec<'_>) -> Result<Parsed, String> {
    let mut out = Parsed::default();
    let mut i = 0;
    while i < args.len() {
        let Some(arg) = args.get(i) else { break };
        if let Some(name) = arg.strip_prefix("--") {
            if spec.switches.contains(&name) {
                out.flags.entry(name.to_owned()).or_default().push(None);
            } else if spec.values.contains(&name) {
                i += 1;
                let v = args
                    .get(i)
                    .ok_or_else(|| format!("--{name} needs a value"))?;
                out.flags
                    .entry(name.to_owned())
                    .or_default()
                    .push(Some(v.clone()));
            } else if spec.optional.contains(&name) {
                let next = args.get(i + 1).filter(|n| !n.starts_with("--"));
                if let Some(v) = next {
                    out.flags
                        .entry(name.to_owned())
                        .or_default()
                        .push(Some(v.clone()));
                    i += 1;
                } else {
                    out.flags.entry(name.to_owned()).or_default().push(None);
                }
            } else {
                return Err(format!("unknown option '--{name}'"));
            }
        } else {
            out.positional.push(arg.clone());
        }
        i += 1;
    }
    Ok(out)
}

/// Parses `WxH` or `WxH:style`.
pub fn parse_size(text: &str) -> Result<(i32, i32, Option<u8>), String> {
    let (dims, style) = match text.split_once(':') {
        Some((d, s)) => (
            d,
            Some(s.parse::<u8>().map_err(|e| format!("style '{s}': {e}"))?),
        ),
        None => (text, None),
    };
    let (w, h) = dims
        .split_once('x')
        .ok_or_else(|| format!("expected WxH (for example 64x48), got '{text}'"))?;
    let w = w.parse::<i32>().map_err(|e| format!("width '{w}': {e}"))?;
    let h = h.parse::<i32>().map_err(|e| format!("height '{h}': {e}"))?;
    Ok((w, h, style))
}

/// Parses `x,y`.
pub fn parse_tile(text: &str) -> Result<(i32, i32), String> {
    let (x, y) = text
        .split_once(',')
        .ok_or_else(|| format!("expected x,y, got '{text}'"))?;
    Ok((
        x.trim()
            .parse::<i32>()
            .map_err(|e| format!("x '{x}': {e}"))?,
        y.trim()
            .parse::<i32>()
            .map_err(|e| format!("y '{y}': {e}"))?,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(s: &[&str]) -> Vec<String> {
        s.iter().map(|x| (*x).to_owned()).collect()
    }

    const SPEC: Spec<'static> = Spec {
        values: &["seed", "nudge"],
        switches: &["pretty"],
        optional: &["events"],
    };

    #[test]
    fn values_switches_and_positionals() {
        let p = parse(
            &v(&[
                "a.json", "--seed", "x", "--pretty", "--nudge", "1:2", "--nudge", "3:4", "b.json",
            ]),
            &SPEC,
        )
        .unwrap();
        assert_eq!(p.positional, ["a.json", "b.json"]);
        assert_eq!(p.one("seed"), Some("x"));
        assert_eq!(p.all("nudge"), ["1:2", "3:4"]);
        assert!(p.has("pretty") && !p.has("events"));
    }

    #[test]
    fn optional_values_are_taken_only_when_present() {
        let with = parse(&v(&["--events", "move.", "--pretty"]), &SPEC).unwrap();
        assert_eq!(with.one("events"), Some("move."));
        let without = parse(&v(&["--events", "--pretty"]), &SPEC).unwrap();
        assert!(without.has("events") && without.one("events").is_none());
        let at_end = parse(&v(&["--events"]), &SPEC).unwrap();
        assert!(at_end.has("events"));
    }

    #[test]
    fn errors_are_specific() {
        assert!(parse(&v(&["--nope"]), &SPEC)
            .unwrap_err()
            .contains("unknown option '--nope'"));
        assert!(parse(&v(&["--seed"]), &SPEC)
            .unwrap_err()
            .contains("needs a value"));
    }

    #[test]
    fn typed_parsing() {
        let p = parse(&v(&["--seed", "12"]), &SPEC).unwrap();
        assert_eq!(p.parse::<u32>("seed").unwrap(), Some(12));
        assert!(p.parse::<u32>("nudge").unwrap().is_none());
        let bad = parse(&v(&["--seed", "x"]), &SPEC).unwrap();
        assert!(bad.parse::<u32>("seed").is_err());
    }

    #[test]
    fn sizes_and_tiles() {
        assert_eq!(parse_size("64x48").unwrap(), (64, 48, None));
        assert_eq!(parse_size("64x48:1").unwrap(), (64, 48, Some(1)));
        assert!(
            parse_size("64").is_err() && parse_size("axb").is_err() && parse_size("1x2:z").is_err()
        );
        assert_eq!(parse_tile("3,4").unwrap(), (3, 4));
        assert_eq!(parse_tile("-1, 2").unwrap(), (-1, 2));
        assert!(parse_tile("3").is_err());
    }
}
