use std::path::{Path, PathBuf};

const BUNDLE_ID: &str = "net.nilambar.tinytext";

/// `~/Library/Application Support/<bundle id>`, home of session and settings files.
pub(crate) fn support_dir() -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;
    Some(
        PathBuf::from(home)
            .join("Library")
            .join("Application Support")
            .join(BUNDLE_ID),
    )
}

pub(crate) fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_else(|| path.display().to_string())
}

/// A byte count for display, in the largest unit that keeps it at or above 1.
pub(crate) fn format_size(bytes: usize) -> String {
    const UNITS: [&str; 3] = ["KB", "MB", "GB"];
    if bytes < 1024 {
        return format!("{bytes} B");
    }
    let mut size = bytes as f64 / 1024.0;
    let mut unit = 0;
    while size >= 1024.0 && unit < UNITS.len() - 1 {
        size /= 1024.0;
        unit += 1;
    }
    format!("{size:.1} {}", UNITS[unit])
}

/// The path for display, with the home directory shortened to `~`.
pub(crate) fn display_path(path: &Path) -> String {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    abbreviate_home(path, home.as_deref())
}

fn abbreviate_home(path: &Path, home: Option<&Path>) -> String {
    match home.and_then(|home| path.strip_prefix(home).ok()) {
        Some(rest) if rest.as_os_str().is_empty() => "~".to_string(),
        Some(rest) => format!("~/{}", rest.display()),
        None => path.display().to_string(),
    }
}

/// A directory entry paired with the directory bit, so callers never have to
/// stat the path a second time.
#[derive(Clone)]
pub(crate) struct DirEntry {
    pub(crate) path: PathBuf,
    pub(crate) is_dir: bool,
}

pub(crate) fn read_dir(dir: &Path) -> Vec<DirEntry> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };

    let mut dirs = Vec::new();
    let mut files = Vec::new();

    for entry in entries.flatten() {
        let path = entry.path();
        let is_dir = entry.file_type().map(|kind| kind.is_dir()).unwrap_or(false);
        let info = DirEntry { path, is_dir };
        if is_dir {
            dirs.push(info);
        } else {
            files.push(info);
        }
    }

    dirs.sort_by(|a, b| a.path.cmp(&b.path));
    files.sort_by(|a, b| a.path.cmp(&b.path));
    dirs.extend(files);
    dirs
}

/// Directories whose contents never belong in the quick-open file index.
const IGNORED_DIRS: &[&str] = &[".git", "node_modules", "target", "__pycache__"];

/// Collects the files under every root with a breadth-first walk, stopping
/// once `limit` paths are collected. Dot directories and dependency/build
/// directories are skipped, and symlinks are not followed, so a symlinked
/// folder appears as a file entry instead of being walked.
pub(crate) fn collect_files(roots: &[PathBuf], limit: usize) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let mut queue: Vec<PathBuf> = roots.to_vec();

    while let Some(dir) = queue.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            if files.len() >= limit {
                return files;
            }
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            let path = entry.path();
            if kind.is_dir() {
                if is_ignored_dir(&path) {
                    continue;
                }
                queue.push(path);
            } else {
                files.push(path);
            }
        }
    }
    files
}

fn is_ignored_dir(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| IGNORED_DIRS.contains(&name) || name.starts_with('.'))
}

pub(crate) fn path_from_file_url(url: &str) -> Option<PathBuf> {
    let rest = url.strip_prefix("file://")?;
    let path = rest.strip_prefix("localhost").unwrap_or(rest);
    path.starts_with('/')
        .then(|| PathBuf::from(percent_decode(path)))
}

pub(crate) fn percent_decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;

    while index < bytes.len() {
        if bytes[index] == b'%'
            && index + 2 < bytes.len()
            && let (Some(high), Some(low)) =
                (hex_digit(bytes[index + 1]), hex_digit(bytes[index + 2]))
        {
            decoded.push((high << 4) | low);
            index += 3;
            continue;
        }
        decoded.push(bytes[index]);
        index += 1;
    }

    String::from_utf8_lossy(&decoded).into_owned()
}

