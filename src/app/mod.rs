mod editor;
mod files;
mod palette;
mod preferences;
mod tabs;
mod ui;

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use gpui_kit::base::StyledExt as _;
use gpui_kit::component::{
    ActiveTheme as _, WindowExt as _,
    command::CommandState,
    input::{EditorState, TabSize, WrappingIndent},
    link::Link,
    notification::Notification,
    text::TextViewState,
};
use gpui_kit::*;

use crate::cli::{app_bundle_path, install_cli};
use crate::paths::{DirEntry, read_dir};
use crate::session::{SessionState, session_path};
use crate::settings::{
    DEFAULT_FONT_WEIGHT, EditorSettings, MAX_FONT_SIZE, MIN_FONT_SIZE, Settings, defaults_document,
    font_family_issue, load_settings, save_editor_flag, save_font_size, save_show_whitespace,
    save_soft_wrap, save_theme, settings_path,
};
use crate::update::{INSTALL_COMMAND, is_newer, latest_version};
use crate::{
    About, CheckForUpdates, CopyFilePath, CopyRelativePath, InstallCli, JumpToTab, NextTab,
    OpenDefaultSettings, OpenSettings, OpenThemesFolder, PreviousTab, Quit, Refresh,
    ReopenClosedTab, RevealInFinder, SetTheme, ToggleAutoClose, ToggleFolding, ToggleIndentGuides,
    ToggleLineNumbers, ToggleMarkdownPreview, ToggleSidebar, ToggleSmartIndent, ToggleWhitespace,
    ToggleWordWrap, ZoomIn, ZoomOut, ZoomReset,
};

/// Maps `editor.indent_wrapped_lines` onto the engine's wrapping indent.
pub(super) fn wrapping_indent(editor: &EditorSettings) -> WrappingIndent {
    if editor.indent_wrapped_lines() {
        WrappingIndent::Same
    } else {
        WrappingIndent::None
    }
}

/// Maps `editor.scroll_beyond_last_line` onto the engine's trailing rows:
/// `None` keeps its default space, `Some(0)` stops at the last line.
pub(super) fn scroll_beyond_last_line(editor: &EditorSettings) -> Option<usize> {
    (!editor.scroll_beyond_last_line()).then_some(0)
}

/// The app name shown in the window title, before the focused tab's path.
pub(crate) const WINDOW_TITLE: &str = "Tinytext";

struct OpenTab {
    path: Option<PathBuf>,
    title: SharedString,
    language: SharedString,
    /// True when the language was picked in the status bar, so saving or
    /// renaming must not re-derive it from the file extension.
    language_override: bool,
    dirty: bool,
    editor: Entity<EditorState>,
    /// The rendered view shown in place of the editor while preview is on.
    /// Kept in sync with the buffer by the change subscription.
    preview: Option<Preview>,
    /// A binary image opened view-only: it always shows its preview, its
    /// editor stays empty, and it is never saved.
    raster: bool,
    _subscriptions: Vec<Subscription>,
}

enum Preview {
    Markdown(Entity<TextViewState>),
    /// An SVG rendered from the buffer, or a raster image read from disk.
    Image(Arc<Image>),
}

