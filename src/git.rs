//! Working-tree status for the sidebar tree.
//!
//! The app needs a file's state, not repository objects, so this shells out to
//! `git status` instead of linking a git library: macOS always ships a usable
//! `git`, and the short calls it makes stay off the main thread.
//!
//! GPUI-free on purpose: [`statuses`] takes the workspace folders and returns
//! what the tree renderer queries by path.

use std::collections::HashMap;
use std::ffi::OsStr;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::process::Command;

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

/// Changed files and, for every folder, the most serious change beneath it,
/// so a collapsed folder can still show what is in it.
#[derive(Debug, Default)]
pub(crate) struct Statuses {
    files: HashMap<PathBuf, Status>,
    folders: HashMap<PathBuf, Status>,
}

impl Statuses {
    pub(crate) fn status(&self, path: &Path) -> Option<Status> {
        self.files.get(path).copied()
    }

    /// The most serious change somewhere beneath `path`, for a folder row.
    pub(crate) fn folder_status(&self, path: &Path) -> Option<Status> {
        self.folders.get(path).copied()
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

/// Reads the status of every workspace folder. A folder outside a repository
/// contributes nothing; a folder inside one reports only its own subtree.
pub(crate) fn statuses(roots: &[PathBuf]) -> Statuses {
    let mut statuses = Statuses::default();
    for root in roots {
        let Some(prefix) = show_prefix(root) else {
            continue;
        };
        let Some(output) = run_status(root) else {
            continue;
        };
        parse_into(&output, root, &prefix, &mut statuses);
    }
    statuses
}

/// The folder's path inside its repository, such as `src/`, or empty at the
/// repository root. `git status` reports paths from the repository root, so
/// this is what turns them back into paths under the workspace folder. It also
/// fails outside a repository, which is how a plain folder is skipped.
fn show_prefix(root: &Path) -> Option<Vec<u8>> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["rev-parse", "--show-prefix"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let mut prefix = output.stdout;
    while prefix
        .last()
        .is_some_and(|byte| *byte == b'\n' || *byte == b'\r')
    {
        prefix.pop();
    }
    Some(prefix)
}

/// Runs `git status` in `root`. Porcelain v2 with `-z` keeps paths unambiguous
/// (no quoting, no newline guessing).
fn run_status(root: &Path) -> Option<Vec<u8>> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["status", "--porcelain=v2", "-z", "--untracked-files=normal"])
        .env("GIT_OPTIONAL_LOCKS", "0")
        .output()
        .ok()?;
    output.status.success().then_some(output.stdout)
}

/// Walks the NUL-separated records of `output`, recording every changed path
/// that sits under `root`.
fn parse_into(output: &[u8], root: &Path, prefix: &[u8], statuses: &mut Statuses) {
    let mut records = output.split(|byte| *byte == 0);
    while let Some(record) = records.next() {
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
        statuses.insert(root, relative, status);
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

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(records: &str) -> Statuses {
        let mut statuses = Statuses::default();
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
        let mut statuses = Statuses::default();
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

        let committed = git(&dir, &["init", "-q"])
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

        let found = statuses(std::slice::from_ref(&dir));

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

        let _ = std::fs::remove_dir_all(&dir);
    }
}
