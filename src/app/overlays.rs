//! State for the transient surfaces: the mutually exclusive command palette and
//! quick-open finder, the file index the finder searches, and the Find panel.
//!
//! Opening and closing stays on the app type: mounting an overlay needs a
//! `Window` (`window.defer` for the focus request, `cx.spawn_in` for the index
//! walk), and a plain state group has no `Context`. This type owns the storage
//! and the focus-to-restore bookkeeping those methods move around.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use gpui_kit::component::{command::CommandState, input::InputState};
use gpui_kit::*;

use crate::search::SearchOptions;

/// The Find panel's text fields, alive while the panel is shown.
pub(super) struct FindFields {
    pub(super) query: Entity<InputState>,
    pub(super) scope: Entity<InputState>,
    pub(super) replace: Entity<InputState>,
}

/// The Find panel's text, kept while the panel is hidden.
#[derive(Clone, Default)]
pub(super) struct FindText {
    pub(super) query: String,
    pub(super) scope: String,
    pub(super) replace: String,
}

/// What the Find panel's status line reports.
#[derive(Default)]
pub(super) enum FindStatus {
    #[default]
    Idle,
    Searching,
    /// The last search's summary.
    Done(String),
    /// The query or the Where field does not compile; holds the reason.
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
    /// The Find panel's fields while it is shown.
    find: Option<FindFields>,
    /// The focus that was active when the Find panel opened, restored on close.
    find_return_focus: Option<FocusHandle>,
    /// The panel's text, kept while it is hidden and across relaunches.
    find_text: FindText,
    find_options: SearchOptions,
    find_status: FindStatus,
    /// Bumped by every new search; a search whose number no longer matches
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

    pub(super) fn find(&self) -> Option<&FindFields> {
        self.find.as_ref()
    }

    /// Shows the Find panel with `fields`, remembering `return_focus`.
    pub(super) fn set_find(&mut self, fields: FindFields, return_focus: Option<FocusHandle>) {
        self.find = Some(fields);
        self.find_return_focus = return_focus;
    }

    /// Hides the Find panel, handing back its fields and the focus to restore.
    pub(super) fn take_find(&mut self) -> Option<(FindFields, Option<FocusHandle>)> {
        let fields = self.find.take()?;
        Some((fields, self.find_return_focus.take()))
    }

    pub(super) fn find_text(&self) -> &FindText {
        &self.find_text
    }

    pub(super) fn set_find_text(&mut self, text: FindText) {
        self.find_text = text;
    }

    pub(super) fn find_options(&self) -> SearchOptions {
        self.find_options
    }

    pub(super) fn find_options_mut(&mut self) -> &mut SearchOptions {
        &mut self.find_options
    }

    pub(super) fn find_status(&self) -> &FindStatus {
        &self.find_status
    }

    pub(super) fn set_find_status(&mut self, status: FindStatus) {
        self.find_status = status;
    }

    pub(super) fn find_generation(&self) -> Arc<AtomicU64> {
        self.find_generation.clone()
    }

    /// Starts a new search generation, cancelling the previous one.
    pub(super) fn next_find_generation(&mut self) -> u64 {
        self.find_generation.fetch_add(1, Ordering::Relaxed) + 1
    }
}