pub(crate) fn hex_digit(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

/// A single path component usable as a new file or folder name. Rejects the
/// empty string, the dot entries, and anything containing a separator so the
/// name cannot escape the directory it is created in.
pub(crate) fn is_valid_entry_name(name: &str) -> bool {
    !name.is_empty() && name != "." && name != ".." && !name.contains('/') && !name.contains('\0')
}

/// Rewrites `path` when it is `from` or lives under it, mapping the prefix to
/// `to`. Returns `None` for unrelated paths so callers can keep them as-is.
pub(crate) fn remap_prefix(path: &Path, from: &Path, to: &Path) -> Option<PathBuf> {
    if path == from {
        return Some(to.to_path_buf());
    }
    path.strip_prefix(from).ok().map(|rest| to.join(rest))
}

/// Path shown relative to `root` when possible, otherwise absolute. Used by
/// "Copy Relative Path".
pub(crate) fn relative_display(path: &Path, root: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .display()
        .to_string()
}

/// Why a folder cannot be added as another workspace root.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum RootConflict {
    /// The folder is already an open root.
    Duplicate,
    /// The folder contains, or lives inside, an existing root.
    Overlap,
}

/// Classifies `candidate` against the open roots: `Duplicate` when it is
/// already a root, `Overlap` when it contains or lives inside one. Returns
/// `None` when the folder is independent and can be added. Comparison is
/// component-wise, so `/a/bc` never conflicts with `/a/b`.
pub(crate) fn root_conflict(roots: &[PathBuf], candidate: &Path) -> Option<RootConflict> {
    for root in roots {
        if root == candidate {
            return Some(RootConflict::Duplicate);
        }
        if candidate.starts_with(root) || root.starts_with(candidate) {
            return Some(RootConflict::Overlap);
        }
    }
    None
}

/// The open root that most closely contains `path`, when any. The deepest match
/// wins, so nested roots resolve to the innermost one.
pub(crate) fn nearest_root<'a>(roots: &'a [PathBuf], path: &Path) -> Option<&'a Path> {
    roots
        .iter()
        .filter(|root| path.starts_with(root))
        .max_by_key(|root| root.components().count())
        .map(PathBuf::as_path)
}

/// Copies a file, or a directory tree, to `to`. Directories are created and
/// filled recursively.
pub(crate) fn copy_entry(from: &Path, to: &Path) -> std::io::Result<()> {
    let metadata = std::fs::symlink_metadata(from)?;
    if metadata.is_dir() {
        std::fs::create_dir_all(to)?;
        for entry in std::fs::read_dir(from)? {
            let entry = entry?;
            copy_entry(&entry.path(), &to.join(entry.file_name()))?;
        }
    } else {
        std::fs::copy(from, to)?;
    }
    Ok(())
}

