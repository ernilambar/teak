//! A tiny fuzzy matcher for the file finder.
//!
//! A query matches a candidate when its characters appear in order (a
//! subsequence), case-insensitively. The score rewards tight matches: runs of
//! consecutive characters, matches at the start or after a path separator, and
//! penalizes how far the match spreads out. GPUI-free, so it can be unit
//! tested in isolation.

/// Scores how well `query` fuzzy-matches `candidate`. Higher is better and
/// `None` means the query is not a subsequence of the candidate. An empty
/// query matches anything.
pub(crate) fn score(query: &str, candidate: &str) -> Option<u32> {
    let query: Vec<char> = query.chars().map(fold).collect();
    if query.is_empty() {
        return Some(0);
    }

    // The candidate is walked in place: quick open scores every indexed path
    // on each keystroke, so allocating a character vector per candidate (and
    // per path view) dominates the filter at 20,000 files.
    let mut total: u32 = 0;
    let mut first: Option<usize> = None;
    let mut previous: Option<usize> = None;
    let mut matched = 0usize;
    // The original character, not the folded one: a camelCase step needs both
    // sides' case, and [`is_boundary`] only ever looks at punctuation.
    let mut preceding: Option<char> = None;

    for (position, raw) in candidate.chars().enumerate() {
        if matched == query.len() {
            break;
        }
        let ch = fold(raw);
        if ch == query[matched] {
            let mut points = 10;
            if previous == Some(position.wrapping_sub(1)) {
                points += 13;
            }
            if position == 0 {
                points += 6;
            } else if preceding.is_some_and(|before| is_boundary(before, raw)) {
                points += 14;
            }
            first = first.or(Some(position));
            previous = Some(position);
            matched += 1;
            total += points;
        }
        preceding = Some(raw);
    }

    if matched < query.len() {
        return None;
    }

    // Prefer candidates where the matched characters sit close together.
    let span = previous.unwrap_or(0).saturating_sub(first.unwrap_or(0)) as u32;
    let spread = span.saturating_sub((query.len() as u32).max(1) - 1);
    Some(total.saturating_sub(spread * 2))
}

/// Lowercases one character without allocating. `char::to_lowercase` can yield
/// two chars (`İ`), and the first is the one that matches a keystroke. The ASCII
/// fast path keeps the common case — every keystroke scores thousands of
/// candidates — a table lookup.
fn fold(ch: char) -> char {
    if ch.is_ascii() {
        ch.to_ascii_lowercase()
    } else {
        ch.to_lowercase().next().unwrap_or(ch)
    }
}

/// Whether a match at `current` starts a component: after punctuation or a
/// space, or at a camelCase step (`quickOpen` for `qO`).
fn is_boundary(previous: char, current: char) -> bool {
    matches!(previous, '/' | '\\' | '_' | '-' | '.' | ' ')
        || (previous.is_lowercase() && current.is_uppercase())
}

#[cfg(test)]
mod tests {
    use super::score;

    #[test]
    fn empty_query_matches_anything() {
        assert_eq!(score("", "anything.rs"), Some(0));
        assert_eq!(score("", ""), Some(0));
    }

    #[test]
    fn non_subsequences_fail() {
        assert_eq!(score("zz", "main.rs"), None);
        assert_eq!(score("niam", "main.rs"), None);
        assert_eq!(score("abc", ""), None);
    }

    #[test]
    fn matching_is_case_insensitive() {
        assert!(score("MP", "MyPath.rs").is_some());
        assert!(score("mypath", "MYPATH").is_some());
        // Non-ASCII folds too: `to_ascii_lowercase` would leave `É` alone.
        assert_eq!(score("café", "CAFÉ.rs"), score("CAFÉ", "café.rs"));
    }

    #[test]
    fn camel_case_steps_count_as_boundaries() {
        // `qO` starts a component at `O`; the same letters mid-word do not.
        assert!(score("qO", "quickOpen.rs").unwrap() > score("qO", "quickxopen.rs").unwrap());
        // The bonus follows the candidate's case, so it holds for a lowercase
        // query too, and it is absent when nothing capitalizes.
        assert!(score("qo", "quickOpen.rs").unwrap() > score("qo", "quickxopen.rs").unwrap());
    }

    #[test]
    fn tight_matches_outscore_scattered_ones() {
        let strict = score("main", "main.rs").unwrap();
        let scattered = score("mrs", "main.rs").unwrap();
        assert!(strict > scattered);
    }

    #[test]
    fn word_boundaries_outscore_mid_word_matches() {
        let on_boundary = score("qui", "src/quick_open.rs").unwrap();
        let mid_word = score("qui", "src/sequick_open.rs").unwrap();
        assert!(on_boundary > mid_word);
    }

    #[test]
    fn runs_of_characters_beat_frequent_skips() {
        let run = score("qui", "quick_open.rs").unwrap();
        let jumps = score("qk", "quick_open.rs").unwrap();
        assert!(run > jumps);
    }
}
