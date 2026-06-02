//! Dependency-free software rasterizer for headless/offscreen rendering. Renders a `Scene`
//! into an RGBA8 pixel buffer (top-left origin, row-major) without gpui, so it is usable for
//! thumbnails and for debug captures that approximate the live editor.
//!
//! Fidelity (vs. the gpui canvas): solid fills, per-node opacity, rotation about the node
//! center, rounded corners, and strokes are honored. Text is drawn with a built-in 5x7 ASCII
//! bitmap font (lowercase mapped to uppercase shapes); non-ASCII glyphs (e.g. Japanese) are
//! drawn as outlined cells, so position/size/color are visible but the exact glyph is not.
//! Images render as a gray placeholder box with a border and diagonals.

use crate::scene::{Paint, Primitive, PxRect, ResolvedRun, Scene, StrokePx, TextBlock};
use hayate_ir::color::Rgba;
use hayate_ir::text::HAlign;

mod font;

/// Rasterize `scene` into an `out_w` x `out_h` RGBA8 buffer (len = out_w*out_h*4),
/// row-major with a top-left origin.
pub fn rasterize(scene: &Scene, out_w: u32, out_h: u32) -> Vec<u8> {
    let w = out_w as usize;
    let h = out_h as usize;
    let mut buf = vec![0u8; w * h * 4];

    // Fill the whole buffer with the background color (opaque write).
    for px in buf.chunks_exact_mut(4) {
        px[0] = scene.background.r;
        px[1] = scene.background.g;
        px[2] = scene.background.b;
        px[3] = scene.background.a;
    }

    // Scene-to-output scale factors. Guard against a zero-sized scene.
    let sx = if scene.size.w > 0.0 {
        out_w as f32 / scene.size.w
    } else {
        0.0
    };
    let sy = if scene.size.h > 0.0 {
        out_h as f32 / scene.size.h
    } else {
        0.0
    };
    // Uniform scale used for radii/stroke/glyph metrics (slides keep aspect ratio in practice).
    let s = 0.5 * (sx + sy);

    // Paint nodes back-to-front.
    for node in &scene.nodes {
        let op = node.opacity.clamp(0.0, 1.0);
        let angle = node.rotation_deg.to_radians();
        match &node.prim {
            Primitive::Quad {
                bounds,
                corner_radius,
                fill,
                stroke,
            } => {
                let style = ShapeStyle {
                    fill: solid(fill, op),
                    stroke: stroke_px(stroke, op, s),
                    corner: (corner_radius * s).max(0.0),
                    ellipse: false,
                };
                paint_shape(&mut buf, w, h, bounds, sx, sy, angle, &style);
            }
            Primitive::Ellipse {
                bounds,
                fill,
                stroke,
            } => {
                let style = ShapeStyle {
                    fill: solid(fill, op),
                    stroke: stroke_px(stroke, op, s),
                    corner: 0.0,
                    ellipse: true,
                };
                paint_shape(&mut buf, w, h, bounds, sx, sy, angle, &style);
            }
            Primitive::Text(block) => {
                draw_text_block(&mut buf, w, h, block, sx, sy, op);
            }
            Primitive::Image { bounds, .. } => {
                draw_image_box(&mut buf, w, h, bounds, sx, sy, op);
            }
        }
    }

    buf
}

/// Resolve an optional solid paint to an opacity-adjusted color.
fn solid(fill: &Option<Paint>, opacity: f32) -> Option<Rgba> {
    match fill {
        Some(Paint::Solid(c)) => Some(apply_opacity(*c, opacity)),
        None => None,
    }
}

/// Resolve an optional stroke to (color, width-in-output-px), opacity-adjusted.
fn stroke_px(stroke: &Option<StrokePx>, opacity: f32, scale: f32) -> Option<(Rgba, f32)> {
    stroke.map(|s| (apply_opacity(s.color, opacity), (s.width * scale).max(1.0)))
}

/// Scale a color's alpha by `opacity` (0..=1).
fn apply_opacity(c: Rgba, opacity: f32) -> Rgba {
    Rgba::rgba(c.r, c.g, c.b, (c.a as f32 * opacity).round().clamp(0.0, 255.0) as u8)
}

