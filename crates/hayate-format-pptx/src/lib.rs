//! Minimal PPTX (OOXML) exporter and importer for a `hayate_ir::presentation::Presentation`.
//!
//! We hand-write the XML parts and pack them into a ZIP via the `zip` crate. The goal is a
//! minimal but valid `.pptx` that opens in PowerPoint / LibreOffice with each slide's shapes
//! (rect / round rect / ellipse) and text boxes shown at their correct positions, sizes,
//! rotation and resolved solid fill color.
//!
//! Theme references in fills and text colors are resolved to literal RGB through the
//! slide's master theme (`Presentation::theme_of` + `Theme::resolve_color`).
//!
//! Import ([`import_pptx`]) is intentionally low-fidelity: it recovers the slide size, slide
//! order and, for each autoshape (`<p:sp>`), its frame, preset geometry, solid fill,
//! rotation and text. Unknown elements are ignored.

use hayate_ir::color::Color;
use hayate_ir::geom::RectEmu;
use hayate_ir::paint::Fill;
use hayate_ir::presentation::Presentation;
use hayate_ir::shape::Geometry;
use hayate_ir::text::{HAlign, TextBody};
use hayate_ir::theme::Theme;
use hayate_ir::units::{Emu, EMU_PER_PT};
use hayate_ir::world::Entity;

use std::io::Write;

use zip::write::SimpleFileOptions;
use zip::ZipWriter;

