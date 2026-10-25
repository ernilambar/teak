# AGENTS.md

Guidance for AI coding agents (Claude, Cursor, Copilot, Codex) working in this repository.

## Overview

This project is a text editor written in Rust. It builds its UI with
[GPUI Kit](https://gpui-kit.com) (`gpui-kit`), which bundles GPUI, GPUI Base, and GPUI
Component, and uses `ropey` plus `tree-sitter` for text buffering and syntax parsing.

## Setup

Requirements: Rust 1.99 or later, macOS 15 or later, and Xcode Command Line Tools.

```sh
git clone git@github.com:ernilambar/teak.git
cd teak
xcode-select --install
rustup component add rustfmt clippy
cargo build
```

`cargo build` fetches and compiles all dependencies. The first build compiles GPUI and is
slow; later builds are fast.

## Commands

Run every command from the repository root.

- Build: `cargo build`
- Release build: `cargo build --release`
- Test: `cargo test`
- Lint: `cargo clippy --all-targets -- -D warnings`
- Format: `cargo fmt`
- Format check: `cargo fmt --check`
- Typecheck: `cargo check`
- Run the app: `cargo run`
- Run and restart on save: `bacon run`
- Install bundle to `/Applications` and relaunch: `./scripts/dev.sh [--release]`

`cargo run` opens a native window and needs a graphical macOS session; do not rely on it
in headless environments.

## Conventions

- Use `gpui-kit` as the only UI dependency. Import GPUI via `use gpui_kit::*;` and
  components via `gpui_kit::component`. Do not add `gpui` or `gpui-component` directly —
  the kit pins their versions together.
- Bring element helper traits into scope: `use gpui_kit::base::StyledExt as _;` for
  `.h_flex()` / `.v_flex()`, and `use gpui_kit::component::ActiveTheme as _;` for
  `cx.theme()`.
- Never edit `Cargo.lock` by hand; regenerate it with a `cargo` command.
- Module layout: `main.rs` (entry, actions, keybindings) and `app/`, which is one
  coordinator type plus five state groups. `TeakApp` holds `focus_handle` and one
  field per group — `Workspace` (`app/workspace.rs`), `TabSet` (`app/tabs.rs`),
  `Overlays` (`app/overlays.rs`), `Chrome` (`app/chrome.rs`), `SettingsState`
  (`app/preferences.rs`) — and keeps every method that takes a `cx`/`Window`, since a
  plain field has no `Context`. A group's fields are private to its own module: callers
  go through methods, and each group needs a `new()` because `mod.rs` cannot initialize
  private fields. Leaf records (`OpenTab`, `TreeRow`, `Settings`) keep `pub(super)`
  fields so rendering can read them directly. `app/files.rs`, `editor.rs`, `find_in_files.rs`,
  `palette.rs`, `quick_open.rs` and `ui.rs` hold further `impl TeakApp`
  orchestration blocks; GPUI-free helpers live in `cli.rs`, `language.rs`, `paths.rs`,
  `search.rs`, `session.rs`, `settings.rs`, `update.rs`. Add a new module only once a feature is large enough to
  justify the boundary.
- Every user-invokable command is declared once in `commands.rs`: label, action,
  chords, menu placement, palette placement and search keywords. The menu bar, the
  command palette and the keymap are all derived from that table, so a new command
  means one row there plus its type in `actions!` and its handler — not three
  separate lists that can drift. Two things stay hand-written: each menu's `Entry`
  skeleton, because the menu bar also shows gpui-kit's own actions and two
  hand-written surfaces (a command's cluster number picks its slot there), and the
  handler registration, because GPUI dispatches by concrete type.
  The table holds `Box<dyn Action>`, which does not satisfy `KeyBinding::new`'s or
  `MenuItem::action`'s `impl Action` bound, so `commands.rs` builds those through
  `KeyBinding::load` and the `MenuItem::Action` variant directly — both public, but
  lower-level than the ergonomic constructors.
- `session.json` (app-written state) and `settings.json` (user-authored preferences) both
  live in `paths::support_dir()`. The app writes `settings.json` only to create a starter
  file or to record a choice made from the Settings overlay or a view toggle. Each write is
  a surgical `section.key` update that never rewrites unrelated keys. Group new settings
  keys by area (`editor.*`, `ui.*`).
- `vendor/gpui-base` is gpui-base with `patches/gpui-base.patch` applied, wired in through
  `[patch.crates-io]`. It adds `selected_ranges()` (read every selection); upstream
  `add_selection` covers adding them.
  Never edit `vendor/` without regenerating the patch: `./scripts/vendor-gpui-base.sh
  --diff`. After bumping `gpui-kit`, run `cargo fetch`, then
  `./scripts/vendor-gpui-base.sh <version>` with the gpui-base version from `Cargo.lock`;
  a version mismatch makes Cargo ignore the patch. If the patch no longer applies, re-add the
  change by hand in the fresh copy and run `--diff`.
- The app targets macOS. Do not add per-OS `cfg` branches.

## Releasing

`version` in `Cargo.toml` is the single source of truth. The About dialog, `--version`
output, and the bundle's `Info.plist` read it at build time; never hardcode a version
anywhere else.

To release a new version:

1. Bump `version` in `Cargo.toml`.
2. Run `cargo update -p teak` so `Cargo.lock` picks up the new version (do not edit it
   by hand).
3. If the year changed, update `copyright` under `[package.metadata.bundle]`.
4. Pass the quality gate and report the changes.

The maintainer tags `vX.Y.Z` and pushes it; the tag must match the `Cargo.toml` version
exactly. `.github/workflows/release.yml` then builds, ad-hoc signs, and publishes
`Teak-macos-arm64.zip` to a GitHub Release. Do not rename that asset:
`scripts/install.sh` downloads it by name.

## Quality gate

Before declaring any task complete, run these in order and confirm every one exits 0:

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
cargo build
```

A future-incompatibility warning from the transitive `block` crate is expected and is not
a failure. If any command fails, fix the cause and rerun the full sequence. Do not run
`git add` or `git commit` unless explicitly asked; report uncommitted work instead.
