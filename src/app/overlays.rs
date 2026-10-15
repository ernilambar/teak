//! State for the mutually exclusive overlay surfaces: the command palette, the
//! quick-open finder and the file index the finder searches.
//!
//! Opening and closing stays on `TinytextApp`: mounting an overlay needs a
//! `Window` (`window.defer` for the focus request, `cx.spawn_in` for the index
//! walk), and a plain state group has no `Context`. This type owns the storage
//! and the focus-to-restore bookkeeping those methods move around.

use std::path::PathBuf;

use gpui_kit::component::command::CommandState;
use gpui_kit::*;

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
}
