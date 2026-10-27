//! Working-tree status, branch and line-level changes for the workspace.
//!
//! The app needs a file's state, not repository objects, so this shells out to
//! `git status`, `git show` and `git diff` instead of linking a git library:
//! macOS always ships a usable `git`, and the short calls it makes stay off the
//! main thread.
//!
//! GPUI-free on purpose: [`state`] takes the workspace folders and returns
//! what the tree and the status bar query by path, and [`line_changes`] takes
//! a buffer and returns what the editor marks under its changed lines.

use std::collections::{HashMap, HashSet};
use std::ffi::OsStr;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

/// A working-tree state. The variants are ordered by how much attention they
/// deserve, so a path reported twice keeps the more serious one.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Status {
    Untracked,
    Modified,
    Renamed,
    Added,
    Deleted,
    Conflicted,
}

impl Status {
    /// The single character shown next to the entry.
    pub(crate) fn letter(self) -> &'static str {
        match self {
            Self::Untracked => "U",
            Self::Modified => "M",
            Self::Renamed => "R",
            Self::Added => "A",
            Self::Deleted => "D",
            Self::Conflicted => "!",
        }
    }
}

/// The checked-out branch of one repository.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Branch {
    /// The branch name, or the short commit id when HEAD is detached.
    pub(crate) name: String,
    /// Commits this branch has that its upstream does not, and the reverse.
    pub(crate) ahead: u32,
    pub(crate) behind: u32,
}

/// One read of git state for the workspace: the changed files, the most
/// serious change beneath each folder, and the branch of each workspace
/// folder's repository.
#[derive(Debug, Default)]
pub(crate) struct GitState {
    files: HashMap<PathBuf, Status>,
    folders: HashMap<PathBuf, Status>,
    branches: HashMap<PathBuf, Branch>,
    git_dirs: HashSet<PathBuf>,
}

impl GitState {
    pub(crate) fn status(&self, path: &Path) -> Option<Status> {
        self.files.get(path).copied()
    }

    /// The most serious change somewhere beneath `path`, for a folder row.
    pub(crate) fn folder_status(&self, path: &Path) -> Option<Status> {
        self.folders.get(path).copied()
    }

    /// The branch of the repository `root` belongs to, when it has one.
    pub(crate) fn branch(&self, root: &Path) -> Option<&Branch> {
        self.branches.get(root)
    }

    /// The repositories' git directories, which the filesystem watcher follows
    /// so a commit or checkout refreshes the tree by itself.
    pub(crate) fn git_dirs(&self) -> impl Iterator<Item = &PathBuf> {
        self.git_dirs.iter()
    }

    /// Records `relative`, a path from `git status` read in `root`, along with
    /// every folder between it and `root` inclusive.
    fn insert(&mut self, root: &Path, relative: &[u8], status: Status) {
        let relative = trim_trailing_slashes(relative);
        if relative.is_empty() {
            return;
        }
        let path = root.join(OsStr::from_bytes(relative));
        let kept = self.files.entry(path.clone()).or_insert(status);
        if status > *kept {
            *kept = status;
        }
        let mut folder = path.parent();
        while let Some(dir) = folder {
            let kept = self.folders.entry(dir.to_path_buf()).or_insert(status);
            if status > *kept {
                *kept = status;
            }
            if dir == root {
                break;
            }
            folder = dir.parent();
        }
    }
}

/// Reads the status and branch of every workspace folder. A folder outside a
/// repository contributes nothing; a folder inside one reports only its own
/// subtree and its repository's branch.
pub(crate) fn state(roots: &[PathBuf]) -> GitState {
    let mut state = GitState::default();
    for root in roots {
        let Some((prefix, git_dir)) = repo_paths(root) else {
            continue;
        };
        state.git_dirs.insert(git_dir);
        let Some(output) = run_status(root) else {
            continue;
        };
        parse_into(&output, root, &prefix, &mut state);
    }
    state
}

/// The folder's path inside its repository, such as `src/`, or empty at the
/// repository root, plus the repository's git directory. Fails outside a
/// repository, which is how a plain folder is skipped.
fn repo_paths(root: &Path) -> Option<(Vec<u8>, PathBuf)> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["rev-parse", "--show-prefix", "--absolute-git-dir"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let mut lines = output.stdout.split(|byte| *byte == b'\n');
    let prefix = lines.next()?.to_vec();
    let git_dir = lines.next()?;
    (!git_dir.is_empty()).then(|| (prefix, PathBuf::from(OsStr::from_bytes(git_dir))))
}

