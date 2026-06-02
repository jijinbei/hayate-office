//! Small free helpers shared across the app: UTF-16/byte conversions, char-boundary
//! navigation, axis-aligned resize math, color conversions, font building, and rotation.

use std::ops::Range;

use gpui::{rgb, Font, FontStyle, FontWeight, Hsla};

use hayate_ir::color::Rgba;
use hayate_ir::geom::{PointEmu, RectEmu, SizeEmu};
use hayate_render::scene::{Primitive, PxRect, ResolvedRun};

/// Byte index in `s` for a UTF-16 code-unit offset (clamped to the end).
pub(crate) fn utf16_to_byte(s: &str, off16: usize) -> usize {
    let mut u = 0;
    for (b, c) in s.char_indices() {
        if u >= off16 {
            return b;
        }
        u += c.len_utf16();
    }
    s.len()
}

/// UTF-16 code-unit offset for a byte index in `s` (clamped to the end).
pub(crate) fn byte_to_utf16(s: &str, byte: usize) -> usize {
    let mut u = 0;
    for (b, c) in s.char_indices() {
        if b >= byte {
            return u;
        }
        u += c.len_utf16();
    }
    u
}

/// Convert a byte range in `s` to a UTF-16 range.
pub(crate) fn range_to_utf16(s: &str, r: &Range<usize>) -> Range<usize> {
    byte_to_utf16(s, r.start)..byte_to_utf16(s, r.end)
}

/// Convert a UTF-16 range to a byte range in `s`.
pub(crate) fn range_from_utf16(s: &str, r: &Range<usize>) -> Range<usize> {
    utf16_to_byte(s, r.start)..utf16_to_byte(s, r.end)
}

/// Byte index of the char boundary before `byte` in `s` (for backspace).
pub(crate) fn prev_char_boundary(s: &str, byte: usize) -> usize {
    s[..byte]
        .char_indices()
        .next_back()
        .map(|(i, _)| i)
        .unwrap_or(0)
}

/// Byte index of the char boundary after `byte` in `s`.
pub(crate) fn next_char_boundary(s: &str, byte: usize) -> usize {
    s[byte..]
        .char_indices()
        .nth(1)
        .map(|(i, _)| byte + i)
        .unwrap_or(s.len())
}

/// New frame when dragging resize `handle` (TL,T,TR,R,BR,B,BL,L) by (dx,dy) EMU from `start`.
/// Axis-aligned; keeps the opposite edge fixed and clamps to a minimum size.
pub(crate) fn resize_frame(handle: usize, start: RectEmu, dx: i64, dy: i64) -> RectEmu {
    let min = 12_700; // 1pt
    let right0 = start.origin.x + start.size.w;
    let bottom0 = start.origin.y + start.size.h;
    let mut w = start.size.w;
    let mut h = start.size.h;
    if matches!(handle, 2 | 3 | 4) {
        w = start.size.w + dx; // right edge
    }
    if matches!(handle, 0 | 6 | 7) {
        w = start.size.w - dx; // left edge
    }
    if matches!(handle, 4 | 5 | 6) {
        h = start.size.h + dy; // bottom edge
    }
    if matches!(handle, 0 | 1 | 2) {
        h = start.size.h - dy; // top edge
    }
    w = w.max(min);
    h = h.max(min);
    let left = matches!(handle, 0 | 6 | 7);
    let top = matches!(handle, 0 | 1 | 2);
    let x = if left { right0 - w } else { start.origin.x };
    let y = if top { bottom0 - h } else { start.origin.y };
    RectEmu {
        origin: PointEmu::new(x, y),
        size: SizeEmu::new(w, h),
    }
}

pub(crate) fn rgb_u32(c: Rgba) -> u32 {
    ((c.r as u32) << 16) | ((c.g as u32) << 8) | (c.b as u32)
}

pub(crate) fn hsla_of(c: Rgba) -> Hsla {
    rgb(rgb_u32(c)).into()
}

pub(crate) fn run_font(r: &ResolvedRun) -> Font {
    let mut f = gpui::font(r.family.clone());
    if r.bold {
        f.weight = FontWeight::BOLD;
    }
    if r.italic {
        f.style = FontStyle::Italic;
    }
    f
}

pub(crate) fn prim_bounds(prim: &Primitive) -> PxRect {
    match prim {
        Primitive::Quad { bounds, .. } => *bounds,
        Primitive::Ellipse { bounds, .. } => *bounds,
        Primitive::Image { bounds, .. } => *bounds,
        Primitive::Text(tb) => tb.bounds,
    }
}

/// Rotate point (x,y) around center (cx,cy) by `rad` radians (clockwise in screen coords).
pub(crate) fn rotate_pt(x: f32, y: f32, cx: f32, cy: f32, rad: f32) -> (f32, f32) {
    let (s, c) = rad.sin_cos();
    let dx = x - cx;
    let dy = y - cy;
    (cx + dx * c - dy * s, cy + dx * s + dy * c)
}
