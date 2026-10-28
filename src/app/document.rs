//! A file's text and the state that belongs to the file rather than to the way
//! it is shown.
//!
//! A [`Document`] owns the editor entity — the buffer, its undo history and the
//! language — plus the path, the language choice, the on-disk format and the
//! bookkeeping that keeps saving honest: whether it is dirty, when the file was
//! last read, and whether it moved or vanished under the buffer.
//!
//! A [`super::tabs::OpenTab`] is a *view* of a document: its preview, its Find
//! Results rows and the subscriptions that tie it to this window. Splitting the
//! two is what keeps a tab's presentation from owning the file's state, and it
//! is the seam a second view (a column, a diff tab) will sit on.
//!
//! Why one document per tab, for now: gpui-component's `EditorState` is both the
//! text and the selection, and its undo history lives inside it. Two views of
//! one file that each had their own caret would need a buffer shared below that
//! entity, which the engine does not expose. Until then a second view of a path
//! shares this document's `EditorState` (and therefore its caret), so tabs that
//! show the same file within a window are deduplicated rather than duplicated.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use gpui_kit::component::input::EditorState;
use gpui_kit::{Entity, SharedString};

use super::files::TextFormat;

/// A file as the editor knows it, independent of the tab showing it.
pub(super) struct Document {
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
    /// A later on-disk version the user chose to keep the buffer over. Compared
    /// only to stop the same change asking again; saving still warns, because
    /// [`Self::disk_modified`] stays behind.
    pub(super) dismissed_disk_change: Option<SystemTime>,
    /// True when the file is gone from disk while the buffer still holds its
    /// text. The tab marks itself and saving asks before writing it back.
    pub(super) deleted_on_disk: bool,
    pub(super) format: TextFormat,
    /// The buffer and its undo history. A view renders this entity; it does not
    /// own it.
    pub(super) editor: Entity<EditorState>,
}

impl Document {
    pub(super) fn new(
        path: Option<PathBuf>,
        title: SharedString,
        language: SharedString,
        format: TextFormat,
        editor: Entity<EditorState>,
    ) -> Self {
        let disk_modified = path.as_deref().and_then(super::files::modified_time);
        Self {
            path,
            title,
            language,
            language_override: false,
            dirty: false,
            disk_modified,
            dismissed_disk_change: None,
            deleted_on_disk: false,
            format,
            editor,
        }
    }

    pub(super) fn editor_id(&self) -> gpui_kit::EntityId {
        self.editor.entity_id()
    }

    /// The path as a plain `&Path`, for the many places that only compare it.
    pub(super) fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    /// Re-derives the title and, unless the user chose one, the language after
    /// the file moved. `first_line` is the buffer's first line, which the
    /// language heuristic reads.
    pub(super) fn moved_to(&mut self, path: PathBuf, first_line: &str) {
        self.title = crate::paths::file_name(&path).into();
        if !self.language_override {
            self.language = crate::language::language_for(&path, first_line);
        }
        self.path = Some(path);
    }

    /// Takes the state a successful read or write leaves behind: the buffer is
    /// now what the disk holds.
    pub(super) fn synced_with_disk(&mut self, modified: Option<SystemTime>, format: TextFormat) {
        self.disk_modified = modified;
        self.dismissed_disk_change = None;
        self.deleted_on_disk = false;
        self.format = format;
    }

    /// True when `current` is a later version than the one last read, and the
    /// user has not already waved that version away.
    pub(super) fn moved_on_disk(&self, current: Option<SystemTime>) -> bool {
        super::files::disk_moved_on(self.disk_modified, current)
            && self.dismissed_disk_change != current
    }

