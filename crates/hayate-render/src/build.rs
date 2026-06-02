//! Build a [`Scene`] for one slide, resolving theme/inheritance/geometry into concrete
//! pixel-space primitives (DESIGN 6.7/6.14).

use crate::scene::{
    Paint, Primitive, PxSize, ResolvedParagraph, ResolvedRun, Scene, SceneNode, StrokePx,
    TextBlock, Viewport,
};
use hayate_ir::anim::{AnimKind, Trigger};
use hayate_ir::color::Rgba;
use hayate_ir::font::Script;
use hayate_ir::paint::Fill;
use hayate_ir::presentation::Presentation;
use hayate_ir::shape::{ArrowHead, Geometry};
use hayate_ir::text::TextBody;
use hayate_ir::theme::Theme;
use hayate_ir::world::Entity;

/// Build the scene for `slide` rendered to fit `target` pixels.
pub fn build_slide_scene(p: &Presentation, slide: Entity, target: PxSize) -> Scene {
    let vp = Viewport::fit(p.slide_size, target);
    let theme = p.theme_of(slide).cloned().unwrap_or_default();

    let background = p
        .background_of(slide)
        .map(|f| paint_to_rgba(&f, &theme))
        .unwrap_or(Rgba::WHITE);

    let mut nodes = Vec::new();
    for e in p.children(slide) {
        let frame = match p.world.frames.get(&e) {
            Some(f) => *f,
            None => continue, // shapes without geometry are skipped for now
        };
        let bounds = vp.rect(frame);
        let rotation_deg = p.world.rotations.get(&e).copied().unwrap_or(0.0);
        let opacity = p.world.opacity.get(&e).copied().unwrap_or(1.0);
        let fill = p.world.fills.get(&e).map(|f| fill_to_paint(f, &theme));
        let stroke = p.world.strokes.get(&e).map(|s| StrokePx {
            color: theme.resolve_color(&s.color),
            width: vp.len(s.width),
        });

        let prim = if let Some(tb) = p.world.texts.get(&e) {
            Primitive::Text(resolve_text(tb, &theme, &vp, bounds))
        } else if let Some(pic) = p.world.pictures.get(&e) {
            // A picture takes precedence over any geometry on the same entity.
            Primitive::Image {
                bounds,
                media_key: pic.media_key.clone(),
            }
        } else if let Some(geom) = p.world.geometries.get(&e) {
            match geom {
                Geometry::Ellipse => Primitive::Ellipse {
                    bounds,
                    fill,
                    stroke,
                },
                Geometry::Rect => Primitive::Quad {
                    bounds,
                    corner_radius: 0.0,
                    fill,
                    stroke,
                },
                Geometry::RoundRect { radius } => Primitive::Quad {
                    bounds,
                    corner_radius: vp.len(*radius),
                    fill,
                    stroke,
                },
                Geometry::Line { start, end } => {
                    // A line runs along the diagonal of its frame (top-left -> bottom-right).
                    // It has no fill; if the shape carries no stroke, synthesize a default
                    // 2pt dark stroke so the line is visible.
                    let stroke = stroke.or(Some(StrokePx {
                        color: Rgba::rgb(0x20, 0x20, 0x20),
                        width: vp.len(hayate_ir::units::pt(2)),
                    }));
                    Primitive::Line {
                        from: (bounds.x, bounds.y),
                        to: (bounds.x + bounds.w, bounds.y + bounds.h),
                        stroke,
                        start_arrow: matches!(start, ArrowHead::Arrow),
                        end_arrow: matches!(end, ArrowHead::Arrow),
                    }
                }
            }
        } else {
            continue;
        };

        nodes.push(SceneNode {
            source: Some(e),
            rotation_deg,
            opacity,
            prim,
        });
    }

    Scene {
        size: vp.size(p.slide_size),
        background,
        nodes,
    }
}