/// Drawing style for a rect/ellipse primitive in output-pixel space.
struct ShapeStyle {
    fill: Option<Rgba>,
    /// (color, width in output px).
    stroke: Option<(Rgba, f32)>,
    /// Corner radius in output px (rects only).
    corner: f32,
    ellipse: bool,
}

/// Paint a (possibly rotated) rect or ellipse described by scene-space `bounds`.
/// Iterates the rotated axis-aligned bounding box and tests each pixel in the shape's
/// local (un-rotated) frame, so rotation, rounded corners, fill, and stroke compose cleanly.
fn paint_shape(
    buf: &mut [u8],
    w: usize,
    h: usize,
    bounds: &PxRect,
    sx: f32,
    sy: f32,
    angle: f32,
    style: &ShapeStyle,
) {
    let cx = (bounds.x + bounds.w * 0.5) * sx;
    let cy = (bounds.y + bounds.h * 0.5) * sy;
    let hw = (bounds.w * 0.5 * sx).abs();
    let hh = (bounds.h * 0.5 * sy).abs();
    if hw <= 0.0 || hh <= 0.0 {
        return;
    }
    let (sin, cos) = angle.sin_cos();

    // Axis-aligned bounding box of the rotated shape.
    let ax = (hw * cos).abs() + (hh * sin).abs();
    let ay = (hw * sin).abs() + (hh * cos).abs();
    let (x0, x1) = pixel_range(cx - ax, cx + ax, w);
    let (y0, y1) = pixel_range(cy - ay, cy + ay, h);

    let stroke_w = style.stroke.map(|(_, wd)| wd).unwrap_or(0.0);

    for y in y0..y1 {
        for x in x0..x1 {
            let dx = x as f32 + 0.5 - cx;
            let dy = y as f32 + 0.5 - cy;
            // Inverse-rotate the pixel into the shape's local frame.
            let lx = dx * cos + dy * sin;
            let ly = -dx * sin + dy * cos;

            if !inside(lx, ly, hw, hh, style.corner, style.ellipse) {
                continue;
            }
            let idx = (y * w + x) * 4;
            if let Some((scol, _)) = style.stroke {
                let inner = inside(
                    lx,
                    ly,
                    hw - stroke_w,
                    hh - stroke_w,
                    (style.corner - stroke_w).max(0.0),
                    style.ellipse,
                );
                if !inner {
                    blend_over(buf, idx, scol);
                    continue;
                }
            }
            if let Some(fc) = style.fill {
                blend_over(buf, idx, fc);
            }
        }
    }
}

/// Membership test in local (un-rotated) coordinates centered on the shape.
fn inside(lx: f32, ly: f32, hw: f32, hh: f32, corner: f32, ellipse: bool) -> bool {
    if hw <= 0.0 || hh <= 0.0 {
        return false;
    }
    if ellipse {
        let nx = lx / hw;
        let ny = ly / hh;
        return nx * nx + ny * ny <= 1.0;
    }
    let ax = lx.abs();
    let ay = ly.abs();
    if ax > hw || ay > hh {
        return false;
    }
    if corner > 0.0 {
        let inset_x = hw - corner;
        let inset_y = hh - corner;
        if ax > inset_x && ay > inset_y {
            let dx = ax - inset_x;
            let dy = ay - inset_y;
            return dx * dx + dy * dy <= corner * corner;
        }
    }
    true
}

/// Convert a float edge range into clamped integer pixel bounds [lo, hi).
fn pixel_range(a: f32, b: f32, max: usize) -> (usize, usize) {
    let lo = a.min(b).floor().max(0.0) as usize;
    let hi = (a.max(b).ceil().max(0.0) as usize).min(max);
    (lo.min(max), hi)
}

