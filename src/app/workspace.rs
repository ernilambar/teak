//! The sidebar's workspace state: the open folders, the expanded set, the
//! cached directory listings and the flattened rows rendered from them.
//!
//! `rows` is derived from `roots` + `expanded`. Most methods rebuild it, but
//! `expand`, `forget` and `remap` do not: their callers batch several changes
//! and then call [`Workspace::reload`] once, which also re-reads the disk.
//! `TreeRow` and `FileClipboard` are leaf records and keep `pub(super)` fields.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use gpui_kit::component::resizable::ResizableState;
use gpui_kit::*;

use crate::paths::{DirEntry, RootConflict, read_dir, remap_prefix, root_conflict};
use crate::session::{SessionState, push_recent};

/// Sidebar width used when the session does not carry one.
const DEFAULT_SIDEBAR_WIDTH: u32 = 240;
/// Floor and ceiling for dragging the sidebar, shared with the workspace
/// layout's panel size range.
pub(super) const SIDEBAR_MIN_WIDTH: u32 = 160;
pub(super) const SIDEBAR_MAX_WIDTH: u32 = 480;

/// One flattened, currently-visible row of the sidebar tree. A root row is a
/// workspace folder itself; every other row is one of its entries.
pub(super) struct TreeRow {
    pub(super) path: PathBuf,
    pub(super) depth: usize,
    pub(super) is_dir: bool,
    pub(super) is_root: bool,
}

/// A pending Cut/Copy of a single tree entry, applied on the next Paste.
#[derive(Clone)]
pub(super) struct FileClipboard {
    pub(super) path: PathBuf,
    pub(super) cut: bool,
}

/// The outcome of [`Workspace::add_root`], so the caller can report why a folder
/// was refused without re-running the conflict check.
pub(super) enum AddRoot {
    Added,
    Duplicate,
    Overlap,
}

pub(super) struct Workspace {
    /// The workspace folders shown in the sidebar, in the order they were added.
    roots: Vec<PathBuf>,
    expanded: HashSet<PathBuf>,
    selected: Option<PathBuf>,
    /// Directory listings for every root and expanded folder, so rendering
    /// the tree never touches the disk. Invalidated on filesystem changes.
    dir_cache: HashMap<PathBuf, Vec<DirEntry>>,
    /// The flattened, visible rows derived from `dir_cache` + `expanded`,
    /// rebuilt only when the tree changes.
    rows: Vec<TreeRow>,
    /// Scroll position and extent of the virtualized sidebar list.
    scroll: UniformListScrollHandle,
    sidebar_visible: bool,
    /// Sidebar width in pixels, persisted to the session and applied to the
    /// workspace layout on each render.
    width: u32,
    /// Sizes for the resizable workspace panels. The sidebar's `.size()`
    /// preference only takes hold while its slot is unset, so a position flip
    /// clears this to re-seed it from [`Workspace::width`].
    panels: Entity<ResizableState>,
    clipboard: Option<FileClipboard>,
    /// Recently opened files and folders, newest first.
    recent: Vec<PathBuf>,
}

impl Workspace {
    /// Builds a workspace from `folders`, keeping the ones that exist and
    /// dropping duplicates. Every root starts expanded; the caller reloads once
    /// the app is assembled.
    pub(super) fn new(folders: Vec<PathBuf>, panels: Entity<ResizableState>) -> Self {
        let roots: Vec<PathBuf> =
            folders
                .into_iter()
                .filter(|path| path.is_dir())
                .fold(Vec::new(), |mut roots, root| {
                    if !roots.contains(&root) {
                        roots.push(root);
                    }
                    roots
                });
        let sidebar_visible = !roots.is_empty();
        let expanded = roots.iter().cloned().collect();

        Self {
            roots,
            expanded,
            selected: None,
            dir_cache: HashMap::new(),
            rows: Vec::new(),
            scroll: UniformListScrollHandle::default(),
            sidebar_visible,
            width: DEFAULT_SIDEBAR_WIDTH,
            panels,
            clipboard: None,
            recent: Vec::new(),
        }
    }

