//! Unit tests for the parent module.

use super::*;

#[test]
fn encodes_valid_png_header() {
    let w = 2;
    let h = 2;
    let rgba = vec![255u8; (w * h * 4) as usize];
    let png = encode_png(&rgba, w, h);
    // PNG signature.
    assert_eq!(&png[0..8], &[137, 80, 78, 71, 13, 10, 26, 10]);
    // First chunk type after the 4-byte length is IHDR.
    assert_eq!(&png[12..16], b"IHDR");
    // Ends with an IEND chunk.
    let n = png.len();
    assert_eq!(&png[n - 8..n - 4], b"IEND");
}

#[test]
fn adler32_known_value() {
    // Adler-32 of "Wikipedia" is 0x11E60398.
    assert_eq!(adler32(b"Wikipedia"), 0x11E6_0398);
}

#[test]
fn crc32_known_value() {
    // CRC-32/IEEE of "123456789" is 0xCBF43926.
    let mut c = Crc32::new();
    c.update(b"123456789");
    assert_eq!(c.finalize(), 0xCBF4_3926);
}

#[test]
fn large_buffer_spans_multiple_stored_blocks() {
    // > 65535 bytes of raw scanline data forces multiple stored blocks.
    let w = 200;
    let h = 200;
    let rgba = vec![128u8; (w * h * 4) as usize];
    let png = encode_png(&rgba, w, h);
    // Sanity: a valid signature and a plausible size.
    assert_eq!(&png[0..8], &[137, 80, 78, 71, 13, 10, 26, 10]);
    assert!(png.len() > (w * h * 4) as usize);
}