/// Runs `git status` in `root`. Porcelain v2 with `-z` keeps paths unambiguous
/// (no quoting, no newline guessing).
fn run_status(root: &Path) -> Option<Vec<u8>> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args([
            "status",
            "--porcelain=v2",
            // Branch headers name the branch and count its divergence from the
            // upstream, which the status bar shows.
            "--branch",
            "-z",
            "--untracked-files=normal",
        ])
        .env("GIT_OPTIONAL_LOCKS", "0")
        .output()
        .ok()?;
    output.status.success().then_some(output.stdout)
}

/// Walks the NUL-separated records of `output`, recording every changed path
/// that sits under `root` and the repository's branch.
fn parse_into(output: &[u8], root: &Path, prefix: &[u8], state: &mut GitState) {
    let mut records = output.split(|byte| *byte == 0);
    let mut branch = BranchRead::default();
    while let Some(record) = records.next() {
        if let Some(header) = record.strip_prefix(b"# ") {
            branch.record(header);
            continue;
        }
        let Some((start, status)) = parse_record(record) else {
            continue;
        };
        // A rename names its original path in the record that follows.
        if record.first() == Some(&b'2') {
            records.next();
        }
        // Changes outside a nested workspace folder, and its own entry, are
        // not part of this tree.
        let Some(relative) = record[start..].strip_prefix(prefix) else {
            continue;
        };
        state.insert(root, relative, status);
    }
    if let Some(branch) = branch.finish() {
        state.branches.insert(root.to_path_buf(), branch);
    }
}

/// The `# branch.*` headers of one repository, collected as they arrive.
#[derive(Default)]
struct BranchRead {
    head: Option<String>,
    oid: Option<String>,
    ahead: u32,
    behind: u32,
}

impl BranchRead {
    fn record(&mut self, header: &[u8]) {
        let header = String::from_utf8_lossy(header);
        if let Some(head) = header.strip_prefix("branch.head ") {
            self.head = Some(head.to_string());
        } else if let Some(oid) = header.strip_prefix("branch.oid ") {
            self.oid = Some(oid.to_string());
        } else if let Some(divergence) = header.strip_prefix("branch.ab ") {
            let mut counts = divergence.split_whitespace();
            if let (Some(ahead), Some(behind)) = (counts.next(), counts.next()) {
                self.ahead = ahead.trim_start_matches('+').parse().unwrap_or(0);
                self.behind = behind.trim_start_matches('-').parse().unwrap_or(0);
            }
        }
    }

    /// `None` when the output carried no branch header at all, which is what a
    /// repository-less folder looks like.
    fn finish(self) -> Option<Branch> {
        let name = match self.head? {
            // Detached HEAD is reported as a marker, so the short commit id is
            // the only thing that names what is checked out.
            head if head == "(detached)" => self.oid.as_deref()?.chars().take(7).collect(),
            head => head,
        };
        Some(Branch {
            name,
            ahead: self.ahead,
            behind: self.behind,
        })
    }
}

/// The offset where a record's path starts, and the status it carries.
fn parse_record(record: &[u8]) -> Option<(usize, Status)> {
    match record.first()? {
        // `#` is a branch header, `!` an ignored file: neither is shown.
        b'#' | b'!' => None,
        b'?' => Some((path_offset(record, 1)?, Status::Untracked)),
        b'u' => Some((path_offset(record, 10)?, Status::Conflicted)),
        b'1' => Some((path_offset(record, 8)?, xy_status(record)?)),
        b'2' => Some((path_offset(record, 9)?, xy_status(record)?)),
        _ => None,
    }
}

/// The offset just past the `count`th space in a record, where its path
/// begins. The path comes last and may itself contain spaces.
fn path_offset(record: &[u8], count: usize) -> Option<usize> {
    let mut seen = 0;
    for (ix, byte) in record.iter().enumerate() {
        if *byte == b' ' {
            seen += 1;
            if seen == count {
                return Some(ix + 1);
            }
        }
    }
    None
}

