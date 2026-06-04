//! Embed a TrueType/OpenType font into a PDF as a Type0 (composite) font with Identity-H
//! encoding, so glyphs can be shown by glyph id (2-byte) and text stays selectable/extractable
//! via a ToUnicode CMap. See [`build_type0_font`].

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::io::Write as _;

use flate2::write::ZlibEncoder;
use flate2::Compression;
use ttf_parser::{Face, GlyphId};

/// A built Type0 font ready to splice into a PDF: the serialized objects (each a full
/// `"N 0 obj ... endobj\n"`), the object id of the referenceable `/Type0` font, and how many
/// object ids were consumed (starting at the `base_obj_id` passed in).
pub struct CidFont {
    /// `(object_id, serialized_object_bytes)` for every object this font needs.
    pub objects: Vec<(u32, Vec<u8>)>,
    /// The `/Type0` font object id to reference from a page's `/Resources /Font`.
    pub font_obj_id: u32,
    /// Number of consecutive object ids consumed, starting at `base_obj_id`.
    pub obj_count: u32,
}

/// Build a Type0/CIDFontType2 font embedding `font_data`, covering the glyphs in `used_glyphs`
/// (`glyph_id -> the original text for that glyph`, used for the ToUnicode CMap). Object ids are
/// allocated consecutively from `base_obj_id`; `subtag` is a short unique tag (e.g. "F0") used in
/// the BaseFont name.
///
/// TrueType Collection note: if `font_data` is a `.ttc` (starts with `b"ttcf"`), embedding the
/// whole collection raw is technically invalid PDF. We keep the implementation simple and embed
/// the collection bytes as-is with `/Length1` set to the full (uncompressed) length; poppler
/// generally accepts a `.ttc` blob in `FontFile2`. A proper single-face sfnt rebuild is left out
/// for simplicity since the project's exporter feeds plain TTF/OTF faces.
pub fn build_type0_font(
    font_data: &[u8],
    used_glyphs: &BTreeMap<u16, String>,
    base_obj_id: u32,
    subtag: &str,
) -> CidFont {
    // Parse the face (index 0 handles ttc as well). Bail out gracefully on failure.
    let face = match Face::parse(font_data, 0) {
        Ok(f) => f,
        Err(_) => {
            return CidFont {
                objects: Vec::new(),
                font_obj_id: base_obj_id,
                obj_count: 0,
            };
        }
    };

    let units_per_em = {
        let u = face.units_per_em();
        if u == 0 {
            1000.0
        } else {
            u as f64
        }
    };
    let scale = 1000.0 / units_per_em;

    // Object id allocation (consecutive, starting at base_obj_id).
    let type0_id = base_obj_id;
    let cidfont_id = base_obj_id + 1;
    let descr_id = base_obj_id + 2;
    let fontfile_id = base_obj_id + 3;
    let tounicode_id = base_obj_id + 4;
    let obj_count = 5;

    let base_font = make_base_font_name(subtag);

    let mut objects: Vec<(u32, Vec<u8>)> = Vec::with_capacity(obj_count as usize);

    // 1. Type0 font.
    {
        let body = format!(
            "<< /Type /Font /Subtype /Type0 /BaseFont /{base_font} /Encoding /Identity-H \
/DescendantFonts [{cidfont_id} 0 R] /ToUnicode {tounicode_id} 0 R >>"
        );
        objects.push((type0_id, serialize_obj(type0_id, &body)));
    }

    // 2. CIDFontType2 (descendant), with per-glyph widths.
    {
        let w_array = build_w_array(&face, used_glyphs, scale);
        let body = format!(
            "<< /Type /Font /Subtype /CIDFontType2 /BaseFont /{base_font} \
/CIDSystemInfo << /Registry (Adobe) /Ordering (Identity) /Supplement 0 >> \
/FontDescriptor {descr_id} 0 R /CIDToGIDMap /Identity /DW 1000 /W {w_array} >>"
        );
        objects.push((cidfont_id, serialize_obj(cidfont_id, &body)));
    }

    // 3. FontDescriptor.
    {
        let bbox = face.global_bounding_box();
        let x0 = (bbox.x_min as f64 * scale).round() as i64;
        let y0 = (bbox.y_min as f64 * scale).round() as i64;
        let x1 = (bbox.x_max as f64 * scale).round() as i64;
        let y1 = (bbox.y_max as f64 * scale).round() as i64;

        let ascent = (face.ascender() as f64 * scale).round() as i64;
        let descent = (face.descender() as f64 * scale).round() as i64;
        let cap_height = match face.capital_height() {
            Some(c) if c != 0 => (c as f64 * scale).round() as i64,
            _ => (face.ascender() as f64 * 0.7 * scale).round() as i64,
        };

        // /Flags 4 = symbolic (safe default).
        let body = format!(
            "<< /Type /FontDescriptor /FontName /{base_font} /Flags 4 \
/FontBBox [{x0} {y0} {x1} {y1}] /ItalicAngle 0 /Ascent {ascent} /Descent {descent} \
/CapHeight {cap_height} /StemV 80 /FontFile2 {fontfile_id} 0 R >>"
        );
        objects.push((descr_id, serialize_obj(descr_id, &body)));
    }

    // 4. FontFile2 stream: zlib-compressed font program. Prefer a glyf subset containing only the
    // used glyphs (shrinks large CJK faces dramatically); fall back to the whole face, de-collected
    // from a .ttc into a standalone sfnt (a .ttc is not a valid FontFile2 payload).
    {
        let used: std::collections::BTreeSet<u16> = used_glyphs.keys().copied().collect();
        let sfnt: std::borrow::Cow<[u8]> = match crate::pdf_subset::subset_to_glyf(font_data, &used)
        {
            Some(v) => std::borrow::Cow::Owned(v),
            None => extract_sfnt(font_data),
        };
        let compressed = zlib_compress(&sfnt);
        let dict = format!(
            "<< /Filter /FlateDecode /Length {} /Length1 {} >>",
            compressed.len(),
            sfnt.len()
        );
        objects.push((
            fontfile_id,
            serialize_stream(fontfile_id, &dict, &compressed),
        ));
    }

    // 5. ToUnicode CMap stream (FlateDecode-compressed).
    {
        let cmap = build_tounicode_cmap(used_glyphs);
        let compressed = zlib_compress(cmap.as_bytes());
        let dict = format!("<< /Filter /FlateDecode /Length {} >>", compressed.len());
        objects.push((
            tounicode_id,
            serialize_stream(tounicode_id, &dict, &compressed),
        ));
    }

    CidFont {
        objects,
        font_obj_id: type0_id,
        obj_count,
    }
}

