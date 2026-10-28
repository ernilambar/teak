//! State for the transient surfaces: the one open picker (command palette,
//! quick open or Open Recent), the file index quick open searches, and the Find
//! panel.
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

/// What a picker is asking the user to choose.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum PickerKind {
    /// Every command, by name.
    Palette,
    /// Every indexed file, fuzzy-matched.
    Files,
    /// The folders and files opened lately.
    Recent,
}

/// The one list overlay that is open, if any, and the focus to restore.
pub(super) struct PickerState {
    pub(super) kind: PickerKind,
    pub(super) state: Entity<CommandState>,
    pub(super) return_focus: Option<FocusHandle>,
}

/// The picker's state, plus the finder's file index and the Find panel.
#[derive(Default)]
pub(super) struct Overlays {
    /// The open picker: palette, quick open or recent. One at a time.
    picker: Option<PickerState>,
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

    pub(super) fn picker(&self) -> Option<&PickerState> {
        self.picker.as_ref()
    }

    /// The palette's interaction state, when the palette is the open picker.
    #[cfg(test)]
    pub(super) fn palette(&self) -> Option<&Entity<CommandState>> {
        self.picker_of(PickerKind::Palette)
    }

    /// The finder's interaction state, when the finder is the open picker.
    #[cfg(test)]
    pub(super) fn quick_open(&self) -> Option<&Entity<CommandState>> {
        self.picker_of(PickerKind::Files)
    }

    #[cfg(test)]
    fn picker_of(&self, kind: PickerKind) -> Option<&Entity<CommandState>> {
        self.picker
            .as_ref()
            .filter(|picker| picker.kind == kind)
            .map(|picker| &picker.state)
    }

    #[cfg(test)]
    pub(super) fn is_palette_open(&self) -> bool {
        self.is_picker_open(PickerKind::Palette)
    }

    #[cfg(test)]
    pub(super) fn is_quick_open_open(&self) -> bool {
        self.is_picker_open(PickerKind::Files)
    }

    pub(super) fn is_picker_open(&self, kind: PickerKind) -> bool {
        self.picker
            .as_ref()
            .is_some_and(|picker| picker.kind == kind)
    }

    pub(super) fn any_picker_open(&self) -> bool {
        self.picker.is_some()
    }

    /// Mounts a picker, remembering `return_focus` for the close.
    pub(super) fn set_picker(
        &mut self,
        kind: PickerKind,
        state: Entity<CommandState>,
        return_focus: Option<FocusHandle>,
    ) {
        self.picker = Some(PickerState {
            kind,
            state,
            return_focus,
        });
    }

    /// Unmounts `kind`'s picker, handing back the focus to restore. `None` when
    /// a different picker is open by now, which is what keeps an action that
    /// opened one from being closed by the one it came from.
    pub(super) fn take_picker(
        &mut self,
        kind: PickerKind,
    ) -> Option<(Entity<CommandState>, Option<FocusHandle>)> {
        if self.picker.as_ref()?.kind != kind {
            return None;
        }
        let picker = self.picker.take()?;
        Some((picker.state, picker.return_focus))
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