/// The status held in a record's `XY` field: the worktree column when it says
/// anything, the index column otherwise.
fn xy_status(record: &[u8]) -> Option<Status> {
    let index = *record.get(2)?;
    let worktree = *record.get(3)?;
    status_from_code(if worktree == b'.' { index } else { worktree })
}

fn status_from_code(code: u8) -> Option<Status> {
    match code {
        // `T` is a type change, which reads as a modification.
        b'M' | b'T' => Some(Status::Modified),
        // `C` is a copy, which reads as an addition.
        b'A' | b'C' => Some(Status::Added),
        b'D' => Some(Status::Deleted),
        b'R' => Some(Status::Renamed),
        b'U' => Some(Status::Conflicted),
        _ => None,
    }
}

/// An untracked directory is reported as `dir/`; the slash would make
/// `Path::parent` disagree with the tree's own folder paths.
fn trim_trailing_slashes(bytes: &[u8]) -> &[u8] {
    let mut end = bytes.len();
    while end > 0 && bytes[end - 1] == b'/' {
        end -= 1;
    }
    &bytes[..end]
}

/// A run of lines that differs from the file's committed version.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct LineChange {
    /// The 1-based first line of the run in the buffer. For a deletion this is
    /// the last line before the removal, and 0 when the removal sits above the
    /// first line.
    pub(crate) line: u32,
    /// How many lines the run covers; zero for a deletion.
    pub(crate) count: u32,
    pub(crate) kind: LineKind,
}

/// What a run of lines carries against the committed file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LineKind {
    Added,
    Modified,
    Deleted,
}

/// Reads the line-level changes of `text` against the file's version in `HEAD`.
/// A file git has not committed yet is all addition; everything else compares
/// against the blob, whether or not the change is saved, so marks follow the
/// buffer while typing.
pub(crate) fn line_changes(path: &Path, text: &str, status: Option<Status>) -> Vec<LineChange> {
    match status {
        // Git holds no committed version yet, so every line is an addition.
        Some(Status::Untracked | Status::Added) => whole_file(text),
        // Both sides of a conflict live in the index, and the working file
        // holds markers HEAD never had.
        Some(Status::Conflicted) => Vec::new(),
        _ => committed(path)
            .and_then(|old| diff_lines(&old, text))
            .unwrap_or_default(),
    }
}

/// The buffered lines as one addition, for a file git has not committed.
fn whole_file(text: &str) -> Vec<LineChange> {
    match text.lines().count() {
        0 => Vec::new(),
        count => vec![LineChange {
            line: 1,
            count: count as u32,
            kind: LineKind::Added,
        }],
    }
}

/// The file's committed text. `./` makes the path relative to the file's own
/// folder, so callers need no repository-prefix arithmetic.
fn committed(path: &Path) -> Option<String> {
    let dir = path.parent()?;
    let name = path.file_name()?.to_str()?;
    let output = Command::new("git")
        .current_dir(dir)
        .arg("show")
        .arg(format!("HEAD:./{name}"))
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).into_owned())
}

/// Line-level changes between two versions, read from `git diff`'s own hunks.
/// The app ships no diff engine, and one more short `git` call off the main
/// thread is cheaper than getting the hunks right by hand.
fn diff_lines(old: &str, new: &str) -> Option<Vec<LineChange>> {
    if old == new {
        return Some(Vec::new());
    }
    // `git diff --no-index` compares two files, so the committed text and the
    // buffer both go to temp files, outside any repository's attributes.
    let stamp = TEMP_STAMP.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir();
    let old_path = dir.join(format!("teak-head-{}-{stamp}", std::process::id()));
    let new_path = dir.join(format!("teak-buffer-{}-{stamp}", std::process::id()));
    let wrote = std::fs::write(&old_path, old).is_ok() && std::fs::write(&new_path, new).is_ok();
    let output = wrote.then(|| {
        Command::new("git")
            .current_dir(&dir)
            .args([
                "diff",
                "--no-index",
                "--no-color",
                "--no-ext-diff",
                "-U0",
                "--",
            ])
            .arg(&old_path)
            .arg(&new_path)
            .output()
    });
    let _ = std::fs::remove_file(&old_path);
    let _ = std::fs::remove_file(&new_path);
    // Exit status 1 means the files differ, which is the normal answer here.
    let output = output?.ok()?;
    Some(parse_hunks(&String::from_utf8_lossy(&output.stdout)))
}

