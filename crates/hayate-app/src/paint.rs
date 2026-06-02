//! Scene painting: resolving scene paints into gpui backgrounds and drawing a Scene's
//! background, shapes, images, and text onto a gpui window.

use gpui::{
    point, px, quad, rgb, size, App, Background, Bounds, Corners, Image, ImageFormat, PathBuilder,
    Pixels, Point, SharedString, TextRun, Window,
};

use hayate_ir::color::Rgba;
use hayate_render::scene::{Paint, Primitive, Scene, TextBlock};

use crate::util::{hsla_of, rgb_u32, rotate_pt, run_font};

/// Fill background from an Rgba, scaling alpha by `opacity` (0..1).
pub(crate) fn fill_bg(c: Rgba, opacity: f32) -> Background {
    gpui_rgba(c, opacity).into()
}

/// Convert a scene color (+ node opacity) into a gpui color.
pub(crate) fn gpui_rgba(c: Rgba, opacity: f32) -> gpui::Rgba {
    gpui::Rgba {
        r: c.r as f32 / 255.0,
        g: c.g as f32 / 255.0,
        b: c.b as f32 / 255.0,
        a: (c.a as f32 / 255.0) * opacity.clamp(0.0, 1.0),
    }
}

/// Resolve a scene `Paint` (solid or two-stop linear gradient) into a gpui `Background`.
pub(crate) fn paint_bg(p: &Paint, opacity: f32) -> Background {
    match p {
        Paint::Solid(c) => fill_bg(*c, opacity),
        Paint::Linear {
            from,
            to,
            angle_deg,
        } => gpui::linear_gradient(
            *angle_deg,
            gpui::linear_color_stop(gpui_rgba(*from, opacity), 0.0),
            gpui::linear_color_stop(gpui_rgba(*to, opacity), 1.0),
        ),
    }
}

pub(crate) fn paint_text(
    tb: &TextBlock,
    ox: Pixels,
    oy: Pixels,
    window: &mut Window,
    cx: &mut App,
) {
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
        // Shape at natural width (force_width = None; passing the box width here would stretch
        // the glyphs across the box). Alignment uses the box width via paint's `align_width`.
        let shaped =
            window
                .text_system()
                .shape_line(SharedString::from(text), font_size, &runs, None);
        let _ = shaped.paint(
            point(left, top),
            line_height,
            align,
            Some(px(tb.bounds.w)),
            window,
            cx,
        );
        top += line_height;
    }
}

/// Guess an image's encoded format from its magic bytes (matches gpui's supported set).
/// Returns `None` for unrecognized data, in which case the caller keeps the placeholder.
pub(crate) fn guess_image_format(bytes: &[u8]) -> Option<ImageFormat> {
    if bytes.starts_with(&[0x89, b'P', b'N', b'G']) {
        Some(ImageFormat::Png)
    } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some(ImageFormat::Jpeg)
    } else if bytes.starts_with(b"GIF8") {
        Some(ImageFormat::Gif)
    } else if bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        Some(ImageFormat::Webp)
    } else if bytes.starts_with(b"BM") {
        Some(ImageFormat::Bmp)
    } else {
        None
    }
}

/// Paint a Scene's background and shapes at `o` (window coords). Shared by the main view and
/// the slide-list thumbnails. Rotated shapes are drawn as paths (quads carry no transform).
///
/// `media` resolves a `Primitive::Image`'s `media_key` to its encoded bytes so real images
/// can be decoded and painted; when missing/undecodable a gray placeholder is drawn instead.
pub(crate) fn paint_scene(
    scene: &Scene,
    o: Point<Pixels>,
    media: &std::collections::BTreeMap<String, Vec<u8>>,
    window: &mut Window,
    cx: &mut App,
) {
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
            Primitive::Quad {
                bounds: r,
                corner_radius,
                fill: Some(paint),
                ..
            } => {
                if angle.abs() < 1e-3 {
                    let b = Bounds {
                        origin: point(o.x + px(r.x), o.y + px(r.y)),
                        size: size(px(r.w), px(r.h)),
                    };
                    window.paint_quad(quad(
                        b,
                        px(*corner_radius),
                        paint_bg(paint, opacity),
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
                        window.paint_path(path, paint_bg(paint, opacity));
                    }
                }
            }
            Primitive::Ellipse {
                bounds: r,
                fill: Some(paint),
                ..
            } => {
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
                    window.paint_path(path, paint_bg(paint, opacity));
                }
            }
            Primitive::Image {
                bounds: r,
                media_key,
            } => {
                let b = Bounds {
                    origin: point(o.x + px(r.x), o.y + px(r.y)),
                    size: size(px(r.w), px(r.h)),
                };
                // Try to decode and paint the real image; fall back to a placeholder.
                let mut painted = false;
                if let Some(bytes) = media.get(media_key) {
                    if let Some(format) = guess_image_format(bytes) {
                        let image = std::sync::Arc::new(Image::from_bytes(format, bytes.clone()));
                        // gpui decodes asynchronously via its asset system; the first paint may
                        // return None and schedule a re-render once the image is ready.
                        if let Some(render) = image.use_render_image(window, cx) {
                            let _ = window.paint_image(b, Corners::default(), render, 0, false);
                            painted = true;
                        }
                    }
                }
                if !painted {
                    // Placeholder: a light-gray box (bytes missing or not yet decoded).
                    window.paint_quad(quad(
                        b,
                        px(0.),
                        Background::from(rgb(0xCCCCCC)),
                        px(1.),
                        rgb(0x888888),
                        Default::default(),
                    ));
                }
            }
            Primitive::Text(tb) => paint_text(tb, o.x, o.y, window, cx),
            _ => {}
        }
    }
}
