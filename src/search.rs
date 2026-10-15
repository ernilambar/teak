//! Find in Files: matches a query against every line of a set of files.
//!
//! GPUI-free so it can run on the background executor and be unit-tested.
//! Every query mode compiles to one `Regex`: a literal query is escaped, whole
//! word wraps it in `\b`, and case-insensitive adds `(?i)`.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use regex::Regex;

/// Files larger than this are skipped: they are rarely what a search is for,
/// and reading them on every keystroke is slow.
const MAX_FILE_BYTES: u64 = 1024 * 1024;
/// Stop collecting once this many matches are found.
pub(crate) const MAX_MATCHES: usize = 1_000;
/// Longest preview kept per match, in characters.
const MAX_PREVIEW_CHARS: usize = 160;
/// Characters of context kept before a match when the line is trimmed.
const PREVIEW_LEAD_CHARS: usize = 40;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct SearchOptions {
    pub(crate) case_sensitive: bool,
    pub(crate) whole_word: bool,
    pub(crate) regex: bool,
}

/// One match: its file, its 0-based line, and its 0-based character columns.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct SearchMatch {
    pub(crate) path: PathBuf,
    pub(crate) line: u32,
    pub(crate) start: u32,
    pub(crate) end: u32,
    /// The line, trimmed and shortened around the match.
    pub(crate) preview: String,
}

#[derive(Debug, Default)]
pub(crate) struct SearchResults {
    pub(crate) matches: Vec<SearchMatch>,
    /// How many distinct files hold a match.
    pub(crate) files: usize,
    /// True when [`MAX_MATCHES`] cut the search short.
    pub(crate) truncated: bool,
}

/// Compiles `query` under `options`, or explains why an invalid regex fails.
pub(crate) fn build_matcher(query: &str, options: SearchOptions) -> Result<Regex, String> {
    let mut pattern = if options.regex {
        query.to_string()
    } else {
        regex::escape(query)
    };
    if options.whole_word {
        pattern = format!(r"\b(?:{pattern})\b");
    }
    if !options.case_sensitive {
        pattern = format!("(?i){pattern}");
    }
    Regex::new(&pattern).map_err(|error| error.to_string())
}

/// Searches `files` in order. Returns `None` once `generation` no longer equals
/// `current`, meaning a newer query replaced this one.
pub(crate) fn search_files(
    files: &[PathBuf],
    matcher: &Regex,
    generation: &AtomicU64,
    current: u64,
) -> Option<SearchResults> {
    let mut results = SearchResults::default();
    for path in files {
        if generation.load(Ordering::Relaxed) != current {
            return None;
        }
        let Some(text) = read_text(path) else {
            continue;
        };
        let before = results.matches.len();
        let full = search_text(path, &text, matcher, &mut results.matches);
        if results.matches.len() > before {
            results.files += 1;
        }
        if full {
            results.truncated = true;
            break;
        }
    }
    Some(results)
}

/// Reads `path` as text, skipping large and binary files.
fn read_text(path: &Path) -> Option<String> {
    let metadata = std::fs::metadata(path).ok()?;
    if metadata.len() > MAX_FILE_BYTES {
        return None;
    }
    let bytes = std::fs::read(path).ok()?;
    if bytes.iter().take(8192).any(|&byte| byte == 0) {
        return None;
    }
    Some(String::from_utf8_lossy(&bytes).into_owned())
}

/// Appends every match in `text` to `out`. Returns true once `out` is full.
fn search_text(path: &Path, text: &str, matcher: &Regex, out: &mut Vec<SearchMatch>) -> bool {
    for (ix, line) in text.lines().enumerate() {
        for found in matcher.find_iter(line) {
            // An empty match (from a regex like `a*`) selects nothing useful.
            if found.is_empty() {
                continue;
            }
            let start = line[..found.start()].chars().count();
            let end = start + found.as_str().chars().count();
            out.push(SearchMatch {
                path: path.to_path_buf(),
                line: ix as u32,
                start: start as u32,
                end: end as u32,
                preview: preview(line, start),
            });
            if out.len() >= MAX_MATCHES {
                return true;
            }
        }
    }
    false
}

