//! One overlay for every list the app asks the user to choose from.
//!
//! The command palette, quick open and Open Recent differ only in their rows:
//! the surface, the focus handoff and the dismissal are the same. Keeping them
//! here means a new picker is a kind, a row builder and a placeholder, not
//! another overlay that has to re-learn the focus rules.
//!
//! The rows come from the app rather than from the widget's own filtering for
//! quick open, which ranks files with [`crate::fuzzy`]; the palette and the
//! recent list list every row and let the widget filter.

use std::path::PathBuf;

use gpui_kit::component::{
    ActiveTheme as _,
    command::{Command, CommandItem, CommandState},
};
use gpui_kit::*;

use crate::paths::{display_path, file_name};
use crate::{CommandPalette, OpenRecentPicker, QuickOpen};

use super::TeakApp;
use super::overlays::PickerKind;
use super::quick_open::{best_score, quick_open_item};

/// Width of the picker panel.
const PICKER_WIDTH: Pixels = px(560.);
/// Gap between the top of the window and the picker panel.
const PICKER_TOP: Pixels = px(72.);

impl PickerKind {
    fn placeholder(self) -> &'static str {
        match self {
            Self::Palette => "Type a command…",
            Self::Files => "Go to file…",
            Self::Recent => "Open recent…",
        }
    }

    fn empty(self) -> &'static str {
        match self {
            Self::Palette => "No matching commands",
            Self::Files => "No matching files",
            Self::Recent => "No recent files",
        }
    }

    /// Every kind ranks its own rows, so the widget must not filter them again:
    /// its filter only keeps rows whose label *contains* the query, which cannot
    /// match an initialism or reorder by score. `fuzzy` decides here instead.
    fn filterable(self) -> bool {
        false
    }
}

impl TeakApp {
    pub(super) fn on_command_palette(
        &mut self,
        _: &CommandPalette,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.toggle_picker(PickerKind::Palette, window, cx);
    }

    pub(super) fn on_quick_open(
        &mut self,
        _: &QuickOpen,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.toggle_picker(PickerKind::Files, window, cx);
    }

    pub(super) fn on_open_recent(
        &mut self,
        _: &OpenRecentPicker,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.toggle_picker(PickerKind::Recent, window, cx);
    }

    /// Opens `kind`, or closes it when it is already the open picker.
    fn toggle_picker(&mut self, kind: PickerKind, window: &mut Window, cx: &mut Context<Self>) {
        if self.overlays.is_picker_open(kind) {
            self.close_picker(kind, window, cx);
        } else {
            self.open_picker(kind, window, cx);
        }
    }

    pub(super) fn open_picker(
        &mut self,
        kind: PickerKind,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Only one picker is ever mounted; opening another replaces it.
        if self.overlays.any_picker_open() {
            self.close_open_picker(window, cx);
        }

        let state = cx.new(|cx| CommandState::new(window, cx));
        // Remember what held focus so it can be restored once the picker closes.
        self.overlays
            .set_picker(kind, state.clone(), window.focused(cx));
        cx.notify();
        // The query input only exists in the tree after the first render, so the
        // focus request is deferred until the overlay has mounted.
        window.defer(cx, move |window, cx| {
            state.update(cx, |state, cx| state.focus(window, cx));
        });

        // Quick open's index is rebuilt for every open, off the main thread, so
        // files created or removed since the last open show up.
        if kind == PickerKind::Files {
            self.start_file_index(window, cx);
        }
    }

    /// Closes `kind` if it is still the open picker. A row that opened another
    /// picker has already replaced this one, and must not be closed by the
    /// confirm or cancel that follows it.
    pub(super) fn close_picker(
        &mut self,
        kind: PickerKind,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((state, return_focus)) = self.overlays.take_picker(kind) else {
            return;
        };
        let handle = state.read(cx).focus_handle(cx);
        // Restore focus only while the picker still holds it. An action that
        // opened its own dialog has already moved focus there.
        if window.focused(cx).as_ref() == Some(&handle)
            && let Some(previous) = return_focus
        {
            window.focus(&previous, cx);
        }
        cx.notify();
    }

