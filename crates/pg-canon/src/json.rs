//! A strict, integer-only JSON parser producing [`Canon`] values.
//!
//! Content packs, replay logs and saves are JSON, and Blueprint §0 and §5.2 say untrusted data is
//! validated at every boundary and that authoritative data has no floating point. This parser enforces
//! both at the lowest level, where `serde_json` would not:
//!
//! * numbers must be integers in `i64::MIN..=u64::MAX` (no fraction, no exponent, no `1.0`);
//! * duplicate object keys are an error (not "last one wins");
//! * nesting is limited ([`MAX_DEPTH`]) so a hostile `[[[[…` cannot overflow the stack;
//! * no comments, no trailing commas, no leading zeros, no lone surrogates.
//!
//! A leading UTF-8 byte-order mark is skipped (Windows editors add one).

use crate::Canon;
use std::collections::BTreeMap;
use std::fmt;

/// The default nesting limit.
pub const MAX_DEPTH: usize = 64;

/// A parse failure with a 1-based position.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JsonError {
    pub line: usize,
    pub column: usize,
    pub message: String,
}

impl fmt::Display for JsonError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "line {}, column {}: {}",
            self.line, self.column, self.message
        )
    }
}

impl std::error::Error for JsonError {}

/// Parses `text` with the default depth limit.
pub fn parse(text: &str) -> Result<Canon, JsonError> {
    parse_with_depth(text, MAX_DEPTH)
}

/// Parses `text`, allowing at most `max_depth` levels of nesting.
pub fn parse_with_depth(text: &str, max_depth: usize) -> Result<Canon, JsonError> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut p = Parser {
        text,
        pos: 0,
        max_depth,
    };
    p.skip_ws();
    let value = p.value(0)?;
    p.skip_ws();
    if p.pos < p.text.len() {
        return Err(p.error("unexpected data after the top-level value"));
    }
    Ok(value)
}

struct Parser<'a> {
    text: &'a str,
    pos: usize,
    max_depth: usize,
}

