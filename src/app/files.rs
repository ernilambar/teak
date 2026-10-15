use std::path::{Path, PathBuf};
use std::sync::Arc;

use gpui_kit::component::{
    WindowExt as _,
    button::ButtonVariant,
    input::{Input, InputState},
    notification::Notification,
};
use gpui_kit::*;

use crate::language::{editor_language_id, language_for};
use crate::paths::{copy_entry, duplicate_name, file_name, is_valid_entry_name, remap_prefix};
use crate::{AddFolder, CloseAllFolders, NewFile, OpenFile, SaveAll, SaveFile, SaveFileAs};

use super::TinytextApp;
use super::workspace::{AddRoot, FileClipboard};

/// A file read for a new tab: text for the editor, or a raster image to view.
pub(super) enum FileContent {
    Text(String),
    Image(Arc<Image>),
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
/// else must be UTF-8 text.
pub(super) fn read_file(path: &Path) -> std::io::Result<FileContent> {
    match raster_format(path) {
        Some(format) => {
            let bytes = std::fs::read(path)?;
            Ok(FileContent::Image(Arc::new(Image::from_bytes(
                format, bytes,
            ))))
        }
        None => std::fs::read_to_string(path).map(FileContent::Text),
    }
}

/// The notification shown when a file the user asked for cannot be opened.
fn open_error(path: &Path, error: &std::io::Error) -> String {
    let filename = file_name(path);
    if error.kind() == std::io::ErrorKind::InvalidData {
        format!("Could not open {filename}: not a UTF-8 text file")
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

impl TinytextApp {
    pub(crate) fn request_open(
        &mut self,
        path: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if path.is_dir() {
            self.add_root(path, window, cx);
            return;
        }

        if let Some(ix) = self
            .tabs
            .iter()
            .position(|tab| tab.path.as_deref() == Some(path.as_path()))
        {
            self.activate_tab(ix, window, cx);
            return;
        }

        cx.spawn_in(window, async move |this, cx| {
            let read = cx
                .background_executor()
                .spawn({
                    let path = path.clone();
                    async move { read_file(&path) }
                })
                .await;
            this.update_in(cx, |this, window, cx| match read {
                Ok(content) => this.add_file_tab(path, content, window, cx),
                Err(error) => {
                    window.push_notification(Notification::error(open_error(&path, &error)), cx)
                }
            })
            .ok();
        })
        .detach();
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
                for path in paths {
                    let read = cx
                        .background_executor()
                        .spawn({
                            let path = path.clone();
                            async move { read_file(&path) }
                        })
                        .await;
                    this.update_in(cx, |this, window, cx| match read {
                        Ok(content) => this.add_file_tab(path, content, window, cx),
                        Err(error) => window
                            .push_notification(Notification::error(open_error(&path, &error)), cx),
                    })
                    .ok();
                }
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

    /// Adds a folder to the sidebar. Exact duplicates are ignored silently;
    /// folders that overlap an open root are rejected with a message.
    pub(super) fn add_root(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        match self.workspace.add_root(path.clone()) {
            AddRoot::Duplicate => return,
            AddRoot::Overlap => {
                window.push_notification(
                    Notification::error(format!(
                        "Cannot add \"{}\": it overlaps an open folder",
                        file_name(&path)
                    )),
                    cx,
                );
                return;
            }
            AddRoot::Added => {}
        }

        self.save_session();
        cx.notify();
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
        self.save_session();
        cx.notify();
    }

    /// Removes every workspace root. Open tabs stay open and the sidebar stays
    /// visible, showing its empty state.
    pub(super) fn close_all_roots(&mut self, cx: &mut Context<Self>) {
        self.workspace.close_all();
        self.save_session();
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
        let id = tab.editor.entity_id();

        if let Some(path) = tab.path.clone() {
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
        if tab.raster {
            return;
        }
        let id = tab.editor.entity_id();
        let suggested = tab
            .path
            .as_deref()
            .map(file_name)
            .unwrap_or_else(|| "untitled.txt".to_string());
        let directory = tab
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
            .filter(|tab| tab.dirty)
            .filter_map(|tab| tab.path.clone().map(|path| (tab.editor.entity_id(), path)))
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
        let text = tab.editor.read(cx).text().to_string();
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
            .find(|tab| tab.editor.entity_id() == id)
        {
            tab.title = file_name(&path).into();
            if !tab.language_override {
                let language = language_for(&path);
                tab.editor.update(cx, |state, cx| {
                    state.set_highlighter(editor_language_id(&language), cx);
                });
                tab.language = language;
            }
            tab.path = Some(path);
            tab.dirty = false;
        }
        self.reload_tree();
        self.save_session();
        cx.notify();
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
                this.reload_tree();
                this.save_session();
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
                        this.reload_tree();
                        this.save_session();
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
            let Some(current) = tab.path.clone() else {
                continue;
            };
            let Some(mapped) = remap_prefix(&current, old, new) else {
                continue;
            };
            tab.title = file_name(&mapped).into();
            if !tab.language_override {
                let language = language_for(&mapped);
                tab.editor.update(cx, |state, cx| {
                    state.set_highlighter(editor_language_id(&language), cx);
                });
                tab.language = language;
            }
            tab.path = Some(mapped);
        }

        self.workspace.remap(old, new);

        self.reload_tree();
        self.save_session();
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
                .description(format!("Delete \"{name}\"? This cannot be undone."))
                .ok_text("Delete")
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
                tab.path
                    .as_deref()
                    .is_some_and(|candidate| candidate == path || candidate.starts_with(path))
            })
            .map(|tab| tab.editor.entity_id())
            .collect();
        for id in ids {
            self.remove_tab(id, window, cx);
        }

        let result = if path.is_dir() {
            std::fs::remove_dir_all(path)
        } else {
            std::fs::remove_file(path)
        };

        match result {
            Ok(()) => {
                let name = file_name(path);
                self.workspace.forget(path);
                self.reload_tree();
                self.save_session();
                cx.notify();
                window.push_notification(Notification::success(format!("Deleted {name}")), cx);
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
                    self.reload_tree();
                    self.save_session();
                    cx.notify();
                    window.push_notification(Notification::success(format!("Pasted {name}")), cx);
                }
            }
            Err(error) => window
                .push_notification(Notification::error(format!("Could not paste: {error}")), cx),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use gpui_kit::ImageFormat;

    use super::raster_format;

    #[test]
    fn raster_format_maps_image_extensions() {
        assert_eq!(raster_format(Path::new("a.PNG")), Some(ImageFormat::Png));
        assert_eq!(raster_format(Path::new("a.jpeg")), Some(ImageFormat::Jpeg));
        assert_eq!(raster_format(Path::new("icon.svg")), None);
        assert_eq!(raster_format(Path::new("notes.txt")), None);
    }
}
