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
                    fill: fill_paint(fill, op),
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
                    fill: fill_paint(fill, op),
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
            Primitive::Line {
                from,
                to,
                stroke,
                start_arrow,
                end_arrow,
            } => {
                if let Some((col, width)) = stroke_px(stroke, op, s) {
                    // Endpoints in output-pixel space.
                    let x0 = from.0 * sx;
                    let y0 = from.1 * sy;
                    let x1 = to.0 * sx;
                    let y1 = to.1 * sy;
                    let thick = width.max(1.0);
                    draw_thick_line(&mut buf, w, h, x0, y0, x1, y1, thick, col);
                    if *end_arrow {
                        // Arrowhead at END (`to`), barbs pointing back toward `from`.
                        draw_arrowhead(&mut buf, w, h, x0, y0, x1, y1, thick, col);
                    }
                    if *start_arrow {
                        // Arrowhead at START (`from`), barbs pointing back toward `to`
                        // (i.e. outward from the start). Swap the segment endpoints so the
                        // helper draws the head at `(x0, y0)`.
                        draw_arrowhead(&mut buf, w, h, x1, y1, x0, y0, thick, col);
                    }
                }
            }
        }
    }

    buf
}

/// A resolved fill in output-pixel space: either a solid color or a two-stop linear gradient.
/// Colors are already opacity-adjusted.
#[derive(Clone, Copy)]
enum FillPaint {
    Solid(Rgba),
    Linear {
        from: Rgba,
        to: Rgba,
        angle_deg: f32,
    },
}

/// Resolve an optional scene paint into an opacity-adjusted [`FillPaint`].
fn fill_paint(fill: &Option<Paint>, opacity: f32) -> Option<FillPaint> {
    match fill {
        Some(Paint::Solid(c)) => Some(FillPaint::Solid(apply_opacity(*c, opacity))),
        Some(Paint::Linear {
            from,
            to,
            angle_deg,
        }) => Some(FillPaint::Linear {
            from: apply_opacity(*from, opacity),
            to: apply_opacity(*to, opacity),
            angle_deg: *angle_deg,
        }),
        None => None,
    }
}

/// Linearly interpolate two colors per channel at `t` in 0..=1.
fn lerp_rgba(a: Rgba, b: Rgba, t: f32) -> Rgba {
    let lerp = |x: u8, y: u8| {
        (x as f32 + (y as f32 - x as f32) * t)
            .round()
            .clamp(0.0, 255.0) as u8
    };
    Rgba::rgba(
        lerp(a.r, b.r),
        lerp(a.g, b.g),
        lerp(a.b, b.b),
        lerp(a.a, b.a),
    )
}

/// Resolve an optional stroke to (color, width-in-output-px), opacity-adjusted.
fn stroke_px(stroke: &Option<StrokePx>, opacity: f32, scale: f32) -> Option<(Rgba, f32)> {
    stroke.map(|s| (apply_opacity(s.color, opacity), (s.width * scale).max(1.0)))
}

/// Scale a color's alpha by `opacity` (0..=1).
fn apply_opacity(c: Rgba, opacity: f32) -> Rgba {
    Rgba::rgba(
        c.r,
        c.g,
        c.b,
        (c.a as f32 * opacity).round().clamp(0.0, 255.0) as u8,
    )
}

/// Drawing style for a rect/ellipse primitive in output-pixel space.
struct ShapeStyle {
    fill: Option<FillPaint>,
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

