//! Keeping secrets out of text (Blueprint §10.2, §17).
//!
//! * [`Secret`] wraps a provider key. It has no `Display`, its `Debug` prints `Secret(***)`, and the text is
//!   only reachable through the deliberately named [`Secret::expose`], so a key cannot reach a log line,
//!   an error message or a `{:?}` by accident.
//! * [`redact`] scrubs text before it is logged, shown or written to a crash report. It removes every
//!   secret it is told about **and** anything that has the shape of a credential (`sk-...`, `AIza...`,
//!   `Bearer ...`, `x-api-key: ...`, `?key=...`), so a key the program has forgotten about, or one a
//!   misbehaving provider echoes back in an error body, is still caught.

use std::fmt;

/// The text that replaces a removed secret.
pub const REDACTED: &str = "[redacted]";

/// A credential. Cloneable, comparable, and impossible to print by accident.
#[derive(Clone, PartialEq, Eq)]
pub struct Secret(String);

impl Secret {
    pub fn new(text: impl Into<String>) -> Secret {
        Secret(text.into())
    }

    /// The secret text. Call this only at the point of use (building a request header); never log it.
    pub fn expose(&self) -> &str {
        &self.0
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Secret(***)")
    }
}

impl Drop for Secret {
    fn drop(&mut self) {
        // Best effort: overwrite the bytes before the allocation is returned.
        // SAFETY-free approach: replace the string content with zeros of the same length.
        let len = self.0.len();
        self.0.clear();
        self.0.extend(std::iter::repeat_n('\0', len));
        self.0.clear();
    }
}

fn is_token_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | '~' | '+' | '/' | '=')
}

/// Length in bytes of the run of token characters starting at `s`.
fn token_len(s: &str) -> usize {
    s.char_indices()
        .find(|(_, c)| !is_token_char(*c))
        .map_or(s.len(), |(i, _)| i)
}

/// Prefixes that mark a credential, with the minimum length of the token after the prefix.
const PREFIXES: [(&str, usize); 3] = [("sk-", 8), ("AIza", 16), ("ghp_", 16)];

/// Header or parameter names whose value is a secret.
const NAMED: [&str; 9] = [
    "authorization",
    "x-api-key",
    "api-key",
    "api_key",
    "apikey",
    "access_token",
    "token",
    "key",
    "secret",
];

fn starts_with_ignore_case(s: &str, prefix: &str) -> bool {
    s.get(..prefix.len())
        .is_some_and(|head| head.eq_ignore_ascii_case(prefix))
}

/// Whether the byte before `i` lets a token start at `i` (start of text or a non-token character).
fn at_boundary(text: &str, i: usize) -> bool {
    text.get(..i)
        .and_then(|head| head.chars().next_back())
        .is_none_or(|c| !c.is_ascii_alphanumeric() && c != '_')
}

/// Removes `secrets` (exact matches) and anything shaped like a credential from `text`.
pub fn redact(text: &str, secrets: &[&Secret]) -> String {
    let mut out = text.to_owned();
    for s in secrets {
        if !s.is_empty() {
            out = out.replace(s.expose(), REDACTED);
        }
    }
    scrub_shapes(&out)
}

/// Like [`redact`] for plain strings the caller knows are secret (for example an environment variable).
pub fn redact_plain(text: &str, secrets: &[&str]) -> String {
    let mut out = text.to_owned();
    for s in secrets {
        if !s.is_empty() {
            out = out.replace(s, REDACTED);
        }
    }
    scrub_shapes(&out)
}

