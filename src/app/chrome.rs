//! The window's non-content state: the native title, the close guard, and the
//! status-bar readouts.

use gpui_kit::component::input::{EditorState, Rope};
use gpui_kit::*;

use crate::app::WINDOW_TITLE;

/// Above this many characters the status bar shows only the character count.
/// Scanning a very large document for words on every edit is not worth it.
pub(super) const MAX_COUNT_CHARS: usize = 1_000_000;

/// The word and character totals of a document. Words are runs of letters,
/// digits, `_` or `'`, so `snake_case` and `don't` each stay one word.
pub(super) fn count_text(text: &Rope) -> (usize, usize) {
    let chars = text.len_chars();
    if chars > MAX_COUNT_CHARS {
        return (0, chars);
    }

    let mut words = 0;
    let mut in_word = false;
    for c in text.chars() {
        let word = c.is_alphanumeric() || c == '_' || c == '\'';
        if word && !in_word {
            words += 1;
        }
        in_word = word;
    }
    (words, chars)
}

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
    /// `(words, chars)` for the active text tab. `None` for no tab or an image.
    counts: Option<(usize, usize)>,
    /// Set while paths from the file manager are dragged over the window.
    /// `Some(true)` means every dragged path is a folder, so the drop hint can
    /// say what will happen; `None` means no external drag is running.
    external_drag: Option<bool>,
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
            counts: None,
            external_drag: None,
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

    /// The active tab's `(words, chars)`, or `None` when it has no text.
    pub(super) fn counts(&self) -> Option<(usize, usize)> {
        self.counts
    }

    pub(super) fn set_counts(&mut self, counts: Option<(usize, usize)>) {
        self.counts = counts;
    }

    /// `None` when no external drag is running, `Some(folders_only)` while one
    /// is. `folders_only` decides whether the hint promises to add folders or
    /// merely to open what was dropped.
    pub(super) fn external_drag(&self) -> Option<bool> {
        self.external_drag
    }

    pub(super) fn set_external_drag(&mut self, drag: Option<bool>) {
        self.external_drag = drag;
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

#[cfg(test)]
mod tests {
    use gpui_kit::component::input::Rope;

    use super::count_text;

    #[test]
    fn counts_words_and_characters() {
        assert_eq!(count_text(&Rope::from("")), (0, 0));
        assert_eq!(count_text(&Rope::from("hello world")), (2, 11));
        assert_eq!(count_text(&Rope::from("one, two; three")), (3, 15));
        // `_` and `'` stay inside a word.
        assert_eq!(count_text(&Rope::from("snake_case don't")), (2, 16));
        assert_eq!(count_text(&Rope::from("one\n二\n")), (2, 6));
    }
}
