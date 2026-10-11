//! The command palette: a searchable list of the app's actions over the editor.
//!
//! The palette is rendered as an in-app overlay rather than a native dialog on
//! purpose. Its rows dispatch the very same actions the menus do, and an action
//! only reaches the root's `on_action` handlers if the overlay sits inside that
//! element tree. A dialog layer is a sibling of the app content, so its actions
//! would propagate past those handlers and never fire.
//!
//! The list and the search field come from gpui-kit's `command` component; this
//! module only supplies the entries, hosts them, and restores focus on close.

use gpui_kit::component::{
    ActiveTheme as _,
    command::{Command, CommandGroup, CommandItem, CommandState},
};
use gpui_kit::*;

use crate::{
    About, AddCursorAbove, AddCursorBelow, AddFolder, CheckForUpdates, CloseAllFolders, CloseTab,
    CommandPalette, CopyFilePath, CopyLineDown, CopyLineUp, CopyRelativePath, DeleteLine, GoToLine,
    InsertLineAbove, InsertLineBelow, MoveLineDown, MoveLineUp, NewFile, NextTab, OpenFile,
    OpenSettings, PreviousTab, Refresh, ReopenClosedTab, RevealInFinder, SaveAll, SaveFile,
    SaveFileAs, SelectLine, SetTheme, ToggleLineComment, ToggleSidebar, ToggleWhitespace,
    ToggleWordWrap, ZoomIn, ZoomOut, ZoomReset,
};

use super::TinytextApp;

/// Width of the palette panel.
const PALETTE_WIDTH: Pixels = px(560.);
/// Gap between the top of the window and the palette panel.
const PALETTE_TOP: Pixels = px(72.);

/// The actions the palette offers, grouped like the menu bar. Each item carries
/// its action, so the default row also shows that action's keybinding.
fn palette_groups() -> Vec<CommandGroup> {
    let item = |label: &'static str, action: Box<dyn Action>| {
        CommandItem::new().label(label).action(action)
    };

    vec![
        CommandGroup::new().label("File").items([
            item("New File", Box::new(NewFile)),
            item("Open…", Box::new(OpenFile)),
            item("Add Folder…", Box::new(AddFolder)),
            item("Close All Folders", Box::new(CloseAllFolders)),
            item("Save", Box::new(SaveFile)),
            item("Save As…", Box::new(SaveFileAs)),
            item("Save All", Box::new(SaveAll)),
            item("Close Tab", Box::new(CloseTab)),
            item("Reopen Closed Tab", Box::new(ReopenClosedTab)),
            item("Reveal in Finder", Box::new(RevealInFinder)),
        ]),
        CommandGroup::new().label("Edit").items([
            item("Toggle Line Comment", Box::new(ToggleLineComment)),
            item("Delete Line", Box::new(DeleteLine)),
            item("Select Line", Box::new(SelectLine)),
            item("Move Line Up", Box::new(MoveLineUp)),
            item("Move Line Down", Box::new(MoveLineDown)),
            item("Copy Line Up", Box::new(CopyLineUp)),
            item("Copy Line Down", Box::new(CopyLineDown)),
            item("Insert Line Above", Box::new(InsertLineAbove)),
            item("Insert Line Below", Box::new(InsertLineBelow)),
            CommandItem::new()
                .label("Go to Line…")
                .keywords(["goto"])
                .action(Box::new(GoToLine)),
            item("Add Cursor Above", Box::new(AddCursorAbove)),
            item("Add Cursor Below", Box::new(AddCursorBelow)),
            item("Copy Path", Box::new(CopyFilePath)),
            item("Copy Relative Path", Box::new(CopyRelativePath)),
        ]),
        CommandGroup::new().label("View").items([
            item("Toggle Sidebar", Box::new(ToggleSidebar)),
            item("Refresh", Box::new(Refresh)),
            item("Zoom In", Box::new(ZoomIn)),
            item("Zoom Out", Box::new(ZoomOut)),
            item("Actual Size", Box::new(ZoomReset)),
            item("Toggle Word Wrap", Box::new(ToggleWordWrap)),
            item("Toggle Invisible Characters", Box::new(ToggleWhitespace)),
            item("Next Tab", Box::new(NextTab)),
            item("Previous Tab", Box::new(PreviousTab)),
        ]),
        CommandGroup::new()
            .label("Appearance")
            .items(crate::THEME_CHOICES.iter().map(|(value, label)| {
                CommandItem::new()
                    .label(*label)
                    .keywords(["theme", "appearance"])
                    .action(Box::new(SetTheme((*value).to_string())))
            })),
        CommandGroup::new().label("Tinytext").items([
            CommandItem::new()
                .label("Settings…")
                .keywords(["preferences", "config"])
                .action(Box::new(OpenSettings)),
            item("Check for Updates…", Box::new(CheckForUpdates)),
            item("About Tinytext", Box::new(About)),
        ]),
    ]
}

impl TinytextApp {
    /// Opens the palette, or closes it when it is already open.
    pub(super) fn on_command_palette(
        &mut self,
        _: &CommandPalette,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.palette.is_some() {
            self.close_palette(window, cx);
        } else {
            self.open_palette(window, cx);
        }
    }

    fn open_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let state = cx.new(|cx| CommandState::new(window, cx));
        // Remember what held focus so it can be restored once the palette closes.
        self.palette_return_focus = window.focused(cx);
        self.palette = Some(state.clone());
        cx.notify();
        // The query input only exists in the tree after the first render, so the
        // focus request is deferred until the overlay has mounted.
        window.defer(cx, move |window, cx| {
            state.update(cx, |state, cx| state.focus(window, cx));
        });
    }

    fn close_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(state) = self.palette.take() else {
            return;
        };
        // Restore focus only while the palette still holds it. An action that
        // opened its own dialog (Open, Go to Line) has already moved focus
        // there, and stealing it back would break that dialog.
        let command_handle = state.read(cx).focus_handle(cx);
        if window.focused(cx).as_ref() == Some(&command_handle)
            && let Some(previous) = self.palette_return_focus.take()
        {
            window.focus(&previous, cx);
        } else {
            self.palette_return_focus = None;
        }
        cx.notify();
    }

    pub(super) fn render_palette(
        &self,
        state: &Entity<CommandState>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let confirm = cx.entity();
        let cancel = confirm.clone();
        let backdrop = confirm.clone();

        let mut command = Command::new(state)
            .placeholder("Type a command…")
            .empty(|_, _, cx| {
                div()
                    .p_3()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child("No matching commands")
            })
            .on_confirm(move |_, window, cx| {
                confirm.update(cx, |this, cx| this.close_palette(window, cx));
            })
            .on_cancel(move |window, cx| {
                cancel.update(cx, |this, cx| this.close_palette(window, cx));
            });
        for group in palette_groups() {
            command = command.group(group);
        }

        div()
            .absolute()
            .inset_0()
            .occlude()
            .flex()
            .items_start()
            .justify_center()
            .pt(PALETTE_TOP)
            .bg(cx.theme().overlay)
            .on_mouse_down(MouseButton::Left, move |_, window, cx| {
                backdrop.update(cx, |this, cx| this.close_palette(window, cx));
            })
            .child(
                div()
                    .w(PALETTE_WIDTH)
                    // Keep clicks on the panel from reaching the backdrop.
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .child(command),
            )
            .into_any_element()
    }
}
