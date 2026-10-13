# Color Schemes

Tinytext separates **appearance** from **palette**:

- **Mode** is the appearance — Light, Dark, or System (follow macOS). It is the
  primary switch and lives in `ui.theme`.
- **Color scheme** is the palette applied *within* an appearance. Each
  appearance has its own scheme, so you can use a warm light theme by day and a
  high-contrast dark theme at night.

A color scheme controls **colors only**. Editor font family, size, and weight,
the tab size, and the UI corner radius are all owned by Settings and are never
taken from a theme — an installed theme cannot change your editor's typography.

## Choosing a scheme

Open **Settings → Appearance**:

- **Mode** — Light, Dark, or System.
- **Color Scheme → Light Theme** — the palette used while the appearance is
  light.
- **Color Scheme → Dark Theme** — the palette used while the appearance is
  dark.

The mode can also be switched from the **View → Appearance** menu. When the mode
is System, Tinytext follows the macOS appearance and swaps between your light
and dark schemes automatically.

Out of the box both slots use the built-in **Default Light** and **Default
Dark** palettes.

## Settings keys

```jsonc
{
  "ui": {
    "theme": "system",                    // "light" | "dark" | "system"
    "color_scheme_light": "Default Light", // scheme name for the light appearance
    "color_scheme_dark": "Default Dark"    // scheme name for the dark appearance
  }
}
```

## Where themes live

Tinytext ships the two default palettes. Themes you install go in:

```
~/Library/Application Support/net.nilambar.tinytext/themes/
```

Drop one JSON file per theme (or per set of themes) into that folder. Tinytext
watches the folder and reloads it automatically, so a newly added theme appears
in the Light/Dark Theme dropdowns as soon as the file is saved. Use
**Tinytext → Open Themes Folder** to reveal the folder in Finder. A fresh install
seeds a short `README.md` there.

A file may hold a whole set of themes; each entry's `mode` decides whether it
shows up under **Light Theme** or **Dark Theme**.

## Theme file format

The format is the one Zed uses, so many existing themes work with little or no
change. A file is a theme *set*:

```jsonc
{
  "name": "My Themes",        // the set's name (informational)
  "author": "You",            // optional
  "url": "https://…",         // optional
  "themes": [
    {
      "name": "My Dark",      // shown in the dropdown — must be unique
      "mode": "dark",         // "light" | "dark"
      "colors": { /* UI colors */ },
      "highlight": { /* editor + syntax colors */ }
    }
  ]
}
```

Both `colors` and `highlight` are optional; anything you leave out falls back to
the built-in palette for the theme's `mode`.

### `colors` — application and editor chrome

Keys name a component and the slot they paint. A useful subset:

| Key | Paints |
| --- | --- |
| `background`, `foreground` | Window/editor background and default text |
| `border` | Default borders |
| `input.border` | Input and field borders |
| `caret` | Text caret |
| `primary.background`, `primary.foreground` | Accent buttons and highlights |
| `secondary.background`, `secondary.foreground` | Secondary surfaces |
| `muted.background`, `muted.foreground` | Muted surfaces and dim text |
| `accent.background`, `accent.foreground` | Hover/menu accents |
| `danger`, `success`, `warning`, `info` | Status colors (each has `.background`) |
| `tab.background`, `tab.active.background`, `tab.foreground` | Tabs |
| `tab_bar.background` | The tab strip |
| `title_bar.background`, `status_bar.background` | Window chrome |
| `sidebar.background`, `sidebar.foreground`, `sidebar.border` | Explorer sidebar |
| `scrollbar.thumb.background` | Scrollbar thumb |
| `selection.background` | Text selection |

Colors accept `#rgb`, `#rrggbb`, `#rrggbbaa`, and named tokens such as
`neutral-100` or `red-500`.

### `highlight` — editor and syntax

```jsonc
"highlight": {
  "editor.background": "#0f1117",
  "editor.foreground": "#c8ccd4",
  "editor.line_number": "#4a5163",
  "editor.active_line_number": "#c8ccd4",
  "editor.active_line.background": "#161b24",
  "editor.invisible": "#4a516366",   // whitespace marks
  "syntax": {
    "keyword":  { "color": "#bb9af7" },
    "function": { "color": "#7aa2f7" },
    "string":   { "color": "#9ece6a" },
    "number":   { "color": "#ff9e64" },
    "comment":  { "color": "#5b6478", "font_style": "italic" },
    "title":    { "color": "#7aa2f7", "font_weight": 700 }
  }
}
```

Common `syntax` tokens include `keyword`, `function`, `string`, `string.escape`,
`number`, `constant`, `type`, `variable`, `property`, `operator`, `comment`,
`comment.doc`, `punctuation`, `tag`, `attribute`, and `title`. Each takes a
`color`, and optionally `font_style` (`"italic"` / `"normal"`) or `font_weight`
(`100`–`900`).

## Example: light theme

Save as `paper-light.json`:

