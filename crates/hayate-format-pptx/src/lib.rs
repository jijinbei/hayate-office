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

    // --- Plan embedded image parts ---
    // Walk every slide's pictures, assigning each unique media key a package image part
    // (`ppt/media/imageN.<ext>`). The same media key is shared across slides (one part).
    let mut media_plan: std::collections::BTreeMap<String, MediaPart> =
        std::collections::BTreeMap::new();
    let mut media_order: Vec<String> = Vec::new();
    for slide in &slides {
        for child in pres.children(*slide) {
            let pic = match pres.world.pictures.get(&child) {
                Some(p) => p,
                None => continue,
            };
            if media_plan.contains_key(&pic.media_key) {
                continue;
            }
            let bytes = match pres.get_media(&pic.media_key) {
                Some(b) => b,
                None => continue,
            };
            let ext = image_ext(bytes);
            let n = media_plan.len() + 1;
            media_plan.insert(
                pic.media_key.clone(),
                MediaPart {
                    part: format!("ppt/media/image{n}.{ext}"),
                    ext,
                },
            );
            media_order.push(pic.media_key.clone());
        }
    }
    // The distinct image extensions needing a `<Default>` content type entry.
    let mut image_exts: Vec<&'static str> = media_plan.values().map(|m| m.ext).collect();
    image_exts.sort_unstable();
    image_exts.dedup();

    // --- [Content_Types].xml ---
    let mut ct = String::new();
    ct.push_str(r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>"#);
    ct.push_str(r#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">"#);
    ct.push_str(r#"<Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>"#);
    ct.push_str(r#"<Default Extension="xml" ContentType="application/xml"/>"#);
    for ext in &image_exts {
        ct.push_str(&format!(
            r#"<Default Extension="{ext}" ContentType="{}"/>"#,
            image_content_type(ext)
        ));
    }
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

    // --- ppt/media/imageN.<ext> (embedded image bytes) ---
    for key in &media_order {
        if let (Some(part), Some(bytes)) = (media_plan.get(key), pres.get_media(key)) {
            write_binary_part(&mut zip, opts, &part.part, bytes)?;
        }
    }

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
    pxml.push_str(
        r#"<p:sldMasterIdLst><p:sldMasterId id="2147483648" r:id="rId1"/></p:sldMasterIdLst>"#,
    );
    pxml.push_str("<p:sldIdLst>");
    for (idx, _slide) in slides.iter().enumerate() {
        let rid = idx + 2; // rId2.. for slides
        let id = 256 + idx as u32; // slide ids must be >= 256
        pxml.push_str(&format!(r#"<p:sldId id="{id}" r:id="rId{rid}"/>"#));
    }
    pxml.push_str("</p:sldIdLst>");
    pxml.push_str(&format!(r#"<p:sldSz cx="{}" cy="{}"/>"#, sz.w, sz.h));
    pxml.push_str(&format!(r#"<p:notesSz cx="{}" cy="{}"/>"#, sz.h, sz.w));
    pxml.push_str("</p:presentation>");
    write_part(&mut zip, opts, "ppt/presentation.xml", &pxml)?;

    // --- ppt/_rels/presentation.xml.rels ---
    let mut prels = String::new();
    prels.push_str(r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>"#);
    prels.push_str(
        r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">"#,
    );
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
        // Image relationships for this slide, collected as the slide xml is built. Each entry is
        // (rel id, target path relative to the slide part).
        let mut image_rels: Vec<(String, String)> = Vec::new();
        let xml = slide_xml(pres, *slide, &slide_theme, &media_plan, &mut image_rels);
        write_part(&mut zip, opts, &format!("ppt/slides/slide{n}.xml"), &xml)?;

        let mut srels = String::new();
        srels.push_str(r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>"#);
        srels.push_str(r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">"#);
        srels.push_str(r#"<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slideLayout" Target="../slideLayouts/slideLayout1.xml"/>"#);
        for (rid, target) in &image_rels {
            srels.push_str(&format!(
                r#"<Relationship Id="{rid}" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/image" Target="{target}"/>"#
            ));
        }
        srels.push_str("</Relationships>");
        write_part(
            &mut zip,
            opts,
            &format!("ppt/slides/_rels/slide{n}.xml.rels"),
            &srels,
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

    // Read a named entry into raw bytes, if it exists (for embedded media).
    let read_bytes = |zip: &mut zip::ZipArchive<std::fs::File>, name: &str| -> Option<Vec<u8>> {
        let mut e = zip.by_name(name).ok()?;
        let mut buf = Vec::new();
        e.read_to_end(&mut buf).ok()?;
        Some(buf)
    };

    // --- presentation.xml: slide size + ordered slide relationship ids ---
    let pres_xml =
        read_entry(&mut zip, "ppt/presentation.xml").ok_or("missing ppt/presentation.xml")?;
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

        // Read the slide's own rels (e.g. ppt/slides/_rels/slide1.xml.rels) so picture blip
        // embeds can be resolved to media parts, and pre-read the referenced image bytes
        // (keyed by relationship id) before parsing the slide xml.
        let rels_part = slide_rels_part(&part);
        let slide_rels = read_entry(&mut zip, &rels_part)
            .map(|x| parse_rels(&x))
            .unwrap_or_default();
        let mut media_by_rid: std::collections::BTreeMap<String, Vec<u8>> =
            std::collections::BTreeMap::new();
        for (rel_id, rel_target) in &slide_rels {
            // Image targets are relative to the slide part's directory (e.g. ../media/x.png).
            let media_part = resolve_relative(&part, rel_target);
            if let Some(bytes) = read_bytes(&mut zip, &media_part) {
                media_by_rid.insert(rel_id.clone(), bytes);
            }
        }

        let slide = pres.add_slide(layout);
        parse_slide_into(&slide_xml, &mut pres, slide, &media_by_rid);
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

/// The `.rels` part path for a part, e.g. `ppt/slides/slide1.xml` ->
/// `ppt/slides/_rels/slide1.xml.rels`.
fn slide_rels_part(part: &str) -> String {
    match part.rsplit_once('/') {
        Some((dir, file)) => format!("{dir}/_rels/{file}.rels"),
        None => format!("_rels/{part}.rels"),
    }
}

/// Resolve a relationship `Target` against the directory of `from_part`, collapsing `../`
/// segments. E.g. `from_part = "ppt/slides/slide1.xml"`, `target = "../media/image1.png"` ->
/// `"ppt/media/image1.png"`.
fn resolve_relative(from_part: &str, target: &str) -> String {
    if let Some(abs) = target.strip_prefix('/') {
        return abs.to_string();
    }
    let mut base: Vec<&str> = match from_part.rsplit_once('/') {
        Some((dir, _)) => dir.split('/').collect(),
        None => Vec::new(),
    };
    let mut t = target;
    while let Some(rest) = t.strip_prefix("../") {
        base.pop();
        t = rest;
    }
    t = t.strip_prefix("./").unwrap_or(t);
    if base.is_empty() {
        t.to_string()
    } else {
        format!("{}/{}", base.join("/"), t)
    }
}

/// Parse one `ppt/slides/slideN.xml` and add its autoshapes to `pres` under `slide`.
///
/// `media_by_rid` maps a slide relationship id to the bytes of the embedded image part it
/// targets, so `<p:pic>` blip embeds can be reconstructed into picture components.
fn parse_slide_into(
    xml: &str,
    pres: &mut Presentation,
    slide: Entity,
    media_by_rid: &std::collections::BTreeMap<String, Vec<u8>>,
) {
    use quick_xml::events::Event;
    use quick_xml::reader::Reader;

    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(false);

    // State accumulated for the shape currently being parsed (between <p:sp> and </p:sp>).
    struct ShapeState {
        off: Option<(Emu, Emu)>,
        ext: Option<(Emu, Emu)>,
        rotation: Option<f32>,
        /// The `prst` preset name from `<a:prstGeom>`, resolved to a `Geometry` at commit time
        /// (so the round-rect `adj` and the frame are both available).
        preset: Option<String>,
        /// The `<a:gd name="adj" fmla="val N">` value, if present, for round-rect radius.
        adj: Option<i64>,
        fill: Option<Color>,
        /// Gradient stops/angle accumulated from a `<a:gradFill>` (first/last gs + lin ang).
        grad_from: Option<Color>,
        grad_to: Option<Color>,
        grad_ang: Option<f32>,
        paras: Vec<ParsedPara>,
        /// Run properties accumulated from the current `<a:rPr>`, applied to the next `<a:t>`.
        pending: ParsedRun,
    }
    impl ShapeState {
        fn new() -> Self {
            Self {
                off: None,
                ext: None,
                rotation: None,
                preset: None,
                adj: None,
                fill: None,
                grad_from: None,
                grad_to: None,
                grad_ang: None,
                paras: Vec::new(),
                pending: ParsedRun::default(),
            }
        }
    }

    // State accumulated for the picture currently being parsed (between <p:pic> and </p:pic>).
    #[derive(Default)]
    struct PicState {
        off: Option<(Emu, Emu)>,
        ext: Option<(Emu, Emu)>,
        rotation: Option<f32>,
        /// The relationship id from `<a:blip r:embed="rIdN"/>`.
        embed: Option<String>,
    }

    let mut state: Option<ShapeState> = None;
    let mut pic: Option<PicState> = None;
    // Whether we are inside a <a:t> element (so the next Text event is run text).
    let mut in_text = false;
    // Track whether we are inside a solidFill, so a nested srgbClr is taken as a color.
    let mut solidfill_depth: i32 = 0;
    // Whether we are inside a run's <a:rPr> (so a nested solidFill is the text color).
    let mut in_rpr = false;
    // Whether we are inside a <a:gradFill> (so nested gs/srgbClr are gradient stops).
    let mut in_gradfill = false;

    // Apply an element's geometry/blip attributes to the current picture state (if any).
    fn apply_pic_attrs(pic: &mut Option<PicState>, name: &str, e: &quick_xml::events::BytesStart) {
        let p = match pic.as_mut() {
            Some(p) => p,
            None => return,
        };
        match name {
            "off" => {
                if let (Some(x), Some(y)) = (attr_i64(e, b"x"), attr_i64(e, b"y")) {
                    p.off = Some((x, y));
                }
            }
            "ext" => {
                if let (Some(cx), Some(cy)) = (attr_i64(e, b"cx"), attr_i64(e, b"cy")) {
                    p.ext = Some((cx, cy));
                }
            }
            "xfrm" => {
                if let Some(rot) = attr_i64(e, b"rot") {
                    p.rotation = Some(rot as f32 / 60_000.0);
                }
            }
            "blip" => {
                if let Some(id) = attr_str(e, b"r:embed") {
                    p.embed = Some(id);
                }
            }
            _ => {}
        }
    }

    // Handle the attributes of an element start (shared by Start and Empty events). The
    // depth-tracking elements (`sp`, `solidFill`, `t`, `rPr`) are handled by the caller, since
    // for self-closing (`Empty`) elements they would open and close at once.
    fn apply_attrs(
        state: &mut Option<ShapeState>,
        solidfill_depth: i32,
        in_rpr: bool,
        in_gradfill: bool,
        name: &str,
        e: &quick_xml::events::BytesStart,
    ) {
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
                    s.preset = Some(prst);
                }
            }
            "gd" => {
                // Round-rect corner radius guide: <a:gd name="adj" fmla="val N"/>.
                if attr_str(e, b"name").as_deref() == Some("adj") {
                    if let Some(fmla) = attr_str(e, b"fmla") {
                        if let Some(val) = fmla.strip_prefix("val ") {
                            if let Ok(n) = val.trim().parse::<i64>() {
                                s.adj = Some(n);
                            }
                        }
                    }
                }
            }
            "pPr" => {
                if let Some(algn) = attr_str(e, b"algn") {
                    if let Some(p) = s.paras.last_mut() {
                        p.align = algn_to_halign(&algn);
                    }
                }
            }
            "rPr" => {
                // Font size: sz is in hundredths of a point.
                if let Some(sz) = attr_i64(e, b"sz") {
                    s.pending.size = Some(sz * EMU_PER_PT / 100);
                }
                s.pending.bold = attr_str(e, b"b").as_deref() == Some("1");
                s.pending.italic = attr_str(e, b"i").as_deref() == Some("1");
                s.pending.underline = attr_str(e, b"u").map(|u| u != "none").unwrap_or(false);
            }
            "lin" if in_gradfill => {
                // Gradient direction in 60000ths of a degree.
                if let Some(ang) = attr_i64(e, b"ang") {
                    s.grad_ang = Some(ang as f32 / 60_000.0);
                }
            }
            "srgbClr" if in_gradfill => {
                if let Some(rgba) = attr_str(e, b"val").as_deref().and_then(parse_hex_rgb) {
                    // First stop is `from`, any later stop updates `to` (last wins).
                    if s.grad_from.is_none() {
                        s.grad_from = Some(Color::Literal(rgba));
                    } else {
                        s.grad_to = Some(Color::Literal(rgba));
                    }
                }
            }
            "srgbClr" if solidfill_depth > 0 => {
                if let Some(rgba) = attr_str(e, b"val").as_deref().and_then(parse_hex_rgb) {
                    if in_rpr {
                        s.pending.color = Some(Color::Literal(rgba));
                    } else if s.fill.is_none() {
                        s.fill = Some(Color::Literal(rgba));
                    }
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
                    "pic" => pic = Some(PicState::default()),
                    "solidFill" => solidfill_depth += 1,
                    "gradFill" => in_gradfill = true,
                    "t" => in_text = true,
                    "rPr" => {
                        in_rpr = true;
                        apply_attrs(&mut state, solidfill_depth, in_rpr, in_gradfill, "rPr", &e);
                    }
                    "p" => {
                        if let Some(s) = state.as_mut() {
                            s.paras.push(ParsedPara {
                                align: hayate_ir::text::HAlign::Left,
                                runs: Vec::new(),
                            });
                        }
                    }
                    other => {
                        apply_attrs(&mut state, solidfill_depth, in_rpr, in_gradfill, other, &e);
                        apply_pic_attrs(&mut pic, other, &e);
                    }
                }
            }
            Ok(Event::Empty(e)) => {
                // Self-closing variants, e.g. <a:off .../>, <a:srgbClr .../>, <a:rPr .../>.
                let name = local_name(e.name().as_ref());
                apply_attrs(&mut state, solidfill_depth, in_rpr, in_gradfill, &name, &e);
                apply_pic_attrs(&mut pic, &name, &e);
            }
            Ok(Event::Text(t)) if in_text => {
                if let (Some(s), Ok(txt)) = (state.as_mut(), t.unescape()) {
                    // Commit the pending run (properties + this text) into the current paragraph.
                    let mut run = std::mem::take(&mut s.pending);
                    run.text = txt.into_owned();
                    if s.paras.is_empty() {
                        s.paras.push(ParsedPara {
                            align: hayate_ir::text::HAlign::Left,
                            runs: Vec::new(),
                        });
                    }
                    s.paras.last_mut().unwrap().runs.push(run);
                }
            }
            Ok(Event::End(e)) => {
                let name = local_name(e.name().as_ref());
                match name.as_str() {
                    "t" => in_text = false,
                    "rPr" => in_rpr = false,
                    "solidFill" => solidfill_depth = solidfill_depth.saturating_sub(1),
                    "gradFill" => in_gradfill = false,
                    "sp" => {
                        if let Some(s) = state.take() {
                            // Resolve geometry now that both the preset and frame are known.
                            let geometry = s
                                .preset
                                .as_deref()
                                .and_then(|prst| preset_to_geometry(prst, s.adj, s.off.zip(s.ext)));
                            // A gradient (if both stops were seen) takes precedence over a solid fill.
                            let fill = match (s.grad_from, s.grad_to) {
                                (Some(from), Some(to)) => Some(Fill::Linear {
                                    from,
                                    to,
                                    angle_deg: s.grad_ang.unwrap_or(0.0),
                                }),
                                _ => s.fill.map(Fill::Solid),
                            };
                            commit_shape(
                                pres, slide, s.off, s.ext, s.rotation, geometry, fill, s.paras,
                            );
                        }
                    }
                    "pic" => {
                        if let Some(p) = pic.take() {
                            commit_picture(
                                pres,
                                slide,
                                p.off,
                                p.ext,
                                p.rotation,
                                p.embed.as_deref(),
                                media_by_rid,
                            );
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

/// A run recovered from `<a:r>` (`<a:rPr>` properties plus its `<a:t>` text).
#[derive(Default)]
struct ParsedRun {
    text: String,
    /// Font size in EMU, if `<a:rPr sz>` was present.
    size: Option<Emu>,
    color: Option<Color>,
    bold: bool,
    italic: bool,
    underline: bool,
}

/// A paragraph recovered from `<a:p>`: its alignment and ordered runs.
struct ParsedPara {
    align: hayate_ir::text::HAlign,
    runs: Vec<ParsedRun>,
}

/// Build a picture entity under `slide` from accumulated `<p:pic>` parse state.
///
/// Resolves the blip embed (`embed`) to image bytes via `media_by_rid`, stores them in the
/// presentation media store, and attaches a `PictureRef` plus the picture's frame.
fn commit_picture(
    pres: &mut Presentation,
    slide: Entity,
    off: Option<(Emu, Emu)>,
    ext: Option<(Emu, Emu)>,
    rotation: Option<f32>,
    embed: Option<&str>,
    media_by_rid: &std::collections::BTreeMap<String, Vec<u8>>,
) {
    use hayate_ir::geom::SizeEmu;
    use hayate_ir::image::PictureRef;

    let bytes = match embed.and_then(|rid| media_by_rid.get(rid)) {
        Some(b) => b.clone(),
        None => return, // no resolvable image; skip
    };
    let key = pres.add_media(bytes);

    let e = pres.add_shape(slide);
    if let (Some((x, y)), Some((cx, cy))) = (off, ext) {
        pres.world.frames.insert(e, RectEmu::new(x, y, cx, cy));
    }
    if let Some(r) = rotation {
        pres.world.rotations.insert(e, r);
    }
    // The natural size is not recorded in the package; fall back to the frame extent.
    let natural = ext
        .map(|(cx, cy)| SizeEmu::new(cx, cy))
        .unwrap_or_else(|| SizeEmu::new(0, 0));
    pres.world.pictures.insert(
        e,
        PictureRef {
            media_key: key,
            natural,
        },
    );
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
    fill: Option<Fill>,
    paras: Vec<ParsedPara>,
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
    if let Some(f) = fill {
        pres.world.fills.insert(e, f);
    }

    let has_text = paras.iter().any(|p| !p.runs.is_empty());
    if has_text {
        use hayate_ir::color::Rgba;
        use hayate_ir::font::{FontRef, ThemeFontSlot};
        use hayate_ir::text::{Paragraph, Run};
        use hayate_ir::units::pt;

        // Recover each run's properties; unspecified ones fall back to sane defaults.
        let paragraphs: Vec<Paragraph> = paras
            .into_iter()
            .filter(|p| !p.runs.is_empty())
            .map(|p| {
                let runs = p
                    .runs
                    .into_iter()
                    .map(|r| Run {
                        text: r.text,
                        font: FontRef::Theme(ThemeFontSlot::Minor),
                        size: r.size.unwrap_or_else(|| pt(18)),
                        color: r.color.unwrap_or_else(|| Color::literal(Rgba::BLACK)),
                        bold: r.bold,
                        italic: r.italic,
                        underline: r.underline,
                    })
                    .collect();
                let mut para = Paragraph::new(runs);
                para.align = p.align;
                para
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

/// Map an OOXML paragraph alignment (`algn`) to our `HAlign` (defaults to Left).
fn algn_to_halign(algn: &str) -> hayate_ir::text::HAlign {
    use hayate_ir::text::HAlign;
    match algn {
        "ctr" => HAlign::Center,
        "r" => HAlign::Right,
        "just" => HAlign::Justify,
        _ => HAlign::Left,
    }
}

/// Map an OOXML `prst` preset name to a `Geometry` (only the shapes we export).
///
/// For `roundRect`, the corner radius is reconstructed from the `adj` guide (`adj`) relative to
/// the shape frame (`off_ext`), inverting the value emitted on export.
fn preset_to_geometry(
    prst: &str,
    adj: Option<i64>,
    off_ext: Option<((Emu, Emu), (Emu, Emu))>,
) -> Option<Geometry> {
    match prst {
        "rect" => Some(Geometry::Rect),
        "roundRect" => {
            let radius = round_rect_radius(adj.unwrap_or(0), off_ext);
            Some(Geometry::RoundRect { radius })
        }
        "ellipse" => Some(Geometry::Ellipse),
        // A plain line or straight connector imports back to a non-arrow line. Arrowhead
        // fidelity is not preserved on import (the head/tail markers are dropped).
        "line" | "straightConnector1" => Some(Geometry::Line { arrow: false }),
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

/// Write one ZIP entry from raw bytes (used for embedded media parts).
fn write_binary_part<W: Write + std::io::Seek>(
    zip: &mut ZipWriter<W>,
    opts: SimpleFileOptions,
    name: &str,
    content: &[u8],
) -> Result<(), Box<dyn std::error::Error>> {
    zip.start_file(name, opts)?;
    zip.write_all(content)?;
    Ok(())
}

/// A planned package image part for an embedded media key.
struct MediaPart {
    /// Full package part path, e.g. `ppt/media/image1.png`.
    part: String,
    /// File extension (`png` / `jpeg`), used for the content-type `<Default>` entry.
    ext: &'static str,
}

/// Guess an image file extension from the leading magic bytes. Defaults to `png`.
fn image_ext(bytes: &[u8]) -> &'static str {
    if bytes.starts_with(&[0x89, b'P', b'N', b'G']) {
        "png"
    } else if bytes.starts_with(&[0xFF, 0xD8]) {
        "jpeg"
    } else {
        "png"
    }
}

/// The OOXML content type for an image extension produced by [`image_ext`].
fn image_content_type(ext: &str) -> &'static str {
    match ext {
        "jpeg" | "jpg" => "image/jpeg",
        _ => "image/png",
    }
}

/// Build a slide part (`ppt/slides/slideN.xml`).
///
/// `media_plan` maps a picture's media key to its package image part; any pictures encountered
/// are emitted as `<p:pic>` and their slide-relative image relationships are pushed into
/// `image_rels` as `(rel id, target)` for the caller to write into the slide `.rels`.
fn slide_xml(
    pres: &Presentation,
    slide: Entity,
    theme: &Theme,
    media_plan: &std::collections::BTreeMap<String, MediaPart>,
    image_rels: &mut Vec<(String, String)>,
) -> String {
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
        let rotation = pres.world.rotations.get(&child).copied().unwrap_or(0.0);

        // Pictures are emitted as `<p:pic>` referencing an embedded image part via a slide rel.
        if let Some(pic) = pres.world.pictures.get(&child) {
            if let Some(mp) = media_plan.get(&pic.media_key) {
                let rid = format!("rId{}", image_rels.len() + 2); // rId1 is the layout
                                                                  // The slide-relative target: ppt/media/imageN.ext -> ../media/imageN.ext.
                let target = mp
                    .part
                    .strip_prefix("ppt/")
                    .map(|t| format!("../{t}"))
                    .unwrap_or_else(|| mp.part.clone());
                image_rels.push((rid.clone(), target));
                let name = pres
                    .world
                    .names
                    .get(&child)
                    .cloned()
                    .unwrap_or_else(|| format!("Picture {shape_id}"));
                s.push_str("<p:pic>");
                s.push_str("<p:nvPicPr>");
                s.push_str(&format!(
                    r#"<p:cNvPr id="{shape_id}" name="{}"/>"#,
                    escape_xml(&name)
                ));
                s.push_str(r#"<p:cNvPicPr/>"#);
                s.push_str("<p:nvPr/>");
                s.push_str("</p:nvPicPr>");
                s.push_str("<p:blipFill>");
                s.push_str(&format!(r#"<a:blip r:embed="{rid}"/>"#));
                s.push_str(r#"<a:stretch><a:fillRect/></a:stretch>"#);
                s.push_str("</p:blipFill>");
                s.push_str("<p:spPr>");
                s.push_str(&xfrm_xml(frame, rotation));
                s.push_str(r#"<a:prstGeom prst="rect"><a:avLst/></a:prstGeom>"#);
                s.push_str("</p:spPr>");
                s.push_str("</p:pic>");
                shape_id += 1;
                continue;
            }
        }

        let text = pres.world.texts.get(&child);
        let geom = pres.world.geometries.get(&child);
        // Only emit shapes that are either vector geometry or text boxes.
        if text.is_none() && geom.is_none() {
            continue;
        }

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
            // Export both plain lines and arrows as a straight connector preset; the kind
            // round-trips, while arrowhead markers are kept simple (not emitted here).
            Some(Geometry::Line { .. }) => "straightConnector1",
            None => "rect",
        };
        // For a round rect, emit the corner radius as the `adj` guide so it round-trips.
        // OOXML expresses `adj` in thousandths of the shape's smaller dimension.
        let av_lst = match geom {
            Some(Geometry::RoundRect { radius }) => {
                let adj = round_rect_adj(*radius, frame);
                format!(r#"<a:avLst><a:gd name="adj" fmla="val {adj}"/></a:avLst>"#)
            }
            _ => "<a:avLst/>".to_string(),
        };
        s.push_str(&format!(
            r#"<a:prstGeom prst="{preset}">{av_lst}</a:prstGeom>"#
        ));
        // Fill (resolved to literal RGB) when present: solid or two-stop linear gradient.
        match fill {
            Some(Fill::Solid(color)) => s.push_str(&solid_fill_xml(*color, theme)),
            Some(Fill::Linear {
                from,
                to,
                angle_deg,
            }) => s.push_str(&grad_fill_xml(*from, *to, *angle_deg, theme)),
            None if text.is_none() => {
                // A geometry with no fill: leave it unfilled (noFill) so it is not opaque black.
                s.push_str("<a:noFill/>");
            }
            None => {}
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

/// Compute the OOXML `roundRect` `adj` guide value from a corner radius and the shape frame.
///
/// `adj` is expressed in thousandths of the shape's smaller dimension; PowerPoint clamps it to
/// `[0, 50000]` (50000 = fully rounded). Returns 0 when the smaller dimension is non-positive.
fn round_rect_adj(radius: Emu, frame: RectEmu) -> i64 {
    let min_dim = frame.size.w.max(0).min(frame.size.h.max(0));
    if min_dim <= 0 {
        return 0;
    }
    let adj = radius.saturating_mul(100_000) / min_dim;
    adj.clamp(0, 50_000)
}

/// Reconstruct a corner radius (EMU) from a `roundRect` `adj` guide value and the shape frame,
/// inverting [`round_rect_adj`].
fn round_rect_radius(adj: i64, off_ext: Option<((Emu, Emu), (Emu, Emu))>) -> Emu {
    let ((_, _), (cx, cy)) = match off_ext {
        Some(v) => v,
        None => return 0,
    };
    let min_dim = cx.max(0).min(cy.max(0));
    if min_dim <= 0 {
        return 0;
    }
    adj.max(0).saturating_mul(min_dim) / 100_000
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
    format!(r#"<a:xfrm{rot_attr}><a:off x="{x}" y="{y}"/><a:ext cx="{cx}" cy="{cy}"/></a:xfrm>"#)
}

/// `<a:solidFill><a:srgbClr val="RRGGBB"/></a:solidFill>`, color resolved via the theme.
fn solid_fill_xml(color: Color, theme: &Theme) -> String {
    let rgba = theme.resolve_color(&color);
    format!(
        r#"<a:solidFill><a:srgbClr val="{}"/></a:solidFill>"#,
        hex_rgb(rgba.r, rgba.g, rgba.b)
    )
}

/// A two-stop linear `gradFill`. `angle_deg` is converted to OOXML's 60000ths of a degree,
/// normalized to 0..360.
fn grad_fill_xml(from: Color, to: Color, angle_deg: f32, theme: &Theme) -> String {
    let a = theme.resolve_color(&from);
    let b = theme.resolve_color(&to);
    let raw = (angle_deg as f64 * 60_000.0).round() as i64;
    let ang = ((raw % 21_600_000) + 21_600_000) % 21_600_000;
    format!(
        r#"<a:gradFill><a:gsLst><a:gs pos="0"><a:srgbClr val="{}"/></a:gs><a:gs pos="100000"><a:srgbClr val="{}"/></a:gs></a:gsLst><a:lin ang="{}" scaled="1"/></a:gradFill>"#,
        hex_rgb(a.r, a.g, a.b),
        hex_rgb(b.r, b.g, b.b),
        ang
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
    s.push_str(
        r#"<p:sldLayoutIdLst><p:sldLayoutId id="2147483649" r:id="rId1"/></p:sldLayoutIdLst>"#,
    );
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
mod tests;
