//! Registers the app's key bindings, and reads the user's overrides.
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
//! the order they are registered in and applies `keybindings.json`.
//!
//! The file uses VS Code's shape, one object per command:
//!
//! ```json
//! [
//!   // Override one command's key.
//!   { "key": "cmd+shift+p", "command": "Command Palette…" },
//!   // Remove every key from another.
//!   { "key": "", "command": "-Clear Recent" }
//! ]
//! ```
//!
//! `command` is the label the palette shows or the action name the table
//! builds (for example `ToggleSidebar`). Chords are single keystrokes written
//! with `+`; VS Code's space-separated sequences are not supported, and an
//! entry with one is ignored. Changes apply at the next start.

use std::path::PathBuf;

use gpui_kit::*;

use crate::commands;
use crate::paths::support_dir;

/// The user's binding file, beside `settings.json`.
pub(crate) fn path() -> Option<PathBuf> {
    Some(support_dir()?.join("keybindings.json"))
}

/// Seed file written the first time Open Keybindings… runs.
pub(crate) const STARTER: &str = "\
// Teak keybindings
//
// Each entry overrides one command's key. `command` is the label the palette
// shows, or the command's action name. Prefix it with `-` to remove every key
// for that command. Write one chord with `+`: cmd+shift+p, ctrl+alt+up.
// `cmd` is Command on macOS. Changes apply the next time Teak starts.
[
  // { \"key\": \"cmd+shift+o\", \"command\": \"Open…\" },
  // { \"key\": \"ctrl+alt+t\", \"command\": \"Toggle Sidebar\" }
]
";

/// Registers every binding with no user overrides. Used by the tests, whose
/// result must not depend on the machine's `keybindings.json`.
#[cfg(test)]
pub(crate) fn bind_keys(cx: &mut App) {
    bind_keys_with(cx, &Overrides::default());
}

/// Registers every binding, replacing or removing the commands `overrides`
/// names.
pub(crate) fn bind_keys_with(cx: &mut App, overrides: &Overrides) {
    cx.bind_keys(commands::app_bindings_with(cx, overrides));
    cx.bind_keys(commands::editor_bindings_with(cx, overrides));
    cx.bind_keys(commands::find_bindings_with(cx, overrides));
}

/// What an entry does to its command.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Change<'a> {
    /// The command keeps no key.
    Removed,
    /// The command's key becomes this chord.
    Key(&'a str),
}

/// One parsed `keybindings.json` entry.
#[derive(Clone, Debug)]
struct Entry {
    /// The normalized chord, or empty for a removal.
    key: String,
    /// The command's label or action name.
    command: String,
    /// True when the command was written with a leading `-`.
    removal: bool,
}

/// The user's overrides, in file order. Later entries win.
#[derive(Debug, Default)]
pub(crate) struct Overrides {
    entries: Vec<Entry>,
}

impl Overrides {
    /// Reads `keybindings.json` beside the settings, or returns no overrides
    /// when the file is absent or unreadable. A malformed file is reported on
    /// stderr rather than blocking startup.
    pub(crate) fn load() -> Self {
        let Some(path) = path() else {
            return Self::default();
        };
        let Ok(text) = std::fs::read_to_string(&path) else {
            return Self::default();
        };
        match Self::parse(&text) {
            Ok(overrides) => overrides,
            Err(error) => {
                eprintln!("Teak: ignoring {}: {error}", path.display());
                Self::default()
            }
        }
    }

    /// Parses the file. Comments and trailing commas are accepted, so the file
    /// can be commented and edited by hand.
    pub(crate) fn parse(text: &str) -> Result<Self, String> {
        #[derive(serde::Deserialize)]
        struct Raw {
            #[serde(default)]
            key: String,
            command: String,
        }

        let raw: Vec<Raw> =
            serde_json::from_str(&strip_jsonc(text)).map_err(|error| error.to_string())?;
        let entries = raw
            .into_iter()
            .filter_map(|raw| {
                let command = raw.command.trim().to_string();
                let (removal, command) = match command.strip_prefix('-') {
                    Some(name) => (true, name.trim().to_string()),
                    None => (false, command),
                };
                if command.is_empty() {
                    return None;
                }
                Some(Entry {
                    key: normalize_chord(&raw.key).unwrap_or_default(),
                    command,
                    removal,
                })
            })
            .collect();
        Ok(Self { entries })
    }

    /// The change that applies to the command with `name` (its action name) or
    /// `label`, if the file names it. A later entry wins.
    pub(crate) fn change(&self, name: &str, label: &str) -> Option<Change<'_>> {
        let entry = self.entries.iter().rev().find(|entry| {
            names_match(&entry.command, name) || labels_match(&entry.command, label)
        })?;
        if entry.removal {
            Some(Change::Removed)
        } else if entry.key.is_empty() {
            // An unusable chord (a sequence, say) leaves the default alone.
            None
        } else {
            Some(Change::Key(&entry.key))
        }
    }
}

/// Compares a written command with an action's name, accepting the bare name
/// without the `teak::` namespace the action macro adds.
fn names_match(written: &str, name: &str) -> bool {
    written.eq_ignore_ascii_case(name)
        || name
            .rsplit("::")
            .next()
            .is_some_and(|bare| written.eq_ignore_ascii_case(bare))
}

/// Compares a written command with a label, ignoring case and the trailing
/// ellipsis the palette shows.
fn labels_match(written: &str, label: &str) -> bool {
    let trim = |text: &str| {
        text.trim()
            .trim_end_matches('…')
            .trim_end()
            .to_ascii_lowercase()
    };
    trim(written) == trim(label)
}

