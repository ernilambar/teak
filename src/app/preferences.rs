//! The Settings overlay, built from gpui-kit's `Settings` component.
//!
//! Field values are read from a snapshot of the app's settings taken at
//! render time rather than read back through the entity: the component invokes
//! the value closures while the app's own element tree is being built, and
//! reading the entity there would re-enter its borrow. The set closures capture
//! a `WeakEntity` and run on interaction, when the entity is free to update.
//!
//! Word wrap, whitespace and the other editor options need a `Window` to push
//! their new state onto the editors that are already open, so they are built
//! with a custom field whose render closure is handed one — a switch, or a
//! number input for the cursor scroll margin. Everything else persists through
//! the window-free setters and reports a write failure in an inline banner.

use std::rc::Rc;

use gpui_kit::base::StyledExt as _;
use gpui_kit::component::{
    ActiveTheme as _, Sizable as _, ThemeMode,
    button::{Button, ButtonVariants as _},
    input::{InputEvent, InputState, NumberInput, TabSize},
    setting::{NumberFieldOptions, SettingField, SettingGroup, SettingItem, SettingPage, Settings},
    switch::Switch,
};
use gpui_kit::*;

use crate::settings::{
    DEFAULT_CURSOR_SURROUNDING_LINES, DEFAULT_DARK_SCHEME, DEFAULT_FONT_WEIGHT, DEFAULT_ICON_STYLE,
    DEFAULT_LIGHT_SCHEME, DEFAULT_SIDEBAR_POSITION, DEFAULT_TAB_SIZE, DEFAULT_THEME,
    EditorSettings, MAX_FONT_SIZE, MAX_UI_FONT_SIZE, MIN_FONT_SIZE, MIN_UI_FONT_SIZE,
    save_color_scheme_dark, save_color_scheme_light, save_cursor_surrounding_lines,
    save_font_family, save_font_size, save_font_weight, save_hard_tabs, save_icon_color,
    save_show_whitespace, save_sidebar_position, save_soft_wrap, save_tab_icons, save_tab_size,
    save_theme, save_ui_font_family, save_ui_font_size,
};

use super::TeakApp;

/// A boolean switch's change handler, given the window it needs to apply.
type WindowSwitchSetter = Rc<dyn Fn(&mut TeakApp, bool, &mut Window, &mut Context<TeakApp>)>;

/// A boolean switch that needs the window to apply its change immediately. The
/// typed `SettingField::switch` callbacks only receive `&mut App`; a custom
/// field gets a `&mut Window`, which is threaded into `apply`.
fn window_switch(
    id: &'static str,
    checked: bool,
    weak: WeakEntity<TeakApp>,
    apply: WindowSwitchSetter,
) -> SettingField<SharedString> {
    SettingField::render(move |options, _window, _cx| {
        let weak = weak.clone();
        let apply = apply.clone();
        Switch::new(id)
            .checked(checked)
            .with_size(options.size())
            .on_click(move |checked: &bool, window: &mut Window, cx: &mut App| {
                let value = *checked;
                weak.update(cx, |this, cx| apply(this, value, window, cx))
                    .ok();
            })
            .into_any_element()
    })
}

/// A number field's change handler, given the window it needs to apply.
type WindowNumberSetter = Rc<dyn Fn(&mut TeakApp, usize, &mut Window, &mut Context<TeakApp>)>;

/// State for [`window_number_input`], held in the window so the field's text and
/// caret survive re-renders of the overlay.
struct NumberFieldState {
    input: Entity<InputState>,
    committed: usize,
    _subscriptions: Vec<Subscription>,
}

