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

use crate::paths::{display_path, reveal_label};
use crate::{
    About, AddCursorAbove, AddCursorBelow, AddFolder, AddNextOccurrence, CheckForUpdates,
    ClearRecent, CloseAllFolders, CloseFindPanel, CloseTab, CommandPalette, CopyFilePath,
    CopyLineDown, CopyLineUp, CopyRelativePath, DeleteLine, FindInFiles, FindNextField,
    FindPreviousField, GoToLine, Hide, HideOthers, InsertLineAbove, InsertLineBelow, InstallCli,
    JumpToTab, MoveLineDown, MoveLineUp, NewFile, NextResult, NextTab, OpenDefaultSettings,
    OpenFile, OpenPath, OpenResult, OpenSettings, OpenThemesFolder, PreviousResult, PreviousTab,
    QuickOpen, Quit, Refresh, ReopenClosedTab, ReplaceAll, RevealInFinder, RunFind, SaveAll,
    SaveFile, SaveFileAs, SelectLine, SetTheme, ShowAll, SplitSelectionIntoLines, THEME_CHOICES,
    ToggleAutoClose, ToggleFolding, ToggleIndentGuides, ToggleLineComment, ToggleLineNumbers,
    ToggleMarkdownPreview, ToggleSidebar, ToggleSmartIndent, ToggleWhitespace, ToggleWordWrap,
    ZoomIn, ZoomOut, ZoomReset,
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
    View,
}

/// A group in the command palette.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum PaletteGroup {
    File,
    Edit,
    View,
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

/// Chords for the app-menu commands, which have no meaning without a native
/// app menu. Off macOS they are bound to nothing.
#[cfg(target_os = "macos")]
const HIDE_KEYS: &[(&str, Scope)] = &[("cmd-h", Scope::Global)];
#[cfg(not(target_os = "macos"))]
const HIDE_KEYS: &[(&str, Scope)] = NO_KEYS;

#[cfg(target_os = "macos")]
const HIDE_OTHERS_KEYS: &[(&str, Scope)] = &[("alt-cmd-h", Scope::Global)];
#[cfg(not(target_os = "macos"))]
const HIDE_OTHERS_KEYS: &[(&str, Scope)] = NO_KEYS;

