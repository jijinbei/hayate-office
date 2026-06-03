//! Unit tests for the parent module.

use super::*;
use crate::scene::{
    Paint, Primitive, PxRect, PxSize, ResolvedParagraph, ResolvedRun, Scene, SceneNode, TextBlock,
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
    assert_eq!(pixel(&buf, w, 50, 50), fill);
    assert_eq!(pixel(&buf, w, 11, 11), Rgba::WHITE);
}

#[test]
fn opacity_halves_alpha_blend() {
    // A half-opaque red over white should land around (255,128,128).
    let mut node = quad(
        PxRect {
            x: 0.0,
            y: 0.0,
            w: 100.0,
            h: 100.0,
        },
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
        PxRect {
            x: 10.0,
            y: 40.0,
            w: 80.0,
            h: 20.0,
        },
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
        bounds: PxRect {
            x: 0.0,
            y: 0.0,
            w: 100.0,
            h: 30.0,
        },
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
            bullet_level: 0,
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
