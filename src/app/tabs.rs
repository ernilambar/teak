use std::path::PathBuf;
use std::sync::Arc;
use std::time::SystemTime;

use gpui_kit::component::{
    WindowExt as _,
    button::ButtonVariant,
    input::{EditorState, InputEvent, RangeDecorationCollection, TabSize},
    notification::Notification,
    text::TextViewState,
};
use gpui_kit::*;

use crate::CloseTab;
use crate::language::{editor_language_id, language_for};
use crate::paths::file_name;

use super::TinytextApp;
use super::files::{FileContent, LARGE_FILE_BYTES, TextFormat, modified_time};
use crate::search::ResultTarget;

/// How many recently closed tabs to remember for `ReopenClosedTab`.
const MAX_CLOSED_TABS: usize = 10;

pub(super) struct OpenTab {
    pub(super) path: Option<PathBuf>,
    pub(super) title: SharedString,
    pub(super) language: SharedString,
    /// True when the language was picked in the status bar, so saving or
    /// renaming must not re-derive it from the file extension.
    pub(super) language_override: bool,
    pub(super) dirty: bool,
    /// The file's modification time when last read or written, compared on
    /// activation and before saving to spot edits made by other programs.
    pub(super) disk_modified: Option<SystemTime>,
    pub(super) format: TextFormat,
    pub(super) editor: Entity<EditorState>,
    /// The rendered view shown in place of the editor while preview is on.
    /// Kept in sync with the buffer by the change subscription.
    pub(super) preview: Option<Preview>,
    /// A binary image opened view-only: it always shows its preview, its
    /// editor stays empty, and it is never saved.
    pub(super) raster: bool,
    /// Set on the read-only Find Results tab, which is never saved or restored.
    pub(super) find_results: Option<FindResultsTab>,
    pub(super) _subscriptions: Vec<Subscription>,
}

/// What the Find Results tab knows beyond its text.
pub(super) struct FindResultsTab {
    /// The results lines that open a match, in document order.
    pub(super) targets: Vec<ResultTarget>,
    /// The match highlights, replaced on every search.
    pub(super) highlights: RangeDecorationCollection,
    /// The target F4 / Shift+F4 last moved to.
    pub(super) current: Option<usize>,
}

pub(super) enum Preview {
    Markdown(Entity<TextViewState>),
    /// An SVG rendered from the buffer, or a raster image read from disk.
    Image(Arc<Image>),
}

impl Preview {
    /// Builds the preview for `text` in `language`, if that language has one.
    pub(super) fn for_text(language: &str, text: &str, cx: &mut App) -> Option<Self> {
        match language {
            "Markdown" => Some(Self::Markdown(
                cx.new(|cx| TextViewState::markdown(text, cx)),
            )),
            "SVG" => Some(Self::Image(Arc::new(Image::from_bytes(
                ImageFormat::Svg,
                text.as_bytes().to_vec(),
            )))),
            _ => None,
        }
    }

    /// Drops a decoded image from GPUI's asset cache, which otherwise keeps
    /// every rendered SVG revision for the life of the app.
    pub(super) fn release(self, cx: &mut App) {
        if let Self::Image(image) = self {
            ImageSource::Image(image).remove_asset(cx);
        }
    }
}

impl OpenTab {
    /// The preview to show instead of the editor. A tab whose language was
    /// switched away from the previewed one falls back to the editor.
    pub(super) fn previewing(&self) -> Option<&Preview> {
        self.preview.as_ref().filter(|preview| match preview {
            Preview::Markdown(_) => self.language.as_ref() == "Markdown",
            Preview::Image(_) => self.raster || self.language.as_ref() == "SVG",
        })
    }
}

/// A tab captured on close so `ReopenClosedTab` can bring it back. Untitled
/// tabs restore their content; file tabs are re-read from disk on reopen.
pub(super) struct ClosedTab {
    pub(super) path: Option<PathBuf>,
    pub(super) content: String,
}

/// The open tabs, plus the indexes that track which one is focused and which the
/// context menu targets.
///
/// `active` and `context` are indexes into `tabs`, so every insert, removal and
/// reorder has to fix them up together — that is why the list is private and
/// callers go through methods. `OpenTab` itself is a leaf record with no
/// invariant and keeps `pub(super)` fields, so rendering can read it directly.
pub(super) struct TabSet {
    tabs: Vec<OpenTab>,
    active: Option<usize>,
    context: Option<usize>,
    scroll: ScrollHandle,
    closed: Vec<ClosedTab>,
    /// A Find in Files match to select once its file finishes opening.
    pending_jump: Option<(PathBuf, Jump)>,
}

