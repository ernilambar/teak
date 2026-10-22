//! Find in Files: matches a query against every line of a set of files and
//! lays the hits out as the Find Results document.
//!
//! GPUI-free so it can run on the background executor and be unit-tested.
//! Every query mode compiles to one `Regex`: a literal query is escaped, whole
//! word wraps it in `\b`, and case-insensitive adds `(?i)`.

use std::collections::BTreeSet;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::SystemTime;

use regex::Regex;
use serde::{Deserialize, Serialize};

/// Files larger than this are skipped: they are rarely what a search is for.
const MAX_FILE_BYTES: u64 = 1024 * 1024;
/// Stop collecting once this many matches are found.
pub(crate) const MAX_MATCHES: usize = 1_000;
/// Lines shown before and after each matching line.
const CONTEXT_LINES: usize = 2;
/// Longest line shown in the results, in characters; minified files would
/// otherwise make the document unreadable.
const MAX_LINE_CHARS: usize = 300;

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct SearchOptions {
    pub(crate) case_sensitive: bool,
    pub(crate) whole_word: bool,
    pub(crate) regex: bool,
}

/// One match: its 0-based line and its 0-based character columns.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct SearchMatch {
    pub(crate) line: u32,
    pub(crate) start: u32,
    pub(crate) end: u32,
}

/// Every match in one file, plus the lines shown around them.
#[derive(Debug)]
pub(crate) struct FileHits {
    pub(crate) path: PathBuf,
    /// The file's modification time when it was searched.
    pub(crate) modified: Option<SystemTime>,
    pub(crate) matches: Vec<SearchMatch>,
    /// Matching and context lines, by 0-based line number, in order.
    pub(crate) lines: Vec<(u32, String)>,
}

#[derive(Debug, Default)]
pub(crate) struct SearchResults {
    pub(crate) files: Vec<FileHits>,
    /// How many files were searched.
    pub(crate) searched: usize,
    /// True when [`MAX_MATCHES`] cut the search short.
    pub(crate) truncated: bool,
}

