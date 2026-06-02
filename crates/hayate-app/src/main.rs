//! HayateOffice presentation editor (gpui app).
//!
//! Renders a sample slide's resolved Scene onto a gpui canvas and supports basic editing:
//! click to select a shape (hit-test against the Scene), drag to move it (committed as one
//! undoable transaction). Undo/redo: Cmd/Ctrl+Z / Shift+Cmd/Ctrl+Z.

#![cfg_attr(target_family = "wasm", no_main)]

use std::cell::Cell;
use std::rc::Rc;

use gpui::{
    App, Background, Bounds, ClickEvent, Context, Font, FontStyle, FontWeight, Hsla, KeyDownEvent,
    MouseButton,
    MouseDownEvent, MouseMoveEvent, MouseUpEvent, PathBuilder, Pixels, Point, SharedString,
    TextRun, Window, WindowBounds, WindowOptions, canvas, div, point, prelude::*, px, quad, rgb,
    size,
};
use gpui_platform::application;

use hayate_ir::color::{Color, Rgba, ThemeColorToken};
use hayate_ir::font::{FontRef, ThemeFontSlot};
use hayate_ir::frac::FracIndex;
use hayate_ir::geom::{PointEmu, RectEmu, SizeEmu};
use hayate_ir::paint::Fill;
use hayate_ir::presentation::Presentation;
use hayate_ir::shape::Geometry;
use hayate_ir::text::{Paragraph, Run, TextBody};
use hayate_ir::theme::Theme;
use hayate_ir::units::{inch_f, pt};
use hayate_ir::world::{CompValue, Entity};
use hayate_model::{edit, History, Operation, Transaction};
use hayate_core::CommandRegistry;
use hayate_render::scene::{Paint, Primitive, PxRect, PxSize, ResolvedRun, Scene, TextBlock};
use hayate_render::{
    alignment_guides, build_slide_scene, grid_lines, hit_test, resize_handles, Guide, GuideKind,
};

const SELECTION: u32 = 0x3B82F6;
const DOC_PATH: &str = "hayate-sample.hayate";

