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
    command::{Command, CommandState},
};
use gpui_kit::*;

use crate::CommandPalette;

use super::TeakApp;

/// Width of the palette panel.
const PALETTE_WIDTH: Pixels = px(560.);
/// Gap between the top of the window and the palette panel.
const PALETTE_TOP: Pixels = px(72.);

impl TeakApp {
    /// Opens the palette, or closes it when it is already open.
    pub(super) fn on_command_palette(
        &mut self,
        _: &CommandPalette,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.overlays.is_palette_open() {
            self.close_palette(window, cx);
        } else {
            self.open_palette(window, cx);
        }
    }

    fn open_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // Keep the quick-open finder and the palette from stacking.
        if self.overlays.is_quick_open_open() {
            self.close_quick_open(window, cx);
        }

        let state = cx.new(|cx| CommandState::new(window, cx));
        // Remember what held focus so it can be restored once the palette closes.
        self.overlays.set_palette(state.clone(), window.focused(cx));
        cx.notify();
        // The query input only exists in the tree after the first render, so the
        // focus request is deferred until the overlay has mounted.
        window.defer(cx, move |window, cx| {
            state.update(cx, |state, cx| state.focus(window, cx));
        });
    }

    pub(super) fn close_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((state, return_focus)) = self.overlays.take_palette() else {
            return;
        };
        // Restore focus only while the palette still holds it. An action that
        // opened its own dialog (Open, Go to Line) has already moved focus
        // there, and stealing it back would break that dialog.
        let command_handle = state.read(cx).focus_handle(cx);
        if window.focused(cx).as_ref() == Some(&command_handle)
            && let Some(previous) = return_focus
        {
            window.focus(&previous, cx);
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
            .on_cancel({
                let cancel = cancel.clone();
                move |window, cx| {
                    // The callback runs while the palette's Cancel dispatch is
                    // still in progress, so closing here would read the palette
                    // state while it is being updated. Hand the dismissal to
                    // the next frame instead.
                    let cancel = cancel.clone();
                    window.defer(cx, move |window, cx| {
                        cancel.update(cx, |this, cx| this.close_palette(window, cx));
                    });
                }
            });
        // Declared once in `commands.rs`; each row carries its action, so the
        // default row also shows that action's keybinding.
        for group in crate::commands::palette_groups() {
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

#[cfg(test)]
mod tests {
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

            window.press("cmd-shift-p", cx);
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
}
