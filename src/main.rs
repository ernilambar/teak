mod app;
mod cli;
mod commands;
mod file_icons;
mod fuzzy;
mod keymap;
mod language;
mod markdown;
mod paths;
mod search;
mod session;
mod settings;
mod theme;
mod update;

use std::path::PathBuf;

use gpui_kit::component::{ThemeMode, ThemeRegistry, WindowExt as _, notification::Notification};
use gpui_kit::*;

use app::{TeakApp, WINDOW_TITLE};
use cli::{CliCommand, parse_args, print_help};
use paths::path_from_file_url;
use session::load_session;
use settings::{ICON_STYLE_COLORFUL, ICON_STYLE_MONOCHROME, load_settings};

gpui_kit::actions!(
    teak,
    [
        NewFile,
        OpenFile,
        QuickOpen,
        FindInFiles,
        RunFind,
        ReplaceAll,
        CloseFindPanel,
        FindNextField,
        FindPreviousField,
        OpenResult,
        NextResult,
        PreviousResult,
        AddFolder,
        CloseAllFolders,
        ClearRecent,
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
        OpenDefaultSettings,
        OpenThemesFolder,
        RevealInFinder,
        CopyFilePath,
        CopyRelativePath,
        CopyFileName,
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
        JoinLines,
        TrimTrailingWhitespace,
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
        SplitSelectionIntoLines,
        AddNextOccurrence,
    ]
);

/// Jump to tab `N` (1-based) from `secondary-1`..`secondary-9`. The `actions!`
/// macro only builds unit actions, so the index is carried here instead.
#[derive(Clone, PartialEq, gpui_kit::Action)]
#[action(namespace = teak, no_json)]
pub struct JumpToTab(usize);

/// Opens the file the quick-open finder confirmed, from the quick-open chord.
/// The `actions!` macro only builds unit actions, so the path is carried here
/// instead.
#[derive(Clone, PartialEq, gpui_kit::Action)]
#[action(namespace = teak, no_json)]
pub struct OpenPath(pub(crate) PathBuf);

/// Picks the appearance from the Appearance menu or the command palette. The
/// payload is the `ui.theme` value: `light`, `dark`, or `system`.
#[derive(Clone, PartialEq, gpui_kit::Action)]
#[action(namespace = teak, no_json)]
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
        // Apply the saved appearance and color scheme now. User-installed themes
        // load asynchronously from the themes directory (below), so a scheme the
        // settings name but that is not registered yet falls back to the
        // built-in palette and is re-applied once the load finishes.
        let resolved = settings.clone().unwrap_or_default();
        theme::apply(&resolved, None, cx);
        let theme_choice = resolved.ui.theme().to_string();

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
        // The app replaces these once it has loaded the recent list.
        cx.set_menus(commands::menus(&theme_choice, &[]));

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
            let app = cx.new(|cx| TeakApp::new(initial_folders.clone(), settings, cx));
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
            // Watch the user themes directory so installed color schemes load
            // and hot-reload. The initial load is asynchronous, so re-apply the
            // scheme once it finishes to pick up one the settings named.
            if let Some(dir) = theme::themes_dir() {
                let weak = app.downgrade();
                theme::watch(dir, cx, move |cx| {
                    weak.update(cx, |this, cx| this.reapply_theme(cx)).ok();
                });
            }
            // A reload of the themes directory re-applies the library's copy of
            // the default palettes, which share the bundled names. Re-apply the
            // scheme the settings select whenever the registry changes so the
            // bundled palettes stay in effect.
            let registry_weak = app.downgrade();
            cx.observe_global::<ThemeRegistry>(move |cx| {
                registry_weak
                    .update(cx, |this, cx| this.reapply_theme(cx))
                    .ok();
            })
            .detach();
            // Re-render whenever the window is (de)activated so a file deleted
            // while the app was in the background is reflected in its tab.
            app.update(cx, |_this, cx| {
                cx.observe_window_activation(window, |this, window, cx| {
                    this.reload_tree();
                    if window.is_window_active() {
                        this.check_outside_change(window, cx);
                    }
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

/// The sidebar position choices as `(ui.sidebar_position value, label)`, in
/// menu order.
pub(crate) const SIDEBAR_POSITION_CHOICES: [(&str, &str); 2] =
    [("left", "Left"), ("right", "Right")];

/// Maps an explicit string form of `ui.theme` (kept GPUI-free in settings.rs) to
/// a `ThemeMode`. `system` is resolved by [`theme::apply`] and never reaches
/// here; unknown values fall back to dark.
pub(crate) fn theme_mode_from(theme: &str) -> ThemeMode {
    match theme {
        "light" => ThemeMode::Light,
        _ => ThemeMode::Dark,
    }
}
