//! Registers the app's key bindings.
//!
//! Bindings are split in two groups. The app bindings are global and match
//! regardless of focus. The editor bindings use gpui-kit's `"Input"` key
//! context, so they only fire while a text editor has focus. The editor
//! bindings are registered after `gpui_kit::init` has installed the engine's
//! own defaults; at equal context depth the later registration wins, so these
//! take precedence where they overlap with the engine.
//!
//! The bindings themselves are declared once in `commands.rs`, next to the
//! label and menu placement each command is shown under; this module only fixes
//! the order they are registered in.

use gpui_kit::*;

use crate::commands;

/// Registers every binding. Call once during startup, after `gpui_kit::init`.
pub(crate) fn bind_keys(cx: &mut App) {
    cx.bind_keys(commands::app_bindings());
    cx.bind_keys(commands::editor_bindings());
}