impl Preview {
    /// Builds the preview for `text` in `language`, if that language has one.
    fn for_text(language: &str, text: &str, cx: &mut App) -> Option<Self> {
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
    fn release(self, cx: &mut App) {
        if let Self::Image(image) = self {
            ImageSource::Image(image).remove_asset(cx);
        }
    }
}

impl OpenTab {
    /// The preview to show instead of the editor. A tab whose language was
    /// switched away from the previewed one falls back to the editor.
    fn previewing(&self) -> Option<&Preview> {
        self.preview.as_ref().filter(|preview| match preview {
            Preview::Markdown(_) => self.language.as_ref() == "Markdown",
            Preview::Image(_) => self.raster || self.language.as_ref() == "SVG",
        })
    }
}

/// A pending Cut/Copy of a single tree entry, applied on the next Paste.
#[derive(Clone)]
struct FileClipboard {
    path: PathBuf,
    cut: bool,
}

/// A tab captured on close so `ReopenClosedTab` can bring it back. Untitled
/// tabs restore their content; file tabs are re-read from disk on reopen.
/// Bounded by MAX_CLOSED_TABS so the stack cannot grow without limit.
struct ClosedTab {
    path: Option<PathBuf>,
    content: String,
}

/// How many recently closed tabs to remember for `ReopenClosedTab`.
const MAX_CLOSED_TABS: usize = 10;

/// One flattened, currently-visible row of the sidebar tree. A root row is a
/// workspace folder itself; every other row is one of its entries.
struct TreeRow {
    path: PathBuf,
    depth: usize,
    is_dir: bool,
    is_root: bool,
}

pub(crate) struct TinytextApp {
    pub(crate) focus_handle: FocusHandle,
    /// The workspace folders shown in the sidebar, in the order they were added.
    workspace_roots: Vec<PathBuf>,
    expanded: HashSet<PathBuf>,
    selected_path: Option<PathBuf>,
    /// Directory listings for every root and expanded folder, so rendering
    /// the tree never touches the disk. Invalidated on filesystem changes.
    dir_cache: HashMap<PathBuf, Vec<DirEntry>>,
    /// The flattened, visible rows derived from `dir_cache` + `expanded`,
    /// rebuilt only when the tree changes.
    visible_rows: Vec<TreeRow>,
    /// Scroll position and extent of the virtualized sidebar list.
    tree_scroll: UniformListScrollHandle,
    tabs: Vec<OpenTab>,
    active_tab: Option<usize>,
    context_tab: Option<usize>,
    /// Tracks the horizontal scroll of the tab strip so the active tab can be
    /// revealed when it would otherwise be off-screen.
    tab_scroll: ScrollHandle,
    file_clipboard: Option<FileClipboard>,
    closed_tabs: Vec<ClosedTab>,
    /// Label for a background task in progress, shown with a spinner in the
    /// status bar. `None` when idle.
    busy: Option<SharedString>,
    /// The command palette's interaction state while the overlay is open.
    palette: Option<Entity<CommandState>>,
    /// The focus that was active when the palette opened, restored on close.
    palette_return_focus: Option<FocusHandle>,
    sidebar_visible: bool,
    cursor_line: usize,
    cursor_col: usize,
    settings: Settings,
    /// Whether the Settings overlay is showing.
    settings_open: bool,
    /// A settings write that could not be reported through a notification,
    /// surfaced in the overlay instead.
    settings_error: Option<SharedString>,
    /// The window title currently applied, so it is only pushed to the platform
    /// when the focused tab changes.
    window_title: String,
    /// Set once the user has confirmed discarding unsaved work in the quit or
    /// window-close alert, so the follow-up close callback stops re-prompting.
    allow_close: bool,
}

impl TinytextApp {
    pub(crate) fn new(
        workspace_roots: Vec<PathBuf>,
        settings: Settings,
        cx: &mut Context<Self>,
    ) -> Self {
        // The sidebar starts open only when there is a folder to show. Toggling
        // it later is independent of the roots, and reveals the empty state.
        let mut roots: Vec<PathBuf> = Vec::new();
        for root in workspace_roots {
            if root.is_dir() && !roots.contains(&root) {
                roots.push(root);
            }
        }
        let sidebar_visible = !roots.is_empty();

        let mut expanded = HashSet::new();
        for root in &roots {
            expanded.insert(root.clone());
        }

        let mut app = Self {
            focus_handle: cx.focus_handle(),
            workspace_roots: roots,
            expanded,
            selected_path: None,
            dir_cache: HashMap::new(),
            visible_rows: Vec::new(),
            tree_scroll: UniformListScrollHandle::default(),
            tabs: Vec::new(),
            active_tab: None,
            context_tab: None,
            tab_scroll: ScrollHandle::new(),
            file_clipboard: None,
            closed_tabs: Vec::new(),
            busy: None,
            palette: None,
            palette_return_focus: None,
            sidebar_visible,
            cursor_line: 1,
            cursor_col: 1,
            settings,
            settings_open: false,
            settings_error: None,
            window_title: WINDOW_TITLE.to_string(),
            allow_close: false,
        };
        app.reload_tree();
        app
    }

    /// Keeps the native window title in sync with the focused tab, appending the
    /// file's full path after the app name.
    fn sync_window_title(&mut self, window: &mut Window) {
        let title = match self.active() {
            Some(tab) => match tab.path.as_deref() {
                Some(path) => format!("{WINDOW_TITLE} — {}", path.display()),
                None => format!("{WINDOW_TITLE} — Untitled"),
            },
            None => WINDOW_TITLE.to_string(),
        };

        if title != self.window_title {
            window.set_window_title(&title);
            self.window_title = title;
        }
    }

