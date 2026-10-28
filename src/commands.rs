//! Every user-invokable command, declared once.
//!
//! A command used to be described in up to four places -- the menu bar, the
//! command palette, the keymap, and the action declaration -- with nothing tying
//! them together. They drifted: the menu said "Reopen Last Closed Tab" where the
//! palette said "Reopen Closed Tab", and the palette carried toggles no menu
//! showed. [`COMMANDS`] is now the one description, and the menu, the palette and
//! the keymap are all derived from it.
//!
//! Two things are deliberately *not* derived:
//!
//! - The action **types**, declared by `gpui_kit::actions!` in `main.rs`. Nothing
//!   in a table can create types.
//! - The **handlers**, registered per action type in `app/mod.rs`. GPUI dispatches
//!   by concrete type, so one `.on_action` per action is unavoidable.
//!
//! Menus are not only app commands either -- they mix in gpui-kit's own actions
//! and two hand-written surfaces -- so each menu keeps an explicit
//! [`Entry`] skeleton that pulls its command rows out of the table by cluster.

use std::path::PathBuf;
use std::rc::Rc;

use gpui_kit::component::{
    command::{CommandGroup, CommandItem},
    input,
};
use gpui_kit::*;

use crate::paths::display_path;
use crate::{
    About, AddCursorAbove, AddCursorBelow, AddFolder, AddNextOccurrence, BringAllToFront,
    CheckForUpdates, ClearRecent, CloseAllFolders, CloseFindPanel, CloseTab, CloseWindow,
    CommandPalette, ConvertToCamelCase, ConvertToConstantCase, ConvertToKebabCase,
    ConvertToLowercase, ConvertToPascalCase, ConvertToSnakeCase, ConvertToTitleCase,
    ConvertToUppercase, CopyFileName, CopyFilePath, CopyLineDown, CopyLineUp, CopyRelativePath,
    DeleteLine, FindAndReplaceInEditor, FindInEditor, FindInFiles, FindNextField, FindNextInEditor,
    FindPreviousField, FindPreviousInEditor, GoToLine, Hide, HideOthers, InsertLineAbove,
    InsertLineBelow, InstallCli, JoinLines, JumpToTab, MinimizeWindow, MoveLineDown, MoveLineUp,
    MoveTabToNewWindow, NewFile, NewWindow, NextResult, NextTab, OpenDefaultSettings, OpenFile,
    OpenFolderInNewWindow, OpenKeybindings, OpenPath, OpenRecentPicker, OpenResult, OpenSettings,
    OpenThemesFolder, PreviousResult, PreviousTab, Print, QuickOpen, Quit, Refresh,
    ReopenClosedTab, ReplaceAll, RevealInFinder, RunFind, SaveAll, SaveFile, SaveFileAs,
    SelectLine, SetTheme, ShowAll, ShowChanges, ShowExplorer, SplitSelectionIntoLines,
    THEME_CHOICES, ToggleAutoClose, ToggleFolding, ToggleFullScreen, ToggleIndentGuides,
    ToggleLineComment, ToggleLineNumbers, ToggleMarkdownPreview, ToggleSidebar, ToggleSmartIndent,
    ToggleWhitespace, ToggleWordWrap, TrimTrailingWhitespace, ZoomIn, ZoomOut, ZoomReset,
    ZoomWindow,
};

/// gpui-kit's editor key context. Its `CONTEXT` constant is private, so the
/// literal it uses is repeated here.
const EDITOR: &str = "Input";

/// A menu in the menu bar, in bar order.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum MenuKind {
    Teak,
    File,
    Edit,
    Selection,
    View,
    Window,
}

/// A group in the command palette.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum PaletteGroup {
    File,
    Edit,
    View,
    Window,
    Teak,
}

/// Which key context a chord is active in.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Scope {
    /// Fires regardless of focus.
    Global,
    /// Fires only while a text editor has focus.
    Editor,
    /// Fires only in a Find panel field.
    FindPanel,
    /// Fires only in the Find Results tab's editor.
    FindResults,
}

/// One user-facing command.
pub(crate) struct Command {
    /// The menu and palette label, written once.
    label: &'static str,
    /// Builds the action to dispatch.
    make: fn() -> Box<dyn Action>,
    /// Chords for this command, each with the context it fires in.
    bindings: &'static [(&'static str, Scope)],
    /// Menu placement: which menu, and which cluster of it. Commands sharing a
    /// cluster appear in table order, so a cluster's rows are declared together.
    menu: Option<(MenuKind, u8)>,
    /// Palette placement: which group, and its position within that group.
    palette: Option<(PaletteGroup, u8)>,
    /// Extra palette search aliases.
    keywords: &'static [&'static str],
}

/// A command with no chord.
const NO_KEYS: &[(&str, Scope)] = &[];

