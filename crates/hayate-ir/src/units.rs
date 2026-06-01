//! 単位系。OOXML 互換のため EMU(English Metric Unit) を基本長さ単位に用いる。
//! これにより将来の PPTX 入出力で座標変換が素直になる（§DESIGN 互換性方針）。

/// English Metric Unit。`914400 EMU = 1 inch`、`12700 EMU = 1 pt`。
pub type Emu = i64;

pub const EMU_PER_INCH: Emu = 914_400;
pub const EMU_PER_PT: Emu = 12_700;
pub const EMU_PER_CM: Emu = 360_000;

/// ポイント(整数)を EMU へ。
pub const fn pt(v: i64) -> Emu {
    v * EMU_PER_PT
}

/// ポイント(小数)を EMU へ（四捨五入）。
pub fn pt_f(v: f64) -> Emu {
    (v * EMU_PER_PT as f64).round() as Emu
}

/// インチ(小数)を EMU へ（四捨五入）。
pub fn inch_f(v: f64) -> Emu {
    (v * EMU_PER_INCH as f64).round() as Emu
}

/// ミリ秒。アニメーション等の時間に用いる（§DESIGN 6.15）。
pub type Ms = u32;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conversions() {
        assert_eq!(pt(1), 12_700);
        assert_eq!(pt(72), EMU_PER_INCH);
        assert_eq!(pt_f(0.5), 6_350);
        assert_eq!(inch_f(1.0), EMU_PER_INCH);
    }
}
