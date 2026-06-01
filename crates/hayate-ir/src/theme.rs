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
            fonts: FontScheme {
                major: mk("Arial", "Noto Sans JP"),
                minor: mk("Arial", "Noto Sans JP"),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::color::ColorXf;

    #[test]
    fn resolve_literal() {
        let t = Theme::default();
        assert_eq!(t.resolve_color(&Color::literal(Rgba::rgb(1, 2, 3))), Rgba::rgb(1, 2, 3));
    }

    #[test]
    fn resolve_token_with_transform() {
        let t = Theme::default();
        // accent1 darkened 50% via shade.
        let c = Color::Theme {
            token: ThemeColorToken::Accent1,
            xf: ColorXf {
                shade: Some(0.5),
                ..Default::default()
            },
        };
        let base = t.color_for(ThemeColorToken::Accent1);
        let expect = ColorXf {
            shade: Some(0.5),
            ..Default::default()
        }
        .apply(base);
        assert_eq!(t.resolve_color(&c), expect);
    }

    #[test]
    fn font_picks_script_slot() {
        let t = Theme::default();
        let body = FontRef::Theme(ThemeFontSlot::Minor);
        assert_eq!(t.font_family(&body, Script::Latin), "Arial");
        assert_eq!(t.font_family(&body, Script::Ea), "Noto Sans JP");
        assert_eq!(t.font_family(&FontRef::Family("Mincho".into()), Script::Latin), "Mincho");
    }
}
