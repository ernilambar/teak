# Building from Source

This page covers building and running Teak from source on any supported OS.
For installing a released build, see [Install options](install.md).

## Requirements

| OS      | Requirements                                                                                                      |
| ------- | ----------------------------------------------------------------------------------------------------------------- |
| macOS   | macOS 15 or later, Apple Silicon. [Xcode Command Line Tools](https://developer.apple.com/xcode/) (`xcode-select --install`). |
| Linux   | x86_64 or aarch64. A Wayland or X11 session, GPU/EGL drivers, and the system libraries below.                     |
| Windows | x86_64. [Visual Studio Build Tools](https://visualstudio.microsoft.com/downloads/) with the "Desktop development with C++" workload (MSVC compiler + Windows 10/11 SDK). See below. |

All platforms need Rust 1.99 or later. Install it with [rustup](https://rustup.rs),
then add the required components:

```sh
rustup component add rustfmt clippy
```

### Windows toolchain

Rust on Windows uses the MSVC linker, so a C++ toolchain is required even
though Teak is pure Rust:

1. Install [Visual Studio Build Tools](https://visualstudio.microsoft.com/downloads/)
   (free, no full Visual Studio needed).
2. In the installer, select the **"Desktop development with C++"** workload.
   This provides the MSVC compiler (`cl.exe`), the linker (`link.exe`), and a
   Windows 10/11 SDK — all required for `cargo build` to link.
3. Make sure `rustup` uses the MSVC toolchain (the default on Windows —
   check with `rustup show` and look for `stable-x86_64-pc-windows-msvc`).
   If it shows `gnu`, switch with `rustup default stable-x86_64-pc-windows-msvc`.
4. Build from a shell that sees the toolchain: the "x64 Native Tools Command
   Prompt for VS", or any terminal after a normal install (rustup finds
   `link.exe` automatically in most setups).

You do **not** need this to *run* a released or already-built `teak.exe` —
only to compile from source.

### Linux system packages

On Debian/Ubuntu, install the build dependencies:

```sh
sudo apt-get update
sudo apt-get install -y --no-install-recommends \
  libfontconfig1-dev \
  libfreetype6-dev \
  libwayland-dev \
  libx11-dev \
  libx11-xcb-dev \
  libxcb1-dev \
  libxcb-composite0-dev \
  libxcb-damage0-dev \
  libxcb-present-dev \
  libxcb-render0-dev \
  libxcb-res0-dev \
  libxcb-shape0-dev \
  libxcb-shm0-dev \
  libxcb-xfixes0-dev \
  libxkbcommon-dev \
  libxkbcommon-x11-dev
```

On other distributions, install the equivalent `-dev`/`-devel` packages for
X11, xcb, Wayland, xkbcommon, fontconfig, and freetype.

To run a built binary on another Linux machine, that machine needs the runtime
libraries: `libxcb1`, `libxkbcommon0`, `libxkbcommon-x11-0`,
`libwayland-client0`, `libwayland-egl1`, and `libegl1`
(or the distribution equivalents). If the app fails to start, `ldd` shows what
is missing:

```sh
ldd target/release/teak
```

## Clone and build

```sh
git clone git@github.com:ernilambar/teak.git
cd teak
cargo build
```

The first build compiles GPUI and its dependencies and can take several
minutes. Later builds are fast.

| Command                 | What it does                              |
| ----------------------- | ----------------------------------------- |
| `cargo build`           | Debug build (`target/debug/teak`)         |
| `cargo build --release` | Optimized build (`target/release/teak`)   |
| `cargo run`             | Build and launch the app                  |
| `bacon run`             | Rebuild and relaunch on every save (`cargo install bacon`) |

`cargo run` needs a graphical session; there is no headless mode.

## Running what you built

### macOS

`cargo run` works for plain editor changes. To test bundle-only behavior (the
`teak` command, Finder's "Open With", files opened through LaunchServices),
build and install the `.app` bundle (needs `cargo install cargo-bundle`):

```sh
./scripts/dev.sh             # debug build
./scripts/dev.sh --release   # optimized build
./scripts/dev.sh file.txt    # relaunch and open a file
```

This replaces `/Applications/Teak.app` and kills the running copy, so save
your work in it first. `./scripts/bundle.sh` builds a release bundle into
`target/release/bundle/osx/` without installing it.

### Linux

Run the binary directly:

```sh
./target/release/teak [FOLDER]...
```

Folders passed on the command line open in the sidebar. For a `teak` command
on `PATH`, choose **Teak → Install Command Line Tool** in the app, which
installs a launcher into `~/.local/bin/teak` (make sure `~/.local/bin` is on
your `PATH`).

### Windows

Run the binary directly:

```sh
target\release\teak.exe
```

For a `teak` command on `PATH`, choose **Teak → Install Command Line Tool**
in the app, which installs a `teak.cmd` shim into
`%LOCALAPPDATA%\Microsoft\WindowsApps`.

## Where the app keeps its data

| OS      | Session, settings, themes                                  | CLI tool                                    |
| ------- | ---------------------------------------------------------- | ------------------------------------------- |
| macOS   | `~/Library/Application Support/net.nilambar.teak`          | `/usr/local/bin/teak` (admin prompt)        |
| Linux   | `~/.config/teak` (or `$XDG_CONFIG_HOME/teak`)              | `~/.local/bin/teak`                         |
| Windows | `%APPDATA%\net.nilambar.teak`                              | `%LOCALAPPDATA%\Microsoft\WindowsApps\teak.cmd` |

## Check before opening a pull request

Run these from the repository root and make sure they all pass:

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
cargo build
```

A future-incompatibility warning from a transitive dependency (`block`) is
expected and is not caused by application code.

## Troubleshooting

- **Linux: missing shared library** — install the runtime libraries above and
  re-check with `ldd target/release/teak`.
- **Linux: no window opens** — make sure you are in a Wayland or X11 session
  (`$WAYLAND_DISPLAY` or `$DISPLAY` set) with working GPU/EGL drivers.
- **macOS: Gatekeeper blocks the bundle** — a local `cargo bundle` build is
  only ad-hoc signed, which is enough for the machine that built it. See
  [Install options](install.md) for sharing builds.
