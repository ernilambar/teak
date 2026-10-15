use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::paths::support_dir;

pub(crate) const MIN_FONT_SIZE: f32 = 6.;
pub(crate) const MAX_FONT_SIZE: f32 = 72.;
pub(crate) const MIN_FONT_WEIGHT: f32 = 100.;
pub(crate) const MAX_FONT_WEIGHT: f32 = 900.;
pub(crate) const DEFAULT_FONT_WEIGHT: f32 = 400.;
/// Bounds for `ui.font_size`. Tighter than the editor's because the interface
/// scales as a whole when its base size changes.
pub(crate) const MIN_UI_FONT_SIZE: f32 = 9.;
pub(crate) const MAX_UI_FONT_SIZE: f32 = 24.;
/// Base interface size when `ui.font_size` is unset. Matches gpui-kit's default
/// theme size, so leaving it unset keeps the interface as it was.
pub(crate) const DEFAULT_UI_FONT_SIZE: f32 = 16.;
/// The virtual family GPUI resolves to the platform UI font. `ui.font_family`
/// unset means this, so clearing the setting restores the system font.
pub(crate) const SYSTEM_UI_FONT: &str = ".SystemUIFont";
/// Effective tab width when `editor.tab_size` is unset.
pub(crate) const DEFAULT_TAB_SIZE: usize = 4;
/// Scroll margin used when `editor.cursor_surrounding_lines` is unset. Mirrors
/// gpui-base's own default, so the settings overlay offers the value the engine
/// would pick on its own.
pub(crate) const DEFAULT_CURSOR_SURROUNDING_LINES: usize = 3;
/// Theme used when `ui.theme` is unset. Kept as a plain string so settings.rs
/// stays free of GPUI types.
pub(crate) const DEFAULT_THEME: &str = "dark";
/// Color scheme applied in the light appearance when `ui.color_scheme_light` is
/// unset. Names the built-in theme gpui-kit registers, so leaving it unset keeps
/// the exact palette the app had before color schemes existed.
pub(crate) const DEFAULT_LIGHT_SCHEME: &str = "Default Light";
/// Color scheme applied in the dark appearance when `ui.color_scheme_dark` is
/// unset.
pub(crate) const DEFAULT_DARK_SCHEME: &str = "Default Dark";

/// File-icon style that keeps each type's own colors (the Material Icon Theme
/// palette), mapped to by the `ui.icon_color` value "colorful".
pub(crate) const ICON_STYLE_COLORFUL: &str = "colorful";
/// File-icon style that paints every icon as a single color following the
/// theme. The default, since it matches the app's muted gray UI.
pub(crate) const ICON_STYLE_MONOCHROME: &str = "monochrome";
/// Icon style used when `ui.icon_color` is unset.
pub(crate) const DEFAULT_ICON_STYLE: &str = ICON_STYLE_MONOCHROME;

/// Workspace tree position used when `ui.sidebar_position` is unset.
pub(crate) const DEFAULT_SIDEBAR_POSITION: &str = "left";

/// Unknown `ui.sidebar_position` values fall back to this.
pub(crate) const SIDEBAR_POSITION_RIGHT: &str = "right";

/// How file-type icons are painted. Kept GPUI-free; the renderer maps it to the
/// right element.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum IconStyle {
    /// Each file type keeps its own colors.
    Colorful,
    /// A single silhouette that follows the theme's foreground color.
    Monochrome,
}

/// Suggested when the configured family has no close match; only installed
/// ones are shown.
const MONOSPACE_FONTS: &[&str] = &[
    "SF Mono",
    "Menlo",
    "Monaco",
    "JetBrains Mono",
    "Fira Code",
    "Source Code Pro",
    "Cascadia Code",
    "IBM Plex Mono",
    "Hack",
    "Courier New",
];

/// Suggested when the configured interface family has no close match; only
/// installed ones are shown. Sans-serif families, unlike [`MONOSPACE_FONTS`].
const UI_FONTS: &[&str] = &[
    "SF Pro Text",
    "SF Pro",
    "Helvetica Neue",
    "Avenir Next",
    "Inter",
    "Roboto",
    "Segoe UI",
    "Ubuntu",
];
const MAX_SUGGESTIONS: usize = 3;

/// User-authored preferences. Grouped by area so new keys land in an existing
/// section (`editor.tab_size`) or a new one (`ui.font_size`) without renames.
///
/// Unknown keys are rejected so a typo like `font_sise` surfaces as an error
/// instead of silently doing nothing.
#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct Settings {
    pub(crate) editor: EditorSettings,
    pub(crate) ui: UiSettings,
}

