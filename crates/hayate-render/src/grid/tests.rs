//! Unit tests for the parent module.

use super::*;

#[test]
fn basic_grid() {
    let lines = grid_lines(PxSize { w: 100.0, h: 50.0 }, 25.0);
    assert_eq!(lines.vertical, vec![0.0, 25.0, 50.0, 75.0, 100.0]);
    assert_eq!(lines.horizontal, vec![0.0, 25.0, 50.0]);
}

#[test]
fn zero_spacing_is_empty() {
    let lines = grid_lines(PxSize { w: 100.0, h: 50.0 }, 0.0);
    assert!(lines.vertical.is_empty());
    assert!(lines.horizontal.is_empty());
}

#[test]
fn negative_spacing_is_empty() {
    let lines = grid_lines(PxSize { w: 100.0, h: 50.0 }, -5.0);
    assert!(lines.vertical.is_empty());
    assert!(lines.horizontal.is_empty());
}

#[test]
fn tiny_spacing_on_large_size_is_capped() {
    // Without the cap this would try to allocate billions of entries.
    let lines = grid_lines(
        PxSize {
            w: 1_000_000.0,
            h: 1_000_000.0,
        },
        0.001,
    );
    assert_eq!(lines.vertical.len(), MAX_LINES);
    assert_eq!(lines.horizontal.len(), MAX_LINES);
}
