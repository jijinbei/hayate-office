//! HayateOffice presentation editor (gpui app).
//!
//! Renders a sample slide's resolved Scene onto a gpui canvas and supports basic editing:
//! click to select a shape (hit-test against the Scene), drag to move it (committed as one
//! undoable transaction). Undo/redo: Cmd/Ctrl+Z / Shift+Cmd/Ctrl+Z.
//!
//! # Debugging & E2E tests
//!
//! Two layers (recipes in the repo `Justfile`):
//! - **`just e2e`** — gpui interaction E2E. The `e2e` module (this crate, `src/e2e.rs`, behind
//!   `#[cfg(test)]`) drives the real handlers (`on_mouse_down`/`on_key_down`/`on_right_down`, menu
//!   and editing actions) headlessly through gpui's `TestAppContext` and asserts on the editor's
//!   real state — no GPU/window needed. Write tests with `#[gpui::test] fn …(cx: &mut
//!   TestAppContext)`: `cx.new(|cx| HayateApp::new(cx))` to open the editor, `app.update(cx, …)` to
//!   inject an event/action, `app.read_with(cx, …)` to assert. Prefer asserting on document/scene
//!   state (`a.pres…`, `a.scene.nodes…`) over pixels. Helpers in `e2e.rs`: `mouse`, `mouse_move`,
//!   `mouse_up`, `keydown("ctrl-s")` (any `Keystroke::parse` string), `prim_bounds`. Copy an
//!   existing test as a template; add/adjust one whenever you change UI behavior.
//! - **`just shots`** — gpui-free PNG snapshots (`debug-shots/*.png`, open with the Read tool) for
//!   shape/layout/color/transform checks. Not for glyph/caret fidelity (the rasterizer is a
//!   separate path from gpui paint and renders text as an ASCII bitmap).
//!
//! Anything touching gpui (app, e2e, run, clippy) must build inside `nix develop`; the `just`
//! recipes handle that. Pure crates build with plain `cargo` (`just test`). Run `cargo fmt --all`
//! before committing.

#![cfg_attr(target_family = "wasm", no_main)]

use std::cell::Cell;
use std::ops::Range;
use std::rc::Rc;

use gpui::{
    div, point, prelude::*, px, rgb, size, App, Bounds, Pixels, Point, Window, WindowBounds,
    WindowOptions,
};
use gpui_platform::application;

use hayate_core::CommandRegistry;
use hayate_ir::color::{Color, ThemeColorToken};
use hayate_ir::font::{FontRef, ThemeFontSlot};
use hayate_ir::geom::RectEmu;
use hayate_ir::paint::Fill;
use hayate_ir::presentation::Presentation;
use hayate_ir::shape::Geometry;
use hayate_ir::text::{Paragraph, Run, TextBody};
use hayate_ir::theme::Theme;
use hayate_ir::units::{inch_f, pt};
use hayate_ir::world::{CompValue, Entity};
use hayate_model::History;
use hayate_render::scene::{PxSize, Scene};
use hayate_render::{build_slide_scene, Guide};

mod actions;
mod home;
mod icons;
mod input;
mod io;
mod layers;
mod menu;
mod mouse;
mod paint;
mod recent;
mod slides;
mod util;
mod view;
mod widgets;

const SELECTION: u32 = 0x3B82F6;
const DOC_PATH: &str = "hayate-sample.hayate";

/// Build a small sample deck: a title, three accent rectangles, and an ellipse.
fn sample_presentation() -> Presentation {
    let mut p = Presentation::new();
    let master = p.add_master(Theme::default());
    let layout = p.add_layout(master, "Blank");
    let slide = p.add_slide(layout);

    let title = p.add_shape(slide);
    p.world.frames.insert(
        title,
        RectEmu::new(inch_f(0.5), inch_f(0.3), inch_f(9.0), inch_f(1.0)),
    );
    p.world.texts.insert(
        title,
        TextBody {
            paragraphs: vec![Paragraph::new(vec![Run {
                text: "Hayate プレゼンテーション".to_string(),
                font: FontRef::Theme(ThemeFontSlot::Major),
                size: pt(40),
                color: Color::theme(ThemeColorToken::Dk1),
                bold: true,
                italic: false,
                underline: false,
            }])],
            autofit: false,
        },
    );

    let accents = [
        ThemeColorToken::Accent1,
        ThemeColorToken::Accent2,
        ThemeColorToken::Accent3,
    ];
    for (i, token) in accents.into_iter().enumerate() {
        let e = p.add_shape(slide);
        let x = inch_f(0.5 + i as f64 * 2.0);
        p.world
            .frames
            .insert(e, RectEmu::new(x, inch_f(1.8), inch_f(1.6), inch_f(1.6)));
        p.world.geometries.insert(e, Geometry::Rect);
        p.world.fills.insert(e, Fill::Solid(Color::theme(token)));
    }

    let oval = p.add_shape(slide);
    p.world.frames.insert(
        oval,
        RectEmu::new(inch_f(6.8), inch_f(1.8), inch_f(2.4), inch_f(1.6)),
    );
    p.world.geometries.insert(oval, Geometry::Ellipse);
    p.world
        .fills
        .insert(oval, Fill::Solid(Color::theme(ThemeColorToken::Accent4)));

    p
}

