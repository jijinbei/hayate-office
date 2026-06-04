//! Glyph subsetting for PDF font embedding. Builds a minimal TrueType (`glyf`) font containing
//! only the used glyph ids, reading each glyph's outline from the source font (works for both
//! TrueType `glyf` and CFF/OpenType sources; cubic curves are approximated by quadratics). Glyph
//! ids are preserved (unused glyphs become empty), so a PDF `/CIDToGIDMap /Identity` stays valid
//! and the embedded font shrinks from megabytes (full CJK face) to the few used glyphs.
//!
//! Curve handling: both cubic (`curve_to`) and quadratic (`quad_to`) segments are FLATTENED into
//! short straight line segments (all on-curve points). TrueType `glyf` is quadratic-only, so a
//! cubic cannot be represented directly; flattening into line segments is trivially correct in the
//! `glyf` model (every emitted point is on-curve) and visually indistinguishable at text sizes. We
//! flatten cubics with K=8 segments and quadratics with K=4 segments for uniformity, avoiding the
//! subtle bugs of an on-the-fly cubic->quadratic control-point conversion.

use std::collections::BTreeSet;
use ttf_parser::{GlyphId, OutlineBuilder};

/// Subdivision counts for flattening curves into line segments.
const CUBIC_SEGMENTS: u32 = 16;
const QUAD_SEGMENTS: u32 = 10;

/// A single outline point in font units. All points are on-curve (we flatten curves to lines).
#[derive(Clone, Copy)]
struct Point {
    x: i32,
    y: i32,
    on_curve: bool,
}

/// Collects glyph outlines as integer contours of on-curve points.
struct ContourBuilder {
    contours: Vec<Vec<Point>>,
    cur: Vec<Point>,
    // Last position (f32 font units), used as the start of each curve segment.
    last_x: f32,
    last_y: f32,
    // Start of the current contour, for implicit closing.
    start_x: f32,
    start_y: f32,
}

impl ContourBuilder {
    fn new() -> Self {
        ContourBuilder {
            contours: Vec::new(),
            cur: Vec::new(),
            last_x: 0.0,
            last_y: 0.0,
            start_x: 0.0,
            start_y: 0.0,
        }
    }

    fn flush(&mut self) {
        if !self.cur.is_empty() {
            self.contours.push(std::mem::take(&mut self.cur));
        }
    }

    fn push(&mut self, x: f32, y: f32) {
        self.cur.push(Point {
            x: x.round() as i32,
            y: y.round() as i32,
            on_curve: true,
        });
        self.last_x = x;
        self.last_y = y;
    }
}

impl OutlineBuilder for ContourBuilder {
    fn move_to(&mut self, x: f32, y: f32) {
        // Start a new contour.
        self.flush();
        self.start_x = x;
        self.start_y = y;
        self.push(x, y);
    }

    fn line_to(&mut self, x: f32, y: f32) {
        self.push(x, y);
    }

    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        let (x0, y0) = (self.last_x, self.last_y);
        // Flatten the quadratic Bezier into QUAD_SEGMENTS line segments.
        for i in 1..=QUAD_SEGMENTS {
            let t = i as f32 / QUAD_SEGMENTS as f32;
            let mt = 1.0 - t;
            let bx = mt * mt * x0 + 2.0 * mt * t * x1 + t * t * x;
            let by = mt * mt * y0 + 2.0 * mt * t * y1 + t * t * y;
            self.push(bx, by);
        }
    }

    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        let (x0, y0) = (self.last_x, self.last_y);
        // Flatten the cubic Bezier into CUBIC_SEGMENTS line segments.
        for i in 1..=CUBIC_SEGMENTS {
            let t = i as f32 / CUBIC_SEGMENTS as f32;
            let mt = 1.0 - t;
            let a = mt * mt * mt;
            let b = 3.0 * mt * mt * t;
            let c = 3.0 * mt * t * t;
            let d = t * t * t;
            let bx = a * x0 + b * x1 + c * x2 + d * x;
            let by = a * y0 + b * y1 + c * y2 + d * y;
            self.push(bx, by);
        }
    }

    fn close(&mut self) {
        // glyf contours are implicitly closed; drop a redundant trailing point equal to start.
        if let Some(last) = self.cur.last() {
            let sx = self.start_x.round() as i32;
            let sy = self.start_y.round() as i32;
            if last.x == sx && last.y == sy && self.cur.len() > 1 {
                self.cur.pop();
            }
        }
        self.flush();
    }
}