/// Build a small sample deck: a title, three accent rectangles, and an ellipse.
fn sample_presentation() -> Presentation {
    let mut p = Presentation::new();
    let master = p.add_master(Theme::default());
    let layout = p.add_layout(master, "Blank");
    let slide = p.add_slide(layout);

    let title = p.add_shape(slide);
    p.world
        .frames
        .insert(title, RectEmu::new(inch_f(0.5), inch_f(0.3), inch_f(9.0), inch_f(1.0)));
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
    p.world
        .frames
        .insert(oval, RectEmu::new(inch_f(6.8), inch_f(1.8), inch_f(2.4), inch_f(1.6)));
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

/// New frame when dragging resize `handle` (TL,T,TR,R,BR,B,BL,L) by (dx,dy) EMU from `start`.
/// Axis-aligned; keeps the opposite edge fixed and clamps to a minimum size.
fn resize_frame(handle: usize, start: RectEmu, dx: i64, dy: i64) -> RectEmu {
    let min = 12_700; // 1pt
    let right0 = start.origin.x + start.size.w;
    let bottom0 = start.origin.y + start.size.h;
    let mut w = start.size.w;
    let mut h = start.size.h;
    if matches!(handle, 2 | 3 | 4) {
        w = start.size.w + dx; // right edge
    }
    if matches!(handle, 0 | 6 | 7) {
        w = start.size.w - dx; // left edge
    }
    if matches!(handle, 4 | 5 | 6) {
        h = start.size.h + dy; // bottom edge
    }
    if matches!(handle, 0 | 1 | 2) {
        h = start.size.h - dy; // top edge
    }
    w = w.max(min);
    h = h.max(min);
    let left = matches!(handle, 0 | 6 | 7);
    let top = matches!(handle, 0 | 1 | 2);
    let x = if left { right0 - w } else { start.origin.x };
    let y = if top { bottom0 - h } else { start.origin.y };
    RectEmu {
        origin: PointEmu::new(x, y),
        size: SizeEmu::new(w, h),
    }
}

/// Fill background from an Rgba, scaling alpha by `opacity` (0..1).
fn fill_bg(c: Rgba, opacity: f32) -> Background {
    gpui::Rgba {
        r: c.r as f32 / 255.0,
        g: c.g as f32 / 255.0,
        b: c.b as f32 / 255.0,
        a: (c.a as f32 / 255.0) * opacity.clamp(0.0, 1.0),
    }
    .into()
}

fn rgb_u32(c: Rgba) -> u32 {
    ((c.r as u32) << 16) | ((c.g as u32) << 8) | (c.b as u32)
}

fn hsla_of(c: Rgba) -> Hsla {
    rgb(rgb_u32(c)).into()
}

fn run_font(r: &ResolvedRun) -> Font {
    let mut f = gpui::font(r.family.clone());
    if r.bold {
        f.weight = FontWeight::BOLD;
    }
    if r.italic {
        f.style = FontStyle::Italic;
    }
    f
}

fn prim_bounds(prim: &Primitive) -> PxRect {
    match prim {
        Primitive::Quad { bounds, .. } => *bounds,
        Primitive::Ellipse { bounds, .. } => *bounds,
        Primitive::Image { bounds, .. } => *bounds,
        Primitive::Text(tb) => tb.bounds,
    }
}

struct Drag {
    entity: Entity,
    start_frame: RectEmu,
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
}

struct TextEdit {
    entity: Entity,
    original: String,
    buf: String,
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
        }
    }

    /// All currently-selected entities (primary + additional).
    fn selected_all(&self) -> Vec<Entity> {
        let mut v: Vec<Entity> = self.selection.into_iter().collect();
        v.extend(self.also.iter().copied());
        v
    }

    /// Insert a placeholder image (a Picture box) on the current slide.
    fn insert_image(&mut self) {
        let key = self.pres.add_media(b"hayate-placeholder-image".to_vec());
        let order = {
            let kids = self.pres.children(self.slide);
            let last = kids.last().and_then(|e| self.pres.world.order.get(e));
            FracIndex::after(last)
        };
        let e = self.pres.world.reserve_id();
        let frame = RectEmu::new(inch_f(3.0), inch_f(2.0), inch_f(3.0), inch_f(2.0));
        let pic = hayate_ir::image::PictureRef {
            media_key: key,
            natural: SizeEmu::new(inch_f(3.0), inch_f(2.0)),
        };
        let tx = Transaction::new(
            "insert image",
            vec![
                Operation::Spawn { entity: e },
                Operation::SetComponent { entity: e, value: CompValue::Parent(self.slide) },
                Operation::SetComponent { entity: e, value: CompValue::Order(order) },
                Operation::SetComponent { entity: e, value: CompValue::Frame(frame) },
                Operation::SetComponent { entity: e, value: CompValue::Picture(pic) },
            ],
        );
        self.commit_tx(tx);
        self.selection = Some(e);
        self.also.clear();
    }

    /// Align the current multi-selection using a registry command.
    fn align(&mut self, cmd_id: &str) {
        let ids: Vec<u64> = self.selected_all().iter().map(|e| e.0).collect();
        if ids.len() < 2 {
            return;
        }
        let args = serde_json::json!({ "entities": ids });
        if let Some(tx) = self.registry.build(cmd_id, &args, &self.pres.world) {
            self.commit_tx(tx);
        }
    }

    fn export_pptx(&self) {
        match hayate_format_pptx::export_pptx(&self.pres, "hayate-deck.pptx") {
            Ok(()) => eprintln!("exported hayate-deck.pptx"),
            Err(e) => eprintln!("pptx export error: {e}"),
        }
    }

    fn next_slide(&mut self, delta: i64) {
        let slides = self.pres.slides();
        if let Some(i) = slides.iter().position(|&s| s == self.slide) {
            let n = slides.len() as i64;
            let ni = ((i as i64 + delta) % n + n) % n;
            self.slide = slides[ni as usize];
            self.selection = None;
            self.also.clear();
            self.rebuild();
        }
    }

    fn first_run_text(&self, e: Entity) -> String {
        self.pres
            .world
            .texts
            .get(&e)
            .and_then(|tb| tb.paragraphs.first())
            .and_then(|p| p.runs.first())
            .map(|r| r.text.clone())
            .unwrap_or_default()
    }

    fn begin_text_edit(&mut self, e: Entity) {
        let original = self.first_run_text(e);
        self.text_edit = Some(TextEdit {
            entity: e,
            buf: original.clone(),
            original,
        });
    }

    /// Apply text to an entity's first run without recording history (live preview).
    fn live_set_text(&mut self, e: Entity, text: String) {
        let tx = edit::set_run_text(&self.pres.world, e, text);
        for op in tx.ops {
            op.apply(&mut self.pres.world);
        }
        self.rebuild();
    }

    fn text_key(&mut self, ev: &KeyDownEvent, cx: &mut Context<Self>) {
        let key = ev.keystroke.key.clone();
        let mut live = false;
        match key.as_str() {
            "escape" => {
                if let Some(te) = self.text_edit.take() {
                    self.live_set_text(te.entity, te.original);
                }
            }
            "enter" => {
                if let Some(te) = self.text_edit.take() {
                    // Revert the live edits, then commit the final text as one undo step.
                    self.live_set_text(te.entity, te.original);
                    let tx = edit::set_run_text(&self.pres.world, te.entity, te.buf);
                    self.commit_tx(tx);
                }
            }
            "backspace" => {
                if let Some(te) = self.text_edit.as_mut() {
                    te.buf.pop();
                }
                live = true;
            }
            "space" => {
                if let Some(te) = self.text_edit.as_mut() {
                    te.buf.push(' ');
                }
                live = true;
            }
            s if s.chars().count() == 1 => {
                if let Some(te) = self.text_edit.as_mut() {
                    te.buf.push_str(s);
                }
                live = true;
            }
            _ => {}
        }
        if live {
            if let Some(te) = &self.text_edit {
                let (e, buf) = (te.entity, te.buf.clone());
                self.live_set_text(e, buf);
            }
        }
        cx.notify();
    }

    /// Add a new text box on the current slide and start editing it.
    fn add_text_box(&mut self) {
        let order = {
            let kids = self.pres.children(self.slide);
            let last = kids.last().and_then(|e| self.pres.world.order.get(e));
            FracIndex::after(last)
        };
        let e = self.pres.world.reserve_id();
        let frame = RectEmu::new(inch_f(1.0), inch_f(3.2), inch_f(5.0), inch_f(1.0));
        let body = TextBody {
            paragraphs: vec![Paragraph::new(vec![Run {
                text: "Text".to_string(),
                font: FontRef::Theme(ThemeFontSlot::Minor),
                size: pt(24),
                color: Color::theme(ThemeColorToken::Dk1),
                bold: false,
                italic: false,
                underline: false,
            }])],
            autofit: false,
        };
        let tx = Transaction::new(
            "add text",
            vec![
                Operation::Spawn { entity: e },
                Operation::SetComponent { entity: e, value: CompValue::Parent(self.slide) },
                Operation::SetComponent { entity: e, value: CompValue::Order(order) },
                Operation::SetComponent { entity: e, value: CompValue::Frame(frame) },
                Operation::SetComponent { entity: e, value: CompValue::Text(body) },
            ],
        );
        self.commit_tx(tx);
        self.selection = Some(e);
        self.begin_text_edit(e);
    }

    fn set_zoom(&mut self, z: f32, cx: &mut Context<Self>) {
        self.zoom = z.clamp(0.1, 8.0);
        self.rebuild();
        cx.notify();
    }

    /// Set zoom so the slide fits the current window's editing area.
    fn fit_zoom(&mut self, window: &Window) {
        let vp = window.viewport_size();
        let inspector_w = if self.selection.is_some() { 244.0 } else { 0.0 };
        let avail_w = (f32::from(vp.width) - 244.0 - inspector_w - 24.0).max(64.0);
        let avail_h = (f32::from(vp.height) - 96.0).max(64.0);
        let pt = |v: i64| (v as f32 / 12_700.0).max(1.0);
        let z = (avail_w / pt(self.pres.slide_size.w))
            .min(avail_h / pt(self.pres.slide_size.h))
            .clamp(0.1, 8.0);
        self.zoom = z;
    }

    /// Handle a key while a numeric field is being edited (digits / . / -).
    fn field_key(&mut self, ev: &KeyDownEvent, cx: &mut Context<Self>) {
        let key = ev.keystroke.key.clone();
        match key.as_str() {
            "escape" => self.field_edit = None,
            "enter" => {
                if let Some(fe) = self.field_edit.take() {
                    if let Ok(v) = fe.buf.trim().parse::<f32>() {
                        match fe.kind {
                            FieldKind::Rotation => self.set_rotation_abs(v.rem_euclid(360.0)),
                            FieldKind::PosX => self.set_frame_field(|f| f.origin.x = pt_to_emu(v)),
                            FieldKind::PosY => self.set_frame_field(|f| f.origin.y = pt_to_emu(v)),
                            FieldKind::SizeW => self.set_frame_field(|f| f.size.w = pt_to_emu(v).max(12_700)),
                            FieldKind::SizeH => self.set_frame_field(|f| f.size.h = pt_to_emu(v).max(12_700)),
                            FieldKind::Opacity => self.set_opacity_pct(v),
                        }
                    }
                }
            }
            "backspace" => {
                if let Some(fe) = self.field_edit.as_mut() {
                    fe.buf.pop();
                }
            }
            s if s.len() == 1 && (s.chars().all(|c| c.is_ascii_digit()) || s == "." || s == "-") => {
                if let Some(fe) = self.field_edit.as_mut() {
                    if fe.buf.len() < 8 {
                        fe.buf.push_str(s);
                    }
                }
            }
            _ => {}
        }
        cx.notify();
    }

    fn set_frame_field(&mut self, f: impl FnOnce(&mut RectEmu)) {
        if let Some(e) = self.selection {
            if let Some(mut fr) = self.pres.world.frames.get(&e).copied() {
                f(&mut fr);
                let tx = edit::set_frame(e, fr);
                self.commit_tx(tx);
            }
        }
    }

    fn set_opacity_pct(&mut self, pct: f32) {
        if let Some(e) = self.selection {
            let v = (pct / 100.0).clamp(0.0, 1.0);
            let tx = Transaction::new(
                "set opacity",
                vec![Operation::SetComponent { entity: e, value: CompValue::Opacity(v) }],
            );
            self.commit_tx(tx);
        }
    }

    fn field_current(&self, kind: FieldKind) -> String {
        let frame = self.selection.and_then(|e| self.pres.world.frames.get(&e).copied());
        let to_pt = |v: i64| (v as f32 / 12_700.0).round() as i32;
        match kind {
            FieldKind::Rotation => format!("{}", self.sel_rotation().round() as i32),
            FieldKind::PosX => frame.map(|f| to_pt(f.origin.x)).unwrap_or(0).to_string(),
            FieldKind::PosY => frame.map(|f| to_pt(f.origin.y)).unwrap_or(0).to_string(),
            FieldKind::SizeW => frame.map(|f| to_pt(f.size.w)).unwrap_or(0).to_string(),
            FieldKind::SizeH => frame.map(|f| to_pt(f.size.h)).unwrap_or(0).to_string(),
            FieldKind::Opacity => {
                let o = self
                    .selection
                    .and_then(|e| self.pres.world.opacity.get(&e).copied())
                    .unwrap_or(1.0);
                format!("{}", (o * 100.0).round() as i32)
            }
        }
    }

    fn begin_field_edit(&mut self, kind: FieldKind) {
        self.field_edit = Some(FieldEdit {
            kind,
            buf: self.field_current(kind),
        });
    }

    /// A clickable numeric field (click to type; shows the current value otherwise).
    fn num_field(&self, id: &'static str, kind: FieldKind, cx: &mut Context<Self>) -> impl IntoElement {
        let editing = matches!(&self.field_edit, Some(fe) if fe.kind == kind);
        let shown = if editing {
            format!(
                "{}|",
                self.field_edit.as_ref().map(|f| f.buf.clone()).unwrap_or_default()
            )
        } else {
            self.field_current(kind)
        };
        div()
            .id(id)
            .px_2()
            .py_1()
            .rounded_md()
            .bg(if editing { rgb(0x1f3a5f) } else { rgb(0x3a3a3a) })
            .child(shown)
            .on_click(cx.listener(move |this, _ev: &ClickEvent, _w, cx| {
                this.begin_field_edit(kind);
                cx.notify();
            }))
    }

    /// Commands matching the palette query, as (id, title).
    fn palette_commands(&self) -> Vec<(String, String)> {
        let q = self
            .palette
            .as_ref()
            .map(|p| p.query.to_lowercase())
            .unwrap_or_default();
        self.registry
            .manifest()
            .into_iter()
            .filter_map(|v| {
                let id = v.get("id")?.as_str()?.to_string();
                let title = v
                    .get("title")
                    .and_then(|t| t.as_str())
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| id.clone());
                if q.is_empty() || id.to_lowercase().contains(&q) || title.to_lowercase().contains(&q) {
                    Some((id, title))
                } else {
                    None
                }
            })
            .collect()
    }

    /// Run a registered command, supplying the current selection + sensible defaults as args.
    fn run_command(&mut self, id: &str) {
        let args = serde_json::json!({
            "entity": self.selection.map(|e| e.0),
            "dx": 200,
            "dy": 0,
            "color": "#e11d48",
        });
        if let Some(tx) = self.registry.build(id, &args, &self.pres.world) {
            self.history.commit(&mut self.pres.world, tx);
        }
    }

    fn palette_key(&mut self, ev: &KeyDownEvent, cx: &mut Context<Self>) {
        let key = ev.keystroke.key.clone();
        match key.as_str() {
            "escape" => self.palette = None,
            "enter" => {
                let sel = self.palette.as_ref().map(|p| p.sel).unwrap_or(0);
                let chosen = self.palette_commands().get(sel).map(|(id, _)| id.clone());
                self.palette = None;
                if let Some(id) = chosen {
                    self.run_command(&id);
                    self.rebuild();
                }
            }
            "backspace" => {
                if let Some(p) = self.palette.as_mut() {
                    p.query.pop();
                    p.sel = 0;
                }
            }
            "up" => {
                if let Some(p) = self.palette.as_mut() {
                    p.sel = p.sel.saturating_sub(1);
                }
            }
            "down" => {
                let n = self.palette_commands().len();
                if let Some(p) = self.palette.as_mut() {
                    if p.sel + 1 < n {
                        p.sel += 1;
                    }
                }
            }
            "space" => {
                if let Some(p) = self.palette.as_mut() {
                    p.query.push(' ');
                    p.sel = 0;
                }
            }
            s if s.chars().count() == 1 => {
                if let Some(p) = self.palette.as_mut() {
                    p.query.push_str(s);
                    p.sel = 0;
                }
            }
            _ => {}
        }
        cx.notify();
    }

    fn rebuild(&mut self) {
        let target = view_px(&self.pres, self.zoom);
        self.scene = build_slide_scene(&self.pres, self.slide, target);
    }

    /// Pixels per EMU (width-fit).
    fn scale(&self) -> f64 {
        self.scene.size.w as f64 / self.pres.slide_size.w.max(1) as f64
    }

    fn on_mouse_down(&mut self, ev: &MouseDownEvent, cx: &mut Context<Self>) {
        let o = self.canvas_origin.get();
        let x = f32::from(ev.position.x - o.x);
        let y = f32::from(ev.position.y - o.y);
        // Grab a resize handle on the current (axis-aligned) selection?
        if let Some(sel) = self.selection {
            if let Some(node) = self.scene.nodes.iter().find(|n| n.source == Some(sel)) {
                if node.rotation_deg.abs() < 1e-3 {
                    let r = prim_bounds(&node.prim);
                    for (i, (hx, hy)) in resize_handles(r, 0.0).iter().enumerate() {
                        if (x - hx).hypot(y - hy) < 8.0 {
                            if let Some(f) = self.pres.world.frames.get(&sel).copied() {
                                self.resize = Some(ResizeDrag {
                                    handle: i,
                                    start_frame: f,
                                    start_cursor: ev.position,
                                });
                                cx.notify();
                                return;
                            }
                        }
                    }
                }
            }
        }
        let hit = hit_test(&self.scene, x, y);
        if ev.modifiers.shift {
            // Shift-click adds/keeps a multi-selection (no drag).
            if let Some(h) = hit {
                if Some(h) != self.selection && !self.also.contains(&h) {
                    self.also.push(h);
                }
            }
            cx.notify();
            return;
        }
        self.also.clear();
        self.selection = hit;
        self.drag = self.selection.and_then(|e| {
            self.pres.world.frames.get(&e).map(|f| Drag {
                entity: e,
                start_frame: *f,
                start_cursor: ev.position,
            })
        });
        cx.notify();
    }

    fn on_mouse_move(&mut self, ev: &MouseMoveEvent, cx: &mut Context<Self>) {
        if let Some(rd) = &self.resize {
            let scale = self.scale();
            if scale <= 0.0 {
                return;
            }
            let dx = (f32::from(ev.position.x - rd.start_cursor.x) as f64 / scale) as i64;
            let dy = (f32::from(ev.position.y - rd.start_cursor.y) as f64 / scale) as i64;
            let nf = resize_frame(rd.handle, rd.start_frame, dx, dy);
            if let Some(e) = self.selection {
                self.pres.world.frames.insert(e, nf);
                self.rebuild();
                cx.notify();
            }
            return;
        }
        let Some(d) = &self.drag else { return };
        let scale = self.scale();
        if scale <= 0.0 {
            return;
        }
        let dx = (f32::from(ev.position.x - d.start_cursor.x) as f64 / scale) as i64;
        let dy = (f32::from(ev.position.y - d.start_cursor.y) as f64 / scale) as i64;
        let nf = RectEmu {
            origin: PointEmu::new(d.start_frame.origin.x + dx, d.start_frame.origin.y + dy),
            size: d.start_frame.size,
        };
        let e = d.entity;
        self.pres.world.frames.insert(e, nf); // live preview, no history
        self.rebuild();
        self.update_guides(e);
        cx.notify();
    }

    /// Recompute alignment guides for the moving shape against the others (scene px coords).
    fn update_guides(&mut self, moving_entity: Entity) {
        let mut moving = None;
        let mut others = Vec::new();
        for n in &self.scene.nodes {
            let r = prim_bounds(&n.prim);
            if n.source == Some(moving_entity) {
                moving = Some(r);
            } else {
                others.push(r);
            }
        }
        self.guides = match moving {
            Some(m) => alignment_guides(m, &others, 6.0),
            None => Vec::new(),
        };
    }

    fn on_mouse_up(&mut self, _ev: &MouseUpEvent, cx: &mut Context<Self>) {
        if let Some(rd) = self.resize.take() {
            if let Some(e) = self.selection {
                if let Some(final_f) = self.pres.world.frames.get(&e).copied() {
                    if final_f != rd.start_frame {
                        self.pres.world.frames.insert(e, rd.start_frame);
                        let tx = edit::set_frame(e, final_f);
                        self.commit_tx(tx);
                    }
                }
            }
            cx.notify();
            return;
        }
        self.guides.clear();
        let Some(d) = self.drag.take() else { return };
        let Some(final_f) = self.pres.world.frames.get(&d.entity).copied() else {
            return;
        };
        if final_f != d.start_frame {
            // Revert to the start, then commit the whole move as one undoable step.
            self.pres.world.frames.insert(d.entity, d.start_frame);
            let tx = edit::set_frame(d.entity, final_f);
            self.commit_tx(tx);
        }
        cx.notify();
    }

    fn on_key_down(&mut self, ev: &KeyDownEvent, cx: &mut Context<Self>) {
        if self.palette.is_some() {
            self.palette_key(ev, cx);
            return;
        }
        if self.field_edit.is_some() {
            self.field_key(ev, cx);
            return;
        }
        if self.text_edit.is_some() {
            self.text_key(ev, cx);
            return;
        }
        if self.present {
            match ev.keystroke.key.as_str() {
                "escape" => {
                    self.present = false;
                    cx.notify();
                }
                "right" | "space" | "down" => self.next_slide(1),
                "left" | "up" => self.next_slide(-1),
                _ => {}
            }
            return;
        }
        let k = &ev.keystroke;
        let cmd = k.modifiers.platform || k.modifiers.control;
        match k.key.as_str() {
            "p" if cmd => {
                self.palette = Some(PaletteState { query: String::new(), sel: 0 });
                cx.notify();
            }
            "e" if cmd && k.modifiers.shift => self.export_pptx(),
            "e" if cmd => self.export_svg(),
            "f5" => {
                self.present = true;
                cx.notify();
            }
            "i" if !cmd => {
                self.insert_image();
                cx.notify();
            }
            "z" if cmd && k.modifiers.shift => {
                self.history.redo(&mut self.pres.world);
                self.after_doc_change();
                cx.notify();
            }
            "z" if cmd => {
                self.history.undo(&mut self.pres.world);
                self.after_doc_change();
                cx.notify();
            }
            "s" if cmd => self.save(),
            "o" if cmd => {
                self.open();
                cx.notify();
            }
            "g" if !cmd => {
                self.show_grid = !self.show_grid;
                cx.notify();
            }
            "r" if !cmd => {
                self.add_rect();
                cx.notify();
            }
            "t" if !cmd => {
                self.add_text_box();
                cx.notify();
            }
            "f2" => {
                if let Some(e) = self.selection {
                    self.begin_text_edit(e);
                    cx.notify();
                }
            }
            "d" if cmd => {
                self.duplicate_selection();
                cx.notify();
            }
            "c" if cmd => self.copy_selection(),
            "v" if cmd => {
                self.paste_clipboard();
                cx.notify();
            }
            "delete" | "backspace" if !cmd => {
                self.delete_selection();
                cx.notify();
            }
            _ => {}
        }
    }

    /// Rebuild + autosave after a document change (undo/redo, etc.).
    fn after_doc_change(&mut self) {
        self.rebuild();
        let _ = hayate_format::autosave(&self.pres, DOC_PATH);
    }

    /// Export the current slide to an SVG file next to the app.
    fn export_svg(&self) {
        let svg = hayate_render::export_svg(&self.scene);
        match std::fs::write("hayate-slide.svg", svg) {
            Ok(()) => eprintln!("exported hayate-slide.svg"),
            Err(e) => eprintln!("svg export error: {e}"),
        }
    }

    /// Add a rectangle at the slide center as one undoable transaction, and select it.
    fn add_rect(&mut self) {
        let order = {
            let kids = self.pres.children(self.slide);
            let last = kids.last().and_then(|e| self.pres.world.order.get(e));
            FracIndex::after(last)
        };
        let e = self.pres.world.reserve_id();
        let frame = RectEmu::new(inch_f(4.0), inch_f(3.5), inch_f(1.6), inch_f(1.6));
        let tx = edit::create_rect(
            e,
            self.slide,
            order,
            frame,
            Fill::Solid(Color::theme(ThemeColorToken::Accent5)),
        );
        self.commit_tx(tx);
        self.selection = Some(e);
    }

    /// Delete the selected shape (undoable: despawn captures components to restore).
    fn delete_selection(&mut self) {
        if let Some(e) = self.selection.take() {
            let tx = Transaction::new("delete shape", vec![Operation::Despawn { entity: e }]);
            self.commit_tx(tx);
        }
    }

    fn save(&self) {
        match hayate_format::save(&self.pres, DOC_PATH) {
            Ok(()) => eprintln!("saved to {DOC_PATH}"),
            Err(e) => eprintln!("save error: {e}"),
        }
    }

    /// Add a new slide based on the current slide's layout and switch to it.
    fn add_slide(&mut self) {
        if let Some(layout) = self.pres.world.slide_info.get(&self.slide).map(|s| s.layout) {
            let s = self.pres.add_slide(layout);
            self.slide = s;
            self.selection = None;
            self.rebuild();
        }
    }

    /// Duplicate the current slide (copying its shapes) and switch to the copy.
    fn duplicate_slide(&mut self) {
        let Some(layout) = self.pres.world.slide_info.get(&self.slide).map(|s| s.layout) else {
            return;
        };
        let children = self.pres.children(self.slide);
        let new_slide = self.pres.add_slide(layout);
        for c in children {
            let comps = self.pres.world.components_of(c);
            let ne = self.pres.world.reserve_id();
            self.pres.world.spawn_at(ne);
            for comp in comps {
                let comp = match comp {
                    CompValue::Parent(_) => CompValue::Parent(new_slide),
                    other => other,
                };
                self.pres.world.set(ne, comp);
            }
        }
        self.slide = new_slide;
        self.selection = None;
        self.rebuild();
        let _ = hayate_format::autosave(&self.pres, DOC_PATH);
    }

    /// Delete the current slide (keeps at least one slide).
    fn delete_slide(&mut self) {
        if self.pres.slides().len() <= 1 {
            return;
        }
        for c in self.pres.children(self.slide) {
            self.pres.world.despawn(c);
        }
        self.pres.world.despawn(self.slide);
        self.slide = self.pres.slides().first().copied().unwrap_or(self.slide);
        self.selection = None;
        self.rebuild();
        let _ = hayate_format::autosave(&self.pres, DOC_PATH);
    }

    // --- inspector (Format pane) actions ---

    fn commit_tx(&mut self, tx: Transaction) {
        if !tx.ops.is_empty() {
            self.history.commit(&mut self.pres.world, tx);
            self.rebuild();
            let _ = hayate_format::autosave(&self.pres, DOC_PATH);
        }
    }

    fn sel_rotation(&self) -> f32 {
        self.selection
            .and_then(|e| self.pres.world.rotations.get(&e).copied())
            .unwrap_or(0.0)
    }

    fn rotate_by(&mut self, delta: f32) {
        if let Some(e) = self.selection {
            let cur = self.pres.world.rotations.get(&e).copied().unwrap_or(0.0);
            let tx = edit::set_rotation(e, cur + delta);
            self.commit_tx(tx);
        }
    }

    fn set_rotation_abs(&mut self, deg: f32) {
        if let Some(e) = self.selection {
            let tx = edit::set_rotation(e, deg);
            self.commit_tx(tx);
        }
    }

    fn nudge(&mut self, dx: i64, dy: i64) {
        if let Some(e) = self.selection {
            let tx = edit::translate(&self.pres.world, e, dx, dy);
            self.commit_tx(tx);
        }
    }

    fn resize_by(&mut self, dw: i64, dh: i64) {
        if let Some(e) = self.selection {
            if let Some(f) = self.pres.world.frames.get(&e).copied() {
                let nw = (f.size.w + dw).max(91_440);
                let nh = (f.size.h + dh).max(91_440);
                let tx = edit::resize(&self.pres.world, e, nw, nh);
                self.commit_tx(tx);
            }
        }
    }

    fn set_fill_accent(&mut self, token: ThemeColorToken) {
        if let Some(e) = self.selection {
            let tx = edit::set_fill(e, Fill::Solid(Color::theme(token)));
            self.commit_tx(tx);
        }
    }

    fn run_on_selection(&mut self, id: &str) {
        if let Some(e) = self.selection {
            let args = serde_json::json!({ "entity": e.0 });
            if let Some(tx) = self.registry.build(id, &args, &self.pres.world) {
                self.commit_tx(tx);
            }
        }
    }

    fn duplicate_selection(&mut self) {
        if let Some(src) = self.selection {
            let ne = self.pres.world.reserve_id();
            let tx = edit::duplicate(&self.pres.world, src, ne);
            self.commit_tx(tx);
            self.selection = Some(ne);
        }
    }

    fn copy_selection(&mut self) {
        self.clipboard = self.selection.map(|e| self.pres.world.components_of(e));
    }

    fn paste_clipboard(&mut self) {
        let Some(comps) = self.clipboard.clone() else {
            return;
        };
        let order = {
            let kids = self.pres.children(self.slide);
            let last = kids.last().and_then(|e| self.pres.world.order.get(e));
            FracIndex::after(last)
        };
        let ne = self.pres.world.reserve_id();
        let mut ops = vec![Operation::Spawn { entity: ne }];
        for comp in comps {
            let comp = match comp {
                CompValue::Frame(f) => CompValue::Frame(RectEmu {
                    origin: PointEmu::new(f.origin.x + 182_880, f.origin.y + 182_880),
                    size: f.size,
                }),
                other => other,
            };
            ops.push(Operation::SetComponent { entity: ne, value: comp });
        }
        // Ensure it lands on the current slide, appended in order.
        ops.push(Operation::SetComponent { entity: ne, value: CompValue::Parent(self.slide) });
        ops.push(Operation::SetComponent { entity: ne, value: CompValue::Order(order) });
        self.commit_tx(Transaction::new("paste", ops));
        self.selection = Some(ne);
    }

    fn open(&mut self) {
        match hayate_format::load(DOC_PATH) {
            Ok(p) => {
                self.pres = p;
                self.slide = self.pres.slides().first().copied().unwrap_or(self.slide);
                self.history = History::new();
                self.selection = None;
                self.rebuild();
                eprintln!("opened {DOC_PATH}");
            }
            Err(e) => eprintln!("open error: {e}"),
        }
    }
}