/// On-screen slide size in pixels at the given zoom (1pt -> 1px at zoom 1.0).
fn view_px(pres: &Presentation, zoom: f32) -> PxSize {
    let pt = |v: i64| v as f32 / 12_700.0;
    PxSize {
        w: pt(pres.slide_size.w) * zoom,
        h: pt(pres.slide_size.h) * zoom,
    }
}

fn pt_to_emu(v: f32) -> i64 {
    (v * 12_700.0) as i64
}

#[derive(Clone)]
struct Drag {
    /// (entity, start frame) for every shape moving in this drag (group / multi-select).
    entities: Vec<(Entity, RectEmu)>,
    /// The shape under the cursor; used as the snapping reference.
    primary: Entity,
    start_cursor: Point<Pixels>,
}

/// Dragging one endpoint of a line/arrow. The other endpoint stays fixed, so the line can be
/// aimed in any direction (the frame's size may go negative). `drag_end` is true when the END
/// point (`to`) follows the cursor, false for the START (`from`).
#[derive(Clone, Copy)]
struct LineDrag {
    entity: Entity,
    drag_end: bool,
    /// The fixed endpoint in slide coordinates (EMU).
    fixed: hayate_ir::geom::PointEmu,
    /// Frame at the start of the drag, for one undoable commit.
    start_frame: RectEmu,
}

