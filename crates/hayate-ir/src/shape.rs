//! Vector shape geometry (DESIGN 6.7). The presence of a `Geometry` component marks an
//! entity as a vector shape; a text box instead carries a `TextBody`, a picture a
//! `PictureRef`, etc. Freeform `Path` geometry is reserved for later.

use crate::units::Emu;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Geometry {
    Rect,
    RoundRect {
        /// Corner radius in EMU.
        radius: Emu,
    },
    Ellipse,
    /// A straight line drawn along the diagonal of the shape's `frame`, from the frame's
    /// top-left (the START point) to its bottom-right (the END point). Each end carries an
    /// independent [`ArrowHead`]. A line has no fill; only a stroke.
    Line {
        start: ArrowHead,
        end: ArrowHead,
    },
}

/// The decoration at one end of a line/connector.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum ArrowHead {
    /// A plain end (no decoration).
    #[default]
    None,
    /// A V-shaped arrowhead pointing outward along the line.
    Arrow,
}
