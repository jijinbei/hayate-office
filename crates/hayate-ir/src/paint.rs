//! Fill and stroke for shapes (DESIGN 6.7/6.14). Colors may be literal or theme refs.

use crate::color::Color;
use crate::units::Emu;
use serde::{Deserialize, Serialize};

/// How a shape's interior is painted. Gradient/image fills are reserved for later.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum Fill {
    Solid(Color),
}

/// Outline of a shape.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Stroke {
    pub color: Color,
    /// Line width in EMU.
    pub width: Emu,
    /// Dash pattern in EMU (on/off lengths). `None` = solid.
    pub dash: Option<Vec<Emu>>,
}

impl Stroke {
    pub fn solid(color: Color, width: Emu) -> Self {
        Self {
            color,
            width,
            dash: None,
        }
    }
}