struct HayateApp {
    pres: Presentation,
    slide: Entity,
    history: History,
    scene: Scene,
    selection: Option<Entity>,
    drag: Option<Drag>,
    /// Canvas top-left in window coords, written each paint, read by mouse handlers.
    canvas_origin: Rc<Cell<Point<Pixels>>>,
    /// Keyboard focus for the editor (so Ctrl/Cmd+Z reaches us).
    focus: gpui::FocusHandle,
    focused_once: bool,
    /// View zoom (display scale only; document coordinates stay in absolute points).
    /// 1.0 = 100% (1pt -> 1px). Independent of window size.
    zoom: f32,
    /// Command registry (palette / scripts / AI surface). `Rc` so the script runtime can hold
    /// it across the engine's `'static` host closures.
    registry: Rc<CommandRegistry>,
    /// Command palette state when open.
    palette: Option<PaletteState>,
    /// Open script console (editable buffer), if any. Ctrl+Shift+R toggles it.
    script_panel: Option<ScriptPanel>,
    /// Commands registered by scripts (via `register_command`), shown in the palette.
    script_commands: Vec<hayate_core::RegisteredCommand>,
    /// Numeric field being typed into (rotation/position/size/opacity), if any.
    field_edit: Option<FieldEdit>,
    /// Alignment guides shown while dragging (scene/px coords relative to the slide origin).
    guides: Vec<Guide>,
    /// Whether the editing grid is shown.
    show_grid: bool,
    /// Active resize-by-handle drag, if any.
    resize: Option<ResizeDrag>,
    /// Copied shape components (Ctrl+C / Ctrl+V).
    /// Copied shapes' components (Ctrl+C / Ctrl+V), one inner Vec per shape.
    clipboard: Option<Vec<Vec<CompValue>>>,
    /// In-canvas text editing state, if any.
    text_edit: Option<TextEdit>,
    /// Additional selected entities (besides `selection`) for multi-select align.
    also: Vec<Entity>,
    /// Fullscreen presentation (slideshow) mode.
    present: bool,
    /// Animation playback time (ms) within the current slide in presentation mode.
    present_t: u32,
    /// Open right-click context menu, if any.
    context_menu: Option<ContextMenu>,
    /// Whether the font picker overlay is open.
    font_picker: bool,
    /// Which list the left panel shows (slide thumbnails vs. layers).
    left_tab: LeftTab,
    /// Layer being renamed in the Layers panel: (entity, edit buffer).
    renaming: Option<(Entity, String)>,
    /// Active line endpoint drag, if any.
    line_drag: Option<LineDrag>,
    /// Path the document saves to (editable via the Save dialog).
    doc_path: String,
    /// Open "Save As" dialog with its editable filename buffer, if any.
    save_modal: Option<SaveModal>,
    /// Transient notice modal (e.g. "PDF exported …"), dismissed by OK/Esc/backdrop click.
    notice: Option<String>,
    /// Layout currently being edited in the Master tab (its placeholders apply to every slide
    /// that uses it). `None` = no layout selected for editing.
    master_layout: Option<Entity>,
    /// What the canvas edits: the current slide, or a layout/master in master edit mode.
    scope: EditScope,
    /// Layout being renamed in the Master tab: (layout, edit buffer).
    layout_rename: Option<(Entity, String)>,
    /// Where the next font-picker choice applies (selection vs. theme fonts).
    font_target: FontTarget,
    /// Bullet-list indent per level, in ems of the line's font size (user-adjustable).
    list_indent_em: f32,
    /// Width of the left sidebar in px (user-draggable).
    sidebar_w: f32,
    /// Whether the sidebar divider is being dragged.
    resizing_sidebar: bool,
    /// Active marquee (rubber-band) selection rect in scene px: (start_x, start_y, cur_x, cur_y).
    marquee: Option<(f32, f32, f32, f32)>,
    /// Last window viewport size; used to refit the slide when the window is resized.
    last_viewport: Option<gpui::Size<gpui::Pixels>>,
    /// Whether the home/start screen is shown instead of the editor. True at launch; left when a
    /// presentation is created ("New") or opened from the recents list.
    home: bool,
    /// Recent presentations (with thumbnails) for the home screen, built lazily on first show.
    home_recents: Vec<RecentThumb>,
    /// Whether `home_recents` has been built since the home screen was last shown.
    home_loaded: bool,
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum MenuTarget {
    Shape,
    Slide,
    Canvas,
    Layout(Entity),
}

/// Where the font picker applies its choice: the selected shape, or the theme heading/body font.
#[derive(Clone, Copy, PartialEq)]
enum FontTarget {
    Selection,
    ThemeMajor,
    ThemeMinor,
}

/// Which list the left panel shows.
#[derive(Clone, Copy, PartialEq)]
enum LeftTab {
    Slides,
    Layers,
    Master,
}

/// What the canvas is currently editing. Slides are the normal case; a layout or master is
/// edited in place ("master edit mode"), so the same shape tools operate on its children.
#[derive(Clone, Copy, PartialEq)]
enum EditScope {
    Slide(Entity),
    Layout(Entity),
    Master(Entity),
}

impl EditScope {
    /// The container entity whose children are edited/rendered.
    fn container(self) -> Entity {
        match self {
            EditScope::Slide(e) | EditScope::Layout(e) | EditScope::Master(e) => e,
        }
    }
    fn is_slide(self) -> bool {
        matches!(self, EditScope::Slide(_))
    }
}

struct ContextMenu {
    /// Window-space position of the menu's top-left.
    x: f32,
    y: f32,
    target: MenuTarget,
}

/// Drag-and-drop payload identifying the slide being reordered in the sidebar.
#[derive(Clone, Copy)]
struct DraggedSlide(Entity);

/// A lightweight preview rendered under the cursor while dragging a slide thumbnail.
struct SlideDragPreview;

impl Render for SlideDragPreview {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .w(px(176.))
            .h(px(99.))
            .border_2()
            .border_color(rgb(SELECTION))
            .bg(rgb(0x2b2b2b))
    }
}

struct TextEdit {
    entity: Entity,
    original: String,
    /// Bullet level per line at edit start, for Esc revert (parallel to `original` lines).
    orig_levels: Vec<u8>,
    buf: String,
    /// Bullet list level per line of `buf` (index = line number; 0 = no bullet).
    levels: Vec<u8>,
    /// Caret/selection as a BYTE range into `buf` (caret when start == end).
    selected: Range<usize>,
    /// IME composing (marked) range, as a BYTE range into `buf`.
    marked: Option<Range<usize>>,
}