/// The changed runs named by a unified diff's hunk headers.
fn parse_hunks(diff: &str) -> Vec<LineChange> {
    diff.lines()
        .filter_map(|line| line.strip_prefix("@@ "))
        .filter_map(parse_hunk)
        .collect()
}

/// One hunk header, `@@ -a,b +c,d @@`, as a changed run. No old lines mean the
/// whole hunk is added, no new lines mean it is a deletion, and the rest
/// modify the lines both sides share.
fn parse_hunk(header: &str) -> Option<LineChange> {
    let (old, new) = header.split_once(" +")?;
    let (_, old_count) = parse_header_range(old.strip_prefix('-')?)?;
    let (new_start, new_count) = parse_header_range(new)?;
    let (line, kind) = if new_count == 0 {
        // The removal sits after the line above it, which is where the editor
        // paints the mark; `0` means it sits above the first line.
        (new_start, LineKind::Deleted)
    } else if old_count == 0 {
        (new_start, LineKind::Added)
    } else {
        (new_start, LineKind::Modified)
    };
    Some(LineChange {
        line,
        count: new_count,
        kind,
    })
}

/// The `start,count` pair of one side of a hunk header. A bare start means one
/// line, and `0,0` marks a side that is missing entirely.
fn parse_header_range(text: &str) -> Option<(u32, u32)> {
    let text = text.split(' ').next()?;
    let (start, count) = text.split_once(',').unwrap_or((text, "1"));
    Some((start.parse().ok()?, count.parse().ok()?))
}

/// The 1-based gutter line for each change: a run marks its own lines, and a
/// deletion marks the top edge of the line it preceded, clamped to the buffer
/// so a removal at the end still lands on the last line.
pub(crate) fn gutter_lines(changes: &[LineChange], text: &str) -> Vec<(u32, LineKind)> {
    let last = text.split('\n').count().max(1) as u32;
    let mut lines: Vec<(u32, LineKind)> = Vec::new();
    for change in changes {
        match change.kind {
            LineKind::Deleted => {
                push_mark(&mut lines, (change.line + 1).min(last), LineKind::Deleted)
            }
            LineKind::Added | LineKind::Modified => {
                for line in change.line..change.line + change.count.max(1) {
                    push_mark(&mut lines, line.min(last), change.kind);
                }
            }
        }
    }
    lines
}

/// Records one mark per line in buffer order. A later mark on a line already
/// marked is dropped, so a removal and a changed line on the same boundary
/// stay one mark.
fn push_mark(lines: &mut Vec<(u32, LineKind)>, line: u32, kind: LineKind) {
    if lines.last().is_none_or(|(last, _)| *last < line) {
        lines.push((line, kind));
    }
}

