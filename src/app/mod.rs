mod chrome;
mod editor;
mod files;
mod find_in_files;
mod overlays;
mod palette;
mod preferences;
mod quick_open;
mod tabs;
mod ui;
mod workspace;

use self::chrome::Chrome;
use self::overlays::{FindText, Overlays};
use self::preferences::SettingsState;
use self::tabs::{OpenTab, Preview, TabSet};
use self::workspace::Workspace;

use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use gpui_kit::base::StyledExt as _;
use gpui_kit::component::{
    ActiveTheme as _, WindowExt as _,
    input::{TabSize, WrappingIndent},
    link::Link,
    notification::Notification,
    resizable::ResizableState,
};
use gpui_kit::*;

use crate::cli::{install_cli, launcher_target};
use crate::session::{SessionState, load_session, session_path};
use crate::settings::{
    DEFAULT_FONT_WEIGHT, EditorSettings, MAX_FONT_SIZE, MIN_FONT_SIZE, Settings, UiSettings,
    defaults_document, font_family_issue, load_settings, save_editor_flag, save_font_size,
    save_show_whitespace, save_soft_wrap, save_theme, settings_path, ui_font_family_issue,
};
use crate::update::{INSTALL_COMMAND, is_newer, latest_version};
use crate::{
    About, CheckForUpdates, ClearRecent, CopyFilePath, CopyRelativePath, InstallCli, JumpToTab,
    NextTab, OpenDefaultSettings, OpenSettings, OpenThemesFolder, PreviousTab, Quit, Refresh,
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

pub(crate) struct TinytextApp {
    pub(crate) focus_handle: FocusHandle,
    /// The open folders, the expanded set, and the rows rendered from them.
    workspace: Workspace,
    /// The open tabs and the indexes that track the focused one.
    tabs: TabSet,
    /// The palette, the finder and the finder's file index.
    overlays: Overlays,
    /// The window title, the close guard, and the status-bar readouts.
    chrome: Chrome,
    /// The user's preferences and the Settings overlay's own state.
    settings: SettingsState,
}

impl TinytextApp {
    pub(crate) fn new(
        workspace_roots: Vec<PathBuf>,
        settings: Settings,
        cx: &mut Context<Self>,
    ) -> Self {
        // The sidebar starts open only when there is a folder to show. Toggling
        // it later is independent of the roots, and reveals the empty state.
        let mut app = Self {
            focus_handle: cx.focus_handle(),
            workspace: Workspace::new(workspace_roots, cx.new(|_| ResizableState::default())),
            tabs: TabSet::new(),
            overlays: Overlays::new(),
            chrome: Chrome::new(),
            settings: SettingsState::new(settings),
        };
        app.workspace.reload();
        // Read on its own because the session is skipped when the app is
        // launched with paths, but the recent list and Find text still apply.
        let session = load_session().unwrap_or_default();
        app.workspace.set_recent(session.recent);
        app.overlays.set_find_text(FindText {
            query: session.find_query,
            scope: session.find_where,
            replace: session.find_replace,
        });
        *app.overlays.find_options_mut() = session.find_options;
        app.refresh_menus(cx);
        app
    }

    /// Rebuilds the menu bar, whose Appearance and Open Recent submenus show
    /// app state.
    fn refresh_menus(&self, cx: &mut App) {
        let theme = self.settings.values.ui.theme().to_string();
        cx.set_menus(crate::commands::menus(&theme, self.workspace.recent()));
    }

    /// Records a file or folder the user opened, for File > Open Recent.
    pub(super) fn note_recent(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        self.workspace.note_recent(path);
        self.save_session();
        self.refresh_menus(cx);
    }

    fn on_clear_recent(&mut self, _: &ClearRecent, _window: &mut Window, cx: &mut Context<Self>) {
        self.workspace.clear_recent();
        self.save_session();
        self.refresh_menus(cx);
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

        if self.chrome.title_changed(&title) {
            window.set_window_title(&title);
        }
    }

    pub(crate) fn restore_session(
        &mut self,
        session: SessionState,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.workspace.restore(&session);

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

        let mut expanded: Vec<PathBuf> = self.workspace.expanded().iter().cloned().collect();
        expanded.sort();

        let state = SessionState {
            roots: self.workspace.roots().to_vec(),
            tabs: self
                .tabs
                .iter()
                .filter_map(|tab| tab.path.clone())
                .collect(),
            active_tab: self.active().and_then(|tab| tab.path.clone()),
            expanded,
            selected_path: self.workspace.selected().map(Path::to_path_buf),
            sidebar_visible: self.workspace.sidebar_visible(),
            sidebar_width: Some(self.workspace.width()),
            recent: self.workspace.recent().to_vec(),
            find_query: self.overlays.find_text().query.clone(),
            find_where: self.overlays.find_text().scope.clone(),
            find_replace: self.overlays.find_text().replace.clone(),
            find_options: self.overlays.find_options(),
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
    /// Call after any filesystem change or when the roots change. Kept on the
    /// app because `main` triggers it on window activation and cannot reach the
    /// workspace internals.
    pub(crate) fn reload_tree(&mut self) {
        self.workspace.reload();
    }

    fn active(&self) -> Option<&OpenTab> {
        self.tabs.active()
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
        if !self.any_dirty() || self.chrome.allow_close() {
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
        self.workspace.reload();
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
        let text = match crate::paths::nearest_root(self.workspace.roots(), path) {
            Some(root) => crate::paths::relative_display(path, root),
            None => path.display().to_string(),
        };
        cx.write_to_clipboard(ClipboardItem::new_string(text));
    }

    fn toggle_sidebar(&mut self, cx: &mut Context<Self>) {
        self.workspace.toggle_sidebar_visible();
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
        self.settings.values.editor.font_size = None;
        self.after_setting_write(save_font_size(None), window, cx);
        cx.notify();
    }

    /// Shifts the editor font size by a whole-point step, clamped to the same
    /// bounds the settings accessor enforces. Applies across every open tab
    /// because the size is read from `self.settings.values` at render time.
    fn step_font_size(&mut self, step: f32, window: &mut Window, cx: &mut Context<Self>) {
        let base = self
            .settings
            .values
            .editor
            .font_size()
            .unwrap_or_else(|| cx.theme().mono_font_size.as_f32());
        self.settings.values.editor.font_size =
            Some((base + step).clamp(MIN_FONT_SIZE, MAX_FONT_SIZE));
        self.after_setting_write(
            save_font_size(self.settings.values.editor.font_size),
            window,
            cx,
        );
        cx.notify();
    }

    fn on_toggle_word_wrap(
        &mut self,
        _: &ToggleWordWrap,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.settings.values.editor.soft_wrap = Some(!self.settings.values.editor.soft_wrap());
        self.apply_editor_options_to_all(window, cx);
        self.after_setting_write(
            save_soft_wrap(self.settings.values.editor.soft_wrap()),
            window,
            cx,
        );
        cx.notify();
    }

    fn on_toggle_whitespace(
        &mut self,
        _: &ToggleWhitespace,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.settings.values.editor.show_whitespace =
            Some(!self.settings.values.editor.show_whitespace());
        self.apply_editor_options_to_all(window, cx);
        self.after_setting_write(
            save_show_whitespace(self.settings.values.editor.show_whitespace()),
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
        let enabled = !self.settings.values.editor.line_numbers();
        self.set_editor_flag("line_numbers", |e| &mut e.line_numbers, enabled, window, cx);
    }

    fn on_toggle_indent_guides(
        &mut self,
        _: &ToggleIndentGuides,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let enabled = !self.settings.values.editor.indent_guides();
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
        let enabled = !self.settings.values.editor.folding();
        self.set_editor_flag("folding", |e| &mut e.folding, enabled, window, cx);
    }

    fn on_toggle_auto_close(
        &mut self,
        _: &ToggleAutoClose,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let enabled = !self.settings.values.editor.auto_close();
        self.set_editor_flag("auto_close", |e| &mut e.auto_close, enabled, window, cx);
    }

    fn on_toggle_smart_indent(
        &mut self,
        _: &ToggleSmartIndent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let enabled = !self.settings.values.editor.smart_indent();
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
        let Some(ix) = self.tabs.active_index() else {
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
        *field(&mut self.settings.values.editor) = Some(enabled);
        self.apply_editor_options_to_all(window, cx);
        self.after_setting_write(save_editor_flag(key, enabled), window, cx);
        cx.notify();
    }

    /// Applies an appearance picked from the Appearance menu or the command
    /// palette, persisting it and refreshing the menu's check mark.
    fn on_set_theme(&mut self, action: &SetTheme, window: &mut Window, cx: &mut Context<Self>) {
        let choice = action.0.clone();
        self.settings.values.ui.theme = Some(choice.clone());
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
        let choice = self.settings.values.ui.theme();
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
        crate::theme::apply(&self.settings.values, window, cx);
        cx.set_menus(crate::commands::menus(choice, self.workspace.recent()));
    }

    /// Re-applies the current appearance and color scheme without a window, used
    /// after the themes directory finishes loading so a scheme named in settings
    /// that only just became available takes effect.
    pub(super) fn reapply_theme(&mut self, cx: &mut Context<Self>) {
        let choice = self.settings.values.ui.theme().to_string();
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
        let Some(active) = self.tabs.active_index() else {
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
        let Some(closed) = self.tabs.pop_closed() else {
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
        let editor = self.settings.values.editor.clone();
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
        let Some(app) = launcher_target() else {
            window.push_notification(
                Notification::error(
                    "Could not locate Tinytext to install the command-line tool".to_string(),
                ),
                cx,
            );
            return;
        };

        self.chrome
            .set_busy(Some("Installing command-line tool…".into()));
        cx.notify();

        cx.spawn_in(window, async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { install_cli(&app) })
                .await;

            this.update_in(cx, |this, window, cx| {
                this.chrome.set_busy(None);
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
        self.chrome.set_busy(Some("Checking for updates…".into()));
        cx.notify();

        cx.spawn_in(window, async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async { latest_version() })
                .await;

            this.update_in(cx, |this, window, cx| {
                this.chrome.set_busy(None);
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
        self.settings.toggle_open();
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
            cx.theme().font_family.as_ref(),
            cx.theme().font_size.as_f32(),
        );
        self.add_document_tab("Default Settings", "JSON", document, window, cx);
    }

    /// Reveals the user themes directory in the file manager so installed color
    /// schemes are easy to find and add to.
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
                ui: UiSettings {
                    font_family: Some(cx.theme().font_family.to_string()),
                    font_size: Some(cx.theme().font_size.as_f32()),
                    ..Default::default()
                },
                ..Default::default()
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
                self.settings.values = settings;
                // Re-run the editor-only options and the theme on top of what
                // was already open, so editing the settings file re-applies.
                self.apply_editor_options_to_all(window, cx);
                let choice = self.settings.values.ui.theme().to_string();
                self.apply_theme_choice(&choice, Some(window), cx);
                self.check_font_family(window, cx);
                cx.notify();
            }
            Err(message) => window.push_notification(Notification::error(message), cx),
        }
    }

    /// Warns when a configured editor or interface font is not installed. Font
    /// enumeration is slow on macOS, so it runs off the main thread.
    pub(crate) fn check_font_family(&self, window: &mut Window, cx: &mut Context<Self>) {
        let editor = self.settings.values.editor.font_family.clone();
        let interface = self.settings.values.ui.font_family.clone();
        if editor.is_none() && interface.is_none() {
            return;
        }
        let text_system = cx.text_system().clone();

        cx.spawn_in(window, async move |this, cx| {
            let installed = cx
                .background_executor()
                .spawn(async move { text_system.all_font_names() })
                .await;
            let mut issues = Vec::new();
            if let Some(family) = &editor {
                issues.extend(font_family_issue(family, &installed));
            }
            if let Some(family) = &interface {
                issues.extend(ui_font_family_issue(family, &installed));
            }
            if issues.is_empty() {
                return;
            }
            this.update_in(cx, |_this, window, cx| {
                for issue in issues {
                    window.push_notification(Notification::warning(issue), cx);
                }
            })
            .ok();
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
            .overlays
            .palette()
            .cloned()
            .map(|state| self.render_palette(&state, cx));
        let quick_open = self
            .overlays
            .quick_open()
            .cloned()
            .map(|state| self.render_quick_open(&state, cx));
        let find_panel = self.render_find_panel(cx);
        let settings_overlay = self.settings.is_open().then(|| self.render_settings(cx));
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
            .on_action(cx.listener(Self::on_clear_recent))
            // Files and folders dragged in from the file manager open like Open… does.
            .on_drop(cx.listener(|this, paths: &ExternalPaths, window, cx| {
                for path in paths.paths() {
                    this.request_open(path.clone(), window, cx);
                }
            }))
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
            .on_action(cx.listener(Self::on_quick_open))
            .on_action(cx.listener(Self::on_find_in_files))
            .on_action(cx.listener(Self::on_run_find))
            .on_action(cx.listener(Self::on_replace_all))
            .on_action(cx.listener(Self::on_close_find_panel))
            .on_action(cx.listener(Self::on_find_next_field))
            .on_action(cx.listener(Self::on_find_previous_field))
            .on_action(cx.listener(Self::on_open_result))
            .on_action(cx.listener(Self::on_next_result))
            .on_action(cx.listener(Self::on_previous_result))
            .on_action(cx.listener(Self::on_open_path))
            .child(self.render_workspace(window, cx))
            .children(find_panel)
            .child(self.render_status_bar(alt_held, cx));

        let root = match palette {
            Some(overlay) => root.child(overlay),
            None => root,
        };
        let root = match quick_open {
            Some(overlay) => root.child(overlay),
            None => root,
        };

        match settings_overlay {
            Some(overlay) => root.child(overlay).into_any_element(),
            None => root.into_any_element(),
        }
    }
}

/// Fixtures for the overlay tests.
///
/// The tests render the real app in gpui-kit's headless window, so they need
/// the same setup `main` performs: the kit initialized and the keymap bound.
#[cfg(test)]
pub(super) mod test_support {
    use gpui_kit::{
        AnyWindowHandle, AppContext as _, Entity, TestAppContext, WindowOptions, open_window,
    };

    use super::TinytextApp;
    use crate::keymap;
    use crate::settings::Settings;

    /// A headless window running the app with no workspace folders open.
    pub(super) fn open_app(cx: &mut TestAppContext) -> (AnyWindowHandle, Entity<TinytextApp>) {
        cx.update(|cx| {
            gpui_kit::init(cx);
            keymap::bind_keys(cx);
        });
        cx.update(|cx| {
            open_window(WindowOptions::default(), cx, |_window, cx| {
                cx.new(|cx| TinytextApp::new(Vec::new(), Settings::default(), cx))
            })
            .unwrap()
        })
    }
}
