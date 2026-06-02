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
        Primitive::Quad { .. } | Primitive::Text(_) => rect_contains(bounds, lx, ly),
        Primitive::Ellipse { .. } => ellipse_contains(bounds, lx, ly),
    }
}

/// The axis-aligned bounds of a primitive.
fn primitive_bounds(prim: &Primitive) -> PxRect {
    match prim {
        Primitive::Quad { bounds, .. } => *bounds,
        Primitive::Ellipse { bounds, .. } => *bounds,
        Primitive::Text(block) => block.bounds,
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
mod tests {
    use super::*;
    use crate::scene::{Primitive, PxRect, PxSize, Scene, SceneNode};
    use hayate_ir::color::Rgba;

    fn quad_node(source: u64, bounds: PxRect, rotation_deg: f32) -> SceneNode {
        SceneNode {
            source: Some(Entity(source)),
            rotation_deg,
            opacity: 1.0,
            prim: Primitive::Quad {
                bounds,
                corner_radius: 0.0,
                fill: None,
                stroke: None,
            },
        }
    }

    fn ellipse_node(source: u64, bounds: PxRect) -> SceneNode {
        SceneNode {
            source: Some(Entity(source)),
            rotation_deg: 0.0,
            opacity: 1.0,
            prim: Primitive::Ellipse {
                bounds,
                fill: None,
                stroke: None,
            },
        }
    }

    fn scene(nodes: Vec<SceneNode>) -> Scene {
        Scene {
            size: PxSize {
                w: 1000.0,
                h: 1000.0,
            },
            background: Rgba::BLACK,
            nodes,
        }
    }

    #[test]
    fn point_inside_single_quad() {
        let s = scene(vec![quad_node(
            7,
            PxRect {
                x: 10.0,
                y: 20.0,
                w: 100.0,
                h: 50.0,
            },
            0.0,
        )]);
        assert_eq!(hit_test(&s, 50.0, 40.0), Some(Entity(7)));
    }

    #[test]
    fn overlapping_quads_topmost_wins() {
        let lower = quad_node(
            1,
            PxRect {
                x: 0.0,
                y: 0.0,
                w: 100.0,
                h: 100.0,
            },
            0.0,
        );
        let upper = quad_node(
            2,
            PxRect {
                x: 50.0,
                y: 50.0,
                w: 100.0,
                h: 100.0,
            },
            0.0,
        );
        let s = scene(vec![lower, upper]);
        // Point (75, 75) is inside both; the last (topmost) node must win.
        assert_eq!(hit_test(&s, 75.0, 75.0), Some(Entity(2)));
    }

    #[test]
    fn point_outside_everything() {
        let s = scene(vec![quad_node(
            1,
            PxRect {
                x: 0.0,
                y: 0.0,
                w: 100.0,
                h: 100.0,
            },
            0.0,
        )]);
        assert_eq!(hit_test(&s, 500.0, 500.0), None);
    }

    #[test]
    fn ellipse_corner_misses_center_hits() {
        let bounds = PxRect {
            x: 0.0,
            y: 0.0,
            w: 100.0,
            h: 100.0,
        };
        let s = scene(vec![ellipse_node(9, bounds)]);
        // Bounding-box corner is outside the inscribed ellipse.
        assert_eq!(hit_test(&s, 1.0, 1.0), None);
        // Center is inside.
        assert_eq!(hit_test(&s, 50.0, 50.0), Some(Entity(9)));
    }

    #[test]
    fn rotated_non_square_quad() {
        // A tall, thin quad centered at (50, 50): 20 wide, 100 tall, unrotated.
        let bounds = PxRect {
            x: 40.0,
            y: 0.0,
            w: 20.0,
            h: 100.0,
        };
        // Rotate 90 degrees about its center -> it becomes 100 wide, 20 tall.
        let s = scene(vec![quad_node(5, bounds, 90.0)]);

        // Point far to the side: inside only when rotation is accounted for.
        // After rotation the shape spans x in [0, 100], y in [40, 60].
        assert_eq!(hit_test(&s, 5.0, 50.0), Some(Entity(5)));
        // A point that would be inside the UNrotated tall quad but outside the rotated one.
        assert_eq!(hit_test(&s, 50.0, 5.0), None);
    }

    #[test]
    fn node_without_source_is_skipped() {
        let mut node = quad_node(
            1,
            PxRect {
                x: 0.0,
                y: 0.0,
                w: 100.0,
                h: 100.0,
            },
            0.0,
        );
        node.source = None;
        let s = scene(vec![node]);
        assert_eq!(hit_test(&s, 50.0, 50.0), None);
    }
}
