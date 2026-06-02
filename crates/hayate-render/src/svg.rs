//! Standalone SVG export of a `Scene` (DESIGN: headless/offscreen rendering). gpui-free and
//! dependency-free: it just serializes the display list to an SVG document string, which is
//! handy for thumbnails, debugging, and vector export.

use crate::scene::{Paint, Primitive, PxRect, Scene, SceneNode, StrokePx};
use hayate_ir::color::Rgba;

/// Escape XML special characters in text content / attribute values.
fn esc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            _ => out.push(ch),
        }
    }
    out
}

/// Format a color as an SVG `rgb(r,g,b)` string (alpha is conveyed separately via
/// `*-opacity` attributes).
fn color(c: Rgba) -> String {
    format!("rgb({},{},{})", c.r, c.g, c.b)
}

/// Append a `fill`/`stroke`-style paint attribute plus its opacity (when alpha < 255).
/// `kind` is "fill" or "stroke".
fn push_paint(out: &mut String, kind: &str, c: Option<Rgba>) {
    match c {
        Some(c) => {
            out.push_str(&format!(" {}=\"{}\"", kind, color(c)));
            if c.a < 255 {
                let opacity = c.a as f32 / 255.0;
                out.push_str(&format!(" {}-opacity=\"{}\"", kind, opacity));
            }
        }
        None => out.push_str(&format!(" {}=\"none\"", kind)),
    }
}

fn paint_color(p: &Option<Paint>) -> Option<Rgba> {
    // SVG export is solid-only for now; a linear gradient is approximated by its start color.
    p.as_ref().map(|paint| match paint {
        Paint::Solid(c) => *c,
        Paint::Linear { from, .. } => *from,
    })
}

/// Build a `transform="rotate(deg cx cy)"` attribute for a node whose primitive occupies
/// `bounds`, or an empty string when rotation is ~zero.
fn rotation_attr(rotation_deg: f32, bounds: PxRect) -> String {
    if rotation_deg.abs() < f32::EPSILON {
        return String::new();
    }
    let cx = bounds.x + bounds.w / 2.0;
    let cy = bounds.y + bounds.h / 2.0;
    format!(" transform=\"rotate({} {} {})\"", rotation_deg, cx, cy)
}

/// Export a `Scene` to a standalone SVG document string.
pub fn export_svg(scene: &Scene) -> String {
    let w = scene.size.w;
    let h = scene.size.h;
    let mut out = String::new();

    out.push_str(&format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{}\" height=\"{}\" viewBox=\"0 0 {} {}\">",
        w, h, w, h
    ));

    // Background rect filling the canvas.
    out.push_str(&format!(
        "<rect x=\"0\" y=\"0\" width=\"{}\" height=\"{}\"",
        w, h
    ));
    push_paint(&mut out, "fill", Some(scene.background));
    out.push_str("/>");

    for node in &scene.nodes {
        emit_node(&mut out, node);
    }

    out.push_str("</svg>");
    out
}

fn emit_node(out: &mut String, node: &SceneNode) {
    match &node.prim {
        Primitive::Quad {
            bounds,
            corner_radius,
            fill,
            stroke,
        } => emit_quad(out, node, *bounds, *corner_radius, fill, stroke),
        Primitive::Ellipse {
            bounds,
            fill,
            stroke,
        } => emit_ellipse(out, node, *bounds, fill, stroke),
        Primitive::Text(block) => emit_text(out, node, block),
        Primitive::Image { bounds, .. } => emit_image(out, node, *bounds),
    }
}

/// Emit a light-gray `<rect>` placeholder for an image (actual pixels are resolved elsewhere).
fn emit_image(out: &mut String, node: &SceneNode, bounds: PxRect) {
    out.push_str(&format!(
        "<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\"",
        bounds.x, bounds.y, bounds.w, bounds.h
    ));
    push_paint(out, "fill", Some(Rgba::rgb(220, 220, 220)));
    out.push_str(&rotation_attr(node.rotation_deg, bounds));
    out.push_str("/>");
}

fn push_stroke(out: &mut String, stroke: &Option<StrokePx>) {
    match stroke {
        Some(s) => {
            push_paint(out, "stroke", Some(s.color));
            out.push_str(&format!(" stroke-width=\"{}\"", s.width));
        }
        None => {}
    }
}

fn emit_quad(
    out: &mut String,
    node: &SceneNode,
    bounds: PxRect,
    corner_radius: f32,
    fill: &Option<Paint>,
    stroke: &Option<StrokePx>,
) {
    out.push_str(&format!(
        "<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\"",
        bounds.x, bounds.y, bounds.w, bounds.h
    ));
    if corner_radius > 0.0 {
        out.push_str(&format!(" rx=\"{}\"", corner_radius));
    }
    push_paint(out, "fill", paint_color(fill));
    push_stroke(out, stroke);
    out.push_str(&rotation_attr(node.rotation_deg, bounds));
    out.push_str("/>");
}

fn emit_ellipse(
    out: &mut String,
    node: &SceneNode,
    bounds: PxRect,
    fill: &Option<Paint>,
    stroke: &Option<StrokePx>,
) {
    let cx = bounds.x + bounds.w / 2.0;
    let cy = bounds.y + bounds.h / 2.0;
    let rx = bounds.w / 2.0;
    let ry = bounds.h / 2.0;
    out.push_str(&format!(
        "<ellipse cx=\"{}\" cy=\"{}\" rx=\"{}\" ry=\"{}\"",
        cx, cy, rx, ry
    ));
    push_paint(out, "fill", paint_color(fill));
    push_stroke(out, stroke);
    out.push_str(&rotation_attr(node.rotation_deg, bounds));
    out.push_str("/>");
}

fn emit_text(out: &mut String, node: &SceneNode, block: &crate::scene::TextBlock) {
    let bounds = block.bounds;
    let rot = rotation_attr(node.rotation_deg, bounds);

    // Stack paragraphs vertically; each paragraph's baseline advances by ~1.3 * size.
    let mut baseline = bounds.y;
    for para in &block.paragraphs {
        // Determine the line's font size from the first run (fallback to a default).
        let size = para.runs.first().map(|r| r.size_px).unwrap_or(16.0);
        baseline += size;

        // Concatenate run text for this paragraph line.
        let text: String = para.runs.iter().map(|r| r.text.as_str()).collect();
        let first = para.runs.first();
        let family = first.map(|r| r.family.as_str()).unwrap_or("sans-serif");
        let fill = first.map(|r| r.color).unwrap_or(Rgba::BLACK);
        let bold = first.map(|r| r.bold).unwrap_or(false);
        let italic = first.map(|r| r.italic).unwrap_or(false);

        out.push_str(&format!(
            "<text x=\"{}\" y=\"{}\" font-family=\"{}\" font-size=\"{}\"",
            bounds.x,
            baseline,
            esc(family),
            size
        ));
        push_paint(out, "fill", Some(fill));
        out.push_str(&format!(
            " font-weight=\"{}\" font-style=\"{}\"",
            if bold { "bold" } else { "normal" },
            if italic { "italic" } else { "normal" }
        ));
        out.push_str(&rot);
        out.push('>');
        out.push_str(&esc(&text));
        out.push_str("</text>");

        // Advance to the next paragraph line.
        baseline += size * 0.3;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::{
        Paint, Primitive, PxRect, PxSize, ResolvedParagraph, ResolvedRun, Scene, SceneNode,
        TextBlock,
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
}