```json
{
  "name": "Paper",
  "themes": [
    {
      "name": "Paper Light",
      "mode": "light",
      "colors": {
        "background": "#faf7f0",
        "foreground": "#3b3733",
        "border": "#e2dccf",
        "input.border": "#d8d0bf",
        "caret": "#8a5a2b",
        "primary.background": "#8a5a2b",
        "primary.foreground": "#faf7f0",
        "secondary.background": "#efe9dd",
        "secondary.foreground": "#3b3733",
        "muted.background": "#f0ebdf",
        "muted.foreground": "#8a8272",
        "accent.background": "#e7dfcf",
        "accent.foreground": "#3b3733",
        "danger.background": "#b4543f",
        "success.background": "#5f7d4f",
        "warning.background": "#c08a2e",
        "info.background": "#3f6f8a",
        "tab_bar.background": "#efe9dd",
        "tab.background": "#efe9dd",
        "tab.active.background": "#faf7f0",
        "tab.foreground": "#8a8272",
        "tab.active.foreground": "#3b3733",
        "title_bar.background": "#efe9dd",
        "status_bar.background": "#efe9dd",
        "status_bar.border": "#e2dccf",
        "sidebar.background": "#f3eede",
        "sidebar.foreground": "#3b3733",
        "sidebar.border": "#e2dccf",
        "scrollbar.thumb.background": "#c9c1ad",
        "selection.background": "#dfd6c2"
      },
      "highlight": {
        "editor.background": "#faf7f0",
        "editor.foreground": "#3b3733",
        "editor.line_number": "#b3aa98",
        "editor.active_line_number": "#3b3733",
        "editor.active_line.background": "#f1ece1",
        "editor.invisible": "#b3aa9866",
        "syntax": {
          "comment": { "color": "#9a917f", "font_style": "italic" },
          "keyword": { "color": "#8a5a2b" },
          "function": { "color": "#3f6f8a" },
          "string": { "color": "#5f7d4f" },
          "number": { "color": "#b4543f" },
          "type": { "color": "#7a5c8a" },
          "constant": { "color": "#b4543f" },
          "variable": { "color": "#3b3733" },
          "title": { "color": "#3f6f8a", "font_weight": 700 },
          "emphasis.strong": { "color": "#3b3733", "font_weight": 700 }
        }
      }
    }
  ]
}
```

## Example: dark theme

Save as `midnight.json`:

```json
{
  "name": "Midnight",
  "themes": [
    {
      "name": "Midnight",
      "mode": "dark",
      "colors": {
        "background": "#0f1117",
        "foreground": "#c8ccd4",
        "border": "#232834",
        "input.border": "#2b3140",
        "caret": "#7aa2f7",
        "primary.background": "#7aa2f7",
        "primary.foreground": "#0f1117",
        "secondary.background": "#1a1f2b",
        "secondary.foreground": "#c8ccd4",
        "muted.background": "#161b24",
        "muted.foreground": "#6b7280",
        "accent.background": "#1f2635",
        "accent.foreground": "#c8ccd4",
        "danger.background": "#f7768e",
        "success.background": "#9ece6a",
        "warning.background": "#e0af68",
        "info.background": "#7dcfff",
        "tab_bar.background": "#161b24",
        "tab.background": "#161b24",
        "tab.active.background": "#0f1117",
        "tab.foreground": "#6b7280",
        "tab.active.foreground": "#c8ccd4",
        "title_bar.background": "#161b24",
        "status_bar.background": "#161b24",
        "status_bar.border": "#232834",
        "sidebar.background": "#131721",
        "sidebar.foreground": "#c8ccd4",
        "sidebar.border": "#232834",
        "scrollbar.thumb.background": "#2b3140",
        "selection.background": "#33467c"
      },
      "highlight": {
        "editor.background": "#0f1117",
        "editor.foreground": "#c8ccd4",
        "editor.line_number": "#4a5163",
        "editor.active_line_number": "#c8ccd4",
        "editor.active_line.background": "#161b24",
        "editor.invisible": "#4a516366",
        "syntax": {
          "comment": { "color": "#5b6478", "font_style": "italic" },
          "keyword": { "color": "#bb9af7" },
          "function": { "color": "#7aa2f7" },
          "string": { "color": "#9ece6a" },
          "number": { "color": "#ff9e64" },
          "type": { "color": "#2ac3de" },
          "constant": { "color": "#ff9e64" },
          "variable": { "color": "#c8ccd4" },
          "title": { "color": "#7aa2f7", "font_weight": 700 },
          "emphasis.strong": { "color": "#c8ccd4", "font_weight": 700 }
        }
      }
    }
  ]
}
```

## Notes and limits

- **Names must be unique.** Tinytext keeps the first theme it loads for a given
  name and ignores later ones, so avoid reusing a built-in name such as
  `Default Dark`.
- **A theme belongs to one appearance.** A `"dark"` theme only appears under
  **Dark Theme**; a `"light"` theme only under **Light Theme**.
- **Bundled themes are read-only.** Only the folder above is user-writable.
- **Editing a theme file reloads it live**; the dropdowns and the active
  appearance update without restarting.
