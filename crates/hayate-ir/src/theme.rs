//! Theme: color scheme + font scheme, and resolution of theme references to concrete
//! values (DESIGN 6.14). Resolution is the "system" the Scene builder runs so the renderer
//! only ever sees literal colors and concrete font families.

use crate::color::{Color, Rgba, ThemeColorToken};
use crate::font::{FontRef, Script, ScriptFonts, ThemeFontSlot};
use serde::{Deserialize, Serialize};

/// The 12 OOXML theme colors.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThemeColors {
    pub dk1: Rgba,
    pub lt1: Rgba,
    pub dk2: Rgba,
    pub lt2: Rgba,
    pub accent: [Rgba; 6],
    pub hlink: Rgba,
    pub fol_hlink: Rgba,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FontScheme {
    pub major: ScriptFonts,
    pub minor: ScriptFonts,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Theme {
    pub colors: ThemeColors,
    pub fonts: FontScheme,
}

impl Theme {
    /// The concrete color a theme token maps to (before any transform).
    #[inline]
    pub fn color_for(&self, token: ThemeColorToken) -> Rgba {
        use ThemeColorToken::*;
        match token {
            Dk1 => self.colors.dk1,
            Lt1 => self.colors.lt1,
            Dk2 => self.colors.dk2,
            Lt2 => self.colors.lt2,
            Accent1 => self.colors.accent[0],
            Accent2 => self.colors.accent[1],
            Accent3 => self.colors.accent[2],
            Accent4 => self.colors.accent[3],
            Accent5 => self.colors.accent[4],
            Accent6 => self.colors.accent[5],
            Hlink => self.colors.hlink,
            FolHlink => self.colors.fol_hlink,
        }
    }

    /// Resolve a `Color` to a concrete `Rgba` (applying the transform for theme refs).
    #[inline]
    pub fn resolve_color(&self, color: &Color) -> Rgba {
        match color {
            Color::Literal(rgba) => *rgba,
            Color::Theme { token, xf } => xf.apply(self.color_for(*token)),
        }
    }

    /// Resolve a `FontRef` to a concrete font family for the given script.
    pub fn font_family(&self, font: &FontRef, script: Script) -> String {
        let fonts = match font {
            FontRef::Family(name) => return name.clone(),
            FontRef::Theme(ThemeFontSlot::Major) => &self.fonts.major,
            FontRef::Theme(ThemeFontSlot::Minor) => &self.fonts.minor,
        };
        match script {
            Script::Latin => fonts.latin.clone(),
            Script::Ea => fonts.ea.clone(),
            Script::Cs => fonts.cs.clone(),
        }
    }
}

/// The default sans-serif family for new presentations, picked per-platform so it names a font
/// that is actually installed on the machine creating the deck. Font selection is by exact family
/// name (gpui/font-kit's `select_family_by_name`, cosmic-text's `Family::Name`); when the named
/// family is missing, gpui falls back to a Latin-only UI font, which leaves CJK runs blank. Each
/// value below is a system font that ships with the platform and covers both Latin and CJK with
/// real weights: macOS "Hiragino Sans", Windows "Yu Gothic UI", and on Linux (incl. the Nix dev
/// shell) "Noto Sans CJK JP".
pub fn default_sans_family() -> &'static str {
    #[cfg(target_os = "macos")]
    {
        "Hiragino Sans"
    }
    #[cfg(target_os = "windows")]
    {
        "Yu Gothic UI"
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        "Noto Sans CJK JP"
    }
}

/// Names of the built-in color presets, in the same order as [`theme_color_presets`]. This is the
/// single source of truth for the order, shared by both accessors (the index selects a preset).
const PRESET_NAMES: [&str; 4] = ["Office", "Warm", "Cool", "Mono"];

/// The built-in color preset names, in the same order as [`theme_color_presets`] (the index is
/// used to select a preset). A lightweight, allocation-free alternative when only the names are
/// needed.
pub fn theme_color_preset_names() -> &'static [&'static str] {
    &PRESET_NAMES
}

