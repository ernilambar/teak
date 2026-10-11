mod app;
mod cli;
mod file_icons;
mod keymap;
mod language;
mod markdown;
mod paths;
mod session;
mod settings;
mod update;

use std::path::PathBuf;

use gpui_kit::component::{Theme, ThemeMode, WindowExt as _, input, notification::Notification};
use gpui_kit::*;

use app::{TinytextApp, WINDOW_TITLE};
use cli::{CliCommand, parse_args, print_help};
use paths::path_from_file_url;
use session::load_session;
use settings::{
    DEFAULT_FONT_WEIGHT, DEFAULT_THEME, ICON_STYLE_COLORFUL, ICON_STYLE_MONOCHROME, load_settings,
};

gpui_kit::actions!(
    tinytext,
    [
        NewFile,
        OpenFile,
        AddFolder,
        CloseAllFolders,
        SaveFile,
        SaveFileAs,
        SaveAll,
        CloseTab,
        Quit,
        Hide,
        HideOthers,
        ShowAll,
        ToggleSidebar,
        CommandPalette,
        InstallCli,
        CheckForUpdates,
        About,
        OpenSettings,
        RevealInFinder,
        CopyFilePath,
        CopyRelativePath,
        ZoomIn,
        ZoomOut,
        ZoomReset,
        ToggleWordWrap,
        ToggleWhitespace,
        ToggleLineNumbers,
        ToggleIndentGuides,
        ToggleFolding,
        ToggleAutoClose,
        ToggleSmartIndent,
        ToggleMarkdownPreview,
        Refresh,
        NextTab,
        PreviousTab,
        ReopenClosedTab,
        ToggleLineComment,
        DeleteLine,
        MoveLineUp,
        MoveLineDown,
        CopyLineUp,
        CopyLineDown,
        InsertLineAbove,
        InsertLineBelow,
        SelectLine,
        GoToLine,
        AddCursorAbove,
        AddCursorBelow,
    ]
);

/// Jump to tab `N` (1-based) from `cmd-1`..`cmd-9`. The `actions!` macro only
/// builds unit actions, so the index is carried here instead.
#[derive(Clone, PartialEq, gpui_kit::Action)]
#[action(namespace = tinytext, no_json)]
pub struct JumpToTab(usize);

/// Picks the appearance from the Appearance menu or the command palette. The
/// payload is the `ui.theme` value: `light`, `dark`, or `system`.
#[derive(Clone, PartialEq, gpui_kit::Action)]
#[action(namespace = tinytext, no_json)]
pub struct SetTheme(pub(crate) String);

