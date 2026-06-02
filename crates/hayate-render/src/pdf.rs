//! Dependency-free PDF export: one page per slide, each carrying a rasterized image of the
//! slide. Vector-faithful export is future work; embedding a raster keeps this self-contained
//! (it reuses [`crate::rasterize`]) and prints at the slide's true physical size.
//!
//! The only binary payload is the per-image FlateDecode stream, built from the same stored
//! (uncompressed) DEFLATE + Adler-32 zlib framing as [`crate::png`]. Everything else is ASCII,
//! and the cross-reference table records exact byte offsets.

use hayate_ir::presentation::Presentation;

use crate::{build_slide_scene, rasterize, PxSize};

/// 1 PDF point = 1/72 inch = 12700 EMU.
const EMU_PER_POINT: f32 = 12_700.0;

/// Render every slide of `p` to a raster and assemble a multi-page PDF (one page per slide).
/// `scale` multiplies the raster resolution (e.g. 2.0 ≈ 144 DPI); each page's size stays the
/// slide size in points, so the document prints at the true physical size.
pub fn export_pdf(p: &Presentation, scale: f32) -> Vec<u8> {
    let scale = if scale.is_finite() && scale > 0.0 {
        scale
    } else {
        1.0
    };
    let slides = p.slides();
    let n = slides.len();
    let pt_w = p.slide_size.w as f32 / EMU_PER_POINT;
    let pt_h = p.slide_size.h as f32 / EMU_PER_POINT;

    // Object numbering: 1 = Catalog, 2 = Pages, then per slide k (0-based) a Page (3 + 3k),
    // a content stream (4 + 3k), and an image XObject (5 + 3k).
    let total_objs = 2 + n * 3;
    let mut out: Vec<u8> = Vec::new();
    let mut offsets = vec![0usize; total_objs + 1]; // 1-based; index 0 is the free entry

    // Header. The binary comment marks the file as containing binary data.
    out.extend_from_slice(b"%PDF-1.7\n");
    out.extend_from_slice(&[b'%', 0xE2, 0xE3, 0xCF, 0xD3, b'\n']);

    // Catalog.
    offsets[1] = out.len();
    out.extend_from_slice(b"1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n");

    // Pages tree.
    offsets[2] = out.len();
    let mut kids = String::new();
    for k in 0..n {
        kids.push_str(&format!("{} 0 R ", 3 + k * 3));
    }
    out.extend_from_slice(
        format!("2 0 obj\n<< /Type /Pages /Kids [ {kids}] /Count {n} >>\nendobj\n").as_bytes(),
    );

    let pw = fmt_num(pt_w);
    let ph = fmt_num(pt_h);
    for (k, &slide) in slides.iter().enumerate() {
        let page_id = 3 + k * 3;
        let content_id = 4 + k * 3;
        let image_id = 5 + k * 3;

        let px_w = ((pt_w * scale).round() as i64).max(1) as u32;
        let px_h = ((pt_h * scale).round() as i64).max(1) as u32;
        let scene = build_slide_scene(
            p,
            slide,
            PxSize {
                w: px_w as f32,
                h: px_h as f32,
            },
        );
        let rgba = rasterize(&scene, px_w, px_h);
        // Drop the alpha channel: slides are opaque, so the raw RGB is what we embed.
        let mut rgb = Vec::with_capacity(rgba.len() / 4 * 3);
        for px in rgba.chunks_exact(4) {
            rgb.extend_from_slice(&px[..3]);
        }
        let zlib = zlib_store(&rgb);

        // Page.
        offsets[page_id] = out.len();
        out.extend_from_slice(
            format!(
                "{page_id} 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {pw} {ph}] \
                 /Resources << /XObject << /Im0 {image_id} 0 R >> >> /Contents {content_id} 0 R >>\nendobj\n"
            )
            .as_bytes(),
        );

        // Content stream: map the unit image space onto the whole page.
        let content = format!("q\n{pw} 0 0 {ph} 0 0 cm\n/Im0 Do\nQ\n");
        offsets[content_id] = out.len();
        out.extend_from_slice(
            format!(
                "{content_id} 0 obj\n<< /Length {} >>\nstream\n{content}endstream\nendobj\n",
                content.len()
            )
            .as_bytes(),
        );

        // Image XObject.
        offsets[image_id] = out.len();
        out.extend_from_slice(
            format!(
                "{image_id} 0 obj\n<< /Type /XObject /Subtype /Image /Width {px_w} /Height {px_h} \
                 /ColorSpace /DeviceRGB /BitsPerComponent 8 /Filter /FlateDecode /Length {} >>\nstream\n",
                zlib.len()
            )
            .as_bytes(),
        );
        out.extend_from_slice(&zlib);
        out.extend_from_slice(b"\nendstream\nendobj\n");
    }

    // Cross-reference table.
    let xref_pos = out.len();
    out.extend_from_slice(format!("xref\n0 {}\n", total_objs + 1).as_bytes());
    out.extend_from_slice(b"0000000000 65535 f \n");
    for i in 1..=total_objs {
        out.extend_from_slice(format!("{:010} 00000 n \n", offsets[i]).as_bytes());
    }
    out.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref_pos}\n%%EOF\n",
            total_objs + 1
        )
        .as_bytes(),
    );
    out
}

