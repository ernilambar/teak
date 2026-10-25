# Installing

The [README](../README.md#install) covers the recommended Homebrew install. This page
covers the other install methods, updates, uninstalling, and the command line tool.

## Install script

Without Homebrew, run this in Terminal:

```sh
curl -fsSL https://raw.githubusercontent.com/ernilambar/teak/main/scripts/install.sh | sh
```

## Manual download

Download `Teak-macos-arm64.zip` from the
[latest release](https://github.com/ernilambar/teak/releases/latest), unzip it, and
move `Teak.app` to `/Applications`.

The app is not notarized by Apple, so macOS blocks it the first time you open a copy
downloaded in a browser. To allow it, run this once:

```sh
xattr -dr com.apple.quarantine /Applications/Teak.app
```

## Update

Choose **Teak → Check for Updates…** to see if a new version is out. To update, save
your work and run:

```sh
brew update
brew upgrade --cask teak
```

If you used the install script, run it again instead. It replaces the existing app with
the latest release.

## Open files from Terminal

Choose **Teak → Install Command Line Tool…** and enter your password when asked.
Then:

```sh
teak notes.txt
teak one.txt two.txt
teak .
```

## Uninstall

```sh
brew uninstall --cask --zap teak
sudo rm -f /usr/local/bin/teak
```

`--zap` also removes your saved session and settings. The second line removes the
command line tool, if you installed it.

Without Homebrew:

```sh
rm -rf /Applications/Teak.app
sudo rm -f /usr/local/bin/teak
rm -rf ~/Library/Application\ Support/net.nilambar.teak
```
