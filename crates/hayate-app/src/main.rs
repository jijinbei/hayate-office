//! HayateOffice presentation editor (gpui app).
//!
//! Renders a sample slide's resolved Scene onto a gpui canvas and supports basic editing:
//! click to select a shape (hit-test against the Scene), drag to move it (committed as one
//! undoable transaction). Undo/redo: Cmd/Ctrl+Z / Shift+Cmd/Ctrl+Z.

#![cfg_attr(target_family = "wasm", no_main)]

use std::cell::Cell;
use std::rc::Rc;

use gpui::{
    App, Background, Bounds, Context, Font, FontStyle, FontWeight, Hsla, KeyDownEvent, MouseButton,
    MouseDownEvent, MouseMoveEvent, MouseUpEvent, PathBuilder, Pixels, Point, SharedString,
    TextRun, Window, WindowBounds, WindowOptions, canvas, div, point, prelude::*, px, quad, rgb,
    size,
};
use gpui_platform::application;

use hayate_ir::color::{Color, Rgba, ThemeColorToken};
use hayate_ir::font::{FontRef, ThemeFontSlot};
use hayate_ir::geom::{PointEmu, RectEmu};
use hayate_ir::paint::Fill;
use hayate_ir::presentation::Presentation;
use hayate_ir::shape::Geometry;
use hayate_ir::text::{Paragraph, Run, TextBody};
use hayate_ir::theme::Theme;
use hayate_ir::units::{inch_f, pt};
use hayate_ir::world::Entity;
use hayate_model::{edit, History};
use hayate_render::scene::{Paint, Primitive, PxRect, PxSize, ResolvedRun, Scene, TextBlock};
use hayate_render::{build_slide_scene, hit_test};

const TARGET: PxSize = PxSize { w: 960.0, h: 540.0 };
const SELECTION: u32 = 0x3B82F6;

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
}

impl HayateApp {
    fn new() -> Self {
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
        }
    }

    fn rebuild(&mut self) {
        self.scene = build_slide_scene(&self.pres, self.slide, TARGET);
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
        let k = &ev.keystroke;
        let cmd = k.modifiers.platform || k.modifiers.control;
        if cmd && k.key == "z" {
            if k.modifiers.shift {
                self.history.redo(&mut self.pres.world);
            } else {
                self.history.undo(&mut self.pres.world);
            }
            self.rebuild();
            cx.notify();
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

impl Render for HayateApp {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let scene = self.scene.clone();
        let selection = self.selection;
        let origin_cell = self.canvas_origin.clone();
        let (sw, sh) = (scene.size.w, scene.size.h);
        let title: SharedString =
            format!("HayateOffice — click to select, drag to move ({} shapes)", scene.nodes.len()).into();

        let slide_canvas = canvas(
            |_, _, _| {},
            move |bounds, _, window, cx| {
                let o = bounds.origin;
                origin_cell.set(o);

                let bg: Background = rgb(rgb_u32(scene.background)).into();
                window.paint_quad(quad(
                    Bounds { origin: o, size: size(px(sw), px(sh)) },
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

        div()
            .key_context("HayateApp")
            .track_focus(&_cx.focus_handle())
            .on_key_down(_cx.listener(|this, ev: &KeyDownEvent, _, cx| this.on_key_down(ev, cx)))
            .flex()
            .flex_col()
            .gap_3()
            .size_full()
            .bg(rgb(0x1e1e1e))
            .text_color(rgb(0xffffff))
            .child(div().text_xl().child(title))
            .child(
                div()
                    .w(px(sw))
                    .h(px(sh))
                    .border_1()
                    .border_color(rgb(0x555555))
                    .on_mouse_down(MouseButton::Left, _cx.listener(|this, ev: &MouseDownEvent, _, cx| this.on_mouse_down(ev, cx)))
                    .on_mouse_move(_cx.listener(|this, ev: &MouseMoveEvent, _, cx| this.on_mouse_move(ev, cx)))
                    .on_mouse_up(MouseButton::Left, _cx.listener(|this, ev: &MouseUpEvent, _, cx| this.on_mouse_up(ev, cx)))
                    .child(slide_canvas),
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
            |_, cx| cx.new(|_| HayateApp::new()),
        )
        .unwrap();
        cx.activate(true);
    });
}

#[cfg(not(target_family = "wasm"))]
fn main() {
    run();
}
