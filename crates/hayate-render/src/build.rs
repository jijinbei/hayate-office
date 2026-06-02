//! Build a [`Scene`] for one slide, resolving theme/inheritance/geometry into concrete
//! pixel-space primitives (DESIGN 6.7/6.14).

use crate::scene::{
    Paint, Primitive, PxSize, ResolvedParagraph, ResolvedRun, Scene, SceneNode, StrokePx,
    TextBlock, Viewport,
};
use hayate_ir::color::Rgba;
use hayate_ir::font::Script;
use hayate_ir::paint::Fill;
use hayate_ir::presentation::Presentation;
use hayate_ir::shape::Geometry;
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
        let fill = p.world.fills.get(&e).map(|f| Paint::Solid(paint_to_rgba(f, &theme)));
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

fn paint_to_rgba(fill: &Fill, theme: &Theme) -> Rgba {
    match fill {
        Fill::Solid(c) => theme.resolve_color(c),
    }
}

fn resolve_text(tb: &TextBody, theme: &Theme, vp: &Viewport, bounds: crate::scene::PxRect) -> TextBlock {
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
mod tests {
    use super::*;
    use hayate_ir::color::{Color, Rgba, ThemeColorToken};
    use hayate_ir::font::{FontRef, ThemeFontSlot};
    use hayate_ir::geom::RectEmu;
    use hayate_ir::text::{Paragraph, Run};
    use hayate_ir::units::pt;

    fn deck() -> (Presentation, Entity) {
        let mut p = Presentation::new();
        let master = p.add_master(Theme::default());
        let layout = p.add_layout(master, "Blank");
        let slide = p.add_slide(layout);

        // A rectangle filled with accent1.
        let rect = p.add_shape(slide);
        p.world.frames.insert(rect, RectEmu::new(0, 0, 914_400, 914_400));
        p.world.geometries.insert(rect, Geometry::Rect);
        p.world
            .fills
            .insert(rect, Fill::Solid(Color::theme(ThemeColorToken::Accent1)));

        // A text box with a Japanese run.
        let text = p.add_shape(slide);
        p.world.frames.insert(text, RectEmu::new(0, 914_400, 5_000_000, 914_400));
        p.world.texts.insert(
            text,
            TextBody {
                paragraphs: vec![Paragraph::new(vec![Run {
                    text: "こんにちは".to_string(),
                    font: FontRef::Theme(ThemeFontSlot::Minor),
                    size: pt(24),
                    color: Color::literal(Rgba::BLACK),
                    bold: false,
                    italic: false,
                    underline: false,
                }])],
                autofit: false,
            },
        );

        (p, slide)
    }

    #[test]
    fn builds_nodes_in_order() {
        let (p, slide) = deck();
        let scene = build_slide_scene(&p, slide, PxSize { w: 960.0, h: 540.0 });
        assert_eq!(scene.nodes.len(), 2);
        // Aspect-fit of a 16:9 slide into 960x540 keeps full width.
        assert!((scene.size.w - 960.0).abs() < 1.0);
    }

    #[test]
    fn resolves_fill_to_theme_color() {
        let (p, slide) = deck();
        let scene = build_slide_scene(&p, slide, PxSize { w: 960.0, h: 540.0 });
        let theme = Theme::default();
        match &scene.nodes[0].prim {
            Primitive::Quad { fill: Some(Paint::Solid(c)), .. } => {
                assert_eq!(*c, theme.color_for(ThemeColorToken::Accent1));
            }
            other => panic!("expected filled quad, got {other:?}"),
        }
    }

    #[test]
    fn japanese_run_uses_ea_font() {
        let (p, slide) = deck();
        let scene = build_slide_scene(&p, slide, PxSize { w: 960.0, h: 540.0 });
        match &scene.nodes[1].prim {
            Primitive::Text(tb) => {
                let run = &tb.paragraphs[0].runs[0];
                assert_eq!(run.family, "Noto Sans JP");
                assert!(run.size_px > 0.0);
            }
            other => panic!("expected text, got {other:?}"),
        }
    }

    #[test]
    fn picture_with_frame_yields_image_node() {
        use hayate_ir::geom::SizeEmu;
        use hayate_ir::image::PictureRef;

        let mut p = Presentation::new();
        let master = p.add_master(Theme::default());
        let layout = p.add_layout(master, "Blank");
        let slide = p.add_slide(layout);

        // An entity with both a picture and a (competing) geometry: picture wins.
        let pic = p.add_shape(slide);
        p.world.frames.insert(pic, RectEmu::new(0, 0, 914_400, 914_400));
        p.world.geometries.insert(pic, Geometry::Rect);
        p.world.pictures.insert(
            pic,
            PictureRef {
                media_key: "sha256:logo".to_string(),
                natural: SizeEmu::new(640, 480),
            },
        );

        let scene = build_slide_scene(&p, slide, PxSize { w: 960.0, h: 540.0 });
        assert_eq!(scene.nodes.len(), 1);
        match &scene.nodes[0].prim {
            Primitive::Image { media_key, bounds } => {
                assert_eq!(media_key, "sha256:logo");
                assert!(bounds.w > 0.0 && bounds.h > 0.0);
            }
            other => panic!("expected image, got {other:?}"),
        }
    }
}
