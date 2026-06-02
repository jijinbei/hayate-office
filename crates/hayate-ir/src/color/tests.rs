//! Unit tests for the parent module.

use super::*;

#[test]
fn xf_tint_toward_white() {
    let out = ColorXf {
        tint: Some(0.5),
        ..Default::default()
    }
    .apply(Rgba::rgb(0, 0, 0));
    assert_eq!(out, Rgba::rgb(128, 128, 128));
}

#[test]
fn xf_shade_toward_black() {
    let out = ColorXf {
        shade: Some(0.5),
        ..Default::default()
    }
    .apply(Rgba::rgb(200, 200, 200));
    assert_eq!(out, Rgba::rgb(100, 100, 100));
}

#[test]
fn xf_alpha() {
    let out = ColorXf {
        alpha: Some(0.5),
        ..Default::default()
    }
    .apply(Rgba::rgb(10, 20, 30));
    assert_eq!(out, Rgba::rgba(10, 20, 30, 128));
}

#[test]
fn identity_default() {
    let c = Rgba::rgb(12, 34, 56);
    assert_eq!(ColorXf::default().apply(c), c);
}