/// A number input that needs the window to apply its change immediately,
/// [`window_switch`]'s numeric counterpart. The typed
/// `SettingField::number_input` hands its setter only `&mut App`, which is not
/// enough for the editor options the engine can only set through a `Window`.
fn window_number_input(
    id: &'static str,
    value: usize,
    options: NumberFieldOptions,
    weak: WeakEntity<TeakApp>,
    apply: WindowNumberSetter,
) -> SettingField<SharedString> {
    SettingField::render(move |render_options, window, cx| {
        let init_bounds = options.clone();
        let live_bounds = options.clone();
        let weak = weak.clone();
        let apply = apply.clone();
        let state = window.use_keyed_state(id, cx, move |window, cx| {
            let input = cx.new(|cx| {
                InputState::new(window, cx)
                    .default_value(value.to_string())
                    .step(init_bounds.step)
                    .min(init_bounds.min)
                    .max(init_bounds.max)
            });
            let subscriptions = vec![cx.subscribe_in(
                &input,
                window,
                move |state: &mut NumberFieldState, input, event: &InputEvent, window, cx| {
                    if !matches!(event, InputEvent::Change) {
                        return;
                    }
                    // Unparsable intermediates ("", "-") are left alone so the
                    // next keystroke can complete them; out-of-range text is
                    // clamped rather than rejected.
                    let Ok(parsed) = input.read(cx).value().trim().parse::<usize>() else {
                        return;
                    };
                    let lines = parsed.clamp(live_bounds.min as usize, live_bounds.max as usize);
                    if state.committed == lines {
                        return;
                    }
                    state.committed = lines;
                    weak.update(cx, |this, cx| apply(this, lines, window, cx))
                        .ok();
                },
            )];
            NumberFieldState {
                input,
                committed: value,
                _subscriptions: subscriptions,
            }
        });

        // Reflect a value that changed while the overlay was closed.
        state.update(cx, |state, cx| {
            if state.committed != value {
                state.committed = value;
                state.input.update(cx, |input, cx| {
                    input.set_value(SharedString::from(value.to_string()), window, cx);
                });
            }
        });

        NumberInput::new(&state.read(cx).input)
            .with_size(render_options.size())
            .w_32()
            .into_any_element()
    })
}

/// The user's preferences together with the Settings overlay's own state.
///
/// `values` is `pub(super)` because it is the parsed `settings.json` value
/// object: five files read it, and there is no invariant a wrapper could
/// protect. `open` and `error` are private because they have to agree — opening
/// the overlay discards an error left over from a previous write.
pub(super) struct SettingsState {
    pub(super) values: crate::settings::Settings,
    open: bool,
    error: Option<SharedString>,
}

impl SettingsState {
    pub(super) fn new(values: crate::settings::Settings) -> Self {
        Self {
            values,
            open: false,
            error: None,
        }
    }

    pub(super) fn is_open(&self) -> bool {
        self.open
    }

    pub(super) fn set_open(&mut self, open: bool) {
        self.open = open;
    }

    /// Shows or hides the overlay, dropping any error from a previous write so a
    /// stale message cannot reappear when it is reopened.
    pub(super) fn toggle_open(&mut self) {
        self.open = !self.open;
        self.error = None;
    }

    pub(super) fn error(&self) -> Option<&SharedString> {
        self.error.as_ref()
    }

    pub(super) fn set_error(&mut self, error: Option<SharedString>) {
        self.error = error;
    }
}

