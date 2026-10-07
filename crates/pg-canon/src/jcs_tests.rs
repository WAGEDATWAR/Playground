//! Cross-check of `Canon::to_canonical_string` against an independent RFC 8785 (JSON Canonicalization
//! Scheme) serializer (suggestion S-004).
//!
//! Our values are integers only, so the number rules of RFC 8785 reduce to "decimal digits". The
//! remaining rules are string escaping and member order. The reference serializer below is written
//! straight from the RFC text, not from `Canon`'s code.
//!
//! Finding: member order. RFC 8785 sorts keys by UTF-16 code units; a `BTreeMap<String, _>` sorts by
//! UTF-8 bytes, which is Unicode code point order. The two agree except when a key contains a character
//! from U+E000..U+FFFF compared with one from the supplementary planes (U+10000 and up), because UTF-16
//! encodes the latter as surrogates (D800..DFFF) that sort *below* E000. Everything else is identical.
//! Keys in engine data are ASCII, so hashes and saves are unaffected and stay deterministic; the gap is
//! recorded in `docs/DECISIONS.md` (D-022) and the divergence is pinned by a test below.

use crate::{json, Canon};
use proptest::prelude::*;
use std::collections::BTreeMap;

/// RFC 8785 section 3.2.2.2 string serialization.
fn jcs_string(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '\u{8}' => out.push_str("\\b"),
            '\t' => out.push_str("\\t"),
            '\n' => out.push_str("\\n"),
            '\u{c}' => out.push_str("\\f"),
            '\r' => out.push_str("\\r"),
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// RFC 8785 section 3.2.3: members sorted by the UTF-16 code units of their names.
fn jcs(v: &Canon) -> String {
    match v {
        Canon::Null => "null".to_owned(),
        Canon::Bool(b) => b.to_string(),
        Canon::Int(i) => i.to_string(),
        Canon::Str(s) => jcs_string(s),
        Canon::List(items) => format!("[{}]", items.iter().map(jcs).collect::<Vec<_>>().join(",")),
        Canon::Map(m) => {
            let mut members: Vec<(&String, &Canon)> = m.iter().collect();
            members.sort_by(|a, b| a.0.encode_utf16().cmp(b.0.encode_utf16()));
            let body: Vec<String> = members
                .into_iter()
                .map(|(k, v)| format!("{}:{}", jcs_string(k), jcs(v)))
                .collect();
            format!("{{{}}}", body.join(","))
        }
    }
}

/// Keys whose UTF-16 and code-point orders coincide: no characters above the BMP and none in E000..FFFF.
fn comparable_key() -> BoxedStrategy<String> {
    prop::collection::vec(
        prop_oneof![
            (0x20u32..0x7F).prop_map(|c| char::from_u32(c).unwrap_or('a')),
            (0xA0u32..0xD7FF).prop_map(|c| char::from_u32(c).unwrap_or('a')),
            Just('"'),
            Just('\\'),
            Just('\n'),
            Just('\u{1}'),
        ],
        0..8,
    )
    .prop_map(|v| v.into_iter().collect())
    .boxed()
}

fn any_string() -> BoxedStrategy<String> {
    prop::collection::vec(any::<char>(), 0..12)
        .prop_map(|v| v.into_iter().collect())
        .boxed()
}

fn tree(key: BoxedStrategy<String>) -> impl Strategy<Value = Canon> {
    let leaf = prop_oneof![
        Just(Canon::Null),
        any::<bool>().prop_map(Canon::Bool),
        any::<i64>().prop_map(|i| Canon::Int(i128::from(i))),
        any::<u64>().prop_map(|i| Canon::Int(i128::from(i))),
        any_string().prop_map(Canon::Str),
    ];
    leaf.prop_recursive(4, 40, 6, move |inner| {
        prop_oneof![
            prop::collection::vec(inner.clone(), 0..5).prop_map(Canon::List),
            prop::collection::btree_map(key.clone(), inner, 0..5).prop_map(Canon::Map),
        ]
    })
}

proptest! {
    #[test]
    fn canon_matches_rfc_8785_when_key_orders_coincide(v in tree(comparable_key())) {
        prop_assert_eq!(v.to_canonical_string(), jcs(&v));
    }

    #[test]
    fn every_canonical_text_parses_back_to_the_same_value(v in tree(any_string())) {
        let text = v.to_canonical_string();
        prop_assert_eq!(json::parse(&text).unwrap(), v);
    }
}

#[test]
fn well_known_rfc_examples() {
    // RFC 8785 section 3.2.3 sorting example (names beyond ASCII), restricted to what our values can hold.
    let mut m = BTreeMap::new();
    m.insert("\u{20ac}".to_owned(), Canon::str("Euro Sign")); // U+20AC
    m.insert("\r".to_owned(), Canon::str("Carriage Return"));
    m.insert(
        "\u{fb33}".to_owned(),
        Canon::str("Hebrew Letter Dalet With Dagesh"),
    ); // U+FB33
    m.insert("1".to_owned(), Canon::str("One"));
    m.insert("\u{1f600}".to_owned(), Canon::str("Emoji: Grinning Face")); // U+1F600
    m.insert("\u{80}".to_owned(), Canon::str("Control"));
    m.insert(
        "\u{f6}".to_owned(),
        Canon::str("Latin Small Letter O With Diaeresis"),
    );
    let expected_order = [
        "\r",
        "1",
        "\u{80}",
        "\u{f6}",
        "\u{20ac}",
        "\u{1f600}",
        "\u{fb33}",
    ];
    // The RFC lists the emoji (U+1F600, surrogates D83D DE00) before U+FB33; code-point order does not.
    let jcs_text = jcs(&Canon::Map(m.clone()));
    let positions: Vec<usize> = expected_order
        .iter()
        .map(|k| jcs_text.find(&jcs_string(k)).unwrap())
        .collect();
    assert!(
        positions.windows(2).all(|w| w[0] < w[1]),
        "the reference follows the RFC order"
    );
    // Our canonical form differs here, and only here: this pins the known deviation (D-022).
    let ours = Canon::Map(m).to_canonical_string();
    let emoji = ours.find(&jcs_string("\u{1f600}")).unwrap();
    let dalet = ours.find(&jcs_string("\u{fb33}")).unwrap();
    assert!(
        dalet < emoji,
        "Canon orders by code point, so U+FB33 comes before U+1F600"
    );
}

#[test]
fn escaping_matches_the_rfc_for_every_control_character() {
    for c in 0u32..0x20 {
        let s = char::from_u32(c).unwrap().to_string();
        assert_eq!(
            Canon::Str(s.clone()).to_canonical_string(),
            jcs_string(&s),
            "U+{c:04X}"
        );
    }
    for s in [
        "\"",
        "\\",
        "/",
        "\u{7f}",
        "\u{2028}",
        "\u{feff}",
        "\u{1f600}",
    ] {
        assert_eq!(Canon::str(s).to_canonical_string(), jcs_string(s), "{s:?}");
    }
}
