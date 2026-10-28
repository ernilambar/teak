//! Find in Files: a panel docked above the status bar runs the search, and the
//! matches land in a read-only Find Results tab.
//!
//! The panel holds plain text fields, not a picker, so its keys (Enter, Tab,
//! Escape) are bound in its own `FindPanel` key context. The results tab is an
//! ordinary editor tab with `find_results` set: Enter or a double-click on a
//! match line opens it, and F4 / Shift+F4 step through matches from any tab.
//! The search itself runs on the background executor (see `search.rs`); a
//! newer search bumps the generation counter, which stops the older one.
//!
//! Replace All searches afresh, confirms with the counts, then changes open
//! files in their tabs (one undo step each) and writes closed files to disk,
//! skipping any changed since the search.

use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::time::SystemTime;

use gpui_kit::base::StyledExt as _;
use gpui_kit::component::{
    ActiveTheme as _, Sizable as _, WindowExt as _,
    button::{Button, ButtonVariant, ButtonVariants as _, Toggle},
    input::{Input, InputState, Position, RangeDecoration, RangeDecorationStyle, RopeExt as _},
    notification::Notification,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use regex::Regex;

use crate::paths::{collect_files, exclude_set, file_name, nearest_root, relative_display};
use crate::search::{
    ResultTarget, SearchOptions, SearchResults, build_matcher, parse_where, replace_matches,
    results_document, search_files, summary,
};
use crate::{
    CloseFindPanel, FindInFiles, FindNextField, FindPreviousField, NextResult, OpenResult,
    PreviousResult, ReplaceAll, RunFind,
};

use super::TeakApp;
use super::editor::changed_span;
use super::files::{TextFormat, modified_time};
use super::overlays::{FindFields, FindStatus, FindText};
use super::tabs::{FindResultsTab, Jump};

/// Hard cap on searched files, matching Quick Open's index.
const MAX_FIND_FILES: usize = 20_000;
/// Title of the reusable results tab.
const RESULTS_TITLE: &str = "Find Results";
/// Width of the field labels, so the rows line up.
const LABEL_WIDTH: Pixels = px(52.);

impl TeakApp {
    /// Shows the Find panel with the query field focused, prefilled with the
    /// editor's selection when it is a single line.
    pub(super) fn on_find_in_files(
        &mut self,
        _: &FindInFiles,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let selection = self.single_line_selection(cx);
        if self.overlays.find().is_none() {
            let FindText {
                query,
                scope,
                replace,
            } = self.overlays.find_text().clone();
            let query = cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder("Find")
                    .default_value(query)
            });
            let scope = cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder("All files · e.g. src/**, -**/*.min.js")
                    .default_value(scope)
            });
            let replace = cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder("Replace")
                    .default_value(replace)
            });
            self.overlays.set_find(
                FindFields {
                    query,
                    scope,
                    replace,
                },
                window.focused(cx),
            );
        }
        let Some(query) = self.overlays.find().map(|fields| fields.query.clone()) else {
            return;
        };
        cx.notify();
        // The field only exists in the tree after the next render.
        window.defer(cx, move |window, cx| {
            query.update(cx, |state, cx| {
                if let Some(selection) = selection {
                    state.set_value(selection, window, cx);
                }
                state.focus(window, cx);
                state.select_all(window, cx);
            });
        });
    }

    /// The active editor's selection, if it is non-empty and on one line.
    fn single_line_selection(&self, cx: &App) -> Option<String> {
        let tab = self.active()?;
        if tab.raster || tab.find_results.is_some() {
            return None;
        }
        let state = tab.document.editor.read(cx);
        let range = state.selected_range();
        let text = state.text().slice(range).to_string();
        (!text.is_empty() && !text.contains('\n')).then_some(text)
    }

    /// Hides the Find panel, keeping its text, and hands focus back when it was
    /// inside the panel.
    pub(super) fn on_close_find_panel(
        &mut self,
        _: &CloseFindPanel,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.remember_find_text(cx);
        let Some((fields, return_focus)) = self.overlays.take_find() else {
            return;
        };
        let inside = [&fields.query, &fields.scope, &fields.replace]
            .iter()
            .any(|field| field.read(cx).focus_handle(cx).is_focused(window));
        if inside {
            match return_focus {
                Some(previous) => window.focus(&previous, cx),
                None => window.focus(&self.focus_handle, cx),
            }
        }
        self.save_session(cx);
        cx.notify();
    }

    /// Copies the panel's field text into the state that outlives the panel.
    fn remember_find_text(&mut self, cx: &App) {
        if let Some(fields) = self.overlays.find() {
            let text = FindText {
                query: fields.query.read(cx).value().to_string(),
                scope: fields.scope.read(cx).value().to_string(),
                replace: fields.replace.read(cx).value().to_string(),
            };
            self.overlays.set_find_text(text);
        }
    }

    /// Tab and Shift+Tab move through Find, Where and Replace.
    pub(super) fn on_find_next_field(
        &mut self,
        _: &FindNextField,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.cycle_find_field(1, window, cx);
    }

    pub(super) fn on_find_previous_field(
        &mut self,
        _: &FindPreviousField,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.cycle_find_field(-1, window, cx);
    }

    fn cycle_find_field(&mut self, delta: isize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(fields) = self.overlays.find() else {
            return;
        };
        let order = [&fields.query, &fields.scope, &fields.replace];
        let current = order
            .iter()
            .position(|field| field.read(cx).focus_handle(cx).is_focused(window))
            .unwrap_or(0);
        let next = (current as isize + delta).rem_euclid(order.len() as isize) as usize;
        let target = order[next].clone();
        target.update(cx, |state, cx| {
            state.focus(window, cx);
            state.select_all(window, cx);
        });
    }

    pub(super) fn on_run_find(&mut self, _: &RunFind, window: &mut Window, cx: &mut Context<Self>) {
        self.run_find(After::Show, window, cx);
    }

    pub(super) fn on_replace_all(
        &mut self,
        _: &ReplaceAll,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.run_find(After::ConfirmReplace, window, cx);
    }

    /// Searches the workspace for the panel's query and shows the matches in
    /// the Find Results tab, then, for Replace All, asks to replace them.
    fn run_find(&mut self, after: After, window: &mut Window, cx: &mut Context<Self>) {
        self.remember_find_text(cx);
        self.save_session(cx);
        let generation = self.overlays.next_find_generation();
        let FindText {
            query,
            scope,
            replace,
        } = self.overlays.find_text().clone();
        if query.is_empty() {
            self.overlays.set_find_status(FindStatus::Idle);
            cx.notify();
            return;
        }
        let options = self.overlays.find_options();
        let matcher = match build_matcher(&query, options) {
            Ok(matcher) => matcher,
            Err(error) => {
                // The regex crate's message spans several lines; the last one
                // names the problem.
                let reason = error.lines().last().unwrap_or_default().trim().to_string();
                self.overlays
                    .set_find_status(FindStatus::Invalid(format!("Invalid regex: {reason}")));
                cx.notify();
                return;
            }
        };
        let (include, mut exclude) = parse_where(&scope);
        exclude.extend(self.settings.values.search.exclude().iter().cloned());
        let globs = exclude_set(&include).and_then(|include| Ok((include, exclude_set(&exclude)?)));
        let (include, exclude) = match globs {
            Ok(globs) => globs,
            Err(error) => {
                self.overlays
                    .set_find_status(FindStatus::Invalid(format!("Invalid glob: {error}")));
                cx.notify();
                return;
            }
        };
        self.overlays.set_find_status(FindStatus::Searching);
        cx.notify();

        // With no workspace folders the open files stand in for the project.
        let roots = self.workspace.roots().to_vec();
        let fallback: Vec<PathBuf> = self
            .tabs
            .iter()
            .filter(|tab| !tab.raster)
            .filter_map(|tab| tab.document.path.clone())
            .collect();
        let counter = self.overlays.find_generation();
        let replace_with = matcher.clone();
        cx.spawn_in(window, async move |this, cx| {
            let results = cx
                .background_executor()
                .spawn(async move {
                    let files = if roots.is_empty() {
                        fallback
                    } else {
                        collect_files(&roots, MAX_FIND_FILES, &exclude, &include)
                    };
                    search_files(&files, &matcher, &counter, generation)
                })
                .await;
            let Some(results) = results else {
                return;
            };
            this.update_in(cx, |this, window, cx| {
                if this.overlays.find_generation().load(Ordering::Relaxed) != generation {
                    return;
                }
                let header = search_header(&query, &scope, options, results.searched);
                let roots = this.workspace.roots().to_vec();
                let doc = results_document(&header, &results, |path| display(&roots, path));
                this.overlays
                    .set_find_status(FindStatus::Done(summary(&results)));
                this.show_results(doc.text, doc.targets, doc.highlights, window, cx);
                if after == After::ConfirmReplace {
                    let replace = Replace {
                        matcher: replace_with,
                        replacement: replace,
                        expand: options.regex,
                    };
                    this.confirm_replace(results, replace, window, cx);
                }
            })
            .ok();
        })
        .detach();
    }

    /// Fills the Find Results tab, creating it on first use, and shows it.
    fn show_results(
        &mut self,
        text: String,
        targets: Vec<ResultTarget>,
        highlights: Vec<std::ops::Range<usize>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let decorations: Vec<RangeDecoration> = highlights
            .into_iter()
            .map(|range| RangeDecoration::new(range).with_style(RangeDecorationStyle::Fill))
            .collect();

        if let Some(ix) = self.results_tab() {
            let Some(tab) = self.tabs.get_mut(ix) else {
                return;
            };
            let editor = tab.document.editor.clone();
            // `set_value` emits no change event, so the tab stays clean.
            editor.update(cx, |state, cx| state.set_value(text, window, cx));
            if let Some(results) = tab.find_results.as_mut() {
                results.targets = targets;
                results.current = None;
                results.highlights.set(decorations, cx);
            }
            self.activate_tab(ix, window, cx);
            return;
        }

        self.add_document_tab(RESULTS_TITLE, "Plain Text", text, window, cx);
        let Some(ix) = self.tabs.active_index() else {
            return;
        };
        let Some(tab) = self.tabs.get_mut(ix) else {
            return;
        };
        // Read-only comes from the `Editor` element, which sets it each render.
        let highlights = tab.document.editor.update(cx, |state, cx| {
            state.create_range_decorations_collection(decorations, cx)
        });
        tab.find_results = Some(FindResultsTab {
            targets,
            highlights,
            current: None,
        });
        cx.notify();
    }

    /// The index of the Find Results tab, if one is open.
    fn results_tab(&self) -> Option<usize> {
        self.tabs.iter().position(|tab| tab.find_results.is_some())
    }

    /// Opens the match on the results line under the caret.
    pub(super) fn on_open_result(
        &mut self,
        _: &OpenResult,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_result_at_caret(window, cx);
    }

    /// Shared by Enter and double-click in the results tab.
    pub(super) fn open_result_at_caret(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(ix) = self.tabs.active_index() else {
            return;
        };
        let Some(tab) = self.tabs.get(ix) else {
            return;
        };
        let Some(results) = tab.find_results.as_ref() else {
            return;
        };
        let line = tab.document.editor.read(cx).cursor_position().line;
        let Some(found) = results
            .targets
            .iter()
            .position(|target| target.doc_line == line)
        else {
            return;
        };
        self.go_to_result(ix, found, window, cx);
    }

    pub(super) fn on_next_result(
        &mut self,
        _: &NextResult,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.step_result(1, window, cx);
    }

    pub(super) fn on_previous_result(
        &mut self,
        _: &PreviousResult,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.step_result(-1, window, cx);
    }

    /// Moves to the next or previous match, wrapping around, from any tab.
    fn step_result(&mut self, delta: isize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(ix) = self.results_tab() else {
            return;
        };
        let Some(results) = self.tabs.get(ix).and_then(|tab| tab.find_results.as_ref()) else {
            return;
        };
        let count = results.targets.len();
        if count == 0 {
            return;
        }
        let next = match results.current {
            Some(current) => (current as isize + delta).rem_euclid(count as isize) as usize,
            None if delta > 0 => 0,
            None => count - 1,
        };
        self.go_to_result(ix, next, window, cx);
    }

    /// Marks result `found` current, puts the results caret on its line, and
    /// opens the match.
    fn go_to_result(
        &mut self,
        results_ix: usize,
        found: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(tab) = self.tabs.get_mut(results_ix) else {
            return;
        };
        let Some(results) = tab.find_results.as_mut() else {
            return;
        };
        let Some(target) = results.targets.get(found).cloned() else {
            return;
        };
        results.current = Some(found);
        // Without focusing the results tab, so the opened file keeps focus.
        tab.document.editor.update(cx, |state, cx| {
            let offset = state
                .text()
                .position_to_offset(&Position::new(target.doc_line, 0));
            state.set_selected_range(offset..offset, cx);
        });
        let jump = Jump {
            line: target.found.line,
            start: target.found.start,
            end: target.found.end,
        };
        self.open_match(target.path, jump, window, cx);
    }

    /// Opens `path` and selects `jump`. A file that is not open yet is
    /// selected once its tab is created.
    pub(super) fn open_match(
        &mut self,
        path: PathBuf,
        jump: Jump,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(ix) = self
            .tabs
            .iter()
            .position(|tab| tab.document.path.as_deref() == Some(path.as_path()))
        {
            self.activate_tab(ix, window, cx);
            self.select_match(ix, jump, window, cx);
            return;
        }
        self.tabs.set_pending_jump(path.clone(), jump);
        self.request_open(path, window, cx);
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
        tab.document.editor.clone().update(cx, |state, cx| {
            let start = Position::new(jump.line, jump.start);
            let end = Position::new(jump.line, jump.end);
            // Moving the caret scrolls the line into view and focuses the editor.
            state.set_cursor_position(start, window, cx);
            let range =
                state.text().position_to_offset(&start)..state.text().position_to_offset(&end);
            state.set_selected_range(range, cx);
        });
    }

    /// Asks before replacing what a fresh search found. A search cut short by
    /// the match cap is refused: replacing only part of it would be worse than
    /// nothing.
    fn confirm_replace(
        &mut self,
        results: SearchResults,
        replace: Replace,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if results.truncated {
            self.overlays.set_find_status(FindStatus::Invalid(
                "Too many matches to replace at once; narrow the search with Where".into(),
            ));
            cx.notify();
            return;
        }
        let matches = results.match_count();
        if matches == 0 {
            return;
        }
        let description = format!(
            "Replace {} with \"{}\"? Open files change in their tabs, where Undo reverts them. \
             Closed files are saved to disk.",
            summary(&results),
            replace.replacement
        );
        let entity = cx.entity();
        let pending = std::rc::Rc::new(std::cell::RefCell::new(Some((results, replace))));
        window.open_alert_dialog(cx, move |alert, _window, _cx| {
            let entity = entity.clone();
            let pending = pending.clone();
            alert
                .confirm()
                .title("Replace All")
                .description(description.clone())
                .ok_text("Replace")
                .ok_variant(ButtonVariant::Danger)
                .cancel_text("Cancel")
                .on_ok(move |_, window, cx| {
                    if let Some((results, replace)) = pending.borrow_mut().take() {
                        entity.update(cx, |this, cx| {
                            this.apply_replace(results, replace, window, cx)
                        });
                    }
                    true
                })
        });
    }

    /// Replaces in open tabs right away, then in closed files off the main
    /// thread, and reports what changed and what was skipped.
    fn apply_replace(
        &mut self,
        results: SearchResults,
        replace: Replace,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let mut done = Outcome::default();
        let mut closed: Vec<(PathBuf, Option<SystemTime>)> = Vec::new();
        for file in results.files {
            let open = self
                .tabs
                .iter()
                .find(|tab| {
                    tab.document.path.as_deref() == Some(file.path.as_path()) && !tab.raster
                })
                .map(|tab| tab.document.editor.clone());
            let Some(editor) = open else {
                closed.push((file.path, file.modified));
                continue;
            };
            // The buffer is the truth for an open file, saved or not.
            let text = editor.read(cx).text().to_string();
            let (new, count) = replace.apply(&text);
            if count == 0 {
                continue;
            }
            let (range, replacement) = changed_span(&text, &new);
            editor.update(cx, |state, cx| {
                state.set_selected_range(range, cx);
                state.replace(replacement, window, cx);
            });
            done.matches += count;
            done.files += 1;
        }

        cx.spawn_in(window, async move |this, cx| {
            let closed_done = cx
                .background_executor()
                .spawn(async move { replace_closed_files(closed, &replace) })
                .await;
            this.update_in(cx, |this, window, cx| {
                done.merge(closed_done);
                this.report_replace(&done, window, cx);
                // Refresh the results so they show the files as they are now.
                this.run_find(After::Show, window, cx);
            })
            .ok();
        })
        .detach();
    }

    fn report_replace(&self, done: &Outcome, window: &mut Window, cx: &mut Context<Self>) {
        let noun = |count: usize, one: &str, many: &str| {
            format!("{count} {}", if count == 1 { one } else { many })
        };
        window.push_notification(
            Notification::success(format!(
                "Replaced {} in {}",
                noun(done.matches, "match", "matches"),
                noun(done.files, "file", "files")
            )),
            cx,
        );
        let skipped = |names: &[String], why: &str| {
            format!("Skipped {} {why}: {}", names.len(), names.join(", "))
        };
        if !done.changed.is_empty() {
            window.push_notification(
                Notification::warning(skipped(&done.changed, "changed since the search")),
                cx,
            );
        }
        if !done.not_utf8.is_empty() {
            window.push_notification(
                Notification::warning(skipped(&done.not_utf8, "not valid UTF-8")),
                cx,
            );
        }
        for error in &done.failed {
            window.push_notification(Notification::error(error.clone()), cx);
        }
    }

    /// Flips one search option and hands focus back to the query field.
    fn toggle_find_option(
        &mut self,
        option: fn(&mut SearchOptions) -> &mut bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let flag = option(self.overlays.find_options_mut());
        *flag = !*flag;
        self.save_session(cx);
        if let Some(fields) = self.overlays.find() {
            fields.query.update(cx, |state, cx| state.focus(window, cx));
        }
        cx.notify();
    }

    /// The panel docked above the status bar, while it is shown.
    pub(super) fn render_find_panel(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let fields = self.overlays.find()?;
        let entity = cx.entity();
        let options = self.overlays.find_options();
        let (status, status_color) = match self.overlays.find_status() {
            FindStatus::Idle => (String::new(), cx.theme().muted_foreground),
            FindStatus::Searching => ("Searching…".to_string(), cx.theme().muted_foreground),
            FindStatus::Done(summary) => (summary.clone(), cx.theme().muted_foreground),
            FindStatus::Invalid(reason) => (reason.clone(), cx.theme().danger),
        };

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
        let muted = cx.theme().muted_foreground;
        let label = |text: &'static str| {
            div()
                .w(LABEL_WIDTH)
                .flex_none()
                .text_xs()
                .text_color(muted)
                .child(text)
        };
        let find_button = {
            let entity = entity.clone();
            Button::new("find-run")
                .small()
                .primary()
                .label("Find")
                .on_click(move |_, window, cx| {
                    entity.update(cx, |this, cx| this.run_find(After::Show, window, cx));
                })
        };
        let replace_button = {
            let entity = entity.clone();
            Button::new("find-replace")
                .small()
                .label("Replace All…")
                .on_click(move |_, window, cx| {
                    entity.update(cx, |this, cx| {
                        this.run_find(After::ConfirmReplace, window, cx)
                    });
                })
        };

        Some(
            div()
                .key_context("FindPanel")
                .flex_none()
                .v_flex()
                .gap_1()
                .px_3()
                .py_2()
                .border_t_1()
                .border_color(cx.theme().border)
                .bg(cx.theme().background)
                .child(
                    div()
                        .h_flex()
                        .items_center()
                        .gap_2()
                        .child(label("Find"))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .child(Input::new(&fields.query).small()),
                        )
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
                        )),
                )
                .child(
                    div()
                        .h_flex()
                        .items_center()
                        .gap_2()
                        .child(label("Where"))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .child(Input::new(&fields.scope).small()),
                        )
                        .child(find_button),
                )
                .child(
                    div()
                        .h_flex()
                        .items_center()
                        .gap_2()
                        .child(label("Replace"))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .child(Input::new(&fields.replace).small()),
                        )
                        .child(replace_button),
                )
                .when(!status.is_empty(), |panel| {
                    panel.child(
                        div()
                            .pl(LABEL_WIDTH + px(8.))
                            .text_xs()
                            .text_color(status_color)
                            .child(status),
                    )
                })
                .into_any_element(),
        )
    }
}

