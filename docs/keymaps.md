# Keyboard Shortcuts

## Files & tabs

| Shortcut | Keys | Action |
| --- | --- | --- |
| `⌘N` | `Cmd + N` | New File |
| `⌘⇧N` | `Cmd + Shift + N` | New Window |
| `⌘O` | `Cmd + O` | Open… |
| `⌘P` | `Cmd + P` | Quick Open… |
| `⌃R` | `Ctrl + R` | Open Recent… |
| `⌃⌘O` | `Ctrl + Cmd + O` | Add Folder… |
| `⌃⌘W` | `Ctrl + Cmd + W` | Close All Folders |
| `⌘S` | `Cmd + S` | Save |
| `⌘⇧S` | `Cmd + Shift + S` | Save As… |
| `⌥⌘S` | `Cmd + Option + S` | Save All |
| `⌘W` | `Cmd + W` | Close Tab |
| `⌘⇧W` | `Cmd + Shift + W` | Close Window |
| `⌘⇧T` | `Cmd + Shift + T` | Reopen Closed Tab |
| `⌘⇧]` / `⌘⇧[` | `Cmd + Shift + ]` / `Cmd + Shift + [` | Next / Previous Tab |
| `⌃⇥` / `⌃⇧⇥` | `Ctrl + Tab` / `Ctrl + Shift + Tab` | Next / Previous Tab |
| `⌘1` … `⌘9` | `Cmd + 1` … `Cmd + 9` | Jump to Tab |
| `⌥⌘R` | `Cmd + Option + R` | Reveal in Finder |
| `⌥⌘C` | `Cmd + Option + C` | Copy File Path |
| `⌥⌘⇧C` | `Cmd + Option + Shift + C` | Copy Relative Path |

## Editing

| Shortcut | Keys | Action |
| --- | --- | --- |
| `⌘/` | `Cmd + /` | Toggle Line Comment |
| `⌘⇧K` | `Cmd + Shift + K` | Delete Line |
| `⌘L` | `Cmd + L` | Select Line |
| `⌘J` | `Cmd + J` | Join Lines |
| `⌥↑` / `⌥↓` | `Option + Up` / `Option + Down` | Move Line Up / Down |
| `⇧⌥↑` / `⇧⌥↓` | `Shift + Option + Up` / `Shift + Option + Down` | Copy Line Up / Down |
| `⌘↩` | `Cmd + Enter` | Insert Line Below |
| `⌘⇧↩` | `Cmd + Shift + Enter` | Insert Line Above |
| `⌃G` | `Ctrl + G` | Go to Line… |
| `⌘F` | `Cmd + F` | Find… |
| `⌥⌘F` | `Cmd + Option + F` | Find and Replace… |
| `⌘G` / `⌘⇧G` | `Cmd + G` / `Cmd + Shift + G` | Find Next / Previous |
| `F3` / `⇧F3` | `F3` / `Shift + F3` | Find Next / Previous (alternate) |
| `⌘⇧F` | `Cmd + Shift + F` | Find in Files… |
| `F4` / `⇧F4` | `F4` / `Shift + F4` | Next / Previous Result |

The usual macOS editing shortcuts also work: `⌘Z` / `⌘⇧Z` (undo / redo),
`⌘X` / `⌘C` / `⌘V`, `⌘A`, `⌘F` (find), `⌥⌘F` (find and replace), word and line
navigation, and indent.

## Multi-cursor

| Shortcut | Keys | Action |
| --- | --- | --- |
| `⌘⌥↑` / `⌘⌥↓` | `Cmd + Option + Up` / `Cmd + Option + Down` | Add Cursor Above / Below |
| `⌃⌥↑` / `⌃⌥↓` | `Ctrl + Option + Up` / `Ctrl + Option + Down` | Add Cursor Above / Below (alternate) |
| `⌥` + click | `Option` + click | Add a cursor at the click |
| `⇧⌥` + drag | `Shift + Option` + drag | Column (block) selection |
| `⌘⇧L` | `Cmd + Shift + L` | Split into Lines |
| `⌘D` | `Cmd + D` | Add Next Occurrence |

Once more than one cursor is active, typing, deleting, pasting, and undo or redo
apply to every cursor at once. Add cursors by name from the Edit menu, the
editor's context menu, or the command palette.

Split into Lines turns a multi-line selection into one selection per line.
Add Next Occurrence selects the word under the caret, then adds the next match
of the selected text on each press, wrapping past the end of the file.

## View & window

| Shortcut | Keys | Action |
| --- | --- | --- |
| `⌘B` | `Cmd + B` | Toggle Sidebar |
| `⌘⇧P` | `Cmd + Shift + P` | Command Palette |
| `⌘R` | `Cmd + R` | Refresh |
| `⌥Z` | `Option + Z` | Toggle Word Wrap |
| `⌥⌘I` | `Cmd + Option + I` | Toggle Invisible Characters |
| `⌃⌘V` | `Ctrl + Cmd + V` | Toggle Preview (Markdown, SVG) |
| `⌘=` / `⌘-` / `⌘0` | `Cmd + =` / `Cmd + -` / `Cmd + 0` | Zoom In / Zoom Out / Actual Size |
| `⌘,` | `Cmd + ,` | Settings… |
| `⌘M` | `Cmd + M` | Minimize |
| `⌃⌘F` | `Ctrl + Cmd + F` | Toggle Full Screen |
| `⌘H` / `⌥⌘H` | `Cmd + H` / `Cmd + Option + H` | Hide / Hide Others |
| `⌘Q` | `Cmd + Q` | Quit |

## Modifier keys

| Symbol | Key |
| --- | --- |
| `⌘` | Command |
| `⇧` | Shift |
| `⌥` | Option |
| `⌃` | Control |
| `↩` | Return (Enter) |

## Find in Files

`⌘⇧F` opens the Find panel above the status bar. **Find** takes the query;
**Where** narrows the files with comma-separated globs, where a leading `-`
excludes (`src/**, -**/*.min.js`). Files listed in `.gitignore`, in the
`files.exclude` setting and in the `search.exclude` setting are always skipped.

**Replace All…** searches again, asks with the counts, then replaces. Open files
change in their tabs, unsaved, and one `⌘Z` reverts each. Closed files are saved
to disk with their line endings and byte order mark kept; a file changed since the
search is skipped. In regex mode `$1` and `${name}` insert capture groups. A search
that hits the 1,000-match limit is not replaced; narrow it with **Where** first.

| Key | In | Action |
| --- | --- | --- |
| `↩` | Find panel | Search into the Find Results tab |
| `⌥⌘↩` | Find panel | Replace All… (asks first) |
| `⇥` / `⇧⇥` | Find panel | Move between Find, Where and Replace |
| `⎋` | Find panel | Hide the panel |
| `↩` or double-click | Find Results | Open the match on that line |
