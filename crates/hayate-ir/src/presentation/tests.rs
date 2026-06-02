//! Unit tests for the parent module.

use super::*;
use crate::color::{Color, Rgba, ThemeColorToken};

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
