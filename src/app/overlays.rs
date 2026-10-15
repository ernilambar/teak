//! State for the mutually exclusive overlay surfaces: the command palette, the
//! quick-open finder, Find in Files, and the file indexes the last two search.
//!
//! Opening and closing stays on `TinytextApp`: mounting an overlay needs a
//! `Window` (`window.defer` for the focus request, `cx.spawn_in` for the index
//! walk), and a plain state group has no `Context`. This type owns the storage
//! and the focus-to-restore bookkeeping those methods move around.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use gpui_kit::component::command::CommandState;
use gpui_kit::*;

use crate::search::{SearchOptions, SearchResults};

/// Where the current Find in Files query stands.
#[derive(Default)]
pub(super) enum FindOutcome {
    /// No query yet.
    #[default]
    Idle,
    Searching,
    Done(SearchResults),
    /// The regex does not compile; holds the reason.
    Invalid(String),
}

/// The palette's and the finder's state, plus the focus each returns to.
#[derive(Default)]
pub(super) struct Overlays {
    /// The command palette's interaction state while the overlay is open.
    palette: Option<Entity<CommandState>>,
    /// The focus that was active when the palette opened, restored on close.
    palette_return_focus: Option<FocusHandle>,
    /// The quick-open finder's interaction state while the overlay is open.
    quick_open: Option<Entity<CommandState>>,
    /// The focus that was active when the finder opened, restored on close.
    quick_open_return_focus: Option<FocusHandle>,
    /// The quick-open file index: every file under the workspace roots, walked
    /// on the last finder open. Cached until it is rebuilt on the next open.
    open_paths: Vec<PathBuf>,
    /// Find in Files' interaction state while the overlay is open.
    find: Option<Entity<CommandState>>,
    /// The focus that was active when Find in Files opened, restored on close.
    find_return_focus: Option<FocusHandle>,
    /// The files Find in Files searches, walked on each open.
    find_files: Arc<Vec<PathBuf>>,
    find_query: String,
    /// Kept across opens, like most editors' search toggles.
    find_options: SearchOptions,
    find_outcome: FindOutcome,
    /// Bumped by every new query; a search whose number no longer matches
    /// stops early and drops its results.
    find_generation: Arc<AtomicU64>,
}

impl Overlays {
    pub(super) fn new() -> Self {
        Self::default()
    }

    pub(super) fn palette(&self) -> Option<&Entity<CommandState>> {
        self.palette.as_ref()
    }

    pub(super) fn quick_open(&self) -> Option<&Entity<CommandState>> {
        self.quick_open.as_ref()
    }

    pub(super) fn is_palette_open(&self) -> bool {
        self.palette.is_some()
    }

    pub(super) fn is_quick_open_open(&self) -> bool {
        self.quick_open.is_some()
    }

    /// Mounts the palette, remembering `return_focus` for the close.
    pub(super) fn set_palette(
        &mut self,
        state: Entity<CommandState>,
        return_focus: Option<FocusHandle>,
    ) {
        self.palette = Some(state);
        self.palette_return_focus = return_focus;
    }

    /// Mounts the finder, remembering `return_focus` for the close.
    pub(super) fn set_quick_open(
        &mut self,
        state: Entity<CommandState>,
        return_focus: Option<FocusHandle>,
    ) {
        self.quick_open = Some(state);
        self.quick_open_return_focus = return_focus;
    }

    /// Unmounts the palette, handing back the focus to restore. Both are taken
    /// together so the caller cannot restore a focus left from an earlier open.
    pub(super) fn take_palette(&mut self) -> Option<(Entity<CommandState>, Option<FocusHandle>)> {
        let state = self.palette.take()?;
        Some((state, self.palette_return_focus.take()))
    }

    /// Unmounts the finder, handing back the focus to restore.
    pub(super) fn take_quick_open(
        &mut self,
    ) -> Option<(Entity<CommandState>, Option<FocusHandle>)> {
        let state = self.quick_open.take()?;
        Some((state, self.quick_open_return_focus.take()))
    }

    pub(super) fn open_paths(&self) -> &[PathBuf] {
        &self.open_paths
    }

    pub(super) fn set_open_paths(&mut self, paths: Vec<PathBuf>) {
        self.open_paths = paths;
    }

    pub(super) fn find(&self) -> Option<&Entity<CommandState>> {
        self.find.as_ref()
    }

    pub(super) fn is_find_open(&self) -> bool {
        self.find.is_some()
    }

    /// Mounts Find in Files with an empty query, remembering `return_focus`.
    pub(super) fn set_find(
        &mut self,
        state: Entity<CommandState>,
        return_focus: Option<FocusHandle>,
    ) {
        self.find = Some(state);
        self.find_return_focus = return_focus;
        self.find_query.clear();
        self.find_outcome = FindOutcome::Idle;
    }

    /// Unmounts Find in Files, cancelling any search still running.
    pub(super) fn take_find(&mut self) -> Option<(Entity<CommandState>, Option<FocusHandle>)> {
        let state = self.find.take()?;
        self.next_find_generation();
        Some((state, self.find_return_focus.take()))
    }

    pub(super) fn find_files(&self) -> Arc<Vec<PathBuf>> {
        self.find_files.clone()
    }

    pub(super) fn set_find_files(&mut self, files: Vec<PathBuf>) {
        self.find_files = Arc::new(files);
    }

    pub(super) fn find_query(&self) -> &str {
        &self.find_query
    }

    pub(super) fn set_find_query(&mut self, query: String) {
        self.find_query = query;
    }

    pub(super) fn find_options(&self) -> SearchOptions {
        self.find_options
    }

    pub(super) fn find_options_mut(&mut self) -> &mut SearchOptions {
        &mut self.find_options
    }

    pub(super) fn find_outcome(&self) -> &FindOutcome {
        &self.find_outcome
    }

    pub(super) fn set_find_outcome(&mut self, outcome: FindOutcome) {
        self.find_outcome = outcome;
    }

    pub(super) fn find_generation(&self) -> Arc<AtomicU64> {
        self.find_generation.clone()
    }

    /// Starts a new search generation, cancelling the previous one.
    pub(super) fn next_find_generation(&mut self) -> u64 {
        self.find_generation.fetch_add(1, Ordering::Relaxed) + 1
    }
}