/// A 0-based line and character columns to select.
#[derive(Clone, Copy)]
pub(super) struct Jump {
    pub(super) line: u32,
    pub(super) start: u32,
    pub(super) end: u32,
}

impl TabSet {
    pub(super) fn new() -> Self {
        Self {
            tabs: Vec::new(),
            active: None,
            context: None,
            scroll: ScrollHandle::new(),
            closed: Vec::new(),
            pending_jump: None,
        }
    }

    pub(super) fn set_pending_jump(&mut self, path: PathBuf, jump: Jump) {
        self.pending_jump = Some((path, jump));
    }

    /// The pending jump, if it is for `path`.
    pub(super) fn take_pending_jump(&mut self, path: &std::path::Path) -> Option<Jump> {
        if self.pending_jump.as_ref()?.0 != path {
            return None;
        }
        self.pending_jump.take().map(|(_, jump)| jump)
    }

    pub(super) fn len(&self) -> usize {
        self.tabs.len()
    }

    pub(super) fn is_empty(&self) -> bool {
        self.tabs.is_empty()
    }

    pub(super) fn get(&self, ix: usize) -> Option<&OpenTab> {
        self.tabs.get(ix)
    }

    pub(super) fn get_mut(&mut self, ix: usize) -> Option<&mut OpenTab> {
        self.tabs.get_mut(ix)
    }

    pub(super) fn iter(&self) -> std::slice::Iter<'_, OpenTab> {
        self.tabs.iter()
    }

    pub(super) fn iter_mut(&mut self) -> std::slice::IterMut<'_, OpenTab> {
        self.tabs.iter_mut()
    }

    pub(super) fn active_index(&self) -> Option<usize> {
        self.active
    }

    pub(super) fn active(&self) -> Option<&OpenTab> {
        self.active.and_then(|ix| self.tabs.get(ix))
    }

    /// Points the active index at `ix`; an index past the end clears it.
    pub(super) fn set_active(&mut self, ix: Option<usize>) {
        self.active = ix.filter(|&ix| ix < self.tabs.len());
    }

    pub(super) fn context(&self) -> Option<usize> {
        self.context
    }

    pub(super) fn set_context(&mut self, ix: Option<usize>) {
        self.context = ix;
    }

    pub(super) fn scroll(&self) -> &ScrollHandle {
        &self.scroll
    }

    /// True when any open tab has unsaved edits.
    pub(super) fn any_dirty(&self) -> bool {
        self.tabs.iter().any(|tab| tab.dirty)
    }

    pub(super) fn position_of(&self, id: EntityId) -> Option<usize> {
        self.tabs
            .iter()
            .position(|tab| tab.editor.entity_id() == id)
    }

    pub(super) fn find(&self, id: EntityId) -> Option<&OpenTab> {
        self.tabs.iter().find(|tab| tab.editor.entity_id() == id)
    }

    /// Appends `tab`, returning its index. Activating it is the caller's call.
    pub(super) fn push(&mut self, tab: OpenTab) -> usize {
        self.tabs.push(tab);
        self.tabs.len() - 1
    }

    /// Moves the tab at `from` to `gap`, reporting whether anything moved:
    /// dropping into the tab's own slot, before or after itself, changes
    /// nothing. `from` must be a valid index.
    pub(super) fn reorder(&mut self, from: usize, gap: usize) -> bool {
        let gap = gap.min(self.tabs.len());
        if gap == from || gap == from + 1 {
            return false;
        }

        let to = if gap > from { gap - 1 } else { gap };
        let tab = self.tabs.remove(from);
        self.tabs.insert(to, tab);
        self.active = self.active.map(|ix| remap_moved_index(ix, from, to));
        self.context = self.context.map(|ix| remap_moved_index(ix, from, to));
        true
    }

    /// Removes the tab at `ix` and re-points the active index at a neighbour,
    /// handing back the preview so the caller can release it. `None` when `ix`
    /// is out of range.
    pub(super) fn drop_at(&mut self, ix: usize) -> Option<Preview> {
        if ix >= self.tabs.len() {
            return None;
        }

        let preview = self.tabs.remove(ix).preview;
        self.context = match self.context {
            Some(context) if context == ix => None,
            Some(context) if context > ix => Some(context - 1),
            context => context,
        };
        self.active = if self.tabs.is_empty() {
            None
        } else {
            let active = self.active.unwrap_or(0);
            let active = if ix < active { active - 1 } else { active };
            Some(active.min(self.tabs.len() - 1))
        };
        preview
    }

    /// Records `tab` for `ReopenClosedTab`, dropping the oldest past the cap.
    pub(super) fn push_closed(&mut self, tab: ClosedTab) {
        self.closed.push(tab);
        if self.closed.len() > MAX_CLOSED_TABS {
            self.closed.remove(0);
        }
    }

    pub(super) fn pop_closed(&mut self) -> Option<ClosedTab> {
        self.closed.pop()
    }
}

