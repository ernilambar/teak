//! Performance budgets, written as ignored tests.
//!
//! ```sh
//! cargo test --release bench_ -- --ignored --nocapture
//! ```
//!
//! Teak is a binary crate, so a `benches/` target (and criterion) would need a
//! library target first. Until then the budgets live in the crate, where they
//! can reach the same code the app runs, and run on demand. Each bench reports
//! its best-of-N time and fails only past a hard ceiling set well above the
//! measured baseline: the ceiling catches an order-of-magnitude regression
//! without failing on a loaded machine.
//!
//! The fixtures are generated deterministically instead of committed, so the
//! repository does not carry a multi-megabyte file that would rot.
//!
//! Baselines and ceilings are recorded in `docs/performance.md`.

use std::hint::black_box;
use std::time::{Duration, Instant};

use super::editor::{LineBuffer, render};
use super::files::TextFormat;
use crate::fuzzy;

/// Lines in the large-file fixtures. Matches the figure in the docs, so a
/// number quoted there is comparable with a run here.
const LINES: usize = 100_000;

/// The most paths quick open will index, from `quick_open.rs`.
const INDEX_FILES: usize = super::quick_open::MAX_INDEX_FILES;

/// Runs per bench. Best-of-N is used because a single slow run on a busy
/// machine says nothing about the code.
const RUNS: usize = 5;

/// A Rust-looking source file of `lines` lines, with no trailing blank line.
fn source(lines: usize) -> String {
    let mut text = String::with_capacity(lines * 40);
    for i in 0..lines {
        text.push_str("    let value_");
        text.push_str(&i.to_string());
        text.push_str(" = compute(");
        text.push_str(&(i % 97).to_string());
        text.push_str(");\n");
    }
    text
}

/// The best wall time of `runs` passes.
fn best_of(runs: usize, mut body: impl FnMut()) -> Duration {
    let mut best = Duration::MAX;
    for _ in 0..runs {
        let start = Instant::now();
        body();
        best = best.min(start.elapsed());
    }
    best
}

/// Prints one line and holds the bench to its ceiling.
fn report(name: &str, took: Duration, ceiling: Duration) {
    let verdict = if took <= ceiling { "ok" } else { "OVER" };
    println!(
        "bench {name:<28} {:>10.2} ms   ceiling {:>7.2} ms   {verdict}",
        took.as_secs_f64() * 1_000.,
        ceiling.as_secs_f64() * 1_000.,
    );
    assert!(
        took <= ceiling,
        "bench {name} took {took:?}, over its {ceiling:?} ceiling; see docs/performance.md",
    );
}

