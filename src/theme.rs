//! Color schemes: named palettes applied on top of the light/dark appearance.
//!
//! A scheme controls colors only. Font family/size for the editor and the
//! interface and the UI radius stay owned by settings and gpui-kit's defaults,
//! so an installed theme can never fight the user's font settings. Themes are
//! Zed-compatible JSON files; the built-in `Default Light`/`Default Dark`
//! palettes are bundled with the app, and user themes are read from
//! [`themes_dir`].

use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::LazyLock;

use gpui_kit::component::{Theme, ThemeConfig, ThemeMode, ThemeRegistry, ThemeSet};
use gpui_kit::*;

use crate::paths::support_dir;
use crate::settings::{DEFAULT_FONT_WEIGHT, DEFAULT_UI_FONT_SIZE, SYSTEM_UI_FONT, Settings};

/// Seed file written into a freshly created themes directory so the format is
/// discoverable. Only `*.json` files are parsed, so this is never a theme.
const README: &str = "\
# Themes

Drop Zed-compatible theme files (*.json) in this folder. The app reloads them
automatically, and any light or dark theme appears under
Settings → Appearance → Color Scheme.

Each file is a theme set:

{
  \"name\": \"My Themes\",
  \"themes\": [
    {
      \"name\": \"My Dark\",
      \"mode\": \"dark\",
      \"colors\": { \"background\": \"#111111\", \"foreground\": \"#eeeeee\" },
      \"highlight\": {
        \"editor.background\": \"#111111\",
        \"syntax\": { \"keyword\": { \"color\": \"#ff79c6\" } }
      }
    }
  ]
}

A theme controls colors only; editor and interface fonts and layout live in
Settings.
";

/// The directory scanned for user-installed theme files.
pub(crate) fn themes_dir() -> Option<PathBuf> {
    Some(support_dir()?.join("themes"))
}

/// Creates the themes directory, seeding a short README, if it does not exist.
pub(crate) fn ensure_dir(dir: &Path) {
    if dir.exists() {
        return;
    }
    if std::fs::create_dir_all(dir).is_ok() {
        let _ = std::fs::write(dir.join("README.md"), README);
    }
}

/// Creates `dir` and watches it for theme files, reloading them live. The
/// initial load is asynchronous, so `on_load` runs once it finishes, letting the
/// app re-apply a scheme the settings name but the registry had not loaded yet.
pub(crate) fn watch<F>(dir: PathBuf, cx: &mut App, on_load: F)
where
    F: Fn(&mut App) + 'static,
{
    ensure_dir(&dir);
    if let Err(error) = ThemeRegistry::watch_dir(dir, cx, on_load) {
        eprintln!(
            "{}: cannot watch themes directory: {error}",
            env!("CARGO_PKG_NAME")
        );
    }
}

/// Applies the color scheme the settings select for the current appearance.
///
/// `system` resolves the OS appearance, `light`/`dark` are explicit. All theme
/// application goes through here so the appearance, the palette, and the
/// markdown emphasis stay in step.
pub(crate) fn apply(settings: &Settings, window: Option<&mut Window>, cx: &mut App) {
    let choice = settings.ui.theme();
    let mode = if choice == crate::THEME_SYSTEM {
        let appearance = match window.as_deref() {
            Some(window) => window.appearance(),
            None => cx.window_appearance(),
        };
        appearance.into()
    } else {
        crate::theme_mode_from(choice)
    };

    let name = match mode {
        ThemeMode::Light => settings.ui.light_scheme(),
        ThemeMode::Dark => settings.ui.dark_scheme(),
    };
    apply_scheme(name, mode, window, cx);
    apply_ui_font(settings, cx);

    crate::markdown::apply_emphasis(
        cx,
        settings.editor.font_weight().unwrap_or(DEFAULT_FONT_WEIGHT),
    );
}

/// Applies the interface font family and base size from settings to the global
/// theme. Every element inherits them from the root surface, so this reaches
/// the whole chrome. Unset keys fall back to gpui-kit's defaults — the virtual
/// system UI family and its base size — so clearing a setting restores them.
/// `Theme::update` re-resolves the fonts and refreshes every window.
fn apply_ui_font(settings: &Settings, cx: &mut App) {
    let family: SharedString = settings
        .ui
        .font_family
        .clone()
        .unwrap_or_else(|| SYSTEM_UI_FONT.to_string())
        .into();
    let size = settings.ui.font_size().unwrap_or(DEFAULT_UI_FONT_SIZE);
    Theme::update(cx, |theme| {
        theme.font_family = family;
        theme.font_size = px(size);
    });
}

/// The bundled color schemes. Embedded so the default light and dark palettes
/// are always available, even before the themes directory has been read.
const BUILTIN_THEMES: &str = include_str!("../assets/themes/default.json");

/// The bundled scheme set, parsed once.
fn builtin_themes() -> &'static [ThemeConfig] {
    static THEMES: LazyLock<Vec<ThemeConfig>> = LazyLock::new(|| {
        serde_json::from_str::<ThemeSet>(BUILTIN_THEMES)
            .expect("the bundled themes are valid")
            .themes
    });
    THEMES.as_slice()
}

