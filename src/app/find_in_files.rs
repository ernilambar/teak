//! Find in Files: a text search across the workspace, opening on Cmd+Shift+F.
//!
//! Built like Quick Open (see `quick_open.rs`): an in-app overlay over
//! gpui-kit's `Command` widget with local filtering off. Each query change
//! starts a debounced background search (see `search.rs`); a newer query bumps
//! the generation counter, which stops the older search and drops its results.

use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::time::Duration;

use gpui_kit::base::StyledExt as _;
use gpui_kit::component::{
    ActiveTheme as _,
    button::Toggle,
    command::{Command, CommandItem, CommandState},
    input::{Position, RopeExt as _},
};
use gpui_kit::*;

use crate::file_icons::file_icon;
use crate::paths::{collect_files, exclude_set, file_name, nearest_root, relative_display};
use crate::search::{MAX_MATCHES, SearchMatch, SearchOptions, build_matcher, search_files};
use crate::settings::IconStyle;
use crate::{FindInFiles, OpenMatch};

use super::overlays::FindOutcome;
use super::tabs::Jump;
use super::{TinytextApp, ui::file_type_icon};

/// Width of the search panel; wider than Quick Open to fit line previews.
const FIND_WIDTH: Pixels = px(720.);
/// Gap between the top of the window and the panel, shared with Quick Open.
const FIND_TOP: Pixels = px(72.);
/// Hard cap on searched files, shared in spirit with Quick Open's index.
const MAX_FIND_FILES: usize = 20_000;
/// Pause after a keystroke before searching, so typing does not start a scan
/// per character.
const DEBOUNCE: Duration = Duration::from_millis(150);

impl TinytextApp {
    /// Opens Find in Files, or closes it when it is already open.
    pub(super) fn on_find_in_files(
        &mut self,
        _: &FindInFiles,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.overlays.is_find_open() {
            self.close_find_in_files(window, cx);
        } else {
            self.open_find_in_files(window, cx);
        }
    }

    fn open_find_in_files(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // Keep the overlays exclusive, they would stack otherwise.
        if self.overlays.is_palette_open() {
            self.close_palette(window, cx);
        }
        if self.overlays.is_quick_open_open() {
            self.close_quick_open(window, cx);
        }

        let state = cx.new(|cx| CommandState::new(window, cx));
        self.overlays.set_find(state.clone(), window.focused(cx));
        cx.notify();
        // The query input only exists after the first render.
        window.defer(cx, move |window, cx| {
            state.update(cx, |state, cx| state.focus(window, cx));
        });

        // Walked on every open so new and removed files are picked up. With no
        // workspace roots the open tabs stand in for the project.
        let roots = self.workspace.roots().to_vec();
        // Settings were validated on load, so a bad glob cannot reach here.
        let exclude = exclude_set(self.settings.values.search.exclude()).unwrap_or_default();
        let fallback: Vec<PathBuf> = self
            .tabs
            .iter()
            .filter(|tab| !tab.raster)
            .filter_map(|tab| tab.path.clone())
            .collect();
        cx.spawn_in(window, async move |this, cx| {
            let files = cx
                .background_executor()
                .spawn(async move {
                    if roots.is_empty() {
                        fallback
                    } else {
                        collect_files(&roots, MAX_FIND_FILES, &exclude)
                    }
                })
                .await;
            this.update_in(cx, |this, window, cx| {
                this.overlays.set_find_files(files);
                // A query typed before the walk finished searched nothing.
                if this.overlays.is_find_open() {
                    this.run_search(window, cx);
                }
            })
            .ok();
        })
        .detach();
    }

    /// Closes Find in Files, restoring focus only while its query field still
    /// holds it, so a just-opened result keeps the focus it took.
    pub(super) fn close_find_in_files(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((state, return_focus)) = self.overlays.take_find() else {
            return;
        };
        let field = state.read(cx).focus_handle(cx);
        if window.focused(cx).as_ref() == Some(&field)
            && let Some(previous) = return_focus
        {
            window.focus(&previous, cx);
        }
        cx.notify();
    }

