use std::path::{Path, PathBuf};
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
};
use crate::watcher::Change;
use crate::windows::{WindowSeed, cascaded_frame};
use crate::{
    AddFolder, CloseAllFolders, NewFile, OpenFile, OpenFolderInNewWindow, SaveAll, SaveFile,
    SaveFileAs,
};

use super::TeakApp;
use super::workspace::{AddRoot, FileClipboard};

/// A file read for a new tab: text for the editor, or a raster image to view.
pub(super) enum FileContent {
    Text { text: String, format: TextFormat },
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

/// Reads `path` for a new tab. Raster images load as raw bytes; everything
/// else is decoded as text.
pub(super) fn read_file(path: &Path) -> std::io::Result<FileContent> {
    let bytes = std::fs::read(path)?;
    match raster_format(path) {
        Some(format) => Ok(FileContent::Image(Arc::new(Image::from_bytes(
            format, bytes,
        )))),
        None => {
            let (text, format) = TextFormat::decode(&bytes)?;
            Ok(FileContent::Text { text, format })
        }
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
                                this.add_file_tab(path, content, window, cx);
                            }
                        }
                        Err(error) => window
                            .push_notification(Notification::error(open_error(&path, &error)), cx),
                    }
                }
                this.finish_open_many(&order, window, cx);
            })
            .ok();
        })
        .detach();
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
        // The results tab is generated; there is nothing to save.
        if tab.find_results.is_some() {
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
        if tab.raster || tab.find_results.is_some() {
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
                this.update_in(cx, |this, window, cx| {
                    this.save_editor(id, path, window, cx)
                })
                .ok();
            }
        })
        .detach();
    }

    /// Saves every edited tab that still points at a file on disk. Untitled and
    /// deleted files are left alone, the latter so a save never recreates a file
    /// the user removed.
    pub(super) fn on_save_all(&mut self, _: &SaveAll, window: &mut Window, cx: &mut Context<Self>) {
        let targets: Vec<(EntityId, PathBuf)> = self
            .tabs
            .iter()
            .filter(|tab| tab.document.dirty)
            .filter_map(|tab| {
                tab.document
                    .path
                    .clone()
                    .map(|path| (tab.document.editor_id(), path))
            })
            .filter(|(_, path)| path.exists())
            .collect();

        for (id, path) in targets {
            self.save_editor(id, path, window, cx);
        }
    }

    pub(super) fn save_editor(
        &mut self,
        id: EntityId,
        path: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(tab) = self.tabs.find(id) else {
            return;
        };
        // A raster tab's editor is empty; writing it would wipe the image.
        if tab.raster {
            return;
        }

        // Writing back a file that was not valid UTF-8 replaces its bad bytes
        // for good, so ask first. Saving a copy elsewhere loses nothing.
        if !tab.document.would_lose_bytes_to(&path) {
            self.save_over_changes(id, path, window, cx);
            return;
        }

        let name = file_name(&path);
        let entity = cx.entity();
        window.open_alert_dialog(cx, move |alert, _window, _cx| {
            let entity = entity.clone();
            let path = path.clone();
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
                        this.save_over_changes(id, path.clone(), window, cx)
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
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(tab) = self.tabs.find(id) else {
            return;
        };

        // Saving over the tab's own file after another program changed it
        // would silently discard that change, and a file deleted on disk would
        // be silently recreated, so ask first.
        let current = modified_time(&path);
        let changed_on_disk =
            tab.document.path() == Some(path.as_path()) && tab.document.moved_on_disk(current);
        if !changed_on_disk {
            self.write_editor(id, path, window, cx);
            return;
        }

        let name = file_name(&path);
        let deleted = current.is_none();
        let entity = cx.entity();
        window.open_alert_dialog(cx, move |alert, _window, _cx| {
            let entity = entity.clone();
            let path = path.clone();
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
                        this.write_editor(id, path.clone(), window, cx)
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
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(editor) = self.tabs.find(id).map(|tab| tab.document.editor.clone()) else {
            return;
        };
        self.tidy_for_save(&editor, window, cx);
        let Some(tab) = self.tabs.find(id) else {
            return;
        };
        let text = tab
            .document
            .format
            .encode(&tab.document.editor.read(cx).text().to_string());
        let filename = file_name(&path);

        cx.spawn_in(window, async move |this, cx| {
            let write_path = path.clone();
            let result = cx
                .background_executor()
                .spawn(async move { std::fs::write(write_path, text) })
                .await;

            this.update_in(cx, |this, window, cx| match result {
                Ok(()) => {
                    this.finish_save(id, path.clone(), cx);
                    this.reload_settings_if(&path, window, cx);
                }
                Err(error) => {
                    window.push_notification(
                        Notification::error(format!("Could not save {filename}: {error}")),
                        cx,
                    );
                }
            })
            .ok();
        })
        .detach();
    }

    pub(super) fn finish_save(&mut self, id: EntityId, path: PathBuf, cx: &mut Context<Self>) {
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
            tab.document.dirty = false;
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
        let parent = path.parent().map(Path::to_path_buf).unwrap_or_default();
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
            move |this, new_name, window, cx| {
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
                if new_path.exists() {
                    window.push_notification(
                        Notification::error(format!("\"{new_name}\" already exists")),
                        cx,
                    );
                    return;
                }

                match std::fs::rename(&path, &new_path) {
                    Ok(()) => {
                        this.after_rename(&path, &new_path, cx);
                        window.push_notification(
                            Notification::success(format!("Renamed to {new_name}")),
                            cx,
                        );
                    }
                    Err(error) => window.push_notification(
                        Notification::error(format!("Could not rename: {error}")),
                        cx,
                    ),
                }
            },
        );
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

                match copy_entry(&path, &destination) {
                    Ok(()) => {
                        if let Some(parent) = destination.parent() {
                            this.workspace.expand(parent.to_path_buf());
                        }
                        let name = file_name(&destination);
                        this.workspace.set_selected(Some(destination));
                        this.reload_tree(cx);
                        this.save_session(cx);
                        cx.notify();
                        window.push_notification(
                            Notification::success(format!("Duplicated as {name}")),
                            cx,
                        );
                    }
                    Err(error) => window.push_notification(
                        Notification::error(format!(
                            "Could not duplicate {}: {error}",
                            file_name(&path)
                        )),
                        cx,
                    ),
                }
            },
        );
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

    use gpui_kit::{AppContext as _, ImageFormat, TestAppContext};

    use super::{DiskChange, TextFormat, classify_disk_change, disk_moved_on, raster_format};
    use crate::app::TeakApp;
    use crate::app::tabs::TabSet;
    use crate::app::test_support::open_app;
    use crate::watcher::Change;

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
                    this.add_file_tab(path.clone(), content, window, cx);
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
                    this.add_file_tab(path.clone(), content, window, cx);
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
