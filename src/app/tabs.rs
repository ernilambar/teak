use std::path::PathBuf;

use gpui_kit::component::{
    WindowExt as _,
    button::ButtonVariant,
    input::{EditorState, InputEvent, TabSize},
};
use gpui_kit::*;

use crate::CloseTab;
use crate::language::{editor_language_id, language_for};
use crate::paths::file_name;

use super::{ClosedTab, MAX_CLOSED_TABS, OpenTab, TinytextApp};

impl TinytextApp {
    pub(super) fn add_tab(
        &mut self,
        path: Option<PathBuf>,
        content: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let language: SharedString = path
            .as_deref()
            .map(language_for)
            .unwrap_or_else(|| "Plain Text".into());
        let title: SharedString = path
            .as_deref()
            .map(|path| file_name(path).into())
            .unwrap_or_else(|| "Untitled".into());

        let editor = cx.new(|cx| {
            EditorState::new(window, cx)
                .language(editor_language_id(&language))
                .default_value(content)
                .soft_wrap(self.settings.editor.soft_wrap())
                .show_whitespaces(self.settings.editor.show_whitespace())
                .tab_size(TabSize {
                    tab_size: self.settings.editor.tab_size(),
                    hard_tabs: self.settings.editor.hard_tabs(),
                })
        });

        let change_subscription = cx.subscribe(&editor, |this, editor, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                this.set_dirty(editor.entity_id(), true, cx);
            }
        });
        let cursor_subscription = cx.observe(&editor, |this, editor, cx| {
            this.sync_cursor(editor.entity_id(), cx);
        });

        self.tabs.push(OpenTab {
            path,
            title,
            language,
            language_override: false,
            dirty: false,
            editor: editor.clone(),
            _subscriptions: vec![change_subscription, cursor_subscription],
        });
        self.active_tab = Some(self.tabs.len() - 1);
        self.reveal_active_tab();

        editor.update(cx, |state, cx| state.focus(window, cx));
        let position = editor.read(cx).cursor_position();
        self.cursor_line = position.line as usize + 1;
        self.cursor_col = position.character as usize + 1;
        self.save_session();
        cx.notify();
    }

    pub(super) fn on_close_tab(
        &mut self,
        _: &CloseTab,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(ix) = self.active_tab {
            self.close_tab(ix, window, cx);
        }
    }

    /// Reveals the active tab in the horizontally scrolling tab strip. The
    /// strip only scrolls when tabs overflow; with few tabs this is a no-op.
    fn reveal_active_tab(&self) {
        if let Some(ix) = self.active_tab {
            self.tab_scroll.scroll_to_item(ix);
        }
    }

    pub(super) fn activate_tab(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.active_tab = Some(ix);
        self.reveal_active_tab();
        if let Some(tab) = self.tabs.get(ix) {
            let editor = tab.editor.clone();
            editor.update(cx, |state, cx| state.focus(window, cx));
            let position = editor.read(cx).cursor_position();
            self.cursor_line = position.line as usize + 1;
            self.cursor_col = position.character as usize + 1;
        }
        // Switching tabs is a natural cue to pick up external filesystem
        // changes, since the tree cache is otherwise only refreshed on
        // activation and in-app file operations.
        self.reload_tree();
        cx.notify();
    }

    /// Moves the tab with `id` to the gap `gap` (an index in `0..=tabs.len()`,
    /// "insert before this index"). A no-op when the tab already sits at that
    /// gap. Remaps `active_tab`/`context_tab`, then reveals and persists the new
    /// order so it survives a relaunch.
    pub(super) fn move_tab(
        &mut self,
        id: EntityId,
        gap: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(from) = self
            .tabs
            .iter()
            .position(|tab| tab.editor.entity_id() == id)
        else {
            return;
        };
        let gap = gap.min(self.tabs.len());
        // Dropping into the tab's own slot, before or after itself, moves nothing.
        if gap == from || gap == from + 1 {
            cx.notify();
            return;
        }

        let to = if gap > from { gap - 1 } else { gap };
        let tab = self.tabs.remove(from);
        self.tabs.insert(to, tab);

        self.active_tab = self.active_tab.map(|ix| remap_moved_index(ix, from, to));
        self.context_tab = self.context_tab.map(|ix| remap_moved_index(ix, from, to));
        self.reveal_active_tab();

        if let Some(tab) = self.active_tab.and_then(|ix| self.tabs.get(ix)) {
            tab.editor.update(cx, |state, cx| state.focus(window, cx));
        }
        self.save_session();
        cx.notify();
    }

    pub(super) fn close_tab(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some((id, dirty, title)) = self
            .tabs
            .get(ix)
            .map(|tab| (tab.editor.entity_id(), tab.dirty, tab.title.clone()))
        else {
            return;
        };

        if !dirty {
            self.remove_tab(id, window, cx);
            return;
        }

        let entity = cx.entity();
        window.open_alert_dialog(cx, move |alert, _window, _cx| {
            let entity = entity.clone();
            alert
                .confirm()
                .title("Unsaved Changes")
                .description(format!(
                    "\"{title}\" has unsaved changes. Close it without saving?"
                ))
                .ok_text("Close Without Saving")
                .ok_variant(ButtonVariant::Danger)
                .cancel_text("Keep Editing")
                .on_ok(move |_, window, cx| {
                    entity.update(cx, |this, cx| this.remove_tab(id, window, cx));
                    true
                })
        });
    }

    pub(super) fn close_other_tabs(
        &mut self,
        ix: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let indices = (0..self.tabs.len()).filter(|other| *other != ix).collect();
        self.close_tabs(indices, window, cx);
    }

    pub(super) fn close_tabs_to_right(
        &mut self,
        ix: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let indices = (ix + 1..self.tabs.len()).collect();
        self.close_tabs(indices, window, cx);
    }

    pub(super) fn close_all_tabs(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let indices = (0..self.tabs.len()).collect();
        self.close_tabs(indices, window, cx);
    }

    pub(super) fn close_tabs_with_deleted_files(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let indices = self
            .tabs
            .iter()
            .enumerate()
            .filter(|(_, tab)| tab.path.as_deref().is_some_and(|path| !path.exists()))
            .map(|(ix, _)| ix)
            .collect();
        self.close_tabs(indices, window, cx);
    }

    pub(super) fn close_tabs(
        &mut self,
        indices: Vec<usize>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let mut ids: Vec<EntityId> = Vec::new();
        let mut dirty = false;
        for ix in indices {
            if let Some(tab) = self.tabs.get(ix) {
                ids.push(tab.editor.entity_id());
                dirty |= tab.dirty;
            }
        }
        if ids.is_empty() {
            return;
        }

        if !dirty {
            for id in ids {
                self.remove_tab(id, window, cx);
            }
            return;
        }

        let entity = cx.entity();
        window.open_alert_dialog(cx, move |alert, _window, _cx| {
            let entity = entity.clone();
            let ids = ids.clone();
            alert
                .confirm()
                .title("Unsaved Changes")
                .description("Some tabs have unsaved changes. Close them without saving?")
                .ok_text("Close Without Saving")
                .ok_variant(ButtonVariant::Danger)
                .cancel_text("Keep Editing")
                .on_ok(move |_, window, cx| {
                    for id in ids.iter().copied() {
                        entity.update(cx, |this, cx| this.remove_tab(id, window, cx));
                    }
                    true
                })
        });
    }

    /// True when any open tab has unsaved edits.
    pub(super) fn any_dirty(&self) -> bool {
        self.tabs.iter().any(|tab| tab.dirty)
    }

    /// Quits after confirming that unsaved edits are discarded.
    pub(super) fn confirm_quit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let entity = cx.entity();
        window.open_alert_dialog(cx, move |alert, _window, _cx| {
            let entity = entity.clone();
            alert
                .confirm()
                .title("Unsaved Changes")
                .description("Some tabs have unsaved changes. Quit without saving?")
                .ok_text("Quit Without Saving")
                .ok_variant(ButtonVariant::Danger)
                .cancel_text("Keep Editing")
                .on_ok(move |_, _window, cx| {
                    entity.update(cx, |this, cx| {
                        this.allow_close = true;
                        cx.quit();
                    });
                    true
                })
        });
    }

    /// Answers the platform's close request. When there is unsaved work it opens
    /// the confirmation alert and refuses this close; the dialog re-closes once
    /// the user confirms.
    pub(crate) fn handle_window_should_close(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.allow_close || !self.any_dirty() {
            return true;
        }
        self.confirm_close_window(window, cx);
        false
    }

    /// Closes the window after confirming that unsaved edits are discarded.
    fn confirm_close_window(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let entity = cx.entity();
        window.open_alert_dialog(cx, move |alert, _window, _cx| {
            let entity = entity.clone();
            alert
                .confirm()
                .title("Unsaved Changes")
                .description("Some tabs have unsaved changes. Close without saving?")
                .ok_text("Close Without Saving")
                .ok_variant(ButtonVariant::Danger)
                .cancel_text("Keep Editing")
                .on_ok(move |_, window, cx| {
                    entity.update(cx, |this, _cx| this.allow_close = true);
                    window.remove_window();
                    true
                })
        });
    }

    pub(super) fn remove_tab(&mut self, id: EntityId, window: &mut Window, cx: &mut Context<Self>) {
        let Some(ix) = self
            .tabs
            .iter()
            .position(|tab| tab.editor.entity_id() == id)
        else {
            return;
        };

        if let Some(tab) = self.tabs.get(ix) {
            self.closed_tabs.push(ClosedTab {
                path: tab.path.clone(),
                content: tab.editor.read(cx).text().to_string(),
            });
            if self.closed_tabs.len() > MAX_CLOSED_TABS {
                self.closed_tabs.remove(0);
            }
        }

        self.tabs.remove(ix);

        self.active_tab = if self.tabs.is_empty() {
            None
        } else {
            let active = self.active_tab.unwrap_or(0);
            let active = if ix < active { active - 1 } else { active };
            Some(active.min(self.tabs.len() - 1))
        };
        self.reveal_active_tab();

        if let Some(tab) = self.active_tab.and_then(|ix| self.tabs.get(ix)) {
            tab.editor.update(cx, |state, cx| state.focus(window, cx));
        } else {
            // The closed editor held focus; without it no window action (About,
            // Quit, New File…) is reachable from the menu bar.
            self.focus_handle.focus(window, cx);
        }
        self.save_session();
        cx.notify();
    }

    pub(super) fn set_dirty(&mut self, id: EntityId, dirty: bool, cx: &mut Context<Self>) {
        if let Some(tab) = self
            .tabs
            .iter_mut()
            .find(|tab| tab.editor.entity_id() == id)
            && tab.dirty != dirty
        {
            tab.dirty = dirty;
            cx.notify();
        }
    }

    pub(super) fn sync_cursor(&mut self, id: EntityId, cx: &mut Context<Self>) {
        let Some(ix) = self.active_tab else {
            return;
        };
        let position = {
            let Some(tab) = self.tabs.get(ix) else {
                return;
            };
            if tab.editor.entity_id() != id {
                return;
            }
            tab.editor.read(cx).cursor_position()
        };

        self.cursor_line = position.line as usize + 1;
        self.cursor_col = position.character as usize + 1;
        cx.notify();
    }
}