/// `None` keeps the theme's monospace default.
#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct EditorSettings {
    pub(crate) font_family: Option<String>,
    pub(crate) font_size: Option<f32>,
    pub(crate) font_weight: Option<f32>,
    pub(crate) soft_wrap: Option<bool>,
    pub(crate) show_whitespace: Option<bool>,
    pub(crate) tab_size: Option<usize>,
    pub(crate) hard_tabs: Option<bool>,
    pub(crate) line_numbers: Option<bool>,
    pub(crate) indent_guides: Option<bool>,
    pub(crate) folding: Option<bool>,
    pub(crate) auto_close: Option<bool>,
    pub(crate) smart_indent: Option<bool>,
    pub(crate) indent_wrapped_lines: Option<bool>,
    pub(crate) scroll_beyond_last_line: Option<bool>,
    /// `None` keeps the engine's default margin.
    pub(crate) cursor_surrounding_lines: Option<usize>,
}

impl EditorSettings {
    pub(crate) fn font_size(&self) -> Option<f32> {
        self.font_size
            .map(|size| size.clamp(MIN_FONT_SIZE, MAX_FONT_SIZE))
    }

    pub(crate) fn font_weight(&self) -> Option<f32> {
        self.font_weight
            .map(|weight| weight.clamp(MIN_FONT_WEIGHT, MAX_FONT_WEIGHT))
    }

    pub(crate) fn soft_wrap(&self) -> bool {
        self.soft_wrap.unwrap_or(false)
    }

    pub(crate) fn show_whitespace(&self) -> bool {
        self.show_whitespace.unwrap_or(false)
    }

    pub(crate) fn tab_size(&self) -> usize {
        self.tab_size.unwrap_or(DEFAULT_TAB_SIZE)
    }

    pub(crate) fn hard_tabs(&self) -> bool {
        self.hard_tabs.unwrap_or(false)
    }

    pub(crate) fn line_numbers(&self) -> bool {
        self.line_numbers.unwrap_or(true)
    }

    pub(crate) fn indent_guides(&self) -> bool {
        self.indent_guides.unwrap_or(true)
    }

    pub(crate) fn folding(&self) -> bool {
        self.folding.unwrap_or(true)
    }

    pub(crate) fn auto_close(&self) -> bool {
        self.auto_close.unwrap_or(true)
    }

    pub(crate) fn smart_indent(&self) -> bool {
        self.smart_indent.unwrap_or(true)
    }

    pub(crate) fn indent_wrapped_lines(&self) -> bool {
        self.indent_wrapped_lines.unwrap_or(true)
    }

    pub(crate) fn scroll_beyond_last_line(&self) -> bool {
        self.scroll_beyond_last_line.unwrap_or(true)
    }
}

/// Application-level preferences.
#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct UiSettings {
    /// The appearance mode: `light`, `dark`, or `system`.
    pub(crate) theme: Option<String>,
    /// Color scheme applied in the light appearance, by theme name.
    pub(crate) color_scheme_light: Option<String>,
    /// Color scheme applied in the dark appearance, by theme name.
    pub(crate) color_scheme_dark: Option<String>,
    /// `None` keeps the system UI font.
    pub(crate) font_family: Option<String>,
    /// `None` keeps the theme's base size.
    pub(crate) font_size: Option<f32>,
    pub(crate) tab_icons: Option<bool>,
    pub(crate) icon_color: Option<String>,
    /// Which side of the workspace the file tree sits on.
    pub(crate) sidebar_position: Option<String>,
}

impl UiSettings {
    pub(crate) fn theme(&self) -> &str {
        self.theme.as_deref().unwrap_or(DEFAULT_THEME)
    }

    pub(crate) fn font_size(&self) -> Option<f32> {
        self.font_size
            .map(|size| size.clamp(MIN_UI_FONT_SIZE, MAX_UI_FONT_SIZE))
    }

    /// The color scheme for the light appearance, falling back to the built-in.
    pub(crate) fn light_scheme(&self) -> &str {
        self.color_scheme_light
            .as_deref()
            .unwrap_or(DEFAULT_LIGHT_SCHEME)
    }

    /// The color scheme for the dark appearance, falling back to the built-in.
    pub(crate) fn dark_scheme(&self) -> &str {
        self.color_scheme_dark
            .as_deref()
            .unwrap_or(DEFAULT_DARK_SCHEME)
    }

    pub(crate) fn tab_icons(&self) -> bool {
        self.tab_icons.unwrap_or(true)
    }

    pub(crate) fn icon_color(&self) -> &str {
        self.icon_color.as_deref().unwrap_or(DEFAULT_ICON_STYLE)
    }

    pub(crate) fn sidebar_position(&self) -> &str {
        self.sidebar_position
            .as_deref()
            .unwrap_or(DEFAULT_SIDEBAR_POSITION)
    }

    pub(crate) fn sidebar_right(&self) -> bool {
        self.sidebar_position() == SIDEBAR_POSITION_RIGHT
    }

