//! Unit tests for the parent module.

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