/// Menu placement for the app-menu commands. They only appear with the native
/// app menu, so off macOS they are on neither the menu bar nor the keymap.
#[cfg(target_os = "macos")]
const MACOS_MENU_SLOT: Option<(MenuKind, u8)> = Some((MenuKind::Teak, 3));
#[cfg(not(target_os = "macos"))]
const MACOS_MENU_SLOT: Option<(MenuKind, u8)> = None;

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
        bindings: &[("cmd-n", Scope::Global)],
        menu: Some((MenuKind::File, 0)),
        palette: Some((PaletteGroup::File, 0)),
        keywords: &[],
    },
    Command {
        label: "Open…",
        make: || Box::new(OpenFile),
        bindings: &[("cmd-o", Scope::Global)],
        menu: Some((MenuKind::File, 0)),
        palette: Some((PaletteGroup::File, 1)),
        keywords: &[],
    },
    Command {
        label: "Quick Open…",
        make: || Box::new(QuickOpen),
        bindings: &[("cmd-p", Scope::Global)],
        menu: Some((MenuKind::File, 0)),
        palette: Some((PaletteGroup::File, 2)),
        keywords: &[],
    },
    Command {
        label: "Add Folder…",
        make: || Box::new(AddFolder),
        bindings: &[("cmd-shift-o", Scope::Global)],
        menu: Some((MenuKind::File, 0)),
        palette: Some((PaletteGroup::File, 3)),
        keywords: &[],
    },
    Command {
        label: "Close All Folders",
        make: || Box::new(CloseAllFolders),
        bindings: &[("alt-cmd-w", Scope::Global)],
        menu: Some((MenuKind::File, 0)),
        palette: Some((PaletteGroup::File, 4)),
        keywords: &[],
    },
    Command {
        label: "Save",
        make: || Box::new(SaveFile),
        bindings: &[("cmd-s", Scope::Global)],
        menu: Some((MenuKind::File, 1)),
        palette: Some((PaletteGroup::File, 5)),
        keywords: &[],
    },
    Command {
        label: "Save As…",
        make: || Box::new(SaveFileAs),
        bindings: &[("cmd-shift-s", Scope::Global)],
        menu: Some((MenuKind::File, 1)),
        palette: Some((PaletteGroup::File, 6)),
        keywords: &[],
    },
    Command {
        label: "Save All",
        make: || Box::new(SaveAll),
        bindings: &[("alt-cmd-s", Scope::Global)],
        menu: Some((MenuKind::File, 1)),
        palette: Some((PaletteGroup::File, 7)),
        keywords: &[],
    },
    Command {
        label: "Close Tab",
        make: || Box::new(CloseTab),
        bindings: &[("cmd-w", Scope::Global)],
        menu: Some((MenuKind::File, 2)),
        palette: Some((PaletteGroup::File, 8)),
        keywords: &[],
    },
    Command {
        // The menu used to read "Reopen Last Closed Tab" while the palette read
        // "Reopen Closed Tab"; one label now serves both.
        label: "Reopen Closed Tab",
        make: || Box::new(ReopenClosedTab),
        bindings: &[("cmd-shift-t", Scope::Global)],
        menu: Some((MenuKind::File, 2)),
        palette: Some((PaletteGroup::File, 9)),
        keywords: &[],
    },
    Command {
        label: reveal_label(),
        make: || Box::new(RevealInFinder),
        bindings: &[("alt-cmd-r", Scope::Global)],
        menu: None,
        palette: Some((PaletteGroup::File, 10)),
        keywords: &[],
    },
    Command {
        // The Open Recent submenu shows this as "Clear Menu", the macOS name.
        label: "Clear Recent",
        make: || Box::new(ClearRecent),
        bindings: NO_KEYS,
        menu: None,
        palette: Some((PaletteGroup::File, 11)),
        keywords: &["recent", "history"],
    },
    // Edit
    Command {
        label: "Find in Files…",
        make: || Box::new(FindInFiles),
        // The editor binds this chord to Replace, so the editor scope is needed
        // for Find in Files to win while an editor has focus.
        bindings: &[
            ("cmd-shift-f", Scope::Global),
            ("cmd-shift-f", Scope::Editor),
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
        bindings: &[("alt-enter", Scope::FindPanel)],
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
        bindings: &[("alt-cmd-c", Scope::Global)],
        menu: Some((MenuKind::Edit, 0)),
        palette: Some((PaletteGroup::Edit, 14)),
        keywords: &[],
    },
    Command {
        label: "Copy Relative Path",
        make: || Box::new(CopyRelativePath),
        bindings: &[("alt-cmd-shift-c", Scope::Global)],
        menu: Some((MenuKind::Edit, 0)),
        palette: Some((PaletteGroup::Edit, 15)),
        keywords: &[],
    },
    Command {
        label: "Toggle Line Comment",
        make: || Box::new(ToggleLineComment),
        bindings: &[("cmd-/", Scope::Editor)],
        menu: Some((MenuKind::Edit, 2)),
        palette: Some((PaletteGroup::Edit, 0)),
        keywords: &[],
    },
    Command {
        label: "Delete Line",
        make: || Box::new(DeleteLine),
        bindings: &[("cmd-shift-k", Scope::Editor)],
        menu: Some((MenuKind::Edit, 2)),
        palette: Some((PaletteGroup::Edit, 1)),
        keywords: &[],
    },
    Command {
        label: "Select Line",
        make: || Box::new(SelectLine),
        bindings: &[("cmd-l", Scope::Editor)],
        menu: Some((MenuKind::Edit, 2)),
        palette: Some((PaletteGroup::Edit, 2)),
        keywords: &[],
    },
    Command {
        label: "Move Line Up",
        make: || Box::new(MoveLineUp),
        bindings: &[("alt-up", Scope::Editor)],
        menu: Some((MenuKind::Edit, 3)),
        palette: Some((PaletteGroup::Edit, 3)),
        keywords: &[],
    },
    Command {
        label: "Move Line Down",
        make: || Box::new(MoveLineDown),
        bindings: &[("alt-down", Scope::Editor)],
        menu: Some((MenuKind::Edit, 3)),
        palette: Some((PaletteGroup::Edit, 4)),
        keywords: &[],
    },
    Command {
        label: "Copy Line Up",
        make: || Box::new(CopyLineUp),
        bindings: &[("shift-alt-up", Scope::Editor)],
        menu: Some((MenuKind::Edit, 3)),
        palette: Some((PaletteGroup::Edit, 5)),
        keywords: &[],
    },
    Command {
        label: "Copy Line Down",
        make: || Box::new(CopyLineDown),
        bindings: &[("shift-alt-down", Scope::Editor)],
        menu: Some((MenuKind::Edit, 3)),
        palette: Some((PaletteGroup::Edit, 6)),
        keywords: &[],
    },
    Command {
        label: "Insert Line Above",
        make: || Box::new(InsertLineAbove),
        bindings: &[("cmd-shift-enter", Scope::Editor)],
        menu: Some((MenuKind::Edit, 4)),
        palette: Some((PaletteGroup::Edit, 7)),
        keywords: &[],
    },
    Command {
        label: "Insert Line Below",
        make: || Box::new(InsertLineBelow),
        bindings: &[("cmd-enter", Scope::Editor)],
        menu: Some((MenuKind::Edit, 4)),
        palette: Some((PaletteGroup::Edit, 8)),
        keywords: &[],
    },
    Command {
        label: "Go to Line…",
        make: || Box::new(GoToLine),
        bindings: &[("cmd-g", Scope::Editor)],
        menu: Some((MenuKind::Edit, 5)),
        palette: Some((PaletteGroup::Edit, 9)),
        keywords: &["goto"],
    },
    Command {
        label: "Add Cursor Above",
        make: || Box::new(AddCursorAbove),
        bindings: &[
            ("cmd-alt-up", Scope::Editor),
            ("ctrl-alt-up", Scope::Editor),
        ],
        menu: Some((MenuKind::Edit, 1)),
        palette: Some((PaletteGroup::Edit, 10)),
        keywords: &[],
    },
    Command {
        label: "Add Cursor Below",
        make: || Box::new(AddCursorBelow),
        bindings: &[
            ("cmd-alt-down", Scope::Editor),
            ("ctrl-alt-down", Scope::Editor),
        ],
        menu: Some((MenuKind::Edit, 1)),
        palette: Some((PaletteGroup::Edit, 11)),
        keywords: &[],
    },
    Command {
        label: "Split into Lines",
        make: || Box::new(SplitSelectionIntoLines),
        bindings: &[("cmd-shift-l", Scope::Editor)],
        menu: Some((MenuKind::Edit, 1)),
        palette: Some((PaletteGroup::Edit, 12)),
        keywords: &[],
    },
    Command {
        label: "Add Next Occurrence",
        make: || Box::new(AddNextOccurrence),
        bindings: &[("cmd-d", Scope::Editor)],
        menu: Some((MenuKind::Edit, 1)),
        palette: Some((PaletteGroup::Edit, 13)),
        keywords: &[],
    },
    // View
    Command {
        label: "Command Palette…",
        make: || Box::new(CommandPalette),
        bindings: &[("cmd-shift-p", Scope::Global)],
        menu: Some((MenuKind::View, 0)),
        palette: None,
        keywords: &[],
    },
    Command {
        label: "Toggle Sidebar",
        make: || Box::new(ToggleSidebar),
        bindings: &[("cmd-b", Scope::Global)],
        menu: Some((MenuKind::View, 1)),
        palette: Some((PaletteGroup::View, 0)),
        keywords: &[],
    },
    Command {
        label: "Refresh",
        make: || Box::new(Refresh),
        bindings: &[("cmd-r", Scope::Global)],
        menu: Some((MenuKind::View, 1)),
        palette: Some((PaletteGroup::View, 1)),
        keywords: &[],
    },
    Command {
        label: "Zoom In",
        make: || Box::new(ZoomIn),
        bindings: &[("cmd-=", Scope::Global), ("cmd-shift-=", Scope::Global)],
        menu: Some((MenuKind::View, 2)),
        palette: Some((PaletteGroup::View, 2)),
        keywords: &[],
    },
    Command {
        label: "Zoom Out",
        make: || Box::new(ZoomOut),
        bindings: &[("cmd--", Scope::Global)],
        menu: Some((MenuKind::View, 2)),
        palette: Some((PaletteGroup::View, 3)),
        keywords: &[],
    },
    Command {
        label: "Actual Size",
        make: || Box::new(ZoomReset),
        bindings: &[("cmd-0", Scope::Global)],
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
        bindings: &[("alt-cmd-i", Scope::Global)],
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
        bindings: &[("cmd-shift-v", Scope::Global)],
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
        bindings: &[("cmd-shift-]", Scope::Global)],
        menu: Some((MenuKind::View, 4)),
        palette: Some((PaletteGroup::View, 13)),
        keywords: &[],
    },
    Command {
        label: "Previous Tab",
        make: || Box::new(PreviousTab),
        bindings: &[("cmd-shift-[", Scope::Global)],
        menu: Some((MenuKind::View, 4)),
        palette: Some((PaletteGroup::View, 14)),
        keywords: &[],
    },
    // App
    Command {
        label: "Settings…",
        make: || Box::new(OpenSettings),
        bindings: &[("cmd-,", Scope::Global)],
        menu: Some((MenuKind::Teak, 1)),
        palette: Some((PaletteGroup::Teak, 0)),
        keywords: &["preferences", "config"],
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
        bindings: HIDE_KEYS,
        menu: MACOS_MENU_SLOT,
        palette: None,
        keywords: &[],
    },
    Command {
        label: "Hide Others",
        make: || Box::new(HideOthers),
        bindings: HIDE_OTHERS_KEYS,
        menu: MACOS_MENU_SLOT,
        palette: None,
        keywords: &[],
    },
    Command {
        label: "Show All",
        make: || Box::new(ShowAll),
        bindings: NO_KEYS,
        menu: MACOS_MENU_SLOT,
        palette: None,
        keywords: &[],
    },
    Command {
        label: "Quit Teak",
        make: || Box::new(Quit),
        bindings: &[("cmd-q", Scope::Global)],
        menu: Some((MenuKind::Teak, 4)),
        palette: None,
        keywords: &[],
    },
    // Jump to tab: one row per chord, since each carries its own index.
    Command {
        label: "Jump to Tab 1",
        make: || Box::new(JumpToTab(1)),
        bindings: &[("cmd-1", Scope::Global)],
        menu: None,
        palette: None,
        keywords: &[],
    },
    Command {
        label: "Jump to Tab 2",
        make: || Box::new(JumpToTab(2)),
        bindings: &[("cmd-2", Scope::Global)],
        menu: None,
        palette: None,
        keywords: &[],
    },
    Command {
        label: "Jump to Tab 3",
        make: || Box::new(JumpToTab(3)),
        bindings: &[("cmd-3", Scope::Global)],
        menu: None,
        palette: None,
        keywords: &[],
    },
    Command {
        label: "Jump to Tab 4",
        make: || Box::new(JumpToTab(4)),
        bindings: &[("cmd-4", Scope::Global)],
        menu: None,
        palette: None,
        keywords: &[],
    },
    Command {
        label: "Jump to Tab 5",
        make: || Box::new(JumpToTab(5)),
        bindings: &[("cmd-5", Scope::Global)],
        menu: None,
        palette: None,
        keywords: &[],
    },
    Command {
        label: "Jump to Tab 6",
        make: || Box::new(JumpToTab(6)),
        bindings: &[("cmd-6", Scope::Global)],
        menu: None,
        palette: None,
        keywords: &[],
    },
    Command {
        label: "Jump to Tab 7",
        make: || Box::new(JumpToTab(7)),
        bindings: &[("cmd-7", Scope::Global)],
        menu: None,
        palette: None,
        keywords: &[],
    },
    Command {
        label: "Jump to Tab 8",
        make: || Box::new(JumpToTab(8)),
        bindings: &[("cmd-8", Scope::Global)],
        menu: None,
        palette: None,
        keywords: &[],
    },
    Command {
        label: "Jump to Tab 9",
        make: || Box::new(JumpToTab(9)),
        bindings: &[("cmd-9", Scope::Global)],
        menu: None,
        palette: None,
        keywords: &[],
    },
];

