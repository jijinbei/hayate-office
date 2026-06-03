//! Unit tests for the parent module.

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
    p.world
        .frames
        .insert(rect, RectEmu::new(0, 0, 914_400, 914_400));
    p.world.geometries.insert(rect, Geometry::Rect);
    p.world
        .fills
        .insert(rect, Fill::Solid(Color::theme(ThemeColorToken::Accent1)));

    // A text box with a Japanese run.
    let text = p.add_shape(slide);
    p.world
        .frames
        .insert(text, RectEmu::new(0, 914_400, 5_000_000, 914_400));
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
        Primitive::Quad {
            fill: Some(Paint::Solid(c)),
            ..
        } => {
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

/// Build a slide with a single faded-in rectangle and a static (un-animated) rectangle.
/// Returns (presentation, slide, animated_rect, static_rect).
fn animated_deck() -> (Presentation, Entity, Entity, Entity) {
    use hayate_ir::anim::{Anim, AnimStep, Easing, Effect, SlideTimeline};

    let mut p = Presentation::new();
    let master = p.add_master(Theme::default());
    let layout = p.add_layout(master, "Blank");
    let slide = p.add_slide(layout);

    let animated = p.add_shape(slide);
    p.world
        .frames
        .insert(animated, RectEmu::new(0, 0, 914_400, 914_400));
    p.world.geometries.insert(animated, Geometry::Rect);

    let still = p.add_shape(slide);
    p.world
        .frames
        .insert(still, RectEmu::new(914_400, 0, 914_400, 914_400));
    p.world.geometries.insert(still, Geometry::Rect);

    // One entrance fade over [0, 1000ms] targeting `animated`.
    p.world.timelines.insert(
        slide,
        SlideTimeline {
            steps: vec![AnimStep {
                trigger: Trigger::OnClick,
                anims: vec![Anim {
                    target: animated,
                    kind: AnimKind::Entrance(Effect::Fade),
                    duration: 1000,
                    delay: 0,
                    easing: Easing::Linear,
                }],
            }],
        },
    );

    (p, slide, animated, still)
}

/// Locate the opacity of the node whose source matches `e`.
fn opacity_of(scene: &Scene, e: Entity) -> f32 {
    scene
        .nodes
        .iter()
        .find(|n| n.source == Some(e))
        .expect("node for entity")
        .opacity
}

#[test]
fn entrance_fade_ramps_opacity_over_time() {
    let (p, slide, animated, _still) = animated_deck();
    let target = PxSize { w: 960.0, h: 540.0 };

    let at0 = build_slide_scene_at(&p, slide, target, 0);
    assert!(opacity_of(&at0, animated) < 0.01, "t=0 should be ~0");

    let at500 = build_slide_scene_at(&p, slide, target, 500);
    assert!(
        (opacity_of(&at500, animated) - 0.5).abs() < 0.01,
        "t=500 should be ~0.5"
    );

    let at1000 = build_slide_scene_at(&p, slide, target, 1000);
    assert!(
        (opacity_of(&at1000, animated) - 1.0).abs() < 0.01,
        "t=1000 should be ~1"
    );
}

#[test]
fn shape_without_animation_stays_visible() {
    let (p, slide, _animated, still) = animated_deck();
    let target = PxSize { w: 960.0, h: 540.0 };

    for t in [0, 500, 1000, 5000] {
        let scene = build_slide_scene_at(&p, slide, target, t);
        assert_eq!(
            opacity_of(&scene, still),
            1.0,
            "static shape stays opaque at t={t}"
        );
    }
}

#[test]
fn build_slide_scene_unchanged_without_timeline() {
    let (p, slide) = deck();
    let target = PxSize { w: 960.0, h: 540.0 };
    // No timeline => the time-parameterized builder matches the static frame exactly.
    assert_eq!(
        build_slide_scene_at(&p, slide, target, 0),
        build_slide_scene(&p, slide, target)
    );
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
    p.world
        .frames
        .insert(pic, RectEmu::new(0, 0, 914_400, 914_400));
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

#[test]
fn line_geometry_builds_line_primitive() {
    let mut p = Presentation::new();
    let master = p.add_master(Theme::default());
    let layout = p.add_layout(master, "Blank");
    let slide = p.add_slide(layout);

    // An arrow with no explicit stroke: build should synthesize a default stroke and emit a
    // Line primitive whose endpoints span the frame's diagonal (top-left -> bottom-right).
    let line = p.add_shape(slide);
    p.world
        .frames
        .insert(line, RectEmu::new(0, 0, 914_400, 914_400));
    p.world.geometries.insert(
        line,
        Geometry::Line {
            start: ArrowHead::None,
            end: ArrowHead::Arrow,
        },
    );

    let scene = build_slide_scene(&p, slide, PxSize { w: 960.0, h: 540.0 });
    assert_eq!(scene.nodes.len(), 1);
    match &scene.nodes[0].prim {
        Primitive::Line {
            from,
            to,
            stroke,
            start_arrow,
            end_arrow,
        } => {
            assert!(!*start_arrow);
            assert!(*end_arrow);
            assert!(from.0 < to.0 && from.1 < to.1);
            // A default stroke is synthesized when the shape carries none.
            assert!(stroke.is_some());
        }
        other => panic!("expected line, got {other:?}"),
    }
}

#[test]
fn container_scene_renders_context_and_editable() {
    use hayate_ir::doc::{PlaceholderRef, PlaceholderType};
    let mut p = Presentation::new();
    let master = p.add_master(Theme::default());
    let layout = p.add_layout(master, "Title and Content");

    // A placeholder defined on the master (context) and one on the layout (editable).
    let title = PlaceholderRef {
        ph_type: PlaceholderType::Title,
        idx: 0,
    };
    let body = PlaceholderRef {
        ph_type: PlaceholderType::Body,
        idx: 0,
    };
    let m_ph = p.add_shape(master);
    p.world
        .frames
        .insert(m_ph, RectEmu::new(0, 0, 914_400, 914_400));
    p.world.geometries.insert(m_ph, Geometry::Rect);
    p.world.placeholders.insert(m_ph, title);
    let l_ph = p.add_shape(layout);
    p.world
        .frames
        .insert(l_ph, RectEmu::new(0, 914_400, 914_400, 914_400));
    p.world.geometries.insert(l_ph, Geometry::Rect);
    p.world.placeholders.insert(l_ph, body);

    let theme = p.container_theme(layout).cloned().unwrap_or_default();
    let bg = p.container_background(layout);
    let scene = build_container_scene(
        &p,
        layout,
        &theme,
        bg,
        &[master],
        PxSize { w: 960.0, h: 540.0 },
    );

    // The master placeholder is display-only context (source None); the layout's own is selectable.
    assert!(
        scene.nodes.iter().any(|n| n.source.is_none()),
        "master context node present"
    );
    assert!(
        scene.nodes.iter().any(|n| n.source == Some(l_ph)),
        "layout placeholder selectable"
    );

    // Editing the master shows only its own children (no context).
    let mscene = build_container_scene(&p, master, &theme, bg, &[], PxSize { w: 960.0, h: 540.0 });
    assert_eq!(mscene.nodes.len(), 1);
    assert_eq!(mscene.nodes[0].source, Some(m_ph));
}

#[test]
fn empty_placeholder_renders_prompt_text() {
    use hayate_ir::doc::{PlaceholderRef, PlaceholderType};
    let mut p = Presentation::new();
    let master = p.add_master(Theme::default());
    let layout = p.add_layout(master, "Title and Content");
    let slide = p.add_slide(layout);
    // A Title placeholder with a frame but NO text, defined on the layout.
    let ph = p.add_shape(layout);
    p.world
        .frames
        .insert(ph, RectEmu::new(0, 0, 5_000_000, 914_400));
    p.world.placeholders.insert(
        ph,
        PlaceholderRef {
            ph_type: PlaceholderType::Title,
            idx: 0,
        },
    );
    let scene = build_slide_scene(&p, slide, PxSize { w: 960.0, h: 540.0 });
    let has_prompt = scene.nodes.iter().any(|n| match &n.prim {
        Primitive::Text(tb) => tb
            .paragraphs
            .iter()
            .flat_map(|para| para.runs.iter())
            .any(|r| r.text == "Click to add title"),
        _ => false,
    });
    assert!(has_prompt, "an empty Title placeholder shows its prompt");

    // Once the slide overrides it with real text, the prompt is gone.
    let ov = p.add_shape(slide);
    p.world
        .frames
        .insert(ov, RectEmu::new(0, 0, 5_000_000, 914_400));
    p.world.placeholders.insert(
        ov,
        PlaceholderRef {
            ph_type: PlaceholderType::Title,
            idx: 0,
        },
    );
    p.world.texts.insert(
        ov,
        TextBody {
            paragraphs: vec![Paragraph::new(vec![Run {
                text: "Real title".to_string(),
                font: FontRef::Theme(ThemeFontSlot::Major),
                size: pt(40),
                color: Color::theme(ThemeColorToken::Dk1),
                bold: true,
                italic: false,
                underline: false,
            }])],
            autofit: false,
        },
    );
    let scene2 = build_slide_scene(&p, slide, PxSize { w: 960.0, h: 540.0 });
    let still_prompt = scene2.nodes.iter().any(|n| match &n.prim {
        Primitive::Text(tb) => tb
            .paragraphs
            .iter()
            .flat_map(|para| para.runs.iter())
            .any(|r| r.text == "Click to add title"),
        _ => false,
    });
    assert!(!still_prompt, "the prompt disappears once real text is set");
}

#[test]
fn bullet_level_flows_into_resolved_paragraph() {
    let mut p = Presentation::new();
    let master = p.add_master(Theme::default());
    let layout = p.add_layout(master, "Blank");
    let slide = p.add_slide(layout);
    let tx = p.add_shape(slide);
    p.world
        .frames
        .insert(tx, RectEmu::new(0, 0, 5_000_000, 3_000_000));
    let mut body = TextBody {
        paragraphs: vec![Paragraph::new(vec![Run {
            text: "Item".to_string(),
            font: FontRef::Theme(ThemeFontSlot::Minor),
            size: pt(24),
            color: Color::theme(ThemeColorToken::Dk1),
            bold: false,
            italic: false,
            underline: false,
        }])],
        autofit: false,
    };
    body.paragraphs[0].bullet_level = 2;
    p.world.texts.insert(tx, body);
    let scene = build_slide_scene(&p, slide, PxSize { w: 960.0, h: 540.0 });
    let lvl = scene.nodes.iter().find_map(|n| match &n.prim {
        Primitive::Text(tb) => tb.paragraphs.first().map(|pp| pp.bullet_level),
        _ => None,
    });
    assert_eq!(
        lvl,
        Some(2),
        "the bullet level reaches the resolved paragraph"
    );
}