/// A few built-in color schemes (name + colors) the UI can apply to a master's theme.
pub fn theme_color_presets() -> Vec<(&'static str, ThemeColors)> {
    let base = |accent: [Rgba; 6], dk2: Rgba| ThemeColors {
        dk1: Rgba::rgb(0, 0, 0),
        lt1: Rgba::rgb(255, 255, 255),
        dk2,
        lt2: Rgba::rgb(238, 238, 238),
        accent,
        hlink: Rgba::rgb(0x00, 0x00, 0xEE),
        fol_hlink: Rgba::rgb(0x80, 0x00, 0x80),
    };
    vec![
        (PRESET_NAMES[0], Theme::default().colors),
        (
            PRESET_NAMES[1],
            base(
                [
                    Rgba::rgb(0xC0, 0x39, 0x2B),
                    Rgba::rgb(0xE6, 0x7E, 0x22),
                    Rgba::rgb(0xF1, 0xC4, 0x0F),
                    Rgba::rgb(0xD3, 0x5F, 0x5F),
                    Rgba::rgb(0xA9, 0x3A, 0x26),
                    Rgba::rgb(0x8E, 0x44, 0x22),
                ],
                Rgba::rgb(0x5A, 0x2A, 0x1E),
            ),
        ),
        (
            PRESET_NAMES[2],
            base(
                [
                    Rgba::rgb(0x2E, 0x86, 0xC1),
                    Rgba::rgb(0x16, 0xA0, 0x85),
                    Rgba::rgb(0x6C, 0x3A, 0xB7),
                    Rgba::rgb(0x21, 0x9E, 0xBC),
                    Rgba::rgb(0x27, 0xAE, 0x60),
                    Rgba::rgb(0x34, 0x95, 0xDB),
                ],
                Rgba::rgb(0x1B, 0x2A, 0x4A),
            ),
        ),
        (
            PRESET_NAMES[3],
            base(
                [
                    Rgba::rgb(0x33, 0x33, 0x33),
                    Rgba::rgb(0x55, 0x55, 0x55),
                    Rgba::rgb(0x77, 0x77, 0x77),
                    Rgba::rgb(0x99, 0x99, 0x99),
                    Rgba::rgb(0xBB, 0xBB, 0xBB),
                    Rgba::rgb(0x22, 0x22, 0x22),
                ],
                Rgba::rgb(0x33, 0x33, 0x33),
            ),
        ),
    ]
}

impl Default for Theme {
    /// A neutral light theme, useful for new presentations and tests.
    fn default() -> Self {
        let mk = |latin: &str, ea: &str| ScriptFonts {
            latin: latin.to_string(),
            ea: ea.to_string(),
            cs: latin.to_string(),
        };
        Theme {
            colors: ThemeColors {
                dk1: Rgba::rgb(0, 0, 0),
                lt1: Rgba::rgb(255, 255, 255),
                dk2: Rgba::rgb(68, 68, 68),
                lt2: Rgba::rgb(238, 238, 238),
                accent: [
                    Rgba::rgb(0x4F, 0x81, 0xBD),
                    Rgba::rgb(0xC0, 0x50, 0x4D),
                    Rgba::rgb(0x9B, 0xBB, 0x59),
                    Rgba::rgb(0x80, 0x64, 0xA2),
                    Rgba::rgb(0x4B, 0xAC, 0xC6),
                    Rgba::rgb(0xF7, 0x96, 0x46),
                ],
                hlink: Rgba::rgb(0x00, 0x00, 0xEE),
                fol_hlink: Rgba::rgb(0x80, 0x00, 0x80),
            },
            // One family for every script (see [`default_sans_family`] for the per-platform pick).
            // The chosen family covers Latin + CJK and ships a real Bold face, so bold resolves on
            // screen (gpui/font-kit does not synthesize bold) and matches the PDF. Using a single
            // family also keeps list bullets a consistent size: the bullet glyph no longer depends
            // on whether a line happens to contain CJK (which used to switch the resolved family,
            // and thus the bullet's font/size, per line).
            fonts: FontScheme {
                major: mk(default_sans_family(), default_sans_family()),
                minor: mk(default_sans_family(), default_sans_family()),
            },
        }
    }
}

#[cfg(test)]
mod tests;