/// Every user-invokable command.
///
/// `OpenPath` and `SetTheme` are absent on purpose: the first is dispatched by
/// the quick-open finder rather than by the user, and the second is built per
/// value from `THEME_CHOICES`, which the Appearance submenu and palette group
/// already share.
static COMMANDS: &[Command] = &[
    // File
    Command {
        label: "New File",
        make: || Box::new(NewFile),
        bindings: &[("secondary-n", Scope::Global)],
        menu: Some((MenuKind::File, 0)),
        palette: Some((PaletteGroup::File, 0)),
        keywords: &[],
    },
    Command {
        label: "New Window",
        make: || Box::new(NewWindow),
        bindings: &[("secondary-shift-n", Scope::Global)],
        menu: Some((MenuKind::File, 0)),
        palette: Some((PaletteGroup::File, 1)),
        keywords: &["window"],
    },
    Command {
        label: "Open…",
        make: || Box::new(OpenFile),
        bindings: &[("secondary-o", Scope::Global)],
        menu: Some((MenuKind::File, 0)),
        palette: Some((PaletteGroup::File, 2)),
        keywords: &[],
    },
    Command {
        label: "Quick Open…",
        make: || Box::new(QuickOpen),
        bindings: &[("secondary-p", Scope::Global)],
        menu: Some((MenuKind::File, 0)),
        palette: Some((PaletteGroup::File, 3)),
        keywords: &[],
    },
    Command {
        label: "Open Recent…",
        make: || Box::new(OpenRecentPicker),
        // `⌃R` is the customary Open Recent chord on macOS.
        bindings: &[("ctrl-r", Scope::Global)],
        menu: None,
        palette: Some((PaletteGroup::File, 4)),
        keywords: &["recent", "folder"],
    },
    Command {
        label: "Add Folder…",
        make: || Box::new(AddFolder),
        // `⇧⌘O` is the customary Go to Symbol chord, so keep it free and put
        // the folder action on `⌃⌘O`.
        bindings: &[("ctrl-secondary-o", Scope::Global)],
        menu: Some((MenuKind::File, 0)),
        palette: Some((PaletteGroup::File, 5)),
        keywords: &[],
    },
    Command {
        label: "Open Folder in New Window…",
        make: || Box::new(OpenFolderInNewWindow),
        bindings: NO_KEYS,
        menu: Some((MenuKind::File, 0)),
        palette: Some((PaletteGroup::File, 6)),
        keywords: &["folder", "window"],
    },
    Command {
        label: "Close All Folders",
        make: || Box::new(CloseAllFolders),
        // `⌥⌘W` is Close All Windows on macOS, so this uses `⌃⌘W`.
        bindings: &[("ctrl-secondary-w", Scope::Global)],
        menu: Some((MenuKind::File, 0)),
        palette: Some((PaletteGroup::File, 7)),
        keywords: &[],
    },
    Command {
        label: "Save",
        make: || Box::new(SaveFile),
        bindings: &[("secondary-s", Scope::Global)],
        menu: Some((MenuKind::File, 1)),
        palette: Some((PaletteGroup::File, 8)),
        keywords: &[],
    },
    Command {
        label: "Save As…",
        make: || Box::new(SaveFileAs),
        bindings: &[("secondary-shift-s", Scope::Global)],
        menu: Some((MenuKind::File, 1)),
        palette: Some((PaletteGroup::File, 9)),
        keywords: &[],
    },
    Command {
        label: "Save All",
        make: || Box::new(SaveAll),
        bindings: &[("alt-secondary-s", Scope::Global)],
        menu: Some((MenuKind::File, 1)),
        palette: Some((PaletteGroup::File, 10)),
        keywords: &[],
    },
    Command {
        label: "Print…",
        make: || Box::new(Print),
        // `secondary-p` is Quick Open; macOS leaves print without a chord here.
        bindings: NO_KEYS,
        menu: Some((MenuKind::File, 1)),
        palette: Some((PaletteGroup::File, 17)),
        keywords: &["print", "paper", "pdf"],
    },
    Command {
        label: "Close Tab",
        make: || Box::new(CloseTab),
        bindings: &[("secondary-w", Scope::Global)],
        menu: Some((MenuKind::File, 2)),
        palette: Some((PaletteGroup::File, 11)),
        keywords: &[],
    },
    Command {
        label: "Close Window",
        make: || Box::new(CloseWindow),
        bindings: &[("secondary-shift-w", Scope::Global)],
        menu: Some((MenuKind::File, 2)),
        palette: Some((PaletteGroup::File, 12)),
        keywords: &["window"],
    },
    Command {
        label: "Move Tab to New Window",
        make: || Box::new(MoveTabToNewWindow),
        bindings: NO_KEYS,
        menu: Some((MenuKind::File, 2)),
        palette: Some((PaletteGroup::File, 13)),
        keywords: &["window", "tab"],
    },
    Command {
        // The menu used to read "Reopen Last Closed Tab" while the palette read
        // "Reopen Closed Tab"; one label now serves both.
        label: "Reopen Closed Tab",
        make: || Box::new(ReopenClosedTab),
        bindings: &[("secondary-shift-t", Scope::Global)],
        menu: Some((MenuKind::File, 2)),
        palette: Some((PaletteGroup::File, 14)),
        keywords: &[],
    },
    Command {
        label: "Reveal in Finder",
        make: || Box::new(RevealInFinder),
        bindings: &[("alt-secondary-r", Scope::Global)],
        menu: Some((MenuKind::File, 3)),
        palette: Some((PaletteGroup::File, 15)),
        keywords: &[],
    },
    Command {
        // The Open Recent submenu shows this as "Clear Menu", the macOS name.
        label: "Clear Recent",
        make: || Box::new(ClearRecent),
        bindings: NO_KEYS,
        menu: None,
        palette: Some((PaletteGroup::File, 16)),
        keywords: &["recent", "history"],
    },
    // Edit
    // In-file find. The engine binds `secondary-f` on the editor, so these
    // carry an `Editor` scope too: the app-level action then wins and can also
    // reach a previewing tab.
    Command {
        label: "Find…",
        make: || Box::new(FindInEditor),
        bindings: &[
            ("secondary-f", Scope::Global),
            ("secondary-f", Scope::Editor),
        ],
        menu: Some((MenuKind::Edit, 7)),
        palette: Some((PaletteGroup::Edit, 30)),
        keywords: &["search", "find", "replace"],
    },
    Command {
        label: "Find and Replace…",
        make: || Box::new(FindAndReplaceInEditor),
        bindings: &[
            ("secondary-alt-f", Scope::Global),
            ("secondary-alt-f", Scope::Editor),
        ],
        menu: Some((MenuKind::Edit, 7)),
        palette: Some((PaletteGroup::Edit, 31)),
        keywords: &["search", "find", "replace"],
    },
    Command {
        label: "Find Next",
        make: || Box::new(FindNextInEditor),
        // `⌘G` is Find Next on macOS; keep `F3` as the alternate.
        bindings: &[("secondary-g", Scope::Global), ("f3", Scope::Global)],
        menu: Some((MenuKind::Edit, 7)),
        palette: Some((PaletteGroup::Edit, 32)),
        keywords: &["search", "find", "match"],
    },
    Command {
        label: "Find Previous",
        make: || Box::new(FindPreviousInEditor),
        bindings: &[
            ("secondary-shift-g", Scope::Global),
            ("shift-f3", Scope::Global),
        ],
        menu: Some((MenuKind::Edit, 7)),
        palette: Some((PaletteGroup::Edit, 33)),
        keywords: &["search", "find", "match"],
    },
    Command {
        label: "Find in Files…",
        make: || Box::new(FindInFiles),
        // The editor binds this chord to Replace, so the editor scope is needed
        // for Find in Files to win while an editor has focus.
        bindings: &[
            ("secondary-shift-f", Scope::Global),
            ("secondary-shift-f", Scope::Editor),
        ],
        menu: Some((MenuKind::Edit, 6)),
        palette: Some((PaletteGroup::Edit, 16)),
        keywords: &["search", "grep", "project"],
    },
    Command {
        label: "Next Result",
        make: || Box::new(NextResult),
        bindings: &[("f4", Scope::Global)],
        menu: Some((MenuKind::Edit, 6)),
        palette: Some((PaletteGroup::Edit, 17)),
        keywords: &["find", "search", "match"],
    },
    Command {
        label: "Previous Result",
        make: || Box::new(PreviousResult),
        bindings: &[("shift-f4", Scope::Global)],
        menu: Some((MenuKind::Edit, 6)),
        palette: Some((PaletteGroup::Edit, 18)),
        keywords: &["find", "search", "match"],
    },
    // Keys that only mean something inside the Find panel or the results tab;
    // on no menu or palette.
    Command {
        label: "Run Find",
        make: || Box::new(RunFind),
        bindings: &[("enter", Scope::FindPanel)],
        menu: None,
        palette: None,
        keywords: &[],
    },
    Command {
        label: "Replace All",
        make: || Box::new(ReplaceAll),
        // `⌥↩` is the customary Select All Matches chord, so Replace All takes
        // `⌥⌘↩` inside the panel.
        bindings: &[("alt-secondary-enter", Scope::FindPanel)],
        menu: None,
        palette: None,
        keywords: &[],
    },
    Command {
        label: "Close Find Panel",
        make: || Box::new(CloseFindPanel),
        bindings: &[("escape", Scope::FindPanel)],
        menu: None,
        palette: None,
        keywords: &[],
    },
    Command {
        label: "Next Find Field",
        make: || Box::new(FindNextField),
        bindings: &[("tab", Scope::FindPanel)],
        menu: None,
        palette: None,
        keywords: &[],
    },
    Command {
        label: "Previous Find Field",
        make: || Box::new(FindPreviousField),
        bindings: &[("shift-tab", Scope::FindPanel)],
        menu: None,
        palette: None,
        keywords: &[],
    },
    Command {
        label: "Open Result",
        make: || Box::new(OpenResult),
        bindings: &[("enter", Scope::FindResults)],
        menu: None,
        palette: None,
        keywords: &[],
    },
    Command {
        label: "Copy Path",
        make: || Box::new(CopyFilePath),
        bindings: &[("alt-secondary-c", Scope::Global)],
        menu: Some((MenuKind::File, 3)),
        palette: Some((PaletteGroup::Edit, 14)),
        keywords: &[],
    },
    Command {
        label: "Copy Relative Path",
        make: || Box::new(CopyRelativePath),
        bindings: &[("alt-secondary-shift-c", Scope::Global)],
        menu: Some((MenuKind::File, 3)),
        palette: Some((PaletteGroup::Edit, 15)),
        keywords: &[],
    },
    Command {
        label: "Copy File Name",
        make: || Box::new(CopyFileName),
        bindings: NO_KEYS,
        menu: Some((MenuKind::File, 3)),
        palette: Some((PaletteGroup::Edit, 19)),
        keywords: &[],
    },
    Command {
        label: "Toggle Line Comment",
        make: || Box::new(ToggleLineComment),
        bindings: &[("secondary-/", Scope::Editor)],
        menu: Some((MenuKind::Edit, 2)),
        palette: Some((PaletteGroup::Edit, 0)),
        keywords: &[],
    },
    Command {
        label: "Delete Line",
        make: || Box::new(DeleteLine),
        bindings: &[("secondary-shift-k", Scope::Editor)],
        menu: Some((MenuKind::Edit, 2)),
        palette: Some((PaletteGroup::Edit, 1)),
        keywords: &[],
    },
    Command {
        label: "Select Line",
        make: || Box::new(SelectLine),
        bindings: &[("secondary-l", Scope::Editor)],
        menu: Some((MenuKind::Selection, 0)),
        palette: Some((PaletteGroup::Edit, 2)),
        keywords: &[],
    },
    Command {
        label: "Join Lines",
        make: || Box::new(JoinLines),
        bindings: &[("secondary-j", Scope::Editor)],
        menu: Some((MenuKind::Edit, 2)),
        palette: Some((PaletteGroup::Edit, 20)),
        keywords: &[],
    },
    Command {
        label: "Trim Trailing Whitespace",
        make: || Box::new(TrimTrailingWhitespace),
        bindings: NO_KEYS,
        menu: Some((MenuKind::Edit, 2)),
        palette: Some((PaletteGroup::Edit, 21)),
        keywords: &[],
    },
    // Convert Case: the submenu's rows come from cluster 3 of the Edit menu.
    Command {
        label: "Convert to Uppercase",
        make: || Box::new(ConvertToUppercase),
        bindings: NO_KEYS,
        menu: Some((MenuKind::Edit, 3)),
        palette: Some((PaletteGroup::Edit, 22)),
        keywords: &["case", "upper", "uppercase"],
    },
    Command {
        label: "Convert to Lowercase",
        make: || Box::new(ConvertToLowercase),
        bindings: NO_KEYS,
        menu: Some((MenuKind::Edit, 3)),
        palette: Some((PaletteGroup::Edit, 23)),
        keywords: &["case", "lower", "lowercase"],
    },
    Command {
        label: "Convert to Title Case",
        make: || Box::new(ConvertToTitleCase),
        bindings: NO_KEYS,
        menu: Some((MenuKind::Edit, 3)),
        palette: Some((PaletteGroup::Edit, 24)),
        keywords: &["case", "title"],
    },
    Command {
        label: "Convert to Camel Case",
        make: || Box::new(ConvertToCamelCase),
        bindings: NO_KEYS,
        menu: Some((MenuKind::Edit, 3)),
        palette: Some((PaletteGroup::Edit, 25)),
        keywords: &["case", "camel", "camelcase"],
    },
    Command {
        label: "Convert to Pascal Case",
        make: || Box::new(ConvertToPascalCase),
        bindings: NO_KEYS,
        menu: Some((MenuKind::Edit, 3)),
        palette: Some((PaletteGroup::Edit, 26)),
        keywords: &["case", "pascal", "pascalcase"],
    },
    Command {
        label: "Convert to Snake Case",
        make: || Box::new(ConvertToSnakeCase),
        bindings: NO_KEYS,
        menu: Some((MenuKind::Edit, 3)),
        palette: Some((PaletteGroup::Edit, 27)),
        keywords: &["case", "snake", "snake_case"],
    },
    Command {
        label: "Convert to Kebab Case",
        make: || Box::new(ConvertToKebabCase),
        bindings: NO_KEYS,
        menu: Some((MenuKind::Edit, 3)),
        palette: Some((PaletteGroup::Edit, 28)),
        keywords: &["case", "kebab", "kebab-case", "slug", "dash"],
    },
    Command {
        label: "Convert to Constant Case",
        make: || Box::new(ConvertToConstantCase),
        bindings: NO_KEYS,
        menu: Some((MenuKind::Edit, 3)),
        palette: Some((PaletteGroup::Edit, 29)),
        keywords: &["case", "constant", "screaming", "upper snake"],
    },
    Command {
        label: "Move Line Up",
        make: || Box::new(MoveLineUp),
        bindings: &[("alt-up", Scope::Editor)],
        menu: Some((MenuKind::Selection, 2)),
        palette: Some((PaletteGroup::Edit, 3)),
        keywords: &[],
    },
    Command {
        label: "Move Line Down",
        make: || Box::new(MoveLineDown),
        bindings: &[("alt-down", Scope::Editor)],
        menu: Some((MenuKind::Selection, 2)),
        palette: Some((PaletteGroup::Edit, 4)),
        keywords: &[],
    },
    Command {
        label: "Copy Line Up",
        make: || Box::new(CopyLineUp),
        bindings: &[("shift-alt-up", Scope::Editor)],
        menu: Some((MenuKind::Selection, 2)),
        palette: Some((PaletteGroup::Edit, 5)),
        keywords: &[],
    },
    Command {
        label: "Copy Line Down",
        make: || Box::new(CopyLineDown),
        bindings: &[("shift-alt-down", Scope::Editor)],
        menu: Some((MenuKind::Selection, 2)),
        palette: Some((PaletteGroup::Edit, 6)),
        keywords: &[],
    },
    Command {
        label: "Insert Line Above",
        make: || Box::new(InsertLineAbove),
        bindings: &[("secondary-shift-enter", Scope::Editor)],
        menu: Some((MenuKind::Edit, 4)),
        palette: Some((PaletteGroup::Edit, 7)),
        keywords: &[],
    },
    Command {
        label: "Insert Line Below",
        make: || Box::new(InsertLineBelow),
        bindings: &[("secondary-enter", Scope::Editor)],
        menu: Some((MenuKind::Edit, 4)),
        palette: Some((PaletteGroup::Edit, 8)),
        keywords: &[],
    },
    Command {
        label: "Go to Line…",
        make: || Box::new(GoToLine),
        // `⌘G` stays Find Next, as macOS expects; Go to Line uses the customary
        // `⌃G` chord for a focused editor.
        bindings: &[("ctrl-g", Scope::Editor)],
        menu: Some((MenuKind::Edit, 5)),
        palette: Some((PaletteGroup::Edit, 9)),
        keywords: &["goto"],
    },
    Command {
        label: "Add Cursor Above",
        make: || Box::new(AddCursorAbove),
        // Also the engine's Ctrl+Alt alternate.
        bindings: &[
            ("secondary-alt-up", Scope::Editor),
            ("ctrl-alt-up", Scope::Editor),
        ],
        menu: Some((MenuKind::Selection, 1)),
        palette: Some((PaletteGroup::Edit, 10)),
        keywords: &[],
    },
    Command {
        label: "Add Cursor Below",
        make: || Box::new(AddCursorBelow),
        bindings: &[
            ("secondary-alt-down", Scope::Editor),
            ("ctrl-alt-down", Scope::Editor),
        ],
        menu: Some((MenuKind::Selection, 1)),
        palette: Some((PaletteGroup::Edit, 11)),
        keywords: &[],
    },
    Command {
        label: "Split into Lines",
        make: || Box::new(SplitSelectionIntoLines),
        bindings: &[("secondary-shift-l", Scope::Editor)],
        menu: Some((MenuKind::Selection, 0)),
        palette: Some((PaletteGroup::Edit, 12)),
        keywords: &[],
    },
    Command {
        label: "Add Next Occurrence",
        make: || Box::new(AddNextOccurrence),
        bindings: &[("secondary-d", Scope::Editor)],
        menu: Some((MenuKind::Selection, 1)),
        palette: Some((PaletteGroup::Edit, 13)),
        keywords: &[],
    },
    // View
    Command {
        label: "Command Palette…",
        make: || Box::new(CommandPalette),
        bindings: &[("secondary-shift-p", Scope::Global)],
        menu: Some((MenuKind::View, 0)),
        palette: None,
        keywords: &[],
    },
    Command {
        label: "Toggle Sidebar",
        make: || Box::new(ToggleSidebar),
        bindings: &[("secondary-b", Scope::Global)],
        menu: Some((MenuKind::View, 1)),
        palette: Some((PaletteGroup::View, 0)),
        keywords: &[],
    },
    Command {
        label: "Show Explorer",
        make: || Box::new(ShowExplorer),
        bindings: &[("secondary-shift-e", Scope::Global)],
        menu: Some((MenuKind::View, 1)),
        palette: Some((PaletteGroup::View, 15)),
        keywords: &["files", "tree", "sidebar"],
    },
    Command {
        label: "Show Changes",
        make: || Box::new(ShowChanges),
        bindings: &[("ctrl-shift-g", Scope::Global)],
        menu: Some((MenuKind::View, 1)),
        palette: Some((PaletteGroup::View, 16)),
        keywords: &["git", "diff", "source control", "scm"],
    },
    Command {
        label: "Refresh",
        make: || Box::new(Refresh),
        bindings: &[("secondary-r", Scope::Global)],
        menu: Some((MenuKind::View, 1)),
        palette: Some((PaletteGroup::View, 1)),
        keywords: &[],
    },
    Command {
        label: "Zoom In",
        make: || Box::new(ZoomIn),
        bindings: &[
            ("secondary-=", Scope::Global),
            ("secondary-shift-=", Scope::Global),
        ],
        menu: Some((MenuKind::View, 2)),
        palette: Some((PaletteGroup::View, 2)),
        keywords: &[],
    },
    Command {
        label: "Zoom Out",
        make: || Box::new(ZoomOut),
        bindings: &[("secondary--", Scope::Global)],
        menu: Some((MenuKind::View, 2)),
        palette: Some((PaletteGroup::View, 3)),
        keywords: &[],
    },
    Command {
        label: "Actual Size",
        make: || Box::new(ZoomReset),
        bindings: &[("secondary-0", Scope::Global)],
        menu: Some((MenuKind::View, 2)),
        palette: Some((PaletteGroup::View, 4)),
        keywords: &[],
    },
    Command {
        label: "Toggle Word Wrap",
        make: || Box::new(ToggleWordWrap),
        bindings: &[("alt-z", Scope::Editor)],
        menu: Some((MenuKind::View, 3)),
        palette: Some((PaletteGroup::View, 5)),
        keywords: &[],
    },
    Command {
        label: "Toggle Invisible Characters",
        make: || Box::new(ToggleWhitespace),
        bindings: &[("alt-secondary-i", Scope::Global)],
        menu: Some((MenuKind::View, 3)),
        palette: Some((PaletteGroup::View, 6)),
        keywords: &[],
    },
    Command {
        label: "Toggle Line Numbers",
        make: || Box::new(ToggleLineNumbers),
        bindings: NO_KEYS,
        menu: Some((MenuKind::View, 3)),
        palette: Some((PaletteGroup::View, 7)),
        keywords: &[],
    },
    Command {
        label: "Toggle Indent Guides",
        make: || Box::new(ToggleIndentGuides),
        bindings: NO_KEYS,
        menu: Some((MenuKind::View, 3)),
        palette: Some((PaletteGroup::View, 8)),
        keywords: &[],
    },
    Command {
        label: "Toggle Code Folding",
        make: || Box::new(ToggleFolding),
        bindings: NO_KEYS,
        menu: Some((MenuKind::View, 3)),
        palette: Some((PaletteGroup::View, 9)),
        keywords: &[],
    },
    Command {
        label: "Toggle Preview",
        make: || Box::new(ToggleMarkdownPreview),
        // `⇧⌘V` is Paste and Match Style system-wide; leave it free.
        bindings: &[("ctrl-secondary-v", Scope::Global)],
        menu: Some((MenuKind::View, 3)),
        palette: Some((PaletteGroup::View, 10)),
        keywords: &[],
    },
    Command {
        label: "Toggle Auto Close Brackets",
        make: || Box::new(ToggleAutoClose),
        bindings: NO_KEYS,
        menu: None,
        palette: Some((PaletteGroup::View, 11)),
        keywords: &[],
    },
    Command {
        label: "Toggle Smart Indent",
        make: || Box::new(ToggleSmartIndent),
        bindings: NO_KEYS,
        menu: None,
        palette: Some((PaletteGroup::View, 12)),
        keywords: &[],
    },
    Command {
        label: "Next Tab",
        make: || Box::new(NextTab),
        bindings: &[
            ("secondary-shift-]", Scope::Global),
            ("ctrl-tab", Scope::Global),
        ],
        menu: Some((MenuKind::View, 4)),
        palette: Some((PaletteGroup::View, 13)),
        keywords: &[],
    },
    Command {
        label: "Previous Tab",
        make: || Box::new(PreviousTab),
        bindings: &[
            ("secondary-shift-[", Scope::Global),
            ("ctrl-shift-tab", Scope::Global),
        ],
        menu: Some((MenuKind::View, 4)),
        palette: Some((PaletteGroup::View, 14)),
        keywords: &[],
    },
    // Window
    Command {
        label: "Minimize",
        make: || Box::new(MinimizeWindow),
        bindings: &[("secondary-m", Scope::Global)],
        menu: Some((MenuKind::Window, 0)),
        palette: Some((PaletteGroup::Window, 0)),
        keywords: &["window"],
    },
    Command {
        label: "Zoom",
        make: || Box::new(ZoomWindow),
        bindings: NO_KEYS,
        menu: Some((MenuKind::Window, 0)),
        palette: Some((PaletteGroup::Window, 1)),
        keywords: &["window"],
    },
    Command {
        label: "Toggle Full Screen",
        make: || Box::new(ToggleFullScreen),
        bindings: &[("ctrl-secondary-f", Scope::Global)],
        menu: Some((MenuKind::Window, 1)),
        palette: Some((PaletteGroup::Window, 2)),
        keywords: &["window", "fullscreen"],
    },
    Command {
        label: "Bring All to Front",
        make: || Box::new(BringAllToFront),
        bindings: NO_KEYS,
        menu: Some((MenuKind::Window, 2)),
        palette: Some((PaletteGroup::Window, 3)),
        keywords: &["window"],
    },
    // App
    Command {
        label: "Settings…",
        make: || Box::new(OpenSettings),
        bindings: &[("secondary-,", Scope::Global)],
        menu: Some((MenuKind::Teak, 1)),
        palette: Some((PaletteGroup::Teak, 0)),
        keywords: &["preferences", "config"],
    },
    Command {
        label: "Open Keybindings…",
        make: || Box::new(OpenKeybindings),
        bindings: NO_KEYS,
        menu: Some((MenuKind::Teak, 1)),
        palette: Some((PaletteGroup::Teak, 4)),
        keywords: &["keyboard", "shortcuts", "keys"],
    },
    Command {
        label: "Default Settings",
        make: || Box::new(OpenDefaultSettings),
        bindings: NO_KEYS,
        menu: None,
        palette: Some((PaletteGroup::Teak, 1)),
        keywords: &["defaults", "preferences", "reference"],
    },
    Command {
        label: "About Teak",
        make: || Box::new(About),
        bindings: NO_KEYS,
        menu: Some((MenuKind::Teak, 0)),
        palette: Some((PaletteGroup::Teak, 3)),
        keywords: &[],
    },
    Command {
        label: "Check for Updates…",
        make: || Box::new(CheckForUpdates),
        bindings: NO_KEYS,
        menu: Some((MenuKind::Teak, 0)),
        palette: Some((PaletteGroup::Teak, 2)),
        keywords: &[],
    },
    Command {
        label: "Open Themes Folder",
        make: || Box::new(OpenThemesFolder),
        bindings: NO_KEYS,
        menu: Some((MenuKind::Teak, 1)),
        palette: None,
        keywords: &[],
    },
    Command {
        label: "Install Command Line Tool…",
        make: || Box::new(InstallCli),
        bindings: NO_KEYS,
        menu: Some((MenuKind::Teak, 2)),
        palette: None,
        keywords: &[],
    },
    Command {
        label: "Hide Teak",
        make: || Box::new(Hide),
        bindings: &[("secondary-h", Scope::Global)],
        menu: Some((MenuKind::Teak, 3)),
        palette: None,
        keywords: &[],
    },
    Command {
        label: "Hide Others",
        make: || Box::new(HideOthers),
        bindings: &[("alt-secondary-h", Scope::Global)],
        menu: Some((MenuKind::Teak, 3)),
        palette: None,
        keywords: &[],
    },
    Command {
        label: "Show All",
        make: || Box::new(ShowAll),
        bindings: NO_KEYS,
        menu: Some((MenuKind::Teak, 3)),
        palette: None,
        keywords: &[],
    },
    Command {
        label: "Quit Teak",
        make: || Box::new(Quit),
        bindings: &[("secondary-q", Scope::Global)],
        menu: Some((MenuKind::Teak, 4)),
        palette: None,
        keywords: &[],
    },
    // Jump to tab: one row per chord, since each carries its own index.
    Command {
        label: "Jump to Tab 1",
        make: || Box::new(JumpToTab(1)),
        bindings: &[("secondary-1", Scope::Global)],
        menu: None,
        palette: None,
        keywords: &[],
    },
    Command {
        label: "Jump to Tab 2",
        make: || Box::new(JumpToTab(2)),
        bindings: &[("secondary-2", Scope::Global)],
        menu: None,
        palette: None,
        keywords: &[],
    },
    Command {
        label: "Jump to Tab 3",
        make: || Box::new(JumpToTab(3)),
        bindings: &[("secondary-3", Scope::Global)],
        menu: None,
        palette: None,
        keywords: &[],
    },
    Command {
        label: "Jump to Tab 4",
        make: || Box::new(JumpToTab(4)),
        bindings: &[("secondary-4", Scope::Global)],
        menu: None,
        palette: None,
        keywords: &[],
    },
    Command {
        label: "Jump to Tab 5",
        make: || Box::new(JumpToTab(5)),
        bindings: &[("secondary-5", Scope::Global)],
        menu: None,
        palette: None,
        keywords: &[],
    },
    Command {
        label: "Jump to Tab 6",
        make: || Box::new(JumpToTab(6)),
        bindings: &[("secondary-6", Scope::Global)],
        menu: None,
        palette: None,
        keywords: &[],
    },
    Command {
        label: "Jump to Tab 7",
        make: || Box::new(JumpToTab(7)),
        bindings: &[("secondary-7", Scope::Global)],
        menu: None,
        palette: None,
        keywords: &[],
    },
    Command {
        label: "Jump to Tab 8",
        make: || Box::new(JumpToTab(8)),
        bindings: &[("secondary-8", Scope::Global)],
        menu: None,
        palette: None,
        keywords: &[],
    },
    Command {
        label: "Jump to Tab 9",
        make: || Box::new(JumpToTab(9)),
        bindings: &[("secondary-9", Scope::Global)],
        menu: None,
        palette: None,
        keywords: &[],
    },
];

