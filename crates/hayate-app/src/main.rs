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
use hayate_ir::geom::{PointEmu, RectEmu};
use hayate_ir::paint::Fill;
use hayate_ir::presentation::Presentation;
use hayate_ir::shape::Geometry;
use hayate_ir::text::{Paragraph, Run, TextBody};
use hayate_ir::theme::Theme;
use hayate_ir::units::{inch_f, pt};
use hayate_ir::world::Entity;
use hayate_model::{edit, History, Operation, Transaction};
use hayate_core::CommandRegistry;
use hayate_render::scene::{Paint, Primitive, PxRect, PxSize, ResolvedRun, Scene, TextBlock};
use hayate_render::{build_slide_scene, hit_test};

const TARGET: PxSize = PxSize { w: 960.0, h: 540.0 };
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
    /// Available slide-view size in pixels (the slide is fit into this; grows with the window).
    view_size: PxSize,
    /// Command registry (palette / scripts / AI surface).
    registry: CommandRegistry,
    /// Command palette state when open.
    palette: Option<PaletteState>,
}

struct PaletteState {
    query: String,
    sel: usize,
}

impl HayateApp {
    fn new(cx: &mut Context<Self>) -> Self {
        let pres = sample_presentation();
        let slide = pres.slides()[0];
        let scene = build_slide_scene(&pres, slide, TARGET);
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
            view_size: TARGET,
            registry: hayate_core::builtins(),
            palette: None,
        }
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
        self.scene = build_slide_scene(&self.pres, self.slide, self.view_size);
    }

    /// Pixels per EMU (width-fit).
    fn scale(&self) -> f64 {
        self.scene.size.w as f64 / self.pres.slide_size.w.max(1) as f64
    }

    fn on_mouse_down(&mut self, ev: &MouseDownEvent, cx: &mut Context<Self>) {
        let o = self.canvas_origin.get();
        let x = f32::from(ev.position.x - o.x);
        let y = f32::from(ev.position.y - o.y);
        self.selection = hit_test(&self.scene, x, y);
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
        cx.notify();
    }

    fn on_mouse_up(&mut self, _ev: &MouseUpEvent, cx: &mut Context<Self>) {
        let Some(d) = self.drag.take() else { return };
        let Some(final_f) = self.pres.world.frames.get(&d.entity).copied() else {
            return;
        };
        if final_f != d.start_frame {
            // Revert to the start, then commit the whole move as one undoable step.
            self.pres.world.frames.insert(d.entity, d.start_frame);
            let tx = edit::set_frame(d.entity, final_f);
            self.history.commit(&mut self.pres.world, tx);
            self.rebuild();
        }
        cx.notify();
    }

    fn on_key_down(&mut self, ev: &KeyDownEvent, cx: &mut Context<Self>) {
        if self.palette.is_some() {
            self.palette_key(ev, cx);
            return;
        }
        let k = &ev.keystroke;
        let cmd = k.modifiers.platform || k.modifiers.control;
        let mut dirty = true;
        match k.key.as_str() {
            "p" if cmd => {
                self.palette = Some(PaletteState { query: String::new(), sel: 0 });
                dirty = false;
                cx.notify();
            }
            "z" if cmd && k.modifiers.shift => {
                self.history.redo(&mut self.pres.world);
            }
            "z" if cmd => {
                self.history.undo(&mut self.pres.world);
            }
            "s" if cmd => {
                self.save();
                dirty = false;
            }
            "o" if cmd => {
                self.open();
            }
            "r" if !cmd => {
                self.add_rect();
            }
            "delete" | "backspace" if !cmd => {
                self.delete_selection();
            }
            _ => dirty = false,
        }
        if dirty {
            self.rebuild();
            cx.notify();
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
        self.history.commit(&mut self.pres.world, tx);
        self.selection = Some(e);
    }

    /// Delete the selected shape (undoable: despawn captures components to restore).
    fn delete_selection(&mut self) {
        if let Some(e) = self.selection.take() {
            let tx = Transaction::new("delete shape", vec![Operation::Despawn { entity: e }]);
            self.history.commit(&mut self.pres.world, tx);
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

    fn open(&mut self) {
        match hayate_format::load(DOC_PATH) {
            Ok(p) => {
                self.pres = p;
                self.slide = self.pres.slides().first().copied().unwrap_or(self.slide);
                self.history = History::new();
                self.selection = None;
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
/// Paint a Scene's background and shapes at `o` (window coords). Shared by the main view and
/// the slide-list thumbnails.
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
        match &node.prim {
            Primitive::Quad { bounds: r, corner_radius, fill: Some(Paint::Solid(c)), .. } => {
                let b = Bounds {
                    origin: point(o.x + px(r.x), o.y + px(r.y)),
                    size: size(px(r.w), px(r.h)),
                };
                window.paint_quad(quad(
                    b,
                    px(*corner_radius),
                    Background::from(rgb(rgb_u32(*c))),
                    px(0.),
                    gpui::transparent_black(),
                    Default::default(),
                ));
            }
            Primitive::Ellipse { bounds: r, fill: Some(Paint::Solid(c)), .. } => {
                let cx_ = o.x + px(r.x + r.w / 2.0);
                let cy_ = o.y + px(r.y + r.h / 2.0);
                let rx = px(r.w / 2.0);
                let ry = px(r.h / 2.0);
                let mut b = PathBuilder::fill();
                b.move_to(point(cx_ + rx, cy_));
                b.arc_to(point(rx, ry), px(0.), false, false, point(cx_ - rx, cy_));
                b.arc_to(point(rx, ry), px(0.), false, false, point(cx_ + rx, cy_));
                b.close();
                if let Ok(path) = b.build() {
                    window.paint_path(path, rgb(rgb_u32(*c)));
                }
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

        // Fit the slide into the current window area: content scales as the window grows.
        let vp = window.viewport_size();
        self.view_size = PxSize {
            w: (f32::from(vp.width) - 240.0).max(64.0), // minus slide-list sidebar
            h: (f32::from(vp.height) - 88.0).max(64.0), // minus top bar
        };
        self.rebuild();

        let scene = self.scene.clone();
        let selection = self.selection;
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

                // Selection outline (drawn on top).
                if let Some(sel) = selection {
                    if let Some(node) = scene.nodes.iter().find(|n| n.source == Some(sel)) {
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
                            rgb(SELECTION),
                            Default::default(),
                        ));
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
                    ),
            )
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
