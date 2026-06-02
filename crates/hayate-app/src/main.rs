//! HayateOffice presentation editor (gpui app).
//!
//! Renders a sample presentation's resolved Scene onto a gpui canvas: the slide background
//! plus each shape painted at its pixel position with theme-resolved colors. Vector shapes
//! are drawn now; text and ellipse fidelity, then editing, come next.

#![cfg_attr(target_family = "wasm", no_main)]

use gpui::{
    App, Background, Bounds, Context, SharedString, Window, WindowBounds, WindowOptions, canvas,
    div, point, prelude::*, px, quad, rgb, size,
};
use gpui_platform::application;

use hayate_ir::color::{Color, Rgba, ThemeColorToken};
use hayate_ir::geom::RectEmu;
use hayate_ir::paint::Fill;
use hayate_ir::presentation::Presentation;
use hayate_ir::shape::Geometry;
use hayate_ir::theme::Theme;
use hayate_ir::units::inch_f;
use hayate_render::build_slide_scene;
use hayate_render::scene::{Paint, Primitive, PxSize, Scene};

/// Build a small sample deck: one slide with three accent-colored rectangles.
fn sample_presentation() -> Presentation {
    let mut p = Presentation::new();
    let master = p.add_master(Theme::default());
    let layout = p.add_layout(master, "Blank");
    let slide = p.add_slide(layout);

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
            .insert(e, RectEmu::new(x, inch_f(1.0), inch_f(1.6), inch_f(1.6)));
        p.world.geometries.insert(e, Geometry::Rect);
        p.world.fills.insert(e, Fill::Solid(Color::theme(token)));
    }
    p
}

fn rgb_u32(c: Rgba) -> u32 {
    ((c.r as u32) << 16) | ((c.g as u32) << 8) | (c.b as u32)
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

impl Render for HayateApp {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let scene = self.scene.clone();
        let (sw, sh) = (scene.size.w, scene.size.h);

        let slide_canvas = canvas(
            |_, _, _| {},
            move |bounds, _, window, _| {
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

                // Shapes (quads for now).
                for node in &scene.nodes {
                    if let Primitive::Quad {
                        bounds: r,
                        corner_radius,
                        fill: Some(Paint::Solid(c)),
                        ..
                    } = &node.prim
                    {
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
