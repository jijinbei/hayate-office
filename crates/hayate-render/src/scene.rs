//! Backend-agnostic display list (DESIGN 6.7). A `Scene` is one slide resolved at a target
//! pixel size: shapes become concrete primitives with literal colors, while text stays
//! semi-abstract (the app layer shapes/paints it via gpui). The same structure is also the
//! hit-testing structure (each node carries its source `Entity`).

use hayate_ir::color::Rgba;
use hayate_ir::geom::{RectEmu, SizeEmu};
use hayate_ir::text::HAlign;
use hayate_ir::world::Entity;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PxSize {
    pub w: f32,
    pub h: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PxRect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

/// Maps slide coordinates (EMU) to pixels. Offset is implicit (slide origin at 0,0); the
/// app positions the whole scene.
#[derive(Clone, Copy, Debug)]
pub struct Viewport {
    /// Pixels per EMU.
    pub scale: f64,
}

impl Viewport {
    /// Scale a slide of `slide` size to fit within `target` while preserving aspect ratio.
    pub fn fit(slide: SizeEmu, target: PxSize) -> Self {
        let sx = target.w as f64 / slide.w.max(1) as f64;
        let sy = target.h as f64 / slide.h.max(1) as f64;
        Viewport { scale: sx.min(sy) }
    }

    pub fn rect(&self, r: RectEmu) -> PxRect {
        PxRect {
            x: (r.origin.x as f64 * self.scale) as f32,
            y: (r.origin.y as f64 * self.scale) as f32,
            w: (r.size.w as f64 * self.scale) as f32,
            h: (r.size.h as f64 * self.scale) as f32,
        }
    }

    pub fn size(&self, s: SizeEmu) -> PxSize {
        PxSize {
            w: (s.w as f64 * self.scale) as f32,
            h: (s.h as f64 * self.scale) as f32,
        }
    }

    /// Scale a scalar length (EMU) to pixels.
    pub fn len(&self, emu: i64) -> f32 {
        (emu as f64 * self.scale) as f32
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Paint {
    Solid(Rgba),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StrokePx {
    pub color: Rgba,
    pub width: f32,
}

/// A run resolved to concrete font family / size / color (the app only shapes & paints).
#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedRun {
    pub text: String,
    pub family: String,
    pub size_px: f32,
    pub color: Rgba,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedParagraph {
    pub runs: Vec<ResolvedRun>,
    pub align: HAlign,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TextBlock {
    pub bounds: PxRect,
    pub paragraphs: Vec<ResolvedParagraph>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Primitive {
    Quad {
        bounds: PxRect,
        corner_radius: f32,
        fill: Option<Paint>,
        stroke: Option<StrokePx>,
    },
    Ellipse {
        bounds: PxRect,
        fill: Option<Paint>,
        stroke: Option<StrokePx>,
    },
    Text(TextBlock),
}

#[derive(Clone, Debug, PartialEq)]
pub struct SceneNode {
    /// Source entity, for hit-testing back to the document.
    pub source: Option<Entity>,
    /// Rotation in degrees about the node's center.
    pub rotation_deg: f32,
    /// Opacity 0.0..=1.0.
    pub opacity: f32,
    pub prim: Primitive,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Scene {
    pub size: PxSize,
    pub background: Rgba,
    /// Back-to-front paint order.
    pub nodes: Vec<SceneNode>,
}

/// Read the pixel bounds of a primitive (Quad/Ellipse carry `bounds`; Text uses its block bounds).
pub fn prim_bounds(prim: &Primitive) -> PxRect {
    match prim {
        Primitive::Quad { bounds, .. } => *bounds,
        Primitive::Ellipse { bounds, .. } => *bounds,
        Primitive::Text(block) => block.bounds,
    }
}

/// Smallest rect covering both inputs.
fn rect_union(a: PxRect, b: PxRect) -> PxRect {
    let min_x = a.x.min(b.x);
    let min_y = a.y.min(b.y);
    let max_x = (a.x + a.w).max(b.x + b.w);
    let max_y = (a.y + a.h).max(b.y + b.h);
    PxRect {
        x: min_x,
        y: min_y,
        w: max_x - min_x,
        h: max_y - min_y,
    }
}

impl Scene {
    /// Union of every node's primitive bounds, or `None` if the scene has no nodes.
    /// Useful for fit-to-content and select-all bounds.
    pub fn content_bounds(&self) -> Option<PxRect> {
        let mut iter = self.nodes.iter();
        let first = iter.next()?;
        let mut acc = prim_bounds(&first.prim);
        for node in iter {
            acc = rect_union(acc, prim_bounds(&node.prim));
        }
        Some(acc)
    }
}

#[cfg(test)]
mod tests {
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
}