    /// True when saving this document would overwrite bytes the editor could
    /// not decode, which only matters while it still names that same path.
    pub(super) fn would_lose_bytes_to(&self, path: &Path) -> bool {
        self.format.lossy && self.path() == Some(path)
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::time::{Duration, SystemTime};

    use gpui_kit::component::input::EditorState;
    use gpui_kit::{AppContext as _, TestAppContext};

    use super::Document;
    use crate::app::files::TextFormat;
    use crate::app::test_support::open_app;

    fn document(cx: &mut TestAppContext, path: Option<&str>) -> Document {
        let (handle, _app) = open_app(cx);
        let editor = cx
            .update_window(handle, |_, window, cx| {
                cx.new(|cx| EditorState::new(window, cx))
            })
            .unwrap();
        Document::new(
            path.map(PathBuf::from),
            "untitled".into(),
            "Plain Text".into(),
            TextFormat::default(),
            editor,
        )
    }

    /// A newer on-disk version asks once; the dismissal silences only that one.
    #[gpui_kit::test]
    async fn moved_on_disk_reports_a_newer_version_once(cx: &mut TestAppContext) {
        let mut doc = document(cx, None);
        doc.disk_modified = Some(SystemTime::UNIX_EPOCH);
        let newer = SystemTime::UNIX_EPOCH + Duration::from_secs(60);

        assert!(!doc.moved_on_disk(Some(SystemTime::UNIX_EPOCH)));
        assert!(doc.moved_on_disk(Some(newer)));

        doc.dismissed_disk_change = Some(newer);
        assert!(!doc.moved_on_disk(Some(newer)), "the same change asks once");
        assert!(
            doc.moved_on_disk(Some(newer + Duration::from_secs(60))),
            "a later change asks again"
        );
    }

    /// A synced document forgets the deletion and dismissal of the version it
    /// replaced, and adopts the format it was read in.
    #[gpui_kit::test]
    async fn syncing_with_disk_forgets_a_previous_conflict(cx: &mut TestAppContext) {
        let mut doc = document(cx, None);
        doc.deleted_on_disk = true;
        doc.dismissed_disk_change = Some(SystemTime::UNIX_EPOCH);
        let crlf = TextFormat {
            crlf: true,
            ..TextFormat::default()
        };

        doc.synced_with_disk(Some(SystemTime::UNIX_EPOCH), crlf);

        assert!(!doc.deleted_on_disk);
        assert_eq!(doc.dismissed_disk_change, None);
        assert!(doc.format.crlf);
        assert_eq!(doc.disk_modified, Some(SystemTime::UNIX_EPOCH));
    }

    /// A language picked in the status bar outlives a rename; otherwise the
    /// new extension decides.
    #[gpui_kit::test]
    async fn a_rename_keeps_the_chosen_language(cx: &mut TestAppContext) {
        let mut doc = document(cx, Some("/tmp/notes.txt"));
        doc.language_override = true;
        doc.language = "Markdown".into();

        doc.moved_to(PathBuf::from("/tmp/notes.py"), "");

        assert_eq!(doc.language.as_ref(), "Markdown");
        assert_eq!(doc.title.as_ref(), "notes.py");
        assert_eq!(doc.path(), Some(std::path::Path::new("/tmp/notes.py")));
    }

    #[gpui_kit::test]
    async fn a_rename_rederives_an_automatic_language(cx: &mut TestAppContext) {
        let mut doc = document(cx, Some("/tmp/notes.txt"));
        assert_eq!(doc.language.as_ref(), "Plain Text");

        doc.moved_to(PathBuf::from("/tmp/main.rs"), "fn main() {}");

        assert_eq!(doc.language.as_ref(), "Rust");
    }

    /// Overwriting undecodable bytes is only a risk for the file they came
    /// from; Save As elsewhere loses nothing.
    #[gpui_kit::test]
    async fn losing_bytes_depends_on_the_path(cx: &mut TestAppContext) {
        let mut doc = document(cx, Some("/tmp/broken.txt"));
        doc.format = TextFormat {
            lossy: true,
            ..TextFormat::default()
        };

        assert!(doc.would_lose_bytes_to(std::path::Path::new("/tmp/broken.txt")));
        assert!(!doc.would_lose_bytes_to(std::path::Path::new("/tmp/copy.txt")));
    }
}
