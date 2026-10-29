//! Reading the `.teak-workspace` files that name a window's folders.
//!
//! The file is a shareable folder list, never session state: tabs, geometry and
//! the expanded set stay in `session.json`. Anything else the file carries is
//! ignored, so a VS Code `.code-workspace` opens with its `folders` intact and
//! its `settings`, `tasks` and `launch` blocks silently unused.
//!
//! Paths in the file are relative to the file itself, so a workspace can be
//! committed and moved with the tree it describes.

use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Extension teak writes. Keep it in sync with the file-type help text.
pub(crate) const EXTENSION: &str = "teak-workspace";

/// Extension VS Code writes, accepted because the `folders` shape is the same.
pub(crate) const VSCODE_EXTENSION: &str = "code-workspace";

/// One `folders` entry: VS Code and Sublime write objects with a `path`; a bare
/// string is accepted too so hand-written files can stay short.
#[derive(Debug, Deserialize, Serialize)]
#[serde(untagged)]
enum FolderEntry {
    /// `{ "path": "../api", "name": "API" }`. Unknown keys are ignored on read,
    /// which is what lets a `.code-workspace` keep its per-folder patterns.
    Object {
        path: PathBuf,
    },
    Bare(PathBuf),
}

impl FolderEntry {
    fn into_path(self) -> PathBuf {
        match self {
            Self::Object { path } => path,
            Self::Bare(path) => path,
        }
    }
}

/// The part of a workspace file teak understands. Unknown top-level keys —
/// `$schema`, `settings`, `tasks`, `launch`, `extensions` — are tolerated
/// because deserialization ignores what it was not asked for.
#[derive(Debug, Default, Deserialize, Serialize)]
#[serde(default)]
struct WorkspaceDocument {
    folders: Vec<FolderEntry>,
}

/// True when `path` names a workspace file teak can open, whatever the case
/// macOS preserved from the file name.
pub(crate) fn is_workspace_path(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            extension.eq_ignore_ascii_case(EXTENSION)
                || extension.eq_ignore_ascii_case(VSCODE_EXTENSION)
        })
}

/// The name shown for a workspace in the title bar: the file's stem, so
/// `work.teak-workspace` reads as `work`.
pub(crate) fn label(path: &Path) -> String {
    path.file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

/// The folders a workspace file declares, absolute, folded, with the entries
/// that are no longer directories dropped. The workspace file itself is the
/// source of truth for the set, so this replaces whatever folders a window had.
pub(crate) fn load(path: &Path) -> Result<Vec<PathBuf>, String> {
    let data = std::fs::read_to_string(path)
        .map_err(|error| format!("Could not read {}: {error}", path.display()))?;
    let base = base_dir(path);
    let folders = parse(&data, &base)?;
    Ok(fold(
        folders
            .into_iter()
            .filter(|folder| folder.is_dir())
            .collect(),
    ))
}

/// Parses a workspace document, resolving every folder against `base`. Kept
/// separate from [`load`] so the parse can be tested without a filesystem.
pub(crate) fn parse(data: &str, base: &Path) -> Result<Vec<PathBuf>, String> {
    let document: WorkspaceDocument = serde_json::from_str(data)
        .map_err(|error| format!("Could not parse workspace file: {error}"))?;
    Ok(document
        .folders
        .into_iter()
        .map(|entry| resolve(base, &entry.into_path()))
        .collect())
}

/// Absolutizes `path` against `base` and folds `.` and `..` lexically, without
/// touching the disk, so a workspace saved in one location and opened from
/// another compares cleanly. Symlinks are left alone.
fn resolve(base: &Path, path: &Path) -> PathBuf {
    let joined = if path.is_absolute() {
        path.to_path_buf()
    } else {
        base.join(path)
    };
    absolute(&joined)
}

/// Makes `path` absolute and removes `.` and `..` components. A `..` that would
/// climb above the root stays at the root, as the filesystem resolves it.
fn absolute(path: &Path) -> PathBuf {
    let joined = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .map(|cwd| cwd.join(path))
            .unwrap_or_else(|_| path.to_path_buf())
    };

    let mut normalized = PathBuf::new();
    for component in joined.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                normalized.pop();
            }
            _ => normalized.push(component.as_os_str()),
        }
    }
    normalized
}

/// The directory a workspace file's relative paths are resolved against.
fn base_dir(path: &Path) -> PathBuf {
    let absolute = absolute(path);
    absolute.parent().map(Path::to_path_buf).unwrap_or(absolute)
}

