//! Unit tests for the parent module.

use super::*;

const EPS: f32 = 1e-4;

fn approx(a: f32, b: f32) -> bool {
    (a - b).abs() <= EPS
}

fn approx_pt(a: (f32, f32), b: (f32, f32)) -> bool {
    approx(a.0, b.0) && approx(a.1, b.1)
}

fn rect() -> PxRect {
    PxRect {
        x: 10.0,
        y: 20.0,
        w: 100.0,
        h: 40.0,
    }
}

#[test]
fn rotation_zero_matches_rect_corners_and_midpoints() {
    let r = rect();
    let h = resize_handles(r, 0.0);

    // TL, T, TR, R, BR, B, BL, L
    assert!(approx_pt(h[0], (r.x, r.y)), "TL = {:?}", h[0]);
    assert!(approx_pt(h[1], (r.x + r.w / 2.0, r.y)), "T = {:?}", h[1]);
    assert!(approx_pt(h[2], (r.x + r.w, r.y)), "TR = {:?}", h[2]);
    assert!(
        approx_pt(h[3], (r.x + r.w, r.y + r.h / 2.0)),
        "R = {:?}",
        h[3]
    );
    assert!(approx_pt(h[4], (r.x + r.w, r.y + r.h)), "BR = {:?}", h[4]);
    assert!(
        approx_pt(h[5], (r.x + r.w / 2.0, r.y + r.h)),
        "B = {:?}",
        h[5]
    );
    assert!(approx_pt(h[6], (r.x, r.y + r.h)), "BL = {:?}", h[6]);
    assert!(approx_pt(h[7], (r.x, r.y + r.h / 2.0)), "L = {:?}", h[7]);
}

#[test]
fn rotation_preserves_center() {
    let r = rect();
    let cx = r.x + r.w / 2.0;
    let cy = r.y + r.h / 2.0;
    let h = resize_handles(r, 37.0);
    // The centroid of opposite corners stays at the rect center under rotation.
    let mid_tl_br = ((h[0].0 + h[4].0) / 2.0, (h[0].1 + h[4].1) / 2.0);
    assert!(approx_pt(mid_tl_br, (cx, cy)), "center = {:?}", mid_tl_br);
}

#[test]
fn rotation_90_rotates_corners() {
    // Use a square so the rotated TL lands exactly on the un-rotated TR position.
    let r = PxRect {
        x: 0.0,
        y: 0.0,
        w: 100.0,
        h: 100.0,
    };
    let cx = r.x + r.w / 2.0;
    let cy = r.y + r.h / 2.0;
    let h = resize_handles(r, 90.0);

    // Clockwise 90 deg in Y-down space: TL (0,0) -> (right, top) = TR position.
    let tr_pos = (r.x + r.w, r.y);
    assert!(
        approx_pt(h[0], tr_pos),
        "rotated TL = {:?}, expected near {:?}",
        h[0],
        tr_pos
    );

    // It must no longer sit on the original TL corner.
    assert!(
        !approx_pt(h[0], (r.x, r.y)),
        "TL should move under rotation"
    );

    // Sanity: still centered.
    assert!(approx(cx, 50.0) && approx(cy, 50.0));
}
