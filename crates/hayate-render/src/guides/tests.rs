//! Unit tests for the parent module.

use super::*;

fn rect(x: f32, y: f32, w: f32, h: f32) -> PxRect {
    PxRect { x, y, w, h }
}

#[test]
fn left_edge_within_threshold_emits_vertical_guide() {
    // Moving's left edge at x=101 is 1px from other's left edge at x=100.
    let moving = rect(101.0, 50.0, 20.0, 20.0);
    let other = rect(100.0, 300.0, 40.0, 40.0);
    let guides = alignment_guides(moving, &[other], 4.0);
    assert!(
        guides
            .iter()
            .any(|g| g.kind == GuideKind::Vertical && (g.pos - 100.0).abs() < 0.001),
        "expected a vertical guide at x=100, got {guides:?}"
    );
}

#[test]
fn far_apart_rect_emits_no_guides() {
    let moving = rect(0.0, 0.0, 10.0, 10.0);
    let other = rect(500.0, 500.0, 10.0, 10.0);
    let guides = alignment_guides(moving, &[other], 4.0);
    assert!(guides.is_empty(), "expected no guides, got {guides:?}");
}

#[test]
fn vertical_center_alignment_emits_horizontal_guide() {
    // Moving's v-center y = 100; other's v-center y = 100 as well.
    let moving = rect(0.0, 90.0, 20.0, 20.0); // v-center at 100
    let other = rect(300.0, 50.0, 40.0, 100.0); // v-center at 100
    let guides = alignment_guides(moving, &[other], 4.0);
    assert!(
        guides
            .iter()
            .any(|g| g.kind == GuideKind::Horizontal && (g.pos - 100.0).abs() < 0.001),
        "expected a horizontal guide at y=100, got {guides:?}"
    );
}

#[test]
fn dedup_when_two_others_share_a_line() {
    // Two other rects both have a left edge at x=100; the moving rect lines up with both.
    let moving = rect(100.0, 50.0, 20.0, 20.0);
    let a = rect(100.0, 200.0, 40.0, 40.0);
    let b = rect(100.0, 400.0, 60.0, 60.0);
    let guides = alignment_guides(moving, &[a, b], 4.0);
    let vert_at_100 = guides
        .iter()
        .filter(|g| g.kind == GuideKind::Vertical && (g.pos - 100.0).abs() < 0.001)
        .count();
    assert_eq!(
        vert_at_100, 1,
        "expected exactly one guide at x=100, got {guides:?}"
    );
}