/// Export `pres` to a `.pptx` file at `path`.
///
/// Emits the minimal OOXML part set: content types, package and presentation relationships,
/// one slide master, one slide layout, one theme, and one slide part per slide. Returns an
/// error if the file cannot be written.
pub fn export_pptx(
    pres: &Presentation,
    path: impl AsRef<std::path::Path>,
) -> Result<(), Box<dyn std::error::Error>> {
    let file = std::fs::File::create(path)?;
    let mut zip = ZipWriter::new(file);
    // Deflated is well supported and keeps the package small; stored would also be valid.
    let opts = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);

    let slides = pres.slides();

    // --- [Content_Types].xml ---
    let mut ct = String::new();
    ct.push_str(r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>"#);
    ct.push_str(r#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">"#);
    ct.push_str(r#"<Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>"#);
    ct.push_str(r#"<Default Extension="xml" ContentType="application/xml"/>"#);
    ct.push_str(r#"<Override PartName="/ppt/presentation.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.presentation.main+xml"/>"#);
    ct.push_str(r#"<Override PartName="/ppt/slideMasters/slideMaster1.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.slideMaster+xml"/>"#);
    ct.push_str(r#"<Override PartName="/ppt/slideLayouts/slideLayout1.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.slideLayout+xml"/>"#);
    ct.push_str(r#"<Override PartName="/ppt/theme/theme1.xml" ContentType="application/vnd.openxmlformats-officedocument.theme+xml"/>"#);
    for i in 1..=slides.len() {
        ct.push_str(&format!(
            r#"<Override PartName="/ppt/slides/slide{i}.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.slide+xml"/>"#
        ));
    }
    ct.push_str("</Types>");
    write_part(&mut zip, opts, "[Content_Types].xml", &ct)?;

    // --- _rels/.rels (package -> presentation) ---
    let rels = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="ppt/presentation.xml"/></Relationships>"#
    );
    write_part(&mut zip, opts, "_rels/.rels", &rels)?;

    // --- ppt/presentation.xml ---
    let sz = pres.slide_size;
    let mut pxml = String::new();
    pxml.push_str(r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>"#);
    pxml.push_str(r#"<p:presentation xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main">"#);
    // The slide master relationship is rId1 in presentation rels; slides start at rId2.
    pxml.push_str(r#"<p:sldMasterIdLst><p:sldMasterId id="2147483648" r:id="rId1"/></p:sldMasterIdLst>"#);
    pxml.push_str("<p:sldIdLst>");
    for (idx, _slide) in slides.iter().enumerate() {
        let rid = idx + 2; // rId2.. for slides
        let id = 256 + idx as u32; // slide ids must be >= 256
        pxml.push_str(&format!(r#"<p:sldId id="{id}" r:id="rId{rid}"/>"#));
    }
    pxml.push_str("</p:sldIdLst>");
    pxml.push_str(&format!(
        r#"<p:sldSz cx="{}" cy="{}"/>"#,
        sz.w, sz.h
    ));
    pxml.push_str(&format!(r#"<p:notesSz cx="{}" cy="{}"/>"#, sz.h, sz.w));
    pxml.push_str("</p:presentation>");
    write_part(&mut zip, opts, "ppt/presentation.xml", &pxml)?;

    // --- ppt/_rels/presentation.xml.rels ---
    let mut prels = String::new();
    prels.push_str(r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>"#);
    prels.push_str(r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">"#);
    prels.push_str(r#"<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slideMaster" Target="slideMasters/slideMaster1.xml"/>"#);
    for (idx, _slide) in slides.iter().enumerate() {
        let rid = idx + 2;
        let n = idx + 1;
        prels.push_str(&format!(
            r#"<Relationship Id="rId{rid}" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slide" Target="slides/slide{n}.xml"/>"#
        ));
    }
    // Theme relationship (highest id).
    let theme_rid = slides.len() + 2;
    prels.push_str(&format!(
        r#"<Relationship Id="rId{theme_rid}" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/theme" Target="theme/theme1.xml"/>"#
    ));
    prels.push_str("</Relationships>");
    write_part(&mut zip, opts, "ppt/_rels/presentation.xml.rels", &prels)?;

    // --- ppt/slideMasters/slideMaster1.xml (+ rels) ---
    write_part(
        &mut zip,
        opts,
        "ppt/slideMasters/slideMaster1.xml",
        &slide_master_xml(),
    )?;
    let master_rels = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slideLayout" Target="../slideLayouts/slideLayout1.xml"/><Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/theme" Target="../theme/theme1.xml"/></Relationships>"#;
    write_part(
        &mut zip,
        opts,
        "ppt/slideMasters/_rels/slideMaster1.xml.rels",
        master_rels,
    )?;

    // --- ppt/slideLayouts/slideLayout1.xml (+ rels) ---
    write_part(
        &mut zip,
        opts,
        "ppt/slideLayouts/slideLayout1.xml",
        &slide_layout_xml(),
    )?;
    let layout_rels = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slideMaster" Target="../slideMasters/slideMaster1.xml"/></Relationships>"#;
    write_part(
        &mut zip,
        opts,
        "ppt/slideLayouts/_rels/slideLayout1.xml.rels",
        layout_rels,
    )?;

    // --- ppt/theme/theme1.xml ---
    let theme = pres
        .slides()
        .first()
        .and_then(|s| pres.theme_of(*s).cloned())
        .or_else(|| {
            pres.default_master
                .and_then(|m| pres.world.master_info.get(&m).map(|mi| mi.theme.clone()))
        })
        .unwrap_or_default();
    write_part(&mut zip, opts, "ppt/theme/theme1.xml", &theme_xml(&theme))?;

    // --- ppt/slides/slideN.xml (+ rels) ---
    for (idx, slide) in slides.iter().enumerate() {
        let n = idx + 1;
        let slide_theme = pres.theme_of(*slide).cloned().unwrap_or_default();
        let xml = slide_xml(pres, *slide, &slide_theme);
        write_part(&mut zip, opts, &format!("ppt/slides/slide{n}.xml"), &xml)?;

        let srels = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slideLayout" Target="../slideLayouts/slideLayout1.xml"/></Relationships>"#;
        write_part(
            &mut zip,
            opts,
            &format!("ppt/slides/_rels/slide{n}.xml.rels"),
            srels,
        )?;
    }

    zip.finish()?;
    Ok(())
}

/// Import a `.pptx` file at `path` into a `Presentation`.
///
/// Reads `ppt/presentation.xml` for the slide size (`<p:sldSz>`) and slide order
/// (`<p:sldId>` -> relationship -> `ppt/slides/slideN.xml`), then parses each slide. A
/// single master (`Theme::default`) and layout are created; every imported slide hangs off
/// that layout. For each `<p:sp>` we recover frame, preset geometry, solid fill, rotation
/// and text. The parse is deliberately tolerant: unknown elements are ignored and missing
/// pieces are simply omitted.
pub fn import_pptx(
    path: impl AsRef<std::path::Path>,
) -> Result<Presentation, Box<dyn std::error::Error>> {
    use std::io::Read;

    let file = std::fs::File::open(path)?;
    let mut zip = zip::ZipArchive::new(file)?;

    // Read a named entry into a string, if it exists.
    let read_entry = |zip: &mut zip::ZipArchive<std::fs::File>, name: &str| -> Option<String> {
        let mut e = zip.by_name(name).ok()?;
        let mut buf = String::new();
        e.read_to_string(&mut buf).ok()?;
        Some(buf)
    };

    // --- presentation.xml: slide size + ordered slide relationship ids ---
    let pres_xml = read_entry(&mut zip, "ppt/presentation.xml")
        .ok_or("missing ppt/presentation.xml")?;
    let (slide_size, slide_rids) = parse_presentation(&pres_xml);

    // --- presentation.xml.rels: map relationship id -> slide part path ---
    let rels_xml = read_entry(&mut zip, "ppt/_rels/presentation.xml.rels").unwrap_or_default();
    let rid_to_target = parse_rels(&rels_xml);

    let mut pres = Presentation::new();
    if let Some(sz) = slide_size {
        pres.slide_size = sz;
    }
    let master = pres.add_master(Theme::default());
    let layout = pres.add_layout(master, "Imported");

    // Resolve each slide relationship id to its part path and parse it. Targets in the rels
    // are relative to the `ppt/` directory.
    for rid in &slide_rids {
        let target = match rid_to_target.get(rid) {
            Some(t) => t,
            None => continue,
        };
        let part = normalize_part_path(target);
        let slide_xml = match read_entry(&mut zip, &part) {
            Some(s) => s,
            None => continue,
        };
        let slide = pres.add_slide(layout);
        parse_slide_into(&slide_xml, &mut pres, slide);
    }

    Ok(pres)
}

/// Parse `ppt/presentation.xml` for the slide size and the ordered list of slide
/// relationship ids (`<p:sldId r:id="rIdN"/>`).
fn parse_presentation(xml: &str) -> (Option<hayate_ir::geom::SizeEmu>, Vec<String>) {
    use quick_xml::events::Event;
    use quick_xml::reader::Reader;

    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(false);

    let mut size: Option<hayate_ir::geom::SizeEmu> = None;
    let mut rids: Vec<String> = Vec::new();

    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) | Ok(Event::Empty(e)) => {
                let name = local_name(e.name().as_ref());
                match name.as_str() {
                    "sldSz" => {
                        let cx = attr_i64(&e, b"cx");
                        let cy = attr_i64(&e, b"cy");
                        if let (Some(cx), Some(cy)) = (cx, cy) {
                            size = Some(hayate_ir::geom::SizeEmu::new(cx, cy));
                        }
                    }
                    "sldId" => {
                        // The relationship id (r:id) points at the slide part; the numeric
                        // p:id is ignored.
                        if let Some(rid) = attr_str(&e, b"r:id") {
                            rids.push(rid);
                        }
                    }
                    _ => {}
                }
            }
            Ok(Event::Eof) => break,
            Err(_) => break,
            _ => {}
        }
    }

    (size, rids)
}

/// Parse a `*.rels` part into a map of relationship id -> target path.
fn parse_rels(xml: &str) -> std::collections::BTreeMap<String, String> {
    use quick_xml::events::Event;
    use quick_xml::reader::Reader;

    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(false);
    let mut map = std::collections::BTreeMap::new();

    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) | Ok(Event::Empty(e))
                if local_name(e.name().as_ref()) == "Relationship" =>
            {
                if let (Some(id), Some(target)) = (attr_str(&e, b"Id"), attr_str(&e, b"Target")) {
                    map.insert(id, target);
                }
            }
            Ok(Event::Eof) => break,
            Err(_) => break,
            _ => {}
        }
    }
    map
}

/// Normalize a relationship `Target` (relative to `ppt/`) into a full package part path.
fn normalize_part_path(target: &str) -> String {
    // Targets are like "slides/slide1.xml" or "../slides/slide1.xml". Resolve against the
    // "ppt/" base, collapsing any leading "../".
    let mut t = target.trim_start_matches('/').to_string();
    let mut base = vec!["ppt"];
    while let Some(rest) = t.strip_prefix("../") {
        base.pop();
        t = rest.to_string();
    }
    if base.is_empty() {
        t
    } else {
        format!("{}/{}", base.join("/"), t)
    }
}

/// Parse one `ppt/slides/slideN.xml` and add its autoshapes to `pres` under `slide`.
fn parse_slide_into(xml: &str, pres: &mut Presentation, slide: Entity) {
    use quick_xml::events::Event;
    use quick_xml::reader::Reader;

    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(false);

    // State accumulated for the shape currently being parsed (between <p:sp> and </p:sp>).
    struct ShapeState {
        off: Option<(Emu, Emu)>,
        ext: Option<(Emu, Emu)>,
        rotation: Option<f32>,
        geometry: Option<Geometry>,
        fill: Option<Color>,
        texts: Vec<String>,
    }
    impl ShapeState {
        fn new() -> Self {
            Self {
                off: None,
                ext: None,
                rotation: None,
                geometry: None,
                fill: None,
                texts: Vec::new(),
            }
        }
    }

    let mut state: Option<ShapeState> = None;
    // Whether we are inside a <a:t> element (so the next Text event is run text).
    let mut in_text = false;
    // Track whether we are inside a solidFill, so a nested srgbClr is taken as the fill.
    let mut solidfill_depth: i32 = 0;

    // Handle the attributes of an element start (shared by Start and Empty events). The
    // depth-tracking elements (`sp`, `solidFill`, `t`) are handled by the caller, since for
    // self-closing (`Empty`) elements they would open and close at once.
    fn apply_attrs(state: &mut Option<ShapeState>, solidfill_depth: i32, name: &str, e: &quick_xml::events::BytesStart) {
        let s = match state.as_mut() {
            Some(s) => s,
            None => return,
        };
        match name {
            "off" => {
                if let (Some(x), Some(y)) = (attr_i64(e, b"x"), attr_i64(e, b"y")) {
                    s.off = Some((x, y));
                }
            }
            "ext" => {
                if let (Some(cx), Some(cy)) = (attr_i64(e, b"cx"), attr_i64(e, b"cy")) {
                    s.ext = Some((cx, cy));
                }
            }
            "xfrm" => {
                if let Some(rot) = attr_i64(e, b"rot") {
                    // OOXML rot is in 60000ths of a degree.
                    s.rotation = Some(rot as f32 / 60_000.0);
                }
            }
            "prstGeom" => {
                if let Some(prst) = attr_str(e, b"prst") {
                    s.geometry = preset_to_geometry(&prst);
                }
            }
            "srgbClr" if solidfill_depth > 0 && s.fill.is_none() => {
                if let Some(rgba) = attr_str(e, b"val").as_deref().and_then(parse_hex_rgb) {
                    s.fill = Some(Color::Literal(rgba));
                }
            }
            _ => {}
        }
    }

    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) => {
                let name = local_name(e.name().as_ref());
                match name.as_str() {
                    "sp" => state = Some(ShapeState::new()),
                    "solidFill" => solidfill_depth += 1,
                    "t" => in_text = true,
                    other => apply_attrs(&mut state, solidfill_depth, other, &e),
                }
            }
            Ok(Event::Empty(e)) => {
                // Self-closing variants, e.g. <a:off .../> or <a:srgbClr .../>.
                let name = local_name(e.name().as_ref());
                apply_attrs(&mut state, solidfill_depth, &name, &e);
            }
            Ok(Event::Text(t)) if in_text => {
                if let (Some(s), Ok(txt)) = (state.as_mut(), t.unescape()) {
                    s.texts.push(txt.into_owned());
                }
            }
            Ok(Event::End(e)) => {
                let name = local_name(e.name().as_ref());
                match name.as_str() {
                    "t" => in_text = false,
                    "solidFill" => solidfill_depth = solidfill_depth.saturating_sub(1),
                    "sp" => {
                        if let Some(s) = state.take() {
                            commit_shape(pres, slide, s.off, s.ext, s.rotation, s.geometry, s.fill, &s.texts);
                        }
                    }
                    _ => {}
                }
            }
            Ok(Event::Eof) => break,
            Err(_) => break,
            _ => {}
        }
    }
}

/// Build a shape entity under `slide` from accumulated parse state.
#[allow(clippy::too_many_arguments)]
fn commit_shape(
    pres: &mut Presentation,
    slide: Entity,
    off: Option<(Emu, Emu)>,
    ext: Option<(Emu, Emu)>,
    rotation: Option<f32>,
    geometry: Option<Geometry>,
    fill: Option<Color>,
    texts: &[String],
) {
    let e = pres.add_shape(slide);

    if let (Some((x, y)), Some((cx, cy))) = (off, ext) {
        pres.world.frames.insert(e, RectEmu::new(x, y, cx, cy));
    }
    if let Some(r) = rotation {
        pres.world.rotations.insert(e, r);
    }
    if let Some(g) = geometry {
        pres.world.geometries.insert(e, g);
    }
    if let Some(c) = fill {
        pres.world.fills.insert(e, Fill::Solid(c));
    }
    if !texts.is_empty() {
        use hayate_ir::color::Rgba;
        use hayate_ir::font::{FontRef, ThemeFontSlot};
        use hayate_ir::text::{Paragraph, Run};
        use hayate_ir::units::pt;

        // One paragraph + one run per <a:t>, with default font/size/color.
        let paragraphs: Vec<Paragraph> = texts
            .iter()
            .map(|t| {
                Paragraph::new(vec![Run {
                    text: t.clone(),
                    font: FontRef::Theme(ThemeFontSlot::Minor),
                    size: pt(18),
                    color: Color::literal(Rgba::BLACK),
                    bold: false,
                    italic: false,
                    underline: false,
                }])
            })
            .collect();
        pres.world.texts.insert(
            e,
            TextBody {
                paragraphs,
                autofit: false,
            },
        );
    }
}

/// Map an OOXML `prst` preset name to a `Geometry` (only the shapes we export).
fn preset_to_geometry(prst: &str) -> Option<Geometry> {
    match prst {
        "rect" => Some(Geometry::Rect),
        "roundRect" => Some(Geometry::RoundRect { radius: 0 }),
        "ellipse" => Some(Geometry::Ellipse),
        _ => None,
    }
}

/// Parse a 6-hex-digit `RRGGBB` color string into an opaque `Rgba`.
fn parse_hex_rgb(s: &str) -> Option<hayate_ir::color::Rgba> {
    let s = s.trim();
    if s.len() != 6 {
        return None;
    }
    let r = u8::from_str_radix(&s[0..2], 16).ok()?;
    let g = u8::from_str_radix(&s[2..4], 16).ok()?;
    let b = u8::from_str_radix(&s[4..6], 16).ok()?;
    Some(hayate_ir::color::Rgba::rgb(r, g, b))
}

/// The local name of an XML element, dropping any namespace prefix (`p:sp` -> `sp`).
fn local_name(qname: &[u8]) -> String {
    let s = String::from_utf8_lossy(qname);
    match s.rsplit_once(':') {
        Some((_, local)) => local.to_string(),
        None => s.into_owned(),
    }
}

/// Read an attribute value (matched by its full, possibly prefixed, key) as a string.
fn attr_str(e: &quick_xml::events::BytesStart, key: &[u8]) -> Option<String> {
    for a in e.attributes().flatten() {
        if a.key.as_ref() == key {
            return a.unescape_value().ok().map(|v| v.into_owned());
        }
    }
    None
}

/// Read an attribute value as an `i64` (EMU / rotation units).
fn attr_i64(e: &quick_xml::events::BytesStart, key: &[u8]) -> Option<Emu> {
    attr_str(e, key)?.trim().parse::<i64>().ok()
}

/// Write one ZIP entry from an in-memory string.
fn write_part<W: Write + std::io::Seek>(
    zip: &mut ZipWriter<W>,
    opts: SimpleFileOptions,
    name: &str,
    content: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    zip.start_file(name, opts)?;
    zip.write_all(content.as_bytes())?;
    Ok(())
}

/// Build a slide part (`ppt/slides/slideN.xml`).
fn slide_xml(pres: &Presentation, slide: Entity, theme: &Theme) -> String {
    let mut s = String::new();
    s.push_str(r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>"#);
    s.push_str(r#"<p:sld xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main">"#);
    s.push_str("<p:cSld>");
    s.push_str("<p:spTree>");
    // Required group shape header (non-visual props + group props).
    s.push_str(r#"<p:nvGrpSpPr><p:cNvPr id="1" name=""/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr>"#);
    s.push_str(r#"<p:grpSpPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="0" cy="0"/><a:chOff x="0" y="0"/><a:chExt cx="0" cy="0"/></a:xfrm></p:grpSpPr>"#);

    // Shape ids start at 2 (1 is the group itself).
    let mut shape_id: u32 = 2;
    for child in pres.children(slide) {
        let frame = match pres.world.frames.get(&child) {
            Some(f) => *f,
            None => continue, // shapes without a frame have no position; skip
        };
        let text = pres.world.texts.get(&child);
        let geom = pres.world.geometries.get(&child);
        // Only emit shapes that are either vector geometry or text boxes.
        if text.is_none() && geom.is_none() {
            continue;
        }

        let rotation = pres.world.rotations.get(&child).copied().unwrap_or(0.0);
        let fill = pres.world.fills.get(&child);
        let name = pres
            .world
            .names
            .get(&child)
            .cloned()
            .unwrap_or_else(|| format!("Shape {shape_id}"));

        s.push_str("<p:sp>");
        // Non-visual shape properties.
        s.push_str("<p:nvSpPr>");
        s.push_str(&format!(
            r#"<p:cNvPr id="{shape_id}" name="{}"/>"#,
            escape_xml(&name)
        ));
        if text.is_some() {
            s.push_str(r#"<p:cNvSpPr txBox="1"/>"#);
        } else {
            s.push_str("<p:cNvSpPr/>");
        }
        s.push_str("<p:nvPr/>");
        s.push_str("</p:nvSpPr>");

        // Visual shape properties.
        s.push_str("<p:spPr>");
        s.push_str(&xfrm_xml(frame, rotation));
        // Preset geometry. Text-only boxes still benefit from a rect preset.
        let preset = match geom {
            Some(Geometry::Rect) => "rect",
            Some(Geometry::RoundRect { .. }) => "roundRect",
            Some(Geometry::Ellipse) => "ellipse",
            None => "rect",
        };
        s.push_str(&format!(
            r#"<a:prstGeom prst="{preset}"><a:avLst/></a:prstGeom>"#
        ));
        // Solid fill (resolved to literal RGB) when present.
        if let Some(Fill::Solid(color)) = fill {
            s.push_str(&solid_fill_xml(*color, theme));
        } else if text.is_none() {
            // A geometry with no fill: leave it unfilled (noFill) so it is not opaque black.
            s.push_str("<a:noFill/>");
        }
        s.push_str("</p:spPr>");

        // Text body.
        if let Some(tb) = text {
            s.push_str(&txbody_xml(tb, theme));
        } else {
            // Shapes need an (empty) txBody to be well-formed in some consumers.
            s.push_str(r#"<p:txBody><a:bodyPr/><a:lstStyle/><a:p/></p:txBody>"#);
        }

        s.push_str("</p:sp>");
        shape_id += 1;
    }

    s.push_str("</p:spTree>");
    s.push_str("</p:cSld>");
    s.push_str(r#"<p:clrMapOvr><a:overrideClrMapping/></p:clrMapOvr>"#);
    s.push_str("</p:sld>");
    s
}

/// `<a:xfrm>` with offset, extent and optional rotation (OOXML rot is 60000ths of a degree).
fn xfrm_xml(frame: RectEmu, rotation_deg: f32) -> String {
    let x = frame.origin.x;
    let y = frame.origin.y;
    let cx = frame.size.w.max(0);
    let cy = frame.size.h.max(0);
    let rot_attr = if rotation_deg != 0.0 {
        // Normalize into [0, 360) then convert to 60000ths of a degree.
        let mut d = rotation_deg % 360.0;
        if d < 0.0 {
            d += 360.0;
        }
        let units = (d * 60_000.0).round() as i64;
        format!(r#" rot="{units}""#)
    } else {
        String::new()
    };
    format!(
        r#"<a:xfrm{rot_attr}><a:off x="{x}" y="{y}"/><a:ext cx="{cx}" cy="{cy}"/></a:xfrm>"#
    )
}

/// `<a:solidFill><a:srgbClr val="RRGGBB"/></a:solidFill>`, color resolved via the theme.
fn solid_fill_xml(color: Color, theme: &Theme) -> String {
    let rgba = theme.resolve_color(&color);
    format!(
        r#"<a:solidFill><a:srgbClr val="{}"/></a:solidFill>"#,
        hex_rgb(rgba.r, rgba.g, rgba.b)
    )
}

/// Build `<p:txBody>` from a `TextBody`.
fn txbody_xml(tb: &TextBody, theme: &Theme) -> String {
    let mut s = String::new();
    s.push_str("<p:txBody>");
    s.push_str(r#"<a:bodyPr wrap="square" rtlCol="0"><a:normAutofit/></a:bodyPr>"#);
    s.push_str("<a:lstStyle/>");
    if tb.paragraphs.is_empty() {
        s.push_str("<a:p/>");
    }
    for para in &tb.paragraphs {
        s.push_str("<a:p>");
        // Paragraph properties: alignment.
        let algn = match para.align {
            HAlign::Left => "l",
            HAlign::Center => "ctr",
            HAlign::Right => "r",
            HAlign::Justify => "just",
        };
        s.push_str(&format!(r#"<a:pPr algn="{algn}"/>"#));
        for run in &para.runs {
            s.push_str("<a:r>");
            // Run properties: size (in hundredths of a point), bold/italic/underline, color.
            let sz = emu_to_hundredth_pt(run.size);
            let mut rpr = format!(r#"<a:rPr lang="en-US" sz="{sz}""#);
            if run.bold {
                rpr.push_str(r#" b="1""#);
            }
            if run.italic {
                rpr.push_str(r#" i="1""#);
            }
            if run.underline {
                rpr.push_str(r#" u="sng""#);
            }
            rpr.push('>');
            rpr.push_str(&solid_fill_xml(run.color, theme));
            rpr.push_str("</a:rPr>");
            s.push_str(&rpr);
            s.push_str(&format!("<a:t>{}</a:t>", escape_xml(&run.text)));
            s.push_str("</a:r>");
        }
        s.push_str("</a:p>");
    }
    s.push_str("</p:txBody>");
    s
}

/// EMU font size to OOXML hundredths of a point (`sz` attribute). Clamped to a sane minimum.
fn emu_to_hundredth_pt(size_emu: Emu) -> i64 {
    // 1 pt = EMU_PER_PT; sz is in 1/100 pt.
    let hundredths = (size_emu * 100) / EMU_PER_PT;
    hundredths.max(100)
}

fn hex_rgb(r: u8, g: u8, b: u8) -> String {
    format!("{r:02X}{g:02X}{b:02X}")
}

/// Escape XML text content / attribute values.
fn escape_xml(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            _ => out.push(ch),
        }
    }
    out
}

/// Minimal slide master. Carries a clrMap and an empty shape tree plus a layout id list.
fn slide_master_xml() -> String {
    let mut s = String::new();
    s.push_str(r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>"#);
    s.push_str(r#"<p:sldMaster xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main">"#);
    s.push_str("<p:cSld>");
    s.push_str("<p:spTree>");
    s.push_str(r#"<p:nvGrpSpPr><p:cNvPr id="1" name=""/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr>"#);
    s.push_str(r#"<p:grpSpPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="0" cy="0"/><a:chOff x="0" y="0"/><a:chExt cx="0" cy="0"/></a:xfrm></p:grpSpPr>"#);
    s.push_str("</p:spTree>");
    s.push_str("</p:cSld>");
    s.push_str(r#"<p:clrMap bg1="lt1" tx1="dk1" bg2="lt2" tx2="dk2" accent1="accent1" accent2="accent2" accent3="accent3" accent4="accent4" accent5="accent5" accent6="accent6" hlink="hlink" folHlink="folHlink"/>"#);
    s.push_str(r#"<p:sldLayoutIdLst><p:sldLayoutId id="2147483649" r:id="rId1"/></p:sldLayoutIdLst>"#);
    s.push_str("</p:sldMaster>");
    s
}

/// Minimal blank slide layout referencing the master.
fn slide_layout_xml() -> String {
    let mut s = String::new();
    s.push_str(r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>"#);
    s.push_str(r#"<p:sldLayout xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" type="blank" preserve="1">"#);
    s.push_str("<p:cSld name=\"Blank\">");
    s.push_str("<p:spTree>");
    s.push_str(r#"<p:nvGrpSpPr><p:cNvPr id="1" name=""/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr>"#);
    s.push_str(r#"<p:grpSpPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="0" cy="0"/><a:chOff x="0" y="0"/><a:chExt cx="0" cy="0"/></a:xfrm></p:grpSpPr>"#);
    s.push_str("</p:spTree>");
    s.push_str("</p:cSld>");
    s.push_str(r#"<p:clrMapOvr><a:overrideClrMapping/></p:clrMapOvr>"#);
    s.push_str("</p:sldLayout>");
    s
}

/// Minimal theme part. The color scheme is filled from the presentation theme so that any
/// downstream theme-color references resolve to sensible values; our shapes already use
/// literal `srgbClr`, so this mainly satisfies the schema and font scheme.
fn theme_xml(theme: &Theme) -> String {
    let c = &theme.colors;
    let acc = |i: usize| hex_rgb(c.accent[i].r, c.accent[i].g, c.accent[i].b);
    let major = &theme.fonts.major;
    let minor = &theme.fonts.minor;

    let mut s = String::new();
    s.push_str(r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>"#);
    s.push_str(r#"<a:theme xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" name="HayateTheme">"#);
    s.push_str("<a:themeElements>");

    // Color scheme. dk1/lt1 use sysClr-free srgbClr for portability.
    s.push_str(r#"<a:clrScheme name="HayateColors">"#);
    s.push_str(&format!(
        r#"<a:dk1><a:srgbClr val="{}"/></a:dk1>"#,
        hex_rgb(c.dk1.r, c.dk1.g, c.dk1.b)
    ));
    s.push_str(&format!(
        r#"<a:lt1><a:srgbClr val="{}"/></a:lt1>"#,
        hex_rgb(c.lt1.r, c.lt1.g, c.lt1.b)
    ));
    s.push_str(&format!(
        r#"<a:dk2><a:srgbClr val="{}"/></a:dk2>"#,
        hex_rgb(c.dk2.r, c.dk2.g, c.dk2.b)
    ));
    s.push_str(&format!(
        r#"<a:lt2><a:srgbClr val="{}"/></a:lt2>"#,
        hex_rgb(c.lt2.r, c.lt2.g, c.lt2.b)
    ));
    for i in 0..6 {
        s.push_str(&format!(
            r#"<a:accent{}><a:srgbClr val="{}"/></a:accent{}>"#,
            i + 1,
            acc(i),
            i + 1
        ));
    }
    s.push_str(&format!(
        r#"<a:hlink><a:srgbClr val="{}"/></a:hlink>"#,
        hex_rgb(c.hlink.r, c.hlink.g, c.hlink.b)
    ));
    s.push_str(&format!(
        r#"<a:folHlink><a:srgbClr val="{}"/></a:folHlink>"#,
        hex_rgb(c.fol_hlink.r, c.fol_hlink.g, c.fol_hlink.b)
    ));
    s.push_str("</a:clrScheme>");

    // Font scheme.
    s.push_str(r#"<a:fontScheme name="HayateFonts">"#);
    s.push_str("<a:majorFont>");
    s.push_str(&format!(
        r#"<a:latin typeface="{}"/><a:ea typeface="{}"/><a:cs typeface="{}"/>"#,
        escape_xml(&major.latin),
        escape_xml(&major.ea),
        escape_xml(&major.cs)
    ));
    s.push_str("</a:majorFont>");
    s.push_str("<a:minorFont>");
    s.push_str(&format!(
        r#"<a:latin typeface="{}"/><a:ea typeface="{}"/><a:cs typeface="{}"/>"#,
        escape_xml(&minor.latin),
        escape_xml(&minor.ea),
        escape_xml(&minor.cs)
    ));
    s.push_str("</a:minorFont>");
    s.push_str("</a:fontScheme>");

    // Minimal format scheme (fill/line/effect/bg styles) required by the schema.
    s.push_str(r#"<a:fmtScheme name="HayateFmt">"#);
    s.push_str(r#"<a:fillStyleLst><a:solidFill><a:schemeClr val="phClr"/></a:solidFill><a:solidFill><a:schemeClr val="phClr"/></a:solidFill><a:solidFill><a:schemeClr val="phClr"/></a:solidFill></a:fillStyleLst>"#);
    s.push_str(r#"<a:lnStyleLst><a:ln w="6350"><a:solidFill><a:schemeClr val="phClr"/></a:solidFill></a:ln><a:ln w="12700"><a:solidFill><a:schemeClr val="phClr"/></a:solidFill></a:ln><a:ln w="19050"><a:solidFill><a:schemeClr val="phClr"/></a:solidFill></a:ln></a:lnStyleLst>"#);
    s.push_str(r#"<a:effectStyleLst><a:effectStyle><a:effectLst/></a:effectStyle><a:effectStyle><a:effectLst/></a:effectStyle><a:effectStyle><a:effectLst/></a:effectStyle></a:effectStyleLst>"#);
    s.push_str(r#"<a:bgFillStyleLst><a:solidFill><a:schemeClr val="phClr"/></a:solidFill><a:solidFill><a:schemeClr val="phClr"/></a:solidFill><a:solidFill><a:schemeClr val="phClr"/></a:solidFill></a:bgFillStyleLst>"#);
    s.push_str("</a:fmtScheme>");

    s.push_str("</a:themeElements>");
    s.push_str("</a:theme>");
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use hayate_ir::color::{Color, Rgba, ThemeColorToken};
    use hayate_ir::font::{FontRef, ThemeFontSlot};
    use hayate_ir::geom::RectEmu;
    use hayate_ir::paint::Fill;
    use hayate_ir::shape::Geometry;
    use hayate_ir::text::{Paragraph, Run, TextBody};
    use hayate_ir::theme::Theme;
    use hayate_ir::units::pt;

    #[test]
    fn exports_valid_pptx_zip() {
        // Build a small deck: master/layout/slide + a filled rect + a text box.
        let mut p = Presentation::new();
        let master = p.add_master(Theme::default());
        let layout = p.add_layout(master, "Blank");
        let slide = p.add_slide(layout);

        let rect = p.add_shape(slide);
        p.world
            .frames
            .insert(rect, RectEmu::new(914_400, 457_200, 1_828_800, 914_400));
        p.world.geometries.insert(rect, Geometry::Rect);
        p.world.rotations.insert(rect, 30.0);
        p.world
            .fills
            .insert(rect, Fill::Solid(Color::theme(ThemeColorToken::Accent1)));

        let tb = p.add_shape(slide);
        p.world
            .frames
            .insert(tb, RectEmu::new(914_400, 1_828_800, 5_000_000, 914_400));
        p.world.texts.insert(
            tb,
            TextBody {
                paragraphs: vec![Paragraph::new(vec![Run {
                    text: "Hello <World> & \"PPTX\"".to_string(),
                    font: FontRef::Theme(ThemeFontSlot::Minor),
                    size: pt(24),
                    color: Color::literal(Rgba::BLACK),
                    bold: true,
                    italic: false,
                    underline: false,
                }])],
                autofit: false,
            },
        );

        // Unique temp path via process id.
        let path = std::env::temp_dir().join(format!("hayate_pptx_test_{}.pptx", std::process::id()));
        let _ = std::fs::remove_file(&path);

        export_pptx(&p, &path).expect("export should succeed");

        // The file must exist.
        assert!(path.exists(), "output file should exist");

        // Open it back as a zip and verify required entries.
        let f = std::fs::File::open(&path).expect("open output");
        let mut zip = zip::ZipArchive::new(f).expect("valid zip archive");

        let names: Vec<String> = (0..zip.len())
            .map(|i| zip.by_index(i).unwrap().name().to_string())
            .collect();

        assert!(
            names.iter().any(|n| n == "[Content_Types].xml"),
            "missing content types: {names:?}"
        );
        assert!(
            names.iter().any(|n| n == "ppt/presentation.xml"),
            "missing presentation.xml: {names:?}"
        );
        assert!(
            names.iter().any(|n| n == "ppt/slides/slide1.xml"),
            "missing slide1.xml: {names:?}"
        );

        // Sanity-check the slide content has our shapes and escaped text.
        {
            let mut slide_file = zip.by_name("ppt/slides/slide1.xml").expect("slide entry");
            let mut buf = String::new();
            use std::io::Read;
            slide_file.read_to_string(&mut buf).expect("read slide");
            assert!(buf.contains(r#"prst="rect""#), "rect preset present");
            assert!(buf.contains("<a:t>Hello &lt;World&gt; &amp; &quot;PPTX&quot;</a:t>"));
            assert!(buf.contains("<a:srgbClr"), "fill color present");
            assert!(buf.contains("rot="), "rotation present");
        }

        // Clean up.
        let _ = std::fs::remove_file(&path);
    }

    /// Process-unique temp path generator (pid + a per-call counter) so parallel tests in
    /// the same process never collide on a file name.
    fn unique_temp_path(tag: &str) -> std::path::PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!(
            "hayate_pptx_{tag}_{}_{n}.pptx",
            std::process::id()
        ))
    }

    #[test]
    fn export_then_import_roundtrips() {
        // Build a deck with two slides; the first carries a filled, rotated rect plus a
        // text box. We then export to a temp file and import it back.
        let mut p = Presentation::new();
        let master = p.add_master(Theme::default());
        let layout = p.add_layout(master, "Blank");
        let slide1 = p.add_slide(layout);
        let _slide2 = p.add_slide(layout);

        let rect = p.add_shape(slide1);
        let rect_frame = RectEmu::new(914_400, 457_200, 1_828_800, 914_400);
        p.world.frames.insert(rect, rect_frame);
        p.world.geometries.insert(rect, Geometry::Rect);
        p.world.rotations.insert(rect, 30.0);
        p.world.fills.insert(
            rect,
            Fill::Solid(Color::literal(Rgba::rgb(0x12, 0x34, 0x56))),
        );

        let tb = p.add_shape(slide1);
        p.world
            .frames
            .insert(tb, RectEmu::new(914_400, 1_828_800, 5_000_000, 914_400));
        p.world.texts.insert(
            tb,
            TextBody {
                paragraphs: vec![Paragraph::new(vec![Run {
                    text: "Round trip".to_string(),
                    font: FontRef::Theme(ThemeFontSlot::Minor),
                    size: pt(24),
                    color: Color::literal(Rgba::BLACK),
                    bold: false,
                    italic: false,
                    underline: false,
                }])],
                autofit: false,
            },
        );

        let path = unique_temp_path("roundtrip");
        let _ = std::fs::remove_file(&path);
        export_pptx(&p, &path).expect("export should succeed");

        let imported = import_pptx(&path).expect("import should succeed");

        // Slide count matches.
        assert_eq!(
            imported.slides().len(),
            p.slides().len(),
            "slide count should round-trip"
        );

        // The slide size round-trips.
        assert_eq!(imported.slide_size, p.slide_size, "slide size should round-trip");

        // At least one shape on the first imported slide has the rect's frame.
        let first = imported.slides()[0];
        let frames: Vec<RectEmu> = imported
            .children(first)
            .iter()
            .filter_map(|c| imported.world.frames.get(c).copied())
            .collect();
        assert!(
            frames.contains(&rect_frame),
            "expected a shape with frame {rect_frame:?}, got {frames:?}"
        );

        // The fill, geometry, rotation and text were recovered for some shape.
        assert!(
            imported
                .children(first)
                .iter()
                .any(|c| imported.world.fills.get(c)
                    == Some(&Fill::Solid(Color::literal(Rgba::rgb(0x12, 0x34, 0x56))))),
            "solid fill should round-trip"
        );
        assert!(
            imported
                .children(first)
                .iter()
                .any(|c| imported.world.geometries.get(c) == Some(&Geometry::Rect)),
            "rect geometry should round-trip"
        );
        assert!(
            imported
                .children(first)
                .iter()
                .any(|c| imported.world.rotations.get(c).copied() == Some(30.0)),
            "rotation should round-trip"
        );
        assert!(
            imported.children(first).iter().any(|c| imported
                .world
                .texts
                .get(c)
                .map(|tb| tb
                    .paragraphs
                    .iter()
                    .flat_map(|p| &p.runs)
                    .any(|r| r.text == "Round trip"))
                .unwrap_or(false)),
            "text should round-trip"
        );

        let _ = std::fs::remove_file(&path);
    }
}
