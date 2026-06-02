//! Editor grid / ruler overlay helper. Computes the line positions for a grid or ruler
//! drawn over the editing surface. gpui-free, so it is unit-testable in isolation.

use crate::scene::PxSize;

/// Line positions (in pixels) for a grid overlay.
#[derive(Clone, Debug, PartialEq)]
pub struct GridLines {
    /// X positions of vertical lines.
    pub vertical: Vec<f32>,
    /// Y positions of horizontal lines.
    pub horizontal: Vec<f32>,
}

/// Upper bound on the number of lines generated along a single axis. Guards against
/// pathological inputs (e.g. a sub-pixel `spacing` on a huge surface) that could otherwise
/// allocate an absurd amount of memory.
const MAX_LINES: usize = 10_000;

/// Compute vertical (x) and horizontal (y) line positions for a grid spaced `spacing`
/// pixels apart, starting at 0 and including the far edge when it lands on a multiple of
/// `spacing`. Returns empty lists if `spacing <= 0`. The number of lines per axis is capped
/// at [`MAX_LINES`] to stay bounded.
pub fn grid_lines(size: PxSize, spacing: f32) -> GridLines {
    if spacing <= 0.0 {
        return GridLines {
            vertical: Vec::new(),
            horizontal: Vec::new(),
        };
    }

    GridLines {
        vertical: axis_positions(size.w, spacing),
        horizontal: axis_positions(size.h, spacing),
    }
}

/// Generate positions 0, spacing, 2*spacing, ... up to and including `extent`, capped at
/// [`MAX_LINES`] entries. Assumes `spacing > 0`.
fn axis_positions(extent: f32, spacing: f32) -> Vec<f32> {
    let mut positions = Vec::new();
    let mut i: usize = 0;
    loop {
        let pos = spacing * i as f32;
        if pos > extent || positions.len() >= MAX_LINES {
            break;
        }
        positions.push(pos);
        i += 1;
    }
    positions
}

#[cfg(test)]
mod tests {
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
}
