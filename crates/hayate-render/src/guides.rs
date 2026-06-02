//! Smart alignment guides for the editor. When the user drags a shape, we compare its edges
//! and centers against the surrounding shapes and surface guide lines whenever they line up
//! within a pixel threshold. gpui-free so it stays unit-testable.

use crate::scene::PxRect;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GuideKind {
    /// A vertical line at a given x position.
    Vertical,
    /// A horizontal line at a given y position.
    Horizontal,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Guide {
    pub kind: GuideKind,
    /// For `Vertical` this is an x position; for `Horizontal` it is a y position.
    pub pos: f32,
}

/// The three x candidates of a rect: left, horizontal center, right.
fn x_candidates(r: PxRect) -> [f32; 3] {
    [r.x, r.x + r.w * 0.5, r.x + r.w]
}

/// The three y candidates of a rect: top, vertical center, bottom.
fn y_candidates(r: PxRect) -> [f32; 3] {
    [r.y, r.y + r.h * 0.5, r.y + r.h]
}

/// Round a position so near-identical lines deduplicate (0.1px buckets).
fn round_pos(pos: f32) -> i32 {
    (pos * 10.0).round() as i32
}

/// Push a guide if an equivalent one (same kind, same rounded pos) is not present yet.
fn push_unique(out: &mut Vec<Guide>, kind: GuideKind, pos: f32) {
    let key = round_pos(pos);
    let exists = out
        .iter()
        .any(|g| g.kind == kind && round_pos(g.pos) == key);
    if !exists {
        out.push(Guide { kind, pos });
    }
}

/// Compute alignment guides for `moving` against the `others` rects.
///
/// For each of `moving`'s x candidates (left, h-center, right) we look for an x candidate of
/// another rect within `threshold`; when found we emit a `Vertical` guide at the other's line
/// position. The same logic applies to y candidates and `Horizontal` guides. Results are
/// deduplicated by (kind, pos rounded).
pub fn alignment_guides(moving: PxRect, others: &[PxRect], threshold: f32) -> Vec<Guide> {
    let mut out: Vec<Guide> = Vec::new();
    let mx = x_candidates(moving);
    let my = y_candidates(moving);

    for &other in others {
        let ox = x_candidates(other);
        for &m in &mx {
            for &o in &ox {
                if (m - o).abs() <= threshold {
                    push_unique(&mut out, GuideKind::Vertical, o);
                }
            }
        }

        let oy = y_candidates(other);
        for &m in &my {
            for &o in &oy {
                if (m - o).abs() <= threshold {
                    push_unique(&mut out, GuideKind::Horizontal, o);
                }
            }
        }
    }

    out
}

#[cfg(test)]
mod tests {
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
        assert_eq!(vert_at_100, 1, "expected exactly one guide at x=100, got {guides:?}");
    }
}