fn scrub_shapes(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < text.len() {
        let rest = &text[i..];
        // 1. Well-known key prefixes.
        if let Some(n) = PREFIXES.iter().find_map(|(p, min)| {
            (rest.starts_with(p) && at_boundary(text, i)).then(|| {
                let t = token_len(&rest[p.len()..]);
                (t >= *min).then_some(p.len() + t)
            })?
        }) {
            out.push_str(REDACTED);
            i += n;
            continue;
        }
        // 2. "Bearer <token>" and "Basic <token>".
        if let Some(scheme) = ["bearer ", "basic "].iter().find(|s| starts_with_ignore_case(rest, s)) {
            let after = &rest[scheme.len()..];
            let t = token_len(after);
            if t >= 8 && at_boundary(text, i) {
                out.push_str(&rest[..scheme.len()]);
                out.push_str(REDACTED);
                i += scheme.len() + t;
                continue;
            }
        }
        // 3. name: value / name=value / "name":"value" for secret-bearing names.
        if at_boundary(text, i) {
            if let Some(name) = NAMED.iter().find(|n| starts_with_ignore_case(rest, n)) {
                let after_name = &rest[name.len()..];
                let after_quote = after_name.strip_prefix('"').unwrap_or(after_name);
                let sep_trim = after_quote.trim_start_matches([' ', '\t']);
                if let Some(value_start) = sep_trim
                    .strip_prefix(':')
                    .or_else(|| sep_trim.strip_prefix('='))
                {
                    let value_start = value_start.trim_start_matches([' ', '\t', '"']);
                    // An auth scheme before the credential ("Bearer xyz") belongs to the value.
                    let scheme = ["bearer ", "basic "]
                        .iter()
                        .find(|s| starts_with_ignore_case(value_start, s))
                        .map_or(0, |s| s.len());
                    let t = scheme + token_len(&value_start[scheme..]);
                    if t >= 6 + scheme {
                        let consumed = rest.len() - value_start.len() + t;
                        out.push_str(&rest[..rest.len() - value_start.len()]);
                        out.push_str(REDACTED);
                        i += consumed;
                        continue;
                    }
                }
            }
        }
        let c = rest.chars().next().unwrap_or(' ');
        out.push(c);
        i += c.len_utf8();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const SENTINEL: &str = "sk-SENTINEL-0123456789abcdef";

    #[test]
    fn secrets_cannot_be_printed_by_accident() {
        let s = Secret::new(SENTINEL);
        assert_eq!(format!("{s:?}"), "Secret(***)");
        assert!(!format!("{:?}", Some(&s)).contains("SENTINEL"));
        assert_eq!(s.expose(), SENTINEL);
        assert!(!s.is_empty());
        let copy = s.clone();
        assert_eq!(copy, s);
    }

    #[test]
    fn known_secrets_are_removed_wherever_they_appear() {
        let s = Secret::new("hunter2hunter2");
        let text = "login hunter2hunter2 failed; retry with hunter2hunter2!";
        let out = redact(text, &[&s]);
        assert!(!out.contains("hunter2"), "{out}");
        assert_eq!(out.matches(REDACTED).count(), 2);
        assert_eq!(redact("nothing here", &[&s]), "nothing here");
        assert_eq!(redact("same", &[&Secret::new("")]), "same");
    }

    #[test]
    fn credential_shapes_are_removed_even_when_unknown() {
        for text in [
            format!("error: invalid key {SENTINEL} for project"),
            "Authorization: Bearer abcdefgh12345678".to_owned(),
            "x-api-key: abcdef123456".to_owned(),
            r#"{"api_key":"abcdef123456","other":1}"#.to_owned(),
            "GET /v1/models?key=AbCdEf123456&alt=json".to_owned(),
            "AIzaSyD-1234567890abcdefghijklmnop leaked".to_owned(),
            "token=abcdef1234".to_owned(),
        ] {
            let out = redact(&text, &[]);
            assert!(out.contains(REDACTED), "{text} -> {out}");
            for needle in ["SENTINEL", "abcdefgh12345678", "abcdef123456", "AbCdEf123456", "AIzaSy", "abcdef1234"] {
                assert!(!out.contains(needle), "{needle} survived in {out}");
            }
        }
    }

    #[test]
    fn ordinary_text_is_left_alone() {
        for text in [
            "The pawn walked to the store.",
            "sky-blue and skip-this",
            "task-force 12 token economy",
            "keyboard layout: qwerty",
            "monkey=banana",
            "Bearer",
            "key",
            "sk-short",
            "naïve café — unicode is fine ✓",
            "",
        ] {
            assert_eq!(redact(text, &[]), text, "{text}");
        }
    }

    #[test]
    fn the_scrubber_handles_unicode_and_boundaries_without_panicking() {
        for text in ["é", "sk-é", "key=é", "x-api-key:é", "\u{1F600}sk-12345678", "Bearer \u{1F600}", "key\":\"", "a=b=c=d"] {
            let _ = redact(text, &[]);
        }
        // A prefix in the middle of a word is not a key.
        assert_eq!(redact("whisk-12345678901", &[]), "whisk-12345678901");
    }

    #[test]
    fn redaction_is_idempotent() {
        let once = redact(&format!("a {SENTINEL} b Bearer abcdefgh12345 c"), &[]);
        assert_eq!(redact(&once, &[]), once);
    }
}