/// Big-endian write helpers.
fn push_u16(buf: &mut Vec<u8>, v: u16) {
    buf.extend_from_slice(&v.to_be_bytes());
}
fn push_i16(buf: &mut Vec<u8>, v: i16) {
    buf.extend_from_slice(&v.to_be_bytes());
}
fn push_u32(buf: &mut Vec<u8>, v: u32) {
    buf.extend_from_slice(&v.to_be_bytes());
}

/// Encode one non-empty glyph in TrueType simple-glyph format. Returns the encoded bytes.
fn encode_simple_glyph(contours: &[Vec<Point>]) -> Vec<u8> {
    let mut end_pts = Vec::with_capacity(contours.len());
    let mut running = 0usize;
    for c in contours {
        running += c.len();
        end_pts.push((running - 1) as u16);
    }

    let (mut x_min, mut y_min, mut x_max, mut y_max) = (i32::MAX, i32::MAX, i32::MIN, i32::MIN);
    for p in contours.iter().flatten() {
        x_min = x_min.min(p.x);
        y_min = y_min.min(p.y);
        x_max = x_max.max(p.x);
        y_max = y_max.max(p.y);
    }

    let mut buf = Vec::new();
    push_i16(&mut buf, contours.len() as i16);
    push_i16(&mut buf, x_min as i16);
    push_i16(&mut buf, y_min as i16);
    push_i16(&mut buf, x_max as i16);
    push_i16(&mut buf, y_max as i16);
    for e in &end_pts {
        push_u16(&mut buf, *e);
    }
    // instructionLength = 0
    push_u16(&mut buf, 0);

    // flags: one byte per point, only ON_CURVE_POINT (0x01) set (all points are on-curve).
    for p in contours.iter().flatten() {
        buf.push(if p.on_curve { 0x01 } else { 0x00 });
    }

    // xCoordinates: signed 16-bit deltas from previous point (first from 0).
    let mut prev = 0i32;
    for p in contours.iter().flatten() {
        let d = (p.x - prev) as i16;
        push_i16(&mut buf, d);
        prev = p.x;
    }
    // yCoordinates
    let mut prev = 0i32;
    for p in contours.iter().flatten() {
        let d = (p.y - prev) as i16;
        push_i16(&mut buf, d);
        prev = p.y;
    }

    // Pad to 2-byte boundary.
    if buf.len() % 2 != 0 {
        buf.push(0);
    }
    buf
}

