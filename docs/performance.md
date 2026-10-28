# Performance

Teak's performance is measured, not assumed. The budgets live in
`src/app/bench.rs` as ignored tests, run on demand against a release build.

```sh
cargo test --release bench_ -- --ignored --nocapture
```

Each bench prints its best-of-five time next to a hard ceiling and fails past
that ceiling. The ceiling catches an order-of-magnitude regression; it is set
several times above the measured baseline so a loaded machine does not fail the
run.

## Why the budgets live in `src/` and not `benches/`

Teak is a binary crate. A `benches/` target (and criterion) can only reach a
library target, so publishing these would mean splitting `main.rs` into a lib and
a thin binary first. That refactor is worth doing, but it is not worth blocking
measurement on. The benches therefore live in the crate, next to the code they
measure, and use the same helpers the app does. When Teak grows a `lib.rs`, the
same five cases can move to criterion without changing what they mean.

Fixtures are generated deterministically in the bench instead of committed. A
multi-megabyte source file in the repository would bloat clones and rot as the
editor changes.

## Budgets

Measured on an Apple Silicon Mac in a release build, October 2026. Numbers vary
with the machine; the ceiling is what the test enforces.

| Bench | What it measures | Baseline | Ceiling |
| --- | --- | --- | --- |
| `open_100k_lines` | Read, decode (BOM/CRLF) and build the line index for a 100,000-line file | ~2.7 ms | 30 ms |
| `keystroke_100k_lines` | One character inserted, then the line index and full text rebuilt as a line command does | ~3.0 ms | 20 ms |
| `quick_open_index` | Fuzzy score for every indexed path, against both the file name and the relative path, at the 20,000-file index cap | ~5.0 ms | 30 ms |
| `find_regex_100k_lines` | Compile a regex and scan a 100,000-line buffer, capped at 1,000 matches | ~3.7 ms | 30 ms |
| `git_gutter_100k_lines` | Changed-line read after one edit: `git show` of HEAD plus `git diff --no-index` against the buffer | ~70 ms | 300 ms |

## What the budgets do not cover

These are deliberate gaps, each with the phase that should close it.

- **Editor engine layout and paint.** The keystroke bench covers Teak's own line
  arithmetic, not GPUI's shaping, wrapping and painting, which need a window to
  measure. A headless render bench (Phase 1) should add frame time on a large
  file.
- **File walk and disk I/O.** `collect_files` and the search walk are excluded;
  the fixtures are already in memory or on a warm cache. A cold-cache bench is
  noisy and belongs in a separate, non-blocking job.
- **Sorting and rendering of results.** The quick-open and find benches measure
  matching only. Sorting 20,000 scores and rendering 100 rows are separate and
  should be measured with the picker refactor (Phase 2).
- **Memory.** No allocation or RSS budgets yet. A large-file memory check is
  worth adding alongside the document/view split (Phase 1), when the number of
  buffers per file becomes explicit.

## Visual regression

gpui-kit 0.8 exposes element-tree snapshots but no pixel capture, so there is no
image diff. Instead, three headless render tests in `src/app/ui.rs` paint a
scene and assert that its surfaces exist with a non-zero size:

- `the_editor_scene_paints_its_surfaces` — tab strip, sidebar, explorer and the
  editor's text area.
- `the_palette_scene_paints_its_search_field` — the command palette's search
  field, found by its placeholder.
- `the_find_in_files_scene_paints_its_controls` — the Find button and the open
  panel state.

These catch a pane that renders empty, a collapsed tab strip, or an overlay that
fails to mount. They do not catch a wrong color or a one-pixel shift; that needs
pixel capture, which the kit does not provide today.

## Rule for every phase

A change that regresses a ceiling does not land. If a ceiling has to move, say
why in the pull request and update this page in the same commit.
