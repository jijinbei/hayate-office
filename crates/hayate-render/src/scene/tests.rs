//! Unit tests for the parent module.

use super::*;

fn quad_node(x: f32, y: f32, w: f32, h: f32) -> SceneNode {
    SceneNode {
        source: None,
        rotation_deg: 0.0,
        opacity: 1.0,
        prim: Primitive::Quad {
            bounds: PxRect { x, y, w, h },
            corner_radius: 0.0,
            fill: None,
            stroke: None,
        },
    }
}

fn scene_with(nodes: Vec<SceneNode>) -> Scene {
    Scene {
        size: PxSize { w: 100.0, h: 100.0 },
        background: Rgba::rgb(0, 0, 0),
        nodes,
    }
}

#[test]
fn content_bounds_unions_two_quads() {
    let scene = scene_with(vec![
        quad_node(10.0, 10.0, 20.0, 20.0),
        quad_node(50.0, 40.0, 30.0, 10.0),
    ]);
    // Union spans x:10..80, y:10..50.
    assert_eq!(
        scene.content_bounds(),
        Some(PxRect {
            x: 10.0,
            y: 10.0,
            w: 70.0,
            h: 40.0,
        })
    );
}

#[test]
fn prim_bounds_returns_image_bounds() {
    let bounds = PxRect {
        x: 3.0,
        y: 4.0,
        w: 20.0,
        h: 30.0,
    };
    let prim = Primitive::Image {
        bounds,
        media_key: "sha256:img".to_string(),
    };
    assert_eq!(prim_bounds(&prim), bounds);
}

#[test]
fn content_bounds_empty_is_none() {
    let scene = scene_with(vec![]);
    assert_eq!(scene.content_bounds(), None);
}

#[test]
fn content_bounds_single_node_is_its_own_bounds() {
    let bounds = PxRect {
        x: 5.0,
        y: 7.0,
        w: 12.0,
        h: 9.0,
    };
    let scene = scene_with(vec![quad_node(bounds.x, bounds.y, bounds.w, bounds.h)]);
    assert_eq!(scene.content_bounds(), Some(bounds));
}