    pub(super) fn roots(&self) -> &[PathBuf] {
        &self.roots
    }

    pub(super) fn expanded(&self) -> &HashSet<PathBuf> {
        &self.expanded
    }

    pub(super) fn selected(&self) -> Option<&Path> {
        self.selected.as_deref()
    }

    pub(super) fn set_selected(&mut self, selected: Option<PathBuf>) {
        self.selected = selected;
    }

    pub(super) fn rows(&self) -> &[TreeRow] {
        &self.rows
    }

    pub(super) fn scroll(&self) -> &UniformListScrollHandle {
        &self.scroll
    }

    pub(super) fn panels(&self) -> &Entity<ResizableState> {
        &self.panels
    }

    pub(super) fn clipboard(&self) -> Option<&FileClipboard> {
        self.clipboard.as_ref()
    }

    pub(super) fn set_clipboard(&mut self, clipboard: Option<FileClipboard>) {
        self.clipboard = clipboard;
    }

    pub(super) fn is_expanded(&self, path: &Path) -> bool {
        self.expanded.contains(path)
    }

    pub(super) fn sidebar_visible(&self) -> bool {
        self.sidebar_visible
    }

    pub(super) fn toggle_sidebar_visible(&mut self) {
        self.sidebar_visible = !self.sidebar_visible;
    }

    pub(super) fn width(&self) -> u32 {
        self.width
    }

    pub(super) fn set_width(&mut self, width: u32) {
        self.width = width.clamp(SIDEBAR_MIN_WIDTH, SIDEBAR_MAX_WIDTH);
    }

    /// Clears the directory cache and re-reads every workspace tree from disk.
    /// Call after any filesystem change or when the roots change.
    pub(super) fn reload(&mut self) {
        self.dir_cache.clear();
        self.rebuild_rows();
    }

    /// Re-flattens the visible rows, reading and caching any expanded directory
    /// that is not cached yet. Cheap to call on every expand/collapse.
    fn rebuild_rows(&mut self) {
        let mut rows = std::mem::take(&mut self.rows);
        rows.clear();
        let roots = self.roots.clone();
        for root in &roots {
            rows.push(TreeRow {
                path: root.clone(),
                depth: 0,
                is_dir: true,
                is_root: true,
            });
            if self.expanded.contains(root) {
                self.cache_dir(root);
                self.collect_rows(root, 1, &mut rows);
            }
        }
        self.rows = rows;
    }

    fn cache_dir(&mut self, dir: &Path) {
        if !self.dir_cache.contains_key(dir) {
            self.dir_cache.insert(dir.to_path_buf(), read_dir(dir));
        }
    }

    /// Walks `dir`'s cached entries in display order, pushing a row per entry
    /// and descending into expanded folders.
    fn collect_rows(&mut self, dir: &Path, depth: usize, rows: &mut Vec<TreeRow>) {
        let Some(entries) = self.dir_cache.get(dir).cloned() else {
            return;
        };
        for entry in entries {
            rows.push(TreeRow {
                path: entry.path.clone(),
                depth,
                is_dir: entry.is_dir,
                is_root: false,
            });
            if entry.is_dir && self.expanded.contains(&entry.path) {
                self.cache_dir(&entry.path);
                self.collect_rows(&entry.path, depth + 1, rows);
            }
        }
    }

    /// Flips `path` between expanded and collapsed and re-flattens the rows.
    pub(super) fn toggle_expanded(&mut self, path: &Path) {
        if !self.expanded.remove(path) {
            self.expanded.insert(path.to_path_buf());
        }
        self.rebuild_rows();
    }

    /// Marks `path` expanded so a following [`Self::reload`] shows its children.
    pub(super) fn expand(&mut self, path: PathBuf) {
        self.expanded.insert(path);
    }

    /// Drops `path` and anything under it from the expansion state, and clears
    /// the selection when it pointed inside. Used when an entry is removed;
    /// call [`Self::reload`] afterwards.
    pub(super) fn forget(&mut self, path: &Path) {
        self.expanded
            .retain(|entry| entry.as_path() != path && !entry.starts_with(path));
        if self
            .selected
            .as_deref()
            .is_some_and(|selected| selected == path || selected.starts_with(path))
        {
            self.selected = None;
        }
    }