fn paint_text(tb: &TextBlock, ox: Pixels, oy: Pixels, window: &mut Window, cx: &mut App) {
    use hayate_ir::text::HAlign;
    let left = ox + px(tb.bounds.x);
    let mut top = oy + px(tb.bounds.y);
    for para in &tb.paragraphs {
        if para.runs.is_empty() {
            continue;
        }
        let align = match para.align {
            HAlign::Center => gpui::TextAlign::Center,
            HAlign::Right => gpui::TextAlign::Right,
            HAlign::Left | HAlign::Justify => gpui::TextAlign::Left,
        };
        let font_size = px(para.runs.iter().map(|r| r.size_px).fold(0.0, f32::max));
        let line_height = font_size * 1.3;

        let mut text = String::new();
        let mut runs: Vec<TextRun> = Vec::new();
        for r in &para.runs {
            let len = r.text.len();
            if len == 0 {
                continue;
            }
            text.push_str(&r.text);
            runs.push(TextRun {
                len,
                font: run_font(r),
                color: hsla_of(r.color),
                background_color: None,
                underline: None,
                strikethrough: None,
            });
        }
        if runs.is_empty() {
            continue;
        }
        let shaped = window
            .text_system()
            .shape_line(SharedString::from(text), font_size, &runs, None);
        let _ = shaped.paint(point(left, top), line_height, align, None, window, cx);
        top += line_height;
    }
}

