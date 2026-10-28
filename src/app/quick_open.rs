//! Quick Open's rows: the workspace file index and the ranking over it.
//!
//! The overlay itself, including the widget and its dismissal, lives in
//! `picker.rs`. This module keeps the part that is specific to files: the
//! background walk that builds the index, the fuzzy ranking of each query, and
//! the row presentation.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use globset::GlobSet;
use gpui_kit::base::StyledExt as _;
use gpui_kit::component::{ActiveTheme as _, command::CommandItem};
use gpui_kit::*;

use crate::OpenPath;
use crate::file_icons::file_icon;
use crate::fuzzy;
use crate::paths::{collect_files, exclude_set, file_name, nearest_root, relative_display};
use crate::settings::IconStyle;

use super::{TeakApp, ui::file_type_icon};

/// Hard cap on indexed files, so a runaway root cannot freeze the walk.
pub(super) const MAX_INDEX_FILES: usize = 20_000;
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

impl TeakApp {
    /// Walks the workspace roots for quick open's index, off the main thread.
    /// Rebuilt on every open so files created or removed since the last one are
    /// picked up. With no roots, the open tabs stand in for the project.
    pub(super) fn start_file_index(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let roots = self.workspace.roots().to_vec();
        // Settings were validated on load, so a bad glob cannot reach here.
        let exclude = exclude_set(self.settings.values.search.exclude()).unwrap_or_default();
        let fallback: Vec<PathBuf> = self
            .tabs
            .iter()
            .filter_map(|tab| tab.document.path.clone())
            .collect();
        cx.spawn_in(window, async move |this, cx| {
            let files = cx
                .background_executor()
                .spawn(async move {
                    if roots.is_empty() {
                        fallback
                    } else {
                        collect_files(&roots, MAX_INDEX_FILES, &exclude, &GlobSet::empty())
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

    /// Opens the path a picker confirmed: a folder becomes a root, a file a
    /// tab. Shared by quick open and Open Recent, whose lists mix both.
    pub(super) fn on_open_path(
        &mut self,
        action: &OpenPath,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let path = action.0.clone();
        if path.is_dir() {
            self.add_root(path, window, cx);
        } else if action.1 {
            // Quick open previews; Explorer double-click and Open Recent keep.
            self.request_open_transient(path, window, cx);
        } else {
            self.request_open(path, window, cx);
        }
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
            .filter_map(|(ix, tab)| tab.document.path.as_deref().map(|path| (path, ix)))
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
        .action(Box::new(OpenPath(path.clone(), true)))
        .child(move |_window, cx| {
            let mut row = div()
                .h_flex()
                .flex_1()
                .min_w_0()
                .items_center()
                .gap_2()
                .child(file_type_icon(file_icon(&path), icon_style, None, cx))
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

            window.press("secondary-p", cx);
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
            window.press("secondary-p", cx);
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
                    .any(|tab| tab.document.path.as_deref() == Some(path.as_path())),
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

            window.press("secondary-shift-p", cx);
            window.render_frame(cx);
            assert!(
                app.read(cx).overlays.is_palette_open(),
                "the palette should open"
            );

            // The File group's fourth row is "Quick Open…".
            window.click(IndexPath::new(3).section(0), cx);
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
