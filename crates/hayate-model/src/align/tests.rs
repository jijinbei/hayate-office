//! Unit tests for the parent module.

use super::*;
use crate::history::History;

/// Spawn an entity with the given frame and return it.
fn spawn_framed(w: &mut World, x: i64, y: i64, width: i64, height: i64) -> Entity {
    let e = w.spawn();
    w.set(e, CompValue::Frame(RectEmu::new(x, y, width, height)));
    e
}

fn frame_of(w: &World, e: Entity) -> RectEmu {
    match w.get(e, CompKind::Frame) {
        Some(CompValue::Frame(f)) => f,
        _ => panic!("entity has no frame"),
    }
}

#[test]
fn align_left_sets_all_x_to_min() {
    let mut w = World::new();
    let mut h = History::new();
    let a = spawn_framed(&mut w, 10, 0, 100, 50);
    let b = spawn_framed(&mut w, 30, 100, 40, 50);
    let c = spawn_framed(&mut w, 70, 200, 60, 50);

    let tx = align(&w, &[a, b, c], Align::Left);
    h.commit(&mut w, tx);

    // All left edges now sit at the minimum x (10); y and size are untouched.
    assert_eq!(frame_of(&w, a), RectEmu::new(10, 0, 100, 50));
    assert_eq!(frame_of(&w, b), RectEmu::new(10, 100, 40, 50));
    assert_eq!(frame_of(&w, c), RectEmu::new(10, 200, 60, 50));

    // Undo restores the original x positions.
    assert!(h.undo(&mut w));
    assert_eq!(frame_of(&w, a), RectEmu::new(10, 0, 100, 50));
    assert_eq!(frame_of(&w, b), RectEmu::new(30, 100, 40, 50));
    assert_eq!(frame_of(&w, c), RectEmu::new(70, 200, 60, 50));
}

#[test]
fn align_hcenter_centers_on_group() {
    let mut w = World::new();
    let mut h = History::new();
    // Group spans x: [10 .. 130]; center_x = 70.
    let a = spawn_framed(&mut w, 10, 0, 100, 50);
    let b = spawn_framed(&mut w, 30, 100, 40, 50);
    let c = spawn_framed(&mut w, 70, 200, 60, 50);

    let tx = align(&w, &[a, b, c], Align::HCenter);
    h.commit(&mut w, tx);

    // Each origin.x = center_x - w/2, with center_x = (10 + 130) / 2 = 70.
    assert_eq!(frame_of(&w, a).origin.x, 70 - 100 / 2); // 20
    assert_eq!(frame_of(&w, b).origin.x, 70 - 40 / 2); // 50
    assert_eq!(frame_of(&w, c).origin.x, 70 - 60 / 2); // 40
                                                       // y and size untouched.
    assert_eq!(frame_of(&w, a).origin.y, 0);
    assert_eq!(frame_of(&w, c).size.w, 60);

    assert!(h.undo(&mut w));
    assert_eq!(frame_of(&w, a).origin.x, 10);
    assert_eq!(frame_of(&w, b).origin.x, 30);
    assert_eq!(frame_of(&w, c).origin.x, 70);
}

#[test]
fn align_fewer_than_two_is_empty() {
    let mut w = World::new();
    let a = spawn_framed(&mut w, 10, 0, 100, 50);
    let unframed = w.spawn();
    // Only one framed entity -> empty.
    assert!(align(&w, &[a, unframed], Align::Left).ops.is_empty());
    assert!(align(&w, &[], Align::Left).ops.is_empty());
}

#[test]
fn distribute_horizontal_equalizes_gaps() {
    let mut w = World::new();
    let mut h = History::new();
    // Widths 100 / 40 / 60; first at x=0, last trailing edge at 300.
    // Provide the middle item out of order to confirm sorting by position.
    let a = spawn_framed(&mut w, 0, 0, 100, 50); // [0 .. 100]
    let c = spawn_framed(&mut w, 240, 0, 60, 50); // [240 .. 300]
    let b = spawn_framed(&mut w, 120, 0, 40, 50); // [120 .. 160], will be moved to 150

    let tx = distribute(&w, &[a, c, b], Axis::Horizontal);
    h.commit(&mut w, tx);

    // span = 300 - 0 = 300; total_extent = 200; gap = (300 - 200) / 2 = 50.
    // a fixed at 0 .. 100; b at 100 + 50 = 150 .. 190; c fixed at 240 .. 300.
    let fa = frame_of(&w, a);
    let fb = frame_of(&w, b);
    let fc = frame_of(&w, c);
    let gap_ab = fb.origin.x - fa.right();
    let gap_bc = fc.origin.x - fb.right();
    assert_eq!(gap_ab, gap_bc, "gaps between adjacent items must be equal");
    assert_eq!(gap_ab, 50);
    // First and last stay put.
    assert_eq!(fa.origin.x, 0);
    assert_eq!(fc.origin.x, 240);
    assert_eq!(fb.origin.x, 150);

    // Undo restores b's original position.
    assert!(h.undo(&mut w));
    assert_eq!(frame_of(&w, b).origin.x, 120);
}

#[test]
fn distribute_fewer_than_three_is_empty() {
    let mut w = World::new();
    let a = spawn_framed(&mut w, 0, 0, 100, 50);
    let b = spawn_framed(&mut w, 150, 0, 40, 50);
    assert!(distribute(&w, &[a, b], Axis::Horizontal).ops.is_empty());
}