    /// Adds `path` as a workspace root, refusing duplicates and a folder that
    /// overlaps one already open.
    pub(super) fn add_root(&mut self, path: PathBuf) -> AddRoot {
        match root_conflict(&self.roots, &path) {
            Some(RootConflict::Duplicate) => return AddRoot::Duplicate,
            Some(RootConflict::Overlap) => return AddRoot::Overlap,
            None => {}
        }

        self.roots.push(path.clone());
        self.expanded.insert(path);
        self.sidebar_visible = true;
        self.reload();
        AddRoot::Added
    }

    /// Removes one workspace root and its subtree expansion state. Open tabs
    /// stay open, and the sidebar stays visible.
    pub(super) fn remove_root(&mut self, path: &Path) {
        self.roots.retain(|root| root != path);
        self.forget(path);
        self.reload();
    }

    /// Removes every workspace root. Open tabs stay open and the sidebar stays
    /// visible, showing its empty state.
    pub(super) fn close_all(&mut self) {
        self.roots.clear();
        self.expanded.clear();
        self.selected = None;
        self.reload();
    }

    /// Rewrites every stored path that sat under `old` to sit under `new`. Call
    /// [`Self::reload`] afterwards.
    pub(super) fn remap(&mut self, old: &Path, new: &Path) {
        let remap = |path: &PathBuf| remap_prefix(path, old, new).unwrap_or_else(|| path.clone());
        self.expanded = self.expanded.iter().map(remap).collect();
        self.selected = self.selected.as_ref().map(remap);
        self.roots = self.roots.iter().map(remap).collect();
    }

    /// Adopts the folders, expansion state, selection and sidebar geometry from
    /// a saved session, then rebuilds the rows.
    pub(super) fn recent(&self) -> &[PathBuf] {
        &self.recent
    }

    /// Seeds the recent list at launch, dropping entries that no longer exist.
    pub(super) fn set_recent(&mut self, mut recent: Vec<PathBuf>) {
        recent.retain(|path| path.exists());
        self.recent = recent;
    }

    pub(super) fn note_recent(&mut self, path: PathBuf) {
        push_recent(&mut self.recent, path);
    }

    pub(super) fn clear_recent(&mut self) {
        self.recent.clear();
    }

    pub(super) fn restore(&mut self, session: &SessionState) {
        self.roots = session.roots.iter().filter(|path| path.is_dir()).fold(
            Vec::new(),
            |mut roots, root| {
                if !roots.contains(root) {
                    roots.push(root.clone());
                }
                roots
            },
        );

        self.expanded = session
            .expanded
            .iter()
            .filter(|path| path.is_dir())
            .cloned()
            .collect();
        for root in &self.roots {
            self.expanded.insert(root.clone());
        }

        self.selected = session.selected_path.clone().filter(|path| path.exists());
        self.sidebar_visible = session.sidebar_visible;
        if let Some(width) = session.sidebar_width {
            self.set_width(width);
        }
        self.reload();
    }
}

#[cfg(test)]
mod tests {
    // These import their types explicitly rather than glob-importing `super`,
    // which would pull in GPUI's `test` macro and shadow `#[gpui_kit::test]`.
    use std::path::PathBuf;

    use gpui_kit::component::resizable::ResizableState;
    use gpui_kit::{AppContext as _, TestAppContext};

    use super::{AddRoot, Workspace};
    use crate::paths::file_name;

