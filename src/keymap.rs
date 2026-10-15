//! Every key binding in the app.
//!
//! Bindings are split in two groups. The app bindings are global and match
//! regardless of focus. The editor bindings use gpui-kit's `"Input"` key
//! context, so they only fire while a text editor has focus. The editor
//! bindings are registered after `gpui_kit::init` has installed the engine's
//! own defaults; at equal context depth the later registration wins, so these
//! take precedence where they overlap with the engine.

use gpui_kit::*;

use crate::{
    AddCursorAbove, AddCursorBelow, AddFolder, AddNextOccurrence, CloseAllFolders, CloseTab,
    CommandPalette, CopyFilePath, CopyLineDown, CopyLineUp, CopyRelativePath, DeleteLine, GoToLine,
    Hide, HideOthers, InsertLineAbove, InsertLineBelow, JumpToTab, MoveLineDown, MoveLineUp,
    NewFile, NextTab, OpenFile, OpenSettings, PreviousTab, QuickOpen, Quit, Refresh,
    ReopenClosedTab, RevealInFinder, SaveAll, SaveFile, SaveFileAs, SelectLine,
    SplitSelectionIntoLines, ToggleLineComment, ToggleMarkdownPreview, ToggleSidebar,
    ToggleWhitespace, ToggleWordWrap, ZoomIn, ZoomOut, ZoomReset,
};

/// gpui-kit's editor key context. Its `CONTEXT` constant is private, so the
/// literal it uses is repeated here.
const EDITOR: &str = "Input";

/// Registers every binding. Call once during startup, after `gpui_kit::init`.
pub(crate) fn bind_keys(cx: &mut App) {
    cx.bind_keys(app_bindings());
    cx.bind_keys(editor_bindings());
}

/// Bindings that are active regardless of which element has focus.
fn app_bindings() -> Vec<KeyBinding> {
    vec![
        KeyBinding::new("cmd-n", NewFile, None),
        KeyBinding::new("cmd-o", OpenFile, None),
        KeyBinding::new("cmd-p", QuickOpen, None),
        KeyBinding::new("cmd-shift-o", AddFolder, None),
        KeyBinding::new("alt-cmd-w", CloseAllFolders, None),
        KeyBinding::new("cmd-s", SaveFile, None),
        KeyBinding::new("cmd-shift-s", SaveFileAs, None),
        KeyBinding::new("alt-cmd-s", SaveAll, None),
        KeyBinding::new("cmd-w", CloseTab, None),
        KeyBinding::new("cmd-q", Quit, None),
        KeyBinding::new("cmd-h", Hide, None),
        KeyBinding::new("alt-cmd-h", HideOthers, None),
        KeyBinding::new("alt-cmd-r", RevealInFinder, None),
        KeyBinding::new("alt-cmd-c", CopyFilePath, None),
        KeyBinding::new("alt-cmd-shift-c", CopyRelativePath, None),
        KeyBinding::new("cmd-b", ToggleSidebar, None),
        KeyBinding::new("cmd-shift-p", CommandPalette, None),
        KeyBinding::new("cmd-r", Refresh, None),
        KeyBinding::new("cmd-,", OpenSettings, None),
        KeyBinding::new("cmd-=", ZoomIn, None),
        KeyBinding::new("cmd-shift-=", ZoomIn, None),
        KeyBinding::new("cmd--", ZoomOut, None),
        KeyBinding::new("cmd-0", ZoomReset, None),
        KeyBinding::new("alt-cmd-i", ToggleWhitespace, None),
        KeyBinding::new("cmd-shift-v", ToggleMarkdownPreview, None),
        KeyBinding::new("cmd-shift-t", ReopenClosedTab, None),
        KeyBinding::new("cmd-shift-]", NextTab, None),
        KeyBinding::new("cmd-shift-[", PreviousTab, None),
        KeyBinding::new("cmd-1", JumpToTab(1), None),
        KeyBinding::new("cmd-2", JumpToTab(2), None),
        KeyBinding::new("cmd-3", JumpToTab(3), None),
        KeyBinding::new("cmd-4", JumpToTab(4), None),
        KeyBinding::new("cmd-5", JumpToTab(5), None),
        KeyBinding::new("cmd-6", JumpToTab(6), None),
        KeyBinding::new("cmd-7", JumpToTab(7), None),
        KeyBinding::new("cmd-8", JumpToTab(8), None),
        KeyBinding::new("cmd-9", JumpToTab(9), None),
    ]
}