/// A small clickable toolbar button.
/// Rotate point (x,y) around center (cx,cy) by `rad` radians (clockwise in screen coords).
fn rotate_pt(x: f32, y: f32, cx: f32, cy: f32, rad: f32) -> (f32, f32) {
    let (s, c) = rad.sin_cos();
    let dx = x - cx;
    let dy = y - cy;
    (cx + dx * c - dy * s, cy + dx * s + dy * c)
}

/// Paint a Scene's background and shapes at `o` (window coords). Shared by the main view and
/// the slide-list thumbnails. Rotated shapes are drawn as paths (quads carry no transform).
fn paint_scene(scene: &Scene, o: Point<Pixels>, window: &mut Window, cx: &mut App) {
    let bg: Background = rgb(rgb_u32(scene.background)).into();
    window.paint_quad(quad(
        Bounds {
            origin: o,
            size: size(px(scene.size.w), px(scene.size.h)),
        },
        px(0.),
        bg,
        px(0.),
        gpui::transparent_black(),
        Default::default(),
    ));

    for node in &scene.nodes {
        let angle = node.rotation_deg.to_radians();
        let opacity = node.opacity;
        match &node.prim {
            Primitive::Quad { bounds: r, corner_radius, fill: Some(Paint::Solid(c)), .. } => {
                if angle.abs() < 1e-3 {
                    let b = Bounds {
                        origin: point(o.x + px(r.x), o.y + px(r.y)),
                        size: size(px(r.w), px(r.h)),
                    };
                    window.paint_quad(quad(
                        b,
                        px(*corner_radius),
                        fill_bg(*c, opacity),
                        px(0.),
                        gpui::transparent_black(),
                        Default::default(),
                    ));
                } else {
                    let (cx_, cy_) = (r.x + r.w / 2.0, r.y + r.h / 2.0);
                    let corners = [
                        (r.x, r.y),
                        (r.x + r.w, r.y),
                        (r.x + r.w, r.y + r.h),
                        (r.x, r.y + r.h),
                    ];
                    let mut b = PathBuilder::fill();
                    for (i, (cxp, cyp)) in corners.iter().enumerate() {
                        let (gx, gy) = rotate_pt(*cxp, *cyp, cx_, cy_, angle);
                        let p = point(o.x + px(gx), o.y + px(gy));
                        if i == 0 {
                            b.move_to(p);
                        } else {
                            b.line_to(p);
                        }
                    }
                    b.close();
                    if let Ok(path) = b.build() {
                        window.paint_path(path, fill_bg(*c, opacity));
                    }
                }
            }
            Primitive::Ellipse { bounds: r, fill: Some(Paint::Solid(c)), .. } => {
                let (cx_, cy_) = (r.x + r.w / 2.0, r.y + r.h / 2.0);
                let (rx, ry) = (r.w / 2.0, r.h / 2.0);
                let mut b = PathBuilder::fill();
                let n = 48;
                for i in 0..n {
                    let th = (i as f32) / (n as f32) * std::f32::consts::TAU;
                    let (ex, ey) = (cx_ + rx * th.cos(), cy_ + ry * th.sin());
                    let (gx, gy) = rotate_pt(ex, ey, cx_, cy_, angle);
                    let p = point(o.x + px(gx), o.y + px(gy));
                    if i == 0 {
                        b.move_to(p);
                    } else {
                        b.line_to(p);
                    }
                }
                b.close();
                if let Ok(path) = b.build() {
                    window.paint_path(path, rgb(rgb_u32(*c)));
                }
            }
            Primitive::Image { bounds: r, .. } => {
                // Placeholder: a light-gray box (real image decoding is a follow-up).
                let b = Bounds {
                    origin: point(o.x + px(r.x), o.y + px(r.y)),
                    size: size(px(r.w), px(r.h)),
                };
                window.paint_quad(quad(
                    b,
                    px(0.),
                    Background::from(rgb(0xCCCCCC)),
                    px(1.),
                    rgb(0x888888),
                    Default::default(),
                ));
            }
            Primitive::Text(tb) => paint_text(tb, o.x, o.y, window, cx),
            _ => {}
        }
    }
}