/// Distinguishes the temp files of overlapping reads in one process.
static TEMP_STAMP: AtomicU64 = AtomicU64::new(0);

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(records: &str) -> GitState {
        let mut statuses = GitState::default();
        parse_into(records.as_bytes(), Path::new("/repo"), b"", &mut statuses);
        statuses
    }

    #[test]
    fn ordinary_records_map_to_statuses() {
        let statuses = parse(
            "1 .M N... 100644 100644 100644 aaa bbb src/main.rs\0\
             1 A. N... 000000 100644 100644 000 ccc src/new.rs\0\
             1 .D N... 100644 100644 100644 aaa bbb src/gone.rs\0",
        );

        assert_eq!(
            statuses.status(Path::new("/repo/src/main.rs")),
            Some(Status::Modified)
        );
        assert_eq!(
            statuses.status(Path::new("/repo/src/new.rs")),
            Some(Status::Added)
        );
        assert_eq!(
            statuses.status(Path::new("/repo/src/gone.rs")),
            Some(Status::Deleted)
        );
    }

    #[test]
    fn the_worktree_column_wins_over_the_index_column() {
        // Staged as modified, then modified again: still just a modification.
        let statuses = parse("1 MM N... 100644 100644 100644 aaa bbb a.rs\0");
        assert_eq!(
            statuses.status(Path::new("/repo/a.rs")),
            Some(Status::Modified)
        );
    }

    #[test]
    fn renames_use_the_new_path_and_skip_the_original() {
        let statuses = parse(
            "2 R. N... 100644 100644 100644 aaa bbb R100 src/to.rs\0src/from.rs\0\
             1 .M N... 100644 100644 100644 aaa bbb src/other.rs\0",
        );

        assert_eq!(
            statuses.status(Path::new("/repo/src/to.rs")),
            Some(Status::Renamed)
        );
        assert_eq!(statuses.status(Path::new("/repo/src/from.rs")), None);
        // The record after a rename must not be read as a status of its own.
        assert_eq!(
            statuses.status(Path::new("/repo/src/other.rs")),
            Some(Status::Modified)
        );
    }

    #[test]
    fn paths_may_contain_spaces() {
        let statuses = parse("1 .M N... 100644 100644 100644 aaa bbb my file.txt\0");
        assert_eq!(
            statuses.status(Path::new("/repo/my file.txt")),
            Some(Status::Modified)
        );
    }

    #[test]
    fn untracked_folders_lose_their_trailing_slash() {
        let statuses = parse("? build/\0");

        assert_eq!(
            statuses.status(Path::new("/repo/build")),
            Some(Status::Untracked)
        );
        assert_eq!(
            statuses.folder_status(Path::new("/repo")),
            Some(Status::Untracked)
        );
    }

    #[test]
    fn conflicts_are_reported_as_such() {
        let statuses = parse("u UU N... 100644 100644 100644 100644 h1 h2 h3 merge.rs\0");
        assert_eq!(
            statuses.status(Path::new("/repo/merge.rs")),
            Some(Status::Conflicted)
        );
    }

    #[test]
    fn folders_up_to_the_root_are_marked() {
        let statuses = parse("1 .M N... 100644 100644 100644 aaa bbb src/deep/file.rs\0");

        for folder in ["/repo/src/deep", "/repo/src", "/repo"] {
            assert_eq!(
                statuses.folder_status(Path::new(folder)),
                Some(Status::Modified),
                "{folder}"
            );
        }
        assert_eq!(statuses.folder_status(Path::new("/elsewhere")), None);
        // A file's own state does not make it a folder.
        assert_eq!(
            statuses.folder_status(Path::new("/repo/src/deep/file.rs")),
            None
        );
    }

    #[test]
    fn a_folder_keeps_the_most_serious_change_beneath_it() {
        let escalation = parse(
            "1 .M N... 100644 100644 100644 aaa bbb src/edited.rs\0\
             1 .D N... 100644 100644 100644 aaa bbb src/gone.rs\0",
        );
        for folder in ["/repo/src", "/repo"] {
            assert_eq!(
                escalation.folder_status(Path::new(folder)),
                Some(Status::Deleted),
                "{folder}"
            );
        }

        // The same changes in the other order must not downgrade the folders.
        let reversed = parse(
            "1 .D N... 100644 100644 100644 aaa bbb src/gone.rs\0\
             1 .M N... 100644 100644 100644 aaa bbb src/edited.rs\0",
        );
        assert_eq!(
            reversed.folder_status(Path::new("/repo/src")),
            Some(Status::Deleted)
        );
    }

    #[test]
    fn headers_ignored_files_and_garbage_are_skipped() {
        let statuses = parse(
            "# branch.oid aaa\0# branch.head main\0! ignored.log\0\
             1 .M N... 100644 100644 100644 aaa bbb kept.rs\0not a record\0",
        );

        assert_eq!(
            statuses.status(Path::new("/repo/kept.rs")),
            Some(Status::Modified)
        );
        assert_eq!(statuses.status(Path::new("/repo/ignored.log")), None);
    }

    #[test]
    fn empty_output_has_no_statuses() {
        let statuses = parse("");
        assert_eq!(statuses.status(Path::new("/repo/a.rs")), None);
        assert_eq!(statuses.folder_status(Path::new("/repo")), None);
    }

    #[test]
    fn a_folder_inside_a_repository_reports_only_its_own_subtree() {
        let mut statuses = GitState::default();
        parse_into(
            b"1 .M N... 100644 100644 100644 aaa bbb src/main.rs\0\
              1 .M N... 100644 100644 100644 aaa bbb lib/x.rs\0",
            Path::new("/repo/src"),
            b"src/",
            &mut statuses,
        );

        assert_eq!(
            statuses.status(Path::new("/repo/src/main.rs")),
            Some(Status::Modified)
        );
        assert_eq!(statuses.status(Path::new("/repo/lib/x.rs")), None);
        assert_eq!(statuses.folder_status(Path::new("/repo/lib")), None);
        assert_eq!(
            statuses.folder_status(Path::new("/repo/src")),
            Some(Status::Modified)
        );
    }

    #[test]
    fn branch_headers_name_the_branch_and_its_divergence() {
        let state = parse(
            "# branch.oid aaa111\0# branch.head main\0\
             # branch.upstream origin/main\0# branch.ab +2 -3\0",
        );

        assert_eq!(
            state.branch(Path::new("/repo")),
            Some(&Branch {
                name: "main".to_string(),
                ahead: 2,
                behind: 3,
            })
        );
    }

    #[test]
    fn a_detached_head_is_named_by_its_short_commit() {
        let state = parse("# branch.oid 0123456789abcdef\0# branch.head (detached)\0");

        assert_eq!(
            state
                .branch(Path::new("/repo"))
                .map(|branch| branch.name.as_str()),
            Some("0123456")
        );
    }

    #[test]
    fn an_empty_repository_keeps_its_unborn_branch_name() {
        let state = parse("# branch.oid (initial)\0# branch.head trunk\0");

        assert_eq!(
            state
                .branch(Path::new("/repo"))
                .map(|branch| branch.name.as_str()),
            Some("trunk")
        );
    }

    #[test]
    fn output_without_headers_has_no_branch() {
        let state = parse("1 .M N... 100644 100644 100644 aaa bbb a.rs\0");
        assert_eq!(state.branch(Path::new("/repo")), None);
    }

    /// Runs `git` in `dir` and says whether it succeeded. A machine without git
    /// just skips the check below.
    fn git(dir: &Path, args: &[&str]) -> bool {
        Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .output()
            .is_ok_and(|output| output.status.success())
    }

    #[test]
    fn reads_a_real_working_tree() {
        let dir = std::env::temp_dir().join(format!("app-git-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::write(dir.join("src/kept.rs"), "x\n").unwrap();

        let committed = git(&dir, &["init", "-q", "-b", "trunk"])
            && git(&dir, &["add", "."])
            && git(
                &dir,
                &[
                    "-c",
                    "user.email=a@b",
                    "-c",
                    "user.name=a",
                    "commit",
                    "-qm",
                    "init",
                ],
            );
        if !committed {
            let _ = std::fs::remove_dir_all(&dir);
            return;
        }

        std::fs::write(dir.join("src/kept.rs"), "changed\n").unwrap();
        std::fs::write(dir.join("src/new.rs"), "new\n").unwrap();

        let found = state(std::slice::from_ref(&dir));

        assert_eq!(
            found.status(&dir.join("src/kept.rs")),
            Some(Status::Modified)
        );
        assert_eq!(
            found.status(&dir.join("src/new.rs")),
            Some(Status::Untracked)
        );
        assert_eq!(
            found.folder_status(&dir.join("src")),
            Some(Status::Modified)
        );
        assert_eq!(found.folder_status(&dir), Some(Status::Modified));
        assert_eq!(
            found.branch(&dir).map(|branch| branch.name.as_str()),
            Some("trunk")
        );
        // The watcher follows the git directory, which git reports canonically.
        let git_dir = dir.canonicalize().unwrap().join(".git");
        assert!(found.git_dirs().any(|path| path == &git_dir));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn hunk_headers_become_added_modified_and_deleted_runs() {
        let diff = "\
diff --git a/a.rs b/a.rs
index aaa..bbb 100644
--- a/a.rs
+++ b/a.rs
@@ -2 +2 @@
-two
+changed
@@ -4,0 +5,2 @@
+new one
+new two
@@ -8 +8,0 @@
-gone
";

        assert_eq!(
            parse_hunks(diff),
            vec![
                LineChange {
                    line: 2,
                    count: 1,
                    kind: LineKind::Modified,
                },
                LineChange {
                    line: 5,
                    count: 2,
                    kind: LineKind::Added,
                },
                // The removal sits after the line above it.
                LineChange {
                    line: 8,
                    count: 0,
                    kind: LineKind::Deleted,
                },
            ]
        );
    }

    #[test]
    fn a_header_without_counts_covers_one_line() {
        assert_eq!(
            parse_hunks("@@ -1 +7 @@"),
            vec![LineChange {
                line: 7,
                count: 1,
                kind: LineKind::Modified,
            }]
        );
        // A removal above the first line is recorded against line zero.
        assert_eq!(
            parse_hunks("@@ -1,2 +0,0 @@"),
            vec![LineChange {
                line: 0,
                count: 0,
                kind: LineKind::Deleted,
            }]
        );
    }

    #[test]
    fn changed_runs_mark_one_gutter_line_each() {
        let text = "one\ntwo\nthree\nfour\n";
        let changes = [
            LineChange {
                line: 2,
                count: 1,
                kind: LineKind::Added,
            },
            LineChange {
                line: 3,
                count: 2,
                kind: LineKind::Modified,
            },
            // A removal after line two sits on the top edge of line three.
            LineChange {
                line: 2,
                count: 0,
                kind: LineKind::Deleted,
            },
        ];

        assert_eq!(
            gutter_lines(&changes, text),
            vec![
                (2, LineKind::Added),
                (3, LineKind::Modified),
                (4, LineKind::Modified),
            ],
            "the removal shares line three with the change, and one mark wins"
        );
    }

    #[test]
    fn a_deletion_marks_the_line_it_preceded() {
        let deleted = |line| LineChange {
            line,
            count: 0,
            kind: LineKind::Deleted,
        };

        // A trailing newline leaves an empty last line to mark.
        assert_eq!(
            gutter_lines(&[deleted(2)], "a\nb\nc\nd\n"),
            vec![(3, LineKind::Deleted)]
        );
        // A removal above the first line marks the top of line one.
        assert_eq!(
            gutter_lines(&[deleted(0)], "one\ntwo\n"),
            vec![(1, LineKind::Deleted)]
        );
        // Past the end, the mark clamps to the last line the buffer holds.
        assert_eq!(
            gutter_lines(&[deleted(5)], "a\nb"),
            vec![(2, LineKind::Deleted)]
        );
    }

    #[test]
    fn an_uncommitted_file_is_one_addition() {
        assert_eq!(
            line_changes(
                Path::new("/nowhere/a.rs"),
                "one\ntwo\n",
                Some(Status::Untracked)
            ),
            vec![LineChange {
                line: 1,
                count: 2,
                kind: LineKind::Added,
            }]
        );
        assert!(line_changes(Path::new("/nowhere/a.rs"), "", Some(Status::Untracked)).is_empty());
        // A rename has no committed text at its new path, so nothing is guessed.
        assert!(
            line_changes(Path::new("/nowhere/a.rs"), "one\n", Some(Status::Renamed)).is_empty()
        );
    }

    #[test]
    fn reads_line_changes_from_a_real_working_tree() {
        let dir = std::env::temp_dir().join(format!("app-git-lines-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.rs"), "one\ntwo\nthree\n").unwrap();

        let committed = git(&dir, &["init", "-q", "-b", "trunk"])
            && git(&dir, &["add", "."])
            && git(
                &dir,
                &[
                    "-c",
                    "user.email=a@b",
                    "-c",
                    "user.name=a",
                    "commit",
                    "-qm",
                    "init",
                ],
            );
        if !committed {
            let _ = std::fs::remove_dir_all(&dir);
            return;
        }

        let path = dir.join("a.rs");
        let modified = line_changes(
            &path,
            "one\nchanged\nthree\nadded\n",
            Some(Status::Modified),
        );
        assert_eq!(
            modified,
            vec![
                LineChange {
                    line: 2,
                    count: 1,
                    kind: LineKind::Modified,
                },
                LineChange {
                    line: 4,
                    count: 1,
                    kind: LineKind::Added,
                },
            ]
        );

        let removed = line_changes(&path, "one\nthree\n", Some(Status::Modified));
        assert_eq!(
            removed,
            vec![LineChange {
                line: 1,
                count: 0,
                kind: LineKind::Deleted,
            }]
        );

        // A buffer the commit already holds has nothing to mark.
        assert!(line_changes(&path, "one\ntwo\nthree\n", Some(Status::Modified)).is_empty());

        let _ = std::fs::remove_dir_all(&dir);
    }
}
