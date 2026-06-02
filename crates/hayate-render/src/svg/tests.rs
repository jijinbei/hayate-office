//! Unit tests for the parent module.

use super::*;
use crate::scene::{
    Paint, Primitive, PxRect, PxSize, ResolvedParagraph, ResolvedRun, Scene, SceneNode, TextBlock,
};
use hayate_ir::color::Rgba;
use hayate_ir::text::HAlign;

fn node(prim: Primitive) -> SceneNode {
    SceneNode {
        source: None,
        rotation_deg: 0.0,
        opacity: 1.0,
        prim,
    }
}

#[test]
fn exports_quad_and_text() {
    let scene = Scene {
        size: PxSize { w: 200.0, h: 100.0 },
        background: Rgba::WHITE,
        nodes: vec![
            node(Primitive::Quad {
                bounds: PxRect {
                    x: 10.0,
                    y: 10.0,
                    w: 50.0,
                    h: 40.0,
                },
                corner_radius: 0.0,
                fill: Some(Paint::Solid(Rgba::rgb(255, 0, 0))),
                stroke: None,
            }),
            node(Primitive::Text(TextBlock {
                bounds: PxRect {
                    x: 10.0,
                    y: 60.0,
                    w: 180.0,
                    h: 30.0,
                },
                paragraphs: vec![ResolvedParagraph {
                    runs: vec![ResolvedRun {
                        text: "a < b & c".to_string(),
                        family: "Arial".to_string(),
                        size_px: 16.0,
                        color: Rgba::BLACK,
                        bold: false,
                        italic: false,
                        underline: false,
                    }],
                    align: HAlign::Left,
                }],
            })),
        ],
    };

    let svg = export_svg(&scene);
    assert!(svg.starts_with("<svg"), "should start with <svg: {}", svg);
    assert!(svg.contains("<rect"), "should contain a <rect");
    // The red fill on the quad.
    assert!(svg.contains("rgb(255,0,0)"), "should contain red fill");
    // Escaped text content.
    assert!(
        svg.contains("a &lt; b &amp; c"),
        "should contain escaped text: {}",
        svg
    );
}

#[test]
fn semi_transparent_yields_fill_opacity() {
    let scene = Scene {
        size: PxSize { w: 100.0, h: 100.0 },
        background: Rgba::WHITE,
        nodes: vec![node(Primitive::Quad {
            bounds: PxRect {
                x: 0.0,
                y: 0.0,
                w: 10.0,
                h: 10.0,
            },
            corner_radius: 0.0,
            fill: Some(Paint::Solid(Rgba::rgba(0, 0, 0, 128))),
            stroke: None,
        })],
    };
    let svg = export_svg(&scene);
    assert!(
        svg.contains("fill-opacity="),
        "alpha < 255 should yield fill-opacity: {}",
        svg
    );
}

#[test]
fn fill_less_uses_none() {
    let scene = Scene {
        size: PxSize { w: 100.0, h: 100.0 },
        background: Rgba::WHITE,
        nodes: vec![node(Primitive::Ellipse {
            bounds: PxRect {
                x: 0.0,
                y: 0.0,
                w: 10.0,
                h: 10.0,
            },
            fill: None,
            stroke: Some(StrokePx {
                color: Rgba::BLACK,
                width: 2.0,
            }),
        })],
    };
    let svg = export_svg(&scene);
    assert!(svg.contains("<ellipse"), "should contain <ellipse");
    assert!(svg.contains("fill=\"none\""), "fill-less should be none");
    assert!(
        svg.contains("stroke-width=\"2\""),
        "should carry stroke width"
    );
}

#[test]
fn rotation_emits_transform() {
    let scene = Scene {
        size: PxSize { w: 100.0, h: 100.0 },
        background: Rgba::WHITE,
        nodes: vec![SceneNode {
            source: None,
            rotation_deg: 45.0,
            opacity: 1.0,
            prim: Primitive::Quad {
                bounds: PxRect {
                    x: 0.0,
                    y: 0.0,
                    w: 10.0,
                    h: 10.0,
                },
                corner_radius: 0.0,
                fill: Some(Paint::Solid(Rgba::BLACK)),
                stroke: None,
            },
        }],
    };
    let svg = export_svg(&scene);
    assert!(
        svg.contains("transform=\"rotate(45 5 5)\""),
        "should emit rotate transform: {}",
        svg
    );
}
