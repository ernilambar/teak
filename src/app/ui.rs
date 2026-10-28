use std::cell::RefCell;
use std::collections::HashMap;
use std::ops::Range;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use gpui_kit::base::{InteractiveElementExt as _, StyledExt as _};
use gpui_kit::component::{
    ActiveTheme as _, Icon, IconName, Sizable as _, Size,
    badge::Badge,
    button::{Button, ButtonVariants as _},
    input::{self, Editor},
    kbd::Kbd,
    list::ListItem,
    menu::{ContextMenuExt as _, DropdownMenu as _, PopupMenuItem},
    resizable::{ResizableState, h_resizable, resizable_panel},
    scroll::ScrollableElement as _,
    spinner::Spinner,
    status_bar::StatusBar,
    text::TextView,
    tooltip::Tooltip,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::file_icons::{file_icon, folder_icon};
use crate::git::{Branch, Status};
use crate::language::{LANGUAGES, editor_language_id};
use crate::paths::{display_path, file_name, format_size};
use crate::settings::IconStyle;
use crate::{
    AddCursorAbove, AddCursorBelow, AddFolder, CopyFilePath, CopyRelativePath, NewFile, OpenFile,
    RevealInFinder,
};

use super::workspace::{SIDEBAR_MAX_WIDTH, SIDEBAR_MIN_WIDTH};
use super::{Preview, TeakApp, chrome::MAX_COUNT_CHARS};

/// Floor on a tab's width. Tabs size to their full titles while the strip has
/// room, shrink with an ellipsis down to this floor as it fills, and past that
/// the strip scrolls horizontally.
const TAB_MIN_WIDTH: Pixels = px(80.);

const TAB_HEIGHT: Pixels = px(32.);
/// Hover time before a tab's path tooltip appears; GPUI's default is 500ms.
const TAB_TOOLTIP_DELAY: Duration = Duration::from_millis(800);

/// Group name shared by every tab, so its close button can react to hover.
const TAB_GROUP: &str = "tab";

/// Payload carried while a tab is dragged. It holds the editor's stable id, not
/// an index, so the drop resolves to the right tab even if the `Vec` shifted.
#[derive(Clone)]
struct DragTab {
    id: EntityId,
    title: SharedString,
}

/// The floating chip that follows the cursor while a tab is dragged. GPUI
/// positions this entity at the pointer; it only draws its own appearance.
struct TabDragPreview {
    title: SharedString,
}

impl Render for TabDragPreview {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .px_3()
            .h(TAB_HEIGHT)
            .flex()
            .items_center()
            .bg(cx.theme().secondary)
            .border_1()
            .border_color(cx.theme().border)
            .rounded(px(4.))
            .text_sm()
            .child(self.title.clone())
    }
}

/// A tab's status dot, drawn in the close button's slot and hidden on hover so
/// the button shows through.
fn status_dot(visible: bool, color: Hsla) -> Div {
    div()
        .absolute()
        .inset_0()
        .flex()
        .items_center()
        .justify_center()
        .group_hover(TAB_GROUP, |style| style.invisible())
        .when(visible, |this| {
            this.child(div().size(px(6.)).rounded_full().bg(color))
        })
}

/// The branch glyph in the status bar. Vendored from Lucide because the icon
/// set the app registers is the small default subset, which has no git icons;
/// embedding the full catalogue for one glyph would cost megabytes.
const GIT_BRANCH_ICON: &[u8] = include_bytes!("../../assets/icons/git-branch.svg");

/// The status-bar text for a branch: its name, and how far it has diverged
/// from its upstream when it tracks one.
fn branch_label(branch: &Branch) -> String {
    let mut label = branch.name.clone();
    if branch.ahead > 0 {
        label.push_str(&format!(" ↑{}", branch.ahead));
    }
    if branch.behind > 0 {
        label.push_str(&format!(" ↓{}", branch.behind));
    }
    label
}

/// The git change tint: the theme's warning yellow, desaturated and pulled
/// halfway toward the text lightness. The raw warning is bright enough to
/// compete with the row content, and on light themes it is too pale to read at
/// this size; the pull gives it the softer, sandier tone of a change marker
/// while staying inside the theme's palette.
pub(super) fn change_tint(warning: Hsla, foreground: Hsla) -> Hsla {
    hsla(
        warning.h,
        warning.s * 0.5,
        warning.l + (foreground.l - warning.l) * 0.5,
        warning.a,
    )
}

/// Logical edge length of a file-type icon, matching `Size::Small` (0.875rem).
const FILE_ICON_SIZE: f32 = 14.;
/// Rasterized edge length of a colorful icon. Twice [`FILE_ICON_SIZE`] keeps it
/// sharp on Retina displays; the image element downsamples it to fit.
const COLOR_ICON_PIXELS: i32 = 28;

/// The status bar's totals: `1,234 words · 5,678 chars`. Above the word-count
/// limit only the character count is shown.
fn count_label(words: usize, chars: usize) -> String {
    let chars_label = format!(
        "{} {}",
        grouped(chars),
        if chars == 1 { "char" } else { "chars" }
    );
    if chars > MAX_COUNT_CHARS {
        return chars_label;
    }
    format!(
        "{} {} · {chars_label}",
        grouped(words),
        if words == 1 { "word" } else { "words" }
    )
}

/// `1234` as `1,234`.
fn grouped(number: usize) -> String {
    let digits = number.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (ix, digit) in digits.chars().enumerate() {
        if ix > 0 && (digits.len() - ix).is_multiple_of(3) {
            out.push(',');
        }
        out.push(digit);
    }
    out
}

thread_local! {
    /// Full-color rasterizations keyed by the address of their static SVG bytes.
    /// `RenderImage::new` mints a fresh id every call, so reusing one cached
    /// image is what keeps the sprite atlas from growing on every frame.
    static COLOR_ICON_CACHE: RefCell<HashMap<usize, Arc<RenderImage>>> =
        RefCell::new(HashMap::new());
}

/// The status bar's one line for an image: format, pixel size and bytes.
fn image_summary(image: &Image) -> String {
    let format = image.format.extension().to_ascii_uppercase();
    match image_dimensions(image) {
        Some((width, height)) => format!(
            "{format} · {width} × {height} · {}",
            format_size(image.bytes.len())
        ),
        None => format!("{format} · {}", format_size(image.bytes.len())),
    }
}

/// The image's pixel size, read from its header only. `None` for a format the
/// header reader does not know, which leaves the viewer in fit mode.
fn image_dimensions(image: &Image) -> Option<(usize, usize)> {
    imagesize::blob_size(&image.bytes)
        .ok()
        .map(|size| (size.width, size.height))
}

