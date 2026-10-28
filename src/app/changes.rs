//! The sidebar's Changes view and the read-only diff tabs it opens.
//!
//! The view lists what the last `git status` read reported, grouped by
//! workspace folder; it reads git and never writes it. A click reads the
//! file's diff against `HEAD` off the main thread and shows it in a generated
//! tab, one per path, tinting the added and removed lines. The status bar's
//! switch and the `ShowExplorer` / `ShowChanges` commands pick the view.

use std::ops::Range;
use std::path::PathBuf;

use gpui_kit::base::{Selectable as _, StyledExt as _};
use gpui_kit::component::{
    ActiveTheme as _, Icon, IconName, Sizable as _, Size,
    button::{Button, ButtonVariants as _},
    input::{RangeDecoration, RangeDecorationStyle},
    list::ListItem,
    scroll::ScrollableElement as _,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::git::{self, FileDiff, Status};
use crate::paths::file_name;
use crate::session::SidebarView;
use crate::{ShowChanges, ShowExplorer};

use super::TeakApp;
use super::tabs::DiffTab;
use super::ui::{GIT_COMPARE_ICON, change_tint};
use super::workspace::ChangeRow;

/// The diff tab's text when git has no hunks to show.
fn diff_text(diff: FileDiff) -> String {
    match diff {
        FileDiff::Text(text) if text.is_empty() => "No content changes".into(),
        FileDiff::Text(text) => text,
        FileDiff::Binary => "Binary file changed".into(),
        FileDiff::Unavailable => "No diff available".into(),
    }
}

/// The color of a status letter, matching the explorer's.
fn status_color(status: Status, cx: &App) -> Hsla {
    let theme = cx.theme();
    match status {
        Status::Untracked | Status::Added => theme.success,
        Status::Modified | Status::Renamed => change_tint(theme.warning, theme.foreground),
        Status::Deleted | Status::Conflicted => theme.danger,
    }
}

impl TeakApp {
    pub(super) fn on_show_explorer(
        &mut self,
        _: &ShowExplorer,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.show_sidebar_view(SidebarView::Explorer, cx);
    }

    pub(super) fn on_show_changes(
        &mut self,
        _: &ShowChanges,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.show_sidebar_view(SidebarView::Changes, cx);
    }

    /// Shows `view` in the sidebar, or hides the sidebar when it already shows
    /// it.
    pub(super) fn show_sidebar_view(&mut self, view: SidebarView, cx: &mut Context<Self>) {
        self.workspace.show_view(view);
        self.save_session(cx);
        cx.notify();
    }

    /// Reads the diff of `path` off the main thread, then shows it.
    fn open_diff(
        &mut self,
        path: PathBuf,
        status: Status,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        cx.spawn_in(window, async move |this, cx| {
            let source = path.clone();
            let diff = cx
                .background_executor()
                .spawn(async move { git::file_diff(&source, Some(status)) })
                .await;
            this.update_in(cx, |this, window, cx| {
                this.show_diff(path, diff_text(diff), window, cx);
            })
            .ok();
        })
        .detach();
    }

    /// Fills the diff tab for `path`, creating it on first use, and shows it.
    /// A second click on the same row re-reads the file into the same tab.
    fn show_diff(
        &mut self,
        path: PathBuf,
        text: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let theme = cx.theme();
        let (added, removed) = (theme.success.opacity(0.15), theme.danger.opacity(0.15));
        let tints: Vec<RangeDecoration> = git::diff_line_ranges(&text)
            .into_iter()
            .map(|(range, kind): (Range<usize>, git::LineKind)| {
                let color = match kind {
                    git::LineKind::Deleted => removed,
                    _ => added,
                };
                RangeDecoration::new(range)
                    .with_style(RangeDecorationStyle::Fill)
                    .with_color(color)
            })
            .collect();

        let existing = self
            .tabs
            .iter()
            .position(|tab| tab.diff.as_ref().is_some_and(|diff| diff.source == path));
        if let Some(ix) = existing {
            let Some(tab) = self.tabs.get_mut(ix) else {
                return;
            };
            let editor = tab.document.editor.clone();
            // `set_value` emits no change event, so the tab stays clean.
            editor.update(cx, |state, cx| state.set_value(text, window, cx));
            if let Some(diff) = tab.diff.as_mut() {
                diff.tints.set(tints, cx);
            }
            self.activate_tab(ix, window, cx);
            return;
        }

        let title = format!("{} (diff)", file_name(&path));
        self.add_document_tab(&title, "Plain Text", text, window, cx);
        let Some(tab) = self
            .tabs
            .active_index()
            .and_then(|ix| self.tabs.get_mut(ix))
        else {
            return;
        };
        // Read-only comes from the `Editor` element, which sets it each render.
        let tints = tab.document.editor.update(cx, |state, cx| {
            state.create_range_decorations_collection(tints, cx)
        });
        tab.diff = Some(DiffTab {
            source: path,
            tints,
        });
        cx.notify();
    }

    /// The Changes view: the virtualized change list, or a line saying why
    /// there is none.
    pub(super) fn render_changes(&self, cx: &mut Context<Self>) -> AnyElement {
        let empty = if self.workspace.roots().is_empty() {
            Some("No folders added")
        } else if !self.workspace.has_repository() {
            Some("No folder is in a git repository")
        } else if self.workspace.changes().is_empty() {
            Some("No changes")
        } else {
            None
        };
        if let Some(message) = empty {
            return div()
                .id("changes-empty")
                .flex_1()
                .min_h_0()
                .v_flex()
                .items_center()
                .justify_center()
                .text_sm()
                .text_color(cx.theme().muted_foreground)
                .child(message)
                .into_any_element();
        }

        let row_count = self.workspace.changes().len();
        div()
            .id("changes-scroll")
            .flex_1()
            .min_h_0()
            .py_1()
            .child(
                uniform_list(
                    "changes-list",
                    row_count,
                    cx.processor(|this, range: Range<usize>, _window, cx| {
                        range.map(|ix| this.change_row(ix, cx)).collect::<Vec<_>>()
                    }),
                )
                .size_full()
                .track_scroll(self.workspace.changes_scroll()),
            )
            .vertical_scrollbar(self.workspace.changes_scroll())
            .into_any_element()
    }

    fn change_row(&self, ix: usize, cx: &mut Context<Self>) -> AnyElement {
        let muted = cx.theme().muted_foreground;
        match &self.workspace.changes()[ix] {
            ChangeRow::Root { path, count } => ListItem::new(("changes-root", ix))
                .w_full()
                .px_2()
                .py_0p5()
                .child(
                    div()
                        .h_flex()
                        .w_full()
                        .items_center()
                        .gap_1()
                        .child(
                            Icon::new(IconName::FolderOpen)
                                .with_size(Size::Small)
                                .text_color(muted),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .text_sm()
                                .font_semibold()
                                .child(file_name(path)),
                        )
                        .child(
                            div()
                                .flex_none()
                                .text_xs()
                                .text_color(muted)
                                .child(count.to_string()),
                        ),
                )
                .into_any_element(),
            ChangeRow::File {
                path,
                label,
                status,
            } => {
                let (path, status) = (path.clone(), *status);
                let selected = self
                    .active()
                    .is_some_and(|tab| tab.diff.as_ref().is_some_and(|diff| diff.source == path));
                ListItem::new(("changes-file", ix))
                    .w_full()
                    .pl(px(20.))
                    .pr_2()
                    .py_0p5()
                    .selected(selected)
                    .accessibility_label(format!("{label} {}", status.letter()))
                    .child(
                        div()
                            .h_flex()
                            .w_full()
                            .items_center()
                            .gap_1()
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .truncate()
                                    .text_sm()
                                    .when(status == Status::Deleted, |name| {
                                        name.line_through().text_color(muted)
                                    })
                                    .child(label.clone()),
                            )
                            .child(
                                div()
                                    .flex_none()
                                    .text_xs()
                                    .text_color(status_color(status, cx))
                                    .child(status.letter()),
                            ),
                    )
                    .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                        this.open_diff(path.clone(), status, window, cx);
                    }))
                    .into_any_element()
            }
        }
    }

    /// The status bar's view switch: Explorer and Changes, the shown one
    /// selected. Clicking the shown view hides the sidebar.
    pub(super) fn render_view_switch(&self, cx: &mut Context<Self>) -> AnyElement {
        let shown = self
            .workspace
            .sidebar_visible()
            .then(|| self.workspace.view());
        let count = self.workspace.change_count();
        div()
            .h_flex()
            .items_center()
            .gap_0p5()
            .child(
                Button::new("switch-explorer")
                    .xsmall()
                    .ghost()
                    .icon(Icon::new(IconName::FolderOpen))
                    .selected(shown == Some(SidebarView::Explorer))
                    .tooltip("Explorer")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.show_sidebar_view(SidebarView::Explorer, cx);
                    })),
            )
            .child(
                Button::new("switch-changes")
                    .xsmall()
                    .ghost()
                    .icon(Icon::empty().data(GIT_COMPARE_ICON))
                    .when(count > 0, |button| button.label(count.to_string()))
                    .selected(shown == Some(SidebarView::Changes))
                    .tooltip("Changes")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.show_sidebar_view(SidebarView::Changes, cx);
                    })),
            )
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use gpui_kit::test::TestWindowExt as _;
    use gpui_kit::{AppContext as _, TestAppContext};

    use super::ChangeRow;
    use crate::app::test_support::open_app_in;
    use crate::git::Status;
    use crate::session::SidebarView;

    /// Initializes a repository in `dir` and commits what it holds. A machine
    /// without git leaves the caller with nothing to check.
    fn commit_all(dir: &Path) -> bool {
        let run = |args: &[&str]| {
            std::process::Command::new("git")
                .arg("-C")
                .arg(dir)
                .args(args)
                .output()
                .is_ok_and(|output| output.status.success())
        };
        run(&["init", "-q", "-b", "trunk"])
            && run(&["add", "."])
            && run(&[
                "-c",
                "user.email=a@b",
                "-c",
                "user.name=a",
                "commit",
                "-qm",
                "init",
            ])
    }

    /// The view lists what git reports under each folder, and a row opens one
    /// read-only diff tab per path that a second click refreshes in place.
    #[gpui_kit::test]
    async fn changed_files_are_listed_and_open_one_diff_tab_each(cx: &mut TestAppContext) {
        let root = std::env::temp_dir().join(format!("teak-changes-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("notes.txt");
        std::fs::write(&path, "one\ntwo\n").unwrap();
        if !commit_all(&root) {
            let _ = std::fs::remove_dir_all(&root);
            return;
        }
        std::fs::write(&path, "one\nchanged\n").unwrap();
        std::fs::write(root.join("new.txt"), "fresh\n").unwrap();

        let (handle, app) = open_app_in(cx, vec![root.clone()]);
        cx.update(|cx| app.update(cx, |this, cx| this.refresh_git_status(cx)));
        cx.run_until_parked();

        let rows = cx.update(|cx| {
            app.read(cx)
                .workspace
                .changes()
                .iter()
                .map(|row| match row {
                    ChangeRow::Root { count, .. } => format!("root {count}"),
                    ChangeRow::File { label, status, .. } => {
                        format!("{label} {}", status.letter())
                    }
                })
                .collect::<Vec<_>>()
        });
        assert_eq!(rows, ["root 2", "new.txt U", "notes.txt M"]);

        cx.update_window(handle, |_, window, cx| {
            app.update(cx, |this, cx| {
                this.show_sidebar_view(SidebarView::Changes, cx);
                this.open_diff(path.clone(), Status::Modified, window, cx);
            });
        })
        .unwrap();
        cx.run_until_parked();
        // Painting a frame runs the Changes view and the status-bar switch.
        cx.update_window(handle, |_, window, cx| window.render_frame(cx))
            .unwrap();

        let (count, text, generated, title) = cx.update(|cx| {
            let this = app.read(cx);
            let tab = this.tabs.active().unwrap();
            (
                this.tabs.len(),
                tab.document.editor.read(cx).text().to_string(),
                tab.generated() && !tab.document.dirty && tab.document.path.is_none(),
                tab.document.title.to_string(),
            )
        });
        assert_eq!(text, "@@ -1,2 +1,2 @@\n one\n-two\n+changed");
        assert!(generated, "a diff tab is generated, clean and pathless");
        assert_eq!(title, "notes.txt (diff)");

        std::fs::write(&path, "one\nagain\n").unwrap();
        cx.update_window(handle, |_, window, cx| {
            app.update(cx, |this, cx| {
                this.open_diff(path.clone(), Status::Modified, window, cx);
            });
        })
        .unwrap();
        cx.run_until_parked();

        let (again, text) = cx.update(|cx| {
            let this = app.read(cx);
            let tab = this.tabs.active().unwrap();
            (
                this.tabs.len(),
                tab.document.editor.read(cx).text().to_string(),
            )
        });
        assert_eq!(again, count, "the same path reuses its tab");
        assert_eq!(text, "@@ -1,2 +1,2 @@\n one\n-two\n+again");

        let _ = std::fs::remove_dir_all(&root);
    }
}
