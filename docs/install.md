# Installing

The [README](../README.md#install) covers the recommended Homebrew install. This page
covers the other install methods, updates, uninstalling, and the command line tool.

## Install script

Without Homebrew, run this in Terminal:

```sh
curl -fsSL https://raw.githubusercontent.com/ernilambar/tine/main/scripts/install.sh | sh
```

## Manual download

Download `Tine-macos-arm64.zip` from the
[latest release](https://github.com/ernilambar/tine/releases/latest), unzip it, and
move `Tine.app` to `/Applications`.

The app is not notarized by Apple, so macOS blocks it the first time you open a copy
downloaded in a browser. To allow it, run this once:

```sh
xattr -dr com.apple.quarantine /Applications/Tine.app
```

## Update

Choose **Tine → Check for Updates…** to see if a new version is out. To update, save
your work and run:

```sh
brew upgrade --cask tine
```

If you used the install script, run it again instead. It replaces the existing app with
the latest release.

## Open files from Terminal

Choose **Tine → Install Command Line Tool…** and enter your password when asked.
Then:

```sh
tine notes.txt
tine one.txt two.txt
tine .
```

## Uninstall

```sh
brew uninstall --cask --zap tine
sudo rm -f /usr/local/bin/tine
```

`--zap` also removes your saved session and settings. The second line removes the
command line tool, if you installed it.

Without Homebrew:

```sh
rm -rf /Applications/Tine.app
sudo rm -f /usr/local/bin/tine
rm -rf ~/Library/Application\ Support/net.nilambar.tine
```
