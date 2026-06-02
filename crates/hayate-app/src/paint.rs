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
        // Shape with wrapping at the box width so long paragraphs flow onto multiple rows
        // instead of overflowing to the right. `shape_text` returns one `WrappedLine` per
        // hard line break; each may itself contain soft wrap boundaries. Passing `None` as
        // `bounds` to `paint` makes alignment use the layout's `wrap_width` (the box width).
        match window.text_system().shape_text(
            SharedString::from(text),
            font_size,
            &runs,
            Some(px(tb.bounds.w)),
            None,
        ) {
            Ok(lines) => {
                for line in lines.iter() {
                    let _ = line.paint(point(left, top), line_height, align, None, window, cx);
                    // A wrapped line occupies one row per soft wrap boundary plus one.
                    let rows = line.wrap_boundaries.len() + 1;
                    top += line_height * (rows as f32);
                }
            }
            Err(_) => {} // skip this paragraph on shaping error
        }
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

/// Read an encoded image's pixel dimensions straight from its header bytes, without decoding
/// the pixels. Supports the same formats as [`guess_image_format`] (PNG/JPEG/GIF/BMP/WebP).
/// Returns `None` when the format is unknown or the header is truncated, so callers fall back
/// to a default frame size.
pub(crate) fn image_dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    let be16 = |i: usize| Some(u16::from_be_bytes([*bytes.get(i)?, *bytes.get(i + 1)?]) as u32);
    let be32 = |i: usize| {
        Some(u32::from_be_bytes([
            bytes[i],
            bytes[i + 1],
            bytes[i + 2],
            bytes[i + 3],
        ]))
    };
    let le16 = |i: usize| Some(u16::from_le_bytes([*bytes.get(i)?, *bytes.get(i + 1)?]) as u32);
    match guess_image_format(bytes)? {
        // PNG IHDR: 4-byte big-endian width/height at offsets 16/20.
        ImageFormat::Png if bytes.len() >= 24 => Some((be32(16)?, be32(20)?)),
        // GIF logical screen: little-endian u16 width/height at offsets 6/8.
        ImageFormat::Gif if bytes.len() >= 10 => Some((le16(6)?, le16(8)?)),
        // BMP BITMAPINFOHEADER: little-endian i32 width/height at offsets 18/22 (height may be
        // negative for top-down rasters).
        ImageFormat::Bmp if bytes.len() >= 26 => {
            let w = i32::from_le_bytes([bytes[18], bytes[19], bytes[20], bytes[21]]);
            let h = i32::from_le_bytes([bytes[22], bytes[23], bytes[24], bytes[25]]);
            Some((w.unsigned_abs(), h.unsigned_abs()))
        }
        // JPEG: walk segment markers until a Start-Of-Frame carries the dimensions.
        ImageFormat::Jpeg => {
            let mut i = 2;
            while i + 9 < bytes.len() {
                if bytes[i] != 0xFF {
                    i += 1;
                    continue;
                }
                let marker = bytes[i + 1];
                // SOF0..SOF15 hold the frame size; DHT/DAC/RST are not frame headers.
                let is_sof = (0xC0..=0xCF).contains(&marker)
                    && marker != 0xC4
                    && marker != 0xC8
                    && marker != 0xCC;
                if is_sof {
                    let h = be16(i + 5)?;
                    let w = be16(i + 7)?;
                    return Some((w, h));
                }
                let len = be16(i + 2)? as usize;
                if len < 2 {
                    return None;
                }
                i += 2 + len;
            }
            None
        }
        // WebP: VP8X (extended), VP8L (lossless), or VP8 (lossy) sub-chunk at offset 12.
        ImageFormat::Webp if bytes.len() >= 30 => match &bytes[12..16] {
            b"VP8X" => {
                let w = 1 + (bytes[24] as u32 | (bytes[25] as u32) << 8 | (bytes[26] as u32) << 16);
                let h = 1 + (bytes[27] as u32 | (bytes[28] as u32) << 8 | (bytes[29] as u32) << 16);
                Some((w, h))
            }
            b"VP8L" => {
                let b = &bytes[21..25];
                let bits =
                    b[0] as u32 | (b[1] as u32) << 8 | (b[2] as u32) << 16 | (b[3] as u32) << 24;
                Some((1 + (bits & 0x3FFF), 1 + ((bits >> 14) & 0x3FFF)))
            }
            b"VP8 " => {
                let w = le16(26)? & 0x3FFF;
                let h = le16(28)? & 0x3FFF;
                Some((w, h))
            }
            _ => None,
        },
        _ => None,
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
            Primitive::Line {
                from,
                to,
                stroke: Some(stroke),
                start_arrow,
                end_arrow,
            } => {
                let color = gpui_rgba(stroke.color, opacity);
                let width = px(stroke.width.max(1.0));
                // Endpoints in window coordinates. Rotation is applied about the line's center
                // (the midpoint of from/to), consistent with the rotated-quad path above; the
                // arrowheads below are computed from the already-rotated endpoints, so they
                // rotate with the line through the full 0-360 deg range.
                let (cx_, cy_) = ((from.0 + to.0) / 2.0, (from.1 + to.1) / 2.0);
                let (fx, fy) = rotate_pt(from.0, from.1, cx_, cy_, angle);
                let (tx, ty) = rotate_pt(to.0, to.1, cx_, cy_, angle);
                let p_from = point(o.x + px(fx), o.y + px(fy));
                let p_to = point(o.x + px(tx), o.y + px(ty));

                let mut b = PathBuilder::stroke(width);
                b.move_to(p_from);
                b.line_to(p_to);
                if let Ok(path) = b.build() {
                    window.paint_path(path, color);
                }

                // Draw an arrowhead at `(hx, hy)`, with barbs pointing back toward `(ox, oy)`
                // (the other endpoint). Both points are in already-rotated scene coords.
                let mut draw_head = |hx: f32, hy: f32, ox: f32, oy: f32| {
                    let dx = hx - ox;
                    let dy = hy - oy;
                    let len = (dx * dx + dy * dy).sqrt();
                    if len <= f32::EPSILON {
                        return;
                    }
                    // Unit vector pointing from the other endpoint toward the head.
                    let (ux, uy) = (dx / len, dy / len);
                    let barb = (stroke.width * 4.0).max(8.0).min(len);
                    let ang = 0.5_f32;
                    let (s, co) = ang.sin_cos();
                    // Base vector points back along the shaft (head -> other endpoint).
                    let (bx, by) = (-ux, -uy);
                    let r1 = (bx * co - by * s, bx * s + by * co);
                    let r2 = (bx * co + by * s, -bx * s + by * co);
                    let p_head = point(o.x + px(hx), o.y + px(hy));
                    for (rx, ry) in [r1, r2] {
                        let mut ab = PathBuilder::stroke(width);
                        ab.move_to(p_head);
                        ab.line_to(point(o.x + px(hx + rx * barb), o.y + px(hy + ry * barb)));
                        if let Ok(path) = ab.build() {
                            window.paint_path(path, color);
                        }
                    }
                };

                if *end_arrow {
                    // Arrowhead at END (`to`), barbs pointing back toward START.
                    draw_head(tx, ty, fx, fy);
                }
                if *start_arrow {
                    // Arrowhead at START (`from`), barbs pointing back toward END.
                    draw_head(fx, fy, tx, ty);
                }
            }
            Primitive::Text(tb) => paint_text(tb, o.x, o.y, window, cx),
            _ => {}
        }
    }
}