impl MenuKind {
    /// Menus in bar order.
    const ALL: [Self; 6] = [
        Self::Teak,
        Self::File,
        Self::Edit,
        Self::Selection,
        Self::View,
        Self::Window,
    ];

    fn label(self) -> &'static str {
        match self {
            Self::Teak => crate::app::WINDOW_TITLE,
            Self::File => "File",
            Self::Edit => "Edit",
            Self::Selection => "Selection",
            Self::View => "View",
            Self::Window => "Window",
        }
    }

    fn skeleton(self) -> &'static [Entry] {
        match self {
            Self::Teak => TEAK_MENU,
            Self::File => FILE_MENU,
            Self::Edit => EDIT_MENU,
            Self::Selection => SELECTION_MENU,
            Self::View => VIEW_MENU,
            Self::Window => WINDOW_MENU,
        }
    }
}

impl PaletteGroup {
    fn label(self) -> &'static str {
        match self {
            Self::File => "File",
            Self::Edit => "Edit",
            Self::View => "View",
            Self::Window => "Window",
            Self::Teak => crate::app::WINDOW_TITLE,
        }
    }
}

/// One slot in a menu's layout. The skeleton exists because a menu mixes app
/// commands with gpui-kit's own actions, separators and two hand-written
/// surfaces, so the running order cannot come from the table alone.
enum Entry {
    /// Every command placed at this menu and cluster, in table order.
    Cluster(u8),
    /// A separator, an engine action, or the Services menu.
    Item(fn() -> MenuItem),
    /// The Appearance submenu, which needs the current theme.
    Appearance,
    /// The Open Recent submenu, which needs the recent list.
    OpenRecent,
    /// A submenu whose rows are placed by its own entries in this same menu.
    Submenu(&'static str, &'static [Entry]),
}

