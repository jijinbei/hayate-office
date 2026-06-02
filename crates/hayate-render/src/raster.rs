//! Dependency-free software rasterizer for slide thumbnails. Renders a `Scene` into an
//! RGBA8 pixel buffer (top-left origin, row-major), used for headless/offscreen thumbnail
//! generation without gpui. Only solid fills are painted; text and stroke-only shapes are
//! skipped (thumbnails do not need glyph shaping).

use crate::scene::{Paint, Primitive, PxRect, Scene};
use hayate_ir::color::Rgba;

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

    // Paint nodes back-to-front.
    for node in &scene.nodes {
        match &node.prim {
            Primitive::Quad { bounds, fill, .. } => {
                if let Some(Paint::Solid(c)) = fill {
                    fill_rect(&mut buf, w, h, scaled(bounds, sx, sy), *c);
                }
            }
            Primitive::Ellipse { bounds, fill, .. } => {
                if let Some(Paint::Solid(c)) = fill {
                    fill_ellipse(&mut buf, w, h, scaled(bounds, sx, sy), *c);
                }
            }
            Primitive::Text(_) => {
                // Text is skipped: thumbnails do not render glyphs.
            }
            Primitive::Image { bounds, .. } => {
                // Image pixels are not resolved here; paint a light-gray placeholder.
                let placeholder = Rgba::rgb(220, 220, 220);
                fill_rect(&mut buf, w, h, scaled(bounds, sx, sy), placeholder);
            }
        }
    }

    buf
}

/// A rect in output-pixel space (float edges).
struct PxBox {
    x0: f32,
    y0: f32,
    x1: f32,
    y1: f32,
}

/// Scale a scene-space rect into output-pixel space.
fn scaled(b: &PxRect, sx: f32, sy: f32) -> PxBox {
    PxBox {
        x0: b.x * sx,
        y0: b.y * sy,
        x1: (b.x + b.w) * sx,
        y1: (b.y + b.h) * sy,
    }
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

/// Fill a scaled rect with `c`, alpha-blended over existing pixels. Coordinates are clamped.
fn fill_rect(buf: &mut [u8], w: usize, h: usize, b: PxBox, c: Rgba) {
    let (x0, x1) = pixel_range(b.x0, b.x1, w);
    let (y0, y1) = pixel_range(b.y0, b.y1, h);
    for y in y0..y1 {
        for x in x0..x1 {
            let idx = (y * w + x) * 4;
            blend_over(buf, idx, c);
        }
    }
}

/// Fill the ellipse inscribed in the scaled bounds with `c`, alpha-blended. Pixels whose
/// centers satisfy the ellipse equation are painted. Coordinates are clamped.
fn fill_ellipse(buf: &mut [u8], w: usize, h: usize, b: PxBox, c: Rgba) {
    let cx = (b.x0 + b.x1) * 0.5;
    let cy = (b.y0 + b.y1) * 0.5;
    let rx = (b.x1 - b.x0).abs() * 0.5;
    let ry = (b.y1 - b.y0).abs() * 0.5;
    if rx <= 0.0 || ry <= 0.0 {
        return;
    }

    let (x0, x1) = pixel_range(b.x0, b.x1, w);
    let (y0, y1) = pixel_range(b.y0, b.y1, h);
    for y in y0..y1 {
        for x in x0..x1 {
            // Test the pixel center against the ellipse equation.
            let px = x as f32 + 0.5;
            let py = y as f32 + 0.5;
            let nx = (px - cx) / rx;
            let ny = (py - cy) / ry;
            if nx * nx + ny * ny <= 1.0 {
                let idx = (y * w + x) * 4;
                blend_over(buf, idx, c);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::{PxSize, SceneNode};

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
                PxRect {
                    x: 0.0,
                    y: 0.0,
                    w: 50.0,
                    h: 100.0,
                },
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
                PxRect {
                    x: 10.0,
                    y: 10.0,
                    w: 80.0,
                    h: 80.0,
                },
                Some(Paint::Solid(fill)),
            )],
        };
        let (w, h) = (100, 100);
        let buf = rasterize(&scene, w, h);

        // Center of the bounding box is inside the ellipse.
        assert_eq!(pixel(&buf, w, 50, 50), fill);
        // The bounding-box corner is outside the ellipse, so it keeps the background.
        assert_eq!(pixel(&buf, w, 11, 11), Rgba::WHITE);
    }
}
