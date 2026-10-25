//! The window's non-content state: the native title, the close guard, and the
//! status-bar readouts.

use gpui_kit::component::input::EditorState;
use gpui_kit::*;

use crate::app::WINDOW_TITLE;

/// Window chrome kept on the app so `render` and the close callbacks can reach
/// it without threading a `Window`.
pub(super) struct Chrome {
    /// The window title currently applied, so it is only pushed to the platform
    /// when the focused tab changes.
    title: String,
    /// Set once the user has confirmed discarding unsaved work in the quit or
    /// window-close alert, so the follow-up close callback stops re-prompting.
    allow_close: bool,
    /// Label for a background task in progress, shown with a spinner in the
    /// status bar. `None` when idle.
    busy: Option<SharedString>,
    cursor_line: usize,
    cursor_col: usize,
    /// Characters selected across every cursor, 0 when nothing is selected.
    selected: usize,
}

impl Chrome {
    pub(super) fn new() -> Self {
        Self {
            title: WINDOW_TITLE.to_string(),
            allow_close: false,
            busy: None,
            cursor_line: 1,
            cursor_col: 1,
            selected: 0,
        }
    }

    /// Records `title`, reporting whether it differs from the one already
    /// applied so the caller only pushes to the platform on a real change.
    pub(super) fn title_changed(&mut self, title: &str) -> bool {
        if self.title == title {
            return false;
        }
        self.title = title.to_string();
        true
    }

    pub(super) fn allow_close(&self) -> bool {
        self.allow_close
    }

    pub(super) fn set_allow_close(&mut self, allow_close: bool) {
        self.allow_close = allow_close;
    }

    pub(super) fn busy(&self) -> Option<&SharedString> {
        self.busy.as_ref()
    }

    pub(super) fn set_busy(&mut self, busy: Option<SharedString>) {
        self.busy = busy;
    }

    /// The caret position shown in the status bar, 1-based.
    pub(super) fn cursor(&self) -> (usize, usize) {
        (self.cursor_line, self.cursor_col)
    }

    /// Characters selected, shown next to the caret position.
    pub(super) fn selected(&self) -> usize {
        self.selected
    }

    /// Reads the caret position and selection size from `state`.
    pub(super) fn sync_cursor(&mut self, state: &EditorState) {
        let position = state.cursor_position();
        self.cursor_line = position.line as usize + 1;
        self.cursor_col = position.character as usize + 1;
        let text = state.text();
        self.selected = state
            .selected_ranges()
            .iter()
            .map(|range| text.byte_to_char_idx(range.end) - text.byte_to_char_idx(range.start))
            .sum();
    }
}