impl SearchResults {
    pub(crate) fn match_count(&self) -> usize {
        self.files.iter().map(|file| file.matches.len()).sum()
    }
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

/// Splits the Where field into include and exclude globs. Entries are comma
/// separated; a leading `-` excludes, anything else includes.
pub(crate) fn parse_where(text: &str) -> (Vec<String>, Vec<String>) {
    let mut include = Vec::new();
    let mut exclude = Vec::new();
    for entry in text
        .split(',')
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
    {
        match entry.strip_prefix('-') {
            Some(glob) if !glob.trim().is_empty() => exclude.push(glob.trim().to_string()),
            Some(_) => {}
            None => include.push(entry.to_string()),
        }
    }
    (include, exclude)
}

/// Searches `files` in order. Returns `None` once `generation` no longer equals
/// `current`, meaning a newer search replaced this one.
pub(crate) fn search_files(
    files: &[PathBuf],
    matcher: &Regex,
    generation: &AtomicU64,
    current: u64,
) -> Option<SearchResults> {
    let mut results = SearchResults::default();
    let mut found = 0;
    for path in files {
        if generation.load(Ordering::Relaxed) != current {
            return None;
        }
        let modified = std::fs::metadata(path)
            .and_then(|meta| meta.modified())
            .ok();
        let Some(text) = read_text(path) else {
            continue;
        };
        results.searched += 1;
        let (hits, full) = search_text(path, &text, matcher, MAX_MATCHES - found);
        if let Some(mut hits) = hits {
            hits.modified = modified;
            found += hits.matches.len();
            results.files.push(hits);
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

/// Collects up to `room` matches in `text` with their context lines. The flag
/// is true when `room` ran out.
fn search_text(path: &Path, text: &str, matcher: &Regex, room: usize) -> (Option<FileHits>, bool) {
    let lines: Vec<&str> = text.lines().collect();
    let mut matches = Vec::new();
    let mut full = false;
    'lines: for (ix, line) in lines.iter().enumerate() {
        for found in matcher.find_iter(line) {
            // An empty match (from a regex like `a*`) selects nothing useful.
            if found.is_empty() {
                continue;
            }
            if matches.len() >= room {
                full = true;
                break 'lines;
            }
            let start = line[..found.start()].chars().count();
            let end = start + found.as_str().chars().count();
            matches.push(SearchMatch {
                line: ix as u32,
                start: start as u32,
                end: end as u32,
            });
        }
    }
    if matches.is_empty() {
        return (None, full);
    }

    let mut shown = BTreeSet::new();
    for found in &matches {
        let line = found.line as usize;
        let first = line.saturating_sub(CONTEXT_LINES);
        let last = (line + CONTEXT_LINES).min(lines.len() - 1);
        shown.extend(first..=last);
    }
    let lines = shown
        .into_iter()
        .map(|ix| (ix as u32, lines[ix].trim_end_matches('\r').to_string()))
        .collect();
    let hits = FileHits {
        path: path.to_path_buf(),
        modified: None,
        matches,
        lines,
    };
    (Some(hits), full)
}

/// Replaces every match in `text` the way the search finds them: line by
/// line, skipping empty matches, so a pattern cannot reach across lines. In
/// regex mode `$1` and `${name}` expand to capture groups; otherwise the
/// replacement is literal. Returns the new text and how many matches changed.
pub(crate) fn replace_matches(
    text: &str,
    matcher: &Regex,
    replacement: &str,
    expand: bool,
) -> (String, usize) {
    let mut out = String::with_capacity(text.len());
    let mut count = 0;
    for line in text.split_inclusive('\n') {
        let (body, ending) = match line.strip_suffix("\r\n") {
            Some(body) => (body, "\r\n"),
            None => match line.strip_suffix('\n') {
                Some(body) => (body, "\n"),
                None => (line, ""),
            },
        };
        let mut last = 0;
        for captures in matcher.captures_iter(body) {
            let found = captures.get(0).expect("group 0 always matches");
            if found.is_empty() {
                continue;
            }
            out.push_str(&body[last..found.start()]);
            if expand {
                captures.expand(replacement, &mut out);
            } else {
                out.push_str(replacement);
            }
            last = found.end();
            count += 1;
        }
        out.push_str(&body[last..]);
        out.push_str(ending);
    }
    (out, count)
}

/// A results line that opens a match.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ResultTarget {
    /// 0-based line in the results document.
    pub(crate) doc_line: u32,
    pub(crate) path: PathBuf,
    /// The first match on that line.
    pub(crate) found: SearchMatch,
}

/// The Find Results document: its text, the lines that open matches, and the
/// byte ranges of every match to highlight.
#[derive(Debug, Default)]
pub(crate) struct ResultsDocument {
    pub(crate) text: String,
    pub(crate) targets: Vec<ResultTarget>,
    pub(crate) highlights: Vec<Range<usize>>,
}

/// Lays `results` out Sublime-style: a header, each file's path with its
/// matching lines (`12:`) between context lines (`12 `), `..` for skipped
/// lines, and a summary. `display` names each file.
pub(crate) fn results_document(
    header: &str,
    results: &SearchResults,
    display: impl Fn(&Path) -> String,
) -> ResultsDocument {
    let mut doc = ResultsDocument::default();
    let mut line_count = 0u32;
    let mut push_line = |doc: &mut ResultsDocument, text: &str| {
        doc.text.push_str(text);
        doc.text.push('\n');
        line_count += 1;
        line_count - 1
    };

    push_line(&mut doc, header);
    for file in &results.files {
        push_line(&mut doc, "");
        push_line(&mut doc, &format!("{}:", display(&file.path)));
        let width = file
            .lines
            .last()
            .map_or(1, |(line, _)| (line + 1).to_string().len());
        let mut previous: Option<u32> = None;
        for (line, text) in &file.lines {
            if previous.is_some_and(|previous| line - previous > 1) {
                push_line(&mut doc, &format!("{:>width$}", ".."));
            }
            previous = Some(*line);

            let on_line: Vec<&SearchMatch> = file
                .matches
                .iter()
                .filter(|found| found.line == *line)
                .collect();
            let marker = if on_line.is_empty() { ' ' } else { ':' };
            let prefix = format!("{:>width$}{marker} ", line + 1);
            let shown: String = text.chars().take(MAX_LINE_CHARS).collect();
            let line_start = doc.text.len() + prefix.len();
            for found in &on_line {
                if let Some(range) = char_range(&shown, found.start, found.end) {
                    doc.highlights
                        .push(line_start + range.start..line_start + range.end);
                }
            }
            let doc_line = push_line(&mut doc, &format!("{prefix}{shown}"));
            if let Some(first) = on_line.first() {
                doc.targets.push(ResultTarget {
                    doc_line,
                    path: file.path.clone(),
                    found: **first,
                });
            }
        }
    }
    push_line(&mut doc, "");
    push_line(&mut doc, &summary(results));
    doc
}

/// "3 matches in 2 files", noting a cap that cut the search short.
pub(crate) fn summary(results: &SearchResults) -> String {
    let matches = results.match_count();
    let files = results.files.len();
    let noun = |count: usize, one: &str, many: &str| {
        format!("{count} {}", if count == 1 { one } else { many })
    };
    let mut text = format!(
        "{} in {}",
        noun(matches, "match", "matches"),
        noun(files, "file", "files")
    );
    if results.truncated {
        text.push_str(&format!(" (stopped at {MAX_MATCHES})"));
    }
    text
}

/// The byte range of characters `start..end` in `text`, clipped to its end;
/// `None` when the match starts past what is shown.
fn char_range(text: &str, start: u32, end: u32) -> Option<Range<usize>> {
    let offset = |ix: u32| {
        text.char_indices()
            .nth(ix as usize)
            .map_or(text.len(), |(offset, _)| offset)
    };
    let range = offset(start)..offset(end);
    (range.start < range.end).then_some(range)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn matches(text: &str, query: &str, options: SearchOptions) -> Vec<(u32, u32, u32)> {
        let matcher = build_matcher(query, options).unwrap();
        let (hits, _) = search_text(Path::new("a.txt"), text, &matcher, MAX_MATCHES);
        hits.map(|hits| hits.matches)
            .unwrap_or_default()
            .iter()
            .map(|m| (m.line, m.start, m.end))
            .collect()
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
    fn context_lines_surround_matches_and_merge() {
        let text = "0\n1\n2 x\n3\n4\n5\n6\n7 x\n8\n";
        let matcher = build_matcher("x", SearchOptions::default()).unwrap();
        let (hits, _) = search_text(Path::new("a"), text, &matcher, MAX_MATCHES);
        let lines: Vec<u32> = hits.unwrap().lines.iter().map(|(line, _)| *line).collect();
        assert_eq!(lines, [0, 1, 2, 3, 4, 5, 6, 7, 8]);

        let text = "x\n1\n2\n3\n4\n5\nx\n";
        let (hits, _) = search_text(Path::new("a"), text, &matcher, MAX_MATCHES);
        let lines: Vec<u32> = hits.unwrap().lines.iter().map(|(line, _)| *line).collect();
        assert_eq!(lines, [0, 1, 2, 4, 5, 6]);
    }

    #[test]
    fn matches_stop_at_the_room_left() {
        let text = "x\n".repeat(10);
        let matcher = build_matcher("x", SearchOptions::default()).unwrap();
        let (hits, full) = search_text(Path::new("a"), &text, &matcher, 4);
        assert!(full);
        assert_eq!(hits.unwrap().matches.len(), 4);
    }

    #[test]
    fn parse_where_splits_includes_and_excludes() {
        let (include, exclude) = parse_where(" src/**, -**/*.min.js ,*.rs, -, ,-vendor/** ");
        assert_eq!(include, ["src/**", "*.rs"]);
        assert_eq!(exclude, ["**/*.min.js", "vendor/**"]);
        assert_eq!(parse_where(""), (Vec::new(), Vec::new()));
    }

    #[test]
    fn results_document_groups_files_and_maps_lines() {
        let results = SearchResults {
            files: vec![FileHits {
                path: PathBuf::from("/p/src/a.rs"),
                modified: None,
                matches: vec![
                    SearchMatch {
                        line: 1,
                        start: 4,
                        end: 10,
                    },
                    SearchMatch {
                        line: 9,
                        start: 0,
                        end: 6,
                    },
                ],
                lines: vec![
                    (0, "one".into()),
                    (1, "two needle".into()),
                    (2, "three".into()),
                    (9, "needle".into()),
                ],
            }],
            searched: 3,
            truncated: false,
        };
        let doc = results_document("Searching 3 files for \"needle\"", &results, |path| {
            path.strip_prefix("/p").unwrap().display().to_string()
        });

        assert_eq!(
            doc.text,
            "Searching 3 files for \"needle\"\n\
             \n\
             src/a.rs:\n \
             1  one\n \
             2: two needle\n \
             3  three\n\
             ..\n\
             10: needle\n\
             \n\
             2 matches in 1 file\n"
        );
        let lines: Vec<u32> = doc.targets.iter().map(|target| target.doc_line).collect();
        assert_eq!(lines, [4, 7]);
        let highlighted: Vec<&str> = doc
            .highlights
            .iter()
            .map(|range| &doc.text[range.clone()])
            .collect();
        assert_eq!(highlighted, ["needle", "needle"]);
    }

    #[test]
    fn replace_matches_is_literal_unless_regex() {
        let literal = build_matcher("a.b", SearchOptions::default()).unwrap();
        assert_eq!(
            replace_matches("a.b axb A.B\r\n", &literal, "$1", false),
            ("$1 axb $1\r\n".to_string(), 2)
        );

        let regex = SearchOptions {
            regex: true,
            case_sensitive: true,
            ..Default::default()
        };
        let swap = build_matcher(r"(\w+)=(?P<v>\d+)", regex).unwrap();
        assert_eq!(
            replace_matches("x=1\ny=22\n", &swap, "${v}:$1", true),
            ("1:x\n22:y\n".to_string(), 2)
        );
    }

    #[test]
    fn replace_matches_stays_within_lines_and_skips_empty_matches() {
        let regex = SearchOptions {
            regex: true,
            ..Default::default()
        };
        let spaces = build_matcher(r"\s+", regex).unwrap();
        assert_eq!(
            replace_matches("a  b\nc\n", &spaces, "_", true),
            ("a_b\nc\n".to_string(), 1),
            "the line break survives"
        );
        let maybe = build_matcher("x*", regex).unwrap();
        assert_eq!(
            replace_matches("abc", &maybe, "-", true),
            ("abc".to_string(), 0)
        );
    }

    #[test]
    fn summary_notes_truncation() {
        let results = SearchResults {
            truncated: true,
            ..Default::default()
        };
        assert_eq!(
            summary(&results),
            format!("0 matches in 0 files (stopped at {MAX_MATCHES})")
        );
    }

    #[test]
    fn search_files_skips_binary_and_stops_when_stale() {
        let dir = std::env::temp_dir().join(format!("app-search-{}", std::process::id()));
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
        assert_eq!(results.match_count(), 2);
        assert_eq!(results.files.len(), 1);
        assert_eq!(results.searched, 2, "the binary file is not searched");
        assert!(results.files[0].modified.is_some());

        assert!(search_files(&files, &matcher, &generation, 0).is_none());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
