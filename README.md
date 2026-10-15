# Tinytext

A fast, lightweight text editor for macOS.

## Screenshot

![Screenshot](screenshot.png)

## Features

- Tabs, each with its own file
- Syntax highlighting
- File explorer sidebar for browsing a project folder
- Command palette (`Cmd+Shift+P`) to search and run any command
- Quick Open (`Cmd+P`) to fuzzy-find a file across the workspace folders
- Markdown preview (`Cmd+Shift+V`)
- Reopens your tabs and folder where you left off
- Warns before closing a file with unsaved changes
- Cursor position, encoding, and language in the status bar
- `tinytext` command to open files from the terminal
- Light, dark, and system-following themes

## Requirements

- macOS 15 or later
- Apple Silicon Mac (M1 or newer)

## Install

Install with [Homebrew](https://brew.sh):

```sh
brew tap ernilambar/tap
brew trust ernilambar/tap
brew install --cask ernilambar/tap/tinytext
```

This installs Tinytext into `/Applications`.

## Docs

- [Keyboard shortcuts](docs/keymaps.md)
- [Install options](docs/install.md)
- [Color schemes](docs/themes.md)

## Contributing

Bug reports and pull requests are welcome. See [CONTRIBUTING.md](CONTRIBUTING.md) to
build Tinytext from source.

## License

[MIT](LICENSE) © Nilambar Sharma ([nilambar.net](https://nilambar.net/))