/// A sibling name for a duplicate of `path`, following the Finder convention:
/// `"main.rs"` becomes `"main copy.rs"`, then `"main copy 2.rs"`, and so on.
/// Directories keep their whole name (`"docs"` becomes `"docs copy"`).
pub(crate) fn duplicate_name(path: &Path) -> String {
    let is_dir = path.is_dir();
    let stem = if is_dir {
        file_name(path)
    } else {
        path.file_stem()
            .map(|stem| stem.to_string_lossy().into_owned())
            .unwrap_or_else(|| file_name(path))
    };
    let extension = if is_dir {
        None
    } else {
        path.extension()
            .map(|extension| extension.to_string_lossy().into_owned())
    };

    let candidate = |suffix: &str| match &extension {
        Some(extension) => format!("{stem} copy{suffix}.{extension}"),
        None => format!("{stem} copy{suffix}"),
    };

    let mut name = candidate("");
    let mut counter = 1;
    while path.with_file_name(&name).exists() {
        counter += 1;
        name = candidate(&format!(" {counter}"));
    }
    name
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_size_picks_unit() {
        assert_eq!(format_size(512), "512 B");
        assert_eq!(format_size(1536), "1.5 KB");
        assert_eq!(format_size(3 * 1024 * 1024), "3.0 MB");
    }

    #[test]
    fn file_name_returns_last_component() {
        assert_eq!(file_name(Path::new("/tmp/project/main.rs")), "main.rs");
        assert_eq!(file_name(Path::new("/")), "/");
    }

    #[test]
    fn abbreviate_home_replaces_home_prefix() {
        let home = Some(Path::new("/Users/me"));
        assert_eq!(
            abbreviate_home(Path::new("/Users/me/notes/a.txt"), home),
            "~/notes/a.txt"
        );
        assert_eq!(abbreviate_home(Path::new("/Users/me"), home), "~");
        assert_eq!(
            abbreviate_home(Path::new("/Users/me2/a.txt"), home),
            "/Users/me2/a.txt"
        );
        assert_eq!(abbreviate_home(Path::new("/tmp/a.txt"), None), "/tmp/a.txt");
    }

    #[test]
    fn read_dir_lists_directories_before_files_sorted() {
        let root = std::env::temp_dir().join(format!("tinytext-read-dir-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("zeta")).unwrap();
        std::fs::create_dir_all(root.join("alpha")).unwrap();
        std::fs::write(root.join("b.txt"), "").unwrap();
        std::fs::write(root.join("a.txt"), "").unwrap();

        let names: Vec<String> = read_dir(&root)
            .iter()
            .map(|entry| file_name(&entry.path))
            .collect();
        std::fs::remove_dir_all(&root).unwrap();

        assert_eq!(names, ["alpha", "zeta", "a.txt", "b.txt"]);
    }

    #[test]
    fn read_dir_returns_empty_for_missing_directory() {
        assert!(read_dir(Path::new("/nonexistent/tinytext")).is_empty());
    }

    #[test]
    fn collect_files_walks_files_and_skips_ignored_directories() {
        let root = std::env::temp_dir().join(format!("tinytext-collect-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("src/deep")).unwrap();
        std::fs::create_dir_all(root.join(".hidden")).unwrap();
        std::fs::create_dir_all(root.join("node_modules")).unwrap();
        std::fs::write(root.join("README.md"), "readme").unwrap();
        std::fs::write(root.join("src/a.rs"), "a").unwrap();
        std::fs::write(root.join("src/deep/b.rs"), "b").unwrap();
        std::fs::write(root.join(".hidden/c.txt"), "c").unwrap();
        std::fs::write(root.join("node_modules/d.txt"), "d").unwrap();

        let files = collect_files(std::slice::from_ref(&root), usize::MAX);
        let mut names: Vec<String> = files.iter().map(|path| file_name(path)).collect();
        names.sort();

        assert_eq!(names, ["README.md", "a.rs", "b.rs"]);

        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn collect_files_stops_at_the_limit() {
        let root = std::env::temp_dir().join(format!("tinytext-collect-2-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        for index in 0..5 {
            std::fs::write(root.join(format!("file-{index}.txt")), "").unwrap();
        }

        assert_eq!(collect_files(std::slice::from_ref(&root), 0).len(), 0);
        assert_eq!(collect_files(std::slice::from_ref(&root), 3).len(), 3);
        assert_eq!(collect_files(&[root], 5).len(), 5);
    }

    #[test]
    fn collect_files_tolerates_missing_roots() {
        assert!(collect_files(&[PathBuf::from("/nonexistent/tinytext")], usize::MAX).is_empty());
        assert!(collect_files(&[], usize::MAX).is_empty());
    }

    #[test]
    fn path_from_file_url_parses_local_paths() {
        assert_eq!(
            path_from_file_url("file:///Users/me/notes.txt"),
            Some(PathBuf::from("/Users/me/notes.txt"))
        );
        assert_eq!(
            path_from_file_url("file://localhost/Users/me/notes.txt"),
            Some(PathBuf::from("/Users/me/notes.txt"))
        );
        assert_eq!(
            path_from_file_url("file:///Users/me/My%20Notes.txt"),
            Some(PathBuf::from("/Users/me/My Notes.txt"))
        );
    }

    #[test]
    fn path_from_file_url_rejects_non_file_urls() {
        assert_eq!(path_from_file_url("https://example.com/a.txt"), None);
        assert_eq!(path_from_file_url("file://remote-host/a.txt"), None);
    }

    #[test]
    fn percent_decode_handles_valid_and_invalid_escapes() {
        assert_eq!(percent_decode("a%20b"), "a b");
        assert_eq!(percent_decode("%2f%2F"), "//");
        assert_eq!(percent_decode("%E2%9C%93"), "✓");
        assert_eq!(percent_decode("100%"), "100%");
        assert_eq!(percent_decode("%zz"), "%zz");
        assert_eq!(percent_decode("%2"), "%2");
    }

    #[test]
    fn valid_entry_names_reject_separators_and_dots() {
        assert!(is_valid_entry_name("main.rs"));
        assert!(is_valid_entry_name("My Notes"));
        assert!(!is_valid_entry_name(""));
        assert!(!is_valid_entry_name("."));
        assert!(!is_valid_entry_name(".."));
        assert!(!is_valid_entry_name("a/b"));
        assert!(!is_valid_entry_name("a\0b"));
    }

    #[test]
    fn remap_prefix_maps_self_and_descendants_only() {
        let from = Path::new("/tmp/project/src");
        let to = Path::new("/tmp/project/lib");
        assert_eq!(
            remap_prefix(from, from, to),
            Some(PathBuf::from("/tmp/project/lib"))
        );
        assert_eq!(
            remap_prefix(Path::new("/tmp/project/src/main.rs"), from, to),
            Some(PathBuf::from("/tmp/project/lib/main.rs"))
        );
        assert_eq!(
            remap_prefix(Path::new("/tmp/project/other"), from, to),
            None
        );
    }

    #[test]
    fn relative_display_strips_root_when_possible() {
        assert_eq!(
            relative_display(
                Path::new("/tmp/project/src/main.rs"),
                Path::new("/tmp/project")
            ),
            "src/main.rs"
        );
        assert_eq!(
            relative_display(Path::new("/elsewhere/main.rs"), Path::new("/tmp/project")),
            "/elsewhere/main.rs"
        );
    }

    #[test]
    fn root_conflict_flags_duplicates_and_overlaps() {
        let roots = vec![PathBuf::from("/tmp/project"), PathBuf::from("/opt/other")];

        assert_eq!(
            root_conflict(&roots, Path::new("/tmp/project")),
            Some(RootConflict::Duplicate)
        );
        assert_eq!(
            root_conflict(&roots, Path::new("/tmp/project/src")),
            Some(RootConflict::Overlap)
        );
        assert_eq!(
            root_conflict(&roots, Path::new("/tmp")),
            Some(RootConflict::Overlap)
        );
        assert_eq!(root_conflict(&roots, Path::new("/tmp/project-extra")), None);
        assert_eq!(root_conflict(&roots, Path::new("/tmp/elsewhere")), None);
        assert_eq!(root_conflict(&[], Path::new("/tmp/project")), None);
    }

    #[test]
    fn nearest_root_picks_the_deepest_match() {
        let roots = vec![
            PathBuf::from("/tmp/project"),
            PathBuf::from("/tmp/project/src"),
        ];

        assert_eq!(
            nearest_root(&roots, Path::new("/tmp/project/src/main.rs")),
            Some(Path::new("/tmp/project/src"))
        );
        assert_eq!(
            nearest_root(&roots, Path::new("/tmp/project/docs/a.md")),
            Some(Path::new("/tmp/project"))
        );
        assert_eq!(nearest_root(&roots, Path::new("/elsewhere/a.md")), None);
    }

    #[test]
    fn copy_entry_duplicates_a_directory_tree() {
        let root = std::env::temp_dir().join(format!("tinytext-copy-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let from = root.join("from");
        std::fs::create_dir_all(from.join("nested")).unwrap();
        std::fs::write(from.join("a.txt"), "a").unwrap();
        std::fs::write(from.join("nested/b.txt"), "b").unwrap();

        let to = root.join("to");
        copy_entry(&from, &to).unwrap();

        assert_eq!(std::fs::read_to_string(to.join("a.txt")).unwrap(), "a");
        assert_eq!(
            std::fs::read_to_string(to.join("nested/b.txt")).unwrap(),
            "b"
        );

        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn duplicate_name_follows_finder_convention() {
        let root = std::env::temp_dir().join(format!("tinytext-dup-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();

        let file = root.join("main.rs");
        std::fs::write(&file, "").unwrap();
        assert_eq!(duplicate_name(&file), "main copy.rs");

        std::fs::write(root.join("main copy.rs"), "").unwrap();
        assert_eq!(duplicate_name(&file), "main copy 2.rs");

        let dir = root.join("docs");
        std::fs::create_dir(&dir).unwrap();
        assert_eq!(duplicate_name(&dir), "docs copy");

        std::fs::remove_dir_all(&root).unwrap();
    }
}
