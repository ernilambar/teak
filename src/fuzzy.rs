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
    let query: Vec<char> = query.chars().map(|c| c.to_ascii_lowercase()).collect();
    let candidate: Vec<char> = candidate.chars().map(|c| c.to_ascii_lowercase()).collect();
    if query.is_empty() {
        return Some(0);
    }

    let mut total: u32 = 0;
    let mut first: Option<usize> = None;
    let mut previous: Option<usize> = None;
    let mut matched = 0usize;

    for (position, ch) in candidate.iter().copied().enumerate() {
        if matched == query.len() {
            break;
        }
        if ch != query[matched] {
            continue;
        }

        let mut points = 10;
        if previous == Some(position.wrapping_sub(1)) {
            points += 13;
        }
        if position == 0 {
            points += 6;
        } else if is_boundary(candidate[position - 1]) {
            points += 14;
        }
        first = first.or(Some(position));
        previous = Some(position);
        matched += 1;
        total += points;
    }

    if matched < query.len() {
        return None;
    }

    // Prefer candidates where the matched characters sit close together.
    let span = previous.unwrap_or(0).saturating_sub(first.unwrap_or(0)) as u32;
    let spread = span.saturating_sub((query.len() as u32).max(1) - 1);
    Some(total.saturating_sub(spread * 2))
}

/// Characters a useful match usually starts a component after.
fn is_boundary(previous: char) -> bool {
    matches!(previous, '/' | '\\' | '_' | '-' | '.' | ' ')
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
