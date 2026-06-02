//! Unit tests for the parent module.

use super::*;
use crate::color::{Color, Rgba, ThemeColorToken};
use crate::doc::{PlaceholderRef, PlaceholderType};
use crate::text::{Paragraph, Run, TextBody};
use crate::world::CompValue;

fn title_ref() -> PlaceholderRef {
    PlaceholderRef {
        ph_type: PlaceholderType::Title,
        idx: 0,
    }
}

fn simple_text(s: &str) -> TextBody {
    let run = Run {
        text: s.to_string(),
        font: crate::font::FontRef::Theme(crate::font::ThemeFontSlot::Major),
        size: crate::units::pt(44),
        color: Color::theme(ThemeColorToken::Dk1),
        bold: false,
        italic: false,
        underline: false,
    };
    TextBody {
        paragraphs: vec![Paragraph::new(vec![run])],
        autofit: false,
    }
}

/// Add a placeholder shape under `container` with the given ref, optional frame, optional text.
fn add_placeholder(
    p: &mut Presentation,
    container: Entity,
    ph: PlaceholderRef,
    frame: Option<crate::geom::RectEmu>,
    text: Option<TextBody>,
) -> Entity {
    let e = p.add_shape(container);
    p.world.set(e, CompValue::Placeholder(ph));
    if let Some(f) = frame {
        p.world.set(e, CompValue::Frame(f));
    }
    if let Some(t) = text {
        p.world.set(e, CompValue::Text(t));
    }
    e
}

#[test]
fn placeholder_inheritance_resolves_frame_and_text_separately() {
    let (mut p, s1, _) = small_deck();
    let layout = p.layout_of(s1).unwrap();
    let ph = title_ref();

    // Title placeholder WITH a frame on the LAYOUT (no text).
    let layout_frame = crate::geom::RectEmu::new(10, 20, 300, 100);
    add_placeholder(&mut p, layout, ph, Some(layout_frame), None);
    // Title placeholder WITH text but NO frame on the SLIDE.
    add_placeholder(&mut p, s1, ph, None, Some(simple_text("Hello")));

    // Frame comes from the layout; text comes from the slide.
    assert_eq!(p.ph_frame(s1, ph), Some(layout_frame));
    let text = p.ph_text(s1, ph).expect("text resolved");
    assert_eq!(text.paragraphs[0].runs[0].text, "Hello");
}

#[test]
fn effective_placeholders_dedupes_title() {
    let (mut p, s1, _) = small_deck();
    let layout = p.layout_of(s1).unwrap();
    let ph = title_ref();
    add_placeholder(
        &mut p,
        layout,
        ph,
        Some(crate::geom::RectEmu::new(0, 0, 1, 1)),
        None,
    );
    add_placeholder(&mut p, s1, ph, None, Some(simple_text("Hi")));

    let effective = p.effective_placeholders(s1);
    let titles = effective
        .iter()
        .filter(|r| r.ph_type == PlaceholderType::Title && r.idx == 0)
        .count();
    assert_eq!(titles, 1);
}

fn small_deck() -> (Presentation, Entity, Entity) {
    let mut p = Presentation::new();
    let master = p.add_master(Theme::default());
    let layout = p.add_layout(master, "Title and Content");
    let s1 = p.add_slide(layout);
    let s2 = p.add_slide(layout);
    (p, s1, s2)
}

#[test]
fn slides_are_ordered_by_creation() {
    let (p, s1, s2) = small_deck();
    assert_eq!(p.slides(), vec![s1, s2]);
}

#[test]
fn children_are_ordered() {
    let (mut p, s1, _) = small_deck();
    let a = p.add_shape(s1);
    let b = p.add_shape(s1);
    let c = p.add_shape(s1);
    assert_eq!(p.children(s1), vec![a, b, c]);
}

#[test]
fn container_resolvers_match_slide_resolvers() {
    let (mut p, s1, _) = small_deck();
    let layout = p.layout_of(s1).unwrap();
    let master = p.master_of(s1).unwrap();
    // owning_master resolves for every container kind.
    assert_eq!(p.owning_master(s1), Some(master));
    assert_eq!(p.owning_master(layout), Some(master));
    assert_eq!(p.owning_master(master), Some(master));
    // container_theme(slide) is the same theme as theme_of(slide).
    assert!(std::ptr::eq(
        p.container_theme(s1).unwrap(),
        p.theme_of(s1).unwrap()
    ));
    // Background set on the master is seen by every container in the chain.
    p.world
        .backgrounds
        .insert(master, Fill::Solid(Color::theme(ThemeColorToken::Lt1)));
    assert!(p.container_background(s1).is_some());
    assert!(p.container_background(layout).is_some());
    assert!(p.container_background(master).is_some());
}

#[test]
fn inheritance_resolves_to_master_and_theme() {
    let (mut p, s1, _) = small_deck();
    let master = p.master_of(s1).unwrap();
    // Background set only on the master is inherited by the slide.
    p.world
        .backgrounds
        .insert(master, Fill::Solid(Color::theme(ThemeColorToken::Lt1)));
    assert!(p.background_of(s1).is_some());
    // Theme resolves through master.
    let theme = p.theme_of(s1).unwrap();
    assert_eq!(
        theme.resolve_color(&Color::theme(ThemeColorToken::Lt1)),
        Rgba::WHITE
    );
}

#[test]
fn add_media_roundtrips_bytes() {
    let mut p = Presentation::new();
    let bytes = vec![1u8, 2, 3, 4, 5];
    let key = p.add_media(bytes.clone());
    assert!(!key.is_empty());
    assert_eq!(p.get_media(&key), Some(&bytes));
}

#[test]
fn add_media_is_idempotent_by_content() {
    let mut p = Presentation::new();
    let key1 = p.add_media(vec![10u8, 20, 30]);
    let key2 = p.add_media(vec![10u8, 20, 30]);
    assert_eq!(key1, key2);
    assert_eq!(p.media.len(), 1);
}