fn main() {
    match parse_args(std::env::args().skip(1)) {
        CliCommand::Run => {}
        CliCommand::Help => {
            print_help();
            return;
        }
        CliCommand::Version => {
            println!("{} {}", env!("CARGO_PKG_NAME"), env!("CARGO_PKG_VERSION"));
            return;
        }
        CliCommand::Unknown(arg) => {
            eprintln!(
                "{name}: unknown option '{arg}'",
                name = env!("CARGO_PKG_NAME")
            );
            eprintln!(
                "Run '{name} --help' for usage.",
                name = env!("CARGO_PKG_NAME")
            );
            std::process::exit(2);
        }
    }

    let (open_tx, open_rx) = async_channel::unbounded::<Vec<PathBuf>>();

    let application = gpui_kit::application().with_assets(gpui_kit::assets::Assets);
    application.on_open_urls(move |urls| {
        let paths: Vec<PathBuf> = urls
            .iter()
            .filter_map(|url| path_from_file_url(url))
            .collect();
        if !paths.is_empty() {
            let _ = open_tx.try_send(paths);
        }
    });

    application.run(move |cx| {
        gpui_kit::init(cx);
        let settings = load_settings();
        let base_weight = settings
            .as_ref()
            .map(|settings| settings.editor.font_weight().unwrap_or(DEFAULT_FONT_WEIGHT))
            .unwrap_or(DEFAULT_FONT_WEIGHT);
        let theme_choice = settings
            .as_ref()
            .map(|settings| settings.ui.theme().to_string())
            .unwrap_or_else(|_| DEFAULT_THEME.to_string());
        apply_theme(&theme_choice, base_weight, None, cx);

        // Positional arguments are the folders to open at launch. Passing any
        // of them defines the initial root set, so the saved session is skipped.
        let arguments: Vec<PathBuf> = std::env::args()
            .skip(1)
            .filter(|argument| !argument.starts_with('-'))
            .map(PathBuf::from)
            .collect();
        let initial_folders: Vec<PathBuf> = arguments
            .iter()
            .filter(|path| path.is_dir())
            .cloned()
            .collect();
        let session = if arguments.is_empty() {
            load_session()
        } else {
            None
        };

        keymap::bind_keys(cx);
        cx.on_action(|_: &Hide, cx| cx.hide());
        cx.on_action(|_: &HideOthers, cx| cx.hide_other_apps());
        cx.on_action(|_: &ShowAll, cx| cx.unhide_other_apps());
        cx.set_menus(app_menus(&theme_choice));

        let options = WindowOptions {
            titlebar: Some(TitlebarOptions {
                title: Some(WINDOW_TITLE.into()),
                ..Default::default()
            }),
            ..Default::default()
        };

        gpui_kit::open_window(options, cx, move |window, cx| {
            let (settings, settings_error) = match settings {
                Ok(settings) => (settings, None),
                Err(message) => (Default::default(), Some(message)),
            };
            let app = cx.new(|cx| TinytextApp::new(initial_folders.clone(), settings, cx));
            // Block the red close button (and app termination) while there are
            // unsaved edits, prompting instead.
            let close_guard = app.clone();
            window.on_window_should_close(cx, move |window, cx| {
                close_guard.update(cx, |this, cx| this.handle_window_should_close(window, cx))
            });
            // The notification layer is attached after this closure returns.
            if let Some(message) = settings_error {
                window.defer(cx, move |window, cx| {
                    window.push_notification(Notification::error(message), cx);
                });
            }
            if let Some(session) = session {
                app.update(cx, |this, cx| this.restore_session(session, window, cx));
            }
            app.update(cx, |this, cx| this.check_font_family(window, cx));
            // Re-render whenever the window is (de)activated so a file deleted
            // while the app was in the background is reflected in its tab.
            app.update(cx, |_this, cx| {
                cx.observe_window_activation(window, |this, _window, cx| {
                    this.reload_tree();
                    cx.notify()
                })
                .detach();
            });
            // Follow macOS appearance changes while the user chose System.
            app.update(cx, |_this, cx| {
                cx.observe_window_appearance(window, |this, window, cx| {
                    this.on_system_appearance_changed(window, cx);
                })
                .detach();
            });
            let focus_handle = app.read(cx).focus_handle.clone();
            focus_handle.focus(window, cx);

            app.update(cx, |_this, cx| {
                cx.spawn_in(window, async move |this, cx| {
                    while let Ok(paths) = open_rx.recv().await {
                        this.update_in(cx, |this, window, cx| {
                            for path in paths {
                                this.request_open(path, window, cx);
                            }
                        })
                        .ok();
                    }
                })
                .detach();
            });

            app
        })
        .expect("failed to open window");
    });
}

/// The `ui.theme` value that follows the OS appearance.
pub(crate) const THEME_SYSTEM: &str = "system";

/// The appearance choices as `(ui.theme value, menu label)`, in menu order.
pub(crate) const THEME_CHOICES: [(&str, &str); 3] = [
    ("light", "Light"),
    ("dark", "Dark"),
    (THEME_SYSTEM, "System"),
];

/// The file-icon style choices as `(ui.icon_color value, label)`, in menu order.
pub(crate) const ICON_STYLE_CHOICES: [(&str, &str); 2] = [
    (ICON_STYLE_COLORFUL, "Colorful"),
    (ICON_STYLE_MONOCHROME, "Monochrome"),
];

/// Maps an explicit string form of `ui.theme` (kept GPUI-free in settings.rs) to
/// a `ThemeMode`. `system` is resolved by [`apply_theme`] and never reaches
/// here; unknown values fall back to dark.
pub(crate) fn theme_mode_from(theme: &str) -> ThemeMode {
    match theme {
        "light" => ThemeMode::Light,
        _ => ThemeMode::Dark,
    }
}

/// Applies `choice` to the live theme: `system` follows the OS appearance, while
/// `light` and `dark` are explicit. Loading a theme resets the highlight theme,
/// so the markdown emphasis is re-applied on top.
pub(crate) fn apply_theme(
    choice: &str,
    base_weight: f32,
    window: Option<&mut Window>,
    cx: &mut App,
) {
    if choice == THEME_SYSTEM {
        Theme::sync_system_appearance(window, cx);
    } else {
        Theme::change(theme_mode_from(choice), window, cx);
    }
    markdown::apply_emphasis(cx, base_weight);
}