/// Drops duplicates and every folder that sits inside another, keeping the
/// outermost one whichever order the file lists them in. Teak's tree gives each
/// root its own identity, so a nested root is already reachable by expanding
/// its parent and would otherwise be listed, watched and searched twice.
pub(crate) fn fold(folders: Vec<PathBuf>) -> Vec<PathBuf> {
    let mut kept: Vec<PathBuf> = Vec::new();
    for folder in folders {
        if kept.iter().any(|root| folder.starts_with(root)) {
            continue;
        }
        // A parent arriving after a child replaces it, so the result never
        // depends on the order the file happens to list entries in.
        kept.retain(|root| !root.starts_with(&folder));
        kept.push(folder);
    }
    kept
}

/// Writes `folders` to `path`, relative to the file so the workspace can move
/// with the tree it describes. Written to a temporary file first so a failed
/// write never truncates an existing workspace.
pub(crate) fn save(path: &Path, folders: &[PathBuf]) -> Result<(), String> {
    let base = base_dir(path);
    let document = WorkspaceDocument {
        folders: folders
            .iter()
            .map(|folder| FolderEntry::Object {
                path: relative_from(&base, folder).unwrap_or_else(|| folder.clone()),
            })
            .collect(),
    };
    let mut json = serde_json::to_string_pretty(&document)
        .map_err(|error| format!("Could not write {}: {error}", path.display()))?;
    json.push('\n');

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("Could not create {}: {error}", parent.display()))?;
    }
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| format!("workspace.{EXTENSION}"));
    let temp = path.with_file_name(format!("{name}.tmp"));
    std::fs::write(&temp, json)
        .map_err(|error| format!("Could not write {}: {error}", temp.display()))?;
    std::fs::rename(&temp, path)
        .map_err(|error| format!("Could not replace {}: {error}", path.display()))
}