    /// Closes whichever picker is open, used before replacing it.
    fn close_open_picker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(kind) = self.overlays.picker().map(|picker| picker.kind) else {
            return;
        };
        self.close_picker(kind, window, cx);
    }

    /// The unified overlay: one surface, one focus handoff, per-kind rows.
    pub(super) fn render_picker(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let picker = self.overlays.picker()?;
        let kind = picker.kind;
        let query = picker.state.read(cx).query(cx).trim().to_string();

        let confirm = cx.entity();
        let cancel = confirm.clone();
        let backdrop = confirm.clone();
        let mut command = Command::new(&picker.state)
            .placeholder(kind.placeholder())
            .filterable(kind.filterable())
            .empty(move |_, _, cx| {
                div()
                    .p_3()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(kind.empty())
            })
            // Quick open re-ranks through the app, so the next render supplies
            // the new rows. The other kinds filter inside the widget.
            .on_query({
                let confirm = confirm.clone();
                move |_, _, cx| {
                    confirm.update(cx, |_, cx| cx.notify());
                }
            })
            .on_confirm({
                let confirm = confirm.clone();
                move |_, window, cx| {
                    confirm.update(cx, |this, cx| this.close_picker(kind, window, cx));
                }
            })
            .on_cancel({
                let cancel = cancel.clone();
                move |window, cx| {
                    // The callback runs while the widget's own Cancel dispatch
                    // is still in progress, so closing here would read its state
                    // mid-update. Hand the dismissal to the next frame.
                    let cancel = cancel.clone();
                    window.defer(cx, move |window, cx| {
                        cancel.update(cx, |this, cx| this.close_picker(kind, window, cx));
                    });
                }
            });

        match kind {
            PickerKind::Palette => {
                for group in crate::commands::palette_groups(&query) {
                    command = command.group(group);
                }
            }
            PickerKind::Files => {
                for item in self.quick_open_items(&query) {
                    command = command.item(item);
                }
            }
            PickerKind::Recent => {
                for item in self.recent_items(&query, cx) {
                    command = command.item(item);
                }
            }
        }

        Some(
            div()
                .absolute()
                .inset_0()
                .occlude()
                .flex()
                .items_start()
                .justify_center()
                .pt(PICKER_TOP)
                .bg(cx.theme().overlay)
                .on_mouse_down(MouseButton::Left, move |_, window, cx| {
                    backdrop.update(cx, |this, cx| this.close_open_picker(window, cx));
                })
                .child(
                    div()
                        .w(PICKER_WIDTH)
                        // Keep clicks on the panel from reaching the backdrop.
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .child(command),
                )
                .into_any_element(),
        )
    }

    /// One row per remembered path, ranked by the same scorer as quick open and
    /// presented the same way: a path fragment matches, but a file name that
    /// matches outranks it. Opening a row reuses [`Self::on_open_path`], so a
    /// folder in the list reveals as a root, and a file is opened for keeps
    /// rather than previewed.
    fn recent_items(&self, query: &str, cx: &mut Context<Self>) -> Vec<CommandItem> {
        let icon_style = self.settings.values.ui.icon_style();
        let query = query.trim();
        let mut ranked: Vec<(u32, usize, String, PathBuf)> = self
            .services
            .recent_paths(cx)
            .into_iter()
            .enumerate()
            .filter_map(|(index, path)| {
                let shown = display_path(&path);
                let score = if query.is_empty() {
                    0
                } else {
                    best_score(query, &file_name(&path), &shown)?
                };
                Some((score, index, shown, path))
            })
            .collect();
        // Stable, so equal scores keep the list's recency order.
        ranked.sort_by_key(|row| std::cmp::Reverse(row.0));

        ranked
            .into_iter()
            .map(|(_, _, shown, path)| quick_open_item(shown, path, icon_style, false))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use gpui_kit::component::IndexPath;
    use gpui_kit::test::TestWindowExt as _;
    use gpui_kit::{AppContext as _, TestAppContext};

    use crate::app::test_support::open_app;

    /// Escape closes the palette. Closing must survive running inside the
    /// palette's own Cancel dispatch, which happens before the dispatch
    /// unwinds, so the close is deferred a frame.
    #[gpui_kit::test]
    async fn escape_closes_the_palette(cx: &mut TestAppContext) {
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
            // The app defers its focus request to the next frame, so mirror it
            // explicitly here the way the kit's palette fixtures do.
            if let Some(palette) = app.read(cx).overlays.palette().cloned() {
                palette.update(cx, |state, cx| state.focus(window, cx));
            }
            window.render_frame(cx);

            window.press("escape", cx);
            window.render_frame(cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            assert!(
                !app.read(cx).overlays.is_palette_open(),
                "Escape closes the palette"
            );
            assert_eq!(
                window.focused(cx).as_ref(),
                Some(&app.read(cx).focus_handle),
                "focus returns to what held it before the palette opened"
            );
        })
        .expect("the headless palette window must survive Escape");
    }

    /// Ctrl+R lists the recent paths and confirming one opens it in a tab.
    #[gpui_kit::test]
    async fn open_recent_lists_and_opens_a_recent_file(cx: &mut TestAppContext) {
        let root = std::env::temp_dir().join(format!("teak-recent-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("recent-note.txt");
        std::fs::write(&path, "hello").unwrap();

        let (handle, app) = open_app(cx);
        cx.update(|cx| {
            app.update(cx, |this, cx| this.note_recent(path.clone(), cx));
        });
        cx.update_window(handle, |_, window, cx| {
            let focus_handle = app.read(cx).focus_handle.clone();
            window.focus(&focus_handle, cx);
            window.render_frame(cx);
            window.press("ctrl-r", cx);
            window.render_frame(cx);
            if let Some(picker) = app.read(cx).overlays.picker().map(|p| p.state.clone()) {
                picker.update(cx, |state, cx| state.focus(window, cx));
            }
            window.render_frame(cx);
            assert!(
                app.read(cx)
                    .overlays
                    .is_picker_open(super::PickerKind::Recent),
                "Ctrl+R opens the recent picker"
            );
            window.click(IndexPath::new(0).section(0), cx);
            window.render_frame(cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, _, cx| {
            let this = app.read(cx);
            assert!(!this.overlays.any_picker_open(), "the picker closes");
            assert!(
                this.tabs
                    .iter()
                    .any(|tab| tab.document.path.as_deref() == Some(path.as_path())),
                "the confirmed recent file opens"
            );
        })
        .unwrap();

        let _ = std::fs::remove_dir_all(&root);
    }

    /// The recent list is filtered and ranked by the app, not by the widget's
    /// `contains` filter: a typed query narrows the rows, and a name match
    /// outranks the newest entry.
    #[gpui_kit::test]
    async fn the_recent_list_ranks_a_name_match_over_recency(cx: &mut TestAppContext) {
        let root = std::env::temp_dir().join(format!("teak-recent-filter-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let alpha = root.join("alpha-note.txt");
        let beta = root.join("beta-note.txt");
        for path in [&alpha, &beta] {
            std::fs::write(path, "hello").unwrap();
        }

        let (handle, app) = open_app(cx);
        cx.update(|cx| {
            app.update(cx, |this, cx| {
                // `alpha` is the newest, so it leads while the query is empty.
                this.note_recent(beta.clone(), cx);
                this.note_recent(alpha.clone(), cx);
            });
        });
        cx.update(|cx| {
            app.update(cx, |this, cx| {
                assert_eq!(this.recent_items("", cx).len(), 2);
                assert!(this.recent_items("zzzz", cx).is_empty());
            });
        });

        cx.update_window(handle, |_, window, cx| {
            let focus_handle = app.read(cx).focus_handle.clone();
            window.focus(&focus_handle, cx);
            window.render_frame(cx);
            window.press("ctrl-r", cx);
            window.render_frame(cx);
            if let Some(picker) = app.read(cx).overlays.picker().map(|p| p.state.clone()) {
                picker.update(cx, |state, cx| state.focus(window, cx));
            }
            window.render_frame(cx);
            // An initialism of `beta`'s name: the widget's substring filter
            // could not find it at all.
            window.input("btn", cx);
            window.render_frame(cx);
            window.click(IndexPath::new(0).section(0), cx);
            window.render_frame(cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, _, cx| {
            let this = app.read(cx);
            assert!(
                this.tabs
                    .iter()
                    .any(|tab| tab.document.path.as_deref() == Some(beta.as_path())),
                "the row whose name matches the query wins"
            );
            assert!(
                !this
                    .tabs
                    .iter()
                    .any(|tab| tab.document.path.as_deref() == Some(alpha.as_path())),
                "the newer, weaker match is not the one opened"
            );
        })
        .unwrap();

        let _ = std::fs::remove_dir_all(&root);
    }

    /// Only one picker is mounted: opening another replaces the first.
    #[gpui_kit::test]
    async fn a_second_picker_replaces_the_first(cx: &mut TestAppContext) {
        let (handle, app) = open_app(cx);
        cx.update_window(handle, |_, window, cx| {
            let focus_handle = app.read(cx).focus_handle.clone();
            window.focus(&focus_handle, cx);
            window.render_frame(cx);
            window.press("secondary-shift-p", cx);
            window.render_frame(cx);
            window.press("ctrl-r", cx);
            window.render_frame(cx);
        })
        .unwrap();

        cx.update_window(handle, |_, _, cx| {
            let this = app.read(cx);
            assert!(!this.overlays.is_palette_open(), "the palette is replaced");
            assert!(
                this.overlays.is_picker_open(super::PickerKind::Recent),
                "the recent picker is the one left"
            );
        })
        .unwrap();
    }
}