/// If `data` is a TrueType Collection (`ttcf`), rebuild face 0 as a standalone sfnt (a valid
/// FontFile2 payload); otherwise return the data unchanged. Does NOT subset glyphs — the whole
/// face is embedded (a future optimization is to subset to the used glyph ids).
fn extract_sfnt(data: &[u8]) -> std::borrow::Cow<'_, [u8]> {
    if data.len() < 12 || &data[0..4] != b"ttcf" {
        return std::borrow::Cow::Borrowed(data);
    }
    let be32 = |o: usize| -> usize {
        u32::from_be_bytes([data[o], data[o + 1], data[o + 2], data[o + 3]]) as usize
    };
    let be16 = |o: usize| -> usize { u16::from_be_bytes([data[o], data[o + 1]]) as usize };
    // numFonts at 8; first font's table-directory offset at 12.
    if be32(8) == 0 {
        return std::borrow::Cow::Borrowed(data);
    }
    let dir = be32(12);
    if dir + 12 > data.len() {
        return std::borrow::Cow::Borrowed(data);
    }
    let sfnt_version = be32(dir);
    let num_tables = be16(dir + 4);
    // Read the table records (tag, checksum, offset, length) pointing into the ttc.
    let mut records: Vec<(u32, u32, usize, usize)> = Vec::with_capacity(num_tables);
    for i in 0..num_tables {
        let r = dir + 12 + i * 16;
        if r + 16 > data.len() {
            return std::borrow::Cow::Borrowed(data);
        }
        let tag = u32::from_be_bytes([data[r], data[r + 1], data[r + 2], data[r + 3]]);
        let checksum = u32::from_be_bytes([data[r + 4], data[r + 5], data[r + 6], data[r + 7]]);
        records.push((tag, checksum, be32(r + 8), be32(r + 12)));
    }
    // Rebuild a standalone sfnt: header + table directory (new offsets) + 4-byte-aligned tables.
    let n = records.len();
    let mut out: Vec<u8> = Vec::new();
    out.extend_from_slice(&(sfnt_version as u32).to_be_bytes());
    let entry_selector = (15u16 - (n as u16).leading_zeros() as u16).min(15);
    let search_range = (1u16 << entry_selector) * 16;
    let range_shift = (n as u16) * 16 - search_range;
    out.extend_from_slice(&(n as u16).to_be_bytes());
    out.extend_from_slice(&search_range.to_be_bytes());
    out.extend_from_slice(&entry_selector.to_be_bytes());
    out.extend_from_slice(&range_shift.to_be_bytes());
    let mut offset = 12 + n * 16;
    let mut bodies: Vec<&[u8]> = Vec::with_capacity(n);
    for (tag, checksum, src_off, len) in &records {
        if src_off + len > data.len() {
            return std::borrow::Cow::Borrowed(data);
        }
        out.extend_from_slice(&tag.to_be_bytes());
        out.extend_from_slice(&checksum.to_be_bytes());
        out.extend_from_slice(&(offset as u32).to_be_bytes());
        out.extend_from_slice(&(*len as u32).to_be_bytes());
        bodies.push(&data[*src_off..src_off + len]);
        offset += len + ((4 - (len % 4)) % 4); // 4-byte aligned
    }
    for body in bodies {
        out.extend_from_slice(body);
        let pad = (4 - (body.len() % 4)) % 4;
        out.resize(out.len() + pad, 0);
    }
    std::borrow::Cow::Owned(out)
}

