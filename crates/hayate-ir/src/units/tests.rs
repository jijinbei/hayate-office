//! Unit tests for the parent module.

use super::*;

#[test]
fn conversions() {
    assert_eq!(pt(1), 12_700);
    assert_eq!(pt(72), EMU_PER_INCH);
    assert_eq!(pt_f(0.5), 6_350);
    assert_eq!(inch_f(1.0), EMU_PER_INCH);
}