    /// Resolves `ui.icon_color`, falling back to the default for a value the
    /// renderer does not know.
    pub(crate) fn icon_style(&self) -> IconStyle {
        match self.icon_color() {
            ICON_STYLE_MONOCHROME => IconStyle::Monochrome,
            _ => IconStyle::Colorful,
        }
    }
}

pub(crate) fn settings_path() -> Option<PathBuf> {
    Some(support_dir()?.join("settings.json"))
}

pub(crate) fn parse_settings(data: &str) -> Result<Settings, serde_json::Error> {
    serde_json::from_str(data)
}

/// A missing file yields defaults. Unreadable or invalid files return a
/// message for the user; callers keep defaults or the previous settings.
pub(crate) fn load_settings() -> Result<Settings, String> {
    let Some(path) = settings_path() else {
        return Ok(Settings::default());
    };
    let data = match std::fs::read_to_string(&path) {
        Ok(data) => data,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(Settings::default());
        }
        Err(error) => return Err(format!("Could not read settings: {error}")),
    };
    parse_settings(&data).map_err(|error| format!("Invalid settings: {error}"))
}

/// Every settings key at the value it takes when left unset, as pretty JSON in
/// the shape of `settings.json`. Values are read through the same accessors the
/// app uses, so the document cannot disagree with the real defaults. The theme
/// supplies the default editor and interface font families and sizes, which
/// this module cannot read itself, so the caller passes them in.
pub(crate) fn defaults_document(
    mono_family: &str,
    mono_size: f32,
    ui_family: &str,
    ui_size: f32,
) -> String {
    let editor = EditorSettings::default();
    let ui = UiSettings::default();
    let defaults = Settings {
        editor: EditorSettings {
            font_family: Some(mono_family.to_string()),
            font_size: Some(mono_size),
            font_weight: Some(DEFAULT_FONT_WEIGHT),
            soft_wrap: Some(editor.soft_wrap()),
            show_whitespace: Some(editor.show_whitespace()),
            tab_size: Some(editor.tab_size()),
            hard_tabs: Some(editor.hard_tabs()),
            line_numbers: Some(editor.line_numbers()),
            indent_guides: Some(editor.indent_guides()),
            folding: Some(editor.folding()),
            auto_close: Some(editor.auto_close()),
            smart_indent: Some(editor.smart_indent()),
            indent_wrapped_lines: Some(editor.indent_wrapped_lines()),
            scroll_beyond_last_line: Some(editor.scroll_beyond_last_line()),
            cursor_surrounding_lines: Some(DEFAULT_CURSOR_SURROUNDING_LINES),
        },
        ui: UiSettings {
            theme: Some(ui.theme().to_string()),
            color_scheme_light: Some(ui.light_scheme().to_string()),
            color_scheme_dark: Some(ui.dark_scheme().to_string()),
            font_family: Some(ui_family.to_string()),
            font_size: Some(ui_size),
            tab_icons: Some(ui.tab_icons()),
            icon_color: Some(ui.icon_color().to_string()),
            sidebar_position: Some(ui.sidebar_position().to_string()),
        },
    };
    serde_json::to_string_pretty(&defaults).expect("settings serialize as JSON")
}

/// Returns `data` with `section.key` set to `value` — or removed when `value`
/// is `None` — preserving every other key so a hand-authored file keeps the rest
/// of its content. Keys are written in the order `serde_json` sorts them, so
/// formatting is normalized.
fn set_setting(
    data: &str,
    section: &str,
    key: &str,
    value: Option<serde_json::Value>,
) -> Result<String, String> {
    let mut root: serde_json::Value =
        serde_json::from_str(data).map_err(|error| format!("Invalid settings: {error}"))?;
    let object = root
        .as_object_mut()
        .ok_or_else(|| "Settings must be a JSON object".to_string())?;

    match value {
        Some(value) => {
            let section = object
                .entry(section)
                .or_insert_with(|| serde_json::json!({}))
                .as_object_mut()
                .ok_or_else(|| format!("The `{section}` section must be a JSON object"))?;
            section.insert(key.to_string(), value);
        }
        None => {
            if let Some(section) = object
                .get_mut(section)
                .and_then(serde_json::Value::as_object_mut)
            {
                section.remove(key);
            }
        }
    }

    serde_json::to_string_pretty(&root).map_err(|error| error.to_string())
}