    pub(crate) fn restore_session(
        &mut self,
        session: SessionState,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.workspace_roots = session.roots.into_iter().filter(|path| path.is_dir()).fold(
            Vec::new(),
            |mut roots, root| {
                if !roots.contains(&root) {
                    roots.push(root);
                }
                roots
            },
        );

        self.expanded = session
            .expanded
            .into_iter()
            .filter(|path| path.is_dir())
            .collect();
        for root in &self.workspace_roots {
            self.expanded.insert(root.clone());
        }

        self.selected_path = session.selected_path.filter(|path| path.exists());
        self.sidebar_visible = session.sidebar_visible;
        self.reload_tree();

        let mut paths = session.tabs;
        paths.retain(|path| path.is_file());
        let active = session.active_tab.filter(|path| path.is_file());

        cx.spawn_in(window, async move |this, cx| {
            for path in paths {
                let read = cx
                    .background_executor()
                    .spawn({
                        let path = path.clone();
                        async move { files::read_file(&path) }
                    })
                    .await;
                if let Ok(content) = read {
                    this.update_in(cx, |this, window, cx| {
                        this.add_file_tab(path, content, window, cx);
                    })
                    .ok();
                }
            }

            this.update_in(cx, |this, window, cx| {
                if let Some(active) = active
                    && let Some(ix) = this
                        .tabs
                        .iter()
                        .position(|tab| tab.path.as_deref() == Some(active.as_path()))
                {
                    this.activate_tab(ix, window, cx);
                    this.save_session();
                }
            })
            .ok();
        })
        .detach();

        cx.notify();
    }

    fn save_session(&self) {
        let Some(path) = session_path() else {
            return;
        };

        let mut expanded: Vec<PathBuf> = self.expanded.iter().cloned().collect();
        expanded.sort();

        let state = SessionState {
            roots: self.workspace_roots.clone(),
            tabs: self
                .tabs
                .iter()
                .filter_map(|tab| tab.path.clone())
                .collect(),
            active_tab: self.active().and_then(|tab| tab.path.clone()),
            expanded,
            selected_path: self.selected_path.clone(),
            sidebar_visible: self.sidebar_visible,
        };

        let Ok(json) = serde_json::to_string_pretty(&state) else {
            return;
        };
        if let Some(parent) = path.parent()
            && std::fs::create_dir_all(parent).is_err()
        {
            return;
        }

        let temp = path.with_file_name("session.json.tmp");
        if std::fs::write(&temp, json).is_ok() {
            let _ = std::fs::rename(&temp, &path);
        }
    }

    /// Clears the directory cache and re-reads every workspace tree from disk.
    /// Call after any filesystem change or when the roots change.
    pub(crate) fn reload_tree(&mut self) {
        self.dir_cache.clear();
        self.rebuild_visible_rows();
    }