/// The bundled scheme named `name` for `mode`, if there is one.
///
/// The bundled palettes own the default names, so they resolve here before the
/// registry, which ships its own palettes under the same names.
fn builtin_scheme(name: &str, mode: ThemeMode) -> Option<ThemeConfig> {
    builtin_themes()
        .iter()
        .find(|config| config.name.as_ref() == name && config.mode == mode)
        .cloned()
}

/// Applies the named scheme for `mode`, falling back to the built-in palette of
/// that mode when the name is unknown or names a theme of the other mode.
fn apply_scheme(name: &str, mode: ThemeMode, window: Option<&mut Window>, cx: &mut App) {
    let config = builtin_scheme(name, mode).map(Rc::new).or_else(|| {
        ThemeRegistry::global(cx)
            .themes()
            .values()
            .find(|config| config.name.as_ref() == name && config.mode == mode)
            .cloned()
    });

    match config {
        Some(config) => {
            let config = Rc::new(colors_only((*config).clone()));
            Theme::update(cx, |theme| theme.apply_config(&config));
        }
        // The appearance is the primary axis: never let a mismatched or missing
        // scheme flip it.
        _ => Theme::change(mode, window, cx),
    }
}

/// Strips everything but colors from a theme config. Fonts, radius and shadow
/// stay owned by settings and the library defaults.
fn colors_only(mut config: ThemeConfig) -> ThemeConfig {
    config.font_size = None;
    config.font_family = None;
    config.mono_font_family = None;
    config.mono_font_size = None;
    config.radius = None;
    config.radius_lg = None;
    config.shadow = None;
    config
}

/// The names of every available scheme for `mode`, sorted for display. This is
/// the registry's themes plus the bundled palettes, named once.
pub(crate) fn schemes(mode: ThemeMode, cx: &App) -> Vec<SharedString> {
    let mut names: Vec<SharedString> = ThemeRegistry::global(cx)
        .themes()
        .values()
        .filter(|config| config.mode == mode)
        .map(|config| config.name.clone())
        .collect();
    names.extend(
        builtin_themes()
            .iter()
            .filter(|config| config.mode == mode)
            .map(|config| config.name.clone()),
    );
    names.sort_by(|a, b| a.as_ref().cmp(b.as_ref()));
    names.dedup();
    names
}

#[cfg(test)]
mod tests {
    // Explicit imports: `use super::*` would pull in gpui-kit's `test` attribute
    // macro and shadow the built-in one, making `#[test]` recurse.
    use super::{ThemeConfig, ThemeMode, builtin_scheme, builtin_themes, colors_only};

    #[test]
    fn bundled_themes_define_both_appearances() {
        let themes = builtin_themes();

        assert!(themes.iter().any(|theme| theme.mode == ThemeMode::Light));
        assert!(themes.iter().any(|theme| theme.mode == ThemeMode::Dark));
        assert_eq!(themes.len(), 2);

        for theme in themes {
            // A key serde does not know is dropped silently, so check the ones
            // whose spelling is easy to get wrong.
            let colors = &theme.colors;
            assert!(colors.background.is_some());
            assert!(colors.foreground.is_some());
            assert!(colors.primary.is_some());
            assert!(colors.input.is_some());
            assert!(colors.sidebar.is_some());
            assert!(colors.tab_bar.is_some());
            assert!(colors.scrollbar_thumb.is_some());
            assert!(colors.selection.is_some());
            assert!(colors.progress_bar.is_some());
            assert!(colors.drag_border.is_some());
            assert!(colors.link.is_some());

            let highlight = theme.highlight.as_ref().expect("a highlight section");
            assert!(highlight.editor_background.is_some());
            assert!(highlight.syntax.keyword.is_some());
        }
    }

    #[test]
    fn builtin_scheme_resolves_only_for_its_own_mode() {
        let name = builtin_themes()[0].name.to_string();

        assert!(builtin_scheme(&name, ThemeMode::Light).is_some());
        assert!(builtin_scheme(&name, ThemeMode::Dark).is_none());
        assert!(builtin_scheme("Not A Theme", ThemeMode::Light).is_none());
    }

    #[test]
    fn colors_only_keeps_colors_and_drops_structure() {
        let config: ThemeConfig = serde_json::from_value(serde_json::json!({
            "name": "Sample",
            "mode": "dark",
            "font.size": 20,
            "mono_font.family": "Fira Code",
            "radius": 12,
            "radius.lg": 16,
            "shadow": false,
            "colors": { "background": "#101010", "foreground": "#f0f0f0" },
            "highlight": { "syntax": { "keyword": { "color": "#ff0000" } } }
        }))
        .unwrap();

        let stripped = colors_only(config);

        assert_eq!(stripped.name.to_string(), "Sample");
        assert_eq!(stripped.mode, ThemeMode::Dark);
        assert!(stripped.font_size.is_none());
        assert!(stripped.mono_font_family.is_none());
        assert!(stripped.radius.is_none());
        assert!(stripped.radius_lg.is_none());
        assert!(stripped.shadow.is_none());
        assert!(stripped.highlight.is_some());
        assert!(stripped.colors.background.is_some());
    }
}
