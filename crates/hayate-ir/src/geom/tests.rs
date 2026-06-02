//! Unit tests for the parent module.

use super::*;

#[test]
fn rect_basics() {
    let r = RectEmu::new(10, 20, 100, 40);
    assert_eq!(r.right(), 110);
    assert_eq!(r.bottom(), 60);
    assert_eq!(r.center(), PointEmu::new(60, 40));
}

#[test]
fn contains() {
    let r = RectEmu::new(0, 0, 100, 100);
    assert!(r.contains(PointEmu::new(0, 0)));
    assert!(r.contains(PointEmu::new(99, 99)));
    assert!(!r.contains(PointEmu::new(100, 100)));
    assert!(!r.contains(PointEmu::new(-1, 50)));
}
