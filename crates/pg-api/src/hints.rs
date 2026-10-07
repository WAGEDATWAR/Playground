//! "Did you mean…?" hints for validation messages.
//!
//! Typos are the most common content mistake, and "unknown field" with a list of everything that is
//! allowed makes the author hunt. A hint names the single closest known word, if one is close enough to
//! be a plausible typo, using Levenshtein edit distance over characters.

/// Edit distance between two strings, counted in characters: insertions, deletions, substitutions and
/// swaps of two adjacent characters each cost one (the "optimal string alignment" form of
/// Damerau-Levenshtein). Counting a swap as one edit matters for typos like `widht` for `width`.
pub fn edit_distance(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let width = b.len() + 1;
    let mut d = vec![0usize; (a.len() + 1) * width];
    let at = |i: usize, j: usize| i * width + j;
    for i in 0..=a.len() {
        if let Some(cell) = d.get_mut(at(i, 0)) {
            *cell = i;
        }
    }
    for j in 0..=b.len() {
        if let Some(cell) = d.get_mut(at(0, j)) {
            *cell = j;
        }
    }
    let get = |d: &[usize], i: usize, j: usize| d.get(at(i, j)).copied().unwrap_or(0);
    for i in 1..=a.len() {
        for j in 1..=b.len() {
            let same = a.get(i - 1) == b.get(j - 1);
            let mut best = (get(&d, i - 1, j - 1) + usize::from(!same))
                .min(get(&d, i - 1, j) + 1)
                .min(get(&d, i, j - 1) + 1);
            if i > 1 && j > 1 && a.get(i - 1) == b.get(j - 2) && a.get(i - 2) == b.get(j - 1) {
                best = best.min(get(&d, i - 2, j - 2) + 1);
            }
            if let Some(cell) = d.get_mut(at(i, j)) {
                *cell = best;
            }
        }
    }
    get(&d, a.len(), b.len())
}

/// How far a word may be from a candidate and still count as a typo of it: one edit for short words,
/// two for medium ones, three for long ones. A candidate as long as the whole mistake is never suggested.
fn allowed_distance(word: &str) -> usize {
    match word.chars().count() {
        0..=3 => 1,
        4..=7 => 2,
        _ => 3,
    }
}

/// The known word closest to `word`, if any is within typo distance. Ties go to the first candidate, so
/// pass candidates in a stable (sorted) order.
pub fn closest<'a>(word: &str, known: impl IntoIterator<Item = &'a str>) -> Option<&'a str> {
    let limit = allowed_distance(word);
    let mut best: Option<(usize, &str)> = None;
    for candidate in known {
        let d = edit_distance(word, candidate);
        if d == 0 || d > limit || d >= word.chars().count().max(1) {
            continue;
        }
        if best.is_none_or(|(bd, _)| d < bd) {
            best = Some((d, candidate));
        }
    }
    best.map(|(_, c)| c)
}

/// A message suffix: ` (did you mean 'x'?)`, or nothing when there is no close match.
pub fn hint<'a>(word: &str, known: impl IntoIterator<Item = &'a str>) -> String {
    closest(word, known).map_or_else(String::new, |c| format!(" (did you mean '{c}'?)"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edit_distance_basics() {
        assert_eq!(edit_distance("", ""), 0);
        assert_eq!(edit_distance("abc", "abc"), 0);
        assert_eq!(edit_distance("abc", ""), 3);
        assert_eq!(edit_distance("", "abc"), 3);
        assert_eq!(edit_distance("kitten", "sitting"), 3);
        assert_eq!(edit_distance("flaw", "lawn"), 2);
        assert_eq!(edit_distance("capacity", "capcity"), 1);
        assert_eq!(
            edit_distance("width", "widht"),
            1,
            "a swap of adjacent letters is one edit"
        );
        assert_eq!(edit_distance("ab", "ba"), 1);
        assert_eq!(edit_distance("abc", "cba"), 2);
        assert_eq!(
            edit_distance("héllo", "hello"),
            1,
            "counted in characters, not bytes"
        );
    }

    #[test]
    fn edit_distance_is_symmetric() {
        for (a, b) in [
            ("weight", "wieght"),
            ("exclude_tags", "exclude_tag"),
            ("x", "yy"),
            ("", "a"),
        ] {
            assert_eq!(edit_distance(a, b), edit_distance(b, a), "{a} {b}");
        }
    }

    #[test]
    fn typos_get_a_suggestion() {
        let fields = ["blocks_movement", "height", "weight", "width"];
        assert_eq!(closest("wieght", fields), Some("weight"));
        assert_eq!(closest("heigth", fields), Some("height"));
        assert_eq!(closest("blocks_movment", fields), Some("blocks_movement"));
        assert_eq!(closest("widht", fields), Some("width"));
    }

    #[test]
    fn unrelated_words_get_no_suggestion() {
        let fields = ["blocks_movement", "height", "weight", "width"];
        assert_eq!(closest("colour", fields), None);
        assert_eq!(closest("zzzzzz", fields), None);
        assert_eq!(closest("", fields), None);
    }

    #[test]
    fn exact_matches_and_tiny_words_are_not_suggested() {
        assert_eq!(
            closest("weight", ["weight"]),
            None,
            "an exact match is not a typo"
        );
        assert_eq!(
            closest("a", ["b", "c"]),
            None,
            "a one-letter word is too short to guess at"
        );
    }

    #[test]
    fn the_closest_wins_and_ties_go_to_the_first() {
        assert_eq!(closest("wid", ["width", "wide", "wider"]), Some("wide"));
        assert_eq!(closest("cat", ["car", "cap"]), Some("car"));
    }

    #[test]
    fn hint_formats_a_message_suffix() {
        assert_eq!(hint("wieght", ["weight"]), " (did you mean 'weight'?)");
        assert_eq!(hint("colour", ["weight"]), "");
    }
}