/// A file-type icon for the sidebar and tabs. GPUI's `svg` element — and the
/// `Icon` component built on it — flattens an SVG to a single tinted color, so
/// the colorful style draws the vendored SVG through the image pipeline
/// instead, which preserves its fills. A rasterized icon cannot be recolored,
/// so a requested `tint` draws the monochrome silhouette even in the colorful
/// style; it is the same glyph, in one color.
pub(super) fn file_type_icon(
    bytes: &'static [u8],
    style: IconStyle,
    tint: Option<Hsla>,
    cx: &App,
) -> AnyElement {
    if tint.is_none()
        && style == IconStyle::Colorful
        && let Some(image) = colored_icon(bytes, cx)
    {
        return img(image)
            .w(px(FILE_ICON_SIZE))
            .h(px(FILE_ICON_SIZE))
            .flex_none()
            .into_any_element();
    }

    let icon = Icon::empty().data(bytes).with_size(Size::Small).flex_none();
    match tint {
        Some(color) => icon.text_color(color),
        None => icon,
    }
    .into_any_element()
}

/// Rasterizes `bytes` in full color, caching the result. `None` only when the
/// SVG cannot be parsed, in which case the caller falls back to the monochrome
/// icon.
fn colored_icon(bytes: &'static [u8], cx: &App) -> Option<Arc<RenderImage>> {
    let key = bytes.as_ptr() as usize;
    if let Some(image) = COLOR_ICON_CACHE.with(|cache| cache.borrow().get(&key).cloned()) {
        return Some(image);
    }

    let renderer = cx.svg_renderer();
    let parsed = renderer.parse_svg(bytes).ok()?;
    let size = gpui_kit::Size::new(
        DevicePixels(COLOR_ICON_PIXELS),
        DevicePixels(COLOR_ICON_PIXELS),
    );
    let image = renderer.render_parsed(&parsed, SvgSize::Size(size)).ok()?;
    COLOR_ICON_CACHE.with(|cache| cache.borrow_mut().insert(key, image.clone()));
    Some(image)
}

impl TeakApp {
    pub(super) fn render_tab_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let entity = cx.entity();
        let active = self.tabs.active_index();
        let theme = cx.theme();
        let dirty_color = theme.foreground;
        let deleted_color = theme.red;
        let border_color = theme.border;
        let bar_bg = theme.secondary;
        let active_bg = theme.background;
        // Separators take the editor background, so the active tab — which uses
        // that same background — is the only bright edge on the strip.
        let separator_color = theme.background;
        let hover_bg = bar_bg.blend(theme.foreground.opacity(0.06));
        let fg = theme.muted_foreground;
        let active_fg = theme.foreground;
        let accent = theme.blue;
        let show_icons = self.settings.values.ui.tab_icons();
        let icon_style = self.settings.values.ui.icon_style();

