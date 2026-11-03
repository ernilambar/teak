use std::cell::Cell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use gpui_kit::component::{
    WindowExt as _,
    button::ButtonVariant,
    input::{EditorState, Input, InputState},
    notification::Notification,
};
use gpui_kit::*;

use crate::language::{editor_language_id, language_for};
use crate::paths::{
    copy_entry, duplicate_name, file_name, is_valid_entry_name, move_to_trash, remap_prefix,
    same_file,
};
use crate::watcher::Change;
use crate::windows::{WindowSeed, cascaded_frame};
use crate::workspace_file;
use crate::{
    AddFolder, CloseAllFolders, CloseWorkspace, NewFile, OpenFile, OpenFolderInNewWindow,
    OpenWorkspace, SaveAll, SaveFile, SaveFileAs, SaveWorkspaceAs,
};

use super::TeakApp;
use super::workspace::{AddRoot, FileClipboard};

/// A workspace being opened: its file and the folders it declared.
type WorkspaceTarget = (PathBuf, Vec<PathBuf>);

/// An action that waits for the tabs to be saved: a method on the app plus the
/// workspace it should act on, if any.
type GuardedAction = fn(&mut TeakApp, Option<WorkspaceTarget>, &mut Window, &mut Context<TeakApp>);

/// Told how one save ended: `true` when the file was written, `false` when a
/// prompt was cancelled or the write failed.
type SaveDone = Rc<dyn Fn(bool, &mut TeakApp, &mut Window, &mut Context<TeakApp>)>;

/// A file read for a new tab: text for the editor, or a raster image to view.
pub(super) enum FileContent {
    /// `modified` is the file's mtime taken before its bytes were read, so a
    /// write landing during the read shows up as a later version.
    Text {
        text: String,
        format: TextFormat,
        modified: Option<SystemTime>,
    },
    Image(Arc<Image>),
}

/// Text files above this size open as Plain Text, since syntax highlighting
/// a huge file can stall the editor.
pub(super) const LARGE_FILE_BYTES: usize = 10 * 1024 * 1024;

/// How long a folder revealed by a drop or Add Folder keeps its accent tint.
const REVEAL_FLASH: Duration = Duration::from_millis(800);

/// Whether Option is held right now.
///
/// A platform file-drop event carries no modifier state, and the synthesized
/// drag events do not update it, so ask AppKit directly at the moment of the
/// drop instead of trusting `window.modifiers()`.
pub(super) fn option_held() -> bool {
    use objc2_app_kit::{NSEvent, NSEventModifierFlags};
    NSEvent::modifierFlags_class().contains(NSEventModifierFlags::Option)
}

const UTF8_BOM: &[u8] = b"\xEF\xBB\xBF";

/// How a text file is stored on disk, so saving writes it back the same way.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(super) struct TextFormat {
    /// Starts with a UTF-8 byte order mark, kept out of the editor's text.
    pub(super) bom: bool,
    /// Uses `\r\n` line breaks, judged from the first break.
    pub(super) crlf: bool,
    /// Held bytes that are not valid UTF-8; the editor shows them as U+FFFD.
    pub(super) lossy: bool,
}

impl TextFormat {
    /// Splits raw file bytes into editor text and the format to save it
    /// with. Binary content (a NUL byte near the start) is refused.
    pub(super) fn decode(bytes: &[u8]) -> std::io::Result<(String, Self)> {
        if bytes.iter().take(8192).any(|&byte| byte == 0) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "binary file",
            ));
        }
        let bom = bytes.starts_with(UTF8_BOM);
        let body = if bom { &bytes[UTF8_BOM.len()..] } else { bytes };
        let (text, lossy) = match std::str::from_utf8(body) {
            Ok(text) => (text.to_string(), false),
            Err(_) => (String::from_utf8_lossy(body).into_owned(), true),
        };
        let crlf = text
            .find('\n')
            .is_some_and(|ix| ix > 0 && text.as_bytes()[ix - 1] == b'\r');
        Ok((text, Self { bom, crlf, lossy }))
    }

    /// The bytes to write for `text`. A CRLF file gets `\r\n` for every bare
    /// `\n` typed or pasted since it was opened, and its BOM goes back on.
    pub(super) fn encode(&self, text: &str) -> Vec<u8> {
        let mut out = Vec::with_capacity(text.len() + UTF8_BOM.len());
        if self.bom {
            out.extend_from_slice(UTF8_BOM);
        }
        if !self.crlf {
            out.extend_from_slice(text.as_bytes());
            return out;
        }
        let mut previous = 0;
        for &byte in text.as_bytes() {
            if byte == b'\n' && previous != b'\r' {
                out.push(b'\r');
            }
            out.push(byte);
            previous = byte;
        }
        out
    }

    pub(super) fn encoding_label(&self) -> &'static str {
        if self.bom { "UTF-8 with BOM" } else { "UTF-8" }
    }

    pub(super) fn line_ending_label(&self) -> &'static str {
        if self.crlf { "CRLF" } else { "LF" }
    }
}

/// The raster format GPUI decodes for `path`'s extension. SVG is absent on
/// purpose: it opens as text and previews from the buffer.
fn raster_format(path: &Path) -> Option<ImageFormat> {
    let extension = path.extension()?.to_str()?.to_ascii_lowercase();
    match extension.as_str() {
        "png" => Some(ImageFormat::Png),
        "jpg" | "jpeg" => Some(ImageFormat::Jpeg),
        "gif" => Some(ImageFormat::Gif),
        "webp" => Some(ImageFormat::Webp),
        "bmp" => Some(ImageFormat::Bmp),
        "tif" | "tiff" => Some(ImageFormat::Tiff),
        "ico" => Some(ImageFormat::Ico),
        _ => None,
    }
}

/// A 1×1 transparent PNG, used by the image-viewer tests.
#[cfg(test)]
pub(super) const ONE_PIXEL_PNG: &[u8] = &[
    0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44, 0x52,
    0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1F, 0x15, 0xC4,
    0x89, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x44, 0x41, 0x54, 0x78, 0xDA, 0x63, 0x64, 0x60, 0xF8, 0x5F,
    0x0F, 0x00, 0x02, 0x87, 0x01, 0x80, 0xEB, 0x47, 0xBA, 0x92, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45,
    0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82,
];

/// Reads `path` for a new tab. Raster images load as raw bytes; everything
/// else is decoded as text.
pub(super) fn read_file(path: &Path) -> std::io::Result<FileContent> {
    let modified = modified_time(path);
    let bytes = std::fs::read(path)?;
    match raster_format(path) {
        Some(format) => Ok(FileContent::Image(Arc::new(Image::from_bytes(
            format, bytes,
        )))),
        None => {
            let (text, format) = TextFormat::decode(&bytes)?;
            Ok(FileContent::Text {
                text,
                format,
                modified,
            })
        }
    }
}

/// Tells `done`, if there is one, how a save ended.
fn report(
    done: Option<&SaveDone>,
    saved: bool,
    this: &mut TeakApp,
    window: &mut Window,
    cx: &mut Context<TeakApp>,
) {
    if let Some(done) = done {
        done(saved, this, window, cx);
    }
}

/// The file's last modification time, or `None` when it cannot be read (for
/// example, because the file no longer exists).
pub(super) fn modified_time(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path)
        .and_then(|meta| meta.modified())
        .ok()
}

/// The start of the editor's first line, enough to read a shebang without
/// copying a huge single-line file.
fn first_line(editor: &Entity<EditorState>, cx: &App) -> String {
    editor
        .read(cx)
        .text()
        .chars()
        .take_while(|&c| c != '\n')
        .take(128)
        .collect()
}

/// The notification shown when a file the user asked for cannot be opened.
pub(super) fn open_error(path: &Path, error: &std::io::Error) -> String {
    let filename = file_name(path);
    if error.kind() == std::io::ErrorKind::InvalidData {
        format!("Could not open {filename}: it looks like a binary file")
    } else {
        format!("Could not open {filename}: {error}")
    }
}

/// Copy for the single-field name prompt shared by create and rename.
struct NamePrompt {
    title: &'static str,
    description: String,
    initial: String,
    ok_text: &'static str,
}

impl TeakApp {
    /// Opens one path: a folder joins the workspace, a file gets a tab, and a
    /// file already in a tab is activated. Multi-file requests go through
    /// [`Self::request_open_many`], which keeps the given order.
    pub(crate) fn request_open(
        &mut self,
        path: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.request_open_many(vec![path], window, cx);
    }

    /// Opens one path in the preview slot: a browsing click reuses one tab
    /// until the user edits the file or opens it explicitly.
    pub(crate) fn request_open_transient(
        &mut self,
        path: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_many(vec![path], true, window, cx);
    }

    /// Opens several paths as one request. The tabs land in the order given,
    /// not in read-completion order, and the last path that resolves to a tab
    /// ends up active. Folders join the workspace; files already open are only
    /// activated, so a path listed twice opens one tab.
    pub(crate) fn request_open_many(
        &mut self,
        paths: Vec<PathBuf>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_many(paths, false, window, cx);
    }

    fn open_many(
        &mut self,
        mut paths: Vec<PathBuf>,
        transient: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // A workspace file is a folder set, not a tab: opening one switches the
        // window rather than showing its JSON. Any other paths in the same
        // request are dropped with it, since a workspace replaces the folder
        // set they would otherwise be resolved against.
        if let Some(ix) = paths
            .iter()
            .position(|path| workspace_file::is_workspace_path(path))
        {
            let path = paths.remove(ix);
            self.request_open_workspace(path, window, cx);
            return;
        }

        let mut order = Vec::with_capacity(paths.len());
        let mut to_read: Vec<PathBuf> = Vec::new();
        for path in paths {
            if path.is_dir() {
                self.add_root(path, window, cx);
                continue;
            }
            order.push(path.clone());
            if !self.tabs.is_open(&path) && !to_read.contains(&path) {
                to_read.push(path);
            }
        }

        if to_read.is_empty() {
            // An explicit open of a preview keeps the tab; a browsing click
            // leaves it in the slot.
            if !transient {
                self.promote_open_paths(&order);
            }
            self.finish_open_many(&order, window, cx);
            return;
        }

        cx.spawn_in(window, async move |this, cx| {
            // Start every read at once, then collect the results in argument
            // order so a slow file cannot move its tab.
            let reads: Vec<_> = to_read
                .into_iter()
                .map(|path| {
                    let read = cx.background_executor().spawn({
                        let path = path.clone();
                        async move { read_file(&path) }
                    });
                    (path, read)
                })
                .collect();

            let mut results = Vec::with_capacity(reads.len());
            for (path, read) in reads {
                results.push((path, read.await));
            }

            this.update_in(cx, |this, window, cx| {
                for (path, result) in results {
                    match result {
                        Ok(content) => {
                            // Another request may have opened it while the read
                            // was in flight; never add a second tab for it.
                            if !this.tabs.is_open(&path) {
                                this.add_file_tab(path, content, transient, window, cx);
                            }
                        }
                        Err(error) => window
                            .push_notification(Notification::error(open_error(&path, &error)), cx),
                    }
                }
                if !transient {
                    this.promote_open_paths(&order);
                }
                this.finish_open_many(&order, window, cx);
            })
            .ok();
        })
        .detach();
    }