const TEAK_MENU: &[Entry] = &[
    Entry::Cluster(0),
    Entry::Item(MenuItem::separator),
    Entry::Cluster(1),
    Entry::Item(MenuItem::separator),
    Entry::Cluster(2),
    Entry::Item(MenuItem::separator),
    Entry::Item(services_menu),
    Entry::Item(MenuItem::separator),
    Entry::Cluster(3),
    Entry::Item(MenuItem::separator),
    Entry::Cluster(4),
];

const FILE_MENU: &[Entry] = &[
    Entry::Cluster(0),
    Entry::OpenRecent,
    Entry::Item(MenuItem::separator),
    Entry::Cluster(1),
    Entry::Item(MenuItem::separator),
    Entry::Cluster(3),
    Entry::Item(MenuItem::separator),
    Entry::Cluster(2),
];

const EDIT_MENU: &[Entry] = &[
    Entry::Item(undo),
    Entry::Item(redo),
    Entry::Item(MenuItem::separator),
    Entry::Item(cut),
    Entry::Item(copy),
    Entry::Item(paste),
    Entry::Item(MenuItem::separator),
    Entry::Cluster(7),
    Entry::Item(MenuItem::separator),
    Entry::Cluster(6),
    Entry::Item(MenuItem::separator),
    Entry::Cluster(2),
    Entry::Item(MenuItem::separator),
    Entry::Submenu("Convert Case", CONVERT_CASE),
    Entry::Item(MenuItem::separator),
    Entry::Cluster(4),
    Entry::Item(MenuItem::separator),
    Entry::Item(indent),
    Entry::Item(outdent),
    Entry::Item(MenuItem::separator),
    Entry::Cluster(5),
];

