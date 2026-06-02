//! Unit system. We use EMU (English Metric Unit) as the base length unit for OOXML
//! compatibility, which keeps future PPTX import/export coordinate conversion clean.

/// English Metric Unit. `914400 EMU = 1 inch`, `12700 EMU = 1 pt`.
pub type Emu = i64;

pub const EMU_PER_INCH: Emu = 914_400;
pub const EMU_PER_PT: Emu = 12_700;
pub const EMU_PER_CM: Emu = 360_000;

/// Points (integer) to EMU.
pub const fn pt(v: i64) -> Emu {
    v * EMU_PER_PT
}

/// Points (fractional) to EMU (rounded).
pub fn pt_f(v: f64) -> Emu {
    (v * EMU_PER_PT as f64).round() as Emu
}

/// Inches (fractional) to EMU (rounded).
pub fn inch_f(v: f64) -> Emu {
    (v * EMU_PER_INCH as f64).round() as Emu
}

/// Milliseconds, used for animation timing (DESIGN 6.15).
pub type Ms = u32;

#[cfg(test)]
mod tests;
