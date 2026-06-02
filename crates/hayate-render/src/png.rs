//! Dependency-free PNG encoder for RGBA8 buffers.
//!
//! Emits a baseline 8-bit RGBA PNG using only uncompressed (stored) DEFLATE blocks inside a
//! zlib stream, so no compression library is needed. This is meant for headless/offscreen
//! debug captures (pair it with [`crate::rasterize`]); files are larger than a real encoder
//! would produce but are valid PNGs readable by any viewer.

/// Encode an `w` x `h` RGBA8 buffer (`rgba.len() == w*h*4`, row-major, top-left origin)
/// into PNG bytes. Panics only if the buffer length does not match the dimensions.
pub fn encode_png(rgba: &[u8], w: u32, h: u32) -> Vec<u8> {
    assert_eq!(
        rgba.len(),
        (w as usize) * (h as usize) * 4,
        "rgba buffer length must be w*h*4"
    );

    let mut out = Vec::new();
    // PNG signature.
    out.extend_from_slice(&[137, 80, 78, 71, 13, 10, 26, 10]);

    // IHDR: width, height, bit depth 8, color type 6 (RGBA), no compression/filter/interlace.
    let mut ihdr = Vec::with_capacity(13);
    ihdr.extend_from_slice(&w.to_be_bytes());
    ihdr.extend_from_slice(&h.to_be_bytes());
    ihdr.extend_from_slice(&[8, 6, 0, 0, 0]);
    write_chunk(&mut out, b"IHDR", &ihdr);

    // Build the raw (filter-prefixed) scanlines, then wrap in a zlib stored-block stream.
    let raw = filtered_scanlines(rgba, w, h);
    let zlib = zlib_stored(&raw);
    write_chunk(&mut out, b"IDAT", &zlib);

    write_chunk(&mut out, b"IEND", &[]);
    out
}

/// Prefix each scanline with filter byte 0 (None) as required by the PNG spec.
fn filtered_scanlines(rgba: &[u8], w: u32, h: u32) -> Vec<u8> {
    let stride = (w as usize) * 4;
    let mut raw = Vec::with_capacity((stride + 1) * h as usize);
    for y in 0..h as usize {
        raw.push(0); // filter type: None
        raw.extend_from_slice(&rgba[y * stride..(y + 1) * stride]);
    }
    raw
}

/// Wrap `data` in a minimal zlib stream using only stored (uncompressed) DEFLATE blocks.
fn zlib_stored(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len() + data.len() / 65535 * 5 + 16);
    // zlib header: CMF=0x78 (deflate, 32K window), FLG=0x01 (fastest, check bits make %31==0).
    out.push(0x78);
    out.push(0x01);

    // Stored blocks, each carrying up to 65535 bytes.
    let mut pos = 0usize;
    if data.is_empty() {
        // A single final empty stored block.
        out.push(1);
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&(!0u16).to_le_bytes());
    } else {
        while pos < data.len() {
            let chunk = (data.len() - pos).min(0xFFFF);
            let is_final = pos + chunk >= data.len();
            out.push(if is_final { 1 } else { 0 }); // BFINAL bit, BTYPE=00 (stored)
            let len = chunk as u16;
            out.extend_from_slice(&len.to_le_bytes());
            out.extend_from_slice(&(!len).to_le_bytes());
            out.extend_from_slice(&data[pos..pos + chunk]);
            pos += chunk;
        }
    }

    // Adler-32 of the uncompressed data, big-endian.
    out.extend_from_slice(&adler32(data).to_be_bytes());
    out
}

/// Write a PNG chunk: length (BE), type, data, CRC-32 over (type + data).
fn write_chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    out.extend_from_slice(kind);
    out.extend_from_slice(data);
    let mut crc = Crc32::new();
    crc.update(kind);
    crc.update(data);
    out.extend_from_slice(&crc.finalize().to_be_bytes());
}

/// Adler-32 checksum (RFC 1950).
fn adler32(data: &[u8]) -> u32 {
    const MOD: u32 = 65521;
    let mut a: u32 = 1;
    let mut b: u32 = 0;
    for &byte in data {
        a = (a + byte as u32) % MOD;
        b = (b + a) % MOD;
    }
    (b << 16) | a
}

/// Streaming CRC-32 (IEEE, as used by PNG) with a lazily built lookup table.
struct Crc32 {
    crc: u32,
}

impl Crc32 {
    fn new() -> Self {
        Crc32 { crc: 0xFFFF_FFFF }
    }

    fn update(&mut self, bytes: &[u8]) {
        let table = crc_table();
        for &b in bytes {
            let idx = ((self.crc ^ b as u32) & 0xFF) as usize;
            self.crc = table[idx] ^ (self.crc >> 8);
        }
    }

    fn finalize(self) -> u32 {
        self.crc ^ 0xFFFF_FFFF
    }
}

/// The CRC-32 table, computed once on first use.
fn crc_table() -> &'static [u32; 256] {
    use std::sync::OnceLock;
    static TABLE: OnceLock<[u32; 256]> = OnceLock::new();
    TABLE.get_or_init(|| {
        let mut table = [0u32; 256];
        for (n, slot) in table.iter_mut().enumerate() {
            let mut c = n as u32;
            for _ in 0..8 {
                c = if c & 1 != 0 {
                    0xEDB8_8320 ^ (c >> 1)
                } else {
                    c >> 1
                };
            }
            *slot = c;
        }
        table
    })
}

#[cfg(test)]
mod tests {
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
}