/// Build the scene for `slide` at playback time `t_ms`, applying any [`SlideTimeline`]
/// entrance animations as a per-target progress in 0..1.
///
/// Steps lay out on a single timeline: they play in order, and an `AfterPrev` (or, for
/// auto-play, `OnClick`) step starts after the previous step's maximum end, while a
/// `WithPrev` step starts at the same point the previous step started. Within a step each
/// anim begins at its own `delay`. For an `Entrance` anim the progress is
/// `clamp((t - step_start - delay) / duration, 0, 1)` (a zero duration snaps to 1).
///
/// Shapes with no entrance animation are left fully visible; a shape whose entrance has not
/// started yet (progress 0) is hidden (opacity 0).
pub fn build_slide_scene_at(p: &Presentation, slide: Entity, target: PxSize, t_ms: u32) -> Scene {
    let mut scene = build_slide_scene(p, slide, target);

    let timeline = match p.world.timelines.get(&slide) {
        Some(tl) => tl,
        None => return scene, // no animation: fully-visible frame
    };

    // Lay the steps out on an absolute timeline and accumulate, per target entity, the
    // progress of the entrance animation that governs its visibility.
    let mut entrance_progress: std::collections::BTreeMap<Entity, f32> =
        std::collections::BTreeMap::new();

    let mut prev_start: u32 = 0;
    let mut prev_end: u32 = 0;
    for (i, step) in timeline.steps.iter().enumerate() {
        // Resolve this step's start on the absolute timeline.
        let start = if i == 0 {
            0
        } else {
            match step.trigger {
                // WithPrev runs alongside the previous step.
                Trigger::WithPrev => prev_start,
                // AfterPrev waits for the previous step to finish, then waits `delay`.
                Trigger::AfterPrev { delay } => prev_end.saturating_add(delay),
                // For auto-play we treat OnClick like AfterPrev with no extra delay.
                Trigger::OnClick => prev_end,
            }
        };

        let mut step_end = start;
        for anim in &step.anims {
            let anim_start = start.saturating_add(anim.delay);
            let anim_end = anim_start.saturating_add(anim.duration);
            step_end = step_end.max(anim_end);

            if let AnimKind::Entrance(_effect) = anim.kind {
                let progress = if anim.duration == 0 {
                    if t_ms >= anim_start {
                        1.0
                    } else {
                        0.0
                    }
                } else {
                    let elapsed = t_ms as i64 - anim_start as i64;
                    (elapsed as f32 / anim.duration as f32).clamp(0.0, 1.0)
                };
                // If several entrance anims target the same entity, the last one wins.
                entrance_progress.insert(anim.target, progress);
            }
        }

        prev_start = start;
        prev_end = step_end;
    }

    // Apply the computed entrance progress to matching nodes.
    for node in &mut scene.nodes {
        if let Some(src) = node.source {
            if let Some(&progress) = entrance_progress.get(&src) {
                // Entrance(Fade) scales opacity. For MVP, other entrance effects (Fly, Wipe,
                // Zoom) also just fade until proper motion/clipping is implemented.
                node.opacity *= progress;
            }
        }
    }

    scene
}

/// Resolve a `Fill` to a single representative color (used for backgrounds, where a gradient
/// is collapsed to its `from` stop).
fn paint_to_rgba(fill: &Fill, theme: &Theme) -> Rgba {
    match fill {
        Fill::Solid(c) => theme.resolve_color(c),
        Fill::Linear { from, .. } => theme.resolve_color(from),
    }
}

/// Resolve a `Fill` to a scene `Paint`, resolving theme colors to literals.
fn fill_to_paint(fill: &Fill, theme: &Theme) -> Paint {
    match fill {
        Fill::Solid(c) => Paint::Solid(theme.resolve_color(c)),
        Fill::Linear {
            from,
            to,
            angle_deg,
        } => Paint::Linear {
            from: theme.resolve_color(from),
            to: theme.resolve_color(to),
            angle_deg: *angle_deg,
        },
    }
}

fn resolve_text(
    tb: &TextBody,
    theme: &Theme,
    vp: &Viewport,
    bounds: crate::scene::PxRect,
) -> TextBlock {
    let paragraphs = tb
        .paragraphs
        .iter()
        .map(|para| {
            let runs = para
                .runs
                .iter()
                .map(|r| {
                    let script = if r.text.chars().any(is_cjk) {
                        Script::Ea
                    } else {
                        Script::Latin
                    };
                    ResolvedRun {
                        text: r.text.clone(),
                        family: theme.font_family(&r.font, script),
                        size_px: vp.len(r.size),
                        color: theme.resolve_color(&r.color),
                        bold: r.bold,
                        italic: r.italic,
                        underline: r.underline,
                    }
                })
                .collect();
            ResolvedParagraph {
                runs,
                align: para.align,
            }
        })
        .collect();
    TextBlock { bounds, paragraphs }
}

/// Rough CJK detection to pick the East-Asian font slot. Covers the common ranges
/// (Hiragana/Katakana/CJK ideographs); a precise classifier can replace this later.
fn is_cjk(ch: char) -> bool {
    matches!(ch as u32,
        0x3040..=0x30FF      // Hiragana + Katakana
        | 0x3400..=0x4DBF    // CJK Ext A
        | 0x4E00..=0x9FFF    // CJK Unified
        | 0xFF00..=0xFFEF    // Halfwidth/Fullwidth forms
    )
}

#[cfg(test)]
mod tests;