/// Alpha-blend `src` over the pixel at `idx` (src-over compositing).
fn blend_over(buf: &mut [u8], idx: usize, src: Rgba) {
    let sa = src.a as f32 / 255.0;
    if sa <= 0.0 {
        return;
    }
    let inv = 1.0 - sa;
    let dr = buf[idx] as f32;
    let dg = buf[idx + 1] as f32;
    let db = buf[idx + 2] as f32;
    let da = buf[idx + 3] as f32 / 255.0;

    buf[idx] = (src.r as f32 * sa + dr * inv).round().clamp(0.0, 255.0) as u8;
    buf[idx + 1] = (src.g as f32 * sa + dg * inv).round().clamp(0.0, 255.0) as u8;
    buf[idx + 2] = (src.b as f32 * sa + db * inv).round().clamp(0.0, 255.0) as u8;
    let out_a = sa + da * inv;
    buf[idx + 3] = (out_a * 255.0).round().clamp(0.0, 255.0) as u8;
}

/// Blend a solid axis-aligned rect given in output-pixel coordinates.
fn fill_px_rect(buf: &mut [u8], w: usize, h: usize, x: f32, y: f32, rw: f32, rh: f32, c: Rgba) {
    let (x0, x1) = pixel_range(x, x + rw, w);
    let (y0, y1) = pixel_range(y, y + rh, h);
    for py in y0..y1 {
        for px in x0..x1 {
            blend_over(buf, (py * w + px) * 4, c);
        }
    }
}

/// Draw a thin line between two output-pixel points (used for image-box diagonals).
fn draw_line(buf: &mut [u8], w: usize, h: usize, x0: f32, y0: f32, x1: f32, y1: f32, c: Rgba) {
    let steps = (x1 - x0).abs().max((y1 - y0).abs()).ceil().max(1.0) as i32;
    for i in 0..=steps {
        let t = i as f32 / steps as f32;
        let px = (x0 + (x1 - x0) * t).round();
        let py = (y0 + (y1 - y0) * t).round();
        if px >= 0.0 && py >= 0.0 && (px as usize) < w && (py as usize) < h {
            blend_over(buf, ((py as usize) * w + px as usize) * 4, c);
        }
    }
}

/// Render an image placeholder: light-gray fill, gray border, and two diagonals.
fn draw_image_box(buf: &mut [u8], w: usize, h: usize, bounds: &PxRect, sx: f32, sy: f32, op: f32) {
    let x = bounds.x * sx;
    let y = bounds.y * sy;
    let bw = bounds.w * sx;
    let bh = bounds.h * sy;
    fill_px_rect(buf, w, h, x, y, bw, bh, apply_opacity(Rgba::rgb(220, 220, 220), op));
    let border = apply_opacity(Rgba::rgb(150, 150, 150), op);
    // Border (1px frame).
    fill_px_rect(buf, w, h, x, y, bw, 1.0, border);
    fill_px_rect(buf, w, h, x, y + bh - 1.0, bw, 1.0, border);
    fill_px_rect(buf, w, h, x, y, 1.0, bh, border);
    fill_px_rect(buf, w, h, x + bw - 1.0, y, 1.0, bh, border);
    // Diagonals.
    draw_line(buf, w, h, x, y, x + bw, y + bh, border);
    draw_line(buf, w, h, x + bw, y, x, y + bh, border);
}

/// Draw a text block: one line per paragraph, ASCII via the 5x7 bitmap font, with horizontal
/// alignment within the block bounds. Rotation is not applied to text (acceptable for debug).
fn draw_text_block(buf: &mut [u8], w: usize, h: usize, block: &TextBlock, sx: f32, sy: f32, op: f32) {
    let bx = block.bounds.x * sx;
    let by = block.bounds.y * sy;
    let bw = block.bounds.w * sx;

    let mut line_top = by;
    for para in &block.paragraphs {
        // Glyph cell height from the first run's size (fallback to a readable minimum).
        let size_px = para
            .runs
            .first()
            .map(|r| r.size_px)
            .unwrap_or(16.0);
        let gh = (size_px * sy).max(7.0);
        let scale = gh / 7.0;
        let cell_w = 6.0 * scale; // 5px glyph + 1px advance

        // Measure the line width to honor center/right alignment.
        let total_chars: usize = para.runs.iter().map(|r| r.text.chars().count()).sum();
        let line_w = total_chars as f32 * cell_w;
        let mut pen_x = bx
            + match para.align {
                HAlign::Left | HAlign::Justify => 0.0,
                HAlign::Center => ((bw - line_w) * 0.5).max(0.0),
                HAlign::Right => (bw - line_w).max(0.0),
            };

        for run in &para.runs {
            pen_x = draw_run(buf, w, h, run, pen_x, line_top, scale, op);
        }
        line_top += gh * 1.4;
    }
}

