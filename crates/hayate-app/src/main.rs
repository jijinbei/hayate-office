//! HayateOffice presentation editor (gpui app).
//!
//! First milestone: open a window and prove the IR -> render(Scene) -> gpui pipeline links
//! end to end. It builds a sample presentation, resolves a Scene, and shows the slide
//! background plus a swatch per filled shape (theme-resolved colors). Drawing the Scene
//! faithfully (shapes/text on a canvas) comes next.

#![cfg_attr(target_family = "wasm", no_main)]

use gpui::{
    App, Bounds, Context, SharedString, Window, WindowBounds, WindowOptions, div, prelude::*, px,
    rgb, size,
};
use gpui_platform::application;

use hayate_ir::color::{Color, ThemeColorToken};
use hayate_ir::geom::RectEmu;
use hayate_ir::paint::Fill;
use hayate_ir::presentation::Presentation;
use hayate_ir::shape::Geometry;
use hayate_ir::theme::Theme;
use hayate_ir::units::inch_f;
use hayate_render::build_slide_scene;
use hayate_render::scene::{Paint, Primitive, PxSize};

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

fn rgb_u32(c: hayate_ir::color::Rgba) -> u32 {
    ((c.r as u32) << 16) | ((c.g as u32) << 8) | (c.b as u32)
}

struct HayateApp {
    title: SharedString,
    background: u32,
    swatches: Vec<u32>,
}

impl HayateApp {
    fn new() -> Self {
        let p = sample_presentation();
        let slide = p.slides()[0];
        let scene = build_slide_scene(&p, slide, PxSize { w: 960.0, h: 540.0 });

        let mut swatches = Vec::new();
        for node in &scene.nodes {
            if let Primitive::Quad {
                fill: Some(Paint::Solid(c)),
                ..
            } = &node.prim
            {
                swatches.push(rgb_u32(*c));
            }
        }

        HayateApp {
            title: format!("HayateOffice — slide 1: {} shapes", scene.nodes.len()).into(),
            background: rgb_u32(scene.background),
            swatches,
        }
    }
}

impl Render for HayateApp {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let mut swatch_row = div().flex().gap_2();
        for c in &self.swatches {
            swatch_row = swatch_row.child(div().w(px(64.)).h(px(64.)).rounded_md().bg(rgb(*c)));
        }

        div()
            .flex()
            .flex_col()
            .gap_3()
            .size_full()
            .bg(rgb(0x1e1e1e))
            .text_color(rgb(0xffffff))
            .child(div().text_xl().child(self.title.clone()))
            .child(
                // The resolved slide background, drawn as a 16:9 panel.
                div()
                    .w(px(480.))
                    .h(px(270.))
                    .border_1()
                    .border_color(rgb(0x555555))
                    .bg(rgb(self.background)),
            )
            .child(swatch_row)
    }
}

fn run() {
    application().run(|cx: &mut App| {
        let bounds = Bounds::centered(None, size(px(1000.), px(640.)), cx);
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
