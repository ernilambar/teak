//! Quick Open: a fuzzy search over the workspace files, opening on Cmd+P.
//!
//! Like the command palette (see `palette.rs`), this is an in-app overlay so
//! the chosen file's action reaches the root's `on_action` handlers. The list
//! reuses gpui-kit's `Command` widget, but ranks the files itself: the widget
//! runs with local filtering off, and this module rebuilds the visible rows
//! with fuzzy-scored results whenever the query changes.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use gpui_kit::base::StyledExt as _;
use gpui_kit::component::{
    ActiveTheme as _,
    command::{Command, CommandItem, CommandState},
};
use gpui_kit::*;

use crate::file_icons::file_icon;
use crate::fuzzy;
use crate::paths::{collect_files, file_name, nearest_root, relative_display};
use crate::settings::IconStyle;
use crate::{OpenPath, QuickOpen};

use super::{TinytextApp, ui::file_type_icon};

/// Width of the finder panel, shared with the command palette.
const FINDER_WIDTH: Pixels = px(560.);
/// Gap between the top of the window and the finder panel.
const FINDER_TOP: Pixels = px(72.);
/// Hard cap on indexed files, so a runaway root cannot freeze the walk.
const MAX_INDEX_FILES: usize = 20_000;
/// Results rebuilt per keystroke; the list itself virtualizes.
const MAX_RESULTS: usize = 100;
/// Extra credit for a query fully matching inside the file name.
const NAME_BONUS: u32 = 30;
/// Rank files with a tab above files that are not open.
const NOT_OPEN: usize = usize::MAX;

/// The best score across the two views of the file.
fn best_score(query: &str, name: &str, rel: &str) -> Option<u32> {
    let on_name = fuzzy::score(query, name).map(|score| score + NAME_BONUS);
    let on_path = fuzzy::score(query, rel);
    match (on_name, on_path) {
        (Some(name), Some(path)) => Some(name.max(path)),
        (Some(name), None) => Some(name),
        (None, Some(path)) => Some(path),
        (None, None) => None,
    }
}

impl TinytextApp {
    /// Opens the finder, or closes it when it is already open.
    pub(super) fn on_quick_open(
        &mut self,
        _: &QuickOpen,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.overlays.is_quick_open_open() {
            self.close_quick_open(window, cx);
        } else {
            self.open_quick_open(window, cx);
        }
    }