    /// Searches for the current query after a short pause. Results land only
    /// if no newer query has started in the meantime.
    fn run_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let generation = self.overlays.next_find_generation();
        let query = self.overlays.find_query().to_string();
        if query.trim().is_empty() {
            self.overlays.set_find_outcome(FindOutcome::Idle);
            cx.notify();
            return;
        }
        let matcher = match build_matcher(&query, self.overlays.find_options()) {
            Ok(matcher) => matcher,
            Err(error) => {
                self.overlays.set_find_outcome(FindOutcome::Invalid(error));
                cx.notify();
                return;
            }
        };
        self.overlays.set_find_outcome(FindOutcome::Searching);
        cx.notify();

        let files = self.overlays.find_files();
        let counter = self.overlays.find_generation();
        cx.spawn_in(window, async move |this, cx| {
            cx.background_executor().timer(DEBOUNCE).await;
            if counter.load(Ordering::Relaxed) != generation {
                return;
            }
            let results = cx
                .background_executor()
                .spawn(async move { search_files(&files, &matcher, &counter, generation) })
                .await;
            let Some(results) = results else {
                return;
            };
            this.update(cx, |this, cx| {
                if this.overlays.find_generation().load(Ordering::Relaxed) == generation {
                    this.overlays.set_find_outcome(FindOutcome::Done(results));
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    /// Flips one search option, reruns the query, and hands focus back to the
    /// query field the toggle took it from.
    fn toggle_find_option(
        &mut self,
        option: fn(&mut SearchOptions) -> &mut bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let flag = option(self.overlays.find_options_mut());
        *flag = !*flag;
        self.run_search(window, cx);
        if let Some(state) = self.overlays.find().cloned() {
            state.update(cx, |state, cx| state.focus(window, cx));
        }
    }

    /// Opens a result's file and selects the match. A file that is not open yet
    /// is selected once its tab is created.
    pub(super) fn on_open_match(
        &mut self,
        action: &OpenMatch,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let jump = Jump {
            line: action.line,
            start: action.start,
            end: action.end,
        };
        if let Some(ix) = self
            .tabs
            .iter()
            .position(|tab| tab.path.as_deref() == Some(action.path.as_path()))
        {
            self.activate_tab(ix, window, cx);
            self.select_match(ix, jump, window, cx);
            return;
        }
        self.tabs.set_pending_jump(action.path.clone(), jump);
        self.request_open(action.path.clone(), window, cx);
    }

    /// Selects `jump` in tab `ix`, scrolling it into view. Columns past the end
    /// of a line that changed since the search are clamped by the editor.
    pub(super) fn select_match(
        &mut self,
        ix: usize,
        jump: Jump,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(tab) = self.tabs.get(ix) else {
            return;
        };
        if tab.raster {
            return;
        }
        tab.editor.clone().update(cx, |state, cx| {
            let start = Position::new(jump.line, jump.start);
            let end = Position::new(jump.line, jump.end);
            // Moving the caret scrolls the line into view and focuses the editor.
            state.set_cursor_position(start, window, cx);
            let range =
                state.text().position_to_offset(&start)..state.text().position_to_offset(&end);
            state.set_selected_range(range, cx);
        });
    }

    pub(super) fn render_find_in_files(
        &self,
        state: &Entity<CommandState>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let entity = cx.entity();
        let icon_style = self.settings.values.ui.icon_style();
        let options = self.overlays.find_options();
        let file_count = self.overlays.find_files().len();

        let (summary, empty): (SharedString, SharedString) = match self.overlays.find_outcome() {
            FindOutcome::Idle => (
                format!("{file_count} files").into(),
                "Type to search the workspace".into(),
            ),
            FindOutcome::Searching => ("Searching…".into(), "Searching…".into()),
            FindOutcome::Done(results) if results.truncated => (
                format!("First {MAX_MATCHES} matches in {} files", results.files).into(),
                "No matches".into(),
            ),
            FindOutcome::Done(results) => (
                format!(
                    "{} matches in {} files",
                    results.matches.len(),
                    results.files
                )
                .into(),
                "No matches".into(),
            ),
            FindOutcome::Invalid(error) => (
                "Invalid regular expression".into(),
                // The regex crate's message spans several lines; the last
                // one names the problem.
                error
                    .lines()
                    .last()
                    .unwrap_or_default()
                    .trim()
                    .to_string()
                    .into(),
            ),
        };

        let mut command = Command::new(state)
            .placeholder("Find in files…")
            .filterable(false)
            .max_h(px(420.))
            .empty(move |_, _, cx| {
                div()
                    .p_3()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(empty.clone())
            })
            .footer({
                let entity = entity.clone();
                move |_, _, cx| find_footer(&entity, options, summary.clone(), cx)
            })
            .on_query({
                let entity = entity.clone();
                move |query, window, cx| {
                    let query = query.to_string();
                    entity.update(cx, |this, cx| {
                        this.overlays.set_find_query(query);
                        this.run_search(window, cx);
                    });
                }
            })
            .on_confirm({
                let entity = entity.clone();
                move |_, window, cx| {
                    entity.update(cx, |this, cx| this.close_find_in_files(window, cx));
                }
            })
            .on_cancel({
                let entity = entity.clone();
                move |window, cx| {
                    // Closing inside the widget's own Cancel dispatch would read
                    // its state mid-update; see `render_quick_open`.
                    let entity = entity.clone();
                    window.defer(cx, move |window, cx| {
                        entity.update(cx, |this, cx| this.close_find_in_files(window, cx));
                    });
                }
            });
        if let FindOutcome::Done(results) = self.overlays.find_outcome() {
            for found in &results.matches {
                command = command.item(self.find_item(found, icon_style));
            }
        }

        let backdrop = entity;
        div()
            .absolute()
            .inset_0()
            .occlude()
            .flex()
            .items_start()
            .justify_center()
            .pt(FIND_TOP)
            .bg(cx.theme().overlay)
            .on_mouse_down(MouseButton::Left, move |_, window, cx| {
                backdrop.update(cx, |this, cx| this.close_find_in_files(window, cx));
            })
            .child(
                div()
                    .w(FIND_WIDTH)
                    // Keep clicks on the panel from reaching the backdrop.
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .child(command),
            )
            .into_any_element()
    }

    /// A result row: the file icon, the matching line, and `path:line` muted on
    /// the right.
    fn find_item(&self, found: &SearchMatch, icon_style: IconStyle) -> CommandItem {
        let rel = match nearest_root(self.workspace.roots(), &found.path) {
            Some(root) => relative_display(&found.path, root),
            None => file_name(&found.path),
        };
        let location: SharedString = format!("{rel}:{}", found.line + 1).into();
        let preview: SharedString = found.preview.clone().into();
        let path = found.path.clone();

        CommandItem::new()
            .action(Box::new(OpenMatch {
                path: found.path.clone(),
                line: found.line,
                start: found.start,
                end: found.end,
            }))
            .child(move |_window, cx| {
                div()
                    .h_flex()
                    .flex_1()
                    .min_w_0()
                    .items_center()
                    .gap_2()
                    .child(file_type_icon(file_icon(&path), icon_style, cx))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .font_family(cx.theme().mono_font_family.clone())
                            .child(preview.clone()),
                    )
                    .child(
                        div()
                            .flex_none()
                            .max_w(px(260.))
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(location.clone()),
                    )
                    .into_any_element()
            })
    }
}

/// The option toggles and the result summary under the list.
fn find_footer(
    entity: &Entity<TinytextApp>,
    options: SearchOptions,
    summary: SharedString,
    cx: &App,
) -> AnyElement {
    let toggle = |id: &'static str,
                  label: &'static str,
                  tooltip: &'static str,
                  checked: bool,
                  option: fn(&mut SearchOptions) -> &mut bool| {
        let entity = entity.clone();
        Toggle::new(id)
            .label(label)
            .tooltip(tooltip)
            .checked(checked)
            .on_click(move |_, window, cx| {
                entity.update(cx, |this, cx| this.toggle_find_option(option, window, cx));
            })
    };

    div()
        .h_flex()
        .items_center()
        .gap_1()
        .px_2()
        .py_1()
        .border_t_1()
        .border_color(cx.theme().border)
        .child(toggle(
            "find-case",
            "Aa",
            "Match Case",
            options.case_sensitive,
            |o| &mut o.case_sensitive,
        ))
        .child(toggle(
            "find-word",
            "ab",
            "Match Whole Word",
            options.whole_word,
            |o| &mut o.whole_word,
        ))
        .child(toggle(
            "find-regex",
            ".*",
            "Use Regular Expression",
            options.regex,
            |o| &mut o.regex,
        ))
        .child(
            div()
                .ml_auto()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child(summary),
        )
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use gpui_kit::component::input::EditorState;
    use gpui_kit::test::TestWindowExt as _;
    use gpui_kit::{AppContext as _, Focusable as _, TestAppContext};

    use crate::OpenMatch;
    use crate::app::files::TextFormat;
    use crate::app::tabs::OpenTab;
    use crate::app::test_support::open_app;

    /// Cmd+Shift+F opens the overlay and Escape closes it, returning focus.
    #[gpui_kit::test]
    async fn shortcut_opens_and_escape_closes(cx: &mut TestAppContext) {
        let (handle, app) = open_app(cx);

        cx.update_window(handle, |_, window, cx| {
            let focus_handle = app.read(cx).focus_handle.clone();
            window.focus(&focus_handle, cx);
            window.render_frame(cx);

            window.press("cmd-shift-f", cx);
            window.render_frame(cx);
            assert!(
                app.read(cx).overlays.is_find_open(),
                "the overlay should open"
            );
            if let Some(find) = app.read(cx).overlays.find().cloned() {
                find.update(cx, |state, cx| state.focus(window, cx));
            }
            window.render_frame(cx);

            window.press("escape", cx);
            window.render_frame(cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            assert!(!app.read(cx).overlays.is_find_open(), "Escape closes it");
            assert_eq!(
                window.focused(cx).as_ref(),
                Some(&app.read(cx).focus_handle),
                "focus returns to what held it before"
            );
        })
        .unwrap();
    }

    /// Opening a result for a file already in a tab selects the match there.
    #[gpui_kit::test]
    async fn opening_a_match_selects_it(cx: &mut TestAppContext) {
        let (handle, app) = open_app(cx);
        let path = PathBuf::from("/tmp/tinytext-find-test.txt");

        cx.update_window(handle, |_, window, cx| {
            let editor =
                cx.new(|cx| EditorState::new(window, cx).default_value("one\ntwo needle\n"));
            // Pushed directly: `push_tab` would write the real session file.
            app.update(cx, |this, _| {
                this.tabs.push(OpenTab {
                    path: Some(path.clone()),
                    title: "find.txt".into(),
                    language: "Plain Text".into(),
                    language_override: false,
                    dirty: false,
                    disk_modified: None,
                    format: TextFormat::default(),
                    editor: editor.clone(),
                    preview: None,
                    raster: false,
                    _subscriptions: Vec::new(),
                });
            });
            app.update(cx, |this, cx| {
                let action = OpenMatch {
                    path: path.clone(),
                    line: 1,
                    start: 4,
                    end: 10,
                };
                this.on_open_match(&action, window, cx);
            });
            let state = editor.read(cx);
            assert_eq!(state.selected_range(), 8..14, "\"needle\" on line 2");
            assert!(state.focus_handle(cx).is_focused(window));
        })
        .unwrap();
    }
}