/// Draw a single run starting at `pen_x`, returning the new pen x.
fn draw_run(
    buf: &mut [u8],
    w: usize,
    h: usize,
    run: &ResolvedRun,
    pen_x: f32,
    top: f32,
    scale: f32,
    op: f32,
) -> f32 {
    let color = apply_opacity(run.color, op);
    let cell_w = 6.0 * scale;
    let glyph_h = 7.0 * scale;
    let mut x = pen_x;
    for ch in run.text.chars() {
        if let Some(rows) = font::glyph(ch) {
            for (ry, bits) in rows.iter().enumerate() {
                for cx in 0..5 {
                    if bits & (1 << (4 - cx)) != 0 {
                        let gx = x + cx as f32 * scale;
                        let gy = top + ry as f32 * scale;
                        fill_px_rect(buf, w, h, gx, gy, scale.max(1.0), scale.max(1.0), color);
                        if run.bold {
                            // Pseudo-bold: a second pass shifted by one device pixel.
                            fill_px_rect(buf, w, h, gx + 1.0, gy, scale.max(1.0), scale.max(1.0), color);
                        }
                    }
                }
            }
        } else if ch != ' ' {
            // Unknown glyph (e.g. CJK): draw an outlined cell so position/size is visible.
            let cw = 5.0 * scale;
            fill_px_rect(buf, w, h, x, top, cw, 1.0, color);
            fill_px_rect(buf, w, h, x, top + glyph_h - 1.0, cw, 1.0, color);
            fill_px_rect(buf, w, h, x, top, 1.0, glyph_h, color);
            fill_px_rect(buf, w, h, x + cw - 1.0, top, 1.0, glyph_h, color);
        }
        if run.underline {
            fill_px_rect(buf, w, h, x, top + glyph_h, cell_w, scale.max(1.0), color);
        }
        x += cell_w;
    }
    x
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::{
        Paint, Primitive, PxRect, PxSize, ResolvedParagraph, ResolvedRun, Scene, SceneNode,
        TextBlock,
    };

    fn quad(bounds: PxRect, fill: Option<Paint>) -> SceneNode {
        SceneNode {
            source: None,
            rotation_deg: 0.0,
            opacity: 1.0,
            prim: Primitive::Quad {
                bounds,
                corner_radius: 0.0,
                fill,
                stroke: None,
            },
        }
    }

    fn ellipse(bounds: PxRect, fill: Option<Paint>) -> SceneNode {
        SceneNode {
            source: None,
            rotation_deg: 0.0,
            opacity: 1.0,
            prim: Primitive::Ellipse {
                bounds,
                fill,
                stroke: None,
            },
        }
    }

    fn pixel(buf: &[u8], w: u32, x: u32, y: u32) -> Rgba {
        let idx = ((y * w + x) * 4) as usize;
        Rgba::rgba(buf[idx], buf[idx + 1], buf[idx + 2], buf[idx + 3])
    }

    #[test]
    fn quad_left_half_red_right_half_white() {
        let red = Rgba::rgb(255, 0, 0);
        let scene = Scene {
            size: PxSize { w: 100.0, h: 100.0 },
            background: Rgba::WHITE,
            nodes: vec![quad(
                PxRect { x: 0.0, y: 0.0, w: 50.0, h: 100.0 },
                Some(Paint::Solid(red)),
            )],
        };
        let (w, h) = (100, 100);
        let buf = rasterize(&scene, w, h);
        assert_eq!(pixel(&buf, w, 25, 50), red);
        assert_eq!(pixel(&buf, w, 75, 50), Rgba::WHITE);
    }

    #[test]
    fn output_length_matches() {
        let scene = Scene {
            size: PxSize { w: 16.0, h: 9.0 },
            background: Rgba::BLACK,
            nodes: vec![],
        };
        let (w, h) = (320, 180);
        let buf = rasterize(&scene, w, h);
        assert_eq!(buf.len(), (w * h * 4) as usize);
    }

    #[test]
    fn ellipse_center_filled_corner_background() {
        let fill = Rgba::rgb(0, 0, 255);
        let scene = Scene {
            size: PxSize { w: 100.0, h: 100.0 },
            background: Rgba::WHITE,
            nodes: vec![ellipse(
                PxRect { x: 10.0, y: 10.0, w: 80.0, h: 80.0 },
                Some(Paint::Solid(fill)),
            )],
        };
        let (w, h) = (100, 100);
        let buf = rasterize(&scene, w, h);
        assert_eq!(pixel(&buf, w, 50, 50), fill);
        assert_eq!(pixel(&buf, w, 11, 11), Rgba::WHITE);
    }

    #[test]
    fn opacity_halves_alpha_blend() {
        // A half-opaque red over white should land around (255,128,128).
        let mut node = quad(
            PxRect { x: 0.0, y: 0.0, w: 100.0, h: 100.0 },
            Some(Paint::Solid(Rgba::rgb(255, 0, 0))),
        );
        node.opacity = 0.5;
        let scene = Scene {
            size: PxSize { w: 100.0, h: 100.0 },
            background: Rgba::WHITE,
            nodes: vec![node],
        };
        let buf = rasterize(&scene, 100, 100);
        let p = pixel(&buf, 100, 50, 50);
        assert_eq!(p.r, 255);
        assert!((120..=136).contains(&p.g), "g was {}", p.g);
        assert_eq!(p.g, p.b);
    }

    #[test]
    fn rotated_90_quad_fills_rotated_bounds() {
        // A 80x20 bar rotated 90deg about its center should cover a vertical strip.
        let mut node = quad(
            PxRect { x: 10.0, y: 40.0, w: 80.0, h: 20.0 },
            Some(Paint::Solid(Rgba::rgb(0, 128, 0))),
        );
        node.rotation_deg = 90.0;
        let scene = Scene {
            size: PxSize { w: 100.0, h: 100.0 },
            background: Rgba::WHITE,
            nodes: vec![node],
        };
        let buf = rasterize(&scene, 100, 100);
        // Center stays filled; a point far above center (within the rotated bar) is filled,
        // while a point to the far left (outside the now-narrow bar) is background.
        assert_eq!(pixel(&buf, 100, 50, 50), Rgba::rgb(0, 128, 0));
        assert_eq!(pixel(&buf, 100, 50, 20), Rgba::rgb(0, 128, 0));
        assert_eq!(pixel(&buf, 100, 15, 50), Rgba::WHITE);
    }

    #[test]
    fn text_draws_some_foreground_pixels() {
        let block = TextBlock {
            bounds: PxRect { x: 0.0, y: 0.0, w: 100.0, h: 30.0 },
            paragraphs: vec![ResolvedParagraph {
                runs: vec![ResolvedRun {
                    text: "AB".to_string(),
                    family: "x".to_string(),
                    size_px: 20.0,
                    color: Rgba::rgb(0, 0, 0),
                    bold: false,
                    italic: false,
                    underline: false,
                }],
                align: HAlign::Left,
            }],
        };
        let scene = Scene {
            size: PxSize { w: 100.0, h: 100.0 },
            background: Rgba::WHITE,
            nodes: vec![SceneNode {
                source: None,
                rotation_deg: 0.0,
                opacity: 1.0,
                prim: Primitive::Text(block),
            }],
        };
        let buf = rasterize(&scene, 100, 100);
        // At least one black foreground pixel should have been painted for the glyphs.
        let any_black = buf
            .chunks_exact(4)
            .any(|p| p[0] < 50 && p[1] < 50 && p[2] < 50);
        assert!(any_black, "expected glyph pixels to be painted");
    }
}