/// The Convert Case submenu's rows.
const CONVERT_CASE: &[Entry] = &[Entry::Cluster(3)];

const SELECTION_MENU: &[Entry] = &[
    Entry::Item(select_all),
    Entry::Cluster(0),
    Entry::Item(MenuItem::separator),
    Entry::Cluster(1),
    Entry::Item(MenuItem::separator),
    Entry::Cluster(2),
];

const VIEW_MENU: &[Entry] = &[
    Entry::Cluster(0),
    Entry::Item(MenuItem::separator),
    Entry::Cluster(1),
    Entry::Item(MenuItem::separator),
    Entry::Cluster(2),
    Entry::Item(MenuItem::separator),
    Entry::Cluster(3),
    Entry::Item(MenuItem::separator),
    Entry::Cluster(4),
    Entry::Item(MenuItem::separator),
    Entry::Appearance,
];

const WINDOW_MENU: &[Entry] = &[
    Entry::Cluster(0),
    Entry::Item(MenuItem::separator),
    Entry::Cluster(1),
    Entry::Item(MenuItem::separator),
    Entry::Cluster(2),
];

/// The palette's groups, in display order. The Appearance group sits between
/// View and the app group, and is built from `THEME_CHOICES` rather than the table.
enum PaletteSlot {
    Group(PaletteGroup),
    Appearance,
}

const PALETTE: &[PaletteSlot] = &[
    PaletteSlot::Group(PaletteGroup::File),
    PaletteSlot::Group(PaletteGroup::Edit),
    PaletteSlot::Group(PaletteGroup::View),
    PaletteSlot::Group(PaletteGroup::Window),
    PaletteSlot::Appearance,
    PaletteSlot::Group(PaletteGroup::Teak),
];

fn services_menu() -> MenuItem {
    MenuItem::os_submenu("Services", SystemMenuType::Services)
}

fn undo() -> MenuItem {
    MenuItem::os_action("Undo", input::Undo, OsAction::Undo)
}

fn redo() -> MenuItem {
    MenuItem::os_action("Redo", input::Redo, OsAction::Redo)
}

fn cut() -> MenuItem {
    MenuItem::os_action("Cut", input::Cut, OsAction::Cut)
}

fn copy() -> MenuItem {
    MenuItem::os_action("Copy", input::Copy, OsAction::Copy)
}

fn paste() -> MenuItem {
    MenuItem::os_action("Paste", input::Paste, OsAction::Paste)
}

fn select_all() -> MenuItem {
    MenuItem::os_action("Select All", input::SelectAll, OsAction::SelectAll)
}

fn indent() -> MenuItem {
    MenuItem::action("Indent", input::Indent)
}

fn outdent() -> MenuItem {
    MenuItem::action("Outdent", input::Outdent)
}

/// Bindings that are active regardless of which element has focus.
#[cfg(test)]
pub(crate) fn app_bindings(cx: &App) -> Vec<KeyBinding> {
    app_bindings_with(cx, &crate::keymap::Overrides::default())
}

/// The global chords with the user's `keybindings.json` applied.
pub(crate) fn app_bindings_with(cx: &App, overrides: &crate::keymap::Overrides) -> Vec<KeyBinding> {
    bindings(Scope::Global, cx, overrides)
}

/// The editing chords, scoped to the focused editor.
///
/// These are registered after the engine's own `"Input"` bindings; at equal
/// context depth the later registration wins, so these take precedence where
/// they overlap:
/// - `shift-alt-up` / `shift-alt-down` — the customary copy-line chords,
///   replacing the engine's alternate add-cursor chord.
/// - `secondary-alt-up` / `secondary-alt-down` (and the `ctrl-alt-*`
///   alternate) — the engine binds these to its own add-cursor
///   actions; the app binds the same chords to its `AddCursorAbove` /
///   `AddCursorBelow` so the menu and palette share one path (the handler
///   re-dispatches the engine action at the editor).
/// - `secondary-enter` — the customary insert-line-below chord; the engine maps
///   `secondary-enter` to a plain newline on macOS, so this is the useful one.
#[cfg(test)]
pub(crate) fn editor_bindings(cx: &App) -> Vec<KeyBinding> {
    editor_bindings_with(cx, &crate::keymap::Overrides::default())
}

