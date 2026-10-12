//! Color schemes: named palettes applied on top of the light/dark appearance.
//!
//! A scheme controls colors only. Editor font family/size/weight and the UI
//! radius stay owned by settings and gpui-kit's defaults, so an installed theme
//! can never fight the user's editor settings. Themes are Zed-compatible JSON
//! files; the built-in `Default Light`/`Default Dark` come from gpui-kit's
//! registry, and user themes are read from [`themes_dir`].

use std::path::{Path, PathBuf};
use std::rc::Rc;

use gpui_kit::component::{Theme, ThemeConfig, ThemeMode, ThemeRegistry};
use gpui_kit::*;

use crate::paths::support_dir;
use crate::settings::{DEFAULT_FONT_WEIGHT, Settings};

/// Seed file written into a freshly created themes directory so the format is
/// discoverable. Only `*.json` files are parsed, so this is never a theme.
const README: &str = "\
# Tinytext themes

Drop Zed-compatible theme files (*.json) in this folder. Tinytext reloads them
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

A theme controls colors only; editor fonts and layout live in Settings.
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
        eprintln!("tinytext: cannot watch themes directory: {error}");
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

    crate::markdown::apply_emphasis(
        cx,
        settings.editor.font_weight().unwrap_or(DEFAULT_FONT_WEIGHT),
    );
}

/// Applies the named scheme for `mode`, falling back to the built-in palette of
/// that mode when the name is unknown or names a theme of the other mode.
fn apply_scheme(name: &str, mode: ThemeMode, window: Option<&mut Window>, cx: &mut App) {
    let config = ThemeRegistry::global(cx)
        .themes()
        .values()
        .find(|config| config.name.as_ref() == name)
        .cloned();

    match config {
        Some(config) if config.mode == mode => {
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

/// The names of every registered scheme for `mode`, sorted for display.
pub(crate) fn schemes(mode: ThemeMode, cx: &App) -> Vec<SharedString> {
    let mut names: Vec<SharedString> = ThemeRegistry::global(cx)
        .themes()
        .values()
        .filter(|config| config.mode == mode)
        .map(|config| config.name.clone())
        .collect();
    names.sort_by(|a, b| a.as_ref().cmp(b.as_ref()));
    names
}

#[cfg(test)]
mod tests {
    // Explicit imports: `use super::*` would pull in gpui-kit's `test` attribute
    // macro and shadow the built-in one, making `#[test]` recurse.
    use super::{ThemeConfig, ThemeMode, colors_only};

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
