//! Build a [`Scene`] for one slide, resolving theme/inheritance/geometry into concrete
//! pixel-space primitives (DESIGN 6.7/6.14).

use crate::scene::{
    Paint, Primitive, PxRect, PxSize, ResolvedParagraph, ResolvedRun, Scene, SceneNode, StrokePx,
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

/// The dim prompt shown in an empty placeholder, by type (mirrors PowerPoint's "Click to add…").
pub fn prompt_text(ph: hayate_ir::doc::PlaceholderType) -> &'static str {
    use hayate_ir::doc::PlaceholderType as PT;
    match ph {
        PT::Title | PT::CenteredTitle => "Click to add title",
        PT::Subtitle => "Click to add subtitle",
        PT::Body => "Click to add text",
        PT::Picture => "Click to add a picture",
        PT::Chart => "Click to add a chart",
        PT::Table => "Click to add a table",
        PT::Date => "Date",
        PT::Footer => "Footer",
        PT::SlideNumber => "#",
    }
}

/// A one-run `TextBody` carrying the placeholder prompt in a muted gray, sized by type.
fn prompt_body(ph: hayate_ir::doc::PlaceholderType) -> TextBody {
    use hayate_ir::color::Color;
    use hayate_ir::doc::PlaceholderType as PT;
    use hayate_ir::font::{FontRef, ThemeFontSlot};
    use hayate_ir::text::{Paragraph, Run};
    let (slot, size) = match ph {
        PT::Title | PT::CenteredTitle => (ThemeFontSlot::Major, hayate_ir::units::pt(40)),
        _ => (ThemeFontSlot::Minor, hayate_ir::units::pt(24)),
    };
    TextBody {
        paragraphs: vec![Paragraph::new(vec![Run {
            text: prompt_text(ph).to_string(),
            font: FontRef::Theme(slot),
            size,
            color: Color::Literal(Rgba::rgb(0x9a, 0xa0, 0xa6)),
            bold: false,
            italic: false,
            underline: false,
        }])],
        autofit: false,
    }
}

/// Build a vector-shape primitive from a resolved geometry, bounds, and paints. Shared by the
/// slide's own shapes and by inherited placeholders. A line with no stroke gets a default 2pt
/// dark stroke so it stays visible.
fn geometry_prim(
    geom: &Geometry,
    bounds: PxRect,
    fill: Option<Paint>,
    stroke: Option<StrokePx>,
    vp: &Viewport,
) -> Primitive {
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
}

/// Build the scene for `slide` rendered to fit `target` pixels.
pub fn build_slide_scene(p: &Presentation, slide: Entity, target: PxSize) -> Scene {
    let vp = Viewport::fit(p.slide_size, target);
    let default;
    let theme = match p.theme_of(slide) {
        Some(t) => t,
        None => {
            default = Theme::default();
            &default
        }
    };

    let background = p
        .background_of(slide)
        .map(|f| paint_to_rgba(&f, theme))
        .unwrap_or(Rgba::WHITE);

    let mut nodes = Vec::new();

    // Master/layout decorations (their non-placeholder shapes, e.g. logos or accent bars) render
    // behind everything and are display-only on the slide, so editing the master or layout
    // updates every slide that uses it.
    if let Some(master) = p.owning_master(slide) {
        push_raw_children(p, master, theme, &vp, true, false, 1.0, &mut nodes);
    }
    if let Some(layout) = p.layout_of(slide) {
        push_raw_children(p, layout, theme, &vp, true, false, 1.0, &mut nodes);
    }

    // Inherited placeholders are drawn first (behind the slide's own content). Each
    // placeholder's fields resolve slide -> layout -> master independently, so a frame may come
    // from the layout while the text comes from the slide. A placeholder the slide overrides
    // directly is selectable (source = the slide entity); a purely inherited one is display-only.
    for ph in p.effective_placeholders(slide) {
        let Some(frame) = p.ph_frame(slide, ph) else {
            continue;
        };
        let bounds = vp.rect(frame);
        let fill = p.ph_fill(slide, ph).map(|f| fill_to_paint(&f, theme));
        let prim = if let Some(tb) = p.ph_text(slide, ph) {
            Primitive::Text(resolve_text(tb, theme, &vp, bounds))
        } else if let Some(geom) = p.ph_geometry(slide, ph) {
            geometry_prim(&geom, bounds, fill, None, &vp)
        } else {
            // An empty placeholder shows a dim prompt ("Click to add title", etc.).
            let body = prompt_body(ph.ph_type);
            Primitive::Text(resolve_text(&body, theme, &vp, bounds))
        };
        let source = p.find_placeholder(slide, ph);
        let rotation_deg = source
            .and_then(|e| p.world.rotations.get(&e).copied())
            .unwrap_or(0.0);
        nodes.push(SceneNode {
            source,
            rotation_deg,
            opacity: 1.0,
            prim,
        });
    }

    // The slide's own non-placeholder shapes (placeholders were handled above).
    push_raw_children(p, slide, theme, &vp, true, true, 1.0, &mut nodes);

    Scene {
        size: vp.size(p.slide_size),
        background,
        nodes,
    }
}

