//! Colors and theme references (DESIGN 6.14).
//!
//! A color is held either as a literal or as a theme token plus a transform. The latter
//! lets changing the theme palette propagate to every slide that references it, which is
//! why we avoid hard-coding literal colors.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rgba {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Rgba {
    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b, a: 255 }
    }
    pub const fn rgba(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self { r, g, b, a }
    }
    pub const BLACK: Rgba = Rgba::rgb(0, 0, 0);
    pub const WHITE: Rgba = Rgba::rgb(255, 255, 255);
    pub const TRANSPARENT: Rgba = Rgba::rgba(0, 0, 0, 0);
}

/// The 12 tokens of an OOXML color scheme.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ThemeColorToken {
    Dk1,
    Lt1,
    Dk2,
    Lt2,
    Accent1,
    Accent2,
    Accent3,
    Accent4,
    Accent5,
    Accent6,
    Hlink,
    FolHlink,
}

/// Transform applied to a theme color (luminance/tint/shade/alpha). Each field is optional
/// (None = identity). Values are 0.0..=1.0. `lum_mod`/`lum_off` are approximate for now
/// (exact OOXML matching is deferred to PPTX support).
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ColorXf {
    pub lum_mod: Option<f32>,
    pub lum_off: Option<f32>,
    pub tint: Option<f32>,
    pub shade: Option<f32>,
    pub alpha: Option<f32>,
}

impl ColorXf {
    /// Apply the transform to an `Rgba`.
    pub fn apply(&self, c: Rgba) -> Rgba {
        let mut r = c.r as f32;
        let mut g = c.g as f32;
        let mut b = c.b as f32;
        let mut a = c.a as f32;

        if let Some(m) = self.lum_mod {
            r *= m;
            g *= m;
            b *= m;
        }
        if let Some(o) = self.lum_off {
            r += o * 255.0;
            g += o * 255.0;
            b += o * 255.0;
        }
        if let Some(t) = self.tint {
            // Move toward white.
            r += (255.0 - r) * t;
            g += (255.0 - g) * t;
            b += (255.0 - b) * t;
        }
        if let Some(s) = self.shade {
            // Move toward black.
            r *= 1.0 - s;
            g *= 1.0 - s;
            b *= 1.0 - s;
        }
        if let Some(al) = self.alpha {
            a *= al;
        }

        let clamp = |v: f32| v.round().clamp(0.0, 255.0) as u8;
        Rgba::rgba(clamp(r), clamp(g), clamp(b), clamp(a))
    }
}

/// A color held by shapes etc. `Theme` is resolved via the master's theme (DESIGN 6.8/6.14).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum Color {
    Literal(Rgba),
    Theme { token: ThemeColorToken, xf: ColorXf },
}

impl Color {
    pub const fn literal(c: Rgba) -> Self {
        Color::Literal(c)
    }
    pub const fn theme(token: ThemeColorToken) -> Self {
        Color::Theme {
            token,
            xf: ColorXf {
                lum_mod: None,
                lum_off: None,
                tint: None,
                shade: None,
                alpha: None,
            },
        }
    }
}

#[cfg(test)]
mod tests;