/// Build a subset TrueType font from `font_data` keeping only `used_gids` (plus glyph 0). Returns
/// `None` when the font can't be parsed or produces no usable outlines, in which case the caller
/// falls back to embedding the whole face.
pub fn subset_to_glyf(font_data: &[u8], used_gids: &BTreeSet<u16>) -> Option<Vec<u8>> {
    let face = ttf_parser::Face::parse(font_data, 0).ok()?;
    let units_per_em = face.units_per_em();
    if units_per_em == 0 {
        return None;
    }
    let num_glyphs = face.number_of_glyphs();
    if num_glyphs == 0 {
        return None;
    }

    // Build per-glyph encoded data (empty for glyphs not included or with no outline).
    // Glyph 0 (.notdef) is always included.
    let mut glyf = Vec::new();
    let mut loca: Vec<u32> = Vec::with_capacity(num_glyphs as usize + 1);
    loca.push(0);

    let mut max_points: u16 = 0;
    let mut max_contours: u16 = 0;
    let mut max_advance: u16 = 0;
    let mut advances: Vec<u16> = Vec::with_capacity(num_glyphs as usize);
    let (mut f_x_min, mut f_y_min, mut f_x_max, mut f_y_max) =
        (i32::MAX, i32::MAX, i32::MIN, i32::MIN);
    let mut any_outline = false;

    for gid in 0..num_glyphs {
        // Track max advance across ALL glyphs (numberOfHMetrics == numGlyphs).
        let adv = face.glyph_hor_advance(GlyphId(gid)).unwrap_or(0);
        max_advance = max_advance.max(adv);
        advances.push(adv);

        let encoded = if gid == 0 || used_gids.contains(&gid) {
            let mut cb = ContourBuilder::new();
            let bbox = face.outline_glyph(GlyphId(gid), &mut cb);
            cb.flush();
            // Drop empty contours (can arise from degenerate input).
            cb.contours.retain(|c| !c.is_empty());
            if bbox.is_some() && !cb.contours.is_empty() {
                any_outline = true;
                let n_points: usize = cb.contours.iter().map(|c| c.len()).sum();
                max_points = max_points.max(n_points as u16);
                max_contours = max_contours.max(cb.contours.len() as u16);
                for c in &cb.contours {
                    for p in c {
                        f_x_min = f_x_min.min(p.x);
                        f_y_min = f_y_min.min(p.y);
                        f_x_max = f_x_max.max(p.x);
                        f_y_max = f_y_max.max(p.y);
                    }
                }
                encode_simple_glyph(&cb.contours)
            } else {
                Vec::new()
            }
        } else {
            Vec::new()
        };

        glyf.extend_from_slice(&encoded);
        loca.push(glyf.len() as u32);
    }

    if !any_outline {
        return None;
    }

    if f_x_min == i32::MAX {
        // No bbox collected; fall back to face global bounding box (already in font units).
        let gb = face.global_bounding_box();
        f_x_min = gb.x_min as i32;
        f_y_min = gb.y_min as i32;
        f_x_max = gb.x_max as i32;
        f_y_max = gb.y_max as i32;
    }

    // ---- loca (LONG format) ----
    // By spec, loca has numGlyphs+1 entries. But ttf-parser (and the loca format) cannot express
    // more than u16::MAX offsets: when numGlyphs == 65535 the offset count would be 65536, which
    // overflows the 16-bit loca array length and makes parsers reject the whole glyf table. In
    // that edge case we drop the trailing sentinel and emit exactly numGlyphs (=65535) entries;
    // the last representable glyph id (65534) still gets a valid [loca[i], loca[i+1]) range, and
    // glyph 65535 (== u16::MAX) is unaddressable by the loca format anyway.
    let loca_entries = if num_glyphs == u16::MAX {
        &loca[..u16::MAX as usize]
    } else {
        &loca[..]
    };
    let mut loca_buf = Vec::with_capacity(loca_entries.len() * 4);
    for off in loca_entries {
        push_u32(&mut loca_buf, *off);
    }

    // ---- head ----
    let mut head = Vec::new();
    push_u32(&mut head, 0x0001_0000); // version
    push_u32(&mut head, 0x0001_0000); // fontRevision
    push_u32(&mut head, 0); // checkSumAdjustment
    push_u32(&mut head, 0x5F0F_3CF5); // magicNumber
    push_u16(&mut head, 0); // flags
    push_u16(&mut head, units_per_em); // unitsPerEm
    push_u32(&mut head, 0); // created (hi)
    push_u32(&mut head, 0); // created (lo)
    push_u32(&mut head, 0); // modified (hi)
    push_u32(&mut head, 0); // modified (lo)
    push_i16(&mut head, f_x_min as i16);
    push_i16(&mut head, f_y_min as i16);
    push_i16(&mut head, f_x_max as i16);
    push_i16(&mut head, f_y_max as i16);
    push_u16(&mut head, 0); // macStyle
    push_u16(&mut head, 8); // lowestRecPPEM
    push_i16(&mut head, 2); // fontDirectionHint
    push_i16(&mut head, 1); // indexToLocFormat = LONG
    push_i16(&mut head, 0); // glyphDataFormat

    // ---- maxp (version 1.0) ----
    let mut maxp = Vec::new();
    push_u32(&mut maxp, 0x0001_0000); // version
    push_u16(&mut maxp, num_glyphs); // numGlyphs
    push_u16(&mut maxp, max_points); // maxPoints
    push_u16(&mut maxp, max_contours); // maxContours
    push_u16(&mut maxp, 0); // maxCompositePoints
    push_u16(&mut maxp, 0); // maxCompositeContours
    push_u16(&mut maxp, 0); // maxZones
    push_u16(&mut maxp, 0); // maxTwilightPoints
    push_u16(&mut maxp, 0); // maxStorage
    push_u16(&mut maxp, 0); // maxFunctionDefs
    push_u16(&mut maxp, 0); // maxInstructionDefs
    push_u16(&mut maxp, 0); // maxStackElements
    push_u16(&mut maxp, 0); // maxSizeOfInstructions
    push_u16(&mut maxp, 0); // maxComponentElements
    push_u16(&mut maxp, 0); // maxComponentDepth

    // ---- hhea ----
    let mut hhea = Vec::new();
    push_u32(&mut hhea, 0x0001_0000); // version
    push_i16(&mut hhea, face.ascender());
    push_i16(&mut hhea, face.descender());
    push_i16(&mut hhea, face.line_gap());
    push_u16(&mut hhea, max_advance); // advanceWidthMax
    push_i16(&mut hhea, 0); // minLeftSideBearing
    push_i16(&mut hhea, 0); // minRightSideBearing
    push_i16(&mut hhea, f_x_max as i16); // xMaxExtent
    push_i16(&mut hhea, 1); // caretSlopeRise
    push_i16(&mut hhea, 0); // caretSlopeRun
    push_i16(&mut hhea, 0); // caretOffset
    push_i16(&mut hhea, 0); // reserved
    push_i16(&mut hhea, 0); // reserved
    push_i16(&mut hhea, 0); // reserved
    push_i16(&mut hhea, 0); // reserved
    push_i16(&mut hhea, 0); // metricDataFormat
    push_u16(&mut hhea, num_glyphs); // numberOfHMetrics

    // ---- hmtx ----
    let mut hmtx = Vec::with_capacity(num_glyphs as usize * 4);
    for adv in &advances {
        push_u16(&mut hmtx, *adv); // advanceWidth
        push_i16(&mut hmtx, 0); // lsb
    }

    // ---- cmap (format 4, single dummy segment) ----
    let mut cmap = Vec::new();
    push_u16(&mut cmap, 0); // version
    push_u16(&mut cmap, 1); // numTables
    push_u16(&mut cmap, 3); // platformID (Windows)
    push_u16(&mut cmap, 1); // encodingID (Unicode BMP)
    push_u32(&mut cmap, 12); // offset to subtable (4 + 8)
                             // format-4 subtable, segCount = 1 -> segCountX2 = 2
    let mut sub = Vec::new();
    push_u16(&mut sub, 4); // format
    push_u16(&mut sub, 24); // length (4*2 header + 2 + 2*4 arrays + 2 reservedPad = 24)
    push_u16(&mut sub, 0); // language
    push_u16(&mut sub, 2); // segCountX2
    push_u16(&mut sub, 2); // searchRange = 2*2^floor(log2(segCount))
    push_u16(&mut sub, 0); // entrySelector
    push_u16(&mut sub, 0); // rangeShift = segCountX2 - searchRange
    push_u16(&mut sub, 0xFFFF); // endCode[0]
    push_u16(&mut sub, 0); // reservedPad
    push_u16(&mut sub, 0xFFFF); // startCode[0]
    push_u16(&mut sub, 1); // idDelta[0]
    push_u16(&mut sub, 0); // idRangeOffset[0]
    cmap.extend_from_slice(&sub);

    // ---- post (version 3.0) ----
    let mut post = Vec::new();
    push_u32(&mut post, 0x0003_0000); // version 3.0
    push_u32(&mut post, 0); // italicAngle
    push_i16(&mut post, 0); // underlinePosition
    push_i16(&mut post, 0); // underlineThickness
    push_u32(&mut post, 0); // isFixedPitch
    push_u32(&mut post, 0); // minMemType42
    push_u32(&mut post, 0); // maxMemType42
    push_u32(&mut post, 0); // minMemType1
    push_u32(&mut post, 0); // maxMemType1

    // ---- assemble sfnt ----
    // (tag, data) — must be sorted by tag ascending in the directory.
    let tables: Vec<(&[u8; 4], Vec<u8>)> = vec![
        (b"cmap", cmap),
        (b"glyf", glyf),
        (b"head", head),
        (b"hhea", hhea),
        (b"hmtx", hmtx),
        (b"loca", loca_buf),
        (b"maxp", maxp),
        (b"post", post),
    ];

    let num_tables = tables.len() as u16;
    // searchRange = (2^floor(log2(numTables))) * 16
    let mut pow2 = 1u16;
    while pow2 * 2 <= num_tables {
        pow2 *= 2;
    }
    let search_range = pow2 * 16;
    let entry_selector = (pow2 as f32).log2() as u16;
    let range_shift = num_tables * 16 - search_range;

    let mut out = Vec::new();
    push_u32(&mut out, 0x0001_0000); // sfntVersion
    push_u16(&mut out, num_tables);
    push_u16(&mut out, search_range);
    push_u16(&mut out, entry_selector);
    push_u16(&mut out, range_shift);

    // Table data begins after the directory.
    let dir_size = 12 + tables.len() * 16;
    let mut offset = dir_size as u32;

    // First pass: write directory records with computed offsets.
    for (tag, data) in &tables {
        out.extend_from_slice(*tag);
        push_u32(&mut out, 0); // checksum (poppler does not verify)
        push_u32(&mut out, offset);
        push_u32(&mut out, data.len() as u32);
        let padded = (data.len() + 3) & !3;
        offset += padded as u32;
    }

    // Second pass: write table data, each padded to 4 bytes.
    for (_tag, data) in &tables {
        out.extend_from_slice(data);
        while out.len() % 4 != 0 {
            out.push(0);
        }
    }

    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    /// Scan gids 1..200 for the first `n` glyphs that produce a non-empty outline.
    fn find_gids_with_outlines(face: &ttf_parser::Face, n: usize) -> Vec<u16> {
        struct Sink {
            count: usize,
        }
        impl OutlineBuilder for Sink {
            fn move_to(&mut self, _: f32, _: f32) {
                self.count += 1;
            }
            fn line_to(&mut self, _: f32, _: f32) {
                self.count += 1;
            }
            fn quad_to(&mut self, _: f32, _: f32, _: f32, _: f32) {
                self.count += 1;
            }
            fn curve_to(&mut self, _: f32, _: f32, _: f32, _: f32, _: f32, _: f32) {
                self.count += 1;
            }
            fn close(&mut self) {}
        }
        let mut found = Vec::new();
        let max = face.number_of_glyphs().min(200);
        for gid in 1..max {
            let mut s = Sink { count: 0 };
            if let Some(bbox) = face.outline_glyph(GlyphId(gid), &mut s) {
                if s.count > 0 && bbox.width() > 0 && bbox.height() > 0 {
                    found.push(gid);
                    if found.len() >= n {
                        break;
                    }
                }
            }
        }
        found
    }

    fn run_for_font(path: &str, cjk_size_check: bool) {
        let data = match std::fs::read(path) {
            Ok(d) => d,
            Err(_) => {
                eprintln!("skipping {path}: not present");
                return;
            }
        };
        let face = match ttf_parser::Face::parse(&data, 0) {
            Ok(f) => f,
            Err(_) => {
                eprintln!("skipping {path}: parse failed");
                return;
            }
        };

        let gids = find_gids_with_outlines(&face, 4);
        assert!(!gids.is_empty(), "no usable gids found in {path}");

        let mut used: BTreeSet<u16> = BTreeSet::new();
        for g in &gids {
            used.insert(*g);
        }

        let sub = subset_to_glyf(&data, &used).expect("subset_to_glyf returned None");

        // Re-parse the subset.
        let subface = ttf_parser::Face::parse(&sub, 0).expect("subset failed to re-parse");
        assert_eq!(
            subface.number_of_glyphs(),
            face.number_of_glyphs(),
            "numGlyphs must be preserved for {path}"
        );

        // Each used gid must yield a non-empty bounding box.
        struct Sink;
        impl OutlineBuilder for Sink {
            fn move_to(&mut self, _: f32, _: f32) {}
            fn line_to(&mut self, _: f32, _: f32) {}
            fn quad_to(&mut self, _: f32, _: f32, _: f32, _: f32) {}
            fn curve_to(&mut self, _: f32, _: f32, _: f32, _: f32, _: f32, _: f32) {}
            fn close(&mut self) {}
        }
        for g in &gids {
            let bbox = subface
                .outline_glyph(GlyphId(*g), &mut Sink)
                .unwrap_or_else(|| panic!("gid {g} has no outline in subset of {path}"));
            assert!(
                bbox.width() > 0 && bbox.height() > 0,
                "gid {g} has empty bbox in subset of {path}"
            );
        }

        if cjk_size_check {
            // The raw subset sfnt must be dramatically smaller than the multi-megabyte source
            // face (only a handful of glyphs carry real `glyf` data). The `loca`/`hmtx` tables are
            // O(numGlyphs) and dominate the raw size for a 65535-glyph CJK face, but they are
            // extremely repetitive: the PDF embeds the stream under FlateDecode, where they shrink
            // to almost nothing. We assert both: a big raw reduction and a tiny compressed size.
            assert!(
                sub.len() < data.len() / 4,
                "subset not far smaller than source for {path}: {} vs {}",
                sub.len(),
                data.len()
            );
            use flate2::{write::ZlibEncoder, Compression};
            use std::io::Write;
            let mut enc = ZlibEncoder::new(Vec::new(), Compression::default());
            enc.write_all(&sub).unwrap();
            let compressed = enc.finish().unwrap();
            assert!(
                compressed.len() < 200 * 1024,
                "FlateDecode-compressed CJK subset too large: {} bytes ({path})",
                compressed.len()
            );
            eprintln!(
                "{path}: source {} -> raw subset {} -> flate {} bytes",
                data.len(),
                sub.len(),
                compressed.len()
            );
            return;
        }

        eprintln!(
            "{path}: source {} -> subset {} bytes",
            data.len(),
            sub.len()
        );
    }

    #[test]
    fn subset_dejavu_glyf() {
        run_for_font("/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf", false);
    }

    #[test]
    fn subset_noto_cjk_cff() {
        run_for_font(
            "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
            true,
        );
    }
}
