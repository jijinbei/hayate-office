//! HayateOffice presentation editor (gpui app).
//!
//! Renders a sample slide's resolved Scene onto a gpui canvas and supports basic editing:
//! click to select a shape (hit-test against the Scene), drag to move it (committed as one
//! undoable transaction). Undo/redo: Cmd/Ctrl+Z / Shift+Cmd/Ctrl+Z.

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
mod icons;
mod input;
mod io;
mod layers;
mod menu;
mod mouse;
mod paint;
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
    /// Command registry (palette / scripts / AI surface).
    registry: CommandRegistry,
    /// Command palette state when open.
    palette: Option<PaletteState>,
    /// Numeric field being typed into (rotation/position/size/opacity), if any.
    field_edit: Option<FieldEdit>,
    /// Alignment guides shown while dragging (scene/px coords relative to the slide origin).
    guides: Vec<Guide>,
    /// Whether the editing grid is shown.
    show_grid: bool,
    /// Active resize-by-handle drag, if any.
    resize: Option<ResizeDrag>,
    /// Copied shape components (Ctrl+C / Ctrl+V).
    clipboard: Option<Vec<CompValue>>,
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
    /// Active marquee (rubber-band) selection rect in scene px: (start_x, start_y, cur_x, cur_y).
    marquee: Option<(f32, f32, f32, f32)>,
    /// Last window viewport size; used to refit the slide when the window is resized.
    last_viewport: Option<gpui::Size<gpui::Pixels>>,
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum MenuTarget {
    Shape,
    Slide,
    Canvas,
}

/// Which list the left panel shows.
#[derive(Clone, Copy, PartialEq)]
enum LeftTab {
    Slides,
    Layers,
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
    buf: String,
    /// Caret/selection as a BYTE range into `buf` (caret when start == end).
    selected: Range<usize>,
    /// IME composing (marked) range, as a BYTE range into `buf`.
    marked: Option<Range<usize>>,
}

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
}

struct FieldEdit {
    kind: FieldKind,
    buf: String,
}

struct PaletteState {
    query: String,
    sel: usize,
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
            registry: hayate_core::builtins(),
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
            marquee: None,
            last_viewport: None,
        }
    }
}

fn run() {
    application().with_assets(icons::Icons).run(|cx: &mut App| {
        let bounds = Bounds::centered(None, size(px(1100.), px(720.)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
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