impl TeakApp {
    /// Builds the full-window settings overlay.
    pub(super) fn render_settings(&self, cx: &mut Context<Self>) -> AnyElement {
        let weak = cx.entity().downgrade();
        let mono_family = cx.theme().mono_font_family.to_string();
        let mono_size = cx.theme().mono_font_size.as_f32();

        let editor = &self.settings.values.editor;
        let ui = &self.settings.values.ui;
        let font_family = editor
            .font_family
            .clone()
            .unwrap_or_else(|| mono_family.clone());
        let font_size = editor.font_size().unwrap_or(mono_size) as f64;
        let font_weight = editor.font_weight().unwrap_or(DEFAULT_FONT_WEIGHT) as f64;
        let tab_size = editor.tab_size() as f64;
        let hard_tabs = editor.hard_tabs();
        let soft_wrap = editor.soft_wrap();
        let show_whitespace = editor.show_whitespace();
        let theme_value = ui.theme().to_string();
        let ui_font_family = ui
            .font_family
            .clone()
            .unwrap_or_else(|| cx.theme().font_family.to_string());
        let ui_font_size = ui
            .font_size()
            .unwrap_or_else(|| cx.theme().font_size.as_f32()) as f64;
        let ui_font_family_is_set = ui.font_family.is_some();
        let ui_font_size_is_set = ui.font_size.is_some();
        let tab_icons = ui.tab_icons();
        let icon_color: SharedString = ui.icon_color().into();
        let light_schemes = crate::theme::schemes(ThemeMode::Light, cx);
        let dark_schemes = crate::theme::schemes(ThemeMode::Dark, cx);

        let family_weak = weak.clone();
        let family_value = font_family.clone();
        let font_family_item = SettingItem::new(
            "Family",
            SettingField::input(
                move |_cx| SharedString::from(family_value.clone()),
                move |value, cx| {
                    family_weak
                        .update(cx, |this, cx| {
                            this.set_editor_font_family(value.to_string(), cx)
                        })
                        .ok();
                },
            )
            .default_value(SharedString::from(mono_family.clone())),
        )
        .description("Name of an installed monospace font.")
        .layout(Axis::Vertical);

        let size_weak = weak.clone();
        let font_size_item = SettingItem::new(
            "Size",
            SettingField::number_input(
                NumberFieldOptions {
                    min: MIN_FONT_SIZE as f64,
                    max: MAX_FONT_SIZE as f64,
                    step: 1.0,
                },
                move |_cx| font_size,
                move |value, cx| {
                    size_weak
                        .update(cx, |this, cx| this.set_editor_font_size(value as f32, cx))
                        .ok();
                },
            )
            .default_value(mono_size as f64),
        );

        let weight_weak = weak.clone();
        let font_weight_item = SettingItem::new(
            "Weight",
            SettingField::number_input(
                NumberFieldOptions {
                    min: 100.0,
                    max: 900.0,
                    step: 100.0,
                },
                move |_cx| font_weight,
                move |value, cx| {
                    weight_weak
                        .update(cx, |this, cx| this.set_editor_font_weight(value as f32, cx))
                        .ok();
                },
            )
            .default_value(DEFAULT_FONT_WEIGHT as f64),
        );

        let tab_size_weak = weak.clone();
        let tab_size_item = SettingItem::new(
            "Tab Size",
            SettingField::number_input(
                NumberFieldOptions {
                    min: 1.0,
                    max: 16.0,
                    step: 1.0,
                },
                move |_cx| tab_size,
                move |value, cx| {
                    tab_size_weak
                        .update(cx, |this, cx| this.set_editor_tab_size(value as usize, cx))
                        .ok();
                },
            )
            .default_value(DEFAULT_TAB_SIZE as f64),
        )
        .description("Number of spaces a tab is rendered as.");

        let hard_tabs_weak = weak.clone();
        let hard_tabs_item = SettingItem::new(
            "Hard Tabs",
            SettingField::switch(
                move |_cx| hard_tabs,
                move |value, cx| {
                    hard_tabs_weak
                        .update(cx, |this, cx| this.set_editor_hard_tabs(value, cx))
                        .ok();
                },
            )
            .default_value(false),
        )
        .description("Indent with tab characters instead of spaces.");

        let soft_wrap_item = SettingItem::new(
            "Word Wrap",
            window_switch(
                "soft-wrap-switch",
                soft_wrap,
                weak.clone(),
                Rc::new(|this, value, window, cx| this.set_editor_soft_wrap(value, window, cx)),
            ),
        )
        .description("Wrap long lines at the editor edge instead of scrolling.");

        let whitespace_item = SettingItem::new(
            "Show Whitespace",
            window_switch(
                "show-whitespace-switch",
                show_whitespace,
                weak.clone(),
                Rc::new(|this, value, window, cx| {
                    this.set_editor_show_whitespace(value, window, cx)
                }),
            ),
        )
        .description("Render spaces and tabs as visible characters.");

        let flag_item =
            |label: &'static str,
             description: &'static str,
             id: &'static str,
             key: &'static str,
             checked: bool,
             field: fn(&mut EditorSettings) -> &mut Option<bool>| {
                SettingItem::new(
                    label,
                    window_switch(
                        id,
                        checked,
                        weak.clone(),
                        Rc::new(move |this, value, window, cx| {
                            this.set_editor_flag(key, field, value, window, cx)
                        }),
                    ),
                )
                .description(description)
            };
        let line_numbers_item = flag_item(
            "Line Numbers",
            "Show line numbers in the gutter.",
            "line-numbers-switch",
            "line_numbers",
            editor.line_numbers(),
            |e| &mut e.line_numbers,
        );
        let indent_guides_item = flag_item(
            "Indent Guides",
            "Draw vertical lines at each indentation level.",
            "indent-guides-switch",
            "indent_guides",
            editor.indent_guides(),
            |e| &mut e.indent_guides,
        );
        let folding_item = flag_item(
            "Code Folding",
            "Show fold controls in the gutter.",
            "folding-switch",
            "folding",
            editor.folding(),
            |e| &mut e.folding,
        );
        let indent_wrapped_item = flag_item(
            "Indent Wrapped Lines",
            "Align wrapped lines with the start of the line.",
            "indent-wrapped-lines-switch",
            "indent_wrapped_lines",
            editor.indent_wrapped_lines(),
            |e| &mut e.indent_wrapped_lines,
        );
        let scroll_beyond_item = flag_item(
            "Scroll Past Last Line",
            "Allow scrolling the last line up the editor.",
            "scroll-beyond-last-line-switch",
            "scroll_beyond_last_line",
            editor.scroll_beyond_last_line(),
            |e| &mut e.scroll_beyond_last_line,
        );
        let cursor_lines_is_set = editor.cursor_surrounding_lines.is_some();
        let cursor_lines = editor
            .cursor_surrounding_lines
            .unwrap_or(DEFAULT_CURSOR_SURROUNDING_LINES);
        let cursor_lines_reset_weak = weak.clone();
        let cursor_surrounding_lines_item = SettingItem::new(
            "Cursor Surrounding Lines",
            window_number_input(
                "cursor-surrounding-lines-field",
                cursor_lines,
                NumberFieldOptions {
                    min: 0.0,
                    max: 50.0,
                    step: 1.0,
                },
                weak.clone(),
                Rc::new(|this, lines, window, cx| {
                    this.set_editor_cursor_surrounding_lines(lines, window, cx)
                }),
            )
            .on_reset(
                move |_cx| cursor_lines_is_set,
                move |window, cx| {
                    cursor_lines_reset_weak
                        .update(cx, |this, cx| {
                            this.reset_cursor_surrounding_lines(window, cx)
                        })
                        .ok();
                },
            ),
        )
        .description("Lines kept above and below the cursor before the editor scrolls.");
        let auto_close_item = flag_item(
            "Auto Close Brackets",
            "Insert the closing bracket or quote as you type.",
            "auto-close-switch",
            "auto_close",
            editor.auto_close(),
            |e| &mut e.auto_close,
        );
        let trim_whitespace_item = flag_item(
            "Trim Trailing Whitespace",
            "Remove spaces and tabs at line ends on save.",
            "trim-whitespace-switch",
            "trim_trailing_whitespace",
            editor.trim_trailing_whitespace(),
            |e| &mut e.trim_trailing_whitespace,
        );
        let final_newline_item = flag_item(
            "Ensure Final Newline",
            "End the file with a newline on save.",
            "final-newline-switch",
            "ensure_final_newline",
            editor.ensure_final_newline(),
            |e| &mut e.ensure_final_newline,
        );
        let smart_indent_item = flag_item(
            "Smart Indent",
            "Indent new lines based on the language.",
            "smart-indent-switch",
            "smart_indent",
            editor.smart_indent(),
            |e| &mut e.smart_indent,
        );

        let theme_weak = weak.clone();
        let theme_value_owned = theme_value.clone();
        let theme_item = SettingItem::new(
            "Mode",
            SettingField::dropdown(
                crate::THEME_CHOICES
                    .iter()
                    .map(|(value, label)| (SharedString::from(*value), SharedString::from(*label)))
                    .collect::<Vec<_>>(),
                move |_cx| SharedString::from(theme_value_owned.clone()),
                move |value, cx| {
                    theme_weak
                        .update(cx, |this, cx| this.set_ui_theme(value.to_string(), cx))
                        .ok();
                },
            )
            .default_value(SharedString::from(DEFAULT_THEME)),
        )
        .description("Follow the system appearance, or force Light or Dark.");

        let light_scheme_weak = weak.clone();
        let light_scheme_value = SharedString::from(ui.light_scheme());
        let light_scheme_item = SettingItem::new(
            "Light Theme",
            SettingField::dropdown(
                light_schemes
                    .iter()
                    .map(|name| (name.clone(), name.clone()))
                    .collect::<Vec<_>>(),
                move |_cx| light_scheme_value.clone(),
                move |value, cx| {
                    light_scheme_weak
                        .update(cx, |this, cx| {
                            this.set_ui_color_scheme_light(value.to_string(), cx)
                        })
                        .ok();
                },
            )
            .default_value(SharedString::from(DEFAULT_LIGHT_SCHEME)),
        )
        .description("Color scheme used while the appearance is light.");

        let dark_scheme_weak = weak.clone();
        let dark_scheme_value = SharedString::from(ui.dark_scheme());
        let dark_scheme_item = SettingItem::new(
            "Dark Theme",
            SettingField::dropdown(
                dark_schemes
                    .iter()
                    .map(|name| (name.clone(), name.clone()))
                    .collect::<Vec<_>>(),
                move |_cx| dark_scheme_value.clone(),
                move |value, cx| {
                    dark_scheme_weak
                        .update(cx, |this, cx| {
                            this.set_ui_color_scheme_dark(value.to_string(), cx)
                        })
                        .ok();
                },
            )
            .default_value(SharedString::from(DEFAULT_DARK_SCHEME)),
        )
        .description("Color scheme used while the appearance is dark.");

        let tab_icons_weak = weak.clone();
        let tab_icons_item = SettingItem::new(
            "Tab Icons",
            SettingField::switch(
                move |_cx| tab_icons,
                move |value, cx| {
                    tab_icons_weak
                        .update(cx, |this, cx| this.set_ui_tab_icons(value, cx))
                        .ok();
                },
            )
            .default_value(true),
        )
        .description("Show a file-type icon on each tab.");

        let icon_color_weak = weak.clone();
        let icon_color_item = SettingItem::new(
            "File Icon Style",
            SettingField::dropdown(
                crate::ICON_STYLE_CHOICES
                    .iter()
                    .map(|(value, label)| (SharedString::from(*value), SharedString::from(*label)))
                    .collect::<Vec<_>>(),
                move |_cx| icon_color.clone(),
                move |value, cx| {
                    icon_color_weak
                        .update(cx, |this, cx| this.set_ui_icon_color(value.to_string(), cx))
                        .ok();
                },
            )
            .default_value(SharedString::from(DEFAULT_ICON_STYLE)),
        )
        .description("Colored file-type icons, or a single color that follows the theme.");

        let sidebar_position_weak = weak.clone();
        let sidebar_position: SharedString = ui.sidebar_position().into();
        let sidebar_position_item = SettingItem::new(
            "Sidebar Position",
            SettingField::dropdown(
                crate::SIDEBAR_POSITION_CHOICES
                    .iter()
                    .map(|(value, label)| (SharedString::from(*value), SharedString::from(*label)))
                    .collect::<Vec<_>>(),
                move |_cx| sidebar_position.clone(),
                move |value, cx| {
                    sidebar_position_weak
                        .update(cx, |this, cx| {
                            this.set_ui_sidebar_position(value.to_string(), cx)
                        })
                        .ok();
                },
            )
            .default_value(SharedString::from(DEFAULT_SIDEBAR_POSITION)),
        )
        .description("Which side of the workspace the file tree sits on.");

        let ui_family_weak = weak.clone();
        let ui_family_reset_weak = weak.clone();
        let ui_family_value = ui_font_family.clone();
        let ui_font_family_item = SettingItem::new(
            "Family",
            SettingField::input(
                move |_cx| SharedString::from(ui_family_value.clone()),
                move |value, cx| {
                    ui_family_weak
                        .update(cx, |this, cx| {
                            this.set_ui_font_family(value.to_string(), cx)
                        })
                        .ok();
                },
            )
            .on_reset(
                move |_cx| ui_font_family_is_set,
                move |window, cx| {
                    ui_family_reset_weak
                        .update(cx, |this, cx| this.reset_ui_font_family(window, cx))
                        .ok();
                },
            ),
        )
        .description("Name of an installed font for the app's interface.")
        .layout(Axis::Vertical);

        let ui_size_weak = weak.clone();
        let ui_size_reset_weak = weak.clone();
        let ui_font_size_item = SettingItem::new(
            "Size",
            SettingField::number_input(
                NumberFieldOptions {
                    min: MIN_UI_FONT_SIZE as f64,
                    max: MAX_UI_FONT_SIZE as f64,
                    step: 1.0,
                },
                move |_cx| ui_font_size,
                move |value, cx| {
                    ui_size_weak
                        .update(cx, |this, cx| this.set_ui_font_size(value as f32, cx))
                        .ok();
                },
            )
            .on_reset(
                move |_cx| ui_font_size_is_set,
                move |window, cx| {
                    ui_size_reset_weak
                        .update(cx, |this, cx| this.reset_ui_font_size(window, cx))
                        .ok();
                },
            ),
        )
        .description("Base size for the interface. Text and controls scale together.");

        // Defaults are surfaced through the "Default Settings" document rather
        // than a page-level reset, so both pages turn the reset button off.
        let pages = vec![
            SettingPage::new("Editor")
                .default_open(true)
                .resettable(false)
                .groups(vec![
                    SettingGroup::new().title("Font").items(vec![
                        font_family_item,
                        font_size_item,
                        font_weight_item,
                    ]),
                    SettingGroup::new().title("Indentation").items(vec![
                        tab_size_item,
                        hard_tabs_item,
                        smart_indent_item,
                    ]),
                    SettingGroup::new().title("Display").items(vec![
                        soft_wrap_item,
                        indent_wrapped_item,
                        whitespace_item,
                        line_numbers_item,
                        indent_guides_item,
                        folding_item,
                        scroll_beyond_item,
                        cursor_surrounding_lines_item,
                    ]),
                    SettingGroup::new().title("Editing").items(vec![
                        auto_close_item,
                        trim_whitespace_item,
                        final_newline_item,
                    ]),
                ]),
            SettingPage::new("Appearance")
                .resettable(false)
                .groups(vec![
                    SettingGroup::new()
                        .title("Appearance")
                        .items(vec![theme_item]),
                    SettingGroup::new()
                        .title("Color Scheme")
                        .items(vec![light_scheme_item, dark_scheme_item]),
                    SettingGroup::new()
                        .title("Font")
                        .items(vec![ui_font_family_item, ui_font_size_item]),
                    SettingGroup::new().title("Interface").items(vec![
                        tab_icons_item,
                        icon_color_item,
                        sidebar_position_item,
                    ]),
                ]),
        ];

        let open_json = Button::new("settings-open-json")
            .small()
            .ghost()
            .label("Open settings.json")
            .on_click(cx.listener(|this, _, window, cx| {
                this.settings.set_open(false);
                this.open_settings_file(window, cx);
                cx.notify();
            }));
        let done = Button::new("settings-done")
            .small()
            .label("Done")
            .on_click(cx.listener(|this, _, _window, cx| {
                this.settings.set_open(false);
                cx.notify();
            }));

        let header = div()
            .h_flex()
            .flex_none()
            .h_9()
            .px_3()
            .items_center()
            .justify_between()
            .border_b_1()
            .border_color(cx.theme().border)
            .child(div().text_sm().font_semibold().child("Settings"))
            .child(
                div()
                    .h_flex()
                    .items_center()
                    .gap_2()
                    .child(open_json)
                    .child(done),
            );

        let banner = self.settings.error().cloned().map(|message| {
            div()
                .flex_none()
                .px_4()
                .py_2()
                .text_sm()
                .text_color(cx.theme().red)
                .child(message)
        });

        div()
            .id("settings-overlay")
            .absolute()
            .inset_0()
            .occlude()
            .v_flex()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .child(header)
            .children(banner)
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .child(Settings::new("app-settings").pages(pages)),
            )
            .into_any_element()
    }

    /// Records the outcome of a settings write that had no window to report
    /// through, keeping the message for the overlay's inline banner.
    fn record_write(&mut self, result: Result<String, String>, cx: &mut Context<Self>) {
        self.settings.set_error(match result {
            Ok(_) => None,
            Err(message) => Some(format!("Could not save settings: {message}").into()),
        });
        // The value changed in memory whether or not the write landed, so keep
        // every other window's snapshot in step with this one.
        let settings = self.settings.values.clone();
        self.services
            .broadcast_settings(self.window_id, &settings, cx);
        cx.notify();
    }

    fn set_editor_font_family(&mut self, family: String, cx: &mut Context<Self>) {
        self.settings.values.editor.font_family = Some(family.clone());
        self.record_write(save_font_family(&family), cx);
    }

    fn set_editor_font_size(&mut self, size: f32, cx: &mut Context<Self>) {
        self.settings.values.editor.font_size = Some(size);
        self.record_write(save_font_size(Some(size)), cx);
    }

    fn set_editor_font_weight(&mut self, weight: f32, cx: &mut Context<Self>) {
        self.settings.values.editor.font_weight = Some(weight);
        crate::markdown::apply_emphasis(cx, weight);
        self.record_write(save_font_weight(weight), cx);
    }

    fn set_editor_tab_size(&mut self, size: usize, cx: &mut Context<Self>) {
        self.settings.values.editor.tab_size = Some(size);
        self.apply_tab_options(cx);
        self.record_write(save_tab_size(size), cx);
    }

    fn set_editor_hard_tabs(&mut self, enabled: bool, cx: &mut Context<Self>) {
        self.settings.values.editor.hard_tabs = Some(enabled);
        self.apply_tab_options(cx);
        self.record_write(save_hard_tabs(enabled), cx);
    }

    fn set_ui_theme(&mut self, theme: String, cx: &mut Context<Self>) {
        self.settings.values.ui.theme = Some(theme.clone());
        self.apply_theme_choice(&theme, None, cx);
        self.record_write(save_theme(&theme), cx);
    }

    fn set_ui_color_scheme_light(&mut self, name: String, cx: &mut Context<Self>) {
        self.settings.values.ui.color_scheme_light = Some(name.clone());
        self.reapply_theme_choice(cx);
        self.record_write(save_color_scheme_light(&name), cx);
    }

    fn set_ui_color_scheme_dark(&mut self, name: String, cx: &mut Context<Self>) {
        self.settings.values.ui.color_scheme_dark = Some(name.clone());
        self.reapply_theme_choice(cx);
        self.record_write(save_color_scheme_dark(&name), cx);
    }

    fn set_ui_font_family(&mut self, family: String, cx: &mut Context<Self>) {
        self.settings.values.ui.font_family = Some(family.clone());
        self.reapply_theme_choice(cx);
        self.record_write(save_ui_font_family(Some(&family)), cx);
    }

    fn set_ui_font_size(&mut self, size: f32, cx: &mut Context<Self>) {
        self.settings.values.ui.font_size = Some(size);
        self.reapply_theme_choice(cx);
        self.record_write(save_ui_font_size(Some(size)), cx);
    }

    /// Clears the interface font override so the system UI font applies again.
    fn reset_ui_font_family(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.settings.values.ui.font_family = None;
        self.reapply_theme_choice(cx);
        self.after_setting_write(save_ui_font_family(None), window, cx);
        cx.notify();
    }

    /// Clears the interface size override so the theme's base size applies again.
    fn reset_ui_font_size(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.settings.values.ui.font_size = None;
        self.reapply_theme_choice(cx);
        self.after_setting_write(save_ui_font_size(None), window, cx);
        cx.notify();
    }

    /// Re-applies the theme after a settings change while keeping the
    /// Appearance menu's check mark on the current mode.
    fn reapply_theme_choice(&self, cx: &mut Context<Self>) {
        let choice = self.settings.values.ui.theme().to_string();
        self.apply_theme_choice(&choice, None, cx);
    }

    fn set_ui_tab_icons(&mut self, enabled: bool, cx: &mut Context<Self>) {
        self.settings.values.ui.tab_icons = Some(enabled);
        self.record_write(save_tab_icons(enabled), cx);
    }

    fn set_ui_icon_color(&mut self, style: String, cx: &mut Context<Self>) {
        self.settings.values.ui.icon_color = Some(style.clone());
        self.record_write(save_icon_color(&style), cx);
    }

    fn set_ui_sidebar_position(&mut self, position: String, cx: &mut Context<Self>) {
        self.settings.values.ui.sidebar_position = Some(position.clone());
        // Panel sizes are tracked per slot index, so a flip would otherwise
        // leave the sidebar with whatever the editor's old slot held. Clearing
        // re-seeds both panels, with the sidebar's width preference.
        self.workspace.panels().update(cx, |state, _| state.clear());
        cx.notify();
        self.record_write(save_sidebar_position(&position), cx);
    }

    fn set_editor_soft_wrap(&mut self, wrap: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.settings.values.editor.soft_wrap = Some(wrap);
        self.apply_editor_options_to_all(window, cx);
        self.after_setting_write(save_soft_wrap(wrap), window, cx);
        cx.notify();
    }

    fn set_editor_show_whitespace(
        &mut self,
        show: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.settings.values.editor.show_whitespace = Some(show);
        self.apply_editor_options_to_all(window, cx);
        self.after_setting_write(save_show_whitespace(show), window, cx);
        cx.notify();
    }

    fn set_editor_cursor_surrounding_lines(
        &mut self,
        lines: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.settings.values.editor.cursor_surrounding_lines = Some(lines);
        self.apply_editor_options_to_all(window, cx);
        self.after_setting_write(save_cursor_surrounding_lines(Some(lines)), window, cx);
        cx.notify();
    }

    /// Clears the scroll margin so the engine picks it again: one line on a
    /// short viewport, three otherwise. No typed value reproduces that, which is
    /// why this setting resets to unset rather than to a number.
    fn reset_cursor_surrounding_lines(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.settings.values.editor.cursor_surrounding_lines = None;
        self.apply_editor_options_to_all(window, cx);
        self.after_setting_write(save_cursor_surrounding_lines(None), window, cx);
        cx.notify();
    }

    /// Pushes the tab-size settings onto open editors. Unlike soft wrap and
    /// whitespace, this needs no window.
    fn apply_tab_options(&mut self, cx: &mut Context<Self>) {
        let tab = TabSize {
            tab_size: self.settings.values.editor.tab_size(),
            hard_tabs: self.settings.values.editor.hard_tabs(),
        };
        let editors: Vec<_> = self.tabs.iter().map(|tab| tab.editor.clone()).collect();
        for editor in editors {
            editor.update(cx, |state, cx| state.set_tab_size(tab, cx));
        }
    }
}