/// The editor chords with the user's `keybindings.json` applied.
pub(crate) fn editor_bindings_with(
    cx: &App,
    overrides: &crate::keymap::Overrides,
) -> Vec<KeyBinding> {
    bindings(Scope::Editor, cx, overrides)
}

/// Enter, Escape and Tab in the Find panel's fields, and Enter in the Find
/// Results tab. Their contexts nest the engine's `"Input"`, and they are
/// registered after the engine's own bindings, so they win over its Enter
/// (newline / submit), Escape and Tab (indent).
#[cfg(test)]
pub(crate) fn find_bindings(cx: &App) -> Vec<KeyBinding> {
    find_bindings_with(cx, &crate::keymap::Overrides::default())
}

/// The Find panel's chords with the user's `keybindings.json` applied.
pub(crate) fn find_bindings_with(
    cx: &App,
    overrides: &crate::keymap::Overrides,
) -> Vec<KeyBinding> {
    let mut list = bindings(Scope::FindPanel, cx, overrides);
    list.extend(bindings(Scope::FindResults, cx, overrides));
    list
}

/// The table's chords for one scope, with the user's overrides applied.
///
/// An override replaces every default chord of its command and keeps the scope
/// the command was first declared in — a global command stays global, an
/// editor command stays scoped to the editor. A command with no default binding
/// is bound globally, so a user can bind any palette command.
fn bindings(context: Scope, cx: &App, overrides: &crate::keymap::Overrides) -> Vec<KeyBinding> {
    use crate::keymap::Change;

    let mut bindings = Vec::new();
    for command in COMMANDS {
        let name = (command.make)().name();
        match overrides.change(name, command.label) {
            Some(Change::Removed) => continue,
            Some(Change::Key(chord)) => {
                let scope = command
                    .bindings
                    .first()
                    .map_or(Scope::Global, |(_, scope)| *scope);
                if scope == context
                    && let Some(binding) = try_binding(chord, (command.make)(), scope, cx)
                {
                    bindings.push(binding);
                }
                continue;
            }
            None => {}
        }
        for (chord, scope) in command.bindings {
            if *scope != context {
                continue;
            }
            bindings.push(binding(chord, (command.make)(), *scope, cx));
        }
    }
    bindings
}

/// Builds one binding. This is `KeyBinding::new` with the action already boxed:
/// the table holds `Box<dyn Action>`, which does not satisfy `new`'s
/// `impl Action` bound, so the equivalent public constructor is used with the
/// same arguments (`new` passes `false` and `None`). `load` also takes `cx`,
/// which is what resolves a chord's `secondary-` prefix against the host OS.
fn binding(chord: &str, action: Box<dyn Action>, context: Scope, cx: &App) -> KeyBinding {
    try_binding(chord, action, context, cx).unwrap_or_else(|| panic!("`{chord}` is a valid chord"))
}

/// Like [`binding`], but a chord from the user's file that does not parse is
/// skipped instead of stopping startup.
fn try_binding(
    chord: &str,
    action: Box<dyn Action>,
    context: Scope,
    cx: &App,
) -> Option<KeyBinding> {
    let parse = |source: &str| {
        Some(Rc::new(
            KeyBindingContextPredicate::parse(source).expect("the key context parses"),
        ))
    };
    let predicate = match context {
        Scope::Global => None,
        Scope::Editor => parse(EDITOR),
        Scope::FindPanel => parse(&format!("FindPanel > {EDITOR}")),
        Scope::FindResults => parse(&format!("FindResults > {EDITOR}")),
    };
    KeyBinding::load(chord, action, predicate, false, None, cx).ok()
}

/// The whole menu bar, derived from [`COMMANDS`] plus each menu's skeleton.
pub(crate) fn menus(theme: &str, recent: &[PathBuf]) -> Vec<Menu> {
    MenuKind::ALL
        .iter()
        .map(|&kind| {
            let mut items = Vec::new();
            fill_menu(kind, kind.skeleton(), theme, recent, &mut items);
            Menu::new(kind.label()).items(items)
        })
        .collect()
}

/// Appends the rows `entries` describes to `items`, recursing into submenus.
fn fill_menu(
    kind: MenuKind,
    entries: &[Entry],
    theme: &str,
    recent: &[PathBuf],
    items: &mut Vec<MenuItem>,
) {
    for entry in entries {
        match entry {
            Entry::Cluster(cluster) => items.extend(
                COMMANDS
                    .iter()
                    .filter(|command| command.menu == Some((kind, *cluster)))
                    .map(menu_item),
            ),
            Entry::Item(build) => items.push(build()),
            Entry::Appearance => items.push(MenuItem::submenu(theme_menu(theme))),
            Entry::OpenRecent => items.push(MenuItem::submenu(recent_menu(recent))),
            Entry::Submenu(label, entries) => {
                let mut submenu = Vec::new();
                fill_menu(kind, entries, theme, recent, &mut submenu);
                items.push(MenuItem::submenu(Menu::new(*label).items(submenu)));
            }
        }
    }
}

fn menu_item(command: &Command) -> MenuItem {
    MenuItem::Action {
        name: command.label.into(),
        action: (command.make)(),
        os_action: None,
        checked: false,
        disabled: false,
    }
}

pub(crate) fn palette_groups() -> Vec<CommandGroup> {
    PALETTE
        .iter()
        .map(|slot| match slot {
            PaletteSlot::Group(group) => CommandGroup::new().label(group.label()).items(
                ranked(
                    |command| matches!(command.palette, Some((slot, _)) if slot == *group),
                    |command| command.palette.map_or(u8::MAX, |(_, rank)| rank),
                )
                .into_iter()
                .map(|command| {
                    let mut item = CommandItem::new()
                        .label(command.label)
                        .action((command.make)());
                    if !command.keywords.is_empty() {
                        item = item.keywords(command.keywords.iter().copied());
                    }
                    item
                }),
            ),
            PaletteSlot::Appearance => {
                CommandGroup::new()
                    .label("Appearance")
                    .items(THEME_CHOICES.iter().map(|(value, label)| {
                        CommandItem::new()
                            .label(*label)
                            .keywords(["theme", "appearance"])
                            .action(Box::new(SetTheme((*value).to_string())))
                    }))
            }
        })
        .collect()
}

/// The commands matching `keep`, sorted by `rank`.
fn ranked(keep: impl Fn(&Command) -> bool, rank: impl Fn(&Command) -> u8) -> Vec<&'static Command> {
    let mut rows: Vec<&'static Command> = COMMANDS.iter().filter(|command| keep(command)).collect();
    rows.sort_by_key(|command| rank(command));
    rows
}

/// The Open Recent submenu: one item per path, newest first, then Clear Menu.
fn recent_menu(recent: &[PathBuf]) -> Menu {
    let mut items: Vec<MenuItem> = recent
        .iter()
        .map(|path| MenuItem::action(display_path(path), OpenPath(path.clone(), false)))
        .collect();
    if !items.is_empty() {
        items.push(MenuItem::separator());
    }
    items.push(MenuItem::Action {
        name: "Clear Menu".into(),
        action: Box::new(ClearRecent),
        os_action: None,
        checked: false,
        disabled: recent.is_empty(),
    });
    Menu::new("Open Recent").items(items)
}

/// The Appearance submenu: one item per choice, with a check mark on the active
/// one.
fn theme_menu(theme: &str) -> Menu {
    Menu::new("Appearance").items(THEME_CHOICES.iter().map(|(value, label)| {
        MenuItem::action(*label, SetTheme((*value).to_string())).checked(*value == theme)
    }))
}

#[cfg(test)]
mod tests {
    use gpui_kit::{Action, KeyContext, Keymap, Keystroke, OperatingSystem, TestAppContext};

    use super::{
        COMMANDS, Entry, MenuKind, PALETTE, PaletteGroup, PaletteSlot, Scope, app_bindings,
        app_bindings_with, binding, editor_bindings, find_bindings,
    };

    /// The palette groups in display order, skipping the hand-written Appearance
    /// group.
    fn palette_groups_in_order() -> Vec<PaletteGroup> {
        PALETTE
            .iter()
            .filter_map(|slot| match slot {
                PaletteSlot::Group(group) => Some(*group),
                PaletteSlot::Appearance => None,
            })
            .collect()
    }