/// Build a valid subset tag of the form `AAAAAA+Embedded`: 6 uppercase letters derived from
/// `subtag`, then `+Embedded`.
fn make_base_font_name(subtag: &str) -> String {
    let mut tag = String::with_capacity(6);
    for ch in subtag.chars() {
        if tag.len() >= 6 {
            break;
        }
        if ch.is_ascii_alphabetic() {
            tag.push(ch.to_ascii_uppercase());
        } else if ch.is_ascii_digit() {
            // Map digits 0..9 -> letters A..J so the tag stays letters-only.
            tag.push((b'A' + (ch as u8 - b'0')) as char);
        }
    }
    while tag.len() < 6 {
        tag.push('A');
    }
    format!("{tag}+Embedded")
}

/// Build the `/W` array: per-glyph advance widths in 1000-units-per-em glyph space. Each used
/// glyph that has an advance is emitted as a `glyph_id [w]` run. Glyphs without an advance are
/// omitted.
fn build_w_array(face: &Face, used_glyphs: &BTreeMap<u16, String>, scale: f64) -> String {
    let mut out = String::from("[ ");
    for &gid in used_glyphs.keys() {
        if let Some(adv) = face.glyph_hor_advance(GlyphId(gid)) {
            let w = (adv as f64 * scale).round() as i64;
            write!(out, "{gid} [{w}] ").unwrap();
        }
    }
    out.push(']');
    out
}

/// Build the ToUnicode CMap text mapping 2-byte glyph codes (== glyph ids, big-endian, since
/// Identity-H uses glyph id = code) to their Unicode values (UTF-16BE) from `used_glyphs`.
/// `beginbfchar` entries are chunked into groups of at most 100.
fn build_tounicode_cmap(used_glyphs: &BTreeMap<u16, String>) -> String {
    let mut out = String::new();
    out.push_str("/CIDInit /ProcSet findresource begin 12 dict begin begincmap\n");
    out.push_str("/CIDSystemInfo << /Registry (Adobe) /Ordering (UCS) /Supplement 0 >> def\n");
    out.push_str("/CMapName /Adobe-Identity-UCS def /CMapType 2 def\n");
    out.push_str("1 begincodespacerange <0000> <FFFF> endcodespacerange\n");

    let entries: Vec<(u16, String)> = used_glyphs
        .iter()
        .map(|(&gid, text)| {
            let mut dst = String::new();
            for cu in text.encode_utf16() {
                write!(dst, "{cu:04X}").unwrap();
            }
            // Guard against empty text -> map to U+0000 so the entry stays valid.
            if dst.is_empty() {
                dst.push_str("0000");
            }
            (gid, dst)
        })
        .collect();

    for chunk in entries.chunks(100) {
        write!(out, "{} beginbfchar\n", chunk.len()).unwrap();
        for (gid, dst) in chunk {
            writeln!(out, "<{gid:04X}> <{dst}>").unwrap();
        }
        out.push_str("endbfchar\n");
    }

    out.push_str("endcmap CMapEndProc end end\n");
    out
}