/// Reads `path`, applies one change with [`set_setting`], and writes it back
/// atomically, returning the exact file contents written. A missing file is
/// created; a malformed one is left untouched.
fn write_setting_at(
    path: &Path,
    section: &str,
    key: &str,
    value: Option<serde_json::Value>,
) -> Result<String, String> {
    let data = match std::fs::read_to_string(path) {
        Ok(data) => data,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => "{}".to_string(),
        Err(error) => return Err(format!("Could not read settings: {error}")),
    };
    let contents = set_setting(&data, section, key, value)? + "\n";

    if let Some(parent) = path.parent()
        && std::fs::create_dir_all(parent).is_err()
    {
        return Err("Could not create the settings directory".to_string());
    }

    let temp = path.with_file_name("settings.json.tmp");
    std::fs::write(&temp, &contents).map_err(|error| error.to_string())?;
    std::fs::rename(&temp, path).map_err(|error| error.to_string())?;
    Ok(contents)
}

fn save_setting(
    section: &str,
    key: &str,
    value: Option<serde_json::Value>,
) -> Result<String, String> {
    let Some(path) = settings_path() else {
        return Err("Could not locate the support directory".to_string());
    };
    write_setting_at(&path, section, key, value)
}

/// Records the theme choice (`ui.theme`).
pub(crate) fn save_theme(theme: &str) -> Result<String, String> {
    save_setting(
        "ui",
        "theme",
        Some(serde_json::Value::String(theme.to_string())),
    )
}

/// Records the light-appearance color scheme (`ui.color_scheme_light`).
pub(crate) fn save_color_scheme_light(name: &str) -> Result<String, String> {
    save_setting(
        "ui",
        "color_scheme_light",
        Some(serde_json::Value::String(name.to_string())),
    )
}

/// Records the dark-appearance color scheme (`ui.color_scheme_dark`).
pub(crate) fn save_color_scheme_dark(name: &str) -> Result<String, String> {
    save_setting(
        "ui",
        "color_scheme_dark",
        Some(serde_json::Value::String(name.to_string())),
    )
}

/// Records the word-wrap toggle (`editor.soft_wrap`).
pub(crate) fn save_soft_wrap(enabled: bool) -> Result<String, String> {
    save_setting(
        "editor",
        "soft_wrap",
        Some(serde_json::Value::Bool(enabled)),
    )
}

/// Records the whitespace toggle (`editor.show_whitespace`).
pub(crate) fn save_show_whitespace(enabled: bool) -> Result<String, String> {
    save_setting(
        "editor",
        "show_whitespace",
        Some(serde_json::Value::Bool(enabled)),
    )
}

/// Records an on/off editor option (`editor.<key>`).
pub(crate) fn save_editor_flag(key: &str, enabled: bool) -> Result<String, String> {
    save_setting("editor", key, Some(serde_json::Value::Bool(enabled)))
}

/// Records the zoom level (`editor.font_size`); `None` removes the override so
/// the theme's default size applies again.
pub(crate) fn save_font_size(size: Option<f32>) -> Result<String, String> {
    save_setting(
        "editor",
        "font_size",
        size.map(|size| serde_json::json!(size)),
    )
}

/// Records the editor font family (`editor.font_family`).
pub(crate) fn save_font_family(family: &str) -> Result<String, String> {
    save_setting(
        "editor",
        "font_family",
        Some(serde_json::Value::String(family.to_string())),
    )
}

/// Records the editor font weight (`editor.font_weight`).
pub(crate) fn save_font_weight(weight: f32) -> Result<String, String> {
    save_setting("editor", "font_weight", Some(serde_json::json!(weight)))
}

/// Records the indentation width (`editor.tab_size`).
pub(crate) fn save_tab_size(size: usize) -> Result<String, String> {
    save_setting("editor", "tab_size", Some(serde_json::json!(size)))
}

/// Records whether indentation uses hard tabs (`editor.hard_tabs`).
pub(crate) fn save_hard_tabs(enabled: bool) -> Result<String, String> {
    save_setting(
        "editor",
        "hard_tabs",
        Some(serde_json::Value::Bool(enabled)),
    )
}

/// Records the cursor scroll margin (`editor.cursor_surrounding_lines`); `None`
/// removes the override so the engine's own margin applies again.
pub(crate) fn save_cursor_surrounding_lines(lines: Option<usize>) -> Result<String, String> {
    save_setting(
        "editor",
        "cursor_surrounding_lines",
        lines.map(|lines| serde_json::json!(lines)),
    )
}

/// Records the interface font family (`ui.font_family`); `None` removes the
/// override so the system UI font applies again.
pub(crate) fn save_ui_font_family(family: Option<&str>) -> Result<String, String> {
    save_setting(
        "ui",
        "font_family",
        family.map(|family| serde_json::Value::String(family.to_string())),
    )
}

/// Records the interface font size (`ui.font_size`); `None` removes the override
/// so the theme's base size applies again.
pub(crate) fn save_ui_font_size(size: Option<f32>) -> Result<String, String> {
    save_setting("ui", "font_size", size.map(|size| serde_json::json!(size)))
}