    /// Action names that would win for `keys` typed in `contexts`, best first.
    ///
    /// The chord text in these tests is the platform-independent kind the table
    /// stores, `secondary-` included; `parse_with_os` is used because plain
    /// `parse` handles only static, explicit-modifier chords and rejects
    /// `secondary-`. The test platform reports macOS, which is the same `os`
    /// that `KeyBinding::load` resolves the registered bindings against.
    fn resolve(keymap: &Keymap, keys: &str, contexts: &[&str]) -> Vec<String> {
        let input: Vec<Keystroke> = keys
            .split_whitespace()
            .map(|key| Keystroke::parse_with_os(key, OperatingSystem::Mac).unwrap())
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

    #[gpui_kit::test]
    async fn every_binding_parses(cx: &mut TestAppContext) {
        let (app, editor, find) = cx.read(|cx| {
            (
                app_bindings(cx).len(),
                editor_bindings(cx).len(),
                find_bindings(cx).len(),
            )
        });
        // Building the lists is itself the assertion that every chord in the
        // table is well-formed, since a chord that fails to parse panics.
        assert_eq!(app, 55);
        assert_eq!(editor, 21);
        assert_eq!(find, 6);
    }

    #[gpui_kit::test]
    async fn app_bindings_are_global_but_editor_bindings_are_not(cx: &mut TestAppContext) {
        let keymap = cx.read(|cx| {
            let mut keymap = Keymap::default();
            keymap.add_bindings(app_bindings(cx));
            keymap.add_bindings(editor_bindings(cx));
            keymap
        });

        assert!(resolve(&keymap, "secondary-n", &[])[0].ends_with("NewFile"));
        assert!(resolve(&keymap, "secondary-r", &[])[0].ends_with("Refresh"));
        // The finder opens from anywhere, including while an editor has focus.
        assert!(resolve(&keymap, "secondary-p", &[])[0].ends_with("QuickOpen"));
        assert!(resolve(&keymap, "secondary-p", &["Input"])[0].ends_with("QuickOpen"));
        // The palette opens from anywhere, including while an editor has focus.
        assert!(resolve(&keymap, "secondary-shift-p", &[])[0].ends_with("CommandPalette"));
        assert!(resolve(&keymap, "secondary-shift-p", &["Input"])[0].ends_with("CommandPalette"));
        assert!(resolve(&keymap, "secondary-/", &[]).is_empty());
        assert!(resolve(&keymap, "secondary-/", &["Input"])[0].ends_with("ToggleLineComment"));

        // `⌘G` is Find Next on macOS, so it stays global and Go to Line moves
        // to the customary `⌃G`, which only applies to a focused editor.
        assert!(resolve(&keymap, "secondary-g", &[])[0].ends_with("FindNextInEditor"));
        assert!(resolve(&keymap, "secondary-g", &["Input"])[0].ends_with("FindNextInEditor"));
        assert!(resolve(&keymap, "secondary-shift-g", &[])[0].ends_with("FindPreviousInEditor"));
        assert!(
            resolve(&keymap, "secondary-shift-g", &["Input"])[0].ends_with("FindPreviousInEditor")
        );
        assert!(resolve(&keymap, "ctrl-g", &[]).is_empty());
        assert!(resolve(&keymap, "ctrl-g", &["Input"])[0].ends_with("GoToLine"));

        // Chords that were moved off macOS conventions: `⇧⌘O` and `⌥⌘W` are
        // Go to Symbol / Close All Windows elsewhere, `⇧⌘V` is Paste and
        // Match Style. `⌃⇥` / `⌃⇧⇥` are the universal next/previous tab keys.
        assert!(resolve(&keymap, "ctrl-secondary-o", &[])[0].ends_with("AddFolder"));
        assert!(resolve(&keymap, "secondary-shift-o", &[]).is_empty());
        assert!(resolve(&keymap, "alt-secondary-w", &[]).is_empty());
        assert!(resolve(&keymap, "secondary-shift-v", &[]).is_empty());
        assert!(resolve(&keymap, "ctrl-tab", &[])[0].ends_with("NextTab"));
        assert!(resolve(&keymap, "ctrl-shift-tab", &[])[0].ends_with("PreviousTab"));

        // The chords added for view/file toggles must stay global so they also
        // fire while an editor has focus.
        for (keys, action) in [
            ("alt-secondary-i", "ToggleWhitespace"),
            ("ctrl-secondary-v", "ToggleMarkdownPreview"),
            ("ctrl-secondary-w", "CloseAllFolders"),
            ("alt-secondary-r", "RevealInFinder"),
            ("alt-secondary-c", "CopyFilePath"),
            ("alt-secondary-shift-c", "CopyRelativePath"),
            ("secondary-shift-f", "FindInFiles"),
        ] {
            assert!(resolve(&keymap, keys, &[])[0].ends_with(action));
            assert!(resolve(&keymap, keys, &["Input"])[0].ends_with(action));
        }
    }

    #[gpui_kit::test]
    async fn editor_bindings_win_over_earlier_engine_defaults(cx: &mut TestAppContext) {
        // Emulate the engine registering its defaults first (as gpui_kit::init
        // does), then this app's bindings afterwards. At equal context depth the
        // later registration must win.
        let keymap = cx.read(|cx| {
            let mut keymap = Keymap::default();
            // The engine's defaults, bound the way the engine binds them: same
            // `"Input"` context, `secondary-` chords resolved through `cx`.
            let engine =
                |chord: &str, action: Box<dyn Action>| binding(chord, action, Scope::Editor, cx);
            keymap.add_bindings([
                engine("shift-alt-up", Box::new(crate::MoveLineUp)),
                engine("shift-alt-down", Box::new(crate::MoveLineDown)),
                engine("secondary-enter", Box::new(crate::MoveLineUp)),
                // The engine's own add-cursor chords. The app now binds the same
                // chords to its actions, so the later registration must still win.
                engine(
                    "secondary-alt-up",
                    Box::new(gpui_kit::base::input::AddCursorAbove),
                ),
                engine(
                    "secondary-alt-down",
                    Box::new(gpui_kit::base::input::AddCursorBelow),
                ),
            ]);
            keymap.add_bindings(editor_bindings(cx));
            keymap
        });

        assert!(resolve(&keymap, "shift-alt-up", &["Input"])[0].ends_with("CopyLineUp"));
        assert!(resolve(&keymap, "shift-alt-down", &["Input"])[0].ends_with("CopyLineDown"));
        assert!(resolve(&keymap, "secondary-enter", &["Input"])[0].ends_with("InsertLineBelow"));
        // The app's action wins over the engine's identically-named one.
        assert_eq!(
            resolve(&keymap, "secondary-alt-up", &["Input"])[0],
            crate::AddCursorAbove.name()
        );
        assert_eq!(
            resolve(&keymap, "secondary-alt-down", &["Input"])[0],
            crate::AddCursorBelow.name()
        );
        // The Ctrl+Alt chord is a second app binding.
        assert_eq!(
            resolve(&keymap, "ctrl-alt-up", &["Input"])[0],
            crate::AddCursorAbove.name()
        );
    }

    #[gpui_kit::test]
    async fn find_panel_uses_its_own_chords(cx: &mut TestAppContext) {
        let keymap = cx.read(|cx| {
            let mut keymap = Keymap::default();
            keymap.add_bindings(find_bindings(cx));
            keymap
        });

        // `⌥↩` is the customary Select All Matches chord, so Replace All moved
        // to `⌥⌘↩` inside the panel.
        assert!(resolve(&keymap, "enter", &["FindPanel", "Input"])[0].ends_with("RunFind"));
        assert!(
            resolve(&keymap, "alt-secondary-enter", &["FindPanel", "Input"])[0]
                .ends_with("ReplaceAll")
        );
        assert!(resolve(&keymap, "alt-enter", &["FindPanel", "Input"]).is_empty());
        assert!(resolve(&keymap, "alt-secondary-enter", &["Input"]).is_empty());
    }

    /// Whether `entries`, or a submenu inside them, shows `cluster`.
    fn shows_cluster(entries: &[Entry], cluster: u8) -> bool {
        entries.iter().any(|entry| match entry {
            Entry::Cluster(found) => *found == cluster,
            Entry::Submenu(_, entries) => shows_cluster(entries, cluster),
            _ => false,
        })
    }

    /// The labels a skeleton shows, in order, including submenu rows.
    fn skeleton_labels(kind: MenuKind, entries: &[Entry]) -> Vec<&'static str> {
        entries
            .iter()
            .flat_map(|entry| match entry {
                Entry::Cluster(cluster) => COMMANDS
                    .iter()
                    .filter(|command| command.menu == Some((kind, *cluster)))
                    .map(|command| command.label)
                    .collect::<Vec<_>>(),
                Entry::Submenu(_, entries) => skeleton_labels(kind, entries),
                _ => Vec::new(),
            })
            .collect()
    }