fn tool_button(
    id: &'static str,
    label: impl Into<SharedString>,
    cx: &mut Context<HayateApp>,
    action: impl Fn(&mut HayateApp, &mut Window, &mut Context<HayateApp>) + 'static,
) -> impl IntoElement {
    div()
        .id(id)
        .px_2()
        .py_1()
        .bg(rgb(0x3a3a3a))
        .rounded_md()
        .hover(|s| s.bg(rgb(0x4a4a4a)))
        .child(label.into())
        .on_click(cx.listener(move |this, _ev: &ClickEvent, window, cx| action(this, window, cx)))
}

impl Render for HayateApp {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.focused_once {
            window.focus(&self.focus, cx);
            self.focused_once = true;
        }

        // Fullscreen presentation mode: the slide fit to the whole window, no panels.
        if self.present {
            let vp = window.viewport_size();
            let target = PxSize {
                w: f32::from(vp.width),
                h: f32::from(vp.height),
            };
            let pscene = build_slide_scene(&self.pres, self.slide, target);
            let (pw, ph) = (pscene.size.w, pscene.size.h);
            let pcanvas = canvas(
                |_, _, _| {},
                move |b, _, window, cx| paint_scene(&pscene, b.origin, window, cx),
            )
            .size_full();
            return div()
                .track_focus(&self.focus)
                .on_key_down(cx.listener(|this, ev: &KeyDownEvent, _, cx| this.on_key_down(ev, cx)))
                .size_full()
                .bg(rgb(0x000000))
                .flex()
                .items_center()
                .justify_center()
                .child(div().w(px(pw)).h(px(ph)).child(pcanvas))
                .into_any_element();
        }