/// VS Code's macOS editing chords, scoped to the focused editor.
///
/// Because these are registered after the engine's own `"Input"` bindings, a
/// few of them deliberately shadow engine defaults:
/// - `shift-alt-up` / `shift-alt-down` — VS Code copies the line, replacing the
///   engine's alternate add-cursor chord.
/// - `cmd-alt-up` / `cmd-alt-down` (and the `ctrl-alt-*` alternate) — the engine
///   binds these to its own add-cursor actions; the app binds the same chords to
///   its `AddCursorAbove` / `AddCursorBelow` so the menu and palette share one
///   path (the handler re-dispatches the engine action at the editor).
/// - `cmd-enter` — VS Code inserts a line below; the engine maps
///   `secondary-enter` to a plain newline on macOS, so this is the useful one.
fn editor_bindings() -> Vec<KeyBinding> {
    vec![
        KeyBinding::new("cmd-/", ToggleLineComment, Some(EDITOR)),
        KeyBinding::new("cmd-shift-k", DeleteLine, Some(EDITOR)),
        KeyBinding::new("cmd-l", SelectLine, Some(EDITOR)),
        KeyBinding::new("alt-up", MoveLineUp, Some(EDITOR)),
        KeyBinding::new("alt-down", MoveLineDown, Some(EDITOR)),
        KeyBinding::new("shift-alt-up", CopyLineUp, Some(EDITOR)),
        KeyBinding::new("shift-alt-down", CopyLineDown, Some(EDITOR)),
        KeyBinding::new("cmd-alt-up", AddCursorAbove, Some(EDITOR)),
        KeyBinding::new("cmd-alt-down", AddCursorBelow, Some(EDITOR)),
        KeyBinding::new("ctrl-alt-up", AddCursorAbove, Some(EDITOR)),
        KeyBinding::new("ctrl-alt-down", AddCursorBelow, Some(EDITOR)),
        KeyBinding::new("cmd-shift-l", SplitSelectionIntoLines, Some(EDITOR)),
        KeyBinding::new("cmd-d", AddNextOccurrence, Some(EDITOR)),
        KeyBinding::new("cmd-enter", InsertLineBelow, Some(EDITOR)),
        KeyBinding::new("cmd-shift-enter", InsertLineAbove, Some(EDITOR)),
        KeyBinding::new("cmd-g", GoToLine, Some(EDITOR)),
        KeyBinding::new("alt-z", ToggleWordWrap, Some(EDITOR)),
    ]
}

#[cfg(test)]
mod tests {
    // Explicit imports: `use super::*` would pull in gpui-kit's `test` attribute
    // macro and shadow the built-in one.
    use super::{app_bindings, editor_bindings};
    use crate::{MoveLineDown, MoveLineUp};
    use gpui_kit::base::input::{
        AddCursorAbove as EngineAddCursorAbove, AddCursorBelow as EngineAddCursorBelow,
    };
    use gpui_kit::{Action as _, KeyBinding, KeyContext, Keymap, Keystroke};

    /// Action names that would win for `keys` typed in `contexts`, best first.
    fn resolve(keymap: &Keymap, keys: &str, contexts: &[&str]) -> Vec<String> {
        let input: Vec<Keystroke> = keys
            .split_whitespace()
            .map(|key| Keystroke::parse(key).unwrap())
            .collect();
        let stack: Vec<KeyContext> = contexts
            .iter()
            .map(|context| KeyContext::parse(context).unwrap())
            .collect();
        keymap
            .bindings_for_input(&input, &stack)
            .0
            .iter()
            .map(|binding| binding.action().name().to_string())
            .collect()
    }

