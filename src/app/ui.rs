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
use crate::language::{LANGUAGES, editor_language_id};
use crate::paths::{display_path, file_name, format_size, reveal_label};
use crate::settings::IconStyle;
use crate::{
    AddCursorAbove, AddCursorBelow, AddFolder, CopyFilePath, CopyRelativePath, NewFile, OpenFile,
    RevealInFinder,
};

use super::workspace::{SIDEBAR_MAX_WIDTH, SIDEBAR_MIN_WIDTH};
use super::{Preview, TinytextApp};

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

/// Logical edge length of a file-type icon, matching `Size::Small` (0.875rem).
const FILE_ICON_SIZE: f32 = 14.;
/// Rasterized edge length of a colorful icon. Twice [`FILE_ICON_SIZE`] keeps it
/// sharp on Retina displays; the image element downsamples it to fit.
const COLOR_ICON_PIXELS: i32 = 28;

thread_local! {
    /// Full-color rasterizations keyed by the address of their static SVG bytes.
    /// `RenderImage::new` mints a fresh id every call, so reusing one cached
    /// image is what keeps the sprite atlas from growing on every frame.
    static COLOR_ICON_CACHE: RefCell<HashMap<usize, Arc<RenderImage>>> =
        RefCell::new(HashMap::new());
}

/// A file-type icon for the sidebar and tabs. GPUI's `svg` element — and the
/// `Icon` component built on it — flattens an SVG to a single tinted color, so
/// the colorful style draws the vendored SVG through the image pipeline
/// instead, which preserves its fills.
pub(super) fn file_type_icon(bytes: &'static [u8], style: IconStyle, cx: &App) -> AnyElement {
    if style == IconStyle::Colorful
        && let Some(image) = colored_icon(bytes, cx)
    {
        return img(image)
            .w(px(FILE_ICON_SIZE))
            .h(px(FILE_ICON_SIZE))
            .flex_none()
            .into_any_element();
    }

    Icon::empty()
        .data(bytes)
        .with_size(Size::Small)
        .flex_none()
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

impl TinytextApp {
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
                let deleted = tab.path.as_deref().is_some_and(|path| !path.exists());
                // Full path on hover, telling apart titles the ellipsis cuts alike.
                let tooltip: SharedString = match &tab.path {
                    Some(path) => display_path(path).into(),
                    None => tab.title.clone(),
                };
                let (dot, dot_color) = if deleted {
                    (true, deleted_color)
                } else {
                    (tab.dirty, dirty_color)
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
                        let path = tab.path.as_deref().unwrap_or(Path::new(""));
                        this.child(file_type_icon(file_icon(path), icon_style, cx))
                    })
                    .child(
                        div()
                            .min_w_0()
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .child(tab.title.clone()),
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
                            id: tab.editor.entity_id(),
                            title: tab.title.clone(),
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
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.activate_tab(ix, window, cx);
                    }))
                    .on_mouse_down(
                        MouseButton::Right,
                        cx.listener(move |this, _, _, _cx| {
                            this.tabs.set_context(Some(ix));
                        }),
                    )
            })
            .collect();

        let titles: Vec<SharedString> = self.tabs.iter().map(|tab| tab.title.clone()).collect();
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
                        app.tabs
                            .iter()
                            .any(|tab| tab.path.as_deref().is_some_and(|path| !path.exists())),
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

                menu.item(
                    PopupMenuItem::new("Close Tab").on_click(move |_, window, cx| {
                        close_entity.update(cx, |this, cx| this.close_tab(ix, window, cx));
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
                            this.save_session();
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
        let expanded = self.workspace.is_expanded(path);
        let muted = cx.theme().muted_foreground;

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
        let type_icon = if is_dir {
            file_type_icon(folder_icon(), icon_style, cx)
        } else {
            file_type_icon(file_icon(path), icon_style, cx)
        };

        let click_path = path.to_path_buf();
        let label = file_name(path);
        let menu_entity = cx.entity();
        let menu_path = path.to_path_buf();

        ListItem::new(path.display().to_string())
            .pl(px(8. + depth as f32 * 12.))
            .pr_2()
            .py_0p5()
            .selected(selected)
            .accessibility_label(label.clone())
            .child(
                div()
                    .h_flex()
                    .items_center()
                    .gap_1()
                    .child(leading)
                    .child(type_icon)
                    .child(div().text_sm().whitespace_nowrap().child(label)),
            )
            .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                this.workspace.set_selected(Some(click_path.clone()));
                if is_dir {
                    this.workspace.toggle_expanded(&click_path);
                } else if event.click_count() == 1 {
                    this.request_open(click_path.clone(), window, cx);
                }
                this.save_session();
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
                        PopupMenuItem::new(reveal_label()).on_click(move |_, _window, cx| {
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
                return div()
                    .size_full()
                    .p_6()
                    .bg(cx.theme().background)
                    .child(
                        img(ImageSource::Image(image.clone()))
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
                    .into_any_element();
            }
            None => {}
        }
        match tab {
            Some(tab) => {
                let has_path = tab.path.is_some();
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
                        Editor::new(&tab.editor)
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
                                        reveal_label(),
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
                                tab.language = value.clone();
                                tab.language_override = true;
                                tab.editor.update(cx, |state, cx| {
                                    state.set_highlighter(editor_language_id(&value), cx);
                                });
                                cx.notify();
                            }
                        });
                    });
                    menu.item(item)
                })
            });

        let save_status = if dirty {
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
            return status.left(div().child(format!(
                "{} · {}",
                image.format.extension().to_ascii_uppercase(),
                format_size(image.bytes.len())
            )));
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
        let format = self.active().map(|tab| tab.format).unwrap_or_default();

        status
            .left(div().child(format!("Ln {cursor_line}, Col {cursor_col}")))
            .left(divider())
            .left(div().child(format.encoding_label()))
            .left(divider())
            .left(div().child(format.line_ending_label()))
            .right(language_button)
            .right(divider())
            .right(save_status)
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::sync::Arc;

    use gpui_kit::{DevicePixels, RenderImage, SvgRenderer, SvgSize};

    use crate::file_icons::file_icon;

    use super::COLOR_ICON_PIXELS;

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
}