impl MenuKind {
    /// Menus in bar order.
    const ALL: [Self; 4] = [Self::Teak, Self::File, Self::Edit, Self::View];

    fn label(self) -> &'static str {
        match self {
            Self::Teak => crate::app::WINDOW_TITLE,
            Self::File => "File",
            Self::Edit => "Edit",
            Self::View => "View",
        }
    }

    fn skeleton(self) -> &'static [Entry] {
        match self {
            Self::Teak => TEAK_MENU,
            Self::File => FILE_MENU,
            Self::Edit => EDIT_MENU,
            Self::View => VIEW_MENU,
        }
    }
}

impl PaletteGroup {
    fn label(self) -> &'static str {
        match self {
            Self::File => "File",
            Self::Edit => "Edit",
            Self::View => "View",
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
}

#[cfg(target_os = "macos")]
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

/// Without the native app menu the hide/show commands and the Services menu
/// have no platform equivalent, so the menu goes straight from the app
/// commands to Quit.
#[cfg(not(target_os = "macos"))]
const TEAK_MENU: &[Entry] = &[
    Entry::Cluster(0),
    Entry::Item(MenuItem::separator),
    Entry::Cluster(1),
    Entry::Item(MenuItem::separator),
    Entry::Cluster(2),
    Entry::Item(MenuItem::separator),
    Entry::Cluster(4),
];

const FILE_MENU: &[Entry] = &[
    Entry::Cluster(0),
    Entry::OpenRecent,
    Entry::Item(MenuItem::separator),
    Entry::Cluster(1),
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
    Entry::Cluster(0),
    Entry::Item(MenuItem::separator),
    Entry::Item(find),
    Entry::Item(find_and_replace),
    Entry::Cluster(6),
    Entry::Item(MenuItem::separator),
    Entry::Item(select_all),
    Entry::Item(MenuItem::separator),
    Entry::Item(indent),
    Entry::Item(outdent),
    Entry::Item(MenuItem::separator),
    Entry::Cluster(1),
    Entry::Item(MenuItem::separator),
    Entry::Cluster(2),
    Entry::Item(MenuItem::separator),
    Entry::Cluster(3),
    Entry::Item(MenuItem::separator),
    Entry::Cluster(4),
    Entry::Item(MenuItem::separator),
    Entry::Cluster(5),
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
    PaletteSlot::Appearance,
    PaletteSlot::Group(PaletteGroup::Teak),
];

#[cfg(target_os = "macos")]
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

fn find() -> MenuItem {
    MenuItem::action("Find…", input::Search)
}

fn find_and_replace() -> MenuItem {
    MenuItem::action("Find and Replace…", input::Replace)
}

fn indent() -> MenuItem {
    MenuItem::action("Indent", input::Indent)
}

fn outdent() -> MenuItem {
    MenuItem::action("Outdent", input::Outdent)
}

/// Bindings that are active regardless of which element has focus.
pub(crate) fn app_bindings() -> Vec<KeyBinding> {
    bindings(Scope::Global)
}

/// VS Code's macOS editing chords, scoped to the focused editor.
///
/// These are registered after the engine's own `"Input"` bindings; at equal
/// context depth the later registration wins, so these take precedence where
/// they overlap:
/// - `shift-alt-up` / `shift-alt-down` — VS Code copies the line, replacing the
///   engine's alternate add-cursor chord.
/// - `cmd-alt-up` / `cmd-alt-down` (and the `ctrl-alt-*` alternate) — the engine
///   binds these to its own add-cursor actions; the app binds the same chords to
///   its `AddCursorAbove` / `AddCursorBelow` so the menu and palette share one
///   path (the handler re-dispatches the engine action at the editor).
/// - `cmd-enter` — VS Code inserts a line below; the engine maps
///   `secondary-enter` to a plain newline on macOS, so this is the useful one.
pub(crate) fn editor_bindings() -> Vec<KeyBinding> {
    bindings(Scope::Editor)
}

/// Enter, Escape and Tab in the Find panel's fields, and Enter in the Find
/// Results tab. Their contexts nest the engine's `"Input"`, and they are
/// registered after the engine's own bindings, so they win over its Enter
/// (newline / submit), Escape and Tab (indent).
pub(crate) fn find_bindings() -> Vec<KeyBinding> {
    let mut list = bindings(Scope::FindPanel);
    list.extend(bindings(Scope::FindResults));
    list
}

fn bindings(context: Scope) -> Vec<KeyBinding> {
    let mut bindings = Vec::new();
    for command in COMMANDS {
        for (chord, scope) in command.bindings {
            if *scope != context {
                continue;
            }
            bindings.push(binding(chord, (command.make)(), *scope));
        }
    }
    bindings
}

/// Builds one binding. This is `KeyBinding::new` with the action already boxed:
/// the table holds `Box<dyn Action>`, which does not satisfy `new`'s
/// `impl Action` bound, so the equivalent public constructor is used with the
/// same arguments (`new` passes `false`, `None` and `DummyKeyboardMapper`).
fn binding(chord: &str, action: Box<dyn Action>, context: Scope) -> KeyBinding {
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
    KeyBinding::load(chord, action, predicate, false, None, &DummyKeyboardMapper)
        .unwrap_or_else(|_| panic!("`{chord}` is a valid chord"))
}

/// The whole menu bar, derived from [`COMMANDS`] plus each menu's skeleton.
pub(crate) fn menus(theme: &str, recent: &[PathBuf]) -> Vec<Menu> {
    MenuKind::ALL
        .iter()
        .map(|&kind| {
            let mut items = Vec::new();
            for entry in kind.skeleton() {
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
                }
            }
            Menu::new(kind.label()).items(items)
        })
        .collect()
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
        .map(|path| MenuItem::action(display_path(path), OpenPath(path.clone())))
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
    use gpui_kit::{Action as _, KeyBinding, KeyContext, Keymap, Keystroke};