    #[test]
    fn every_binding_parses() {
        // `KeyBinding::new` panics on an invalid keystroke, so building the
        // lists is itself the assertion that every chord is well-formed.
        assert_eq!(app_bindings().len(), 37);
        assert_eq!(editor_bindings().len(), 17);
    }

    #[test]
    fn app_bindings_are_global_but_editor_bindings_are_not() {
        let mut keymap = Keymap::default();
        keymap.add_bindings(app_bindings());
        keymap.add_bindings(editor_bindings());

        assert!(resolve(&keymap, "cmd-n", &[])[0].ends_with("NewFile"));
        assert!(resolve(&keymap, "cmd-r", &[])[0].ends_with("Refresh"));
        // The finder opens from anywhere, including while an editor has focus.
        assert!(resolve(&keymap, "cmd-p", &[])[0].ends_with("QuickOpen"));
        assert!(resolve(&keymap, "cmd-p", &["Input"])[0].ends_with("QuickOpen"));
        // The palette opens from anywhere, including while an editor has focus.
        assert!(resolve(&keymap, "cmd-shift-p", &[])[0].ends_with("CommandPalette"));
        assert!(resolve(&keymap, "cmd-shift-p", &["Input"])[0].ends_with("CommandPalette"));
        assert!(resolve(&keymap, "cmd-/", &[]).is_empty());
        assert!(resolve(&keymap, "cmd-/", &["Input"])[0].ends_with("ToggleLineComment"));

        // The Option+Cmd family added for view/file toggles must stay global so
        // it also fires while an editor has focus.
        for (keys, action) in [
            ("alt-cmd-i", "ToggleWhitespace"),
            ("cmd-shift-v", "ToggleMarkdownPreview"),
            ("alt-cmd-w", "CloseAllFolders"),
            ("alt-cmd-r", "RevealInFinder"),
            ("alt-cmd-c", "CopyFilePath"),
            ("alt-cmd-shift-c", "CopyRelativePath"),
        ] {
            assert!(resolve(&keymap, keys, &[])[0].ends_with(action));
            assert!(resolve(&keymap, keys, &["Input"])[0].ends_with(action));
        }
    }

    #[test]
    fn editor_bindings_win_over_earlier_engine_defaults() {
        // Emulate the engine registering its defaults first (as gpui_kit::init
        // does), then this app's bindings afterwards. At equal context depth the
        // later registration must win.
        let mut keymap = Keymap::default();
        keymap.add_bindings([
            KeyBinding::new("shift-alt-up", MoveLineUp, Some("Input")),
            KeyBinding::new("shift-alt-down", MoveLineDown, Some("Input")),
            KeyBinding::new("secondary-enter", MoveLineUp, Some("Input")),
            // The engine's own add-cursor chords. The app now binds the same
            // chords to its actions, so the later registration must still win.
            KeyBinding::new("cmd-alt-up", EngineAddCursorAbove, Some("Input")),
            KeyBinding::new("cmd-alt-down", EngineAddCursorBelow, Some("Input")),
        ]);
        keymap.add_bindings(editor_bindings());

        assert!(resolve(&keymap, "shift-alt-up", &["Input"])[0].ends_with("CopyLineUp"));
        assert!(resolve(&keymap, "shift-alt-down", &["Input"])[0].ends_with("CopyLineDown"));
        assert!(resolve(&keymap, "cmd-enter", &["Input"])[0].ends_with("InsertLineBelow"));
        // The app's action wins over the engine's identically-named one.
        assert_eq!(
            resolve(&keymap, "cmd-alt-up", &["Input"])[0],
            crate::AddCursorAbove.name()
        );
        assert_eq!(
            resolve(&keymap, "cmd-alt-down", &["Input"])[0],
            crate::AddCursorBelow.name()
        );
        // The alternate chord is app-only; nothing shadows it.
        assert_eq!(
            resolve(&keymap, "ctrl-alt-up", &["Input"])[0],
            crate::AddCursorAbove.name()
        );
    }
}
