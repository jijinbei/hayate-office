# HayateOffice — guide for Claude

A fast, lightweight Office suite in Rust (MVP: a presentation editor) built on the **gpui** GPU UI
framework (a Zed fork, pinned in `Cargo.toml`). Design docs live in `docs/DESIGN.md` and
`docs/REQUIREMENTS.md`.

## Workspace layout
- `hayate-ir` — data-oriented document model (`World`/`Entity` + sparse component columns via the
  `define_world!` macro; `Presentation`, `Theme`, placeholders). gpui-free.
- `hayate-model` — editing helpers that build `Transaction`s, plus undo/redo `History`. gpui-free.
- `hayate-render` — backend-agnostic `Scene`/`Primitive` builder (`build_slide_scene`,
  `build_container_scene`), a headless rasterizer, SVG and PDF export. gpui-free → unit-testable.
- `hayate-core` — command registry. `hayate-format` / `hayate-format-pptx` — `.hayate` and PPTX I/O.
- `hayate-shot` — headless PNG debug harness (gpui-free).
- `hayate-app` — the gpui application (canvas, mouse/keyboard handlers, panels). Split into
  `actions.rs`, `input.rs`, `mouse.rs`, `view.rs`, `paint.rs`, `menu.rs`, `slides.rs`, `layers.rs`,
  `icons.rs`, `io.rs`, `util.rs`, `widgets.rs`, and `e2e.rs` (tests).

## Conventions
- Code comments, identifiers, and commit messages are in **English**; chat with the user in Japanese.
- gpui comes from the pinned fork (do not switch to crates.io).
- This is a Nix dev environment: anything touching gpui (the app, e2e, clippy, run) must go through
  `nix develop` — the `just` recipes below already do this. The pure crates build with plain `cargo`.

## Running tests & debugging

There are **two debugging layers** because this is a GPU app and there is no way to screenshot a real
window headlessly:

### 1. gpui interaction E2E (`just e2e`) — the main way to debug UI behavior
`crates/hayate-app/src/e2e.rs` drives the **real** UI handlers (`on_mouse_down`, `on_key_down`,
`on_right_down`, context-menu actions, editing actions) through gpui's headless `TestAppContext` and
asserts on the editor's real state. No GPU/window needed; the `test-support` feature stubs the
platform. Run with:

```
just e2e          # = nix develop --command cargo test -p hayate-app
```

Pattern (see existing tests for many examples):
```rust
#[gpui::test]
fn my_behavior(cx: &mut TestAppContext) {
    let app = cx.new(|cx| HayateApp::new(cx));               // open the editor
    app.update(cx, |a, cx| a.on_key_down(&keydown("ctrl-s"), cx)); // inject a real event/action
    let ok = app.read_with(cx, |a, _| a.save_modal.is_some());     // assert on real state
    assert!(ok);
}
```
Helpers in `e2e.rs`: `mouse(button, x, y)`, `mouse_move(x, y)`, `mouse_up(x, y)`, `keydown("ctrl-s")`
(any `Keystroke::parse` string, e.g. `"enter"`, `"shift-tab"`), and `prim_bounds(&node.prim)` to find
a shape's on-screen rect. Use `app.update` to mutate/drive and `app.read_with` to assert. Prefer
asserting on document/scene state (`a.pres…`, `a.scene.nodes…`) over pixels.

When you change UI behavior, add or update an `e2e.rs` test and keep `just e2e` green.

### 2. Visual PNG snapshots (`just shots`) — for "does it look right" on the gpui-free render path
`hayate-shot` runs edit scenarios and rasterizes scenes to `debug-shots/*.png`, which you can open
with the Read tool to eyeball geometry/layout/colors. Note: the rasterizer is a separate path from the
gpui `paint_*` code, and its text is an ASCII bitmap (non-Latin shows as boxes) — use it for shapes,
layout, transforms, z-order, theme colors, not for real glyph/caret fidelity.

```
just shots        # writes debug-shots/*.png (gpui-free, no Nix needed)
```

For PDF output you can additionally validate with `pdfinfo` / render a page with `pdftoppm` and Read
the PNG.

### Other recipes
```
just test         # pure-crate tests (ir/model/core/render/format) with plain cargo
just build-app    # compile the gpui app inside the Nix shell
just run          # run the app (injects the host GPU driver via nix-gl-host)
cargo fmt --all   # format before committing
```

### Typical loop
1. Change code. 2. `just test` for logic, add/adjust an `e2e.rs` test and run `just e2e` for UI
behavior, `just shots` + Read the PNGs for visual checks. 3. `cargo fmt --all`. 4. Commit when green.