    use super::{
        COMMANDS, Entry, MenuKind, PALETTE, PaletteGroup, PaletteSlot, Scope, app_bindings,
        editor_bindings, find_bindings,
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
        // Building the lists is itself the assertion that every chord in the
        // table is well-formed, since a chord that fails to parse panics.
        assert_eq!(app_bindings().len(), 40);
        assert_eq!(editor_bindings().len(), 18);
        assert_eq!(find_bindings().len(), 6);
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
            ("cmd-shift-f", "FindInFiles"),
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
            KeyBinding::new("shift-alt-up", crate::MoveLineUp, Some("Input")),
            KeyBinding::new("shift-alt-down", crate::MoveLineDown, Some("Input")),
            KeyBinding::new("secondary-enter", crate::MoveLineUp, Some("Input")),
            // The engine's own add-cursor chords. The app now binds the same
            // chords to its actions, so the later registration must still win.
            KeyBinding::new(
                "cmd-alt-up",
                gpui_kit::base::input::AddCursorAbove,
                Some("Input"),
            ),
            KeyBinding::new(
                "cmd-alt-down",
                gpui_kit::base::input::AddCursorBelow,
                Some("Input"),
            ),
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

    /// A command placed at a cluster no skeleton entry shows would silently miss
    /// the menu bar, so every placement has to resolve.
    #[test]
    fn every_menu_placement_reaches_a_cluster() {
        for command in COMMANDS {
            let Some((kind, rank)) = command.menu else {
                continue;
            };
            assert!(
                kind.skeleton()
                    .iter()
                    .any(|entry| matches!(entry, Entry::Cluster(cluster) if *cluster == rank)),
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
        #[cfg(target_os = "macos")]
        let teak_commands: &[&str] = &[
            "About Teak",
            "Check for Updates…",
            "Settings…",
            "Open Themes Folder",
            "Install Command Line Tool…",
            "Hide Teak",
            "Hide Others",
            "Show All",
            "Quit Teak",
        ];
        #[cfg(not(target_os = "macos"))]
        let teak_commands: &[&str] = &[
            "About Teak",
            "Check for Updates…",
            "Settings…",
            "Open Themes Folder",
            "Install Command Line Tool…",
            "Quit Teak",
        ];

        let expected: &[(MenuKind, &[&str])] = &[
            (MenuKind::Teak, teak_commands),
            (
                MenuKind::File,
                &[
                    "New File",
                    "Open…",
                    "Quick Open…",
                    "Add Folder…",
                    "Close All Folders",
                    "Save",
                    "Save As…",
                    "Save All",
                    "Close Tab",
                    "Reopen Closed Tab",
                ],
            ),
            (
                MenuKind::Edit,
                &[
                    "Copy Path",
                    "Copy Relative Path",
                    "Find in Files…",
                    "Next Result",
                    "Previous Result",
                    "Add Cursor Above",
                    "Add Cursor Below",
                    "Split into Lines",
                    "Add Next Occurrence",
                    "Toggle Line Comment",
                    "Delete Line",
                    "Select Line",
                    "Move Line Up",
                    "Move Line Down",
                    "Copy Line Up",
                    "Copy Line Down",
                    "Insert Line Above",
                    "Insert Line Below",
                    "Go to Line…",
                ],
            ),
            (
                MenuKind::View,
                &[
                    "Command Palette…",
                    "Toggle Sidebar",
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
        ];

        for (kind, labels) in expected {
            let actual: Vec<&str> = kind
                .skeleton()
                .iter()
                .flat_map(|entry| match entry {
                    Entry::Cluster(cluster) => COMMANDS
                        .iter()
                        .filter(|command| command.menu == Some((*kind, *cluster)))
                        .map(|command| command.label)
                        .collect(),
                    _ => Vec::new(),
                })
                .collect();
            assert_eq!(actual, *labels, "the {} menu changed", kind.label());
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

    /// Pins how much of the table each surface shows, so a row dropped while
    /// editing the table shows up here rather than as a missing menu entry.
    #[test]
    fn surface_sizes_are_pinned() {
        #[cfg(target_os = "macos")]
        const MENU_COMMANDS: usize = 52;
        #[cfg(not(target_os = "macos"))]
        const MENU_COMMANDS: usize = 49;

        assert_eq!(
            COMMANDS.iter().filter(|c| c.menu.is_some()).count(),
            MENU_COMMANDS
        );
        assert_eq!(COMMANDS.iter().filter(|c| c.palette.is_some()).count(), 50);
    }
}