/// Reading and indexing a 100,000-line file, the cost of opening one tab.
///
/// Covers the disk read, the text decode and [`LineBuffer`]'s line index. The
/// editor engine's own parse is not included; this is the part Teak owns.
#[test]
#[ignore = "performance budget"]
fn bench_open_100k_lines() {
    let text = source(LINES);
    let dir = std::env::temp_dir().join(format!("teak-bench-open-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("large.rs");
    std::fs::write(&path, &text).unwrap();

    let took = best_of(RUNS, || {
        let bytes = std::fs::read(&path).unwrap();
        let (text, format) = TextFormat::decode(&bytes).unwrap();
        let buffer = LineBuffer::new(&text);
        black_box((text.len(), format.crlf, buffer.lines.len()));
    });
    let _ = std::fs::remove_dir_all(&dir);
    report("open_100k_lines", took, Duration::from_millis(30));
}

/// The per-command cost on a 100,000-line file: one character typed, then the
/// line index and the whole text rebuilt the way a line command does it.
///
/// This is the number that decides whether editing a large file feels instant.
#[test]
#[ignore = "performance budget"]
fn bench_keystroke_100k_lines() {
    let text = source(LINES);
    let middle = text.len() / 2;

    let took = best_of(RUNS, || {
        let mut edited = text.clone();
        edited.replace_range(middle..middle, "x");
        let buffer = LineBuffer::new(&edited);
        let rebuilt = render(&buffer.lines, buffer.trailing_newline);
        black_box(rebuilt.len());
    });
    report("keystroke_100k_lines", took, Duration::from_millis(20));
}

/// Quick open's filter over the whole index: the fuzzy score for every path,
/// scored once against the file name and once against the relative path.
///
/// The index is capped at `quick_open::MAX_INDEX_FILES`, so that is the size a
/// keystroke actually pays for; sorting and rendering are excluded.
#[test]
#[ignore = "performance budget"]
fn bench_quick_open_index() {
    let paths: Vec<(String, String)> = (0..INDEX_FILES)
        .map(|i| {
            (
                format!("file_{i}.rs"),
                format!("src/module_{}/subdir_{}/file_{i}.rs", i % 500, i % 31),
            )
        })
        .collect();

    let took = best_of(RUNS, || {
        let mut hits = 0u32;
        for (name, path) in &paths {
            if let Some(score) = fuzzy::score("modfile", name) {
                hits += score + 30;
            }
            if let Some(score) = fuzzy::score("modfile", path) {
                hits += score;
            }
        }
        black_box(hits);
    });
    report("quick_open_index", took, Duration::from_millis(30));
}

/// Find in files' matching over a 100,000-line buffer, regex included.
///
/// The file walk and the disk reads are separate; this is the scan.
#[test]
#[ignore = "performance budget"]
fn bench_find_regex_100k_lines() {
    let text = source(LINES);
    let matcher = crate::search::build_matcher(
        "compute\\(4[0-9]\\)",
        crate::search::SearchOptions::default(),
    )
    .unwrap();

    let took = best_of(RUNS, || {
        let found = matcher.find_iter(&text).take(1_000).count();
        black_box(found);
    });
    report("find_regex_100k_lines", took, Duration::from_millis(30));
}

/// The gutter's changed-line read on a 100,000-line file after one edit.
///
/// This is the debounced path that runs 300 ms after typing stops, so it is
/// allowed to be slower than a keystroke, but not seconds.
#[test]
#[ignore = "performance budget"]
fn bench_git_gutter_100k_lines() {
    let Some(repo) = GitRepo::new() else {
        return;
    };
    let text = source(LINES);
    repo.write("large.rs", &text);
    if !repo.commit_all() {
        return;
    }
    let mut edited = text.clone();
    let middle = edited.len() / 2;
    edited.replace_range(middle..middle, "// note\n");

    let path = repo.dir.join("large.rs");
    let took = best_of(RUNS, || {
        let changes = crate::git::line_changes(&path, &edited, Some(crate::git::Status::Modified));
        black_box(changes.len());
    });
    report("git_gutter_100k_lines", took, Duration::from_millis(300));
}

/// A throwaway repository for the benches that need `git`.
struct GitRepo {
    dir: std::path::PathBuf,
}

impl GitRepo {
    /// `None` on a machine without git, so the bench is skipped, not failed.
    fn new() -> Option<Self> {
        let dir = std::env::temp_dir().join(format!("teak-bench-git-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).ok()?;
        let repo = Self { dir };
        repo.run(&["init", "-q", "-b", "trunk"]).then_some(repo)
    }

    fn run(&self, args: &[&str]) -> bool {
        std::process::Command::new("git")
            .arg("-C")
            .arg(&self.dir)
            .args(args)
            .output()
            .is_ok_and(|output| output.status.success())
    }

    fn write(&self, name: &str, text: &str) {
        std::fs::write(self.dir.join(name), text).unwrap();
    }

    fn commit_all(&self) -> bool {
        self.run(&["add", "."])
            && self.run(&[
                "-c",
                "user.email=bench@example.invalid",
                "-c",
                "user.name=bench",
                "commit",
                "-qm",
                "fixture",
            ])
    }
}

impl Drop for GitRepo {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}