/// What happens once a search finishes.
#[derive(Clone, Copy, PartialEq)]
enum After {
    Show,
    ConfirmReplace,
}

/// A Replace All request: the search's matcher and what replaces each match.
struct Replace {
    matcher: Regex,
    replacement: String,
    /// Expand `$1` and `${name}`; on in regex mode only.
    expand: bool,
}

impl Replace {
    fn apply(&self, text: &str) -> (String, usize) {
        replace_matches(text, &self.matcher, &self.replacement, self.expand)
    }
}

/// What a Replace All changed and skipped. Skipped files are listed by name.
#[derive(Default)]
struct Outcome {
    matches: usize,
    files: usize,
    changed: Vec<String>,
    not_utf8: Vec<String>,
    failed: Vec<String>,
}

impl Outcome {
    fn merge(&mut self, other: Outcome) {
        self.matches += other.matches;
        self.files += other.files;
        self.changed.extend(other.changed);
        self.not_utf8.extend(other.not_utf8);
        self.failed.extend(other.failed);
    }
}

/// Replaces in files that are not open, keeping each file's BOM and line
/// endings. A file whose modification time moved since the search is left
/// alone, so a change made elsewhere is never overwritten.
fn replace_closed_files(files: Vec<(PathBuf, Option<SystemTime>)>, replace: &Replace) -> Outcome {
    let mut done = Outcome::default();
    for (path, searched) in files {
        let name = file_name(&path);
        if searched.is_none() || modified_time(&path) != searched {
            done.changed.push(name);
            continue;
        }
        let decoded = std::fs::read(&path).and_then(|bytes| TextFormat::decode(&bytes));
        let (text, format) = match decoded {
            Ok(decoded) => decoded,
            Err(error) => {
                done.failed.push(format!("Could not read {name}: {error}"));
                continue;
            }
        };
        // Writing back a lossy decode would replace its bad bytes for good.
        if format.lossy {
            done.not_utf8.push(name);
            continue;
        }
        let (new, count) = replace.apply(&text);
        if count == 0 {
            continue;
        }
        match std::fs::write(&path, format.encode(&new)) {
            Ok(()) => {
                done.matches += count;
                done.files += 1;
            }
            Err(error) => done.failed.push(format!("Could not save {name}: {error}")),
        }
    }
    done
}

