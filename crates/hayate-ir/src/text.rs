//! Rich text model (DESIGN 6.6). Logical content only; shaping/layout happens in the app
//! layer via gpui, while line-break policy (kinsoku) lives in a core trait elsewhere.

use crate::color::Color;
use crate::font::FontRef;
use crate::units::Emu;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum HAlign {
    Left,
    Center,
    Right,
    Justify,
}

/// Text content of a text box. Held as a `texts` component on a shape entity.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TextBody {
    pub paragraphs: Vec<Paragraph>,
    /// Shrink text to fit the box; the scale factor is computed in the app layer.
    pub autofit: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Paragraph {
    pub runs: Vec<Run>,
    pub align: HAlign,
    /// Bullet list level (0 = no bullet). One level supported in MVP; deeper reserved.
    pub bullet_level: u8,
    /// Line spacing as a multiple of single (1.0 = single).
    pub line_spacing: f32,
}

/// A contiguous span of text sharing formatting.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Run {
    pub text: String,
    pub font: FontRef,
    /// Font size in EMU (use `units::pt` to construct from points).
    pub size: Emu,
    pub color: Color,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
}

impl Paragraph {
    /// A left-aligned, single-spaced paragraph with the given runs.
    pub fn new(runs: Vec<Run>) -> Self {
        Self {
            runs,
            align: HAlign::Left,
            bullet_level: 0,
            line_spacing: 1.0,
        }
    }
}
