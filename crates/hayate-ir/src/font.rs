//! Font references (DESIGN 6.6/6.14). A run's font is either a literal family or a theme
//! slot resolved per script via the master's theme.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ThemeFontSlot {
    /// Heading font.
    Major,
    /// Body font.
    Minor,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum FontRef {
    Family(String),
    Theme(ThemeFontSlot),
}

/// Per-script font families for one theme slot: latin / East-Asian (e.g. Japanese) /
/// complex-script. Resolution picks a slot from the run's script (DESIGN 6.6).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScriptFonts {
    pub latin: String,
    pub ea: String,
    pub cs: String,
}
