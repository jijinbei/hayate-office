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
}