impl TinytextApp {
    pub(super) fn add_tab(
        &mut self,
        path: Option<PathBuf>,
        content: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let first_line = content.lines().next().unwrap_or_default();
        let language: SharedString = path
            .as_deref()
            .map(|path| language_for(path, first_line))
            .unwrap_or_else(|| "Plain Text".into());
        let title: SharedString = path
            .as_deref()
            .map(|path| file_name(path).into())
            .unwrap_or_else(|| "Untitled".into());
        self.push_tab(path, title, language, content, window, cx);
    }

    /// Opens a pathless document under an explicit title and language, for
    /// generated content that belongs to no file on disk. The tab behaves like
    /// any other untitled one: the session never persists it, and nothing is
    /// written unless the user saves it somewhere.
    pub(super) fn add_document_tab(
        &mut self,
        title: &str,
        language: &str,
        content: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.push_tab(None, title.into(), language.into(), content, window, cx);
    }

    /// Shared body of [`Self::add_tab`] and [`Self::add_document_tab`].
    fn push_tab(
        &mut self,
        path: Option<PathBuf>,
        title: SharedString,
        language: SharedString,
        content: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let editor = cx.new(|cx| {
            EditorState::new(window, cx)
                .language(editor_language_id(&language))
                .default_value(content)
                .soft_wrap(self.settings.values.editor.soft_wrap())
                .show_whitespaces(self.settings.values.editor.show_whitespace())
                .tab_size(TabSize {
                    tab_size: self.settings.values.editor.tab_size(),
                    hard_tabs: self.settings.values.editor.hard_tabs(),
                })
                .line_number(self.settings.values.editor.line_numbers())
                .indent_guides(self.settings.values.editor.indent_guides())
                .folding(self.settings.values.editor.folding())
                .auto_close(self.settings.values.editor.auto_close())
                .smart_indent(self.settings.values.editor.smart_indent())
                .wrapping_indent(super::wrapping_indent(&self.settings.values.editor))
                .scroll_beyond_last_line(super::scroll_beyond_last_line(
                    &self.settings.values.editor,
                ))
                .cursor_surrounding_lines(self.settings.values.editor.cursor_surrounding_lines)
        });

        let change_subscription = cx.subscribe(&editor, |this, editor, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                this.set_dirty(editor.entity_id(), true, cx);
                this.sync_preview(&editor, cx);
            }
        });
        let cursor_subscription = cx.observe(&editor, |this, editor, cx| {
            this.sync_cursor(editor.entity_id(), cx);
        });

        let disk_modified = path.as_deref().and_then(modified_time);
        let ix = self.tabs.push(OpenTab {
            path,
            title,
            language,
            language_override: false,
            dirty: false,
            disk_modified,
            format: TextFormat::default(),
            editor: editor.clone(),
            preview: None,
            raster: false,
            find_results: None,
            _subscriptions: vec![change_subscription, cursor_subscription],
        });
        self.tabs.set_active(Some(ix));
        self.reveal_active_tab();

        editor.update(cx, |state, cx| state.focus(window, cx));
        let position = editor.read(cx).cursor_position();
        self.chrome
            .set_cursor(position.line as usize + 1, position.character as usize + 1);
        self.save_session();
        cx.notify();
    }

    /// Opens a tab for a file read from disk: an editor for text, or a
    /// view-only preview for a raster image.
    pub(super) fn add_file_tab(
        &mut self,
        path: PathBuf,
        content: FileContent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match content {
            FileContent::Text { text, format } => {
                let name = file_name(&path);
                let large = text.len() > LARGE_FILE_BYTES;
                let language = if large {
                    "Plain Text".into()
                } else {
                    language_for(&path, text.lines().next().unwrap_or_default())
                };
                self.push_tab(
                    Some(path.clone()),
                    name.clone().into(),
                    language,
                    text,
                    window,
                    cx,
                );
                if let Some(ix) = self.tabs.active_index()
                    && let Some(tab) = self.tabs.get_mut(ix)
                {
                    tab.format = format;
                    // Keeps a save from turning highlighting back on.
                    tab.language_override = large;
                }
                if format.lossy {
                    window.push_notification(
                        Notification::warning(format!(
                            "{name} is not valid UTF-8; unreadable bytes show as �"
                        )),
                        cx,
                    );
                }
                if let Some(jump) = self.tabs.take_pending_jump(&path)
                    && let Some(ix) = self.tabs.active_index()
                {
                    self.select_match(ix, jump, window, cx);
                }
                if large {
                    window.push_notification(
                        Notification::info(format!(
                            "{name} is over {} MB; syntax highlighting is off",
                            LARGE_FILE_BYTES / (1024 * 1024)
                        )),
                        cx,
                    );
                }
            }
            FileContent::Image(image) => {
                self.add_tab(Some(path), String::new(), window, cx);
                let ix = self.tabs.len() - 1;
                if let Some(tab) = self.tabs.get_mut(ix) {
                    tab.raster = true;
                    tab.preview = Some(Preview::Image(image));
                }
                self.focus_tab(ix, window, cx);
            }
        }
    }

    pub(super) fn on_close_tab(
        &mut self,
        _: &CloseTab,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(ix) = self.tabs.active_index() {
            self.close_tab(ix, window, cx);
        }
    }

    /// Reveals the active tab in the horizontally scrolling tab strip. The
    /// strip only scrolls when tabs overflow; with few tabs this is a no-op.
    fn reveal_active_tab(&self) {
        if let Some(ix) = self.tabs.active_index() {
            self.tabs.scroll().scroll_to_item(ix);
        }
    }

    pub(super) fn activate_tab(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.tabs.set_active(Some(ix));
        self.reveal_active_tab();
        if let Some(tab) = self.tabs.get(ix) {
            let editor = tab.editor.clone();
            self.focus_tab(ix, window, cx);
            let position = editor.read(cx).cursor_position();
            self.chrome
                .set_cursor(position.line as usize + 1, position.character as usize + 1);
        }
        // Switching tabs is a natural cue to pick up external filesystem
        // changes, since the tree cache is otherwise only refreshed on
        // activation and in-app file operations.
        self.reload_tree();
        self.check_outside_change(window, cx);
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
        let Some(from) = self.tabs.position_of(id) else {
            return;
        };
        // Dropping into the tab's own slot, before or after itself, moves nothing.
        if !self.tabs.reorder(from, gap) {
            cx.notify();
            return;
        }
        self.reveal_active_tab();

        if let Some(ix) = self.tabs.active_index() {
            self.focus_tab(ix, window, cx);
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
        self.tabs.any_dirty()
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
                        this.chrome.set_allow_close(true);
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
        if self.chrome.allow_close() || !self.any_dirty() {
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
                    entity.update(cx, |this, _cx| this.chrome.set_allow_close(true));
                    window.remove_window();
                    true
                })
        });
    }

    pub(super) fn remove_tab(&mut self, id: EntityId, window: &mut Window, cx: &mut Context<Self>) {
        let Some(ix) = self.tabs.position_of(id) else {
            return;
        };

        // Reopening the results tab would bring it back as a plain Untitled copy.
        if let Some(tab) = self.tabs.get(ix)
            && tab.find_results.is_none()
        {
            self.tabs.push_closed(ClosedTab {
                path: tab.path.clone(),
                content: tab.editor.read(cx).text().to_string(),
            });
        }

        if let Some(preview) = self.tabs.drop_at(ix) {
            preview.release(cx);
        }

        self.reveal_active_tab();

        if let Some(ix) = self.tabs.active_index() {
            self.focus_tab(ix, window, cx);
        } else {
            // The closed editor held focus; without it no window action (About,
            // Quit, New File…) is reachable from the menu bar.
            self.focus_handle.focus(window, cx);
        }
        self.save_session();
        cx.notify();
    }

    /// Focuses tab `ix`'s editor, or the window root while the tab shows a
    /// preview: a focused editor that is not rendered leaves no window action
    /// reachable.
    pub(super) fn focus_tab(&self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.get(ix) else {
            return;
        };
        if tab.previewing().is_some() {
            self.focus_handle.focus(window, cx);
        } else {
            tab.editor.update(cx, |state, cx| state.focus(window, cx));
        }
    }

    /// Pushes `editor`'s text into its tab's preview, if one is open.
    pub(super) fn sync_preview(&mut self, editor: &Entity<EditorState>, cx: &mut Context<Self>) {
        let Some(tab) = self
            .tabs
            .iter_mut()
            .find(|tab| tab.editor.entity_id() == editor.entity_id())
        else {
            return;
        };
        if tab.raster {
            return;
        }
        let text = editor.read(cx).value();
        match &tab.preview {
            Some(Preview::Markdown(view)) => view.update(cx, |state, cx| state.set_text(&text, cx)),
            Some(Preview::Image(_)) => {
                let next = Preview::for_text("SVG", &text, cx);
                if let Some(old) = std::mem::replace(&mut tab.preview, next) {
                    old.release(cx);
                }
            }
            None => {}
        }
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
        let Some(ix) = self.tabs.active_index() else {
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

        self.chrome
            .set_cursor(position.line as usize + 1, position.character as usize + 1);
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
    use std::path::PathBuf;

    use gpui_kit::component::input::EditorState;
    use gpui_kit::{AnyWindowHandle, AppContext as _, TestAppContext};

    use crate::app::test_support::open_app;

    use super::{OpenTab, TabSet, TextFormat, remap_moved_index};

    /// `n` tabs whose editors are real (empty) editor entities, so the entity
    /// ids the index bookkeeping matches on are distinct.
    fn tab_set(cx: &mut TestAppContext, handle: AnyWindowHandle, n: usize) -> TabSet {
        let mut set = TabSet::new();
        for ix in 0..n {
            let editor = cx
                .update_window(handle, |_, window, cx| {
                    cx.new(|cx| EditorState::new(window, cx))
                })
                .unwrap();
            set.push(OpenTab {
                path: Some(PathBuf::from(format!("/tmp/tinytext-tab-{ix}.txt"))),
                title: format!("tab-{ix}").into(),
                language: "Plain Text".into(),
                language_override: false,
                dirty: false,
                disk_modified: None,
                format: TextFormat::default(),
                editor,
                preview: None,
                raster: false,
                find_results: None,
                _subscriptions: Vec::new(),
            });
        }
        set
    }

    fn titles(set: &TabSet) -> Vec<String> {
        set.iter().map(|tab| tab.title.to_string()).collect()
    }

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

    #[gpui_kit::test]
    async fn dropping_a_tab_before_the_active_one_shifts_the_active_index(cx: &mut TestAppContext) {
        let (handle, _app) = open_app(cx);
        let mut set = tab_set(cx, handle, 3);
        set.set_active(Some(2));

        assert!(set.drop_at(0).is_none(), "an editor tab has no preview");
        assert_eq!(titles(&set), ["tab-1", "tab-2"]);
        assert_eq!(set.active_index(), Some(1), "active follows its tab down");
    }

    #[gpui_kit::test]
    async fn dropping_the_last_tab_clears_the_active_index(cx: &mut TestAppContext) {
        let (handle, _app) = open_app(cx);
        let mut set = tab_set(cx, handle, 1);
        set.set_active(Some(0));

        set.drop_at(0);

        assert!(set.is_empty());
        assert_eq!(set.active_index(), None);
    }

    #[gpui_kit::test]
    async fn dropping_a_tab_keeps_the_context_index_on_its_tab(cx: &mut TestAppContext) {
        let (handle, _app) = open_app(cx);
        let mut set = tab_set(cx, handle, 3);

        set.set_context(Some(2));
        set.drop_at(0);
        assert_eq!(set.context(), Some(1), "context follows its tab down");

        set.drop_at(1);
        assert_eq!(set.context(), None, "dropping the context tab clears it");
    }

    #[gpui_kit::test]
    async fn set_active_rejects_an_index_past_the_end(cx: &mut TestAppContext) {
        let (handle, _app) = open_app(cx);
        let mut set = tab_set(cx, handle, 2);

        set.set_active(Some(2));
        assert_eq!(set.active_index(), None);
    }

    #[gpui_kit::test]
    async fn dropping_past_the_end_is_a_no_op(cx: &mut TestAppContext) {
        let (handle, _app) = open_app(cx);
        let mut set = tab_set(cx, handle, 2);

        assert!(set.drop_at(2).is_none());
        assert_eq!(set.len(), 2);
    }

    #[gpui_kit::test]
    async fn reorder_ignores_a_drop_into_the_tabs_own_slot(cx: &mut TestAppContext) {
        let (handle, _app) = open_app(cx);
        let mut set = tab_set(cx, handle, 3);

        assert!(!set.reorder(1, 1), "dropping before itself moves nothing");
        assert!(!set.reorder(1, 2), "dropping after itself moves nothing");
        assert_eq!(titles(&set), ["tab-0", "tab-1", "tab-2"]);
    }

    #[gpui_kit::test]
    async fn reorder_carries_the_active_index_with_the_tab(cx: &mut TestAppContext) {
        let (handle, _app) = open_app(cx);
        let mut set = tab_set(cx, handle, 3);
        set.set_active(Some(0));

        // Move the first tab to the end.
        assert!(set.reorder(0, 3));

        assert_eq!(titles(&set), ["tab-1", "tab-2", "tab-0"]);
        assert_eq!(set.active_index(), Some(2), "active follows its tab");
    }
}