        // Document coordinates are absolute (points); on-screen size = slide_pt * zoom.
        // Window resizing does not change zoom (use the zoom controls / Fit).
        let scene = self.scene.clone();
        let selection = self.selection;
        let also = self.also.clone();
        let guides = self.guides.clone();
        let show_grid = self.show_grid;
        let origin_cell = self.canvas_origin.clone();
        let (sw, sh) = (scene.size.w, scene.size.h);
        let title: SharedString = format!(
            "HayateOffice — Ctrl+P palette · R add · Del delete · Ctrl+Z undo · Ctrl+S/O save/open ({} shapes)",
            scene.nodes.len()
        )
        .into();

        let palette_panel = self.palette.as_ref().map(|p| {
            let list = self.palette_commands();
            let sel = p.sel;
            let mut col = div()
                .flex()
                .flex_col()
                .gap_1()
                .w(px(560.))
                .bg(rgb(0x2a2a2a))
                .border_1()
                .border_color(rgb(SELECTION));
            col = col.child(div().bg(rgb(0x111111)).child(format!("\u{203a} {}", p.query)));
            for (i, (_id, t)) in list.iter().enumerate() {
                let row = div().child(t.clone());
                let row = if i == sel {
                    row.bg(rgb(SELECTION)).text_color(rgb(0xffffff))
                } else {
                    row
                };
                col = col.child(row);
            }
            col
        });