    // Precompute the gradient axis (in the shape's local frame) once. `half_span` is the
    // shape's half-extent projected onto that axis, so the gradient parameter t spans 0..1.
    let gradient = match style.fill {
        Some(FillPaint::Linear {
            from,
            to,
            angle_deg,
        }) => {
            let (gs, gc) = angle_deg.to_radians().sin_cos();
            let half_span = ((hw * gc).abs() + (hh * gs).abs()).max(1.0);
            Some((from, to, gs, gc, half_span))
        }
        _ => None,
    };

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
            match style.fill {
                Some(FillPaint::Solid(fc)) => blend_over(buf, idx, fc),
                Some(FillPaint::Linear { .. }) => {
                    let (from, to, gs, gc, half_span) = gradient.unwrap();
                    let t = (lx * gc + ly * gs) / (2.0 * half_span) + 0.5;
                    blend_over(buf, idx, lerp_rgba(from, to, t.clamp(0.0, 1.0)));
                }
                None => {}
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

/// Draw a line of `thickness` output px between two output-pixel points by stamping a small
/// square of pixels at each sampled point along the segment.
fn draw_thick_line(
    buf: &mut [u8],
    w: usize,
    h: usize,
    x0: f32,
    y0: f32,
    x1: f32,
    y1: f32,
    thickness: f32,
    c: Rgba,
) {
    let half = (thickness * 0.5).max(0.5);
    let steps = (x1 - x0).abs().max((y1 - y0).abs()).ceil().max(1.0) as i32;
    for i in 0..=steps {
        let t = i as f32 / steps as f32;
        let px = x0 + (x1 - x0) * t;
        let py = y0 + (y1 - y0) * t;
        fill_px_rect(buf, w, h, px - half, py - half, thickness, thickness, c);
    }
}

/// Draw a simple two-stroke arrowhead at the end point `(x1, y1)` of the segment, pointing
/// away from `(x0, y0)`. The barbs are sized relative to the stroke thickness.
fn draw_arrowhead(
    buf: &mut [u8],
    w: usize,
    h: usize,
    x0: f32,
    y0: f32,
    x1: f32,
    y1: f32,
    thickness: f32,
    c: Rgba,
) {
    let dx = x1 - x0;
    let dy = y1 - y0;
    let len = (dx * dx + dy * dy).sqrt();
    if len <= f32::EPSILON {
        return;
    }
    // Unit vector along the line (from -> to).
    let ux = dx / len;
    let uy = dy / len;
    // Barb length: a few stroke widths, but not longer than the line itself.
    let barb = (thickness * 4.0).max(6.0).min(len);
    let ang = 0.5_f32; // ~28.6 degrees off the shaft
    let (s, co) = ang.sin_cos();
    // Base vector points back along the shaft (from `to` toward `from`); rotate by +/- ang.
    let (bx, by) = (-ux, -uy);
    let r1x = bx * co - by * s;
    let r1y = bx * s + by * co;
    let r2x = bx * co + by * s;
    let r2y = -bx * s + by * co;
    draw_thick_line(
        buf,
        w,
        h,
        x1,
        y1,
        x1 + r1x * barb,
        y1 + r1y * barb,
        thickness,
        c,
    );
    draw_thick_line(
        buf,
        w,
        h,
        x1,
        y1,
        x1 + r2x * barb,
        y1 + r2y * barb,
        thickness,
        c,
    );
}

/// Render an image placeholder: light-gray fill, gray border, and two diagonals.
fn draw_image_box(buf: &mut [u8], w: usize, h: usize, bounds: &PxRect, sx: f32, sy: f32, op: f32) {
    let x = bounds.x * sx;
    let y = bounds.y * sy;
    let bw = bounds.w * sx;
    let bh = bounds.h * sy;
    fill_px_rect(
        buf,
        w,
        h,
        x,
        y,
        bw,
        bh,
        apply_opacity(Rgba::rgb(220, 220, 220), op),
    );
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
fn draw_text_block(
    buf: &mut [u8],
    w: usize,
    h: usize,
    block: &TextBlock,
    sx: f32,
    sy: f32,
    op: f32,
) {
    let bx = block.bounds.x * sx;
    let by = block.bounds.y * sy;
    let bw = block.bounds.w * sx;

    let mut line_top = by;
    for para in &block.paragraphs {
        // Glyph cell height from the first run's size (fallback to a readable minimum).
        let size_px = para.runs.first().map(|r| r.size_px).unwrap_or(16.0);
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
                            fill_px_rect(
                                buf,
                                w,
                                h,
                                gx + 1.0,
                                gy,
                                scale.max(1.0),
                                scale.max(1.0),
                                color,
                            );
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
mod tests;
