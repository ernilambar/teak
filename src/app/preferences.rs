//! The Settings overlay, built from gpui-kit's `Settings` component.
//!
//! Field values are read from a snapshot of `TinytextApp::settings` taken at
//! render time rather than read back through the entity: the component invokes
//! the value closures while the app's own element tree is being built, and
//! reading the entity there would re-enter its borrow. The set closures capture
//! a `WeakEntity` and run on interaction, when the entity is free to update.
//!
//! Word wrap, whitespace and the other editor display switches need a `Window`
//! to push their new state onto the editors that are already open, so they are
//! built with a custom field whose render closure is handed one. Everything
//! else persists through the window-free setters and reports a write failure in
//! an inline banner.

use std::rc::Rc;

use gpui_kit::base::StyledExt as _;
use gpui_kit::component::{
    ActiveTheme as _, Sizable as _,
    button::{Button, ButtonVariants as _},
    input::TabSize,
    setting::{NumberFieldOptions, SettingField, SettingGroup, SettingItem, SettingPage, Settings},
    switch::Switch,
};
use gpui_kit::*;

use crate::settings::{
    DEFAULT_FONT_WEIGHT, DEFAULT_ICON_STYLE, DEFAULT_TAB_SIZE, DEFAULT_THEME, EditorSettings,
    MAX_FONT_SIZE, MIN_FONT_SIZE, save_font_family, save_font_size, save_font_weight,
    save_hard_tabs, save_icon_color, save_show_whitespace, save_soft_wrap, save_tab_icons,
    save_tab_size, save_theme,
};

use super::TinytextApp;

/// A boolean switch's change handler, given the window it needs to apply.
type WindowSwitchSetter =
    Rc<dyn Fn(&mut TinytextApp, bool, &mut Window, &mut Context<TinytextApp>)>;

/// A boolean switch that needs the window to apply its change immediately. The
/// typed `SettingField::switch` callbacks only receive `&mut App`; a custom
/// field gets a `&mut Window`, which is threaded into `apply`.
fn window_switch(
    id: &'static str,
    checked: bool,
    weak: WeakEntity<TinytextApp>,
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

impl TinytextApp {
    /// Builds the full-window settings overlay.
    pub(super) fn render_settings(&self, cx: &mut Context<Self>) -> AnyElement {
        let weak = cx.entity().downgrade();
        let mono_family = cx.theme().mono_font_family.to_string();
        let mono_size = cx.theme().mono_font_size.as_f32();

        let editor = &self.settings.editor;
        let ui = &self.settings.ui;
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
        let tab_icons = ui.tab_icons();
        let icon_color: SharedString = ui.icon_color().into();

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
        let auto_close_item = flag_item(
            "Auto Close Brackets",
            "Insert the closing bracket or quote as you type.",
            "auto-close-switch",
            "auto_close",
            editor.auto_close(),
            |e| &mut e.auto_close,
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
            "Theme",
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
        );

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

        let pages = vec![
            SettingPage::new("Editor").default_open(true).groups(vec![
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
                ]),
                SettingGroup::new()
                    .title("Editing")
                    .items(vec![auto_close_item]),
            ]),
            SettingPage::new("Appearance").groups(vec![
                SettingGroup::new().title("Theme").items(vec![theme_item]),
                SettingGroup::new()
                    .title("Interface")
                    .items(vec![tab_icons_item, icon_color_item]),
            ]),
        ];

        let open_json = Button::new("settings-open-json")
            .small()
            .ghost()
            .label("Open settings.json")
            .on_click(cx.listener(|this, _, window, cx| {
                this.settings_open = false;
                this.open_settings_file(window, cx);
                cx.notify();
            }));
        let done = Button::new("settings-done")
            .small()
            .label("Done")
            .on_click(cx.listener(|this, _, _window, cx| {
                this.settings_open = false;
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

        let banner = self.settings_error.clone().map(|message| {
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
                    .child(Settings::new("tinytext-settings").pages(pages)),
            )
            .into_any_element()
    }

    /// Records the outcome of a settings write that had no window to report
    /// through, keeping the message for the overlay's inline banner.
    fn record_write(&mut self, result: Result<String, String>, cx: &mut Context<Self>) {
        self.settings_error = match result {
            Ok(_) => None,
            Err(message) => Some(format!("Could not save settings: {message}").into()),
        };
        cx.notify();
    }

    fn set_editor_font_family(&mut self, family: String, cx: &mut Context<Self>) {
        self.settings.editor.font_family = Some(family.clone());
        self.record_write(save_font_family(&family), cx);
    }

    fn set_editor_font_size(&mut self, size: f32, cx: &mut Context<Self>) {
        self.settings.editor.font_size = Some(size);
        self.record_write(save_font_size(Some(size)), cx);
    }

    fn set_editor_font_weight(&mut self, weight: f32, cx: &mut Context<Self>) {
        self.settings.editor.font_weight = Some(weight);
        crate::markdown::apply_emphasis(cx, weight);
        self.record_write(save_font_weight(weight), cx);
    }

    fn set_editor_tab_size(&mut self, size: usize, cx: &mut Context<Self>) {
        self.settings.editor.tab_size = Some(size);
        self.apply_tab_options(cx);
        self.record_write(save_tab_size(size), cx);
    }

    fn set_editor_hard_tabs(&mut self, enabled: bool, cx: &mut Context<Self>) {
        self.settings.editor.hard_tabs = Some(enabled);
        self.apply_tab_options(cx);
        self.record_write(save_hard_tabs(enabled), cx);
    }

    fn set_ui_theme(&mut self, theme: String, cx: &mut Context<Self>) {
        self.settings.ui.theme = Some(theme.clone());
        self.apply_theme_choice(&theme, None, cx);
        self.record_write(save_theme(&theme), cx);
    }

    fn set_ui_tab_icons(&mut self, enabled: bool, cx: &mut Context<Self>) {
        self.settings.ui.tab_icons = Some(enabled);
        self.record_write(save_tab_icons(enabled), cx);
    }

    fn set_ui_icon_color(&mut self, style: String, cx: &mut Context<Self>) {
        self.settings.ui.icon_color = Some(style.clone());
        self.record_write(save_icon_color(&style), cx);
    }

    fn set_editor_soft_wrap(&mut self, wrap: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.settings.editor.soft_wrap = Some(wrap);
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
        self.settings.editor.show_whitespace = Some(show);
        self.apply_editor_options_to_all(window, cx);
        self.after_setting_write(save_show_whitespace(show), window, cx);
        cx.notify();
    }

    /// Pushes the tab-size settings onto open editors. Unlike soft wrap and
    /// whitespace, this needs no window.
    fn apply_tab_options(&mut self, cx: &mut Context<Self>) {
        let tab = TabSize {
            tab_size: self.settings.editor.tab_size(),
            hard_tabs: self.settings.editor.hard_tabs(),
        };
        let editors: Vec<_> = self.tabs.iter().map(|tab| tab.editor.clone()).collect();
        for editor in editors {
            editor.update(cx, |state, cx| state.set_tab_size(tab, cx));
        }
    }
}
