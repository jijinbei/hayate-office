# HayateOffice — guide for Claude

Fast, lightweight Office suite in Rust (MVP: presentation editor) on the **gpui** GPU framework
(pinned Zed fork). Design: `docs/DESIGN.md`, `docs/REQUIREMENTS.md`. Comments/identifiers/commits in
English; chat in Japanese. gpui-touching builds need `nix develop` — the `just` recipes handle that.

## Debugging (two layers; see `Justfile` for all recipes)
- **`just e2e`** — gpui interaction E2E: `crates/hayate-app/src/e2e.rs` drives the real handlers
  (`on_mouse_down`/`on_key_down`/menu/editing actions) headlessly via `TestAppContext` and asserts on
  real state. **Read `e2e.rs` for the patterns/helpers** (`mouse`/`keydown`/`prim_bounds`,
  `app.update`/`read_with`); copy an existing test. Add/adjust one when you change UI behavior.
- **`just shots`** — gpui-free PNG snapshots to `debug-shots/*.png` (open with Read) for shape/layout/
  color/transform checks. Not for glyph/caret fidelity (rasterizer ≠ gpui paint; text is ASCII-only).

Other: `just test` (pure crates), `just build-app`, `just run`, `cargo fmt --all`. Loop: change →
`just test` for logic / `just e2e` for UI / `just shots` to eyeball → fmt → commit when green.
