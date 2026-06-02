//! HayateOffice presentation editor (gpui app).
//!
//! Renders a sample presentation's resolved Scene onto a gpui canvas: background, vector
//! shapes (quads + ellipses) and text, each at its pixel position with theme-resolved
//! colors and fonts. Editing/interaction comes next.

#![cfg_attr(target_family = "wasm", no_main)]

use gpui::{
    App, Background, Bounds, Context, Font, FontStyle, FontWeight, Hsla, PathBuilder, SharedString,
    TextRun, Window, WindowBounds, WindowOptions, canvas, div, point, prelude::*, px, quad, rgb,
    size,
};
use gpui_platform::application;

use hayate_ir::color::{Color, Rgba, ThemeColorToken};
use hayate_ir::font::{FontRef, ThemeFontSlot};
use hayate_ir::geom::RectEmu;
use hayate_ir::paint::Fill;
use hayate_ir::presentation::Presentation;
use hayate_ir::shape::Geometry;
use hayate_ir::text::{Paragraph, Run, TextBody};
use hayate_ir::theme::Theme;
use hayate_ir::units::{inch_f, pt};
use hayate_render::build_slide_scene;
use hayate_render::scene::{Paint, Primitive, PxSize, ResolvedRun, Scene, TextBlock};

/// Build a small sample deck: a title, three accent rectangles, and an ellipse.
fn sample_presentation() -> Presentation {
    let mut p = Presentation::new();
    let master = p.add_master(Theme::default());
    let layout = p.add_layout(master, "Blank");
    let slide = p.add_slide(layout);

    // Title text box (mixed Japanese + Latin to exercise the ea/latin font slots).
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

    // An ellipse.
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

struct HayateApp {
    title: SharedString,
    scene: Scene,
}

impl HayateApp {
    fn new() -> Self {
        let p = sample_presentation();
        let slide = p.slides()[0];
        let scene = build_slide_scene(&p, slide, PxSize { w: 960.0, h: 540.0 });
        HayateApp {
            title: format!("HayateOffice — slide 1: {} shapes", scene.nodes.len()).into(),
            scene,
        }
    }
}

/// Paint one text block: each paragraph is shaped as a single line, stacked vertically.
fn paint_text(tb: &TextBlock, ox: gpui::Pixels, oy: gpui::Pixels, window: &mut Window, cx: &mut App) {
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
        let (sw, sh) = (scene.size.w, scene.size.h);

        let slide_canvas = canvas(
            |_, _, _| {},
            move |bounds, _, window, cx| {
                let o = bounds.origin;

                // Slide background.
                let bg: Background = rgb(rgb_u32(scene.background)).into();
                window.paint_quad(quad(
                    Bounds {
                        origin: o,
                        size: size(px(sw), px(sh)),
                    },
                    px(0.),
                    bg,
                    px(0.),
                    gpui::transparent_black(),
                    Default::default(),
                ));

                for node in &scene.nodes {
                    match &node.prim {
                        Primitive::Quad {
                            bounds: r,
                            corner_radius,
                            fill: Some(Paint::Solid(c)),
                            ..
                        } => {
                            let b = Bounds {
                                origin: point(o.x + px(r.x), o.y + px(r.y)),
                                size: size(px(r.w), px(r.h)),
                            };
                            let f: Background = rgb(rgb_u32(*c)).into();
                            window.paint_quad(quad(
                                b,
                                px(*corner_radius),
                                f,
                                px(0.),
                                gpui::transparent_black(),
                                Default::default(),
                            ));
                        }
                        Primitive::Ellipse {
                            bounds: r,
                            fill: Some(Paint::Solid(c)),
                            ..
                        } => {
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
                        Primitive::Text(tb) => {
                            paint_text(tb, o.x, o.y, window, cx);
                        }
                        _ => {}
                    }
                }
            },
        )
        .size_full();

        div()
            .flex()
            .flex_col()
            .gap_3()
            .size_full()
            .bg(rgb(0x1e1e1e))
            .text_color(rgb(0xffffff))
            .child(div().text_xl().child(self.title.clone()))
            .child(
                div()
                    .w(px(sw))
                    .h(px(sh))
                    .border_1()
                    .border_color(rgb(0x555555))
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
