//! Unit tests for the parent module.

use super::*;

#[test]
fn known_glyphs_present() {
    assert!(glyph('A').is_some());
    assert!(glyph('a').is_some()); // mapped to uppercase
    assert!(glyph('0').is_some());
    assert_eq!(glyph(' '), Some([0; 7]));
}

#[test]
fn non_ascii_returns_none() {
    assert!(glyph('あ').is_none());
    assert!(glyph('字').is_none());
}
