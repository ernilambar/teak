# Contributing

Contributions are welcome from anyone — bug reports, fixes, documentation, and feature
work. This guide covers setting up the project, making changes, and running the checks
required before a pull request.

## Who can contribute

Anyone. The project is maintained by
[Nilambar Sharma](https://nilambar.net/) ([@ernilambar](https://github.com/ernilambar)),
but external contributions are encouraged.
If you are planning a larger change, open an issue to discuss the approach first so we
can agree on the direction before you write code.

## Install

### Requirements

- Rust 1.92 or later — install with [rustup](https://rustup.rs)
- macOS 15 or later
- Xcode Command Line Tools — `xcode-select --install`
- `rustfmt` and `clippy` components — `rustup component add rustfmt clippy`

### Set up

```sh
git clone git@github.com:ernilambar/teak.git
cd teak
cargo build
```

The first build compiles GPUI and its dependencies and can take several minutes. Later
builds are fast.

## Fix and run

```sh
cargo run
```

This opens the native window. It requires a graphical macOS session; there is no
headless mode yet.

For a faster loop, `bacon run` (`cargo install bacon`) rebuilds and relaunches the app
on every save.

Some features only work from an app bundle: the `teak` command, Finder's "Open
With", and files opened through LaunchServices. To test them, build, install, and
relaunch the bundle in one step (needs `cargo install cargo-bundle`):

```sh
./scripts/dev.sh             # debug build
./scripts/dev.sh --release   # optimized build
./scripts/dev.sh file.txt    # relaunch and open a file
```

This replaces `/Applications/Teak.app` and kills the running copy, so save your work
in it first.

`./scripts/bundle.sh` builds a release bundle into `target/release/bundle/osx/` without
installing it. Regenerate the app icon with `python3 scripts/make-icon.py`.

### Code layout

- `src/main.rs`: entry point, actions, and keybindings
- `src/app/`: `TeakApp` state, split into `files.rs`, `tabs.rs`, and `ui.rs`
- `src/cli.rs`, `src/language.rs`, `src/paths.rs`, `src/session.rs`, `src/update.rs`:
  helpers with no GPUI dependency

Add a new module only once a feature is large enough to justify the boundary.

## Check locally before opening a pull request

Run these from the repository root and make sure they all pass:

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
cargo build
```

- `cargo fmt --check` — confirms the code matches `rustfmt` formatting.
- `cargo clippy --all-targets -- -D warnings` — lints the crate and treats warnings as errors.
- `cargo test` — runs the test suite.
- `cargo build` — confirms the app still compiles.

A future-incompatibility warning from a transitive dependency (`block`) is expected and
is not caused by application code.

## Open a pull request

- Branch off `main`, push your branch, and open a pull request against `main`.
- Write a clear, concise commit message and pull request description explaining what
  changed and why.
- Make sure the checks above pass, and mention anything you could not verify locally.

## Release

Maintainers only. `version` in `Cargo.toml` is the single source of truth. Bump it, then
let cargo update `Cargo.lock`:

```sh
# edit version in Cargo.toml
cargo update -p teak
```

Run the quality gate above, then tag the release so it matches the version:

```sh
git tag v0.1.2
git push origin v0.1.2
```

The Release workflow checks that the tag matches `Cargo.toml`, builds and ad-hoc signs
the app, and publishes `Teak-macos-arm64.zip` to a GitHub Release.

## Resources

- [GPUI Kit documentation](https://gpui-kit.com/docs/)
- [GPUI Kit component catalog](https://gpui-kit.com/component)