#[cfg(test)]
mod tests {
    use gpui_kit::test::TestWindowExt as _;
    use gpui_kit::{AppContext as _, TestAppContext};

    use crate::app::test_support::open_app;
    use crate::settings::Settings;

    use super::SettingsState;

    /// Opening the overlay drops any error left over from an earlier write, so a
    /// stale banner cannot reappear when it is reopened.
    #[test]
    fn toggle_open_clears_a_stale_error() {
        let mut state = SettingsState::new(Settings::default());
        assert!(!state.is_open());
        assert_eq!(state.error(), None);

        state.set_error(Some("Could not save settings".into()));
        state.toggle_open();
        assert!(state.is_open());
        assert_eq!(state.error(), None, "opening discards the stale error");

        state.toggle_open();
        assert!(!state.is_open());
    }

    /// The overlay builds every field it offers, including the custom number
    /// input for the cursor scroll margin, and renders again with that field's
    /// window-held state already present.
    #[gpui_kit::test]
    async fn settings_overlay_renders_the_cursor_margin_field(cx: &mut TestAppContext) {
        let (handle, app) = open_app(cx);

        cx.update_window(handle, |_, window, cx| {
            let focus_handle = app.read(cx).focus_handle.clone();
            window.focus(&focus_handle, cx);
            app.update(cx, |app, cx| {
                app.settings.set_open(true);
                cx.notify();
            });
            window.render_frame(cx);
            window.render_frame(cx);
            assert!(app.read(cx).settings.is_open(), "the overlay stays open");
        })
        .unwrap();
        cx.run_until_parked();
    }
}