/// `target` expressed relative to `base`, for the workspace file. `None` only
/// when the two paths do not share a root, which on macOS means never; the
/// caller then writes the absolute path.
fn relative_from(base: &Path, target: &Path) -> Option<PathBuf> {
    let base: Vec<Component> = base.components().collect();
    let target: Vec<Component> = target.components().collect();
    if base.first() != target.first() {
        return None;
    }
    let common = base
        .iter()
        .zip(&target)
        .take_while(|(left, right)| left == right)
        .count();

    let mut relative = PathBuf::new();
    for _ in common..base.len() {
        relative.push("..");
    }
    for component in &target[common..] {
        relative.push(component.as_os_str());
    }
    if relative.as_os_str().is_empty() {
        relative.push(".");
    }
    Some(relative)
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use super::{EXTENSION, fold, is_workspace_path, label, load, parse, save};

    fn fixture(name: &str) -> PathBuf {
        let root =
            std::env::temp_dir().join(format!("teak-workspace-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    #[test]
    fn workspace_extensions_are_recognized() {
        assert!(is_workspace_path(Path::new("/w/work.teak-workspace")));
        assert!(is_workspace_path(Path::new("/w/work.code-workspace")));
        assert!(is_workspace_path(Path::new("/w/work.TEAK-WORKSPACE")));
        assert!(!is_workspace_path(Path::new("/w/work.json")));
        assert!(!is_workspace_path(Path::new("/w/teak-workspace")));
    }

    #[test]
    fn vscode_files_keep_their_folders_and_drop_the_rest() {
        let folders = parse(
            r#"{
                "$schema": "vscode://schemas/workspace",
                "folders": [
                    { "path": "..", "name": "Root" },
                    { "path": "/abs/other" }
                ],
                "settings": { "editor.tabSize": 2 },
                "tasks": { "version": "2.0.0" },
                "launch": {}
            }"#,
            Path::new("/work/project"),
        )
        .unwrap();

        assert_eq!(
            folders,
            [PathBuf::from("/work"), PathBuf::from("/abs/other")]
        );
    }

    #[test]
    fn bare_strings_and_relative_paths_resolve_against_the_file() {
        let folders = parse(
            r#"{ "folders": ["src", "./tests", "../api"] }"#,
            Path::new("/work/project"),
        )
        .unwrap();

        assert_eq!(
            folders,
            [
                PathBuf::from("/work/project/src"),
                PathBuf::from("/work/project/tests"),
                PathBuf::from("/work/api"),
            ]
        );
    }

    #[test]
    fn a_document_without_folders_is_empty_not_an_error() {
        assert!(
            parse(r#"{ "settings": {} }"#, Path::new("/work"))
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn label_drops_the_extension() {
        assert_eq!(label(Path::new("/w/work.teak-workspace")), "work");
        assert_eq!(label(Path::new("/w/team.code-workspace")), "team");
    }

    #[test]
    fn malformed_json_names_the_problem() {
        let error = parse(r#"{ "folders": ["#, Path::new("/work")).unwrap_err();
        assert!(error.starts_with("Could not parse workspace file: "));
    }

    #[test]
    fn fold_keeps_the_outermost_folder_in_either_order() {
        let folders =
            |paths: &[&str]| -> Vec<PathBuf> { fold(paths.iter().map(PathBuf::from).collect()) };

        assert_eq!(folders(&["/a", "/a/b", "/a/b/c"]), [PathBuf::from("/a")]);
        assert_eq!(folders(&["/a/b/c", "/a/b", "/a"]), [PathBuf::from("/a")]);
        assert_eq!(
            folders(&["/a", "/a/b", "/a", "/b", "/a/b/c"]),
            [PathBuf::from("/a"), PathBuf::from("/b")]
        );
        // The comparison is component-wise, so a sibling that only shares a
        // prefix is not swallowed.
        assert_eq!(
            folders(&["/a/b", "/a/bc"]),
            [PathBuf::from("/a/b"), PathBuf::from("/a/bc")]
        );
    }

    #[test]
    fn load_resolves_folds_and_skips_missing_folders() {
        let root = fixture("load");
        std::fs::create_dir_all(root.join("app").join("src")).unwrap();
        std::fs::create_dir_all(root.join("api")).unwrap();
        let file = root.join(format!("work.{EXTENSION}"));
        std::fs::write(
            &file,
            r#"{ "folders": [
                { "path": "app" },
                { "path": "app/src" },
                { "path": "app" },
                { "path": "api" },
                { "path": "gone" }
            ] }"#,
        )
        .unwrap();

        assert_eq!(
            load(&file).unwrap(),
            [root.join("app"), root.join("api")],
            "the nested, duplicate and missing folders are dropped"
        );

        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn load_reports_an_unreadable_file() {
        let error = load(Path::new("/nonexistent/work.teak-workspace")).unwrap_err();
        assert!(error.starts_with("Could not read "), "{error}");
    }

    #[test]
    fn relative_entries_are_absolute_in_the_result() {
        let root = fixture("absolute");
        let file = root.join(format!("work.{EXTENSION}"));
        std::fs::write(&file, r#"{ "folders": ["."] }"#).unwrap();

        let folders = load(&file).unwrap();
        assert_eq!(folders.len(), 1);
        assert_eq!(folders[0], root);
        assert!(folders[0].is_absolute());

        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn save_writes_relative_paths_that_load_back() {
        let root = fixture("save");
        std::fs::create_dir_all(root.join("app")).unwrap();
        std::fs::create_dir_all(root.join("api")).unwrap();
        let file = root.join(format!("work.{EXTENSION}"));

        save(&file, &[root.join("app"), root.join("api")]).unwrap();

        let written = std::fs::read_to_string(&file).unwrap();
        assert!(written.contains(r#""path": "app""#), "{written}");
        assert!(written.contains(r#""path": "api""#), "{written}");
        assert!(
            !written.contains(&root.display().to_string()),
            "the folders are written relative to the file:\n{written}"
        );
        assert_eq!(
            load(&file).unwrap(),
            [root.join("app"), root.join("api")],
            "a saved workspace reopens with the same folders"
        );

        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn save_climbs_out_of_the_file_directory_for_a_sibling_folder() {
        let root = fixture("save-sibling");
        std::fs::create_dir_all(root.join("repo")).unwrap();
        std::fs::create_dir_all(root.join("shared")).unwrap();
        let file = root.join("repo").join(format!("work.{EXTENSION}"));

        save(&file, &[root.join("repo"), root.join("shared")]).unwrap();

        let written = std::fs::read_to_string(&file).unwrap();
        assert!(written.contains(r#""path": ".""#), "{written}");
        assert!(written.contains(r#""path": "../shared""#), "{written}");
        assert_eq!(
            load(&file).unwrap(),
            [root.join("repo"), root.join("shared")]
        );

        std::fs::remove_dir_all(root).unwrap();
    }
}
