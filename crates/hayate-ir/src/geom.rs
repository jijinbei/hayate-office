//! Slide-coordinate geometry types (in EMU).

use crate::units::Emu;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PointEmu {
    pub x: Emu,
    pub y: Emu,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SizeEmu {
    pub w: Emu,
    pub h: Emu,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RectEmu {
    pub origin: PointEmu,
    pub size: SizeEmu,
}

impl PointEmu {
    pub const fn new(x: Emu, y: Emu) -> Self {
        Self { x, y }
    }
}

impl SizeEmu {
    pub const fn new(w: Emu, h: Emu) -> Self {
        Self { w, h }
    }
}

impl RectEmu {
    pub const fn new(x: Emu, y: Emu, w: Emu, h: Emu) -> Self {
        Self {
            origin: PointEmu::new(x, y),
            size: SizeEmu::new(w, h),
        }
    }

    pub const fn right(&self) -> Emu {
        self.origin.x + self.size.w
    }

    pub const fn bottom(&self) -> Emu {
        self.origin.y + self.size.h
    }

    pub const fn center(&self) -> PointEmu {
        PointEmu::new(
            self.origin.x + self.size.w / 2,
            self.origin.y + self.size.h / 2,
        )
    }

    /// Axis-aligned containment (pre-rotation). Hit-testing of rotated shapes is done
    /// at the Scene layer (DESIGN 6.7).
    pub fn contains(&self, p: PointEmu) -> bool {
        p.x >= self.origin.x && p.x < self.right() && p.y >= self.origin.y && p.y < self.bottom()
    }
}

#[cfg(test)]
mod tests {
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
}