/// The new index of an element after the element at `from` is moved to `to`
/// (both final-array indices). Remaps `active_tab`/`context_tab` so they keep
/// pointing at the same tab after a reorder.
fn remap_moved_index(index: usize, from: usize, to: usize) -> usize {
    if index == from {
        return to;
    }
    let index = if index > from { index - 1 } else { index };
    if index >= to { index + 1 } else { index }
}

#[cfg(test)]
mod tests {
    use super::remap_moved_index;

    #[test]
    fn remaps_indices_when_moving_a_tab_to_the_front() {
        // [A, B, C, D] with C (2) moved to index 0.
        let moved = |ix| remap_moved_index(ix, 2, 0);
        assert_eq!([moved(0), moved(1), moved(2), moved(3)], [1, 2, 0, 3]);
    }

    #[test]
    fn remaps_indices_when_moving_a_tab_to_the_back() {
        // [A, B, C, D] with A (0) moved to index 2.
        let moved = |ix| remap_moved_index(ix, 0, 2);
        assert_eq!([moved(0), moved(1), moved(2), moved(3)], [2, 0, 1, 3]);
    }

    #[test]
    fn remaps_indices_when_moving_a_tab_rightwards() {
        // [A, B, C, D] with B (1) moved to index 3.
        let moved = |ix| remap_moved_index(ix, 1, 3);
        assert_eq!([moved(0), moved(1), moved(2), moved(3)], [0, 3, 1, 2]);
    }
}
