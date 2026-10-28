# Keybindings

Teak's chords are declared in `commands.rs`, so every command has a default.
`keybindings.json` overrides them without touching the app. It sits next to
`settings.json` in Teak's support folder:

```
~/Library/Application Support/net.nilambar.teak/keybindings.json
```

Open it with **Teak → Open Keybindings…**, which writes a commented starter the
first time. The file is read at startup; restart Teak after editing it.

## Format

The file is an array of entries, the same shape VS Code uses:

```json
[
  // Press cmd+shift+p for the command palette.
  { "key": "cmd+shift+p", "command": "Command Palette…" },

  // Take every key away from New File.
  { "key": "", "command": "-New File" },

  // Bind a command that has no default key.
  { "key": "ctrl+alt+b", "command": "Toggle Sidebar" }
]
```

Comments (`//` and `/* */`) and trailing commas are allowed.

- `command` is the label shown in the command palette, or the action name from
  `commands.rs` (`ToggleSidebar`, `SaveFile`).
- A leading `-` removes every default chord for that command.
- A later entry wins over an earlier one for the same command.

## Chords

Write modifiers and one key with `+`:

| Written | Meaning |
| --- | --- |
| `cmd` | Command on macOS |
| `ctrl` | Control |
| `alt` | Option |
| `shift` | Shift |
| `fn` | Function |

Examples: `cmd+shift+p`, `ctrl+alt+up`, `cmd+/`. The gpui spelling
(`secondary-shift-p`) is accepted too.

Only single chords are supported. VS Code's space-separated sequences
(`cmd+k cmd+s`) do not exist in Teak yet, and an entry that uses one is
ignored.

## Notes

- An override keeps the command's scope: editor commands stay active only in
  the editor, global commands stay global.
- A command with no default key is bound globally.
- An unreadable or malformed file is reported when Teak starts, and the app
  runs with its defaults.