        let slide_canvas = canvas(
            |_, _, _| {},
            move |bounds, _, window, cx| {
                let o = bounds.origin;
                origin_cell.set(o);

                paint_scene(&scene, o, window, cx);

                if show_grid {
                    let g = grid_lines(scene.size, scene.size.w / 16.0);
                    let gc = rgb(0xD0D0D0);
                    for x in g.vertical {
                        window.paint_quad(quad(
                            Bounds { origin: point(o.x + px(x), o.y), size: size(px(1.0), px(scene.size.h)) },
                            px(0.),
                            Background::from(gc),
                            px(0.),
                            gpui::transparent_black(),
                            Default::default(),
                        ));
                    }
                    for y in g.horizontal {
                        window.paint_quad(quad(
                            Bounds { origin: point(o.x, o.y + px(y)), size: size(px(scene.size.w), px(1.0)) },
                            px(0.),
                            Background::from(gc),
                            px(0.),
                            gpui::transparent_black(),
                            Default::default(),
                        ));
                    }
                }

                // Selection outline (drawn on top), rotated to match the shape.
                if let Some(sel) = selection {
                    if let Some(node) = scene.nodes.iter().find(|n| n.source == Some(sel)) {
                        let r = prim_bounds(&node.prim);
                        let angle = node.rotation_deg.to_radians();
                        let pad = 2.0;
                        if angle.abs() < 1e-3 {
                            let b = Bounds {
                                origin: point(o.x + px(r.x - pad), o.y + px(r.y - pad)),
                                size: size(px(r.w + 2.0 * pad), px(r.h + 2.0 * pad)),
                            };
                            window.paint_quad(quad(
                                b,
                                px(0.),
                                gpui::transparent_black(),
                                px(2.),
                                rgb(SELECTION),
                                Default::default(),
                            ));
                        } else {
                            let (cx_, cy_) = (r.x + r.w / 2.0, r.y + r.h / 2.0);
                            let corners = [
                                (r.x - pad, r.y - pad),
                                (r.x + r.w + pad, r.y - pad),
                                (r.x + r.w + pad, r.y + r.h + pad),
                                (r.x - pad, r.y + r.h + pad),
                                (r.x - pad, r.y - pad), // close the loop for the stroke
                            ];
                            let mut sb = PathBuilder::stroke(px(2.));
                            for (i, (xx, yy)) in corners.iter().enumerate() {
                                let (gx, gy) = rotate_pt(*xx, *yy, cx_, cy_, angle);
                                let p = point(o.x + px(gx), o.y + px(gy));
                                if i == 0 {
                                    sb.move_to(p);
                                } else {
                                    sb.line_to(p);
                                }
                            }
                            if let Ok(path) = sb.build() {
                                window.paint_path(path, rgb(SELECTION));
                            }
                        }
                    }
                }
                // Additional multi-selection outlines.
                for ae in &also {
                    if let Some(node) = scene.nodes.iter().find(|n| n.source == Some(*ae)) {
                        let r = prim_bounds(&node.prim);
                        let b = Bounds {
                            origin: point(o.x + px(r.x - 2.0), o.y + px(r.y - 2.0)),
                            size: size(px(r.w + 4.0), px(r.h + 4.0)),
                        };
                        window.paint_quad(quad(
                            b,
                            px(0.),
                            gpui::transparent_black(),
                            px(2.),
                            rgb(0x60A5FA),
                            Default::default(),
                        ));
                    }
                }

                // Resize handles on the selection.
                if let Some(sel) = selection {
                    if let Some(node) = scene.nodes.iter().find(|n| n.source == Some(sel)) {
                        let r = prim_bounds(&node.prim);
                        for (hx, hy) in resize_handles(r, node.rotation_deg) {
                            let b = Bounds {
                                origin: point(o.x + px(hx - 4.0), o.y + px(hy - 4.0)),
                                size: size(px(8.0), px(8.0)),
                            };
                            window.paint_quad(quad(
                                b,
                                px(1.0),
                                Background::from(rgb(0xffffff)),
                                px(1.0),
                                rgb(SELECTION),
                                Default::default(),
                            ));
                        }
                    }
                }
                // Smart alignment guides (drawn while dragging).
                for g in &guides {
                    let color = Background::from(rgb(0xFF3DAA));
                    match g.kind {
                        GuideKind::Vertical => window.paint_quad(quad(
                            Bounds {
                                origin: point(o.x + px(g.pos - 0.5), o.y),
                                size: size(px(1.0), px(scene.size.h)),
                            },
                            px(0.),
                            color,
                            px(0.),
                            gpui::transparent_black(),
                            Default::default(),
                        )),
                        GuideKind::Horizontal => window.paint_quad(quad(
                            Bounds {
                                origin: point(o.x, o.y + px(g.pos - 0.5)),
                                size: size(px(scene.size.w), px(1.0)),
                            },
                            px(0.),
                            color,
                            px(0.),
                            gpui::transparent_black(),
                            Default::default(),
                        )),
                    }
                }
            },
        )
        .size_full();

        // Slide-list sidebar: a clickable thumbnail per slide + an "add slide" button.
        let slides = self.pres.slides();
        let current = self.slide;
        let mut sidebar = div()
            .flex()
            .flex_col()
            .gap_2()
            .w(px(208.))
            .p_2()
            .bg(rgb(0x252525));
        sidebar = sidebar.child(tool_button("add_slide", "+ Slide", cx, |this, _w, cx| {
            this.add_slide();
            cx.notify();
        }));
        sidebar = sidebar.child(
            div()
                .flex()
                .flex_row()
                .gap_2()
                .child(tool_button("dup_slide", "Dup", cx, |t, _w, cx| {
                    t.duplicate_slide();
                    cx.notify();
                }))
                .child(tool_button("del_slide", "Del", cx, |t, _w, cx| {
                    t.delete_slide();
                    cx.notify();
                })),
        );
        for (i, &s) in slides.iter().enumerate() {
            let tscene = build_slide_scene(&self.pres, s, PxSize { w: 176.0, h: 99.0 });
            let is_cur = s == current;
            let tcanvas = canvas(
                |_, _, _| {},
                move |b, _, window, cx| paint_scene(&tscene, b.origin, window, cx),
            )
            .size_full();
            sidebar = sidebar.child(
                div()
                    .id(("slide", i))
                    .w(px(176.))
                    .h(px(99.))
                    .border_2()
                    .border_color(if is_cur { rgb(SELECTION) } else { rgb(0x444444) })
                    .child(tcanvas)
                    .on_click(cx.listener(move |this, _ev: &ClickEvent, _w, cx| {
                        this.slide = s;
                        this.selection = None;
                        this.rebuild();
                        cx.notify();
                    })),
            );
        }