/// Render a container's direct children straight from their own components (frame/text/picture/
/// geometry), appending one [`SceneNode`] each. `skip_placeholders` excludes placeholder shapes
/// (used by the slide path, which renders placeholders via the inheritance resolvers instead);
/// `selectable` sets each node's `source` (None makes it display-only context); `opacity_mul`
/// scales opacity (used to dim ancestor context when editing a layout/master).
#[allow(clippy::too_many_arguments)]
fn push_raw_children(
    p: &Presentation,
    container: Entity,
    theme: &Theme,
    vp: &Viewport,
    skip_placeholders: bool,
    selectable: bool,
    opacity_mul: f32,
    nodes: &mut Vec<SceneNode>,
) {
    for e in p.children(container) {
        if skip_placeholders && p.world.placeholders.contains_key(&e) {
            continue;
        }
        let frame = match p.world.frames.get(&e) {
            Some(f) => *f,
            None => continue,
        };
        let bounds = vp.rect(frame);
        let rotation_deg = p.world.rotations.get(&e).copied().unwrap_or(0.0);
        let opacity = p.world.opacity.get(&e).copied().unwrap_or(1.0) * opacity_mul;
        let fill = p.world.fills.get(&e).map(|f| fill_to_paint(f, theme));
        let stroke = p.world.strokes.get(&e).map(|s| StrokePx {
            color: theme.resolve_color(&s.color),
            width: vp.len(s.width),
        });
        let prim = if let Some(tb) = p.world.texts.get(&e) {
            Primitive::Text(resolve_text(tb, theme, vp, bounds))
        } else if let Some(pic) = p.world.pictures.get(&e) {
            Primitive::Image {
                bounds,
                media_key: pic.media_key.clone(),
            }
        } else if let Some(geom) = p.world.geometries.get(&e) {
            geometry_prim(geom, bounds, fill, stroke, vp)
        } else {
            continue;
        };
        nodes.push(SceneNode {
            source: selectable.then_some(e),
            rotation_deg,
            opacity,
            prim,
        });
    }
}

/// Build a scene for editing a non-slide container (a layout or master) in place. `context`
/// lists ancestor containers (e.g. the owning master when editing a layout) whose shapes render
/// dimmed and display-only; the `container` itself renders fully and selectable. The caller
/// resolves `theme`/`background` via [`Presentation::container_theme`]/`container_background`
/// (a bare layout/master has no `SlideInfo`).
pub fn build_container_scene(
    p: &Presentation,
    container: Entity,
    theme: &Theme,
    background: Option<Fill>,
    context: &[Entity],
    target: PxSize,
) -> Scene {
    let vp = Viewport::fit(p.slide_size, target);
    let background = background
        .map(|f| paint_to_rgba(&f, theme))
        .unwrap_or(Rgba::WHITE);
    let mut nodes = Vec::new();
    for &anc in context {
        push_raw_children(p, anc, theme, &vp, false, false, 0.5, &mut nodes);
    }
    push_raw_children(p, container, theme, &vp, false, true, 1.0, &mut nodes);
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
                bullet_level: para.bullet_level,
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