    /// Re-flattens the visible rows, reading and caching any expanded directory
    /// that is not cached yet. Cheap to call on every expand/collapse.
    fn rebuild_visible_rows(&mut self) {
        let mut rows = std::mem::take(&mut self.visible_rows);
        rows.clear();
        let roots = self.workspace_roots.clone();
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
        self.visible_rows = rows;
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

    fn active(&self) -> Option<&OpenTab> {
        self.active_tab.and_then(|ix| self.tabs.get(ix))
    }

    fn active_language(&self) -> SharedString {
        self.active()
            .map(|tab| tab.language.clone())
            .unwrap_or_else(|| "Plain Text".into())
    }

    /// The image shown by the active tab when it is a view-only raster tab.
    fn active_raster(&self) -> Option<&Arc<Image>> {
        match self.active().filter(|tab| tab.raster)?.preview.as_ref()? {
            Preview::Image(image) => Some(image),
            Preview::Markdown(_) => None,
        }
    }

    fn active_dirty(&self) -> bool {
        self.active().map(|tab| tab.dirty).unwrap_or(false)
    }

    fn on_quit(&mut self, _: &Quit, window: &mut Window, cx: &mut Context<Self>) {
        if !self.any_dirty() || self.allow_close {
            cx.quit();
        } else {
            self.confirm_quit(window, cx);
        }
    }

    fn on_toggle_sidebar(
        &mut self,
        _: &ToggleSidebar,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.toggle_sidebar(cx);
    }

    fn on_refresh(&mut self, _: &Refresh, _window: &mut Window, cx: &mut Context<Self>) {
        self.reload_tree();
        cx.notify();
    }

    fn on_reveal_in_finder(
        &mut self,
        _: &RevealInFinder,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(path) = self.active().and_then(|tab| tab.path.clone()) {
            self.reveal_in_finder(&path, cx);
        }
    }

    fn on_copy_file_path(
        &mut self,
        _: &CopyFilePath,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(path) = self.active().and_then(|tab| tab.path.clone()) {
            self.copy_path_to_clipboard(&path, cx);
        }
    }

    pub(super) fn reveal_in_finder(&self, path: &Path, cx: &mut Context<Self>) {
        cx.reveal_path(path);
    }

    fn on_copy_relative_path(
        &mut self,
        _: &CopyRelativePath,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(path) = self.active().and_then(|tab| tab.path.clone()) {
            self.copy_relative_path(&path, cx);
        }
    }

    pub(super) fn copy_path_to_clipboard(&self, path: &Path, cx: &mut Context<Self>) {
        cx.write_to_clipboard(ClipboardItem::new_string(path.display().to_string()));
    }

    pub(super) fn copy_name_to_clipboard(&self, path: &Path, cx: &mut Context<Self>) {
        cx.write_to_clipboard(ClipboardItem::new_string(crate::paths::file_name(path)));
    }

    pub(super) fn copy_relative_path(&self, path: &Path, cx: &mut Context<Self>) {
        let text = match crate::paths::nearest_root(&self.workspace_roots, path) {
            Some(root) => crate::paths::relative_display(path, root),
            None => path.display().to_string(),
        };
        cx.write_to_clipboard(ClipboardItem::new_string(text));
    }

    fn toggle_sidebar(&mut self, cx: &mut Context<Self>) {
        self.sidebar_visible = !self.sidebar_visible;
        self.save_session();
        cx.notify();
    }

    fn on_zoom_in(&mut self, _: &ZoomIn, window: &mut Window, cx: &mut Context<Self>) {
        self.step_font_size(1., window, cx);
    }

    fn on_zoom_out(&mut self, _: &ZoomOut, window: &mut Window, cx: &mut Context<Self>) {
        self.step_font_size(-1., window, cx);
    }

    fn on_zoom_reset(&mut self, _: &ZoomReset, window: &mut Window, cx: &mut Context<Self>) {
        self.settings.editor.font_size = None;
        self.after_setting_write(save_font_size(None), window, cx);
        cx.notify();
    }

    /// Shifts the editor font size by a whole-point step, clamped to the same
    /// bounds the settings accessor enforces. Applies across every open tab
    /// because the size is read from `self.settings` at render time.
    fn step_font_size(&mut self, step: f32, window: &mut Window, cx: &mut Context<Self>) {
        let base = self
            .settings
            .editor
            .font_size()
            .unwrap_or_else(|| cx.theme().mono_font_size.as_f32());
        self.settings.editor.font_size = Some((base + step).clamp(MIN_FONT_SIZE, MAX_FONT_SIZE));
        self.after_setting_write(save_font_size(self.settings.editor.font_size), window, cx);
        cx.notify();
    }

    fn on_toggle_word_wrap(
        &mut self,
        _: &ToggleWordWrap,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.settings.editor.soft_wrap = Some(!self.settings.editor.soft_wrap());
        self.apply_editor_options_to_all(window, cx);
        self.after_setting_write(save_soft_wrap(self.settings.editor.soft_wrap()), window, cx);
        cx.notify();
    }

    fn on_toggle_whitespace(
        &mut self,
        _: &ToggleWhitespace,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.settings.editor.show_whitespace = Some(!self.settings.editor.show_whitespace());
        self.apply_editor_options_to_all(window, cx);
        self.after_setting_write(
            save_show_whitespace(self.settings.editor.show_whitespace()),
            window,
            cx,
        );
        cx.notify();
    }

    fn on_toggle_line_numbers(
        &mut self,
        _: &ToggleLineNumbers,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let enabled = !self.settings.editor.line_numbers();
        self.set_editor_flag("line_numbers", |e| &mut e.line_numbers, enabled, window, cx);
    }

    fn on_toggle_indent_guides(
        &mut self,
        _: &ToggleIndentGuides,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let enabled = !self.settings.editor.indent_guides();
        self.set_editor_flag(
            "indent_guides",
            |e| &mut e.indent_guides,
            enabled,
            window,
            cx,
        );
    }

    fn on_toggle_folding(
        &mut self,
        _: &ToggleFolding,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let enabled = !self.settings.editor.folding();
        self.set_editor_flag("folding", |e| &mut e.folding, enabled, window, cx);
    }

    fn on_toggle_auto_close(
        &mut self,
        _: &ToggleAutoClose,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let enabled = !self.settings.editor.auto_close();
        self.set_editor_flag("auto_close", |e| &mut e.auto_close, enabled, window, cx);
    }

    fn on_toggle_smart_indent(
        &mut self,
        _: &ToggleSmartIndent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let enabled = !self.settings.editor.smart_indent();
        self.set_editor_flag("smart_indent", |e| &mut e.smart_indent, enabled, window, cx);
    }

    /// Swaps the active Markdown or SVG tab between its editor and a rendered
    /// preview. Other languages and raster images are left alone.
    fn on_toggle_markdown_preview(
        &mut self,
        _: &ToggleMarkdownPreview,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(ix) = self.active_tab else {
            return;
        };
        let Some(tab) = self.tabs.get_mut(ix) else {
            return;
        };
        if tab.raster {
            return;
        }
        let next = if tab.previewing().is_some() {
            None
        } else {
            let text = tab.editor.read(cx).value();
            match Preview::for_text(&tab.language, &text, cx) {
                Some(preview) => Some(preview),
                None => return,
            }
        };
        if let Some(old) = std::mem::replace(&mut tab.preview, next) {
            old.release(cx);
        }
        self.focus_tab(ix, window, cx);
        cx.notify();
    }

    /// Sets the on/off editor option `editor.<key>`, pushes it onto open
    /// editors, and persists it.
    pub(super) fn set_editor_flag(
        &mut self,
        key: &str,
        field: fn(&mut EditorSettings) -> &mut Option<bool>,
        enabled: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        *field(&mut self.settings.editor) = Some(enabled);
        self.apply_editor_options_to_all(window, cx);
        self.after_setting_write(save_editor_flag(key, enabled), window, cx);
        cx.notify();
    }

    /// Applies an appearance picked from the Appearance menu or the command
    /// palette, persisting it and refreshing the menu's check mark.
    fn on_set_theme(&mut self, action: &SetTheme, window: &mut Window, cx: &mut Context<Self>) {
        let choice = action.0.clone();
        self.settings.ui.theme = Some(choice.clone());
        self.apply_theme_choice(&choice, Some(window), cx);
        self.after_setting_write(save_theme(&choice), window, cx);
        cx.notify();
    }

    /// Re-resolves the theme when macOS switches appearance, but only while the
    /// user chose to follow the system.
    pub(super) fn on_system_appearance_changed(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let choice = self.settings.ui.theme();
        if choice != crate::THEME_SYSTEM {
            return;
        }
        self.apply_theme_choice(choice, Some(window), cx);
        cx.notify();
    }

    /// Applies the current appearance and color scheme to the live theme and
    /// refreshes the Appearance menu so its check mark matches.
    fn apply_theme_choice(
        &self,
        choice: &str,
        window: Option<&mut Window>,
        cx: &mut Context<Self>,
    ) {
        crate::theme::apply(&self.settings, window, cx);
        cx.set_menus(crate::app_menus(choice));
    }

    /// Re-applies the current appearance and color scheme without a window, used
    /// after the themes directory finishes loading so a scheme named in settings
    /// that only just became available takes effect.
    pub(super) fn reapply_theme(&mut self, cx: &mut Context<Self>) {
        let choice = self.settings.ui.theme().to_string();
        self.apply_theme_choice(&choice, None, cx);
        cx.notify();
    }

    /// Applies the outcome of a settings write: on success, mirrors the new file
    /// contents into an open, unmodified settings tab; on failure, reports it
    /// without undoing the change, which has already been applied in memory.
    fn after_setting_write(
        &mut self,
        result: Result<String, String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match result {
            Ok(contents) => self.refresh_settings_tab(&contents, window, cx),
            Err(message) => window.push_notification(
                Notification::error(format!("Could not save settings: {message}")),
                cx,
            ),
        }
    }

    /// Reflects a programmatic settings write in the open settings tab, if one
    /// exists. A tab with unsaved edits is left untouched so the write cannot
    /// discard the user's work.
    fn refresh_settings_tab(
        &mut self,
        contents: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(path) = settings_path() else {
            return;
        };
        let Some(editor) = self
            .tabs
            .iter()
            .find(|tab| tab.path.as_deref() == Some(path.as_path()) && !tab.dirty)
            .map(|tab| tab.editor.clone())
        else {
            return;
        };
        // `set_value` clears the undo history and emits no change event, so the
        // refreshed tab is not marked dirty by this reload.
        editor.update(cx, |state, cx| state.set_value(contents, window, cx));
    }

    fn on_next_tab(&mut self, _: &NextTab, window: &mut Window, cx: &mut Context<Self>) {
        self.shift_active_tab(1, window, cx);
    }

    fn on_previous_tab(&mut self, _: &PreviousTab, window: &mut Window, cx: &mut Context<Self>) {
        self.shift_active_tab(-1, window, cx);
    }

    fn shift_active_tab(&mut self, delta: isize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(active) = self.active_tab else {
            return;
        };
        let n = self.tabs.len();
        if n == 0 {
            return;
        }
        let next = (active as isize + delta).rem_euclid(n as isize) as usize;
        self.activate_tab(next, window, cx);
    }

    fn on_jump_to_tab(&mut self, action: &JumpToTab, window: &mut Window, cx: &mut Context<Self>) {
        let n = action.0;
        if n == 0 || n > self.tabs.len() {
            return;
        }
        self.activate_tab(n - 1, window, cx);
    }

    fn on_reopen_closed_tab(
        &mut self,
        _: &ReopenClosedTab,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(closed) = self.closed_tabs.pop() else {
            return;
        };
        match closed.path {
            Some(path) => self.request_open(path, window, cx),
            None => self.add_tab(None, closed.content, window, cx),
        }
    }

    /// Pushes the currently active editor settings onto every open editor, so
    /// a live toggle or a settings reload re-applies without reopening any tab.
    fn apply_editor_options_to_all(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let editor = self.settings.editor.clone();
        let tab = TabSize {
            tab_size: editor.tab_size(),
            hard_tabs: editor.hard_tabs(),
        };
        let editors: Vec<_> = self.tabs.iter().map(|tab| tab.editor.clone()).collect();
        for state in editors {
            state.update(cx, |state, cx| {
                state.set_soft_wrap(editor.soft_wrap(), window, cx);
                state.set_show_whitespaces(editor.show_whitespace(), window, cx);
                state.set_tab_size(tab, cx);
                state.set_line_number(editor.line_numbers(), window, cx);
                state.set_indent_guides(editor.indent_guides(), window, cx);
                state.set_folding(editor.folding(), window, cx);
                state.set_auto_close(editor.auto_close(), window, cx);
                state.set_smart_indent(editor.smart_indent(), window, cx);
                state.set_wrapping_indent(wrapping_indent(&editor), window, cx);
                state.set_scroll_beyond_last_line(scroll_beyond_last_line(&editor), window, cx);
                state.set_cursor_surrounding_lines(editor.cursor_surrounding_lines, window, cx);
            });
        }
    }

    fn on_install_cli(&mut self, _: &InstallCli, window: &mut Window, cx: &mut Context<Self>) {
        let Some(bundle) = app_bundle_path() else {
            window.push_notification(
                Notification::error(
                    "Run the bundled Tinytext.app to install the command-line tool".to_string(),
                ),
                cx,
            );
            return;
        };

        self.busy = Some("Installing command-line tool…".into());
        cx.notify();

        cx.spawn_in(window, async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { install_cli(&bundle) })
                .await;

            this.update_in(cx, |this, window, cx| {
                this.busy = None;
                match result {
                    Ok(path) => window.push_notification(
                        Notification::success(format!("Installed command at {}", path.display())),
                        cx,
                    ),
                    Err(message) => window.push_notification(
                        Notification::error(format!("Could not install command: {message}")),
                        cx,
                    ),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn on_check_for_updates(
        &mut self,
        _: &CheckForUpdates,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.busy = Some("Checking for updates…".into());
        cx.notify();

        cx.spawn_in(window, async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async { latest_version() })
                .await;

            this.update_in(cx, |this, window, cx| {
                this.busy = None;
                let current = env!("CARGO_PKG_VERSION");
                match result {
                    Ok(latest) if is_newer(&latest, current) => {
                        window.open_alert_dialog(cx, move |alert, _, _| {
                            alert
                                .title("Update Available")
                                .description(
                                    div()
                                        .v_flex()
                                        .gap_1()
                                        .child(format!(
                                            "Tinytext {latest} is available. You have {current}."
                                        ))
                                        .child("Save your work, then run this in Terminal:")
                                        .child(INSTALL_COMMAND),
                                )
                                .ok_text("Copy Command")
                                .show_cancel(true)
                                .cancel_text("Later")
                                .on_ok(|_, window, cx| {
                                    cx.write_to_clipboard(ClipboardItem::new_string(
                                        INSTALL_COMMAND.to_string(),
                                    ));
                                    window.push_notification(
                                        Notification::success("Copied install command"),
                                        cx,
                                    );
                                    true
                                })
                        });
                    }
                    Ok(_) => window.push_notification(
                        Notification::success(format!("Tinytext {current} is up to date")),
                        cx,
                    ),
                    Err(message) => window.push_notification(
                        Notification::error(format!("Could not check for updates: {message}")),
                        cx,
                    ),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn on_open_settings(&mut self, _: &OpenSettings, _window: &mut Window, cx: &mut Context<Self>) {
        self.settings_open = !self.settings_open;
        self.settings_error = None;
        cx.notify();
    }

    /// Opens the generated "Default Settings" document: every key with the
    /// value it takes when unset. The tab is an ordinary untitled document, so
    /// nothing is written to disk and the session never restores it.
    fn on_open_default_settings(
        &mut self,
        _: &OpenDefaultSettings,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let document = defaults_document(
            cx.theme().mono_font_family.as_ref(),
            cx.theme().mono_font_size.as_f32(),
        );
        self.add_document_tab("Default Settings", "JSON", document, window, cx);
    }

    /// Reveals the user themes directory in Finder so installed color schemes
    /// are easy to find and add to.
    fn on_open_themes_folder(
        &mut self,
        _: &OpenThemesFolder,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(dir) = crate::theme::themes_dir() {
            crate::theme::ensure_dir(&dir);
            cx.reveal_path(&dir);
        }
    }

    /// Opens `settings.json` as a text tab, creating a starter file on first
    /// use. Reached from the Settings overlay for direct edits.
    pub(super) fn open_settings_file(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(path) = settings_path() else {
            return;
        };

        if !path.exists() {
            let starter = Settings {
                editor: EditorSettings {
                    font_family: Some(cx.theme().mono_font_family.to_string()),
                    font_size: Some(cx.theme().mono_font_size.as_f32()),
                    font_weight: Some(DEFAULT_FONT_WEIGHT),
                    ..Default::default()
                },
                ui: Default::default(),
            };
            let written = serde_json::to_string_pretty(&starter)
                .map_err(std::io::Error::other)
                .and_then(|json| {
                    if let Some(parent) = path.parent() {
                        std::fs::create_dir_all(parent)?;
                    }
                    std::fs::write(&path, json + "\n")
                });
            if let Err(error) = written {
                window.push_notification(
                    Notification::error(format!("Could not create settings: {error}")),
                    cx,
                );
                return;
            }
        }

        self.request_open(path, window, cx);
    }

    /// Re-applies settings when the saved file is the settings file. Invalid
    /// JSON keeps the previous settings and reports the parse error.
    pub(super) fn reload_settings_if(
        &mut self,
        path: &Path,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if settings_path().as_deref() != Some(path) {
            return;
        }

        match load_settings() {
            Ok(settings) => {
                self.settings = settings;
                // Re-run the editor-only options and the theme on top of what
                // was already open, so editing the settings file re-applies.
                self.apply_editor_options_to_all(window, cx);
                let choice = self.settings.ui.theme().to_string();
                self.apply_theme_choice(&choice, Some(window), cx);
                self.check_font_family(window, cx);
                cx.notify();
            }
            Err(message) => window.push_notification(Notification::error(message), cx),
        }
    }

    /// Warns when the configured editor font is not installed. Font enumeration
    /// is slow on macOS, so it runs off the main thread.
    pub(crate) fn check_font_family(&self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(family) = self.settings.editor.font_family.clone() else {
            return;
        };
        let text_system = cx.text_system().clone();

        cx.spawn_in(window, async move |this, cx| {
            let installed = cx
                .background_executor()
                .spawn(async move { text_system.all_font_names() })
                .await;
            if let Some(message) = font_family_issue(&family, &installed) {
                this.update_in(cx, |_this, window, cx| {
                    window.push_notification(Notification::warning(message), cx);
                })
                .ok();
            }
        })
        .detach();
    }

    /// Re-renders while a modifier is held so the status bar can hint at the
    /// Option-click and Option-drag multi-cursor gestures.
    fn on_modifiers_changed(
        &mut self,
        _: &ModifiersChangedEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        cx.notify();
    }

    fn on_about(&mut self, _: &About, window: &mut Window, cx: &mut Context<Self>) {
        window.open_alert_dialog(cx, |alert, _, cx| {
            alert
                .description(
                    div()
                        .v_flex()
                        .w_full()
                        .items_center()
                        .gap_2()
                        .child(logo(96.))
                        .child(
                            div()
                                .text_lg()
                                .font_weight(FontWeight::BOLD)
                                .text_color(cx.theme().foreground)
                                .child("Tinytext"),
                        )
                        .child(format!("Version {}", env!("CARGO_PKG_VERSION")))
                        .child("A native macOS text editor built with GPUI Kit.")
                        .child(
                            div()
                                .v_flex()
                                .items_center()
                                .gap_1()
                                .child(
                                    Link::new("about-repository")
                                        .href(env!("CARGO_PKG_REPOSITORY"))
                                        .child(env!("CARGO_PKG_REPOSITORY")),
                                )
                                .child(
                                    Link::new("about-homepage")
                                        .href(env!("CARGO_PKG_HOMEPAGE"))
                                        .child(env!("CARGO_PKG_HOMEPAGE")),
                                ),
                        ),
                )
                .ok_text("Close")
        });
    }
}

/// The Tinytext logo image, decoded once and reused across renders.
fn logo_image() -> Arc<Image> {
    static LOGO: OnceLock<Arc<Image>> = OnceLock::new();
    LOGO.get_or_init(|| {
        Arc::new(Image::from_bytes(
            ImageFormat::Png,
            include_bytes!("../../assets/icon.png").to_vec(),
        ))
    })
    .clone()
}

/// The app logo rendered at the given edge length in pixels.
pub(super) fn logo(size: f32) -> Img {
    img(logo_image()).w(px(size)).h(px(size))
}

impl Render for TinytextApp {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.sync_window_title(window);

        let palette = self
            .palette
            .clone()
            .map(|state| self.render_palette(&state, cx));
        let settings_overlay = self.settings_open.then(|| self.render_settings(cx));
        // Only worth hinting while an editor is open; other panes have no carets.
        let alt_held = window.modifiers().alt && self.active().is_some();

        let root = div()
            .id("tinytext-root")
            .relative()
            .v_flex()
            .size_full()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .text_sm()
            .track_focus(&self.focus_handle)
            .on_modifiers_changed(cx.listener(Self::on_modifiers_changed))
            .on_action(cx.listener(Self::on_new_file))
            .on_action(cx.listener(Self::on_open_file))
            .on_action(cx.listener(Self::on_add_folder))
            .on_action(cx.listener(Self::on_close_all_folders))
            .on_action(cx.listener(Self::on_save_file))
            .on_action(cx.listener(Self::on_save_file_as))
            .on_action(cx.listener(Self::on_save_all))
            .on_action(cx.listener(Self::on_close_tab))
            .on_action(cx.listener(Self::on_quit))
            .on_action(cx.listener(Self::on_toggle_sidebar))
            .on_action(cx.listener(Self::on_refresh))
            .on_action(cx.listener(Self::on_install_cli))
            .on_action(cx.listener(Self::on_check_for_updates))
            .on_action(cx.listener(Self::on_about))
            .on_action(cx.listener(Self::on_open_settings))
            .on_action(cx.listener(Self::on_open_default_settings))
            .on_action(cx.listener(Self::on_open_themes_folder))
            .on_action(cx.listener(Self::on_reveal_in_finder))
            .on_action(cx.listener(Self::on_copy_file_path))
            .on_action(cx.listener(Self::on_copy_relative_path))
            .on_action(cx.listener(Self::on_zoom_in))
            .on_action(cx.listener(Self::on_zoom_out))
            .on_action(cx.listener(Self::on_zoom_reset))
            .on_action(cx.listener(Self::on_toggle_word_wrap))
            .on_action(cx.listener(Self::on_toggle_whitespace))
            .on_action(cx.listener(Self::on_toggle_line_numbers))
            .on_action(cx.listener(Self::on_toggle_indent_guides))
            .on_action(cx.listener(Self::on_toggle_folding))
            .on_action(cx.listener(Self::on_toggle_auto_close))
            .on_action(cx.listener(Self::on_toggle_smart_indent))
            .on_action(cx.listener(Self::on_toggle_markdown_preview))
            .on_action(cx.listener(Self::on_set_theme))
            .on_action(cx.listener(Self::on_next_tab))
            .on_action(cx.listener(Self::on_previous_tab))
            .on_action(cx.listener(Self::on_jump_to_tab))
            .on_action(cx.listener(Self::on_reopen_closed_tab))
            .on_action(cx.listener(Self::on_toggle_line_comment))
            .on_action(cx.listener(Self::on_delete_line))
            .on_action(cx.listener(Self::on_move_line_up))
            .on_action(cx.listener(Self::on_move_line_down))
            .on_action(cx.listener(Self::on_copy_line_up))
            .on_action(cx.listener(Self::on_copy_line_down))
            .on_action(cx.listener(Self::on_insert_line_above))
            .on_action(cx.listener(Self::on_insert_line_below))
            .on_action(cx.listener(Self::on_select_line))
            .on_action(cx.listener(Self::on_go_to_line))
            .on_action(cx.listener(Self::on_add_cursor_above))
            .on_action(cx.listener(Self::on_add_cursor_below))
            .on_action(cx.listener(Self::on_split_selection_into_lines))
            .on_action(cx.listener(Self::on_add_next_occurrence))
            .on_action(cx.listener(Self::on_command_palette))
            .child(self.render_workspace(window, cx))
            .child(self.render_status_bar(alt_held, cx));

        let root = match palette {
            Some(overlay) => root.child(overlay),
            None => root,
        };

        match settings_overlay {
            Some(overlay) => root.child(overlay).into_any_element(),
            None => root.into_any_element(),
        }
    }
}