/// The line without its indentation, shortened so the match at character
/// `start` stays visible.
fn preview(line: &str, start: usize) -> String {
    let line = line.trim_end_matches('\r');
    let indent = line.chars().take_while(|c| c.is_whitespace()).count();
    let skip = indent.max(start.saturating_sub(PREVIEW_LEAD_CHARS));
    let mut out: String = line.chars().skip(skip).take(MAX_PREVIEW_CHARS).collect();
    if skip > indent {
        out.insert(0, '…');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn matches(text: &str, query: &str, options: SearchOptions) -> Vec<(u32, u32, u32)> {
        let matcher = build_matcher(query, options).unwrap();
        let mut out = Vec::new();
        search_text(Path::new("a.txt"), text, &matcher, &mut out);
        out.iter().map(|m| (m.line, m.start, m.end)).collect()
    }

    #[test]
    fn literal_search_ignores_case_by_default() {
        let text = "Foo bar\nfoo.bar\n";
        assert_eq!(
            matches(text, "foo", SearchOptions::default()),
            [(0, 0, 3), (1, 0, 3)]
        );
        // A literal query does not treat `.` as a wildcard.
        assert_eq!(
            matches("fooxbar foo.bar", "foo.bar", SearchOptions::default()),
            [(0, 8, 15)]
        );
    }

    #[test]
    fn case_sensitive_and_whole_word_narrow_matches() {
        let case = SearchOptions {
            case_sensitive: true,
            ..Default::default()
        };
        assert_eq!(matches("Foo foo", "foo", case), [(0, 4, 7)]);

        let word = SearchOptions {
            whole_word: true,
            ..Default::default()
        };
        assert_eq!(
            matches("cat concat cat_x cat", "cat", word),
            [(0, 0, 3), (0, 17, 20)]
        );
    }

    #[test]
    fn regex_mode_uses_the_pattern() {
        let regex = SearchOptions {
            regex: true,
            ..Default::default()
        };
        assert_eq!(matches("a1 b22 c", r"\d+", regex), [(0, 1, 2), (0, 4, 6)]);
        assert!(build_matcher("(", regex).is_err());
        // Empty matches are skipped instead of reported at every position.
        assert!(matches("abc", "x*", regex).is_empty());
    }

    #[test]
    fn columns_count_characters_not_bytes() {
        assert_eq!(
            matches("héllo wörld", "wörld", SearchOptions::default()),
            [(0, 6, 11)]
        );
    }

    #[test]
    fn preview_trims_indent_and_long_leads() {
        assert_eq!(preview("    let x = 1;\r", 8), "let x = 1;");
        let long = format!("{}needle", "x".repeat(100));
        let shown = preview(&long, 100);
        assert!(shown.starts_with('…'));
        assert!(shown.ends_with("needle"));
    }

    #[test]
    fn search_files_counts_files_skips_binary_and_stops_when_stale() {
        let dir = std::env::temp_dir().join(format!("tinytext-search-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let a = dir.join("a.txt");
        let b = dir.join("b.txt");
        let bin = dir.join("c.bin");
        std::fs::write(&a, "needle\nneedle\n").unwrap();
        std::fs::write(&b, "nothing here\n").unwrap();
        std::fs::write(&bin, b"needle\0\0").unwrap();
        let files = vec![a, b, bin];
        let matcher = build_matcher("needle", SearchOptions::default()).unwrap();

        let generation = AtomicU64::new(1);
        let results = search_files(&files, &matcher, &generation, 1).unwrap();
        assert_eq!(results.matches.len(), 2);
        assert_eq!(results.files, 1);
        assert!(!results.truncated);

        assert!(search_files(&files, &matcher, &generation, 0).is_none());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn matches_stop_at_the_cap() {
        let text = "x\n".repeat(MAX_MATCHES + 10);
        let matcher = build_matcher("x", SearchOptions::default()).unwrap();
        let mut out = Vec::new();
        assert!(search_text(Path::new("a"), &text, &matcher, &mut out));
        assert_eq!(out.len(), MAX_MATCHES);
    }
}
