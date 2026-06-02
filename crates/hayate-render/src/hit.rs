//! Hit-testing against a resolved `Scene` (DESIGN 6.7). gpui-free, so it stays unit-testable
//! and reusable headlessly. Maps a pixel-space point back to the source `Entity` of the
//! front-most node that contains it.

use crate::scene::{Primitive, PxRect, Scene, SceneNode};
use hayate_ir::world::Entity;

/// Return the source `Entity` of the front-most scene node whose primitive contains `(x, y)`.
///
/// Nodes paint back-to-front, so we iterate in reverse to let the topmost hit win. Nodes
/// without a source entity are skipped. Returns `None` if nothing is hit.
pub fn hit_test(scene: &Scene, x: f32, y: f32) -> Option<Entity> {
    for node in scene.nodes.iter().rev() {
        let Some(source) = node.source else {
            continue;
        };
        if node_contains(node, x, y) {
            return Some(source);
        }
    }
    None
}

/// Test whether `(x, y)` lies inside `node`'s primitive, accounting for rotation.
fn node_contains(node: &SceneNode, x: f32, y: f32) -> bool {
    let bounds = primitive_bounds(&node.prim);
    // Rotate the test point into the node's local (unrotated) frame.
    let (lx, ly) = rotate_about_center(x, y, bounds, -node.rotation_deg);
    match &node.prim {
        Primitive::Quad { .. } | Primitive::Text(_) | Primitive::Image { .. } => {
            rect_contains(bounds, lx, ly)
        }
        Primitive::Ellipse { .. } => ellipse_contains(bounds, lx, ly),
    }
}

/// The axis-aligned bounds of a primitive.
fn primitive_bounds(prim: &Primitive) -> PxRect {
    match prim {
        Primitive::Quad { bounds, .. } => *bounds,
        Primitive::Ellipse { bounds, .. } => *bounds,
        Primitive::Text(block) => block.bounds,
        Primitive::Image { bounds, .. } => *bounds,
    }
}

/// Rotate `(x, y)` by `deg` degrees about the center of `bounds`.
fn rotate_about_center(x: f32, y: f32, bounds: PxRect, deg: f32) -> (f32, f32) {
    let cx = bounds.x + bounds.w / 2.0;
    let cy = bounds.y + bounds.h / 2.0;
    let rad = deg.to_radians();
    let (sin, cos) = rad.sin_cos();
    let dx = x - cx;
    let dy = y - cy;
    let rx = dx * cos - dy * sin;
    let ry = dx * sin + dy * cos;
    (cx + rx, cy + ry)
}

/// Axis-aligned containment.
fn rect_contains(r: PxRect, x: f32, y: f32) -> bool {
    x >= r.x && x <= r.x + r.w && y >= r.y && y <= r.y + r.h
}

/// Ellipse containment via the standard ellipse equation.
fn ellipse_contains(r: PxRect, x: f32, y: f32) -> bool {
    let rx = r.w / 2.0;
    let ry = r.h / 2.0;
    if rx <= 0.0 || ry <= 0.0 {
        return false;
    }
    let cx = r.x + rx;
    let cy = r.y + ry;
    let nx = (x - cx) / rx;
    let ny = (y - cy) / ry;
    nx * nx + ny * ny <= 1.0
}

#[cfg(test)]
mod tests;
