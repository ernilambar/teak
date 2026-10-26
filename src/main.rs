mod app;
mod cli;
mod commands;
mod file_icons;
mod fuzzy;
mod keymap;
mod language;
mod markdown;
mod paths;
mod print;
mod search;
mod session;
mod settings;
mod theme;
mod update;
mod windows;

use std::path::PathBuf;

use gpui_kit::component::{ThemeMode, ThemeRegistry};

use cli::{CliCommand, parse_args, print_help};
use paths::path_from_file_url;
use session::load_session;
use settings::{ICON_STYLE_COLORFUL, ICON_STYLE_MONOCHROME, load_settings};
use windows::{AppServices, WindowSeed};

gpui_kit::actions!(
    teak,
    [
        NewFile,
        NewWindow,
        CloseWindow,
        OpenFile,
        OpenFolderInNewWindow,
        MoveTabToNewWindow,
        MinimizeWindow,
        ZoomWindow,
        ToggleFullScreen,
        BringAllToFront,
        QuickOpen,
        FindInEditor,
        FindAndReplaceInEditor,
        FindNextInEditor,
        FindPreviousInEditor,
        Print,
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
        ConvertToUppercase,
        ConvertToLowercase,
        ConvertToTitleCase,
        ConvertToCamelCase,
        ConvertToPascalCase,
        ConvertToSnakeCase,
        ConvertToKebabCase,
        ConvertToConstantCase,
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
    // `Application::on_reopen` runs without access to `App`, so a dock click
    // hands the request to a task that can create the window.
    let (reopen_tx, reopen_rx) = async_channel::unbounded::<()>();

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
    application.on_reopen(move |_cx| {
        let _ = reopen_tx.try_send(());
    });

    application.run(move |cx| {
        gpui_kit::init(cx);
        let (settings, settings_error) = match load_settings() {
            Ok(settings) => (settings, None),
            Err(message) => (Default::default(), Some(message)),
        };
        // Apply the saved appearance and color scheme now. User-installed themes
        // load asynchronously from the themes directory (below), so a scheme the
        // settings name but that is not registered yet falls back to the
        // built-in palette and is re-applied once the load finishes.
        theme::apply(&settings, None, cx);
        let theme_choice = settings.ui.theme().to_string();

        // Positional arguments are paths to open at launch. Passing any of them
        // replaces the saved window layout, but still shares its recent list.
        let arguments: Vec<PathBuf> = std::env::args()
            .skip(1)
            .filter(|argument| !argument.starts_with('-'))
            .map(PathBuf::from)
            .collect();
        let saved = load_session();
        let recent = saved
            .as_ref()
            .map(|session| session.recent.clone())
            .unwrap_or_default();
        let services = AppServices::new(cx, recent.clone());

        keymap::bind_keys(cx);
        cx.on_action(|_: &Hide, cx| cx.hide());
        cx.on_action(|_: &HideOthers, cx| cx.hide_other_apps());
        cx.on_action(|_: &ShowAll, cx| cx.unhide_other_apps());
        // With no window focused, the menu bar still dispatches app-level
        // actions here; the per-window handlers cover the normal case.
        let services_for_new = services.clone();
        cx.on_action(move |_: &NewWindow, cx| {
            if cx.active_window().is_none() {
                services_for_new
                    .open_window(cx, WindowSeed::blank(load_settings().unwrap_or_default()));
            }
        });
        cx.on_action(|_: &Quit, cx| {
            if cx.windows().is_empty() {
                cx.quit();
            }
        });
        // The menu bar stays usable after the last window closes; an Open Recent
        // row then starts a window at that file.
        let services_for_open_path = services.clone();
        cx.on_action(move |action: &OpenPath, cx| {
            if cx.active_window().is_none() {
                services_for_open_path.route_open(vec![action.0.clone()], cx);
            }
        });
        // The app replaces these once it has loaded the recent list.
        cx.set_menus(commands::menus(&theme_choice, &recent));

        // Watch the user themes directory so installed color schemes load and
        // hot-reload. The watcher belongs to the app, not a window: one watcher
        // serves every window, and a scheme that finishes loading re-applies to
        // all of them.
        if let Some(dir) = theme::themes_dir() {
            let services = services.clone();
            theme::watch(dir, cx, move |cx| services.reapply_theme_all(cx));
        }
        // A reload of the themes directory re-applies the library's copy of the
        // default palettes, which share the bundled names. Re-apply the scheme
        // the settings select whenever the registry changes so the bundled
        // palettes stay in effect.
        let services_for_registry = services.clone();
        cx.observe_global::<ThemeRegistry>(move |cx| {
            services_for_registry.reapply_theme_all(cx);
        })
        .detach();

        // Dock click or Cmd+Tab after the last window closed: open a fresh one.
        let services_for_reopen = services.clone();
        cx.spawn(async move |cx| {
            while reopen_rx.recv().await.is_ok() {
                cx.update(|cx| {
                    if services_for_reopen.has_windows(cx) {
                        // Minimized or hidden windows come back to the front.
                        cx.activate(true);
                    } else {
                        services_for_reopen.open_window(
                            cx,
                            WindowSeed::blank(load_settings().unwrap_or_default()),
                        );
                    }
                });
            }
        })
        .detach();

        // External opens (the command-line launcher and file associations) go to
        // one window, whichever is focused, or start one when none is open.
        let services_for_open = services.clone();
        cx.spawn(async move |cx| {
            while let Ok(paths) = open_rx.recv().await {
                cx.update(|cx| services_for_open.route_open(paths, cx));
            }
        })
        .detach();

        let mut seeds: Vec<WindowSeed> = Vec::new();
        if arguments.is_empty() {
            if let Some(saved) = saved {
                for window in saved.windows {
                    seeds.push(WindowSeed::restored(settings.clone(), window));
                }
            }
        } else {
            seeds.push(WindowSeed::for_paths(settings.clone(), arguments));
        }
        if seeds.is_empty() {
            seeds.push(WindowSeed::blank(settings.clone()));
        }
        for (ix, mut seed) in seeds.into_iter().enumerate() {
            if ix == 0 {
                seed.settings_error = settings_error.clone();
            }
            services.open_window(cx, seed);
        }
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