    /// Ends the preview lifetime of every path in an explicit open.
    fn promote_open_paths(&mut self, order: &[PathBuf]) {
        for path in order {
            if let Some(ix) = self.tabs.index_of_path(path) {
                self.tabs.promote(ix);
            }
        }
    }

    /// Records each opened path, in the order given, and activates the last one
    /// that made it into a tab. A path that failed to read is skipped.
    fn finish_open_many(&mut self, order: &[PathBuf], window: &mut Window, cx: &mut Context<Self>) {
        let mut last = None;
        for path in order {
            let Some(ix) = self.tabs.index_of_path(path) else {
                continue;
            };
            self.note_recent(path.clone(), cx);
            last = Some(ix);
        }
        if let Some(ix) = last {
            self.activate_tab(ix, window, cx);
        }
    }

    pub(super) fn on_new_file(&mut self, _: &NewFile, window: &mut Window, cx: &mut Context<Self>) {
        self.add_tab(None, String::new(), window, cx);
    }

    pub(super) fn on_open_file(
        &mut self,
        _: &OpenFile,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let receiver = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: true,
            prompt: Some("Open File".into()),
        });

        cx.spawn_in(window, async move |this, cx| {
            if let Ok(Ok(Some(paths))) = receiver.await {
                this.update_in(cx, |this, window, cx| {
                    this.request_open_many(paths, window, cx);
                })
                .ok();
            }
        })
        .detach();
    }

    pub(super) fn on_add_folder(
        &mut self,
        _: &AddFolder,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let receiver = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: true,
            prompt: Some("Add Folder".into()),
        });

        cx.spawn_in(window, async move |this, cx| {
            if let Ok(Ok(Some(paths))) = receiver.await {
                this.update_in(cx, |this, window, cx| {
                    for path in paths {
                        this.add_root(path, window, cx);
                    }
                })
                .ok();
            }
        })
        .detach();
    }

    pub(super) fn on_open_folder_in_new_window(
        &mut self,
        _: &OpenFolderInNewWindow,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let receiver = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: true,
            prompt: Some("Open Folder in New Window".into()),
        });
        let settings = self.settings.values.clone();
        let frame = Some(cascaded_frame(window));
        let services = self.services.clone();
        cx.spawn_in(window, async move |_this, cx| {
            if let Ok(Ok(Some(paths))) = receiver.await {
                let paths: Vec<PathBuf> = paths.into_iter().filter(|path| path.is_dir()).collect();
                if paths.is_empty() {
                    return;
                }
                let mut seed = WindowSeed::for_paths(settings, paths);
                seed.frame = frame;
                cx.update(|_window, cx| {
                    services.open_window(cx, seed);
                })
                .ok();
            }
        })
        .detach();
    }

    /// Opens a workspace file from the picker, switching this window to it.
    pub(super) fn on_open_workspace(
        &mut self,
        _: &OpenWorkspace,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let receiver = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("Open Workspace".into()),
        });

        cx.spawn_in(window, async move |this, cx| {
            if let Ok(Ok(Some(paths))) = receiver.await
                && let Some(path) = paths.into_iter().next()
            {
                this.update_in(cx, |this, window, cx| {
                    this.request_open_workspace(path, window, cx)
                })
                .ok();
            }
        })
        .detach();
    }

    /// Makes `path` this window's workspace: its folders replace the open ones
    /// and the tab set parked under it comes back. A malformed file changes
    /// nothing.
    pub(crate) fn request_open_workspace(
        &mut self,
        path: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let folders = match workspace_file::load(&path) {
            Ok(folders) => folders,
            Err(message) => {
                window.push_notification(Notification::error(message), cx);
                return;
            }
        };
        self.with_clean_tabs(
            window,
            cx,
            "Switching workspaces closes the open tabs. Save them first?",
            Self::switch_workspace,
            Some((path, folders)),
        );
    }

    /// Writes the open folders as a workspace file, then binds this window to
    /// it so later switches park their tabs under the new key.
    pub(super) fn on_save_workspace_as(
        &mut self,
        _: &SaveWorkspaceAs,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.workspace.roots().is_empty() {
            window.push_notification(
                Notification::warning("Open a folder before saving a workspace"),
                cx,
            );
            return;
        }

        let directory = self
            .workspace_file
            .as_deref()
            .and_then(Path::parent)
            .or_else(|| {
                self.workspace
                    .roots()
                    .first()
                    .and_then(|root| root.parent())
            })
            .map(Path::to_path_buf)
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
        let suggested = self
            .workspace_file
            .as_deref()
            .and_then(Path::file_stem)
            .map(|stem| stem.to_string_lossy().into_owned())
            .or_else(|| self.workspace.roots().first().map(|root| file_name(root)))
            .unwrap_or_else(|| "workspace".to_string());
        let suggested = format!("{suggested}.{}", workspace_file::EXTENSION);

        let receiver = cx.prompt_for_new_path(&directory, Some(suggested.as_str()));
        cx.spawn_in(window, async move |this, cx| {
            if let Ok(Ok(Some(path))) = receiver.await {
                this.update_in(cx, |this, window, cx| {
                    this.save_workspace_file(path, window, cx)
                })
                .ok();
            }
        })
        .detach();
    }

    /// Closes the workspace: its tabs and folders go, and the window becomes a
    /// plain one. The parked set stays in the session, so reopening the
    /// workspace later restores it.
    pub(super) fn on_close_workspace(
        &mut self,
        _: &CloseWorkspace,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.workspace_file.is_none() && self.workspace.roots().is_empty() {
            return;
        }
        self.with_clean_tabs(
            window,
            cx,
            "Closing the workspace closes its tabs. Save them first?",
            Self::close_workspace,
            None,
        );
    }

    /// Builds the workspace file and binds the window to it. A name the save
    /// pane returned without an extension gets one, so the file is recognized
    /// the next time it is opened.
    fn save_workspace_file(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        let path = if workspace_file::is_workspace_path(&path) {
            path
        } else {
            path.with_extension(workspace_file::EXTENSION)
        };
        match workspace_file::save(&path, self.workspace.roots()) {
            Ok(()) => {
                self.workspace_file = Some(path.clone());
                // Park the current tabs under the new name too.
                self.save_session(cx);
                self.note_recent(path.clone(), cx);
                window.push_notification(
                    Notification::success(format!("Saved {}", file_name(&path))),
                    cx,
                );
            }
            Err(message) => window.push_notification(Notification::error(message), cx),
        }
    }

    /// Continues with `action` once no tab has unsaved edits, offering to save
    /// everything first. A save that cannot finish — an Untitled tab or a file
    /// whose path is gone — stops the action instead of dropping the edits.
    fn with_clean_tabs(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
        description: &'static str,
        action: GuardedAction,
        target: Option<WorkspaceTarget>,
    ) {
        if !self.any_dirty() {
            action(self, target, window, cx);
            return;
        }

        let entity = cx.entity();
        window.open_alert_dialog(cx, move |alert, _window, _cx| {
            let entity = entity.clone();
            let target = target.clone();
            alert
                .confirm()
                .title("Unsaved Changes")
                .description(description)
                .ok_text("Save All and Continue")
                .cancel_text("Cancel")
                .on_ok(move |_, window, cx| {
                    entity.update(cx, |this, cx| {
                        this.save_all_then(action, target.clone(), window, cx);
                    });
                    true
                })
        });
    }

    /// Saves every edited tab, then runs `action` once the last write has
    /// landed. A cancelled prompt or a failed write stops the action; so does
    /// a tab that cannot be saved (Untitled, deleted on disk), with a warning.
    fn save_all_then(
        &mut self,
        action: GuardedAction,
        target: Option<WorkspaceTarget>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let targets = self.saveable_dirty_tabs();
        if targets.is_empty() {
            self.continue_if_clean(action, target, window, cx);
            return;
        }

        let pending = Rc::new(Cell::new(targets.len()));
        let failed = Rc::new(Cell::new(false));
        let done: SaveDone = Rc::new(move |saved, this, window, cx| {
            failed.set(failed.get() || !saved);
            pending.set(pending.get() - 1);
            if pending.get() == 0 && !failed.get() {
                this.continue_if_clean(action, target.clone(), window, cx);
            }
        });
        for (id, path) in targets {
            self.save_editor_then(id, path, Some(done.clone()), window, cx);
        }
    }

    /// Runs `action` unless a tab is still dirty, which after a Save All means
    /// it had nowhere to be written.
    fn continue_if_clean(
        &mut self,
        action: GuardedAction,
        target: Option<WorkspaceTarget>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.any_dirty() {
            window.push_notification(
                Notification::warning("Some files need a path before they can be saved."),
                cx,
            );
            return;
        }
        action(self, target, window, cx);
    }

    /// Switches this window to the workspace in `target`. The tabs it was
    /// showing are already parked by the preceding [`Self::save_session`], so
    /// closing them cannot lose the set.
    fn switch_workspace(
        &mut self,
        target: Option<WorkspaceTarget>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((path, folders)) = target else {
            return;
        };

        self.save_session(cx);
        let parked = self.services.parked_workspace(&path, cx);

        // Close the old tabs while unbound: saving per closed tab would
        // otherwise overwrite the set parked a line above with an empty one.
        self.workspace_file = None;
        self.close_all_tabs(window, cx);

        let mut session = self.window_session();
        session.workspace = Some(path.clone());
        session.roots = folders;
        session.tabs = parked
            .as_ref()
            .map(|state| state.tabs.clone())
            .unwrap_or_default();
        session.active_tab = parked.as_ref().and_then(|state| state.active_tab.clone());
        session.expanded = parked
            .map(|state| state.expanded)
            .unwrap_or_else(|| session.roots.clone());

        self.workspace_file = Some(path.clone());
        self.restore_session(session, Vec::new(), window, cx);
        // Opening a workspace is an open like any other, so it joins Open Recent.
        self.note_recent(path, cx);
        cx.notify();
    }

    /// Clears the workspace binding, its tabs and its folders. The parked tab
    /// set stays in the session for the next time the workspace is opened.
    fn close_workspace(
        &mut self,
        _target: Option<WorkspaceTarget>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.workspace_file = None;
        self.close_all_tabs(window, cx);
        self.close_all_roots(cx);
        cx.notify();
    }

    /// Handles paths dropped from the file manager. A plain drop adds folders
    /// to this workspace and opens files as tabs; holding Option opens
    /// everything in a new window instead.
    pub(super) fn on_external_drop(
        &mut self,
        paths: &ExternalPaths,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.chrome.set_external_drag(None);
        let paths = paths.paths().to_vec();
        if paths.is_empty() {
            cx.notify();
            return;
        }

        if option_held() {
            self.open_paths_in_new_window(paths, window, cx);
        } else {
            for path in paths {
                self.request_open(path, window, cx);
            }
        }
        cx.notify();
    }

    /// Opens `paths` in a new window: folders become workspace roots there and
    /// files become tabs, in the order given.
    fn open_paths_in_new_window(
        &mut self,
        paths: Vec<PathBuf>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let mut seed = WindowSeed::for_paths(self.settings.values.clone(), paths);
        seed.frame = Some(cascaded_frame(window));
        if self.services.open_window(cx, seed).is_none() {
            window.push_notification(Notification::error("Could not open a new window"), cx);
        }
    }

    /// Adds a folder to the sidebar. A folder that is already open, or that
    /// lives inside an open root, is revealed in the tree instead of being
    /// added twice; only a folder that *contains* an open root is refused,
    /// because it cannot be shown without nesting one root inside another.
    pub(super) fn add_root(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        let outcome = self.workspace.add_root(path.clone());
        let inside_open_root = self
            .workspace
            .roots()
            .iter()
            .any(|root| path.starts_with(root));
        if matches!(outcome, AddRoot::Overlap) && !inside_open_root {
            window.push_notification(
                Notification::error(format!(
                    "Cannot add \"{}\": it contains an open folder",
                    file_name(&path)
                )),
                cx,
            );
            return;
        }

        self.workspace.reveal(&path);
        if matches!(outcome, AddRoot::Added) {
            self.note_recent(path.clone(), cx);
            self.refresh_git_status(cx);
        }
        self.save_session(cx);
        self.flash_revealed(path, window, cx);
    }

    /// Tints a freshly revealed folder for a moment, so a drop that lands on an
    /// already-visible row still gives feedback.
    fn flash_revealed(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        self.workspace.set_flash(Some(path.clone()));
        cx.notify();

        cx.spawn_in(window, async move |this, cx| {
            cx.background_executor().timer(REVEAL_FLASH).await;
            this.update_in(cx, |this, _window, cx| {
                if this
                    .workspace
                    .flash()
                    .is_some_and(|flash| flash == path.as_path())
                {
                    this.workspace.set_flash(None);
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    pub(super) fn on_close_all_folders(
        &mut self,
        _: &CloseAllFolders,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // The open set no longer matches the workspace file, so the window stops
        // being bound to it.
        self.workspace_file = None;
        self.close_all_roots(cx);
    }

    /// Removes one workspace root and its subtree expansion state. Open tabs
    /// stay open, and the sidebar stays visible.
    pub(super) fn remove_root(&mut self, path: &Path, cx: &mut Context<Self>) {
        self.workspace.remove_root(path);
        self.refresh_git_status(cx);
        self.save_session(cx);
        cx.notify();
    }

    /// Removes every workspace root. Open tabs stay open and the sidebar stays
    /// visible, showing its empty state.
    pub(super) fn close_all_roots(&mut self, cx: &mut Context<Self>) {
        self.workspace.close_all();
        self.refresh_git_status(cx);
        self.save_session(cx);
        cx.notify();
    }

    pub(super) fn on_save_file(
        &mut self,
        _: &SaveFile,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(ix) = self.tabs.active_index() else {
            return;
        };
        let Some(tab) = self.tabs.get(ix) else {
            return;
        };
        // Find Results and diffs are generated; there is nothing to save.
        if tab.generated() {
            return;
        }
        let id = tab.document.editor_id();

        if let Some(path) = tab.document.path.clone() {
            self.save_editor(id, path, window, cx);
            return;
        }

        let directory = self
            .workspace
            .roots()
            .first()
            .cloned()
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
        let receiver = cx.prompt_for_new_path(&directory, Some("untitled.txt"));

        cx.spawn_in(window, async move |this, cx| {
            if let Ok(Ok(Some(path))) = receiver.await {
                this.update_in(cx, |this, window, cx| {
                    this.save_editor(id, path, window, cx)
                })
                .ok();
            }
        })
        .detach();
    }

    pub(super) fn on_save_file_as(
        &mut self,
        _: &SaveFileAs,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(ix) = self.tabs.active_index() else {
            return;
        };
        let Some(tab) = self.tabs.get(ix) else {
            return;
        };
        if tab.raster || tab.generated() {
            return;
        }
        let id = tab.document.editor_id();
        let suggested = tab
            .document
            .path
            .as_deref()
            .map(file_name)
            .unwrap_or_else(|| "untitled.txt".to_string());
        let directory = tab
            .document
            .path
            .as_deref()
            .and_then(Path::parent)
            .map(Path::to_path_buf)
            .or_else(|| self.workspace.roots().first().cloned())
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));

        let receiver = cx.prompt_for_new_path(&directory, Some(suggested.as_str()));

        cx.spawn_in(window, async move |this, cx| {
            if let Ok(Ok(Some(path))) = receiver.await {
                this.update_in(cx, |this, window, cx| this.save_as(id, path, window, cx))
                    .ok();
            }
        })
        .detach();
    }

    /// Saves tab `id` under `path`, unless another tab already shows that
    /// file: two documents on one path would each overwrite the other's
    /// edits, so that tab is brought forward instead.
    fn save_as(
        &mut self,
        id: EntityId,
        path: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(ix) = self.tabs.index_of_path(&path)
            && self.tabs.get(ix).map(|tab| tab.document.editor_id()) != Some(id)
        {
            self.activate_tab(ix, window, cx);
            window.push_notification(
                Notification::warning(format!("\"{}\" is already open", file_name(&path))),
                cx,
            );
            return;
        }
        self.save_editor(id, path, window, cx);
    }

    /// Saves every edited tab that still points at a file on disk. Untitled and
    /// deleted files are left alone, the latter so a save never recreates a file
    /// the user removed.
    pub(super) fn on_save_all(&mut self, _: &SaveAll, window: &mut Window, cx: &mut Context<Self>) {
        self.save_all_editors(window, cx);
    }

    /// The Save All body, shared with the workspace switch and close guards so
    /// every path that closes tabs saves the same set.
    fn save_all_editors(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        for (id, path) in self.saveable_dirty_tabs() {
            self.save_editor(id, path, window, cx);
        }
    }

    /// The edited tabs that still point at a file on disk.
    fn saveable_dirty_tabs(&self) -> Vec<(EntityId, PathBuf)> {
        self.tabs
            .iter()
            .filter(|tab| tab.document.dirty)
            .filter_map(|tab| {
                tab.document
                    .path
                    .clone()
                    .map(|path| (tab.document.editor_id(), path))
            })
            .filter(|(_, path)| path.exists())
            .collect()
    }

    pub(super) fn save_editor(
        &mut self,
        id: EntityId,
        path: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.save_editor_then(id, path, None, window, cx);
    }

    /// [`Self::save_editor`], reporting how the save ended to `done`.
    fn save_editor_then(
        &mut self,
        id: EntityId,
        path: PathBuf,
        done: Option<SaveDone>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // A raster tab's editor is empty; writing it would wipe the image.
        let Some(tab) = self.tabs.find(id).filter(|tab| !tab.raster) else {
            report(done.as_ref(), true, self, window, cx);
            return;
        };

        // Writing back a file that was not valid UTF-8 replaces its bad bytes
        // for good, so ask first. Saving a copy elsewhere loses nothing.
        if !tab.document.would_lose_bytes_to(&path) {
            self.save_over_changes(id, path, done, window, cx);
            return;
        }

        let name = file_name(&path);
        let entity = cx.entity();
        window.open_alert_dialog(cx, move |alert, _window, _cx| {
            let entity = entity.clone();
            let cancel_entity = entity.clone();
            let path = path.clone();
            let done = done.clone();
            let cancel_done = done.clone();
            alert
                .confirm()
                .title("Not Valid UTF-8")
                .description(format!(
                    "\"{name}\" has bytes that are not valid UTF-8. Saving replaces them with �. Save anyway?"
                ))
                .ok_text("Save")
                .ok_variant(ButtonVariant::Danger)
                .cancel_text("Cancel")
                .on_ok(move |_, window, cx| {
                    entity.update(cx, |this, cx| {
                        this.save_over_changes(id, path.clone(), done.clone(), window, cx)
                    });
                    true
                })
                .on_cancel(move |_, window, cx| {
                    cancel_entity.update(cx, |this, cx| {
                        report(cancel_done.as_ref(), false, this, window, cx)
                    });
                    true
                })
        });
    }

    /// Writes the tab, first asking when its file changed on disk.
    fn save_over_changes(
        &mut self,
        id: EntityId,
        path: PathBuf,
        done: Option<SaveDone>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(tab) = self.tabs.find(id) else {
            report(done.as_ref(), true, self, window, cx);
            return;
        };

        // Saving over the tab's own file after another program changed it
        // would silently discard that change, and a file deleted on disk would
        // be silently recreated, so ask first. A change the user chose to keep
        // their edits over still asks: that choice kept the buffer, it did not
        // agree to overwrite the file.
        let current = modified_time(&path);
        let changed_on_disk = tab.document.path() == Some(path.as_path())
            && disk_moved_on(tab.document.disk_modified, current);
        if !changed_on_disk {
            self.write_editor(id, path, done, window, cx);
            return;
        }

        let name = file_name(&path);
        let deleted = current.is_none();
        let entity = cx.entity();
        window.open_alert_dialog(cx, move |alert, _window, _cx| {
            let entity = entity.clone();
            let cancel_entity = entity.clone();
            let path = path.clone();
            let done = done.clone();
            let cancel_done = done.clone();
            let (title, description, ok_text) = if deleted {
                (
                    "File Deleted",
                    format!("\"{name}\" was deleted on disk. Write it again?"),
                    "Write Again",
                )
            } else {
                (
                    "File Changed",
                    format!("\"{name}\" changed on disk since it was opened. Overwrite it?"),
                    "Overwrite",
                )
            };
            alert
                .confirm()
                .title(title)
                .description(description)
                .ok_text(ok_text)
                .ok_variant(ButtonVariant::Danger)
                .cancel_text("Cancel")
                .on_ok(move |_, window, cx| {
                    entity.update(cx, |this, cx| {
                        this.write_editor(id, path.clone(), done.clone(), window, cx)
                    });
                    true
                })
                .on_cancel(move |_, window, cx| {
                    cancel_entity.update(cx, |this, cx| {
                        report(cancel_done.as_ref(), false, this, window, cx)
                    });
                    true
                })
        });
    }

    /// Writes the tab's text to `path` without any checks.
    fn write_editor(
        &mut self,
        id: EntityId,
        path: PathBuf,
        done: Option<SaveDone>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(editor) = self.tabs.find(id).map(|tab| tab.document.editor.clone()) else {
            report(done.as_ref(), true, self, window, cx);
            return;
        };
        self.tidy_for_save(&editor, window, cx);
        let Some(tab) = self.tabs.find(id) else {
            report(done.as_ref(), true, self, window, cx);
            return;
        };
        // Kept to tell, once the write lands, whether typing changed the
        // buffer meanwhile. Cloning the rope shares its nodes.
        let written = tab.document.editor.read(cx).text().clone();
        let text = tab.document.format.encode(&written.to_string());
        let filename = file_name(&path);

        cx.spawn_in(window, async move |this, cx| {
            let write_path = path.clone();
            let result = cx
                .background_executor()
                .spawn(async move { std::fs::write(write_path, text) })
                .await;

            this.update_in(cx, |this, window, cx| {
                let saved = match result {
                    Ok(()) => {
                        let unchanged = this
                            .tabs
                            .find(id)
                            .is_some_and(|tab| *tab.document.editor.read(cx).text() == written);
                        this.finish_save(id, path.clone(), unchanged, cx);
                        this.reload_settings_if(&path, window, cx);
                        true
                    }
                    Err(error) => {
                        window.push_notification(
                            Notification::error(format!("Could not save {filename}: {error}")),
                            cx,
                        );
                        false
                    }
                };
                report(done.as_ref(), saved, this, window, cx);
            })
            .ok();
        })
        .detach();
    }

    /// Records a landed write. `unchanged` is false when the buffer was edited
    /// while the write ran, so the tab stays dirty with the newer text.
    pub(super) fn finish_save(
        &mut self,
        id: EntityId,
        path: PathBuf,
        unchanged: bool,
        cx: &mut Context<Self>,
    ) {
        if let Some(tab) = self
            .tabs
            .iter_mut()
            .find(|tab| tab.document.editor_id() == id)
        {
            let before = tab.document.language.clone();
            tab.document
                .moved_to(path.clone(), &first_line(&tab.document.editor, cx));
            if tab.document.language != before {
                let language = tab.document.language.clone();
                tab.document.editor.update(cx, |state, cx| {
                    state.set_highlighter(editor_language_id(&language), cx);
                });
            }
            // The bad bytes were replaced on disk, so the file is clean UTF-8.
            let format = TextFormat {
                lossy: false,
                ..tab.document.format
            };
            tab.document.synced_with_disk(modified_time(&path), format);
            tab.document.dirty = !unchanged;
        }
        self.note_recent(path, cx);
        self.reload_tree(cx);
        self.save_session(cx);
        cx.notify();
    }

    /// Follows a file changed by another program. A tab without unsaved edits
    /// reloads silently; one with edits asks before discarding them. Only the
    /// visible tab interrupts — the rest ask when they become active. A file
    /// that vanished leaves the buffer alone and flags the tab instead.
    pub(super) fn handle_disk_change(
        &mut self,
        path: &Path,
        change: Change,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(tab) = self
            .tabs
            .iter()
            .find(|tab| tab.document.path.as_deref() == Some(path))
        else {
            return;
        };
        if tab.raster {
            return;
        }
        let id = tab.document.editor_id();
        let dirty = tab.document.dirty;
        let disk_modified = tab.document.disk_modified;
        let dismissed = tab.document.dismissed_disk_change;
        // A removal flags the tab: there is nothing to reload, and the buffer
        // keeps its text so the user can write the file back deliberately.
        let modified = if change == Change::Changed {
            modified_time(path)
        } else {
            None
        };
        let Some(modified) = modified else {
            self.set_deleted_on_disk(id, true, cx);
            return;
        };
        // Back on disk after a removal we missed.
        self.set_deleted_on_disk(id, false, cx);

        let decision = classify_disk_change(dirty, disk_modified, dismissed, modified);
        let active = self.tabs.active().map(|tab| tab.document.editor_id()) == Some(id);

        match decision {
            DiskChange::Ignored => {}
            DiskChange::Reload => self.reload_from_disk(id, path.to_path_buf(), window, cx),
            DiskChange::Ask if active => {
                self.prompt_disk_conflict(id, path.to_path_buf(), window, cx);
            }
            DiskChange::Ask => {}
        }
    }

    /// Flags the tab for a file that is gone from disk, or clears the flag when
    /// it is back. Repaints only on a real change.
    fn set_deleted_on_disk(&mut self, id: EntityId, deleted: bool, cx: &mut Context<Self>) {
        if let Some(tab) = self
            .tabs
            .iter_mut()
            .find(|tab| tab.document.editor_id() == id)
            && tab.document.deleted_on_disk != deleted
        {
            tab.document.deleted_on_disk = deleted;
            cx.notify();
        }
    }

    /// Picks up edits another program made to the active tab's file, and asks
    /// when those edits would overwrite unsaved work. Only that tab reloads or
    /// asks now; a background tab's question waits until it is focused. Every
    /// tab, though, refreshes whether its file still exists, so a deletion
    /// while the app was in the background is marked even on a tab that is not
    /// focused — and even on a file outside the watched folders, which raises
    /// no event at all.
    pub(crate) fn check_outside_changes(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let active = self.active().and_then(|tab| tab.document.path.clone());
        let others: Vec<(EntityId, PathBuf)> = self
            .tabs
            .iter()
            .filter(|tab| !tab.raster)
            .filter_map(|tab| {
                let path = tab.document.path.clone()?;
                (active.as_deref() != Some(path.as_path()))
                    .then(|| (tab.document.editor_id(), path))
            })
            .collect();
        for (id, path) in others {
            self.set_deleted_on_disk(id, modified_time(&path).is_none(), cx);
        }
        if let Some(path) = active {
            self.handle_disk_change(&path, Change::Changed, window, cx);
        }
    }

    /// Asks whether to reload a file that changed under unsaved edits. The
    /// version is recorded as dismissed before the dialog opens, so Escape or
    /// Keep My Edits does not ask again about the same state, while saving over
    /// it still warns: `disk_modified` stays on the version the buffer is
    /// based on.
    fn prompt_disk_conflict(
        &mut self,
        id: EntityId,
        path: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let modified = modified_time(&path);
        if let Some(tab) = self
            .tabs
            .iter_mut()
            .find(|tab| tab.document.editor_id() == id)
        {
            tab.document.dismissed_disk_change = modified;
        }
        let name = file_name(&path);
        let entity = cx.entity();
        window.open_alert_dialog(cx, move |alert, _window, _cx| {
            let entity = entity.clone();
            let path = path.clone();
            alert
                .confirm()
                .title("File Changed on Disk")
                .description(format!(
                    "\"{name}\" changed on disk and has unsaved edits. Reload it and discard them?"
                ))
                .ok_text("Reload from Disk")
                .ok_variant(ButtonVariant::Danger)
                .cancel_text("Keep My Edits")
                .on_ok(move |_, window, cx| {
                    entity.update(cx, |this, cx| {
                        this.reload_from_disk(id, path.clone(), window, cx);
                    });
                    true
                })
        });
    }

    /// Replaces the tab's text with the file's current contents, keeping the
    /// caret on the same line where possible. Undo history is cleared.
    fn reload_from_disk(
        &mut self,
        id: EntityId,
        path: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let started_dirty = self.tabs.find(id).is_some_and(|tab| tab.document.dirty);
        cx.spawn_in(window, async move |this, cx| {
            let read_path = path.clone();
            let read = cx
                .background_executor()
                .spawn(async move {
                    let modified = modified_time(&read_path);
                    let bytes = std::fs::read(&read_path)?;
                    TextFormat::decode(&bytes).map(|(text, format)| (text, format, modified))
                })
                .await;

            this.update_in(cx, |this, window, cx| {
                let (text, format, modified) = match read {
                    Ok(read) => read,
                    Err(error) => {
                        window
                            .push_notification(Notification::error(open_error(&path, &error)), cx);
                        return;
                    }
                };
                let Some(tab) = this
                    .tabs
                    .iter_mut()
                    .find(|tab| tab.document.editor_id() == id)
                else {
                    return;
                };
                // Typing during the read made the tab dirty; leave it alone and
                // let the next activation ask instead.
                if tab.document.path() != Some(path.as_path())
                    || (tab.document.dirty && !started_dirty)
                {
                    return;
                }
                tab.document.synced_with_disk(modified, format);
                let editor = tab.document.editor.clone();
                let is_active = this.tabs.active().map(|tab| tab.document.editor_id()) == Some(id);
                editor.update(cx, |state, cx| {
                    let position = state.cursor_position();
                    // `set_value` emits no change event, so the tab stays clean.
                    state.set_value(text, window, cx);
                    // Restoring the caret also focuses the editor, so only do it
                    // when the visible editor already has focus.
                    if is_active && state.focus_handle(cx).is_focused(window) {
                        state.set_cursor_position(position, window, cx);
                    }
                });
                this.set_dirty(id, false, cx);
                this.sync_preview(&editor, cx);
                this.sync_counts(cx);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Prompts for a single name with a text field and runs `on_confirm` with
    /// the trimmed value when the field is not empty.
    fn prompt_for_name<F>(
        &mut self,
        prompt: NamePrompt,
        window: &mut Window,
        cx: &mut Context<Self>,
        on_confirm: F,
    ) where
        F: Fn(&mut Self, String, &mut Window, &mut Context<Self>) + 'static,
    {
        let NamePrompt {
            title,
            description,
            initial,
            ok_text,
        } = prompt;
        let input = cx.new(|cx| {
            let state = InputState::new(window, cx).placeholder("Name");
            if initial.is_empty() {
                state
            } else {
                state.default_value(initial.clone())
            }
        });
        let value_input = input.clone();
        let focus_input = input.clone();
        let entity = cx.entity();
        let on_confirm = std::rc::Rc::new(on_confirm);

        window.open_alert_dialog(cx, move |alert, _window, _cx| {
            let value_input = value_input.clone();
            let entity = entity.clone();
            let on_confirm = on_confirm.clone();
            alert
                .confirm()
                .title(title)
                .description(description.clone())
                .child(div().w_full().child(Input::new(&input).w_full()))
                .ok_text(ok_text)
                .cancel_text("Cancel")
                .on_ok(move |_, window, cx| {
                    let name = value_input.read(cx).value().trim().to_string();
                    if name.is_empty() {
                        return false;
                    }
                    entity.update(cx, |this, cx| on_confirm(this, name, window, cx));
                    true
                })
        });

        window.defer(cx, move |window, cx| {
            focus_input.update(cx, |state, cx| state.focus(window, cx));
        });
    }

    pub(super) fn create_entry(
        &mut self,
        parent: PathBuf,
        is_dir: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (title, ok_text) = if is_dir {
            ("New Folder", "Create Folder")
        } else {
            ("New File", "Create File")
        };
        let description = format!("Create in {}", file_name(&parent));

        self.prompt_for_name(
            NamePrompt {
                title,
                description,
                initial: String::new(),
                ok_text,
            },
            window,
            cx,
            move |this, name, window, cx| {
                if !is_valid_entry_name(&name) {
                    window.push_notification(
                        Notification::error(format!("Invalid name \"{name}\"")),
                        cx,
                    );
                    return;
                }
                let path = parent.join(&name);
                if path.exists() {
                    window.push_notification(
                        Notification::error(format!("\"{name}\" already exists")),
                        cx,
                    );
                    return;
                }

                let result = if is_dir {
                    std::fs::create_dir(&path)
                } else {
                    std::fs::write(&path, "")
                };
                if let Err(error) = result {
                    window.push_notification(
                        Notification::error(format!("Could not create {name}: {error}")),
                        cx,
                    );
                    return;
                }

                this.workspace.expand(parent.clone());
                if is_dir {
                    this.workspace.expand(path.clone());
                }
                this.workspace.set_selected(Some(path.clone()));
                this.reload_tree(cx);
                this.save_session(cx);
                if is_dir {
                    cx.notify();
                } else {
                    this.request_open(path, window, cx);
                }
            },
        );
    }

    pub(super) fn rename_entry(
        &mut self,
        path: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let name = file_name(&path);
        let description = format!("Rename \"{name}\"");

        self.prompt_for_name(
            NamePrompt {
                title: "Rename",
                description,
                initial: name,
                ok_text: "Rename",
            },
            window,
            cx,
            move |this, new_name, window, cx| this.rename_to(&path, new_name, window, cx),
        );
    }

    /// Renames `path` to `new_name` within its folder.
    fn rename_to(
        &mut self,
        path: &Path,
        new_name: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let parent = path.parent().map(Path::to_path_buf).unwrap_or_default();
        if !is_valid_entry_name(&new_name) {
            window.push_notification(
                Notification::error(format!("Invalid name \"{new_name}\"")),
                cx,
            );
            return;
        }
        let new_path = parent.join(&new_name);
        if new_path == path {
            return;
        }
        // A case-only rename names the same file on a case-insensitive
        // volume, so it is not taken. Hard links to one file share an inode
        // too, which is why the names must also differ only by case.
        let case_only = new_name.to_lowercase() == file_name(path).to_lowercase();
        if new_path.exists() && !(case_only && same_file(path, &new_path)) {
            window.push_notification(
                Notification::error(format!("\"{new_name}\" already exists")),
                cx,
            );
            return;
        }

        match std::fs::rename(path, &new_path) {
            Ok(()) => {
                self.after_rename(path, &new_path, cx);
                window
                    .push_notification(Notification::success(format!("Renamed to {new_name}")), cx);
            }
            Err(error) => window.push_notification(
                Notification::error(format!("Could not rename: {error}")),
                cx,
            ),
        }
    }

    /// Prompts for the destination path, pre-filled with a non-colliding
    /// sibling name, and copies `path` there.
    pub(super) fn duplicate_entry(
        &mut self,
        path: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(parent) = path.parent() else {
            return;
        };
        let initial = parent.join(duplicate_name(&path)).display().to_string();
        let description = format!("Duplicate \"{}\"", file_name(&path));

        self.prompt_for_name(
            NamePrompt {
                title: "Duplicate",
                description,
                initial,
                ok_text: "Duplicate",
            },
            window,
            cx,
            move |this, value, window, cx| this.duplicate_to(&path, value, window, cx),
        );
    }

    /// Copies `path` to `value`, a path typed in the Duplicate prompt that is
    /// resolved against `path`'s folder when relative.
    fn duplicate_to(
        &mut self,
        path: &Path,
        value: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(parent) = path.parent() else {
            return;
        };
        let typed = PathBuf::from(value);
        let destination = if typed.is_absolute() {
            typed
        } else {
            parent.join(typed)
        };

        if destination == path {
            return;
        }
        if destination.exists() {
            window.push_notification(
                Notification::error(format!("\"{}\" already exists", file_name(&destination))),
                cx,
            );
            return;
        }
        // Copying a folder into itself would copy the copy as it grows.
        if path.is_dir() && destination.starts_with(path) {
            window.push_notification(
                Notification::error("Cannot duplicate a folder into itself".to_string()),
                cx,
            );
            return;
        }
        if let Some(parent) = destination.parent()
            && !parent.is_dir()
            && let Err(error) = std::fs::create_dir_all(parent)
        {
            window.push_notification(
                Notification::error(format!("Could not create {}: {error}", parent.display())),
                cx,
            );
            return;
        }

        match copy_entry(path, &destination) {
            Ok(()) => {
                if let Some(parent) = destination.parent() {
                    self.workspace.expand(parent.to_path_buf());
                }
                let name = file_name(&destination);
                self.workspace.set_selected(Some(destination));
                self.reload_tree(cx);
                self.save_session(cx);
                cx.notify();
                window
                    .push_notification(Notification::success(format!("Duplicated as {name}")), cx);
            }
            Err(error) => window.push_notification(
                Notification::error(format!("Could not duplicate {}: {error}", file_name(path))),
                cx,
            ),
        }
    }

    /// Prompts for the destination path, pre-filled with the current path, and
    /// moves `path` there.
    pub(super) fn move_entry(
        &mut self,
        path: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let initial = path.display().to_string();
        let description = format!("Move \"{}\"", file_name(&path));

        self.prompt_for_name(
            NamePrompt {
                title: "Move",
                description,
                initial,
                ok_text: "Move",
            },
            window,
            cx,
            move |this, value, window, cx| {
                let Some(parent) = path.parent() else {
                    return;
                };
                let typed = PathBuf::from(value);
                let destination = if typed.is_absolute() {
                    typed
                } else {
                    parent.join(typed)
                };

                if destination == path {
                    return;
                }
                if destination.exists() {
                    window.push_notification(
                        Notification::error(format!(
                            "\"{}\" already exists",
                            file_name(&destination)
                        )),
                        cx,
                    );
                    return;
                }
                if path.is_dir() && destination.starts_with(&path) {
                    window.push_notification(
                        Notification::error("Cannot move a folder into itself".to_string()),
                        cx,
                    );
                    return;
                }
                if let Some(parent) = destination.parent()
                    && !parent.is_dir()
                    && let Err(error) = std::fs::create_dir_all(parent)
                {
                    window.push_notification(
                        Notification::error(format!(
                            "Could not create {}: {error}",
                            parent.display()
                        )),
                        cx,
                    );
                    return;
                }

                match std::fs::rename(&path, &destination) {
                    Ok(()) => {
                        if let Some(parent) = destination.parent() {
                            this.workspace.expand(parent.to_path_buf());
                        }
                        let name = file_name(&destination);
                        this.after_rename(&path, &destination, cx);
                        window
                            .push_notification(Notification::success(format!("Moved {name}")), cx);
                    }
                    Err(error) => window.push_notification(
                        Notification::error(format!(
                            "Could not move {}: {error}",
                            file_name(&path)
                        )),
                        cx,
                    ),
                }
            },
        );
    }

    /// Moves every open tab, expansion entry, and workspace root from the
    /// old path prefix to the new one after a rename or move.
    fn after_rename(&mut self, old: &Path, new: &Path, cx: &mut Context<Self>) {
        for tab in self.tabs.iter_mut() {
            let Some(current) = tab.document.path.clone() else {
                continue;
            };
            let Some(mapped) = remap_prefix(&current, old, new) else {
                continue;
            };
            tab.document.title = file_name(&mapped).into();
            if !tab.document.language_override {
                let language = language_for(&mapped, &first_line(&tab.document.editor, cx));
                tab.document.editor.update(cx, |state, cx| {
                    state.set_highlighter(editor_language_id(&language), cx);
                });
                tab.document.language = language;
            }
            tab.document.path = Some(mapped);
        }

        self.workspace.remap(old, new);

        self.reload_tree(cx);
        self.save_session(cx);
        cx.notify();
    }

    pub(super) fn delete_entry(
        &mut self,
        path: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let name = file_name(&path);
        let entity = cx.entity();

        window.open_alert_dialog(cx, move |alert, _window, _cx| {
            let entity = entity.clone();
            let path = path.clone();
            let name = name.clone();
            alert
                .confirm()
                .title("Delete")
                .description(format!("Move \"{name}\" to the Trash?"))
                .ok_text("Move to Trash")
                .ok_variant(ButtonVariant::Danger)
                .cancel_text("Cancel")
                .on_ok(move |_, window, cx| {
                    entity.update(cx, |this, cx| this.remove_entry(&path, window, cx));
                    true
                })
        });
    }

    fn remove_entry(&mut self, path: &Path, window: &mut Window, cx: &mut Context<Self>) {
        let ids: Vec<EntityId> = self
            .tabs
            .iter()
            .filter(|tab| {
                tab.document
                    .path
                    .as_deref()
                    .is_some_and(|candidate| candidate == path || candidate.starts_with(path))
            })
            .map(|tab| tab.document.editor_id())
            .collect();
        for id in ids {
            self.remove_tab(id, window, cx);
        }

        match move_to_trash(path) {
            Ok(()) => {
                let name = file_name(path);
                self.workspace.forget(path);
                self.reload_tree(cx);
                self.save_session(cx);
                cx.notify();
                window
                    .push_notification(Notification::success(format!("Moved {name} to Trash")), cx);
            }
            Err(error) => window.push_notification(
                Notification::error(format!("Could not delete {}: {error}", file_name(path))),
                cx,
            ),
        }
    }

    pub(super) fn set_file_clipboard(&mut self, path: PathBuf, cut: bool, cx: &mut Context<Self>) {
        self.workspace
            .set_clipboard(Some(FileClipboard { path, cut }));
        cx.notify();
    }

    pub(super) fn paste_clipboard(
        &mut self,
        target_dir: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(clipboard) = self.workspace.clipboard().cloned() else {
            return;
        };
        let source = clipboard.path.clone();
        let destination = target_dir.join(file_name(&source));

        if source == destination {
            return;
        }
        if destination.exists() {
            window.push_notification(
                Notification::error(format!("\"{}\" already exists", file_name(&destination))),
                cx,
            );
            return;
        }
        if source.is_dir() && target_dir.starts_with(&source) {
            window.push_notification(
                Notification::error("Cannot paste a folder into itself".to_string()),
                cx,
            );
            return;
        }

        let result = if clipboard.cut {
            std::fs::rename(&source, &destination)
        } else {
            copy_entry(&source, &destination)
        };

        match result {
            Ok(()) => {
                let name = file_name(&destination);
                if clipboard.cut {
                    self.workspace.set_clipboard(None);
                    self.after_rename(&source, &destination, cx);
                    window.push_notification(Notification::success(format!("Moved {name}")), cx);
                } else {
                    self.workspace.expand(target_dir);
                    self.workspace.set_selected(Some(destination.clone()));
                    self.reload_tree(cx);
                    self.save_session(cx);
                    cx.notify();
                    window.push_notification(Notification::success(format!("Pasted {name}")), cx);
                }
            }
            Err(error) => window
                .push_notification(Notification::error(format!("Could not paste: {error}")), cx),
        }
    }
}

/// Whether writing the buffer would clobber a change another program made.
/// `disk_modified` is the version the buffer is based on, `current` what the
/// disk holds now. A file that is gone counts: writing would bring it back
/// silently.
pub(super) fn disk_moved_on(
    disk_modified: Option<SystemTime>,
    current: Option<SystemTime>,
) -> bool {
    disk_modified.is_some() && current != disk_modified
}

/// What a changed file means for a tab.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DiskChange {
    /// Nothing to do: the buffer is based on that version, or the user already
    /// chose to keep the buffer over it.
    Ignored,
    /// The tab has no unsaved edits, so the new contents can load silently.
    Reload,
    /// Loading would throw away unsaved edits, so ask first.
    Ask,
}

/// `disk_modified` is the version the buffer is based on, `dismissed` a later
/// version the user chose to keep the buffer over. A dismissal only silences a
/// conflict: a clean tab always follows the disk.
fn classify_disk_change(
    dirty: bool,
    disk_modified: Option<SystemTime>,
    dismissed: Option<SystemTime>,
    modified: SystemTime,
) -> DiskChange {
    if disk_modified == Some(modified) {
        return DiskChange::Ignored;
    }
    if !dirty {
        return DiskChange::Reload;
    }
    if dismissed == Some(modified) {
        return DiskChange::Ignored;
    }
    DiskChange::Ask
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};
    use std::time::SystemTime;

    use gpui_kit::test::TestWindowExt as _;
    use gpui_kit::{AppContext as _, ImageFormat, TestAppContext};

    use super::{DiskChange, TextFormat, classify_disk_change, disk_moved_on, raster_format};
    use crate::app::TeakApp;
    use crate::app::tabs::TabSet;
    use crate::app::test_support::open_app;
    use crate::watcher::Change;

    /// A raster file opens as a view-only tab with its preview already built,
    /// so typing and saving never touch the bytes on disk.
    #[gpui_kit::test]
    async fn a_png_opens_as_a_view_only_image_tab(cx: &mut TestAppContext) {
        let path = std::env::temp_dir().join(format!("teak-image-{}.png", std::process::id()));
        std::fs::write(&path, super::ONE_PIXEL_PNG).unwrap();

        let (handle, app) = open_app(cx);
        cx.update_window(handle, |_, window, cx| {
            app.update(cx, |this, cx| {
                let content = super::read_file(&path).unwrap();
                this.add_file_tab(path.clone(), content, false, window, cx);
            });
            window.render_frame(cx);
            window.render_frame(cx);
        })
        .unwrap();

        cx.update(|cx| {
            let app = app.read(cx);
            let tab = app.tabs.active().expect("the image tab opens");
            assert!(tab.raster, "it is view-only");
            assert!(tab.previewing().is_some(), "the image is what is shown");
            assert!(!tab.document.dirty, "opening does not edit it");
            assert!(tab.image.fit, "a freshly opened image is fitted");
        });

        let _ = std::fs::remove_file(&path);
    }

    /// One tab is left in the background so the dirty case never opens a
    /// dialog: only the visible tab asks.
    #[gpui_kit::test]
    async fn an_external_change_reloads_a_clean_tab_and_spares_an_edited_one(
        cx: &mut TestAppContext,
    ) {
        let edited = temp_path("a");
        let other = temp_path("b");
        std::fs::write(&edited, "one\n").unwrap();
        std::fs::write(&other, "other\n").unwrap();

        let (handle, app) = open_app(cx);
        for path in [&edited, &other] {
            cx.update_window(handle, |_, window, cx| {
                app.update(cx, |this, cx| {
                    let content = super::read_file(path).unwrap();
                    this.add_file_tab(path.clone(), content, false, window, cx);
                });
            })
            .unwrap();
        }

        write_and_handle(cx, handle, &app, &edited, "two\n");
        assert_eq!(tab_text(&app, cx, &edited).as_deref(), Some("two\n"));

        cx.update_window(handle, |_, _, cx| {
            app.update(cx, |this, cx| {
                let id = tab_id(&this.tabs, &edited);
                this.set_dirty(id, true, cx);
            });
        })
        .unwrap();
        write_and_handle(cx, handle, &app, &edited, "three\n");
        assert_eq!(
            tab_text(&app, cx, &edited).as_deref(),
            Some("two\n"),
            "an edited buffer is never replaced silently"
        );
        assert!(cx.update(|cx| {
            let app = app.read(cx);
            app.tabs
                .iter()
                .find(|tab| tab.document.path.as_deref() == Some(edited.as_path()))
                .unwrap()
                .document
                .dirty
        }));
        assert_eq!(tab_text(&app, cx, &other).as_deref(), Some("other\n"));

        // A file deleted on disk flags its tab instead of vanishing silently.
        std::fs::remove_file(&other).unwrap();
        handle_change(cx, handle, &app, &other, Change::Removed);
        assert_eq!(tab_text(&app, cx, &other).as_deref(), Some("other\n"));
        assert!(deleted_on_disk(&app, cx, &other));

        // Brought back: the flag clears and the clean tab reloads it.
        write_and_handle(cx, handle, &app, &other, "back\n");
        assert_eq!(tab_text(&app, cx, &other).as_deref(), Some("back\n"));
        assert!(!deleted_on_disk(&app, cx, &other));

        let _ = std::fs::remove_file(&edited);
        let _ = std::fs::remove_file(&other);
    }

    /// A file outside the watched folders raises no event, so an activation has
    /// to look at every tab: a background one deleted while the app was away
    /// must be marked without being activated. The active tab is still the only
    /// one that reloads.
    #[gpui_kit::test]
    async fn an_activation_flags_a_background_tab_whose_file_vanished(cx: &mut TestAppContext) {
        let background = temp_path("background");
        let active = temp_path("active");
        std::fs::write(&background, "gone soon\n").unwrap();
        std::fs::write(&active, "stays\n").unwrap();

        let (handle, app) = open_app(cx);
        for path in [&background, &active] {
            cx.update_window(handle, |_, window, cx| {
                app.update(cx, |this, cx| {
                    let content = super::read_file(path).unwrap();
                    this.add_file_tab(path.clone(), content, false, window, cx);
                });
            })
            .unwrap();
        }

        std::fs::remove_file(&background).unwrap();
        cx.update_window(handle, |_, window, cx| {
            app.update(cx, |this, cx| this.check_outside_changes(window, cx));
        })
        .unwrap();
        cx.run_until_parked();

        assert!(deleted_on_disk(&app, cx, &background));
        assert!(!deleted_on_disk(&app, cx, &active));
        assert_eq!(
            tab_text(&app, cx, &background).as_deref(),
            Some("gone soon\n"),
            "a deleted tab keeps its text"
        );

        // Brought back, the mark clears on the next check as well.
        std::fs::write(&background, "back\n").unwrap();
        cx.update_window(handle, |_, window, cx| {
            app.update(cx, |this, cx| this.check_outside_changes(window, cx));
        })
        .unwrap();
        cx.run_until_parked();
        assert!(!deleted_on_disk(&app, cx, &background));

        let _ = std::fs::remove_file(&background);
        let _ = std::fs::remove_file(&active);
    }

    /// A batch open keeps the argument order and leaves the last path active,
    /// even though the reads finish independently. Re-listing open files only
    /// activates them and never adds duplicates.
    #[gpui_kit::test]
    async fn batch_open_keeps_argument_order_and_activates_the_last_path(cx: &mut TestAppContext) {
        let dir = std::env::temp_dir().join(format!("app-open-order-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let names = ["z-first.txt", "m-second.txt", "a-third.txt"];
        let paths: Vec<PathBuf> = names
            .iter()
            .map(|name| {
                let path = dir.join(name);
                std::fs::write(&path, name).unwrap();
                path
            })
            .collect();

        let (handle, app) = open_app(cx);
        cx.update_window(handle, |_, window, cx| {
            app.update(cx, |this, cx| {
                this.request_open_many(paths.clone(), window, cx)
            });
        })
        .unwrap();
        cx.run_until_parked();

        let (titles, active) = cx.update(|cx| {
            let app = app.read(cx);
            let titles: Vec<String> = app
                .tabs
                .iter()
                .map(|tab| tab.document.title.to_string())
                .collect();
            (titles, app.tabs.active_index())
        });
        assert_eq!(titles, names);
        assert_eq!(active, Some(names.len() - 1));

        // Asking for two files that are already open changes neither the tab
        // list nor the order; it only activates the last one given.
        cx.update_window(handle, |_, window, cx| {
            app.update(cx, |this, cx| {
                this.request_open_many(vec![paths[2].clone(), paths[1].clone()], window, cx)
            });
        })
        .unwrap();
        let (titles, active) = cx.update(|cx| {
            let app = app.read(cx);
            let titles: Vec<String> = app
                .tabs
                .iter()
                .map(|tab| tab.document.title.to_string())
                .collect();
            (titles, app.tabs.active_index())
        });
        assert_eq!(titles, names);
        assert_eq!(active, Some(1));

        std::fs::remove_dir_all(dir).unwrap();
    }

    /// Adding a folder that already lives under an open root reveals it there
    /// instead of adding a second, overlapping root or reporting an error.
    #[gpui_kit::test]
    async fn adding_a_nested_folder_reveals_it_in_the_open_root(cx: &mut TestAppContext) {
        let root = std::env::temp_dir().join(format!("app-add-nested-{}", std::process::id()));
        let nested = root.join("one").join("two");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&nested).unwrap();

        let (handle, app) = open_app(cx);
        for path in [root.clone(), nested.clone()] {
            cx.update_window(handle, |_, window, cx| {
                app.update(cx, |this, cx| this.add_root(path.clone(), window, cx));
            })
            .unwrap();
        }

        let (roots, selected, expanded) = cx.update(|cx| {
            let app = app.read(cx);
            (
                app.workspace.roots().to_vec(),
                app.workspace.selected().map(Path::to_path_buf),
                app.workspace.is_expanded(&root.join("one")),
            )
        });
        assert_eq!(roots, vec![root.clone()], "the nested folder is not a root");
        assert_eq!(
            selected.as_deref(),
            Some(nested.as_path()),
            "the nested folder is selected in the existing tree"
        );
        assert!(expanded, "its parent is expanded so the row is visible");

        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn writing_is_unsafe_only_when_the_disk_moved_from_the_buffer_base() {
        let base = SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(10);
        let newer = SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(20);

        assert!(!disk_moved_on(None, None), "a brand-new file");
        assert!(!disk_moved_on(Some(base), Some(base)), "our own write");
        assert!(disk_moved_on(Some(base), Some(newer)), "someone else wrote");
        assert!(disk_moved_on(Some(base), None), "deleted on disk");
    }

    #[test]
    fn an_external_change_asks_only_when_the_buffer_has_edits() {
        let old = SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(5);
        let new = SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(10);
        let newer = SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(20);

        // Our own write: nothing to do.
        assert_eq!(
            classify_disk_change(true, Some(new), None, new),
            DiskChange::Ignored
        );
        // A clean buffer follows the disk.
        assert_eq!(
            classify_disk_change(false, Some(old), None, new),
            DiskChange::Reload
        );
        // Edits are never discarded silently.
        assert_eq!(
            classify_disk_change(true, Some(old), None, new),
            DiskChange::Ask
        );
        // A version the user already kept their buffer over is not asked again.
        assert_eq!(
            classify_disk_change(true, Some(old), Some(new), new),
            DiskChange::Ignored
        );
        assert_eq!(
            classify_disk_change(true, Some(old), Some(new), newer),
            DiskChange::Ask
        );
        // A clean buffer follows the disk even after such a dismissal.
        assert_eq!(
            classify_disk_change(false, Some(old), Some(new), new),
            DiskChange::Reload
        );
    }

    /// Opens `path` in a tab and replaces its buffer with `text`, leaving the
    /// tab dirty, as typing would.
    fn open_edited(
        cx: &mut TestAppContext,
        handle: gpui_kit::AnyWindowHandle,
        app: &gpui_kit::Entity<TeakApp>,
        path: &Path,
        text: &str,
    ) {
        cx.update_window(handle, |_, window, cx| {
            app.update(cx, |this, cx| {
                let content = super::read_file(path).unwrap();
                this.add_file_tab(path.to_path_buf(), content, false, window, cx);
                let id = tab_id(&this.tabs, path);
                let editor = this.tabs.find(id).unwrap().document.editor.clone();
                editor.update(cx, |state, cx| {
                    state.set_value(text.to_string(), window, cx)
                });
                this.set_dirty(id, true, cx);
            });
        })
        .unwrap();
    }

    fn save_all_then_close_workspace(
        cx: &mut TestAppContext,
        handle: gpui_kit::AnyWindowHandle,
        app: &gpui_kit::Entity<TeakApp>,
    ) {
        cx.update_window(handle, |_, window, cx| {
            app.update(cx, |this, cx| {
                this.save_all_then(TeakApp::close_workspace, None, window, cx)
            });
        })
        .unwrap();
        cx.run_until_parked();
    }

    fn tab_count(app: &gpui_kit::Entity<TeakApp>, cx: &mut TestAppContext) -> usize {
        cx.update(|cx| app.read(cx).tabs.iter().count())
    }

    /// Save All and Continue waits for the writes, then runs its action.
    #[gpui_kit::test]
    async fn save_all_and_continue_runs_once_the_writes_land(cx: &mut TestAppContext) {
        let path = temp_path("save-all-continue");
        std::fs::write(&path, "before\n").unwrap();
        let (handle, app) = open_app(cx);
        open_edited(cx, handle, &app, &path, "after\n");

        save_all_then_close_workspace(cx, handle, &app);

        assert_eq!(std::fs::read_to_string(&path).unwrap(), "after\n");
        assert_eq!(tab_count(&app, cx), 0, "the guarded action ran");
        let _ = std::fs::remove_file(&path);
    }

    /// A write that fails stops the action, so the unsaved tab stays open.
    #[gpui_kit::test]
    async fn save_all_and_continue_stops_when_a_write_fails(cx: &mut TestAppContext) {
        use std::os::unix::fs::PermissionsExt as _;

        let path = temp_path("save-all-fails");
        std::fs::write(&path, "before\n").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o444)).unwrap();
        let (handle, app) = open_app(cx);
        open_edited(cx, handle, &app, &path, "after\n");

        save_all_then_close_workspace(cx, handle, &app);

        assert_eq!(std::fs::read_to_string(&path).unwrap(), "before\n");
        assert_eq!(tab_count(&app, cx), 1, "the action did not run");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        let _ = std::fs::remove_file(&path);
    }

    /// Keeping the buffer over an outside change does not agree to overwrite
    /// it: the save still asks, so nothing is written until the user confirms.
    #[gpui_kit::test]
    async fn saving_over_a_dismissed_change_still_asks(cx: &mut TestAppContext) {
        let path = temp_path("dismissed-save");
        std::fs::write(&path, "original\n").unwrap();
        let (handle, app) = open_app(cx);
        open_edited(cx, handle, &app, &path, "mine\n");

        std::fs::write(&path, "outside\n").unwrap();
        let later = SystemTime::now() + std::time::Duration::from_secs(60);
        std::fs::File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_modified(later)
            .unwrap();
        cx.update_window(handle, |_, window, cx| {
            app.update(cx, |this, cx| {
                let id = tab_id(&this.tabs, &path);
                let tab = this
                    .tabs
                    .iter_mut()
                    .find(|tab| tab.document.editor_id() == id);
                tab.unwrap().document.dismissed_disk_change = super::modified_time(&path);
                this.save_editor(id, path.clone(), window, cx);
            });
        })
        .unwrap();
        cx.run_until_parked();

        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "outside\n",
            "the outside change is not overwritten without asking"
        );
        let _ = std::fs::remove_file(&path);
    }

    /// Duplicating a folder into its own subtree is refused before anything is
    /// copied, as Move refuses it.
    #[gpui_kit::test]
    async fn duplicating_a_folder_into_itself_is_refused(cx: &mut TestAppContext) {
        let dir = std::env::temp_dir().join(format!("teak-dup-self-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let docs = dir.join("docs");
        std::fs::create_dir_all(&docs).unwrap();
        std::fs::write(docs.join("a.txt"), "a").unwrap();
        let (handle, app) = open_app(cx);

        cx.update_window(handle, |_, window, cx| {
            app.update(cx, |this, cx| {
                this.duplicate_to(&docs, "docs/inner".to_string(), window, cx)
            });
        })
        .unwrap();

        assert!(!docs.join("inner").exists(), "nothing was copied");
        std::fs::remove_dir_all(dir).unwrap();
    }

    fn tab_dirty(app: &gpui_kit::Entity<TeakApp>, cx: &mut TestAppContext, path: &Path) -> bool {
        cx.update(|cx| {
            let app = app.read(cx);
            app.tabs
                .find(tab_id(&app.tabs, path))
                .unwrap()
                .document
                .dirty
        })
    }

    /// Typing while a write is in flight keeps the tab dirty: the disk holds
    /// the older text, so closing must still ask.
    #[gpui_kit::test]
    async fn typing_during_a_save_keeps_the_tab_dirty(cx: &mut TestAppContext) {
        let path = temp_path("dirty-during-write");
        std::fs::write(&path, "before\n").unwrap();
        let (handle, app) = open_app(cx);
        open_edited(cx, handle, &app, &path, "first\n");

        cx.update_window(handle, |_, window, cx| {
            app.update(cx, |this, cx| {
                let id = tab_id(&this.tabs, &path);
                this.save_editor(id, path.clone(), window, cx);
                // The write has not run yet; this edit lands after its snapshot.
                let editor = this.tabs.find(id).unwrap().document.editor.clone();
                editor.update(cx, |state, cx| state.set_value("second\n", window, cx));
            });
        })
        .unwrap();
        cx.run_until_parked();

        assert_eq!(std::fs::read_to_string(&path).unwrap(), "first\n");
        assert!(
            tab_dirty(&app, cx, &path),
            "the newer text is still unsaved"
        );
        let _ = std::fs::remove_file(&path);
    }

    /// A save that lands with the buffer untouched leaves the tab clean.
    #[gpui_kit::test]
    async fn a_save_with_no_edits_meanwhile_cleans_the_tab(cx: &mut TestAppContext) {
        let path = temp_path("clean-after-write");
        std::fs::write(&path, "before\n").unwrap();
        let (handle, app) = open_app(cx);
        open_edited(cx, handle, &app, &path, "after\n");

        cx.update_window(handle, |_, window, cx| {
            app.update(cx, |this, cx| {
                let id = tab_id(&this.tabs, &path);
                this.save_editor(id, path.clone(), window, cx);
            });
        })
        .unwrap();
        cx.run_until_parked();

        assert_eq!(std::fs::read_to_string(&path).unwrap(), "after\n");
        assert!(!tab_dirty(&app, cx, &path));
        let _ = std::fs::remove_file(&path);
    }

    /// The mtime a tab records is the one taken before its bytes were read, so
    /// a write between the read and the tab opening is still noticed.
    #[gpui_kit::test]
    async fn a_write_during_the_open_is_seen_as_a_later_version(cx: &mut TestAppContext) {
        let path = temp_path("mtime-before-read");
        std::fs::write(&path, "read\n").unwrap();
        let (handle, app) = open_app(cx);
        let content = super::read_file(&path).unwrap();

        std::fs::write(&path, "outside\n").unwrap();
        let later = SystemTime::now() + std::time::Duration::from_secs(60);
        std::fs::File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_modified(later)
            .unwrap();
        cx.update_window(handle, |_, window, cx| {
            app.update(cx, |this, cx| {
                this.add_file_tab(path.clone(), content, false, window, cx)
            });
        })
        .unwrap();

        let recorded = cx.update(|cx| {
            let app = app.read(cx);
            app.tabs
                .find(tab_id(&app.tabs, &path))
                .unwrap()
                .document
                .disk_modified
        });
        assert!(disk_moved_on(recorded, super::modified_time(&path)));
        let _ = std::fs::remove_file(&path);
    }

    /// Changing only the case of a name renames the file, and its tab follows.
    #[gpui_kit::test]
    async fn a_case_only_rename_goes_through(cx: &mut TestAppContext) {
        let dir = std::env::temp_dir().join(format!("teak-case-rename-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let lower = dir.join("foo.txt");
        std::fs::write(&lower, "a").unwrap();
        let (handle, app) = open_app(cx);
        open_edited(cx, handle, &app, &lower, "a");

        cx.update_window(handle, |_, window, cx| {
            app.update(cx, |this, cx| {
                this.rename_to(&lower, "Foo.txt".to_string(), window, cx)
            });
        })
        .unwrap();

        let names: Vec<String> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, ["Foo.txt"]);
        let upper = dir.join("Foo.txt");
        assert_eq!(
            tab_text(&app, cx, &upper).as_deref(),
            Some("a"),
            "the tab follows the new name"
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// Renaming onto a different file that already has the name is refused.
    #[gpui_kit::test]
    async fn a_rename_onto_another_file_is_refused(cx: &mut TestAppContext) {
        let dir = std::env::temp_dir().join(format!("teak-rename-taken-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let from = dir.join("a.txt");
        let taken = dir.join("b.txt");
        std::fs::write(&from, "a").unwrap();
        std::fs::write(&taken, "b").unwrap();
        let (handle, app) = open_app(cx);

        cx.update_window(handle, |_, window, cx| {
            app.update(cx, |this, cx| {
                this.rename_to(&from, "b.txt".to_string(), window, cx)
            });
        })
        .unwrap();

        assert_eq!(std::fs::read_to_string(&from).unwrap(), "a");
        assert_eq!(std::fs::read_to_string(&taken).unwrap(), "b");
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// Save As onto a file another tab shows brings that tab forward and
    /// writes nothing, so the two buffers never fight over one file.
    #[gpui_kit::test]
    async fn save_as_onto_an_open_file_activates_its_tab(cx: &mut TestAppContext) {
        let open = temp_path("save-as-open");
        let other = temp_path("save-as-other");
        std::fs::write(&open, "open\n").unwrap();
        std::fs::write(&other, "other\n").unwrap();
        let (handle, app) = open_app(cx);
        open_edited(cx, handle, &app, &open, "open\n");
        open_edited(cx, handle, &app, &other, "changed\n");

        cx.update_window(handle, |_, window, cx| {
            app.update(cx, |this, cx| {
                let id = tab_id(&this.tabs, &other);
                this.save_as(id, open.clone(), window, cx);
            });
        })
        .unwrap();
        cx.run_until_parked();

        assert_eq!(std::fs::read_to_string(&open).unwrap(), "open\n");
        cx.update(|cx| {
            let app = app.read(cx);
            assert_eq!(app.active().unwrap().document.path(), Some(open.as_path()));
            assert!(app.tabs.is_open(&other), "the saved tab keeps its own path");
        });
        let _ = std::fs::remove_file(&open);
        let _ = std::fs::remove_file(&other);
    }

    fn temp_path(name: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!("app-reload-{name}-{}", std::process::id()));
        let _ = std::fs::remove_file(&path);
        path
    }

    fn tab_id(tabs: &TabSet, path: &Path) -> gpui_kit::EntityId {
        tabs.iter()
            .find(|tab| tab.document.path.as_deref() == Some(path))
            .unwrap()
            .document
            .editor
            .entity_id()
    }

    fn write_and_handle(
        cx: &mut TestAppContext,
        handle: gpui_kit::AnyWindowHandle,
        app: &gpui_kit::Entity<TeakApp>,
        path: &Path,
        text: &str,
    ) {
        std::fs::write(path, text).unwrap();
        handle_change(cx, handle, app, path, Change::Changed);
    }

    fn handle_change(
        cx: &mut TestAppContext,
        handle: gpui_kit::AnyWindowHandle,
        app: &gpui_kit::Entity<TeakApp>,
        path: &Path,
        change: Change,
    ) {
        cx.update_window(handle, |_, window, cx| {
            app.update(cx, |this, cx| {
                this.handle_disk_change(path, change, window, cx)
            });
        })
        .unwrap();
        cx.run_until_parked();
    }

    fn deleted_on_disk(
        app: &gpui_kit::Entity<TeakApp>,
        cx: &mut TestAppContext,
        path: &Path,
    ) -> bool {
        cx.update(|cx| {
            app.read(cx)
                .tabs
                .iter()
                .find(|tab| tab.document.path.as_deref() == Some(path))
                .unwrap()
                .document
                .deleted_on_disk
        })
    }

    fn tab_text(
        app: &gpui_kit::Entity<TeakApp>,
        cx: &mut TestAppContext,
        path: &Path,
    ) -> Option<String> {
        cx.update(|cx| {
            let app = app.read(cx);
            let tab = app
                .tabs
                .iter()
                .find(|tab| tab.document.path.as_deref() == Some(path))?;
            Some(tab.document.editor.read(cx).text().to_string())
        })
    }

    #[test]
    fn raster_format_maps_image_extensions() {
        assert_eq!(raster_format(Path::new("a.PNG")), Some(ImageFormat::Png));
        assert_eq!(raster_format(Path::new("a.jpeg")), Some(ImageFormat::Jpeg));
        assert_eq!(raster_format(Path::new("icon.svg")), None);
        assert_eq!(raster_format(Path::new("notes.txt")), None);
    }

    #[test]
    fn unchanged_files_round_trip_byte_for_byte() {
        for bytes in [
            &b"plain\nlf\n"[..],
            b"windows\r\nlines\r\n",
            b"\xEF\xBB\xBFwith bom\r\n",
            b"lf file keeps a stray \r here\n",
            b"",
        ] {
            let (text, format) = TextFormat::decode(bytes).unwrap();
            assert_eq!(format.encode(&text), bytes, "{bytes:?}");
        }
    }

    #[test]
    fn bom_stays_out_of_the_editor_text() {
        let (text, format) = TextFormat::decode(b"\xEF\xBB\xBFhi").unwrap();
        assert_eq!(text, "hi");
        assert!(format.bom);
        assert_eq!(format.encoding_label(), "UTF-8 with BOM");
    }

    #[test]
    fn crlf_file_saves_new_lines_as_crlf() {
        let (_, format) = TextFormat::decode(b"a\r\nb\r\n").unwrap();
        assert!(format.crlf);
        assert_eq!(format.line_ending_label(), "CRLF");
        assert_eq!(format.encode("a\r\nnew\nb\r\n"), b"a\r\nnew\r\nb\r\n");
    }

    #[test]
    fn mixed_file_saves_with_its_first_line_ending() {
        let (text, format) = TextFormat::decode(b"a\r\nb\nc\r\n").unwrap();
        assert_eq!(format.encode(&text), b"a\r\nb\r\nc\r\n");
    }

    #[test]
    fn line_ending_follows_the_first_break() {
        assert!(!TextFormat::decode(b"a\nb\r\n").unwrap().1.crlf);
        assert!(!TextFormat::decode(b"no breaks").unwrap().1.crlf);
    }

    #[test]
    fn invalid_utf8_opens_lossy() {
        let (text, format) = TextFormat::decode(b"caf\xE9").unwrap();
        assert_eq!(text, "caf\u{FFFD}");
        assert!(format.lossy);
    }

    #[test]
    fn binary_content_is_refused() {
        let error = TextFormat::decode(b"\x89PNG\0\0").unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
    }
}