/// Records the sidebar tab-icon preference (`ui.tab_icons`).
pub(crate) fn save_tab_icons(enabled: bool) -> Result<String, String> {
    save_setting("ui", "tab_icons", Some(serde_json::Value::Bool(enabled)))
}

/// Records the file-icon style (`ui.icon_color`).
pub(crate) fn save_icon_color(style: &str) -> Result<String, String> {
    save_setting(
        "ui",
        "icon_color",
        Some(serde_json::Value::String(style.to_string())),
    )
}

/// Records which side the file tree sits on (`ui.sidebar_position`).
pub(crate) fn save_sidebar_position(value: &str) -> Result<String, String> {
    save_setting(
        "ui",
        "sidebar_position",
        Some(serde_json::Value::String(value.to_string())),
    )
}

/// Explains why the editor font `family` will not render, with installed names
/// to use instead.
pub(crate) fn font_family_issue(family: &str, installed: &[String]) -> Option<String> {
    font_issue(family, installed, MONOSPACE_FONTS, "monospace")
}

/// Like [`font_family_issue`], for the interface font, suggesting sans-serif
/// families instead of monospace ones.
pub(crate) fn ui_font_family_issue(family: &str, installed: &[String]) -> Option<String> {
    font_issue(family, installed, UI_FONTS, "sans-serif")
}

fn font_issue(
    family: &str,
    installed: &[String],
    suggestions: &[&str],
    label: &str,
) -> Option<String> {
    if installed.iter().any(|name| name == family) {
        return None;
    }

    let wanted = family.to_lowercase();
    let close: Vec<&str> = installed
        .iter()
        .filter(|name| {
            let name = name.to_lowercase();
            name.contains(&wanted) || wanted.contains(&name)
        })
        .take(MAX_SUGGESTIONS)
        .map(String::as_str)
        .collect();
    if !close.is_empty() {
        return Some(format!(
            "Font \"{family}\" is not installed. Did you mean {}?",
            quoted(&close)
        ));
    }

    let available: Vec<&str> = suggestions
        .iter()
        .copied()
        .filter(|font| installed.iter().any(|name| name == font))
        .collect();
    if available.is_empty() {
        return Some(format!("Font \"{family}\" is not installed."));
    }
    Some(format!(
        "Font \"{family}\" is not installed. Installed {label} fonts: {}.",
        quoted(&available)
    ))
}