        // Format (properties) pane for the selected shape — PowerPoint-style.
        let accents = [
            ThemeColorToken::Accent1,
            ThemeColorToken::Accent2,
            ThemeColorToken::Accent3,
            ThemeColorToken::Accent4,
            ThemeColorToken::Accent5,
            ThemeColorToken::Accent6,
        ];
        let theme = self.pres.theme_of(self.slide).cloned().unwrap_or_default();
        let inspector = self.selection.map(|_e| {
            let mut swatches = div().flex().flex_row().gap_1();
            for (i, t) in accents.into_iter().enumerate() {
                let cu = rgb_u32(theme.color_for(t));
                swatches = swatches.child(
                    div()
                        .id(("acc", i))
                        .w(px(28.))
                        .h(px(28.))
                        .rounded_md()
                        .bg(rgb(cu))
                        .on_click(cx.listener(move |this, _ev: &ClickEvent, _w, cx| {
                            this.set_fill_accent(t);
                            cx.notify();
                        })),
                );
            }
            div()
                .flex()
                .flex_col()
                .gap_2()
                .w(px(228.))
                .p_2()
                .bg(rgb(0x252525))
                .child(div().text_xl().child("Format"))
                .child(div().child("Rotation (click to type 0-360)"))
                .child(self.num_field("f_rot", FieldKind::Rotation, cx))
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .gap_2()
                        .child(tool_button("rot_m", "\u{27F2}-15", cx, |t, _w, cx| {
                            t.rotate_by(-15.0);
                            cx.notify();
                        }))
                        .child(tool_button("rot_p", "\u{27F3}+15", cx, |t, _w, cx| {
                            t.rotate_by(15.0);
                            cx.notify();
                        }))
                        .child(tool_button("rot_0", "0\u{00B0}", cx, |t, _w, cx| {
                            t.set_rotation_abs(0.0);
                            cx.notify();
                        })),
                )
                .child(div().child("Position (pt)"))
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .gap_2()
                        .child(self.num_field("f_x", FieldKind::PosX, cx))
                        .child(self.num_field("f_y", FieldKind::PosY, cx)),
                )
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .gap_2()
                        .child(tool_button("x_m", "X-", cx, |t, _w, cx| { t.nudge(-91_440, 0); cx.notify(); }))
                        .child(tool_button("x_p", "X+", cx, |t, _w, cx| { t.nudge(91_440, 0); cx.notify(); }))
                        .child(tool_button("y_m", "Y-", cx, |t, _w, cx| { t.nudge(0, -91_440); cx.notify(); }))
                        .child(tool_button("y_p", "Y+", cx, |t, _w, cx| { t.nudge(0, 91_440); cx.notify(); })),
                )
                .child(div().child("Size (pt)"))
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .gap_2()
                        .child(self.num_field("f_w", FieldKind::SizeW, cx))
                        .child(self.num_field("f_h", FieldKind::SizeH, cx)),
                )
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .gap_2()
                        .child(tool_button("w_m", "W-", cx, |t, _w, cx| { t.resize_by(-182_880, 0); cx.notify(); }))
                        .child(tool_button("w_p", "W+", cx, |t, _w, cx| { t.resize_by(182_880, 0); cx.notify(); }))
                        .child(tool_button("h_m", "H-", cx, |t, _w, cx| { t.resize_by(0, -182_880); cx.notify(); }))
                        .child(tool_button("h_p", "H+", cx, |t, _w, cx| { t.resize_by(0, 182_880); cx.notify(); })),
                )
                .child(div().child("Fill"))
                .child(swatches)
                .child(div().child("Opacity (%)"))
                .child(self.num_field("f_op", FieldKind::Opacity, cx))
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .gap_2()
                        .child(tool_button("front", "Front", cx, |t, _w, cx| {
                            t.run_on_selection("shape.bring_to_front");
                            cx.notify();
                        }))
                        .child(tool_button("back", "Back", cx, |t, _w, cx| {
                            t.run_on_selection("shape.send_to_back");
                            cx.notify();
                        })),
                )
                .child(div().child("Align (Shift-click for multi)"))
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .gap_1()
                        .child(tool_button("al_l", "L", cx, |t, _w, cx| { t.align("shapes.align_left"); cx.notify(); }))
                        .child(tool_button("al_c", "C", cx, |t, _w, cx| { t.align("shapes.align_hcenter"); cx.notify(); }))
                        .child(tool_button("al_r", "R", cx, |t, _w, cx| { t.align("shapes.align_right"); cx.notify(); }))
                        .child(tool_button("al_t", "T", cx, |t, _w, cx| { t.align("shapes.align_top"); cx.notify(); }))
                        .child(tool_button("al_m", "M", cx, |t, _w, cx| { t.align("shapes.align_vcenter"); cx.notify(); }))
                        .child(tool_button("al_b", "B", cx, |t, _w, cx| { t.align("shapes.align_bottom"); cx.notify(); })),
                )
                .child(tool_button("edit_text", "Edit Text (F2)", cx, |t, _w, cx| {
                    if let Some(e) = t.selection {
                        t.begin_text_edit(e);
                    }
                    cx.notify();
                }))
                .child(tool_button("del", "Delete", cx, |t, _w, cx| {
                    t.delete_selection();
                    t.rebuild();
                    cx.notify();
                }))
        });

        div()
            .track_focus(&self.focus)
            .on_key_down(cx.listener(|this, ev: &KeyDownEvent, _, cx| this.on_key_down(ev, cx)))
            .flex()
            .flex_col()
            .gap_3()
            .size_full()
            .bg(rgb(0x1e1e1e))
            .text_color(rgb(0xffffff))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .justify_between()
                    .items_center()
                    .child(div().text_xl().child(title))
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .gap_2()
                            .items_center()
                            .child(tool_button("zoom_out", "\u{2212}", cx, |t, _w, cx| {
                                let z = t.zoom / 1.25;
                                t.set_zoom(z, cx);
                            }))
                            .child(div().child(format!("{}%", (self.zoom * 100.0).round() as i32)))
                            .child(tool_button("zoom_in", "+", cx, |t, _w, cx| {
                                let z = t.zoom * 1.25;
                                t.set_zoom(z, cx);
                            }))
                            .child(tool_button("zoom_fit", "Fit", cx, |t, w, cx| {
                                t.fit_zoom(w);
                                t.rebuild();
                                cx.notify();
                            }))
                            .child(tool_button("min", "\u{2013}", cx, |_this, window, _cx| {
                                window.minimize_window();
                            }))
                            .child(tool_button("max", "\u{25A1}", cx, |_this, window, _cx| {
                                window.zoom_window();
                            }))
                            .child(tool_button("close", "\u{00D7}", cx, |_this, window, _cx| {
                                window.remove_window();
                            })),
                    ),
            )
            .children(palette_panel)
            .child(
                div()
                    .flex()
                    .flex_row()
                    .gap_3()
                    .child(sidebar)
                    .child(
                        div()
                            .w(px(sw))
                            .h(px(sh))
                            .border_1()
                            .border_color(rgb(0x555555))
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|this, ev: &MouseDownEvent, window, cx| {
                                    window.focus(&this.focus, cx);
                                    this.on_mouse_down(ev, cx);
                                }),
                            )
                            .on_mouse_move(
                                cx.listener(|this, ev: &MouseMoveEvent, _, cx| this.on_mouse_move(ev, cx)),
                            )
                            .on_mouse_up(
                                MouseButton::Left,
                                cx.listener(|this, ev: &MouseUpEvent, _, cx| this.on_mouse_up(ev, cx)),
                            )
                            .child(slide_canvas),
                    )
                    .children(inspector),
            )
            .into_any_element()
    }
}

fn run() {
    application().run(|cx: &mut App| {
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