    /// A command placed at a cluster no skeleton entry shows would silently miss
    /// the menu bar, so every placement has to resolve.
    #[test]
    fn every_menu_placement_reaches_a_cluster() {
        for command in COMMANDS {
            let Some((kind, rank)) = command.menu else {
                continue;
            };
            assert!(
                shows_cluster(kind.skeleton(), rank),
                "`{}` sits in {} cluster {rank}, which no skeleton entry shows",
                command.label,
                kind.label()
            );
        }
    }

    /// The commands each menu shows, in the order they appear. This is the
    /// layout guard: the cluster skeleton plus table order have to reproduce the
    /// bar the app had before the table existed.
    #[test]
    fn menu_command_order_is_unchanged() {
        let teak_commands: &[&str] = &[
            "About Teak",
            "Check for Updates…",
            "Settings…",
            "Open Keybindings…",
            "Open Themes Folder",
            "Install Command Line Tool…",
            "Hide Teak",
            "Hide Others",
            "Show All",
            "Quit Teak",
        ];

        let expected: &[(MenuKind, &[&str])] = &[
            (MenuKind::Teak, teak_commands),
            (
                MenuKind::File,
                &[
                    "New File",
                    "New Window",
                    "Open…",
                    "Quick Open…",
                    "Add Folder…",
                    "Open Folder in New Window…",
                    "Close All Folders",
                    "Save",
                    "Save As…",
                    "Save All",
                    "Print…",
                    "Reveal in Finder",
                    "Copy Path",
                    "Copy Relative Path",
                    "Copy File Name",
                    "Close Tab",
                    "Close Window",
                    "Move Tab to New Window",
                    "Reopen Closed Tab",
                ],
            ),
            (
                MenuKind::Edit,
                &[
                    "Find…",
                    "Find and Replace…",
                    "Find Next",
                    "Find Previous",
                    "Find in Files…",
                    "Next Result",
                    "Previous Result",
                    "Toggle Line Comment",
                    "Delete Line",
                    "Join Lines",
                    "Trim Trailing Whitespace",
                    "Convert to Uppercase",
                    "Convert to Lowercase",
                    "Convert to Title Case",
                    "Convert to Camel Case",
                    "Convert to Pascal Case",
                    "Convert to Snake Case",
                    "Convert to Kebab Case",
                    "Convert to Constant Case",
                    "Insert Line Above",
                    "Insert Line Below",
                    "Go to Line…",
                ],
            ),
            (
                MenuKind::Selection,
                &[
                    "Select Line",
                    "Split into Lines",
                    "Add Cursor Above",
                    "Add Cursor Below",
                    "Add Next Occurrence",
                    "Move Line Up",
                    "Move Line Down",
                    "Copy Line Up",
                    "Copy Line Down",
                ],
            ),
            (
                MenuKind::View,
                &[
                    "Command Palette…",
                    "Toggle Sidebar",
                    "Show Explorer",
                    "Show Changes",
                    "Refresh",
                    "Zoom In",
                    "Zoom Out",
                    "Actual Size",
                    "Toggle Word Wrap",
                    "Toggle Invisible Characters",
                    "Toggle Line Numbers",
                    "Toggle Indent Guides",
                    "Toggle Code Folding",
                    "Toggle Preview",
                    "Next Tab",
                    "Previous Tab",
                ],
            ),
            (
                MenuKind::Window,
                &[
                    "Minimize",
                    "Zoom",
                    "Toggle Full Screen",
                    "Bring All to Front",
                ],
            ),
        ];

        for (kind, labels) in expected {
            assert_eq!(
                skeleton_labels(*kind, kind.skeleton()),
                *labels,
                "the {} menu changed",
                kind.label()
            );
        }
    }

    /// The palette's order comes from its ranks rather than the table order, so
    /// two commands in one group sharing a rank would land arbitrarily.
    #[test]
    fn palette_ranks_are_unique_within_their_group() {
        for group in palette_groups_in_order() {
            let mut ranks: Vec<u8> = COMMANDS
                .iter()
                .filter_map(|command| match command.palette {
                    Some((slot, rank)) if slot == group => Some(rank),
                    _ => None,
                })
                .collect();
            let placed = ranks.len();
            ranks.sort_unstable();
            ranks.dedup();
            assert_eq!(
                ranks.len(),
                placed,
                "the {} palette group reuses a rank",
                group.label()
            );
        }
    }

    #[test]
    fn labels_are_unique_within_a_menu_and_a_group() {
        for kind in MenuKind::ALL {
            let mut labels: Vec<&str> = COMMANDS
                .iter()
                .filter_map(|command| match command.menu {
                    Some((menu, _)) if menu == kind => Some(command.label),
                    _ => None,
                })
                .collect();
            let placed = labels.len();
            labels.sort_unstable();
            labels.dedup();
            assert_eq!(labels.len(), placed, "{} shows a label twice", kind.label());
        }

        for group in palette_groups_in_order() {
            let mut labels: Vec<&str> = COMMANDS
                .iter()
                .filter_map(|command| match command.palette {
                    Some((slot, _)) if slot == group => Some(command.label),
                    _ => None,
                })
                .collect();
            let placed = labels.len();
            labels.sort_unstable();
            labels.dedup();
            assert_eq!(
                labels.len(),
                placed,
                "the {} palette group shows a label twice",
                group.label()
            );
        }
    }

    #[test]
    fn chords_do_not_collide_within_a_context() {
        for context in [
            Scope::Global,
            Scope::Editor,
            Scope::FindPanel,
            Scope::FindResults,
        ] {
            let mut chords: Vec<&str> = COMMANDS
                .iter()
                .flat_map(|command| command.bindings.iter())
                .filter(|(_, scope)| *scope == context)
                .map(|(chord, _)| *chord)
                .collect();
            let bound = chords.len();
            chords.sort_unstable();
            chords.dedup();
            assert_eq!(chords.len(), bound, "one chord is bound twice");
        }
    }

    /// Two visible rows dispatching the same action would be a duplicated row.
    /// The `JumpToTab` rows are exempt: they share an action name by design and
    /// appear on neither surface.
    #[test]
    fn visible_commands_have_distinct_actions() {
        let mut names: Vec<String> = COMMANDS
            .iter()
            .filter(|command| command.menu.is_some() || command.palette.is_some())
            .map(|command| (command.make)().name().to_string())
            .collect();
        let visible = names.len();
        names.sort();
        names.dedup();
        assert_eq!(names.len(), visible, "two visible rows dispatch one action");
    }

    /// A user entry replaces its command's default chord; a `-` entry removes
    /// it. Without a file, the table's own chords are registered.
    #[gpui_kit::test]
    async fn user_overrides_replace_and_remove_bindings(cx: &mut TestAppContext) {
        let overrides = crate::keymap::Overrides::parse(
            r#"[
                { "key": "ctrl+alt+up", "command": "Toggle Sidebar" },
                { "key": "", "command": "-New File" }
            ]"#,
        )
        .unwrap();

        let (sidebar, new_file, default_new_file) = cx.read(|cx| {
            let with_overrides = app_bindings_with(cx, &overrides);
            let count = |bindings: &[gpui_kit::KeyBinding], suffix: &str| {
                bindings
                    .iter()
                    .filter(|binding| binding.action().name().ends_with(suffix))
                    .count()
            };
            (
                count(&with_overrides, "ToggleSidebar"),
                count(&with_overrides, "NewFile"),
                count(&app_bindings(cx), "NewFile"),
            )
        });
        assert_eq!(sidebar, 1, "the override is the command's only chord");
        assert_eq!(new_file, 0, "a leading minus removes every chord");
        assert_eq!(default_new_file, 1, "the default survives without the file");
    }

    /// Pins how much of the table each surface shows, so a row dropped while
    /// editing the table shows up here rather than as a missing menu entry.
    #[test]
    fn surface_sizes_are_pinned() {
        assert_eq!(COMMANDS.iter().filter(|c| c.menu.is_some()).count(), 80);
        assert_eq!(COMMANDS.iter().filter(|c| c.palette.is_some()).count(), 78);
    }
}