fn quoted(names: &[&str]) -> String {
    names
        .iter()
        .map(|name| format!("\"{name}\""))
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_editor_section() {
        let settings = parse_settings(
            r#"{"editor": {"font_family": "Menlo", "font_size": 15, "font_weight": 600}}"#,
        )
        .unwrap();

        assert_eq!(settings.editor.font_family.as_deref(), Some("Menlo"));
        assert_eq!(settings.editor.font_size(), Some(15.));
        assert_eq!(settings.editor.font_weight(), Some(600.));
    }

    #[test]
    fn defaults_missing_sections() {
        assert_eq!(parse_settings("{}").unwrap(), Settings::default());
        assert_eq!(
            parse_settings(r#"{"editor": {}}"#).unwrap(),
            Settings::default()
        );
    }

    #[test]
    fn parses_editor_options() {
        let settings = parse_settings(
            r#"{"editor": {"soft_wrap": true, "show_whitespace": true, "tab_size": 8, "hard_tabs": true}}"#,
        )
        .unwrap();

        assert!(settings.editor.soft_wrap());
        assert!(settings.editor.show_whitespace());
        assert_eq!(settings.editor.tab_size(), 8);
        assert!(settings.editor.hard_tabs());
    }

    #[test]
    fn editor_options_default() {
        let settings = Settings::default();

        assert!(!settings.editor.soft_wrap());
        assert!(!settings.editor.show_whitespace());
        assert_eq!(settings.editor.tab_size(), DEFAULT_TAB_SIZE);
        assert!(!settings.editor.hard_tabs());
        assert!(settings.editor.line_numbers());
        assert!(settings.editor.indent_guides());
        assert!(settings.editor.folding());
        assert!(settings.editor.auto_close());
        assert!(settings.editor.smart_indent());
        assert!(settings.editor.indent_wrapped_lines());
        assert!(settings.editor.scroll_beyond_last_line());
        assert_eq!(settings.editor.cursor_surrounding_lines, None);
    }

    #[test]
    fn cursor_surrounding_lines_round_trips_through_set_setting() {
        let json = set_setting(
            r#"{"editor": {"font_size": 15}}"#,
            "editor",
            "cursor_surrounding_lines",
            Some(serde_json::json!(6)),
        )
        .unwrap();
        let settings = parse_settings(&json).unwrap();

        // Re-parsing proves the key matches the field: an unknown key would be
        // rejected rather than silently ignored.
        assert_eq!(settings.editor.cursor_surrounding_lines, Some(6));
        assert_eq!(settings.editor.font_size(), Some(15.));

        // Clearing it drops the override so the engine's own margin applies.
        let cleared = set_setting(&json, "editor", "cursor_surrounding_lines", None).unwrap();
        let settings = parse_settings(&cleared).unwrap();
        assert_eq!(settings.editor.cursor_surrounding_lines, None);
        assert_eq!(settings.editor.font_size(), Some(15.));
    }

    #[test]
    fn defaults_document_lists_every_key_at_its_effective_default() {
        let document = defaults_document("Menlo", 13., "SF Pro Text", 16.);
        let parsed = parse_settings(&document).unwrap();
        let editor = EditorSettings::default();
        let ui = UiSettings::default();

        assert_eq!(parsed.editor.font_family.as_deref(), Some("Menlo"));
        assert_eq!(parsed.editor.font_size(), Some(13.));
        assert_eq!(parsed.editor.font_weight(), Some(DEFAULT_FONT_WEIGHT));
        assert_eq!(parsed.editor.soft_wrap(), editor.soft_wrap());
        assert_eq!(parsed.editor.show_whitespace(), editor.show_whitespace());
        assert_eq!(parsed.editor.tab_size(), editor.tab_size());
        assert_eq!(parsed.editor.hard_tabs(), editor.hard_tabs());
        assert_eq!(parsed.editor.line_numbers(), editor.line_numbers());
        assert_eq!(parsed.editor.indent_guides(), editor.indent_guides());
        assert_eq!(parsed.editor.folding(), editor.folding());
        assert_eq!(parsed.editor.auto_close(), editor.auto_close());
        assert_eq!(parsed.editor.smart_indent(), editor.smart_indent());
        assert_eq!(
            parsed.editor.indent_wrapped_lines(),
            editor.indent_wrapped_lines()
        );
        assert_eq!(
            parsed.editor.scroll_beyond_last_line(),
            editor.scroll_beyond_last_line()
        );
        assert_eq!(
            parsed.editor.cursor_surrounding_lines,
            Some(DEFAULT_CURSOR_SURROUNDING_LINES)
        );
        assert_eq!(parsed.ui.theme(), ui.theme());
        assert_eq!(parsed.ui.light_scheme(), ui.light_scheme());
        assert_eq!(parsed.ui.dark_scheme(), ui.dark_scheme());
        assert_eq!(parsed.ui.font_family.as_deref(), Some("SF Pro Text"));
        assert_eq!(parsed.ui.font_size(), Some(16.));
        assert_eq!(parsed.ui.tab_icons(), ui.tab_icons());
        assert_eq!(parsed.ui.icon_color(), ui.icon_color());
        assert_eq!(parsed.ui.sidebar_position(), ui.sidebar_position());
    }

    #[test]
    fn parses_editor_display_options() {
        let settings = parse_settings(
            r#"{"editor": {"line_numbers": false, "indent_guides": false, "folding": false,
                "auto_close": false, "smart_indent": false, "indent_wrapped_lines": false,
                "scroll_beyond_last_line": false, "cursor_surrounding_lines": 5}}"#,
        )
        .unwrap();

        assert!(!settings.editor.line_numbers());
        assert!(!settings.editor.indent_guides());
        assert!(!settings.editor.folding());
        assert!(!settings.editor.auto_close());
        assert!(!settings.editor.smart_indent());
        assert!(!settings.editor.indent_wrapped_lines());
        assert!(!settings.editor.scroll_beyond_last_line());
        assert_eq!(settings.editor.cursor_surrounding_lines, Some(5));
    }

    #[test]
    fn ui_theme_defaults_and_parses() {
        assert_eq!(Settings::default().ui.theme(), DEFAULT_THEME);

        let settings = parse_settings(r#"{"ui": {"theme": "light"}}"#).unwrap();
        assert_eq!(settings.ui.theme(), "light");
    }

    #[test]
    fn ui_color_schemes_default_and_parse() {
        let settings = Settings::default();
        assert_eq!(settings.ui.light_scheme(), DEFAULT_LIGHT_SCHEME);
        assert_eq!(settings.ui.dark_scheme(), DEFAULT_DARK_SCHEME);

        let settings = parse_settings(
            r#"{"ui": {"color_scheme_light": "Nord Light", "color_scheme_dark": "Nord"}}"#,
        )
        .unwrap();
        assert_eq!(settings.ui.light_scheme(), "Nord Light");
        assert_eq!(settings.ui.dark_scheme(), "Nord");
    }

    #[test]
    fn color_scheme_round_trips_through_set_setting() {
        let json = set_setting(
            r#"{"ui": {"theme": "system"}}"#,
            "ui",
            "color_scheme_dark",
            text("Dracula"),
        )
        .unwrap();
        let settings = parse_settings(&json).unwrap();

        assert_eq!(settings.ui.dark_scheme(), "Dracula");
        assert_eq!(settings.ui.theme(), "system");
    }

    #[test]
    fn ui_tab_icons_defaults_and_parses() {
        assert!(Settings::default().ui.tab_icons());

        let settings = parse_settings(r#"{"ui": {"tab_icons": false}}"#).unwrap();
        assert!(!settings.ui.tab_icons());
    }

    #[test]
    fn ui_font_defaults_and_parses() {
        let settings = Settings::default();
        assert_eq!(settings.ui.font_family, None);
        assert_eq!(settings.ui.font_size(), None);

        let settings =
            parse_settings(r#"{"ui": {"font_family": "Inter", "font_size": 18}}"#).unwrap();
        assert_eq!(settings.ui.font_family.as_deref(), Some("Inter"));
        assert_eq!(settings.ui.font_size(), Some(18.));
    }

    #[test]
    fn clamps_ui_font_size() {
        let settings = parse_settings(r#"{"ui": {"font_size": 500}}"#).unwrap();
        assert_eq!(settings.ui.font_size(), Some(MAX_UI_FONT_SIZE));

        let settings = parse_settings(r#"{"ui": {"font_size": 1}}"#).unwrap();
        assert_eq!(settings.ui.font_size(), Some(MIN_UI_FONT_SIZE));
    }

    #[test]
    fn ui_icon_style_defaults_and_parses() {
        assert_eq!(Settings::default().ui.icon_color(), DEFAULT_ICON_STYLE);
        assert_eq!(Settings::default().ui.icon_style(), IconStyle::Monochrome);

        let settings = parse_settings(r#"{"ui": {"icon_color": "monochrome"}}"#).unwrap();
        assert_eq!(settings.ui.icon_style(), IconStyle::Monochrome);

        let settings = parse_settings(r#"{"ui": {"icon_color": "colorful"}}"#).unwrap();
        assert_eq!(settings.ui.icon_style(), IconStyle::Colorful);
    }

    #[test]
    fn ui_icon_style_falls_back_for_unknown_values() {
        let settings = parse_settings(r#"{"ui": {"icon_color": "neon"}}"#).unwrap();
        assert_eq!(settings.ui.icon_style(), IconStyle::Colorful);
    }

    #[test]
    fn ui_sidebar_position_defaults_and_parses() {
        let settings = Settings::default();
        assert_eq!(settings.ui.sidebar_position(), DEFAULT_SIDEBAR_POSITION);
        assert!(!settings.ui.sidebar_right());

        let settings = parse_settings(r#"{"ui": {"sidebar_position": "right"}}"#).unwrap();
        assert_eq!(settings.ui.sidebar_position(), SIDEBAR_POSITION_RIGHT);
        assert!(settings.ui.sidebar_right());

        let settings = parse_settings(r#"{"ui": {"sidebar_position": "unknown"}}"#).unwrap();
        assert!(!settings.ui.sidebar_right());
    }

    fn text(value: &str) -> Option<serde_json::Value> {
        Some(serde_json::Value::String(value.to_string()))
    }

    #[test]
    fn set_setting_updates_a_key_and_keeps_the_rest() {
        let updated = set_setting(
            r#"{"editor": {"font_size": 15}}"#,
            "editor",
            "soft_wrap",
            Some(serde_json::Value::Bool(true)),
        )
        .unwrap();
        let value: serde_json::Value = serde_json::from_str(&updated).unwrap();

        assert_eq!(value["editor"]["soft_wrap"], true);
        assert_eq!(value["editor"]["font_size"], 15);
    }

    #[test]
    fn set_setting_adds_a_missing_section() {
        let updated = set_setting("{}", "ui", "theme", text("light")).unwrap();
        let value: serde_json::Value = serde_json::from_str(&updated).unwrap();

        assert_eq!(value["ui"]["theme"], "light");
    }

    #[test]
    fn set_setting_removes_a_key_when_value_is_none() {
        let updated = set_setting(
            r#"{"editor": {"font_size": 15, "soft_wrap": true}}"#,
            "editor",
            "font_size",
            None,
        )
        .unwrap();
        let value: serde_json::Value = serde_json::from_str(&updated).unwrap();

        assert!(value["editor"].get("font_size").is_none());
        assert_eq!(value["editor"]["soft_wrap"], true);
    }

    #[test]
    fn set_setting_rejects_malformed_json() {
        assert!(set_setting("{", "ui", "theme", text("light")).is_err());
    }

    #[test]
    fn saved_settings_round_trip_through_load() {
        let json = set_setting(
            r#"{"editor": {"font_size": 15}}"#,
            "ui",
            "theme",
            text("light"),
        )
        .unwrap();
        let settings = parse_settings(&json).unwrap();

        assert_eq!(settings.ui.theme(), "light");
        assert_eq!(settings.editor.font_size(), Some(15.));
    }

    #[test]
    fn write_setting_creates_then_updates_the_file() {
        let dir = std::env::temp_dir().join(format!("tinytext-settings-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("settings.json");

        // A missing file is created with just the key, and the returned
        // contents match what landed on disk.
        let written = write_setting_at(&path, "ui", "theme", text("light")).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), written);
        let settings = parse_settings(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(settings.ui.theme(), "light");

        // An existing file keeps its other keys.
        std::fs::write(&path, r#"{"editor": {"font_size": 15}}"#).unwrap();
        write_setting_at(&path, "ui", "theme", text("dark")).unwrap();
        let settings = parse_settings(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(settings.ui.theme(), "dark");
        assert_eq!(settings.editor.font_size(), Some(15.));

        // A cleared value removes the key again.
        write_setting_at(&path, "editor", "font_size", None).unwrap();
        let settings = parse_settings(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(settings.editor.font_size(), None);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn rejects_unknown_keys() {
        let error = parse_settings(r#"{"editor": {"font_sise": 14}}"#).unwrap_err();
        assert!(error.to_string().contains("unknown field `font_sise`"));

        let error = parse_settings(r#"{"edtor": {}}"#).unwrap_err();
        assert!(error.to_string().contains("unknown field `edtor`"));
    }

    #[test]
    fn rejects_wrong_types() {
        assert!(parse_settings(r#"{"editor": {"font_size": "14"}}"#).is_err());
    }

    #[test]
    fn clamps_font_size() {
        let settings = parse_settings(r#"{"editor": {"font_size": 500}}"#).unwrap();
        assert_eq!(settings.editor.font_size(), Some(MAX_FONT_SIZE));

        let settings = parse_settings(r#"{"editor": {"font_size": 0}}"#).unwrap();
        assert_eq!(settings.editor.font_size(), Some(MIN_FONT_SIZE));
    }

    #[test]
    fn clamps_font_weight() {
        let settings = parse_settings(r#"{"editor": {"font_weight": 1000}}"#).unwrap();
        assert_eq!(settings.editor.font_weight(), Some(MAX_FONT_WEIGHT));

        let settings = parse_settings(r#"{"editor": {"font_weight": 50}}"#).unwrap();
        assert_eq!(settings.editor.font_weight(), Some(MIN_FONT_WEIGHT));
    }

    fn installed(names: &[&str]) -> Vec<String> {
        names.iter().map(|name| name.to_string()).collect()
    }

    #[test]
    fn accepts_installed_font() {
        assert_eq!(font_family_issue("Menlo", &installed(&["Menlo"])), None);
    }

    #[test]
    fn suggests_close_font_names() {
        let fonts = installed(&["JetBrains Mono", "JetBrains Mono NL", "Menlo"]);

        assert_eq!(
            font_family_issue("jetbrains mono", &fonts).unwrap(),
            r#"Font "jetbrains mono" is not installed. Did you mean "JetBrains Mono", "JetBrains Mono NL"?"#
        );
        assert_eq!(
            font_family_issue("JetBrains", &fonts).unwrap(),
            r#"Font "JetBrains" is not installed. Did you mean "JetBrains Mono", "JetBrains Mono NL"?"#
        );
    }

    #[test]
    fn lists_installed_monospace_fonts_without_close_match() {
        let fonts = installed(&["Arial", "Menlo", "Monaco"]);

        assert_eq!(
            font_family_issue("Comic Code", &fonts).unwrap(),
            r#"Font "Comic Code" is not installed. Installed monospace fonts: "Menlo", "Monaco"."#
        );
        assert_eq!(
            font_family_issue("Comic Code", &installed(&["Arial"])).unwrap(),
            r#"Font "Comic Code" is not installed."#
        );
    }

    #[test]
    fn ui_font_issue_suggests_sans_serif_fonts() {
        assert_eq!(ui_font_family_issue("Inter", &installed(&["Inter"])), None);
        assert_eq!(
            ui_font_family_issue("Nonesuch", &installed(&["Helvetica Neue", "Menlo"])).unwrap(),
            r#"Font "Nonesuch" is not installed. Installed sans-serif fonts: "Helvetica Neue"."#
        );
    }

    #[test]
    fn rejects_malformed_json() {
        let missing_comma =
            "{\n  \"editor\": {\n    \"font_family\": \"Menlo\"\n    \"font_size\": 14\n  }\n}";
        let error = parse_settings(missing_comma).unwrap_err();
        assert_eq!(error.line(), 4);
    }
}