        let tabs: Vec<Stateful<Div>> = self
            .tabs
            .iter()
            .enumerate()
            .map(|(ix, tab)| {
                let close_entity = entity.clone();
                let selected = active == Some(ix);
                let deleted = tab.document.deleted_on_disk;
                // Full path on hover, telling apart titles the ellipsis cuts alike.
                let tooltip: SharedString = match &tab.document.path {
                    Some(path) => display_path(path).into(),
                    None => tab.document.title.clone(),
                };
                let (dot, dot_color) = if deleted {
                    (true, deleted_color)
                } else {
                    (tab.document.dirty, dirty_color)
                };
                div()
                    .id(("tab", ix))
                    .group(TAB_GROUP)
                    .h_flex()
                    .flex_shrink(1.)
                    .min_w(TAB_MIN_WIDTH)
                    .h(TAB_HEIGHT)
                    .px_2()
                    .gap_1()
                    .text_sm()
                    // A trailing rule separates adjacent tabs; on the active tab
                    // it matches the background and disappears.
                    .border_r_1()
                    .border_color(separator_color)
                    .map(|this| {
                        if selected {
                            this.bg(active_bg).text_color(active_fg)
                        } else {
                            this.bg(bar_bg)
                                .text_color(fg)
                                .hover(|style| style.bg(hover_bg).text_color(active_fg))
                        }
                    })
                    .when(show_icons, |this| {
                        if tab.find_results.is_some() {
                            return this.child(
                                Icon::new(IconName::Search)
                                    .with_size(Size::Small)
                                    .flex_none(),
                            );
                        }
                        // Untitled tabs have no path and fall back to the default file icon.
                        let path = tab.document.path.as_deref().unwrap_or(Path::new(""));
                        this.child(file_type_icon(file_icon(path), icon_style, None, cx))
                    })
                    .child(
                        div()
                            .min_w_0()
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_ellipsis()
                            // A preview tab is italic until it is kept.
                            .when(tab.transient, |this| this.italic())
                            .child(tab.document.title.clone()),
                    )
                    .child(
                        div()
                            .relative()
                            .flex_none()
                            .child(
                                div()
                                    .invisible()
                                    .group_hover(TAB_GROUP, |style| style.visible())
                                    .child(
                                        Button::new(("close-tab", ix))
                                            .xsmall()
                                            .ghost()
                                            .icon(IconName::Close)
                                            .on_click(move |_, window, cx| {
                                                cx.stop_propagation();
                                                close_entity.update(cx, |this, cx| {
                                                    this.close_tab(ix, window, cx)
                                                });
                                            }),
                                    ),
                            )
                            .child(status_dot(dot, dot_color)),
                    )
                    .tooltip(move |window, cx| Tooltip::new(tooltip.clone()).build(window, cx))
                    .tooltip_show_delay(TAB_TOOLTIP_DELAY)
                    .on_drag(
                        DragTab {
                            id: tab.document.editor_id(),
                            title: tab.document.title.clone(),
                        },
                        move |drag: &DragTab, _offset, _window, cx| {
                            cx.new(|_| TabDragPreview {
                                title: drag.title.clone(),
                            })
                        },
                    )
                    // Dropping onto a tab inserts the dragged tab before it; the
                    // border on the tab's left edge marks that gap.
                    .drag_over::<DragTab>(move |this, _, _, _cx| {
                        this.border_l_2().border_r_0().border_color(accent)
                    })
                    .on_drop::<DragTab>(cx.listener(move |this, drag: &DragTab, window, cx| {
                        this.move_tab(drag.id, ix, window, cx);
                    }))
                    .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                        this.activate_tab(ix, window, cx);
                        // A double click keeps a preview tab.
                        if event.click_count() >= 2
                            && this.tabs.get(ix).is_some_and(|tab| tab.transient)
                        {
                            this.tabs.promote(ix);
                            cx.notify();
                        }
                    }))
                    .on_mouse_down(
                        MouseButton::Right,
                        cx.listener(move |this, _, _, _cx| {
                            this.tabs.set_context(Some(ix));
                        }),
                    )
            })
            .collect();

        let titles: Vec<SharedString> = self
            .tabs
            .iter()
            .map(|tab| tab.document.title.clone())
            .collect();
        let overflow_entity = entity.clone();
        let menu_entity = entity.clone();
        div()
            .id("workspace-tab-bar")
            .flex_none()
            .relative()
            .h_flex()
            .bg(bar_bg)
            .capture_any_mouse_down(cx.listener(|this, event: &MouseDownEvent, _, _cx| {
                if event.button == MouseButton::Right {
                    this.tabs.set_context(None);
                }
            }))
            .context_menu(move |menu, _, cx| {
                let Some(ix) = menu_entity.read(cx).tabs.context() else {
                    return menu;
                };
                let (tab_count, has_deleted) = {
                    let app = menu_entity.read(cx);
                    (
                        app.tabs.len(),
                        app.tabs.iter().any(|tab| {
                            tab.document
                                .path
                                .as_deref()
                                .is_some_and(|path| !path.exists())
                        }),
                    )
                };
                if ix >= tab_count {
                    return menu;
                }

                let close_entity = menu_entity.clone();
                let others_entity = menu_entity.clone();
                let right_entity = menu_entity.clone();
                let all_entity = menu_entity.clone();
                let deleted_entity = menu_entity.clone();
                let move_entity = menu_entity.clone();
                let movable = menu_entity.read(cx).tabs.get(ix).is_some_and(|tab| {
                    !tab.document.dirty
                        && !tab.raster
                        && tab.find_results.is_none()
                        && tab.previewing().is_none()
                        && tab.document.path.as_deref().is_some_and(Path::is_file)
                });

                menu.item(
                    PopupMenuItem::new("Close Tab").on_click(move |_, window, cx| {
                        close_entity.update(cx, |this, cx| this.close_tab(ix, window, cx));
                    }),
                )
                .item(
                    PopupMenuItem::new("Move to New Window")
                        .disabled(!movable)
                        .on_click(move |_, window, cx| {
                            move_entity
                                .update(cx, |this, cx| this.move_tab_to_new_window(ix, window, cx));
                        }),
                )
                .separator()
                .item(
                    PopupMenuItem::new("Close Other Tabs")
                        .disabled(tab_count <= 1)
                        .on_click(move |_, window, cx| {
                            others_entity
                                .update(cx, |this, cx| this.close_other_tabs(ix, window, cx));
                        }),
                )
                .item(
                    PopupMenuItem::new("Close Tabs to the Right")
                        .disabled(ix + 1 >= tab_count)
                        .on_click(move |_, window, cx| {
                            right_entity
                                .update(cx, |this, cx| this.close_tabs_to_right(ix, window, cx));
                        }),
                )
                .item(
                    PopupMenuItem::new("Close All").on_click(move |_, window, cx| {
                        all_entity.update(cx, |this, cx| this.close_all_tabs(window, cx));
                    }),
                )
                .separator()
                .item(
                    PopupMenuItem::new("Close Tabs with Deleted Files")
                        .disabled(!has_deleted)
                        .on_click(move |_, window, cx| {
                            deleted_entity.update(cx, |this, cx| {
                                this.close_tabs_with_deleted_files(window, cx)
                            });
                        }),
                )
            })
            // The bar's bottom edge, painted beneath the tabs. The active tab's
            // opaque background covers it, so only the active tab blends into
            // the editor below.
            .child(
                div()
                    .absolute()
                    .left_0()
                    .right_0()
                    .bottom_0()
                    .h(px(1.))
                    .bg(border_color),
            )
            .child(
                div()
                    .id("workspace-tabs")
                    .flex_1()
                    .min_w_0()
                    .h_flex()
                    .overflow_x_scroll()
                    .lock_scroll_axis()
                    .track_scroll(self.tabs.scroll())
                    .children(tabs)
                    // The empty area right of the last tab. Dropping here moves the
                    // dragged tab to the end; the border marks that final gap. It is
                    // the last child, so it never disturbs `reveal_active_tab`'s
                    // index into the tabs.
                    .child(
                        div()
                            .id("tab-strip-tail")
                            .relative()
                            .flex_1()
                            .h_full()
                            .min_w(px(16.))
                            .drag_over::<DragTab>(move |this, _, _, _cx| {
                                this.border_l_2().border_color(accent)
                            })
                            .on_drop::<DragTab>(cx.listener(|this, drag: &DragTab, window, cx| {
                                let end = this.tabs.len();
                                this.move_tab(drag.id, end, window, cx);
                            })),
                    ),
            )
            .child(
                div().flex_none().px_1().child(
                    Button::new("tab-overflow")
                        .xsmall()
                        .ghost()
                        .dropdown_caret(true)
                        .dropdown_menu(move |menu, _, _| {
                            titles.iter().enumerate().fold(
                                menu.scrollable(true),
                                |menu, (ix, title)| {
                                    let entity = overflow_entity.clone();
                                    menu.item(
                                        PopupMenuItem::new(title.clone())
                                            .checked(active == Some(ix))
                                            .on_click(move |_, window, cx| {
                                                entity.update(cx, |this, cx| {
                                                    this.activate_tab(ix, window, cx)
                                                });
                                            }),
                                    )
                                },
                            )
                        })
                        .anchor(Anchor::TopRight),
                ),
            )
    }

    pub(super) fn render_workspace(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        if self.workspace.sidebar_visible() {
            let sidebar = resizable_panel()
                .size(px(self.workspace.width() as f32))
                .size_range(px(SIDEBAR_MIN_WIDTH as f32)..px(SIDEBAR_MAX_WIDTH as f32))
                .child(self.render_sidebar(cx));
            let editor = resizable_panel().child(self.render_editor_panel(window, cx));

            let on_resize = {
                let weak = cx.entity().downgrade();
                move |state: &Entity<ResizableState>, _: &mut Window, cx: &mut App| {
                    weak.update(cx, |this, cx| {
                        // The panel sized by the drag is the one the sidebar
                        // sits in: slot 0 on the left, the leftover slot on the
                        // right.
                        let ix = usize::from(this.settings.values.ui.sidebar_right());
                        let Some(width) = state.read(cx).sizes().get(ix).copied() else {
                            return;
                        };
                        let width = (width.as_f32().round() as u32)
                            .clamp(SIDEBAR_MIN_WIDTH, SIDEBAR_MAX_WIDTH);
                        if width != this.workspace.width() {
                            this.workspace.set_width(width);
                            this.save_session(cx);
                        }
                    })
                    .ok();
                }
            };

            let group = h_resizable("workspace-panels")
                .with_state(self.workspace.panels())
                .on_resize(on_resize);
            let group = if self.settings.values.ui.sidebar_right() {
                group.child(editor).child(sidebar)
            } else {
                group.child(sidebar).child(editor)
            };

            div().flex_1().min_h_0().child(group)
        } else {
            div()
                .flex_1()
                .min_h_0()
                .child(self.render_editor_panel(window, cx))
        }
    }

    pub(super) fn render_editor_panel(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        div()
            .v_flex()
            .size_full()
            .min_w_0()
            .when(!self.tabs.is_empty(), |this| {
                this.child(self.render_tab_bar(cx))
            })
            .child(div().flex_1().min_h_0().child(self.render_editor_for(
                self.tabs.active_index(),
                window,
                cx,
            )))
    }

    pub(super) fn render_sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .v_flex()
            .size_full()
            .bg(cx.theme().background)
            .child(
                div()
                    .id("sidebar-header")
                    .h_flex()
                    .flex_none()
                    .h_9()
                    .px_3()
                    .items_center()
                    .gap_2()
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .child(div().flex_1().text_sm().font_semibold().child("Explorer"))
                    .child(
                        Button::new("sidebar-add-folder")
                            .xsmall()
                            .ghost()
                            .icon(IconName::Plus)
                            .tooltip("Add Folder…")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.on_add_folder(&AddFolder, window, cx)
                            })),
                    ),
            )
            .child(if self.workspace.roots().is_empty() {
                self.render_sidebar_empty(cx).into_any_element()
            } else {
                self.render_tree(cx).into_any_element()
            })
    }

    /// The placeholder shown while the sidebar has no folders. Its button
    /// opens the same folder picker as the header's Add button.
    fn render_sidebar_empty(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let muted = cx.theme().muted_foreground;
        div()
            .id("sidebar-empty")
            .flex_1()
            .min_h_0()
            .v_flex()
            .items_center()
            .justify_center()
            .gap_3()
            .child(
                Icon::new(IconName::FolderOpen)
                    .with_size(Size::Large)
                    .text_color(muted),
            )
            .child(div().text_sm().text_color(muted).child("No folders added"))
            .child(
                Button::new("sidebar-empty-add")
                    .small()
                    .label("Add Folder…")
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.on_add_folder(&AddFolder, window, cx)
                    })),
            )
    }

    /// The accent frame shown while paths from the file manager hover the
    /// window. `folders_only` and the live Option key decide the promise it
    /// makes, so the hint matches what the drop will actually do.
    pub(super) fn render_drop_overlay(
        &self,
        folders_only: bool,
        alt: bool,
        cx: &App,
    ) -> impl IntoElement {
        let theme = cx.theme();
        let accent = theme.drag_border;
        let label = match (folders_only, alt) {
            (true, false) => "Drop to add folder",
            (true, true) => "Drop to add folder in a new window",
            (false, false) => "Drop to open",
            (false, true) => "Drop to open in a new window",
        };

        div()
            .absolute()
            .inset_0()
            .border_2()
            .border_dashed()
            .border_color(accent.opacity(0.65))
            .bg(accent.opacity(0.05))
            .v_flex()
            .items_center()
            .justify_center()
            .child(
                div()
                    .px_4()
                    .py_2()
                    .rounded_lg()
                    .bg(theme.primary)
                    .text_color(theme.primary_foreground)
                    .text_sm()
                    .font_semibold()
                    .child(label),
            )
    }

    /// The virtualized list of the flattened workspace tree. Roots are the
    /// first rows at depth 0, each followed by its expanded subtree.
    fn render_tree(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let row_count = self.workspace.rows().len();
        div()
            .id("explorer-scroll")
            .flex_1()
            .min_h_0()
            .py_1()
            .child(
                uniform_list(
                    "explorer-list",
                    row_count,
                    cx.processor(|this, range: Range<usize>, _window, cx| {
                        range
                            .map(|ix| {
                                let row = &this.workspace.rows()[ix];
                                let (path, depth, is_dir, is_root) =
                                    (row.path.clone(), row.depth, row.is_dir, row.is_root);
                                this.tree_row(&path, is_dir, depth, is_root, cx)
                            })
                            .collect::<Vec<_>>()
                    }),
                )
                .size_full()
                .track_scroll(self.workspace.scroll()),
            )
            .vertical_scrollbar(self.workspace.scroll())
    }

    pub(super) fn tree_row(
        &self,
        path: &Path,
        is_dir: bool,
        depth: usize,
        is_root: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let selected = self.workspace.selected() == Some(path);
        let flashed = self.workspace.flash() == Some(path);
        let flash_bg = cx.theme().drag_border.opacity(0.22);
        let expanded = self.workspace.is_expanded(path);
        let muted = cx.theme().muted_foreground;
        let git = self.workspace.git();
        let tint = change_tint(cx.theme().warning, cx.theme().foreground);
        // A file shows its own state. A folder shows the most serious change
        // beneath it, or its own entry when it is untracked, so the cue
        // survives collapsing it.
        let (status, below) = if is_dir {
            (None, git.folder_status(path).or_else(|| git.status(path)))
        } else {
            (git.status(path), None)
        };
        let status_color = status.map(|status| match status {
            Status::Untracked | Status::Added => cx.theme().success,
            Status::Modified | Status::Renamed => tint,
            Status::Deleted | Status::Conflicted => cx.theme().danger,
        });
        // A modification tints the name, for a file and for every folder above
        // it, so it is visible while the folder is still collapsed.
        let name_color = match status.or(below) {
            Some(Status::Modified | Status::Renamed) => Some(tint),
            _ => None,
        };
        let folder_changed = below.is_some();

        let leading = if is_dir {
            Icon::new(if expanded {
                IconName::ChevronDown
            } else {
                IconName::ChevronRight
            })
            .with_size(Size::Small)
            .text_color(muted)
            .into_any_element()
        } else {
            div().w(px(14.)).into_any_element()
        };

        let icon_style = self.settings.values.ui.icon_style();
        // Only folders take the tint. A file already shows its state in a
        // yellow name and letter, while a collapsed folder needs the icon and
        // dot to keep the cue visible.
        let type_icon = if is_dir {
            file_type_icon(folder_icon(), icon_style, name_color, cx)
        } else {
            file_type_icon(file_icon(path), icon_style, None, cx)
        };

        let click_path = path.to_path_buf();
        let label = file_name(path);
        let accessible = match status.or(below) {
            Some(change) => format!("{label} {}", change.letter()),
            None => label.clone(),
        };
        let menu_entity = cx.entity();
        let menu_path = path.to_path_buf();

        ListItem::new(path.display().to_string())
            .w_full()
            .pl(px(8. + depth as f32 * 12.))
            .pr_2()
            .py_0p5()
            .selected(selected)
            .accessibility_label(accessible)
            .child(
                div()
                    .h_flex()
                    .w_full()
                    .items_center()
                    .gap_1()
                    .when(flashed, |row| row.bg(flash_bg).rounded_sm())
                    .child(leading)
                    .child(type_icon)
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_sm()
                            .when_some(name_color, |name, color| name.text_color(color))
                            .child(label),
                    )
                    .when_some(status.zip(status_color), |row, (status, color)| {
                        row.child(
                            div()
                                .flex_none()
                                .text_xs()
                                .text_color(color)
                                .child(status.letter()),
                        )
                    })
                    .when(folder_changed, |row| {
                        row.child(
                            div()
                                .size(px(6.))
                                .rounded_full()
                                .flex_none()
                                .bg(name_color.unwrap_or(muted)),
                        )
                    }),
            )
            .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                this.workspace.set_selected(Some(click_path.clone()));
                if is_dir {
                    this.workspace.toggle_expanded(&click_path);
                } else if event.click_count() == 1 {
                    // A browsing click previews; a double click keeps the tab.
                    this.request_open_transient(click_path.clone(), window, cx);
                } else {
                    this.request_open(click_path.clone(), window, cx);
                }
                this.save_session(cx);
                cx.notify();
            }))
            .context_menu(move |menu, _, cx| {
                let paste_dir = if is_dir {
                    menu_path.clone()
                } else {
                    menu_path
                        .parent()
                        .map(Path::to_path_buf)
                        .unwrap_or_default()
                };
                let can_paste = menu_entity.read(cx).workspace.clipboard().is_some();

                let menu = if is_dir {
                    let new_file_entity = menu_entity.clone();
                    let new_file_dir = menu_path.clone();
                    let new_folder_entity = menu_entity.clone();
                    let new_folder_dir = menu_path.clone();
                    menu.item(
                        PopupMenuItem::new("New File…").on_click(move |_, window, cx| {
                            new_file_entity.update(cx, |this, cx| {
                                this.create_entry(new_file_dir.clone(), false, window, cx)
                            });
                        }),
                    )
                    .item(
                        PopupMenuItem::new("New Folder…").on_click(move |_, window, cx| {
                            new_folder_entity.update(cx, |this, cx| {
                                this.create_entry(new_folder_dir.clone(), true, window, cx)
                            });
                        }),
                    )
                    .separator()
                } else {
                    let open_entity = menu_entity.clone();
                    let open_path = menu_path.clone();
                    menu.item(PopupMenuItem::new("Open").on_click(move |_, window, cx| {
                        open_entity.update(cx, |this, cx| {
                            this.request_open(open_path.clone(), window, cx)
                        });
                    }))
                    .separator()
                };

                let reveal_entity = menu_entity.clone();
                let reveal_path = menu_path.clone();
                let cut_entity = menu_entity.clone();
                let cut_path = menu_path.clone();
                let copy_entity = menu_entity.clone();
                let copy_path = menu_path.clone();
                let paste_entity = menu_entity.clone();
                let name_entity = menu_entity.clone();
                let name_value = menu_path.clone();
                let path_entity = menu_entity.clone();
                let path_value = menu_path.clone();
                let relative_entity = menu_entity.clone();
                let relative_value = menu_path.clone();
                let remove_entity = menu_entity.clone();
                let remove_path = menu_path.clone();
                let rename_entity = menu_entity.clone();
                let rename_path = menu_path.clone();
                let duplicate_entity = menu_entity.clone();
                let duplicate_path = menu_path.clone();
                let move_entity = menu_entity.clone();
                let move_path = menu_path.clone();
                let delete_entity = menu_entity.clone();
                let delete_path = menu_path.clone();

                let menu = menu
                    .item(
                        PopupMenuItem::new("Reveal in Finder").on_click(move |_, _window, cx| {
                            reveal_entity
                                .update(cx, |this, cx| this.reveal_in_finder(&reveal_path, cx));
                        }),
                    )
                    .separator();

                // Cut and Copy route through the file clipboard, which has no
                // meaning for a whole workspace root.
                let menu = if is_root {
                    menu
                } else {
                    menu.item(PopupMenuItem::new("Cut").on_click(move |_, _window, cx| {
                        cut_entity.update(cx, |this, cx| {
                            this.set_file_clipboard(cut_path.clone(), true, cx)
                        });
                    }))
                    .item(PopupMenuItem::new("Copy").on_click(move |_, _window, cx| {
                        copy_entity.update(cx, |this, cx| {
                            this.set_file_clipboard(copy_path.clone(), false, cx)
                        });
                    }))
                };

                let menu = menu
                    .item(PopupMenuItem::new("Paste").disabled(!can_paste).on_click(
                        move |_, window, cx| {
                            paste_entity.update(cx, |this, cx| {
                                this.paste_clipboard(paste_dir.clone(), window, cx)
                            });
                        },
                    ))
                    .separator()
                    .item(
                        PopupMenuItem::new("Copy Name").on_click(move |_, _window, cx| {
                            name_entity.update(cx, |this, cx| {
                                this.copy_name_to_clipboard(&name_value, cx)
                            });
                        }),
                    )
                    .item(
                        PopupMenuItem::new("Copy Path").on_click(move |_, _window, cx| {
                            path_entity.update(cx, |this, cx| {
                                this.copy_path_to_clipboard(&path_value, cx)
                            });
                        }),
                    );

                // A root's relative path is the empty string, so the item is
                // only offered for entries inside a root.
                let menu = if is_root {
                    menu
                } else {
                    menu.item(PopupMenuItem::new("Copy Relative Path").on_click(
                        move |_, _window, cx| {
                            relative_entity.update(cx, |this, cx| {
                                this.copy_relative_path(&relative_value, cx)
                            });
                        },
                    ))
                };

                let menu = menu.separator();

                if is_root {
                    menu.item(PopupMenuItem::new("Remove Folder").on_click(
                        move |_, _window, cx| {
                            remove_entity.update(cx, |this, cx| this.remove_root(&remove_path, cx));
                        },
                    ))
                } else {
                    menu.item(
                        PopupMenuItem::new("Rename…").on_click(move |_, window, cx| {
                            rename_entity.update(cx, |this, cx| {
                                this.rename_entry(rename_path.clone(), window, cx)
                            });
                        }),
                    )
                    .item(
                        PopupMenuItem::new("Duplicate").on_click(move |_, window, cx| {
                            duplicate_entity.update(cx, |this, cx| {
                                this.duplicate_entry(duplicate_path.clone(), window, cx)
                            });
                        }),
                    )
                    .item(PopupMenuItem::new("Move…").on_click(move |_, window, cx| {
                        move_entity.update(cx, |this, cx| {
                            this.move_entry(move_path.clone(), window, cx)
                        });
                    }))
                    .item(PopupMenuItem::new("Delete").on_click(move |_, window, cx| {
                        delete_entity.update(cx, |this, cx| {
                            this.delete_entry(delete_path.clone(), window, cx)
                        });
                    }))
                }
            })
            .into_any_element()
    }

    pub(super) fn render_editor_for(
        &self,
        ix: Option<usize>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let tab = ix.and_then(|ix| self.tabs.get(ix));
        match tab.and_then(|tab| tab.previewing()) {
            Some(Preview::Markdown(view)) => {
                return div()
                    .size_full()
                    .bg(cx.theme().background)
                    .child(
                        TextView::new(view)
                            .scrollable(true)
                            .selectable(true)
                            .size_full()
                            .px_6()
                            .py_4(),
                    )
                    .into_any_element();
            }
            Some(Preview::Image(image)) => {
                let muted = cx.theme().muted_foreground;
                let view = tab.map(|tab| tab.image).unwrap_or_default();
                // The image's own pixels, expressed in the window's logical
                // units so `zoom` 1 is actual size on a Retina display too.
                let dimensions = image_dimensions(image);
                let factor = window.scale_factor();
                let source = |image: &Arc<Image>| ImageSource::Image(image.clone());

                let body = if view.fit || dimensions.is_none() {
                    div()
                        .size_full()
                        .p_6()
                        .child(
                            img(source(image))
                                .size_full()
                                .object_fit(ObjectFit::ScaleDown)
                                .with_fallback(move || {
                                    div()
                                        .size_full()
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .text_color(muted)
                                        .child("Cannot display image")
                                        .into_any_element()
                                }),
                        )
                        .into_any_element()
                } else {
                    let (width, height) = dimensions.unwrap_or_default();
                    div()
                        .min_w_full()
                        .min_h_full()
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(
                            img(source(image))
                                .w(px(width as f32 / factor * view.zoom))
                                .h(px(height as f32 / factor * view.zoom))
                                .with_fallback(move || {
                                    div()
                                        .size_full()
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .text_color(muted)
                                        .child("Cannot display image")
                                        .into_any_element()
                                }),
                        )
                        .into_any_element()
                };

                return div()
                    .id("image-view")
                    .size_full()
                    .bg(cx.theme().background)
                    .overflow_scroll()
                    // ⌘-scroll zooms; a plain scroll pans the zoomed image.
                    .on_scroll(cx.listener(|this, event: &ScrollEvent, _window, cx| {
                        if !(event.modifiers.platform || event.modifiers.control) {
                            return;
                        }
                        let direction = match event.delta {
                            ScrollDelta::Lines(point) => point.y,
                            ScrollDelta::Pixels(point) => point.y.as_f32() / 40.,
                        };
                        if direction == 0. {
                            return;
                        }
                        this.step_image_zoom(direction.signum());
                        cx.stop_propagation();
                        cx.notify();
                    }))
                    // Double-click toggles fit and actual size.
                    .on_click(cx.listener(|this, event: &ClickEvent, _window, cx| {
                        if event.click_count() >= 2 {
                            this.toggle_image_fit();
                            cx.notify();
                        }
                    }))
                    .child(body)
                    .into_any_element();
            }
            None => {}
        }
        match tab {
            Some(tab) => {
                let has_path = tab.document.path.is_some();
                div()
                    .size_full()
                    .bg(cx.theme().background)
                    // Enter and double-click open the match under the caret; the
                    // context lets the Enter binding beat the editor's newline.
                    .when(tab.find_results.is_some(), |panel| {
                        panel
                            .key_context("FindResults")
                            .capture_any_mouse_down(cx.listener(
                                |_this, event: &MouseDownEvent, window, cx| {
                                    // The first click already put the caret on the
                                    // line. Deferred so the editor finishes handling
                                    // the press, and taking focus, before the jump.
                                    if event.click_count == 2 {
                                        cx.defer_in(window, |this, window, cx| {
                                            this.open_result_at_caret(window, cx);
                                        });
                                    }
                                },
                            ))
                    })
                    .child(
                        Editor::new(&tab.document.editor)
                            .appearance(false)
                            .bordered(false)
                            // The element sets this on the state every render.
                            .readonly(tab.find_results.is_some())
                            .when_some(
                                self.settings.values.editor.font_family.clone(),
                                |editor, family| editor.font_family(family),
                            )
                            .when_some(self.settings.values.editor.font_size(), |editor, size| {
                                editor.text_size(px(size))
                            })
                            .when_some(
                                self.settings.values.editor.font_weight(),
                                |editor, weight| editor.font_weight(FontWeight::from(weight)),
                            )
                            .context_menu(move |menu, _, _| {
                                // Built while the editor state is mid-update, so it must not
                                // read that state. Cut and Copy no-op without a selection.
                                menu.menu("Cut", Box::new(input::Cut))
                                    .menu("Copy", Box::new(input::Copy))
                                    .menu("Paste", Box::new(input::Paste))
                                    .separator()
                                    .menu("Select All", Box::new(input::SelectAll))
                                    .separator()
                                    .menu("Add Cursor Above", Box::new(AddCursorAbove))
                                    .menu("Add Cursor Below", Box::new(AddCursorBelow))
                                    .separator()
                                    .menu_with_disabled(
                                        "Reveal in Finder",
                                        !has_path,
                                        Box::new(RevealInFinder),
                                    )
                                    .menu_with_disabled(
                                        "Copy Path",
                                        !has_path,
                                        Box::new(CopyFilePath),
                                    )
                                    .menu_with_disabled(
                                        "Copy Relative Path",
                                        !has_path,
                                        Box::new(CopyRelativePath),
                                    )
                            })
                            .h_full(),
                    )
                    .into_any_element()
            }
            None => {
                let muted = cx.theme().muted_foreground;
                let hints = [
                    ("New File", Kbd::global_binding_for_action(&NewFile, window)),
                    (
                        "Open File…",
                        Kbd::global_binding_for_action(&OpenFile, window),
                    ),
                    (
                        "Add Folder…",
                        Kbd::global_binding_for_action(&AddFolder, window),
                    ),
                ];
                div()
                    .v_flex()
                    .size_full()
                    .items_center()
                    .justify_center()
                    .gap_8()
                    .child(super::logo(128.).grayscale(true).opacity(0.5))
                    .child(div().v_flex().gap_2().children(hints.into_iter().map(
                        |(label, kbd)| {
                            div()
                                .h_flex()
                                .items_center()
                                .gap_6()
                                .child(div().w(px(112.)).text_color(muted).child(label))
                                .child(
                                    kbd.map(|kbd| kbd.into_any_element())
                                        .unwrap_or_else(|| div().into_any_element()),
                                )
                        },
                    )))
                    .into_any_element()
            }
        }
    }

    pub(super) fn render_status_bar(
        &self,
        alt_held: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let entity = cx.entity();
        let language = self.active_language();
        let dirty = self.active_dirty();
        let deleted = self
            .active()
            .is_some_and(|tab| tab.document.deleted_on_disk);
        let divider = || div().h_3().w(px(1.)).flex_none().bg(cx.theme().border);

        let language_button = Button::new("language-mode")
            .xsmall()
            .ghost()
            .label(language)
            .dropdown_menu(move |menu, _, _| {
                LANGUAGES.iter().fold(menu, |menu, name| {
                    let entity = entity.clone();
                    let value: SharedString = (*name).into();
                    let item = PopupMenuItem::new(value.clone()).on_click(move |_, _, cx| {
                        let value = value.clone();
                        entity.update(cx, |this, cx| {
                            if let Some(tab) = this
                                .tabs
                                .active_index()
                                .and_then(|ix| this.tabs.get_mut(ix))
                            {
                                tab.document.language = value.clone();
                                tab.document.language_override = true;
                                tab.document.editor.update(cx, |state, cx| {
                                    state.set_highlighter(editor_language_id(&value), cx);
                                });
                                cx.notify();
                            }
                        });
                    });
                    menu.item(item)
                })
            });

        let save_status = if deleted {
            div()
                .h_flex()
                .items_center()
                .gap_1()
                .text_color(cx.theme().danger)
                .child(Icon::new(IconName::CircleAlert).with_size(Size::XSmall))
                .child("Deleted on Disk")
                .into_any_element()
        } else if dirty {
            div()
                .h_flex()
                .items_center()
                .gap_1()
                .child(
                    Badge::new()
                        .dot()
                        .child(Icon::new(IconName::FileText).with_size(Size::XSmall)),
                )
                .child("Unsaved")
                .into_any_element()
        } else {
            div()
                .h_flex()
                .items_center()
                .gap_1()
                .child(Icon::new(IconName::CircleCheck).with_size(Size::XSmall))
                .child("Saved")
                .into_any_element()
        };

        let mut status = StatusBar::new();
        if let Some(image) = self.active_raster() {
            return status.left(div().child(image_summary(image)));
        }
        if let Some(label) = self.chrome.busy().cloned() {
            status = status.left(
                div()
                    .h_flex()
                    .items_center()
                    .gap_2()
                    .child(Spinner::new().with_size(Size::XSmall))
                    .child(label),
            );
        }
        if alt_held {
            status = status.left(
                div()
                    .h_flex()
                    .items_center()
                    .gap_1()
                    .text_color(cx.theme().muted_foreground)
                    .child("⌥ Click adds a cursor · ⌥⇧ Drag selects a column"),
            );
        }

        let (cursor_line, cursor_col) = self.chrome.cursor();
        let caret = match self.chrome.selected() {
            0 => format!("Ln {cursor_line}, Col {cursor_col}"),
            selected => format!("Ln {cursor_line}, Col {cursor_col} ({selected} selected)"),
        };
        let format = self
            .active()
            .map(|tab| tab.document.format)
            .unwrap_or_default();

        status = status
            .left(div().child(caret))
            .left(divider())
            .left(div().child(format.encoding_label()))
            .left(divider())
            .left(div().child(format.line_ending_label()));
        if let Some((words, chars)) = self.chrome.counts() {
            status = status
                .left(divider())
                .left(div().child(count_label(words, chars)));
        }
        // The branch of the repository the active tab lives in, or of the only
        // workspace folder when the tab has no path yet.
        let branch = self
            .workspace
            .branch_for(self.active().and_then(|tab| tab.document.path.as_deref()));
        if let Some(branch) = branch {
            status = status
                .right(
                    div()
                        .h_flex()
                        .items_center()
                        .gap_1()
                        // A line-art branch glyph, monochrome: the Git logo is
                        // a solid shape that turns into a blob at this size.
                        .child(
                            Icon::empty()
                                .data(GIT_BRANCH_ICON)
                                .with_size(Size::XSmall)
                                .flex_none(),
                        )
                        .child(
                            div()
                                // A long branch name must not crowd out the
                                // rest of the status bar.
                                .max_w(px(200.))
                                .truncate()
                                .child(branch_label(branch)),
                        ),
                )
                .right(divider());
        }
        status
            .right(language_button)
            .right(divider())
            .right(save_status)
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::sync::Arc;

    use gpui_kit::test::{TestQueryExt as _, TestWindowExt as _};
    use gpui_kit::{
        AppContext as _, DevicePixels, RenderImage, SvgRenderer, SvgSize, TestAppContext, hsla, px,
    };

    use crate::app::test_support::open_app_in;
    use crate::file_icons::file_icon;
    use crate::git::Branch;

    use super::COLOR_ICON_PIXELS;
    use super::{GIT_BRANCH_ICON, branch_label, change_tint, count_label, grouped};

    /// A workspace with one source file, for the scenes that need an editor.
    fn workspace_with_a_file(name: &str) -> (std::path::PathBuf, std::path::PathBuf) {
        let root = std::env::temp_dir().join(format!("teak-smoke-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("main.rs");
        std::fs::write(&path, "fn main() {\n    println!(\"hello\");\n}\n").unwrap();
        (root, path)
    }

    /// The editor scene paints its three fixed surfaces, and each one lands
    /// with a non-zero size. gpui-kit 0.8 has no pixel capture, so this is the
    /// closest a headless run gets to a screenshot: an empty pane, a collapsed
    /// tab strip or a sidebar that failed to lay out all show up here.
    #[gpui_kit::test]
    async fn the_editor_scene_paints_its_surfaces(cx: &mut TestAppContext) {
        let (root, path) = workspace_with_a_file("editor");
        let (handle, app) = open_app_in(cx, vec![root.clone()]);
        cx.update_window(handle, |_, window, cx| {
            app.update(cx, |this, cx| {
                let content = crate::app::files::read_file(&path).unwrap();
                this.add_file_tab(path.clone(), content, false, window, cx);
            });
            // The first frame mounts the tab, the second paints its editor.
            window.render_frame(cx);
            window.render_frame(cx);
        })
        .unwrap();

        cx.update_window(handle, |_, window, _| {
            let painted: Vec<_> = window
                .elements()
                .into_iter()
                .filter(|element| element.visible() && element.bounds().size.height > px(0.))
                .collect();
            // Surfaces are checked by id path: a container that wraps children
            // is not itself registered, so `find` cannot reach the tab strip.
            let laid_out = |needle: &str| {
                painted.iter().any(|element| {
                    format!("{:?}", element.path()).contains(needle)
                        && element.bounds().size.width > px(0.)
                })
            };
            for surface in ["workspace-tab-bar", "explorer-scroll", "sidebar-header"] {
                assert!(laid_out(surface), "{surface} must paint with a size");
            }
            // The editor surface: the kit registers the text area under its
            // input path, so a painted editor proves the content pane is not
            // blank even though teak gives it no id of its own.
            assert!(
                laid_out("input::editor::Editor"),
                "the editor's text area must paint with a size",
            );
            assert!(painted.len() >= 5, "the window painted almost nothing");
        })
        .unwrap();

        let _ = std::fs::remove_dir_all(&root);
    }

    /// The command palette paints its search field over the editor.
    #[gpui_kit::test]
    async fn the_palette_scene_paints_its_search_field(cx: &mut TestAppContext) {
        let (root, path) = workspace_with_a_file("palette");
        let (handle, app) = open_app_in(cx, vec![root.clone()]);
        cx.update_window(handle, |_, window, cx| {
            app.update(cx, |this, cx| {
                let content = crate::app::files::read_file(&path).unwrap();
                this.add_file_tab(path.clone(), content, false, window, cx);
            });
            window.render_frame(cx);
            window.press("secondary-shift-p", cx);
            window.render_frame(cx);
            if let Some(palette) = app.read(cx).overlays.palette().cloned() {
                palette.update(cx, |state, cx| state.focus(window, cx));
            }
            window.render_frame(cx);
        })
        .unwrap();

        cx.update_window(handle, |_, window, cx| {
            assert!(app.read(cx).overlays.is_palette_open());
            let field = window
                .elements()
                .into_iter()
                .find(|element| element.placeholder() == Some("Type a command…"))
                .expect("the palette's search field must be painted");
            assert!(field.visible());
            assert!(field.bounds().size.width > px(100.));
        })
        .unwrap();

        let _ = std::fs::remove_dir_all(&root);
    }

    /// The find-in-files panel paints its controls below the editor.
    #[gpui_kit::test]
    async fn the_find_in_files_scene_paints_its_controls(cx: &mut TestAppContext) {
        let (root, path) = workspace_with_a_file("find");
        let (handle, app) = open_app_in(cx, vec![root.clone()]);
        cx.update_window(handle, |_, window, cx| {
            app.update(cx, |this, cx| {
                let content = crate::app::files::read_file(&path).unwrap();
                this.add_file_tab(path.clone(), content, false, window, cx);
            });
            window.render_frame(cx);
            window.press("secondary-shift-f", cx);
            window.render_frame(cx);
            window.render_frame(cx);
        })
        .unwrap();

        cx.update_window(handle, |_, window, cx| {
            assert!(
                app.read(cx).overlays.find().is_some(),
                "Cmd+Shift+F must open the find panel",
            );
            let run = window.find("find-run");
            assert!(run.visible());
            assert!(run.bounds().size.height > px(0.));
        })
        .unwrap();

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn an_image_summary_names_its_format_and_pixels() {
        let image = gpui_kit::Image::from_bytes(
            gpui_kit::ImageFormat::Png,
            super::super::files::ONE_PIXEL_PNG.to_vec(),
        );
        assert_eq!(super::image_summary(&image), "PNG · 1 × 1 · 70 B");
    }

    #[test]
    fn the_status_bar_git_glyph_is_a_valid_svg() {
        let renderer = SvgRenderer::new(Arc::new(()));
        assert!(renderer.parse_svg(GIT_BRANCH_ICON).is_ok());
    }

    #[test]
    fn branch_labels_show_divergence_only_when_there_is_some() {
        let branch = |name: &str, ahead, behind| Branch {
            name: name.to_string(),
            ahead,
            behind,
        };

        assert_eq!(branch_label(&branch("main", 0, 0)), "main");
        assert_eq!(branch_label(&branch("main", 2, 0)), "main ↑2");
        assert_eq!(branch_label(&branch("main", 0, 3)), "main ↓3");
        assert_eq!(branch_label(&branch("main", 2, 3)), "main ↑2 ↓3");
    }

    #[test]
    fn change_tint_halves_saturation_and_meets_the_foreground_halfway() {
        let tint = change_tint(hsla(0.25, 0.5, 0.5, 1.), hsla(0., 0., 0.75, 1.));

        assert_eq!(tint.h, 0.25);
        assert_eq!(tint.s, 0.25);
        assert_eq!(tint.l, 0.625);
        assert_eq!(tint.a, 1.);
    }

    fn rasterize(bytes: &'static [u8]) -> Arc<RenderImage> {
        let renderer = SvgRenderer::new(Arc::new(()));
        let parsed = renderer.parse_svg(bytes).unwrap();
        let size = gpui_kit::Size::new(
            DevicePixels(COLOR_ICON_PIXELS),
            DevicePixels(COLOR_ICON_PIXELS),
        );
        renderer
            .render_parsed(&parsed, SvgSize::Size(size))
            .unwrap()
    }

    #[test]
    fn colorful_icons_are_square_at_the_cached_size() {
        // Icons ship at different intrinsic sizes (16px and 32px viewBoxes); the
        // requested size must yield a uniform square raster either way.
        for bytes in [
            file_icon(Path::new("main.rs")),
            file_icon(Path::new("notes.txt")),
        ] {
            let image = rasterize(bytes);
            assert_eq!(
                image.size(0),
                gpui_kit::Size::new(
                    DevicePixels(COLOR_ICON_PIXELS),
                    DevicePixels(COLOR_ICON_PIXELS)
                ),
            );
        }
    }

    #[test]
    fn colorful_icons_keep_the_svg_fill_color() {
        // Rust's icon is brand orange (#ff7043). The tinted `svg` element would
        // flatten it to one theme color, so a red-leaning opaque pixel proves
        // the image path preserved the fill. RenderImage bytes are BGRA.
        let image = rasterize(file_icon(Path::new("main.rs")));
        let (pixels, _) = image.as_bytes(0).unwrap().as_chunks::<4>();
        assert!(
            pixels
                .iter()
                .any(|pixel| pixel[3] > 0 && pixel[2] > pixel[0].saturating_add(40)),
        );
    }

    #[test]
    fn groups_thousands() {
        assert_eq!(grouped(0), "0");
        assert_eq!(grouped(999), "999");
        assert_eq!(grouped(1_000), "1,000");
        assert_eq!(grouped(5_678), "5,678");
        assert_eq!(grouped(1_234_567), "1,234,567");
    }

    #[test]
    fn labels_words_and_chars() {
        assert_eq!(count_label(1_234, 5_678), "1,234 words · 5,678 chars");
        assert_eq!(count_label(1, 1), "1 word · 1 char");
        assert_eq!(count_label(0, 0), "0 words · 0 chars");
        // Past the limit only the character count is kept.
        assert_eq!(count_label(0, 1_000_001), "1,000,001 chars");
        assert_eq!(count_label(1, 1_000_001), "1,000,001 chars");
    }
}