    fn open_quick_open(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // Keep these overlays exclusive, they would stack otherwise.
        if self.overlays.is_palette_open() {
            self.close_palette(window, cx);
        }
        if self.overlays.is_find_open() {
            self.close_find_in_files(window, cx);
        }

        let state = cx.new(|cx| CommandState::new(window, cx));
        // Remember what held focus so it can be restored once the finder closes.
        self.overlays
            .set_quick_open(state.clone(), window.focused(cx));
        cx.notify();
        // The query input only exists in the tree after the first render, so the
        // focus request is deferred until the overlay has mounted.
        window.defer(cx, move |window, cx| {
            state.update(cx, |state, cx| state.focus(window, cx));
        });

        // The index is rebuilt for every open so late file creation and
        // removals are picked up; the walk runs off the main thread. With no
        // workspace roots the open tabs stand in for the project.
        let roots = self.workspace.roots().to_vec();
        let fallback: Vec<PathBuf> = self
            .tabs
            .iter()
            .filter_map(|tab| tab.path.clone())
            .collect();
        cx.spawn_in(window, async move |this, cx| {
            let files = cx
                .background_executor()
                .spawn(async move {
                    if roots.is_empty() {
                        fallback
                    } else {
                        collect_files(&roots, MAX_INDEX_FILES)
                    }
                })
                .await;
            this.update_in(cx, |this, _window, cx| {
                this.overlays.set_open_paths(files);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Closes the finder, restoring focus only while the query field still
    /// holds it. An action that opened its own dialog has already moved focus
    /// there, and stealing it back would break that dialog.
    pub(super) fn close_quick_open(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((state, return_focus)) = self.overlays.take_quick_open() else {
            return;
        };
        let finder_handle = state.read(cx).focus_handle(cx);
        if window.focused(cx).as_ref() == Some(&finder_handle)
            && let Some(previous) = return_focus
        {
            window.focus(&previous, cx);
        }
        cx.notify();
    }

    /// Opens the confirmed file, sharing the open path with the sidebar.
    pub(super) fn on_open_path(
        &mut self,
        action: &OpenPath,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.request_open(action.0.clone(), window, cx);
    }

    /// The rows for the current query: fuzzy matches ranked best first, or with
    /// an empty query the open tabs ahead of the remaining index in path order.
    pub(super) fn quick_open_items(&self, query: &str) -> Vec<CommandItem> {
        let icon_style = self.settings.values.ui.icon_style();
        let query = query.trim();
        let opened: HashMap<&Path, usize> = self
            .tabs
            .iter()
            .enumerate()
            .filter_map(|(ix, tab)| tab.path.as_deref().map(|path| (path, ix)))
            .collect();

        let mut ranked: Vec<(u32, usize, String, PathBuf)> = self
            .overlays
            .open_paths()
            .iter()
            .filter_map(|path| {
                let rel = match nearest_root(self.workspace.roots(), path) {
                    Some(root) => relative_display(path, root),
                    None => path.display().to_string(),
                };
                let (score, tab_order) = if query.is_empty() {
                    (0, opened.get(path.as_path()).copied().unwrap_or(NOT_OPEN))
                } else {
                    (best_score(query, &file_name(path), &rel)?, NOT_OPEN)
                };
                Some((score, tab_order, rel, path.clone()))
            })
            .collect();

        if query.is_empty() {
            ranked.sort_by(|a, b| a.1.cmp(&b.1).then_with(|| a.2.cmp(&b.2)));
        } else {
            ranked.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.2.cmp(&b.2)));
        }
        ranked.truncate(MAX_RESULTS);

        ranked
            .into_iter()
            .map(|(_, _, rel, path)| quick_open_item(rel, path, icon_style))
            .collect()
    }

    pub(super) fn render_quick_open(
        &self,
        state: &Entity<CommandState>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let confirm = cx.entity();
        let cancel = confirm.clone();
        let backdrop = confirm.clone();

        // Local filtering stays off: the rows already answer the query.
        let query = state.read(cx).query(cx).trim().to_string();
        let mut command = Command::new(state)
            .placeholder("Go to file…")
            .filterable(false)
            .empty(|_, _, cx| {
                div()
                    .p_3()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child("No matching files")
            })
            // Re-rank through the app so the next render supplies the new rows.
            .on_query({
                let confirm = confirm.clone();
                move |_, _, cx| {
                    confirm.update(cx, |_, cx| cx.notify());
                }
            })
            .on_confirm(move |_, window, cx| {
                confirm.update(cx, |this, cx| this.close_quick_open(window, cx));
            })
            .on_cancel({
                let cancel = cancel.clone();
                move |window, cx| {
                    // The callback runs while the finder's Cancel dispatch is
                    // still in progress, so closing here would read the finder
                    // state while it is being updated. Hand the dismissal to
                    // the next frame instead.
                    let cancel = cancel.clone();
                    window.defer(cx, move |window, cx| {
                        cancel.update(cx, |this, cx| this.close_quick_open(window, cx));
                    });
                }
            });
        for item in self.quick_open_items(&query) {
            command = command.item(item);
        }

        div()
            .absolute()
            .inset_0()
            .occlude()
            .flex()
            .items_start()
            .justify_center()
            .pt(FINDER_TOP)
            .bg(cx.theme().overlay)
            .on_mouse_down(MouseButton::Left, move |_, window, cx| {
                backdrop.update(cx, |this, cx| this.close_quick_open(window, cx));
            })
            .child(
                div()
                    .w(FINDER_WIDTH)
                    // Keep clicks on the panel from reaching the backdrop.
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .child(command),
            )
            .into_any_element()
    }
}

/// A finder row: the file's icon, its name, and its directory, muted, on the
/// right. The custom content owns the full row presentation, including which
/// parts the ellipsis can drop.
fn quick_open_item(rel: String, path: PathBuf, icon_style: IconStyle) -> CommandItem {
    let name: SharedString = file_name(&path).into();
    let directory: SharedString = match Path::new(&rel).parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent.display().to_string().into(),
        _ => String::new().into(),
    };

    CommandItem::new()
        .action(Box::new(OpenPath(path.clone())))
        .child(move |_window, cx| {
            let mut row = div()
                .h_flex()
                .flex_1()
                .min_w_0()
                .items_center()
                .gap_2()
                .child(file_type_icon(file_icon(&path), icon_style, cx))
                .child(
                    div()
                        .min_w_0()
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_ellipsis()
                        .child(name.clone()),
                );
            if !directory.is_empty() {
                row = row.child(
                    div()
                        .ml_auto()
                        .max_w(px(220.))
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_ellipsis()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(directory.clone()),
                );
            }
            row.into_any_element()
        })
}

#[cfg(test)]
mod tests {
    // `#[gpui_kit::test]` requires the kit's test-support feature, which the
    // dev-dependency turns on for the test build; the headless harness then
    // lets these tests render the real app in a window.
    use gpui_kit::component::IndexPath;
    use gpui_kit::test::TestWindowExt as _;
    use gpui_kit::{AppContext as _, Focusable as _, TestAppContext};

    use crate::app::test_support::open_app;

    /// Escape closes the finder: it clears a typed query first and closes the
    /// empty finder on the next press. Closing must survive running inside the
    /// finder's own Cancel dispatch.
    #[gpui_kit::test]
    async fn escape_closes_the_finder(cx: &mut TestAppContext) {
        let (handle, app) = open_app(cx);

        cx.update_window(handle, |_, window, cx| {
            let focus_handle = app.read(cx).focus_handle.clone();
            window.focus(&focus_handle, cx);
            window.render_frame(cx);

            window.press("cmd-p", cx);
            window.render_frame(cx);
            assert!(
                app.read(cx).overlays.is_quick_open_open(),
                "the finder should be open"
            );
            // The app defers its focus request to the next frame, so mirror it
            // explicitly here the way the kit's palette fixtures do.
            if let Some(finder) = app.read(cx).overlays.quick_open().cloned() {
                finder.update(cx, |state, cx| state.focus(window, cx));
            }
            window.render_frame(cx);

            window.input("a", cx);
            window.press("escape", cx);
            window.render_frame(cx);
            let finder = app.read(cx).overlays.quick_open().cloned();
            assert_eq!(
                finder.map(|state| state.read(cx).query(cx).to_string()),
                Some(String::new()),
                "the first Escape clears the query"
            );

            window.press("escape", cx);
            window.render_frame(cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            assert!(
                !app.read(cx).overlays.is_quick_open_open(),
                "the second Escape closes the finder"
            );
            assert_eq!(
                window.focused(cx).as_ref(),
                Some(&app.read(cx).focus_handle),
                "focus returns to what held it before the finder opened"
            );
        })
        .expect("the headless finder window must survive Escape");
    }

    /// Confirming a row opens its file in a tab.
    #[gpui_kit::test]
    async fn confirming_a_row_opens_the_file(cx: &mut TestAppContext) {
        let (handle, app) = open_app(cx);
        let dir = crate::paths::support_dir().unwrap();
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("quick-open-target.txt");
        std::fs::write(&path, "hello").unwrap();

        cx.update_window(handle, |_, window, cx| {
            let focus_handle = app.read(cx).focus_handle.clone();
            window.focus(&focus_handle, cx);
            window.render_frame(cx);
            window.press("cmd-p", cx);
        })
        .unwrap();
        // Let the index walk land before replacing its result.
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            app.update(cx, |this, _| {
                this.overlays.set_open_paths(vec![path.clone()])
            });
            window.render_frame(cx);
            window.click(IndexPath::new(0).section(0), cx);
            window.render_frame(cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, _, cx| {
            let app = app.read(cx);
            assert!(!app.overlays.is_quick_open_open(), "the finder closes");
            assert!(
                app.tabs
                    .iter()
                    .any(|tab| tab.path.as_deref() == Some(path.as_path())),
                "the confirmed file opens in a tab"
            );
        })
        .unwrap();
    }

    /// Picking "Quick Open…" from the command palette closes the palette and
    /// opens the finder. The palette closes from inside the dispatch of one of
    /// its own rows, which must not read the palette state mid-update.
    #[gpui_kit::test]
    async fn quick_open_from_the_palette_swaps_the_overlays(cx: &mut TestAppContext) {
        let (handle, app) = open_app(cx);

        cx.update_window(handle, |_, window, cx| {
            let focus_handle = app.read(cx).focus_handle.clone();
            window.focus(&focus_handle, cx);
            window.render_frame(cx);

            window.press("cmd-shift-p", cx);
            window.render_frame(cx);
            assert!(
                app.read(cx).overlays.is_palette_open(),
                "the palette should open"
            );

            // The File group's third row is "Quick Open…".
            window.click(IndexPath::new(2).section(0), cx);
            window.render_frame(cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            assert!(
                !app.read(cx).overlays.is_palette_open(),
                "the palette should close behind the finder"
            );
            let finder = app.read(cx).overlays.quick_open().cloned();
            assert!(finder.is_some(), "the finder should be open");
            assert_eq!(
                window.focused(cx).as_ref(),
                finder.map(|state| state.read(cx).focus_handle(cx)).as_ref(),
                "the finder's field takes focus from the palette"
            );
        })
        .expect("swapping overlays must survive the palette's own dispatch");
    }
}