    /// An empty directory named uniquely per test.
    fn fixture(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("tinytext-ws-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    fn workspace(cx: &mut TestAppContext, folders: Vec<PathBuf>) -> Workspace {
        cx.update(|cx| {
            let panels = cx.new(|_| ResizableState::default());
            Workspace::new(folders, panels)
        })
    }

    /// The rows as (name, depth, is-root) triples, for compact assertions.
    fn shape(workspace: &Workspace) -> Vec<(String, usize, bool)> {
        workspace
            .rows()
            .iter()
            .map(|row| (file_name(&row.path), row.depth, row.is_root))
            .collect()
    }

    #[gpui_kit::test]
    async fn roots_are_laid_out_before_their_children(cx: &mut TestAppContext) {
        let root = fixture("rows");
        std::fs::create_dir_all(root.join("zeta")).unwrap();
        std::fs::create_dir_all(root.join("alpha")).unwrap();
        std::fs::write(root.join("b.txt"), "").unwrap();
        std::fs::write(root.join("a.txt"), "").unwrap();

        let mut workspace = workspace(cx, vec![root.clone()]);
        workspace.reload();

        assert_eq!(
            shape(&workspace),
            [
                (file_name(&root), 0, true),
                ("alpha".to_string(), 1, false),
                ("zeta".to_string(), 1, false),
                ("a.txt".to_string(), 1, false),
                ("b.txt".to_string(), 1, false),
            ]
        );

        std::fs::remove_dir_all(&root).unwrap();
    }

    #[gpui_kit::test]
    async fn collapse_hides_a_subtree_and_expand_brings_it_back(cx: &mut TestAppContext) {
        let root = fixture("collapse");
        std::fs::create_dir_all(root.join("sub")).unwrap();
        std::fs::write(root.join("sub").join("deep.txt"), "").unwrap();

        let mut workspace = workspace(cx, vec![root.clone()]);
        workspace.reload();
        let collapsed = shape(&workspace);

        workspace.toggle_expanded(&root.join("sub"));
        assert_eq!(
            shape(&workspace),
            [
                (file_name(&root), 0, true),
                ("sub".to_string(), 1, false),
                ("deep.txt".to_string(), 2, false),
            ]
        );

        workspace.toggle_expanded(&root.join("sub"));
        assert_eq!(shape(&workspace), collapsed);

        std::fs::remove_dir_all(&root).unwrap();
    }

    #[gpui_kit::test]
    async fn add_root_reports_duplicates_and_overlaps(cx: &mut TestAppContext) {
        let root = fixture("add");
        let nested = root.join("nested");
        std::fs::create_dir_all(&nested).unwrap();
        let other = fixture("add-other");

        let mut workspace = workspace(cx, Vec::new());
        assert!(!workspace.sidebar_visible());

        assert!(matches!(workspace.add_root(root.clone()), AddRoot::Added));
        assert_eq!(workspace.roots(), std::slice::from_ref(&root));
        assert!(
            workspace.sidebar_visible(),
            "adding the first folder reveals the sidebar"
        );

        assert!(matches!(
            workspace.add_root(root.clone()),
            AddRoot::Duplicate
        ));
        assert!(
            matches!(workspace.add_root(nested.clone()), AddRoot::Overlap),
            "a folder inside an open root overlaps it"
        );
        assert_eq!(workspace.roots().len(), 1, "rejected folders are not added");

        assert!(matches!(workspace.add_root(other.clone()), AddRoot::Added));
        assert_eq!(workspace.roots().len(), 2);

        std::fs::remove_dir_all(&root).unwrap();
        std::fs::remove_dir_all(&other).unwrap();
    }

    #[gpui_kit::test]
    async fn remap_moves_roots_expansion_and_selection_together(cx: &mut TestAppContext) {
        let root = fixture("remap");
        std::fs::create_dir_all(root.join("sub")).unwrap();
        std::fs::write(root.join("sub").join("file.txt"), "").unwrap();

        let mut workspace = workspace(cx, vec![root.clone()]);
        workspace.reload();
        workspace.toggle_expanded(&root.join("sub"));
        workspace.set_selected(Some(root.join("sub").join("file.txt")));

        let renamed = root.with_file_name("remap-renamed");
        workspace.remap(&root, &renamed);

        assert_eq!(workspace.roots(), std::slice::from_ref(&renamed));
        assert!(workspace.is_expanded(&renamed.join("sub")));
        assert_eq!(
            workspace.selected(),
            Some(renamed.join("sub").join("file.txt").as_path())
        );

        std::fs::remove_dir_all(&root).unwrap();
    }
}