/// Turns a written chord into gpui's form: `cmd+shift+p` becomes
/// `secondary-shift-p`. `None` for an empty chord or one with a space, which
/// would be a multi-keystroke sequence this app does not support.
pub(crate) fn normalize_chord(input: &str) -> Option<String> {
    let trimmed = input.trim();
    if trimmed.is_empty() || trimmed.contains(char::is_whitespace) {
        return None;
    }

    let mut modifiers: Vec<&str> = Vec::new();
    let mut key = None;
    for part in trimmed
        .split('+')
        .map(str::trim)
        .filter(|part| !part.is_empty())
    {
        let lower = part.to_ascii_lowercase();
        let modifier = match lower.as_str() {
            "cmd" | "command" | "super" | "win" | "meta" => Some("secondary"),
            "ctrl" | "control" => Some("ctrl"),
            "alt" | "option" | "opt" => Some("alt"),
            "shift" => Some("shift"),
            "fn" => Some("fn"),
            _ => None,
        };
        match modifier {
            Some(modifier) => modifiers.push(modifier),
            // A second non-modifier part is a malformed chord, not a key.
            None if key.is_some() => return None,
            None => key = Some(lower),
        }
    }

    let key = key?;
    // A chord already in gpui's form (`secondary-shift-p`) has one part; keeping
    // it whole is what makes both spellings work.
    modifiers.push(&key);
    Some(modifiers.join("-"))
}

/// Removes `//` and `/* */` comments and trailing commas, leaving strings
/// alone. Written as a small scanner so the file needs no JSONC dependency.
fn strip_jsonc(input: &str) -> String {
    let chars: Vec<char> = input.chars().collect();
    let mut out = String::with_capacity(input.len());
    let mut i = 0;
    while i < chars.len() {
        let ch = chars[i];
        if ch == '"' {
            out.push(ch);
            i += 1;
            let mut escaped = false;
            while i < chars.len() {
                let inside = chars[i];
                out.push(inside);
                i += 1;
                if escaped {
                    escaped = false;
                } else if inside == '\\' {
                    escaped = true;
                } else if inside == '"' {
                    break;
                }
            }
            continue;
        }
        if ch == '/' && chars.get(i + 1) == Some(&'/') {
            while i < chars.len() && chars[i] != '\n' {
                i += 1;
            }
            continue;
        }
        if ch == '/' && chars.get(i + 1) == Some(&'*') {
            i += 2;
            while i + 1 < chars.len() && !(chars[i] == '*' && chars[i + 1] == '/') {
                i += 1;
            }
            i = (i + 2).min(chars.len());
            continue;
        }
        out.push(ch);
        i += 1;
    }
    drop_trailing_commas(&out)
}

/// Drops a comma that only precedes a closing bracket or brace.
fn drop_trailing_commas(input: &str) -> String {
    let chars: Vec<char> = input.chars().collect();
    let mut out = String::with_capacity(input.len());
    for (i, ch) in chars.iter().enumerate() {
        if *ch == ',' {
            let mut next = i + 1;
            while next < chars.len() && chars[next].is_whitespace() {
                next += 1;
            }
            if matches!(chars.get(next), Some(']' | '}')) {
                continue;
            }
        }
        out.push(*ch);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{Change, Overrides, normalize_chord, strip_jsonc};

    #[test]
    fn chords_from_both_spellings_normalize_to_gpui_form() {
        assert_eq!(
            normalize_chord("cmd+shift+p").as_deref(),
            Some("secondary-shift-p")
        );
        assert_eq!(
            normalize_chord("Ctrl+Alt+Up").as_deref(),
            Some("ctrl-alt-up")
        );
        assert_eq!(
            normalize_chord("secondary-shift-p").as_deref(),
            Some("secondary-shift-p"),
        );
        assert_eq!(
            normalize_chord("cmd+k cmd+s"),
            None,
            "sequences are unsupported"
        );
        assert_eq!(normalize_chord("  "), None);
    }

    #[test]
    fn comments_and_trailing_commas_do_not_break_parsing() {
        let text = r#"
        // a comment
        [
          { "key": "cmd+shift+p", "command": "Command Palette…" }, // trailing
          /* block */
        ]
        "#;
        let overrides = Overrides::parse(text).unwrap();
        assert_eq!(
            overrides.change("CommandPalette", "Command Palette…"),
            Some(Change::Key("secondary-shift-p")),
        );
    }

    #[test]
    fn a_leading_minus_removes_the_command() {
        let text = r#"[{ "key": "", "command": "-Clear Recent" }]"#;
        let overrides = Overrides::parse(text).unwrap();
        assert_eq!(
            overrides.change("ClearRecent", "Clear Recent"),
            Some(Change::Removed),
        );
    }

    #[test]
    fn commands_match_by_action_name_or_label_without_the_ellipsis() {
        let overrides = Overrides::parse(
            r#"[{ "key": "cmd+o", "command": "open" }, { "key": "cmd+s", "command": "SaveFile" }]"#,
        )
        .unwrap();
        assert_eq!(
            overrides.change("OpenFile", "Open…"),
            Some(Change::Key("secondary-o"))
        );
        assert_eq!(
            overrides.change("SaveFile", "Save"),
            Some(Change::Key("secondary-s")),
        );
        assert_eq!(overrides.change("NewFile", "New File"), None);
    }

    #[test]
    fn the_scanner_leaves_string_bodies_alone() {
        assert_eq!(strip_jsonc(r#"["a//b", 1,]"#), r#"["a//b", 1]"#);
    }
}