/// The first line of the results: what was searched, and how.
fn search_header(query: &str, scope: &str, options: SearchOptions, searched: usize) -> String {
    let mut flags = Vec::new();
    if options.case_sensitive {
        flags.push("case");
    }
    if options.whole_word {
        flags.push("whole word");
    }
    if options.regex {
        flags.push("regex");
    }
    let files = if searched == 1 { "file" } else { "files" };
    let mut header = format!("Searching {searched} {files} for \"{query}\"");
    if !scope.trim().is_empty() {
        header.push_str(&format!(" in {}", scope.trim()));
    }
    if !flags.is_empty() {
        header.push_str(&format!(" ({})", flags.join(", ")));
    }
    header
}

/// A file's name in the results: relative to its workspace folder, or the full
/// path when it is outside every folder.
fn display(roots: &[PathBuf], path: &Path) -> String {
    match nearest_root(roots, path) {
        Some(root) => relative_display(path, root),
        None => path.display().to_string(),
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use gpui_kit::component::input::EditorState;
    use gpui_kit::test::TestWindowExt as _;
    use gpui_kit::{AppContext as _, Focusable as _, TestAppContext};

    use super::{Replace, search_header};
    use crate::app::document::Document;
    use crate::app::files::TextFormat;
    use crate::app::tabs::{Jump, OpenTab};
    use crate::app::test_support::open_app;
    use crate::search::SearchOptions;
    use crate::search::{build_matcher, search_files};

    /// The find-in-files chord shows the panel with the query field focused,
    /// Tab moves to Where, and Escape hides the panel, returning focus to where
    /// it was.
    #[gpui_kit::test]
    async fn shortcut_opens_tab_cycles_and_escape_closes(cx: &mut TestAppContext) {
        let (handle, app) = open_app(cx);

        cx.update_window(handle, |_, window, cx| {
            let focus_handle = app.read(cx).focus_handle.clone();
            window.focus(&focus_handle, cx);
            window.render_frame(cx);
            window.press("secondary-shift-f", cx);
            window.render_frame(cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            let query = app.read(cx).overlays.find().map(|f| f.query.clone());
            let query = query.expect("the panel should open");
            assert!(query.read(cx).focus_handle(cx).is_focused(window));

            window.press("tab", cx);
            let scope = app.read(cx).overlays.find().map(|f| f.scope.clone());
            let scope = scope.unwrap();
            assert!(
                scope.read(cx).focus_handle(cx).is_focused(window),
                "Tab moves to Where"
            );

            window.press("escape", cx);
            window.render_frame(cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            assert!(app.read(cx).overlays.find().is_none(), "Escape hides it");
            assert_eq!(
                window.focused(cx).as_ref(),
                Some(&app.read(cx).focus_handle),
                "focus returns to what held it before"
            );
        })
        .unwrap();
    }

    /// Enter in the panel searches the workspace into a read-only results tab,
    /// and F4 opens the first match with it selected.
    #[gpui_kit::test]
    async fn search_fills_results_and_f4_opens_the_match(cx: &mut TestAppContext) {
        let (handle, app) = open_app(cx);
        let root = crate::paths::support_dir().unwrap().join("find-e2e");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let file = root.join("a.txt");
        std::fs::write(&file, "one\ntwo needle\n").unwrap();

        cx.update_window(handle, |_, window, cx| {
            app.update(cx, |this, cx| this.add_root(root.clone(), window, cx));
            let focus_handle = app.read(cx).focus_handle.clone();
            window.focus(&focus_handle, cx);
            window.render_frame(cx);
            window.press("secondary-shift-f", cx);
            window.render_frame(cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            let query = app
                .read(cx)
                .overlays
                .find()
                .map(|f| f.query.clone())
                .unwrap();
            query.update(cx, |state, cx| state.set_value("needle", window, cx));
            window.press("enter", cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            let this = app.read(cx);
            let tab = this.tabs.active().expect("the results tab opens");
            let results = tab.find_results.as_ref().expect("it is the results tab");
            assert_eq!(results.targets.len(), 1);
            let text = tab.document.editor.read(cx).text().to_string();
            assert!(text.contains("a.txt:"), "{text}");
            assert!(text.contains("2: two needle"), "{text}");
            window.render_frame(cx);
            assert!(
                !app.read(cx)
                    .tabs
                    .active()
                    .unwrap()
                    .document
                    .editor
                    .read(cx)
                    .is_editable(),
                "results are read-only"
            );

            window.press("f4", cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, _, cx| {
            let this = app.read(cx);
            let tab = this.tabs.active().unwrap();
            assert_eq!(
                tab.document.path.as_deref(),
                Some(file.as_path()),
                "F4 opens the file"
            );
            assert_eq!(tab.document.editor.read(cx).selected_range(), 8..14);
        })
        .unwrap();
        std::fs::remove_dir_all(&root).unwrap();
    }

    /// Opening a match for a file already in a tab selects it there.
    #[gpui_kit::test]
    async fn opening_a_match_selects_it(cx: &mut TestAppContext) {
        let (handle, app) = open_app(cx);
        let path = PathBuf::from("/tmp/app-find-test.txt");

        cx.update_window(handle, |_, window, cx| {
            let editor =
                cx.new(|cx| EditorState::new(window, cx).default_value("one\ntwo needle\n"));
            app.update(cx, |this, _| {
                this.tabs.push(OpenTab {
                    document: Document::new(
                        Some(path.clone()),
                        "find.txt".into(),
                        "Plain Text".into(),
                        TextFormat::default(),
                        editor.clone(),
                    ),
                    preview: None,
                    raster: false,
                    find_results: None,
                    diff: None,
                    transient: false,
                    image: Default::default(),
                    _subscriptions: Vec::new(),
                });
            });
            app.update(cx, |this, cx| {
                let jump = Jump {
                    line: 1,
                    start: 4,
                    end: 10,
                };
                this.open_match(path.clone(), jump, window, cx);
            });
            let state = editor.read(cx);
            assert_eq!(state.selected_range(), 8..14, "\"needle\" on line 2");
            assert!(state.focus_handle(cx).is_focused(window));
        })
        .unwrap();
    }

    /// Replace All changes closed files on disk keeping BOM and CRLF, changes
    /// an open file in its buffer as one undo step, and skips a file that
    /// changed since the search.
    #[gpui_kit::test]
    async fn replace_all_changes_disk_and_buffers_and_skips_stale(cx: &mut TestAppContext) {
        let (handle, app) = open_app(cx);
        let root = crate::paths::support_dir().unwrap().join("replace-e2e");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let closed = root.join("closed.txt");
        let stale = root.join("stale.txt");
        let open = root.join("open.txt");
        std::fs::write(&closed, b"\xEF\xBB\xBFfoo bar\r\nfoo\r\n").unwrap();
        std::fs::write(&stale, "foo\n").unwrap();
        std::fs::write(&open, "foo on disk\n").unwrap();

        let matcher = build_matcher("foo", SearchOptions::default()).unwrap();
        let files = vec![closed.clone(), stale.clone(), open.clone()];
        let mut results =
            search_files(&files, &matcher, &std::sync::atomic::AtomicU64::new(0), 0).unwrap();
        // Pretend `stale.txt` changed after the search.
        results.files[1].modified = Some(std::time::UNIX_EPOCH);

        cx.update_window(handle, |_, window, cx| {
            // The open tab's buffer differs from disk; the buffer must win.
            let editor = cx.new(|cx| EditorState::new(window, cx).default_value("foo in buffer\n"));
            app.update(cx, |this, _| {
                this.tabs.push(OpenTab {
                    document: Document::new(
                        Some(open.clone()),
                        "open.txt".into(),
                        "Plain Text".into(),
                        TextFormat::default(),
                        editor.clone(),
                    ),
                    preview: None,
                    raster: false,
                    find_results: None,
                    diff: None,
                    transient: false,
                    image: Default::default(),
                    _subscriptions: Vec::new(),
                });
            });
            let replace = Replace {
                matcher: matcher.clone(),
                replacement: "baz".into(),
                expand: false,
            };
            app.update(cx, |this, cx| {
                this.apply_replace(results, replace, window, cx)
            });
        })
        .unwrap();
        cx.run_until_parked();

        assert_eq!(
            std::fs::read(&closed).unwrap(),
            b"\xEF\xBB\xBFbaz bar\r\nbaz\r\n",
            "BOM and CRLF survive"
        );
        assert_eq!(
            std::fs::read_to_string(&stale).unwrap(),
            "foo\n",
            "stale file untouched"
        );
        assert_eq!(
            std::fs::read_to_string(&open).unwrap(),
            "foo on disk\n",
            "an open file is not written"
        );
        cx.update_window(handle, |_, window, cx| {
            let editor = app
                .read(cx)
                .tabs
                .iter()
                .find(|tab| tab.document.path.as_deref() == Some(open.as_path()))
                .map(|tab| tab.document.editor.clone())
                .unwrap();
            assert_eq!(editor.read(cx).text().to_string(), "baz in buffer\n");
            app.update(cx, |this, cx| {
                let ix = this
                    .tabs
                    .iter()
                    .position(|tab| tab.document.editor == editor)
                    .unwrap();
                this.activate_tab(ix, window, cx);
            });
            window.render_frame(cx);
            window.press("secondary-z", cx);
            assert_eq!(
                editor.read(cx).text().to_string(),
                "foo in buffer\n",
                "one Undo reverts the replace"
            );
        })
        .unwrap();
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn header_names_scope_and_flags() {
        let options = SearchOptions {
            case_sensitive: true,
            regex: true,
            ..Default::default()
        };
        assert_eq!(
            search_header("a.b", " src/** ", options, 12),
            "Searching 12 files for \"a.b\" in src/** (case, regex)"
        );
        assert_eq!(
            search_header("x", "", SearchOptions::default(), 1),
            "Searching 1 file for \"x\""
        );
    }
}