impl Parser<'_> {
    fn error(&self, message: impl Into<String>) -> JsonError {
        let consumed = self.text.get(..self.pos).unwrap_or("");
        let line = consumed.bytes().filter(|&b| b == b'\n').count() + 1;
        let column = consumed
            .rsplit('\n')
            .next()
            .map_or(0, |l| l.chars().count())
            + 1;
        JsonError {
            line,
            column,
            message: message.into(),
        }
    }

    fn peek(&self) -> Option<char> {
        self.text.get(self.pos..).and_then(|r| r.chars().next())
    }

    fn bump(&mut self) -> Option<char> {
        let c = self.peek()?;
        self.pos += c.len_utf8();
        Some(c)
    }

    fn skip_ws(&mut self) {
        while matches!(self.peek(), Some(' ' | '\t' | '\n' | '\r')) {
            self.pos += 1;
        }
    }

    fn expect(&mut self, want: char) -> Result<(), JsonError> {
        match self.peek() {
            Some(c) if c == want => {
                self.pos += c.len_utf8();
                Ok(())
            }
            Some(c) => Err(self.error(format!("expected '{want}', found '{c}'"))),
            None => Err(self.error(format!("expected '{want}', found end of input"))),
        }
    }

    fn literal(&mut self, word: &str, value: Canon) -> Result<Canon, JsonError> {
        if self
            .text
            .get(self.pos..)
            .is_some_and(|r| r.starts_with(word))
        {
            self.pos += word.len();
            Ok(value)
        } else {
            Err(self.error(format!("invalid literal, expected '{word}'")))
        }
    }

    fn value(&mut self, depth: usize) -> Result<Canon, JsonError> {
        match self.peek() {
            None => Err(self.error("unexpected end of input")),
            Some('{') => self.object(depth),
            Some('[') => self.array(depth),
            Some('"') => self.string().map(Canon::Str),
            Some('t') => self.literal("true", Canon::Bool(true)),
            Some('f') => self.literal("false", Canon::Bool(false)),
            Some('n') => self.literal("null", Canon::Null),
            Some(c) if c == '-' || c.is_ascii_digit() => self.number(),
            Some(c) => Err(self.error(format!("unexpected character '{c}'"))),
        }
    }

    fn enter(&self, depth: usize) -> Result<usize, JsonError> {
        if depth >= self.max_depth {
            Err(self.error(format!("nesting is deeper than {}", self.max_depth)))
        } else {
            Ok(depth + 1)
        }
    }

    fn object(&mut self, depth: usize) -> Result<Canon, JsonError> {
        let depth = self.enter(depth)?;
        self.expect('{')?;
        let mut map = BTreeMap::new();
        self.skip_ws();
        if self.peek() == Some('}') {
            self.pos += 1;
            return Ok(Canon::Map(map));
        }
        loop {
            self.skip_ws();
            if self.peek() != Some('"') {
                return Err(self.error("expected a string key"));
            }
            let key_pos = self.pos;
            let key = self.string()?;
            self.skip_ws();
            self.expect(':')?;
            self.skip_ws();
            let value = self.value(depth)?;
            if map.insert(key.clone(), value).is_some() {
                self.pos = key_pos;
                return Err(self.error(format!("duplicate key '{key}'")));
            }
            self.skip_ws();
            match self.bump() {
                Some(',') => {}
                Some('}') => return Ok(Canon::Map(map)),
                Some(c) => {
                    self.pos -= c.len_utf8();
                    return Err(self.error(format!("expected ',' or '}}', found '{c}'")));
                }
                None => return Err(self.error("unterminated object")),
            }
        }
    }

    fn array(&mut self, depth: usize) -> Result<Canon, JsonError> {
        let depth = self.enter(depth)?;
        self.expect('[')?;
        let mut items = Vec::new();
        self.skip_ws();
        if self.peek() == Some(']') {
            self.pos += 1;
            return Ok(Canon::List(items));
        }
        loop {
            self.skip_ws();
            items.push(self.value(depth)?);
            self.skip_ws();
            match self.bump() {
                Some(',') => {}
                Some(']') => return Ok(Canon::List(items)),
                Some(c) => {
                    self.pos -= c.len_utf8();
                    return Err(self.error(format!("expected ',' or ']', found '{c}'")));
                }
                None => return Err(self.error("unterminated array")),
            }
        }
    }

    fn number(&mut self) -> Result<Canon, JsonError> {
        let start = self.pos;
        let negative = self.peek() == Some('-');
        if negative {
            self.pos += 1;
        }
        let digits_start = self.pos;
        match self.peek() {
            Some('0') => {
                self.pos += 1;
                if self.peek().is_some_and(|c| c.is_ascii_digit()) {
                    return Err(self.error("numbers may not have leading zeros"));
                }
            }
            Some(c) if c.is_ascii_digit() => {
                while self.peek().is_some_and(|c| c.is_ascii_digit()) {
                    self.pos += 1;
                }
            }
            _ => return Err(self.error("a '-' must be followed by digits")),
        }
        if matches!(self.peek(), Some('.' | 'e' | 'E')) {
            return Err(
                self.error("non-integer number: floating point is not allowed in game data")
            );
        }
        let digits = self.text.get(digits_start..self.pos).unwrap_or("");
        if digits.len() > 20 {
            self.pos = start;
            return Err(self.error("integer is out of range"));
        }
        let magnitude: u128 = digits.parse().map_err(|_| {
            self.pos = start;
            self.error("integer is out of range")
        })?;
        let value = if negative {
            let v = i128::try_from(magnitude).ok().map(|m| -m);
            v.filter(|v| *v >= i128::from(i64::MIN))
        } else {
            i128::try_from(magnitude)
                .ok()
                .filter(|v| *v <= i128::from(u64::MAX))
        };
        match value {
            Some(v) => Ok(Canon::Int(v)),
            None => {
                self.pos = start;
                Err(self.error("integer is out of range (must fit i64 or u64)"))
            }
        }
    }

    fn hex4(&mut self) -> Result<u32, JsonError> {
        let mut v = 0u32;
        for _ in 0..4 {
            let c = self
                .bump()
                .ok_or_else(|| self.error("unterminated \\u escape"))?;
            let d = c
                .to_digit(16)
                .ok_or_else(|| self.error("invalid \\u escape"))?;
            v = v * 16 + d;
        }
        Ok(v)
    }

    fn string(&mut self) -> Result<String, JsonError> {
        self.expect('"')?;
        let mut out = String::new();
        loop {
            let c = self
                .bump()
                .ok_or_else(|| self.error("unterminated string"))?;
            match c {
                '"' => return Ok(out),
                '\\' => {
                    let e = self
                        .bump()
                        .ok_or_else(|| self.error("unterminated escape"))?;
                    match e {
                        '"' => out.push('"'),
                        '\\' => out.push('\\'),
                        '/' => out.push('/'),
                        'b' => out.push('\u{08}'),
                        'f' => out.push('\u{0c}'),
                        'n' => out.push('\n'),
                        'r' => out.push('\r'),
                        't' => out.push('\t'),
                        'u' => {
                            let hi = self.hex4()?;
                            let code = if (0xD800..0xDC00).contains(&hi) {
                                if self
                                    .text
                                    .get(self.pos..)
                                    .is_some_and(|r| r.starts_with("\\u"))
                                {
                                    self.pos += 2;
                                    let lo = self.hex4()?;
                                    if !(0xDC00..0xE000).contains(&lo) {
                                        return Err(self.error(
                                            "a high surrogate must be followed by a low surrogate",
                                        ));
                                    }
                                    0x10000 + ((hi - 0xD800) << 10) + (lo - 0xDC00)
                                } else {
                                    return Err(self.error("lone surrogate in \\u escape"));
                                }
                            } else if (0xDC00..0xE000).contains(&hi) {
                                return Err(self.error("lone surrogate in \\u escape"));
                            } else {
                                hi
                            };
                            out.push(
                                char::from_u32(code)
                                    .ok_or_else(|| self.error("invalid code point"))?,
                            );
                        }
                        other => return Err(self.error(format!("invalid escape '\\{other}'"))),
                    }
                }
                c if (c as u32) < 0x20 => {
                    self.pos -= c.len_utf8();
                    return Err(self.error("control characters must be escaped inside strings"));
                }
                c => out.push(c),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn ok(text: &str) -> Canon {
        parse(text).unwrap_or_else(|e| panic!("{text:?} should parse: {e}"))
    }

    fn err(text: &str) -> JsonError {
        parse(text).expect_err(&format!("{text:?} should be rejected"))
    }

    #[test]
    fn scalars_and_containers() {
        assert_eq!(ok("null"), Canon::Null);
        assert_eq!(ok(" true "), Canon::Bool(true));
        assert_eq!(ok("false"), Canon::Bool(false));
        assert_eq!(ok("0"), Canon::Int(0));
        assert_eq!(ok("-0"), Canon::Int(0));
        assert_eq!(ok("42"), Canon::Int(42));
        assert_eq!(ok("-17"), Canon::Int(-17));
        assert_eq!(ok("\"hi\""), Canon::str("hi"));
        assert_eq!(ok("[]"), Canon::List(vec![]));
        assert_eq!(ok("{}"), Canon::Map(Default::default()));
        assert_eq!(
            ok(r#" { "b" : [1, 2, {"c": null}], "a": "x" } "#).to_canonical_string(),
            r#"{"a":"x","b":[1,2,{"c":null}]}"#
        );
    }

    #[test]
    fn integer_range_is_i64_min_to_u64_max() {
        assert_eq!(ok("18446744073709551615"), Canon::Int(i128::from(u64::MAX)));
        assert_eq!(ok("-9223372036854775808"), Canon::Int(i128::from(i64::MIN)));
        assert!(parse("18446744073709551616").is_err());
        assert!(parse("-9223372036854775809").is_err());
        assert!(parse("123456789012345678901234567890").is_err());
        assert!(
            parse("00000000000000000000001").is_err(),
            "leading zeros are rejected before range"
        );
    }

    #[test]
    fn floats_are_rejected_in_every_form() {
        for text in [
            "1.5",
            "1.0",
            "-0.5",
            "1e3",
            "1E3",
            "1e-2",
            "0.0",
            "[2.5]",
            r#"{"x": 0.1}"#,
        ] {
            let e = err(text);
            assert!(e.message.contains("non-integer"), "{text}: {e}");
        }
    }

    #[test]
    fn malformed_numbers() {
        for text in ["01", "-", "-a", "+1", ".5", "1.", "--1", "0x10"] {
            assert!(parse(text).is_err(), "{text}");
        }
    }

    #[test]
    fn string_escapes() {
        assert_eq!(
            ok(r#""a\"b\\c\/d\be\ff\ng\rh\ti""#),
            Canon::str("a\"b\\c/d\u{08}e\u{0c}f\ng\rh\ti")
        );
        assert_eq!(ok(r#""Aé""#), Canon::str("Aé"));
        assert_eq!(ok(r#""😀""#), Canon::str("😀"), "surrogate pair");
        assert_eq!(ok("\"héllo 😀\""), Canon::str("héllo 😀"));
    }

    #[test]
    fn bad_strings() {
        for text in [
            r#""abc"#,
            r#""\x""#,
            r#""\u12""#,
            r#""\u12g4""#,
            r#""\ud83d""#,
            r#""\ude00""#,
            r#""\ud83dA""#,
            "\"tab\there\"",
            "\"line\nbreak\"",
        ] {
            assert!(parse(text).is_err(), "{text:?}");
        }
    }

    #[test]
    fn structural_errors() {
        for text in [
            "",
            "   ",
            "[",
            "]",
            "{",
            "}",
            "[1,]",
            "[,1]",
            "[1 2]",
            r#"{"a":1,}"#,
            r#"{"a" 1}"#,
            r#"{a:1}"#,
            r#"{"a":}"#,
            "nul",
            "tru",
            "nulll",
            "[1] x",
            "{} {}",
            "// c\n1",
            "/* c */ 1",
            "'a'",
        ] {
            assert!(parse(text).is_err(), "{text:?}");
        }
    }

    #[test]
    fn duplicate_keys_are_an_error_not_last_one_wins() {
        let e = err(r#"{"a":1,"b":2,"a":3}"#);
        assert!(e.message.contains("duplicate key 'a'"), "{e}");
        assert!(
            parse(r#"{"a":{"x":1},"b":{"x":2}}"#).is_ok(),
            "same key in different objects is fine"
        );
    }

    #[test]
    fn nesting_is_limited_without_overflowing_the_stack() {
        let deep = format!("{}{}", "[".repeat(100_000), "]".repeat(100_000));
        assert!(err(&deep).message.contains("deeper"));
        let at_limit = format!("{}{}", "[".repeat(MAX_DEPTH), "]".repeat(MAX_DEPTH));
        assert!(parse(&at_limit).is_ok());
        let over = format!("{}{}", "[".repeat(MAX_DEPTH + 1), "]".repeat(MAX_DEPTH + 1));
        assert!(parse(&over).is_err());
        let deep_obj = format!("{}1{}", r#"{"a":"#.repeat(100_000), "}".repeat(100_000));
        assert!(parse(&deep_obj).is_err());
    }

    #[test]
    fn a_byte_order_mark_is_skipped() {
        assert_eq!(ok("\u{feff}{\"a\":1}").to_canonical_string(), r#"{"a":1}"#);
    }

    #[test]
    fn errors_report_line_and_column() {
        let e = err("{\n  \"a\": 1,\n  \"b\": oops\n}");
        assert_eq!((e.line, e.column), (3, 8), "{e}");
        let e = err("[1, 2.5]");
        assert_eq!((e.line, e.column), (1, 6), "{e}");
    }

    fn canon_strategy() -> impl Strategy<Value = Canon> {
        let leaf = prop_oneof![
            Just(Canon::Null),
            any::<bool>().prop_map(Canon::Bool),
            any::<i64>().prop_map(|v| Canon::Int(i128::from(v))),
            any::<u64>().prop_map(|v| Canon::Int(i128::from(v))),
            ".{0,12}".prop_map(Canon::Str),
        ];
        leaf.prop_recursive(4, 48, 6, |inner| {
            prop_oneof![
                proptest::collection::vec(inner.clone(), 0..5).prop_map(Canon::List),
                proptest::collection::btree_map(".{0,6}", inner, 0..5).prop_map(Canon::Map),
            ]
        })
    }

    proptest! {
        #[test]
        fn canonical_text_always_parses_back_to_the_same_value(c in canon_strategy()) {
            let text = c.to_canonical_string();
            prop_assert_eq!(parse(&text), Ok(c));
        }

        #[test]
        fn arbitrary_text_never_panics(s in ".{0,200}") {
            let _ = parse(&s);
        }

        #[test]
        fn arbitrary_json_ish_bytes_never_panic(s in r#"[\[\]\{\}",:0-9a-z\\ .eE+-]{0,80}"#) {
            let _ = parse(&s);
        }
    }
}