impl TextEdit {
    /// Number of lines in the buffer (paragraphs once committed).
    fn line_count(&self) -> usize {
        self.buf.split('\n').count()
    }
    /// Line index containing the caret.
    fn caret_line(&self) -> usize {
        self.buf[..self.selected.start.min(self.buf.len())]
            .matches('\n')
            .count()
    }
    /// Keep `levels` the same length as the line count (pad with 0, truncate extras).
    fn reconcile_levels(&mut self) {
        let n = self.line_count().max(1);
        if self.levels.len() < n {
            self.levels.resize(n, 0);
        } else {
            self.levels.truncate(n);
        }
    }
}

#[derive(Clone, Copy)]
struct ResizeDrag {
    handle: usize,
    start_frame: RectEmu,
    start_cursor: Point<Pixels>,
}

#[derive(Clone, Copy, PartialEq)]
enum FieldKind {
    Rotation,
    PosX,
    PosY,
    SizeW,
    SizeH,
    Opacity,
    StrokeWidth,
}

struct FieldEdit {
    kind: FieldKind,
    buf: String,
}

struct PaletteState {
    query: String,
    sel: usize,
}

/// A recent presentation shown on the home screen: its file path, display name, and a pre-built
/// first-slide scene (+ media) rendered as the thumbnail.
struct RecentThumb {
    path: String,
    name: String,
    scene: Scene,
    media: std::collections::BTreeMap<String, Vec<u8>>,
}

impl HayateApp {
    fn new(cx: &mut Context<Self>) -> Self {
        let pres = sample_presentation();
        let slide = pres.slides()[0];
        let zoom = 0.8;
        let scene = build_slide_scene(&pres, slide, view_px(&pres, zoom));
        HayateApp {
            pres,
            slide,
            history: History::new(),
            scene,
            selection: None,
            drag: None,
            canvas_origin: Rc::new(Cell::new(point(px(0.), px(0.)))),
            focus: cx.focus_handle(),
            focused_once: false,
            zoom,
            registry: Rc::new(hayate_core::builtins()),
            palette: None,
            field_edit: None,
            guides: Vec::new(),
            show_grid: false,
            resize: None,
            clipboard: None,
            text_edit: None,
            also: Vec::new(),
            present: false,
            present_t: 0,
            context_menu: None,
            font_picker: false,
            left_tab: LeftTab::Slides,
            renaming: None,
            line_drag: None,
            doc_path: DOC_PATH.to_string(),
            save_modal: None,
            script_panel: None,
            script_commands: Vec::new(),
            notice: None,
            master_layout: None,
            scope: EditScope::Slide(slide),
            layout_rename: None,
            font_target: FontTarget::Selection,
            list_indent_em: 0.5,
            sidebar_w: 208.0,
            resizing_sidebar: false,
            marquee: None,
            last_viewport: None,
            home: true,
            home_recents: Vec::new(),
            home_loaded: false,
        }
    }
}

/// "Save As" dialog state: an editable filename buffer.
struct SaveModal {
    buf: String,
}

/// Script console state: an editable Rhai source buffer. Run with Ctrl/Cmd+Enter; the result
/// (op count / print log / error) is shown in the notice modal.
struct ScriptPanel {
    buf: String,
}

/// Embedded application logo, used for the window/taskbar icon.
const LOGO_PNG: &[u8] = include_bytes!("../../../assets/icon.png");

/// Decode the embedded logo into the RGBA image gpui takes for the window icon (X11). Returns
/// `None` if decoding fails, in which case the window opens without a custom icon.
fn app_icon() -> Option<std::sync::Arc<image::RgbaImage>> {
    let img = image::load_from_memory(LOGO_PNG).ok()?;
    Some(std::sync::Arc::new(img.to_rgba8()))
}

fn run() {
    application().with_assets(icons::Icons).run(|cx: &mut App| {
        let bounds = Bounds::centered(None, size(px(1100.), px(720.)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                // App identity for the taskbar: `icon` is honored on X11; on Wayland the
                // compositor matches `app_id` to a desktop entry to find the icon.
                icon: app_icon(),
                app_id: Some("hayate-office".to_string()),
                ..Default::default()
            },
            |_, cx| cx.new(|cx| HayateApp::new(cx)),
        )
        .unwrap();
        cx.on_window_closed(|cx, _| cx.quit()).detach();
        cx.activate(true);
    });
}

#[cfg(not(target_family = "wasm"))]
fn main() {
    run();
}

#[cfg(test)]
mod e2e;