/// zlib-compress bytes via flate2 `ZlibEncoder` (default compression).
fn zlib_compress(data: &[u8]) -> Vec<u8> {
    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
    encoder
        .write_all(data)
        .expect("zlib write to Vec never fails");
    encoder.finish().expect("zlib finish to Vec never fails")
}

/// Serialize a non-stream object: `"{id} 0 obj\n{body}\nendobj\n"`.
fn serialize_obj(id: u32, body: &str) -> Vec<u8> {
    format!("{id} 0 obj\n{body}\nendobj\n").into_bytes()
}

/// Serialize a stream object: dict + `stream\n` + raw bytes + `\nendstream\nendobj\n`.
fn serialize_stream(id: u32, dict: &str, data: &[u8]) -> Vec<u8> {
    let mut out = format!("{id} 0 obj\n{dict}\nstream\n").into_bytes();
    out.extend_from_slice(data);
    out.extend_from_slice(b"\nendstream\nendobj\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Locate a usable TTF for testing; returns None to skip the test if none exist.
    fn find_test_ttf() -> Option<Vec<u8>> {
        let candidates = [
            "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
            "/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf",
        ];
        for path in candidates {
            if let Ok(bytes) = std::fs::read(path) {
                return Some(bytes);
            }
        }
        // Fall back to scanning /usr/share/fonts for any .ttf.
        scan_for_ttf("/usr/share/fonts")
    }

    fn scan_for_ttf(dir: &str) -> Option<Vec<u8>> {
        let entries = std::fs::read_dir(dir).ok()?;
        let mut subdirs = Vec::new();
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                subdirs.push(path);
            } else if path.extension().and_then(|e| e.to_str()) == Some("ttf") {
                if let Ok(bytes) = std::fs::read(&path) {
                    return Some(bytes);
                }
            }
        }
        for sub in subdirs {
            if let Some(s) = sub.to_str() {
                if let Some(bytes) = scan_for_ttf(s) {
                    return Some(bytes);
                }
            }
        }
        None
    }

    #[test]
    fn builds_valid_type0_font() {
        let font_data = match find_test_ttf() {
            Some(d) => d,
            None => return, // skip when no font is available
        };

        let mut used = BTreeMap::new();
        used.insert(3u16, "A".to_string());
        used.insert(4u16, "B".to_string());

        let base = 10u32;
        let cid = build_type0_font(&font_data, &used, base, "F0");

        assert!(cid.obj_count >= 4, "expected at least 4 objects");
        assert_eq!(
            cid.font_obj_id, base,
            "Type0 must be the first allocated id"
        );

        // Concatenate all object bytes and assert the required markers are present.
        let mut all = Vec::new();
        for (_, bytes) in &cid.objects {
            all.extend_from_slice(bytes);
        }

        for needle in [
            &b"/Type0"[..],
            &b"/CIDFontType2"[..],
            &b"/FontFile2"[..],
            &b"/ToUnicode"[..],
            &b"/Identity-H"[..],
            &b"FlateDecode"[..],
        ] {
            assert!(
                all.windows(needle.len()).any(|w| w == needle),
                "missing marker {:?}",
                String::from_utf8_lossy(needle)
            );
        }
    }

    #[test]
    fn parse_failure_returns_empty() {
        let used = BTreeMap::new();
        let cid = build_type0_font(b"not a font", &used, 7, "F1");
        assert_eq!(cid.obj_count, 0);
        assert!(cid.objects.is_empty());
        assert_eq!(cid.font_obj_id, 7);
    }
}
