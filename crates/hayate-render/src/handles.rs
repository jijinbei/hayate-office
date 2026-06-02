//! Selection resize handles. Given a rect (in scene px) and a rotation, produce the 8 handle
//! centers (corners + edge midpoints) rotated about the rect's center. gpui-free so it stays
//! unit-testable.

use crate::scene::PxRect;

/// Rotate point `(x, y)` clockwise by `rad` radians about center `(cx, cy)`.
///
/// Scene px space has Y pointing down, so a positive (clockwise on screen) rotation uses
/// `[[cos, -sin], [sin, cos]]` applied to the offset from the center.
fn rotate(x: f32, y: f32, cx: f32, cy: f32, rad: f32) -> (f32, f32) {
    let (s, c) = rad.sin_cos();
    let dx = x - cx;
    let dy = y - cy;
    (cx + dx * c - dy * s, cy + dx * s + dy * c)
}

/// The 8 handle centers of rect `r`, each rotated clockwise by `rotation_deg` about r's center.
///
/// Order: TL, T (top-mid), TR, R (right-mid), BR, B (bottom-mid), BL, L (left-mid).
pub fn resize_handles(r: PxRect, rotation_deg: f32) -> [(f32, f32); 8] {
    let cx = r.x + r.w / 2.0;
    let cy = r.y + r.h / 2.0;
    let rad = rotation_deg.to_radians();

    let left = r.x;
    let right = r.x + r.w;
    let top = r.y;
    let bottom = r.y + r.h;
    let mid_x = cx;
    let mid_y = cy;

    let unrotated = [
        (left, top),     // TL
        (mid_x, top),    // T
        (right, top),    // TR
        (right, mid_y),  // R
        (right, bottom), // BR
        (mid_x, bottom), // B
        (left, bottom),  // BL
        (left, mid_y),   // L
    ];

    let mut out = [(0.0f32, 0.0f32); 8];
    for (i, &(x, y)) in unrotated.iter().enumerate() {
        out[i] = rotate(x, y, cx, cy, rad);
    }
    out
}

#[cfg(test)]
mod tests;
