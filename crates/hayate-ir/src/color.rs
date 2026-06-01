//! 色とテーマ参照（§DESIGN 6.14）。
//!
//! 色は「リテラル」または「テーマトークン + 変換」で持つ。後者により、テーマの
//! 配色を変えるだけで全スライドの該当色が連動する（リテラル直書きを避ける理由）。

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

/// OOXML の配色スキーム 12 トークン。
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

/// テーマ色への変換（明暗・透過）。すべて省略可（None = 恒等）。
/// 値は 0.0..=1.0。`lum_mod`/`lum_off` は近似実装（厳密な OOXML 一致は PPTX 対応時に詰める）。
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ColorXf {
    pub lum_mod: Option<f32>,
    pub lum_off: Option<f32>,
    pub tint: Option<f32>,
    pub shade: Option<f32>,
    pub alpha: Option<f32>,
}

impl ColorXf {
    /// 変換を Rgba に適用する。
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
            // 白に近づける
            r += (255.0 - r) * t;
            g += (255.0 - g) * t;
            b += (255.0 - b) * t;
        }
        if let Some(s) = self.shade {
            // 黒に近づける
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

/// 図形等が持つ色。`Theme` はマスターのテーマ経由で解決される（§DESIGN 6.8/6.14）。
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
mod tests {
    use super::*;

    #[test]
    fn xf_tint_toward_white() {
        let out = ColorXf {
            tint: Some(0.5),
            ..Default::default()
        }
        .apply(Rgba::rgb(0, 0, 0));
        assert_eq!(out, Rgba::rgb(128, 128, 128));
    }

    #[test]
    fn xf_shade_toward_black() {
        let out = ColorXf {
            shade: Some(0.5),
            ..Default::default()
        }
        .apply(Rgba::rgb(200, 200, 200));
        assert_eq!(out, Rgba::rgb(100, 100, 100));
    }

    #[test]
    fn xf_alpha() {
        let out = ColorXf {
            alpha: Some(0.5),
            ..Default::default()
        }
        .apply(Rgba::rgb(10, 20, 30));
        assert_eq!(out, Rgba::rgba(10, 20, 30, 128));
    }

    #[test]
    fn identity_default() {
        let c = Rgba::rgb(12, 34, 56);
        assert_eq!(ColorXf::default().apply(c), c);
    }
}