/// Format a coordinate with up to 3 decimals and no trailing zeros (PDF accepts plain decimals).
fn fmt_num(v: f32) -> String {
    let s = format!("{v:.3}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s.is_empty() {
        "0".to_string()
    } else {
        s.to_string()
    }
}

/// Wrap `data` in a minimal zlib stream using only stored (uncompressed) DEFLATE blocks.
/// Mirrors the technique in [`crate::png`] but kept local so the PDF writer is self-contained.
fn zlib_store(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len() + data.len() / 0xFFFF * 5 + 16);
    out.push(0x78); // CMF: deflate, 32K window
    out.push(0x01); // FLG: fastest; check bits make the header %31 == 0
    if data.is_empty() {
        out.push(1);
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&(!0u16).to_le_bytes());
    } else {
        let mut pos = 0usize;
        while pos < data.len() {
            let chunk = (data.len() - pos).min(0xFFFF);
            let is_final = pos + chunk >= data.len();
            out.push(if is_final { 1 } else { 0 }); // BFINAL bit, BTYPE = 00 (stored)
            let len = chunk as u16;
            out.extend_from_slice(&len.to_le_bytes());
            out.extend_from_slice(&(!len).to_le_bytes());
            out.extend_from_slice(&data[pos..pos + chunk]);
            pos += chunk;
        }
    }
    out.extend_from_slice(&adler32(data).to_be_bytes());
    out
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

#[cfg(test)]
mod tests {
    use super::*;
    use hayate_ir::color::{Color, ThemeColorToken};
    use hayate_ir::geom::RectEmu;
    use hayate_ir::paint::Fill;
    use hayate_ir::shape::Geometry;
    use hayate_ir::theme::Theme;
    use hayate_ir::units::inch_f;

    fn count(haystack: &[u8], needle: &[u8]) -> usize {
        haystack
            .windows(needle.len())
            .filter(|w| *w == needle)
            .count()
    }

    fn deck(slides: usize) -> Presentation {
        let mut p = Presentation::new();
        let master = p.add_master(Theme::default());
        let layout = p.add_layout(master, "Blank");
        for _ in 0..slides {
            let slide = p.add_slide(layout);
            let e = p.add_shape(slide);
            p.world.frames.insert(
                e,
                RectEmu::new(inch_f(1.0), inch_f(1.0), inch_f(3.0), inch_f(2.0)),
            );
            p.world.geometries.insert(e, Geometry::Rect);
            p.world
                .fills
                .insert(e, Fill::Solid(Color::theme(ThemeColorToken::Accent1)));
        }
        p
    }

    #[test]
    fn exports_one_page_per_slide() {
        let p = deck(3);
        let pdf = export_pdf(&p, 1.0);
        assert!(pdf.starts_with(b"%PDF-"), "has a PDF header");
        assert!(count(&pdf, b"%%EOF") >= 1, "has an EOF marker");
        assert_eq!(
            count(&pdf, b"/Type /Page /Parent"),
            3,
            "one Page object per slide"
        );
        assert!(count(&pdf, b"/Subtype /Image") >= 3, "an image per slide");
        assert!(
            count(&pdf, b"/FlateDecode") >= 3,
            "image streams are deflated"
        );
    }

    #[test]
    fn empty_deck_is_still_valid() {
        let p = deck(0);
        let pdf = export_pdf(&p, 1.0);
        assert!(pdf.starts_with(b"%PDF-"));
        assert!(count(&pdf, b"%%EOF") >= 1);
        assert!(count(&pdf, b"/Count 0") >= 1);
    }
}
