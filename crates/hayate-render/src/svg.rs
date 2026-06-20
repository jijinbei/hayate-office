//! Standalone SVG export of a `Scene` (DESIGN: headless/offscreen rendering). gpui-free and
//! dependency-free: it just serializes the display list to an SVG document string, which is
//! handy for thumbnails, debugging, and vector export.

use crate::scene::{Paint, Primitive, PxRect, Scene, SceneNode, StrokePx};
use hayate_ir::color::Rgba;
use std::fmt::Write as _;

/// Escape XML special characters in text content / attribute values, writing directly into `out`.
fn push_escaped(out: &mut String, s: &str) {
    for ch in s.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            _ => out.push(ch),
        }
    }
}

/// Append a `fill`/`stroke`-style paint attribute plus its opacity (when alpha < 255).
/// `kind` is "fill" or "stroke".
fn push_paint(out: &mut String, kind: &str, c: Option<Rgba>) {
    match c {
        Some(c) => {
            let _ = write!(out, " {}=\"rgb({},{},{})\"", kind, c.r, c.g, c.b);
            if c.a < 255 {
                let opacity = c.a as f32 / 255.0;
                let _ = write!(out, " {}-opacity=\"{}\"", kind, opacity);
            }
        }
        None => {
            let _ = write!(out, " {}=\"none\"", kind);
        }
    }
}

fn paint_color(p: &Option<Paint>) -> Option<Rgba> {
    // SVG export is solid-only for now; a linear gradient is approximated by its start color.
    p.as_ref().map(|paint| match paint {
        Paint::Solid(c) => *c,
        Paint::Linear { from, .. } => *from,
    })
}

/// Write a `transform="rotate(deg cx cy)"` attribute for a node whose primitive occupies
/// `bounds`, or nothing when rotation is ~zero.
fn push_rotation(out: &mut String, rotation_deg: f32, bounds: PxRect) {
    if rotation_deg.abs() < f32::EPSILON {
        return;
    }
    let cx = bounds.x + bounds.w / 2.0;
    let cy = bounds.y + bounds.h / 2.0;
    let _ = write!(out, " transform=\"rotate({} {} {})\"", rotation_deg, cx, cy);
}

/// Export a `Scene` to a standalone SVG document string.
pub fn export_svg(scene: &Scene) -> String {
    let w = scene.size.w;
    let h = scene.size.h;
    let mut out = String::new();

    let _ = write!(
        out,
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{}\" height=\"{}\" viewBox=\"0 0 {} {}\">",
        w, h, w, h
    );

    // Background rect filling the canvas.
    let _ = write!(
        out,
        "<rect x=\"0\" y=\"0\" width=\"{}\" height=\"{}\"",
        w, h
    );
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
        Primitive::Line {
            from,
            to,
            stroke,
            start_arrow: _,
            end_arrow: _,
        } => emit_line(out, node, *from, *to, stroke),
        // SVG export of Typst boxes is not implemented; omit (on-screen/raster previews show it).
        Primitive::Typst { .. } => {}
    }
}

/// Emit an SVG `<line>` between the two endpoints. The arrowhead is omitted for simplicity
/// (the kind round-trips through the IR geometry, not the SVG).
fn emit_line(
    out: &mut String,
    node: &SceneNode,
    from: (f32, f32),
    to: (f32, f32),
    stroke: &Option<StrokePx>,
) {
    let _ = write!(
        out,
        "<line x1=\"{}\" y1=\"{}\" x2=\"{}\" y2=\"{}\"",
        from.0, from.1, to.0, to.1
    );
    push_stroke(out, stroke);
    push_rotation(
        out,
        node.rotation_deg,
        crate::scene::prim_bounds(&node.prim),
    );
    out.push_str("/>");
}

/// Emit a light-gray `<rect>` placeholder for an image (actual pixels are resolved elsewhere).
fn emit_image(out: &mut String, node: &SceneNode, bounds: PxRect) {
    let _ = write!(
        out,
        "<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\"",
        bounds.x, bounds.y, bounds.w, bounds.h
    );
    push_paint(out, "fill", Some(Rgba::rgb(220, 220, 220)));
    push_rotation(out, node.rotation_deg, bounds);
    out.push_str("/>");
}

fn push_stroke(out: &mut String, stroke: &Option<StrokePx>) {
    match stroke {
        Some(s) => {
            push_paint(out, "stroke", Some(s.color));
            let _ = write!(out, " stroke-width=\"{}\"", s.width);
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
    let _ = write!(
        out,
        "<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\"",
        bounds.x, bounds.y, bounds.w, bounds.h
    );
    if corner_radius > 0.0 {
        let _ = write!(out, " rx=\"{}\"", corner_radius);
    }
    push_paint(out, "fill", paint_color(fill));
    push_stroke(out, stroke);
    push_rotation(out, node.rotation_deg, bounds);
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
    let _ = write!(
        out,
        "<ellipse cx=\"{}\" cy=\"{}\" rx=\"{}\" ry=\"{}\"",
        cx, cy, rx, ry
    );
    push_paint(out, "fill", paint_color(fill));
    push_stroke(out, stroke);
    push_rotation(out, node.rotation_deg, bounds);
    out.push_str("/>");
}

fn emit_text(out: &mut String, node: &SceneNode, block: &crate::scene::TextBlock) {
    let bounds = block.bounds;
    let mut rot = String::new();
    push_rotation(&mut rot, node.rotation_deg, bounds);

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

        let _ = write!(
            out,
            "<text x=\"{}\" y=\"{}\" font-family=\"",
            bounds.x, baseline
        );
        push_escaped(out, family);
        let _ = write!(out, "\" font-size=\"{}\"", size);
        push_paint(out, "fill", Some(fill));
        let _ = write!(
            out,
            " font-weight=\"{}\" font-style=\"{}\"",
            if bold { "bold" } else { "normal" },
            if italic { "italic" } else { "normal" }
        );
        out.push_str(&rot);
        out.push('>');
        push_escaped(out, &text);
        out.push_str("</text>");

        // Advance to the next paragraph line.
        baseline += size * 0.3;
    }
}

#[cfg(test)]
mod tests;
