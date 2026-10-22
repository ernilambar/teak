# Teak

A fast, lightweight text editor.

## Screenshot

![Screenshot](screenshot.png)

## Features

- Tabs, each with its own file
- Syntax highlighting
- File explorer sidebar for browsing a project folder
- Command palette (`Cmd+Shift+P`) to search and run any command
- Quick Open (`Cmd+P`) to fuzzy-find a file across the workspace folders
- Find in Files (`Cmd+Shift+F`): results grouped by file in a Find Results tab,
  `F4` / `Shift+F4` to step through matches, match case / whole word / regex, a
  Where field for include and exclude globs, and Replace All across files
- Quick Open and Find in Files skip what `.gitignore` lists, plus any globs in the
  `search.exclude` setting
- Markdown preview (`Cmd+Shift+V`)
- Reopens your tabs and folder where you left off
- File → Open Recent for recently opened files and folders
- Drag files and folders from Finder onto the window to open them
- Warns before closing a file with unsaved changes
- Reloads files changed by other apps, and asks first if you have unsaved edits
- Deleting from the sidebar moves files to the Trash
- Keeps each file's line endings (LF or CRLF) and byte order mark
- Optional trim trailing whitespace and final newline on save
- Cursor position, encoding, line endings, and language in the status bar
- `teak` command to open files from the terminal
- Light, dark, and system-following themes

## Requirements

- macOS 15 or later
- Apple Silicon Mac (M1 or newer)

## Install

Install with [Homebrew](https://brew.sh):

```sh
brew tap ernilambar/tap
brew trust ernilambar/tap
brew install --cask ernilambar/tap/teak
```

This installs the app into `/Applications`.

## Docs

- [Keyboard shortcuts](docs/keymaps.md)
- [Install options](docs/install.md)
- [Color schemes](docs/themes.md)

## Contributing

Bug reports and pull requests are welcome. See [CONTRIBUTING.md](CONTRIBUTING.md) to
build it from source.

## License

[MIT](LICENSE) © Nilambar Sharma ([nilambar.net](https://nilambar.net/))