pub(crate) fn app_menus(theme: &str) -> Vec<Menu> {
    vec![
        Menu::new("Tinytext").items([
            MenuItem::action("About Tinytext", About),
            MenuItem::action("Check for Updates…", CheckForUpdates),
            MenuItem::separator(),
            MenuItem::action("Settings…", OpenSettings),
            MenuItem::separator(),
            MenuItem::action("Install Command Line Tool…", InstallCli),
            MenuItem::separator(),
            MenuItem::os_submenu("Services", SystemMenuType::Services),
            MenuItem::separator(),
            MenuItem::action("Hide Tinytext", Hide),
            MenuItem::action("Hide Others", HideOthers),
            MenuItem::action("Show All", ShowAll),
            MenuItem::separator(),
            MenuItem::action("Quit Tinytext", Quit),
        ]),
        Menu::new("File").items([
            MenuItem::action("New File", NewFile),
            MenuItem::action("Open…", OpenFile),
            MenuItem::action("Add Folder…", AddFolder),
            MenuItem::action("Close All Folders", CloseAllFolders),
            MenuItem::separator(),
            MenuItem::action("Save", SaveFile),
            MenuItem::action("Save As…", SaveFileAs),
            MenuItem::action("Save All", SaveAll),
            MenuItem::separator(),
            MenuItem::action("Close Tab", CloseTab),
            MenuItem::action("Reopen Last Closed Tab", ReopenClosedTab),
        ]),
        Menu::new("Edit").items([
            MenuItem::os_action("Undo", input::Undo, OsAction::Undo),
            MenuItem::os_action("Redo", input::Redo, OsAction::Redo),
            MenuItem::separator(),
            MenuItem::os_action("Cut", input::Cut, OsAction::Cut),
            MenuItem::os_action("Copy", input::Copy, OsAction::Copy),
            MenuItem::os_action("Paste", input::Paste, OsAction::Paste),
            MenuItem::separator(),
            MenuItem::action("Copy Path", CopyFilePath),
            MenuItem::action("Copy Relative Path", CopyRelativePath),
            MenuItem::separator(),
            MenuItem::action("Find…", input::Search),
            MenuItem::action("Find and Replace…", input::Replace),
            MenuItem::separator(),
            MenuItem::os_action("Select All", input::SelectAll, OsAction::SelectAll),
            MenuItem::separator(),
            MenuItem::action("Indent", input::Indent),
            MenuItem::action("Outdent", input::Outdent),
            MenuItem::separator(),
            MenuItem::action("Add Cursor Above", AddCursorAbove),
            MenuItem::action("Add Cursor Below", AddCursorBelow),
            MenuItem::separator(),
            MenuItem::action("Toggle Line Comment", ToggleLineComment),
            MenuItem::action("Delete Line", DeleteLine),
            MenuItem::action("Select Line", SelectLine),
            MenuItem::separator(),
            MenuItem::action("Move Line Up", MoveLineUp),
            MenuItem::action("Move Line Down", MoveLineDown),
            MenuItem::action("Copy Line Up", CopyLineUp),
            MenuItem::action("Copy Line Down", CopyLineDown),
            MenuItem::separator(),
            MenuItem::action("Insert Line Above", InsertLineAbove),
            MenuItem::action("Insert Line Below", InsertLineBelow),
            MenuItem::separator(),
            MenuItem::action("Go to Line…", GoToLine),
        ]),
        Menu::new("View").items([
            MenuItem::action("Command Palette…", CommandPalette),
            MenuItem::separator(),
            MenuItem::action("Toggle Sidebar", ToggleSidebar),
            MenuItem::action("Refresh", Refresh),
            MenuItem::separator(),
            MenuItem::action("Zoom In", ZoomIn),
            MenuItem::action("Zoom Out", ZoomOut),
            MenuItem::action("Actual Size", ZoomReset),
            MenuItem::separator(),
            MenuItem::action("Toggle Word Wrap", ToggleWordWrap),
            MenuItem::action("Toggle Invisible Characters", ToggleWhitespace),
            MenuItem::action("Toggle Line Numbers", ToggleLineNumbers),
            MenuItem::action("Toggle Indent Guides", ToggleIndentGuides),
            MenuItem::action("Toggle Code Folding", ToggleFolding),
            MenuItem::action("Toggle Preview", ToggleMarkdownPreview),
            MenuItem::separator(),
            MenuItem::action("Next Tab", NextTab),
            MenuItem::action("Previous Tab", PreviousTab),
            MenuItem::separator(),
            MenuItem::submenu(theme_menu(theme)),
        ]),
    ]
}

/// The Appearance submenu: one item per choice, with a check mark on the active
/// one.
fn theme_menu(theme: &str) -> Menu {
    Menu::new("Appearance").items(THEME_CHOICES.iter().map(|(value, label)| {
        MenuItem::action(*label, SetTheme((*value).to_string())).checked(*value == theme)
    }))
}
