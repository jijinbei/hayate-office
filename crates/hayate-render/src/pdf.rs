//! Vector PDF export. Each slide becomes a PDF page whose content is built from the slide's
//! `Scene`: shapes are drawn as vector paths, text is shaped with cosmic-text and shown with
//! embedded CID fonts (selectable/extractable, resolution-independent), and pictures are embedded
//! as image XObjects. Streams are FlateDecode-compressed. Page size is the slide size in points,
//! so the document prints at its true physical size and stays crisp at any zoom.

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::io::Write as _;

use cosmic_text::{Attrs, Buffer, Family, FontSystem, Metrics, Shaping, Style, Weight};
use rayon::prelude::*;

use hayate_ir::presentation::Presentation;
use hayate_ir::text::HAlign;

use crate::build_slide_scene;
use crate::pdf_font::build_type0_font;
use crate::scene::{Paint, Primitive, PxRect, PxSize, Scene, StrokePx, TextBlock};

/// 1 PDF point = 1/72 inch = 12700 EMU.
const EMU_PER_POINT: f32 = 12_700.0;

/// Options for [`export_pdf`].
#[derive(Clone, Debug)]
pub struct PdfOptions {
    /// Document title (written to the `/Info` dictionary), if any.
    pub title: Option<String>,
    /// Bullet-list indent per level, in ems (match the editor's `list_indent_em`).
    pub list_indent_em: f32,
    /// Max DPI for embedded raster images; larger sources are downscaled to bound file size.
    pub image_dpi: f32,
}

impl Default for PdfOptions {
    fn default() -> Self {
        PdfOptions {
            title: None,
            list_indent_em: 0.5,
            image_dpi: 150.0,
        }
    }
}

thread_local! {
    static FONTS: RefCell<FontSystem> = RefCell::new(FontSystem::new());
}

/// zlib-compress `data` for a `/FlateDecode` stream.
fn flate(data: &[u8]) -> Vec<u8> {
    let mut e = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
    let _ = e.write_all(data);
    e.finish().unwrap_or_default()
}

/// One positioned glyph in a text run (scene/point coordinates, baseline origin).
struct Glyph {
    font_id: cosmic_text::fontdb::ID,
    glyph_id: u16,
    x: f32,
    baseline_y: f32,
    size: f32,
    color: (u8, u8, u8),
}

/// A drawing operation on a page, in scene/point coordinates (top-left origin, y down).
enum Op {
    Rect {
        b: PxRect,
        radius: f32,
        fill: Option<(u8, u8, u8)>,
        stroke: Option<((u8, u8, u8), f32)>,
        rot: f32,
    },
    Ellipse {
        b: PxRect,
        fill: Option<(u8, u8, u8)>,
        stroke: Option<((u8, u8, u8), f32)>,
        rot: f32,
    },
    Line {
        from: (f32, f32),
        to: (f32, f32),
        color: (u8, u8, u8),
        width: f32,
        start_arrow: bool,
        end_arrow: bool,
    },
    Image {
        b: PxRect,
        key: String,
        rot: f32,
    },
    Glyphs(Vec<Glyph>),
}

/// A page ready to serialize: its background and ordered draw ops.
struct Page {
    background: (u8, u8, u8),
    ops: Vec<Op>,
}

/// Render every slide of `p` to a vector PDF page.
pub fn export_pdf(p: &Presentation, opts: &PdfOptions) -> Vec<u8> {
    let slides = p.slides();
    let pt_w = (p.slide_size.w as f32 / EMU_PER_POINT).max(1.0);
    let pt_h = (p.slide_size.h as f32 / EMU_PER_POINT).max(1.0);

    // Pass 1: build each page's draw ops, collecting the glyphs each font needs and the images
    // used. Shaping happens once here.
    let mut pages: Vec<Page> = Vec::new();
    let mut font_glyphs: BTreeMap<cosmic_text::fontdb::ID, BTreeMap<u16, String>> = BTreeMap::new();
    let mut used_images: BTreeSet<String> = BTreeSet::new();
    for &slide in &slides {
        let scene = build_slide_scene(p, slide, PxSize { w: pt_w, h: pt_h });
        let page = build_page(&scene, opts, &mut font_glyphs);
        for op in &page.ops {
            if let Op::Image { key, .. } = op {
                used_images.insert(key.clone());
            }
        }
        pages.push(page);
    }

    // Build embedded fonts (one Type0 per font id actually used by some glyph).
    let mut pdf = PdfWriter::new();
    let catalog = pdf.alloc();
    let pages_id = pdf.alloc();
    let info_id = pdf.alloc();

    let mut font_res: BTreeMap<cosmic_text::fontdb::ID, (String, u32)> = BTreeMap::new();
    FONTS.with(|fs| {
        let mut fs = fs.borrow_mut();
        for (fi, (font_id, glyphs)) in font_glyphs.iter().enumerate() {
            if glyphs.is_empty() {
                continue;
            }
            let Some(font) = fs.get_font(*font_id, cosmic_text::fontdb::Weight::NORMAL) else {
                continue;
            };
            let base = pdf.peek();
            let subtag = format!("F{fi}");
            let built = build_type0_font(font.data(), glyphs, base, &subtag);
            if built.obj_count == 0 {
                continue;
            }
            pdf.reserve(built.obj_count);
            for (id, body) in built.objects {
                pdf.put(id, body);
            }
            font_res.insert(*font_id, (format!("F{fi}"), built.font_obj_id));
        }
    });

    // Build image XObjects (one per used media key).
    let mut image_res: BTreeMap<String, (String, u32)> = BTreeMap::new();
    // Decode + compress each image across cores; assign object ids sequentially afterwards so the
    // output (and the "Im{ii}" names, gaps included) stays byte-identical to the serial path.
    let image_keys: Vec<&String> = used_images.iter().collect();
    let embedded: Vec<Option<(usize, u32, u32, &'static str, std::borrow::Cow<[u8]>)>> = image_keys
        .par_iter()
        .enumerate()
        .map(|(ii, &key)| {
            let bytes = p.media.get(key)?;
            let (iw, ih, filter, data) = embed_image(bytes, opts.image_dpi, pt_w, pt_h)?;
            Some((ii, iw, ih, filter, data))
        })
        .collect();
    for (ii, iw, ih, filter, data) in embedded.into_iter().flatten() {
        let id = pdf.alloc();
        let mut body = format!(
            "<< /Type /XObject /Subtype /Image /Width {iw} /Height {ih} /ColorSpace /DeviceRGB \
             /BitsPerComponent 8 /Filter /{filter} /Length {} >>\nstream\n",
            data.len()
        )
        .into_bytes();
        body.extend_from_slice(&data);
        body.extend_from_slice(b"\nendstream");
        pdf.put(id, obj_bytes(id, &body));
        image_res.insert(image_keys[ii].clone(), (format!("Im{ii}"), id));
    }

    // Shared resources dictionary (fonts + images), referenced by every page.
    let mut res = String::from("<< ");
    if !font_res.is_empty() {
        res.push_str("/Font << ");
        for (name, id) in font_res.values() {
            let _ = write!(res, "/{name} {id} 0 R ");
        }
        res.push_str(">> ");
    }
    if !image_res.is_empty() {
        res.push_str("/XObject << ");
        for (name, id) in image_res.values() {
            let _ = write!(res, "/{name} {id} 0 R ");
        }
        res.push_str(">> ");
    }
    res.push_str(">>");

    // Pass 2: serialize + compress every page's content stream in parallel (the CPU-heavy part,
    // pure given the shared read-only resources), then allocate object ids sequentially so the
    // byte output matches the serial path exactly.
    let compressed: Vec<Vec<u8>> = pages
        .par_iter()
        .map(|page| flate(serialize_page(page, pt_w, pt_h, &font_res, &image_res).as_bytes()))
        .collect();
    let mut page_ids: Vec<u32> = Vec::new();
    for zipped in &compressed {
        let content_id = pdf.alloc();
        let mut cbody = format!(
            "<< /Length {} /Filter /FlateDecode >>\nstream\n",
            zipped.len()
        )
        .into_bytes();
        cbody.extend_from_slice(zipped);
        cbody.extend_from_slice(b"\nendstream");
        pdf.put(content_id, obj_bytes(content_id, &cbody));

        let page_id = pdf.alloc();
        page_ids.push(page_id);
        let page_dict = format!(
            "<< /Type /Page /Parent {pages_id} 0 R /MediaBox [0 0 {} {}] /Resources {res} \
             /Contents {content_id} 0 R >>",
            fmt(pt_w),
            fmt(pt_h)
        );
        pdf.put(page_id, obj_bytes(page_id, page_dict.as_bytes()));
    }

    // Catalog / Pages / Info.
    pdf.put(
        catalog,
        obj_bytes(
            catalog,
            format!("<< /Type /Catalog /Pages {pages_id} 0 R >>").as_bytes(),
        ),
    );
    let kids: String = page_ids.iter().map(|id| format!("{id} 0 R ")).collect();
    pdf.put(
        pages_id,
        obj_bytes(
            pages_id,
            format!(
                "<< /Type /Pages /Kids [ {kids}] /Count {} >>",
                page_ids.len()
            )
            .as_bytes(),
        ),
    );
    let title = opts.title.as_deref().unwrap_or("HayateOffice presentation");
    pdf.put(
        info_id,
        obj_bytes(
            info_id,
            format!(
                "<< /Title ({}) /Producer (HayateOffice) /Creator (HayateOffice) >>",
                pdf_text_escape(title)
            )
            .as_bytes(),
        ),
    );

    pdf.finish(catalog, info_id)
}

/// Build the ordered draw ops for a slide scene, collecting per-font used glyphs along the way.
fn build_page(
    scene: &Scene,
    opts: &PdfOptions,
    font_glyphs: &mut BTreeMap<cosmic_text::fontdb::ID, BTreeMap<u16, String>>,
) -> Page {
    let mut ops = Vec::new();
    for node in &scene.nodes {
        let rot = node.rotation_deg;
        match &node.prim {
            Primitive::Quad {
                bounds,
                corner_radius,
                fill,
                stroke,
            } => ops.push(Op::Rect {
                b: *bounds,
                radius: *corner_radius,
                fill: solid(fill),
                stroke: stroke.as_ref().map(strokepx),
                rot,
            }),
            Primitive::Ellipse {
                bounds,
                fill,
                stroke,
            } => ops.push(Op::Ellipse {
                b: *bounds,
                fill: solid(fill),
                stroke: stroke.as_ref().map(strokepx),
                rot,
            }),
            Primitive::Line {
                from,
                to,
                stroke,
                start_arrow,
                end_arrow,
            } => {
                if let Some((color, width)) = stroke.as_ref().map(strokepx) {
                    ops.push(Op::Line {
                        from: *from,
                        to: *to,
                        color,
                        width,
                        start_arrow: *start_arrow,
                        end_arrow: *end_arrow,
                    });
                }
            }
            Primitive::Image { bounds, media_key } => ops.push(Op::Image {
                b: *bounds,
                key: media_key.clone(),
                rot,
            }),
            Primitive::Text(tb) => {
                let glyphs = shape_block(tb, opts.list_indent_em, font_glyphs);
                if !glyphs.is_empty() {
                    ops.push(Op::Glyphs(glyphs));
                }
            }
            // Phase 2 will re-layout the Typst source into real PDF text + vectors; until then a
            // Typst box is omitted from the PDF (on-screen/raster previews show it).
            Primitive::Typst { .. } => {}
        }
    }
    Page {
        background: (scene.background.r, scene.background.g, scene.background.b),
        ops,
    }
}

/// Shape a text block with cosmic-text, returning positioned glyphs and recording the glyph ids
/// each font needs (with a representative unicode string for the ToUnicode map).
fn shape_block(
    tb: &TextBlock,
    indent_em: f32,
    font_glyphs: &mut BTreeMap<cosmic_text::fontdb::ID, BTreeMap<u16, String>>,
) -> Vec<Glyph> {
    let mut out = Vec::new();
    FONTS.with(|fs| {
        let mut fs = fs.borrow_mut();
        let bw = tb.bounds.w.max(1.0);
        let mut para_top = tb.bounds.y;
        for para in &tb.paragraphs {
            let size = para
                .runs
                .iter()
                .map(|r| r.size_px)
                .fold(0.0, f32::max)
                .max(1.0);
            let line_height = size * 1.3;
            let indent = size * (indent_em * para.bullet_level as f32);
            let metrics = Metrics::new(size, line_height);

            let bullet = match para.bullet_level {
                0 => "",
                1 => "\u{2022} ",
                2 => "\u{25E6} ",
                _ => "\u{25AA} ",
            };
            let mut spans: Vec<(&str, Attrs)> = Vec::new();
            if !bullet.is_empty() {
                let c = para
                    .runs
                    .first()
                    .map(|r| r.color)
                    .unwrap_or(hayate_ir::color::Rgba::rgb(0, 0, 0));
                spans.push((
                    bullet,
                    Attrs::new()
                        .metrics(metrics)
                        .color(cosmic_text::Color::rgb(c.r, c.g, c.b)),
                ));
            }
            for run in &para.runs {
                if run.text.is_empty() {
                    continue;
                }
                let mut a = Attrs::new()
                    .family(Family::Name(&run.family))
                    .metrics(metrics)
                    .color(cosmic_text::Color::rgb(
                        run.color.r,
                        run.color.g,
                        run.color.b,
                    ));
                if run.bold {
                    a = a.weight(Weight::BOLD);
                }
                if run.italic {
                    a = a.style(Style::Italic);
                }
                spans.push((run.text.as_str(), a));
            }
            if spans.is_empty() {
                para_top += line_height;
                continue;
            }
            let align = match para.align {
                HAlign::Center => Some(cosmic_text::Align::Center),
                HAlign::Right => Some(cosmic_text::Align::Right),
                HAlign::Left | HAlign::Justify => None,
            };
            let mut buf = Buffer::new_empty(metrics);
            buf.set_size(Some((bw - indent).max(1.0)), None);
            buf.set_rich_text(spans, &Attrs::new(), Shaping::Advanced, align);
            buf.shape_until_scroll(&mut fs, false);

            for run in buf.layout_runs() {
                for g in run.glyphs {
                    let color = g.color_opt.map(|c| {
                        let [r, gg, b, _] = c.as_rgba();
                        (r, gg, b)
                    });
                    font_glyphs
                        .entry(g.font_id)
                        .or_default()
                        .entry(g.glyph_id)
                        .or_insert_with(|| run.text.get(g.start..g.end).unwrap_or("").to_string());
                    out.push(Glyph {
                        font_id: g.font_id,
                        glyph_id: g.glyph_id,
                        x: tb.bounds.x + indent + g.x,
                        baseline_y: para_top + run.line_y,
                        size: g.font_size,
                        color: color.unwrap_or((0, 0, 0)),
                    });
                }
            }
            let rows = buf.layout_runs().count().max(1);
            para_top += line_height * rows as f32;
        }
    });
    out
}

/// Serialize a page's ops into a PDF content stream. Scene coords are top-left/y-down; PDF is
/// bottom-left/y-up, so y is flipped by `page_h`.
fn serialize_page(
    page: &Page,
    page_w: f32,
    page_h: f32,
    font_res: &BTreeMap<cosmic_text::fontdb::ID, (String, u32)>,
    image_res: &BTreeMap<String, (String, u32)>,
) -> String {
    let mut s = String::new();
    // Background.
    let (br, bg, bb) = page.background;
    let _ = writeln!(
        s,
        "{} {} {} rg\n0 0 {} {} re\nf",
        c(br),
        c(bg),
        c(bb),
        fmt(page_w),
        fmt(page_h)
    );

    for op in &page.ops {
        match op {
            Op::Rect {
                b,
                radius,
                fill,
                stroke,
                rot,
            } => {
                let pre = rot_wrap(*rot, b, page_h);
                s.push_str(&pre.0);
                rect_path(&mut s, b, *radius, page_h);
                paint_op(&mut s, fill, stroke);
                s.push_str(&pre.1);
            }
            Op::Ellipse {
                b,
                fill,
                stroke,
                rot,
            } => {
                let pre = rot_wrap(*rot, b, page_h);
                s.push_str(&pre.0);
                ellipse_path(&mut s, b, page_h);
                paint_op(&mut s, fill, stroke);
                s.push_str(&pre.1);
            }
            Op::Line {
                from,
                to,
                color,
                width,
                start_arrow,
                end_arrow,
            } => {
                let (r, g, bl) = *color;
                let _ = writeln!(
                    s,
                    "{} {} {} RG\n{} w\n{} {} m\n{} {} l\nS",
                    c(r),
                    c(g),
                    c(bl),
                    fmt(*width),
                    fmt(from.0),
                    fmt(page_h - from.1),
                    fmt(to.0),
                    fmt(page_h - to.1)
                );
                if *end_arrow {
                    arrowhead(&mut s, *from, *to, *width, *color, page_h);
                }
                if *start_arrow {
                    arrowhead(&mut s, *to, *from, *width, *color, page_h);
                }
            }
            Op::Image { b, key, rot } => {
                if let Some((name, _)) = image_res.get(key) {
                    let pre = rot_wrap(*rot, b, page_h);
                    s.push_str(&pre.0);
                    // Map the unit image square onto the image rect (PDF y-up).
                    let _ = writeln!(
                        s,
                        "q\n{} 0 0 {} {} {} cm\n/{name} Do\nQ",
                        fmt(b.w),
                        fmt(b.h),
                        fmt(b.x),
                        fmt(page_h - b.y - b.h),
                        name = name
                    );
                    s.push_str(&pre.1);
                }
            }
            Op::Glyphs(glyphs) => {
                for g in glyphs {
                    let Some((name, _)) = font_res.get(&g.font_id) else {
                        continue;
                    };
                    let (r, gg, b) = g.color;
                    let _ = writeln!(
                        s,
                        "BT\n{} {} {} rg\n/{name} {} Tf\n1 0 0 1 {} {} Tm\n<{:04X}> Tj\nET",
                        c(r),
                        c(gg),
                        c(b),
                        fmt(g.size),
                        fmt(g.x),
                        fmt(page_h - g.baseline_y),
                        g.glyph_id,
                        name = name
                    );
                }
            }
        }
    }
    s
}

/// `q`/cm prefix + `Q` suffix that rotates the node's content clockwise (screen sense) about its
/// center. Returns ("", "") when `deg` is ~0.
fn rot_wrap(deg: f32, b: &PxRect, page_h: f32) -> (String, String) {
    if deg.abs() < 1e-3 {
        return (String::new(), String::new());
    }
    let cx = b.x + b.w * 0.5;
    let cy = page_h - (b.y + b.h * 0.5);
    let phi = -deg.to_radians(); // screen-clockwise = negative angle in PDF (y-up)
    let (s, co) = (phi.sin(), phi.cos());
    let e = cx - cx * co + cy * s;
    let f = cy - cx * s - cy * co;
    (
        format!(
            "q\n{} {} {} {} {} {} cm\n",
            fmt(co),
            fmt(s),
            fmt(-s),
            fmt(co),
            fmt(e),
            fmt(f)
        ),
        "Q\n".to_string(),
    )
}

/// Append a rectangle (optionally rounded) path in PDF coords.
fn rect_path(s: &mut String, b: &PxRect, radius: f32, page_h: f32) {
    let x0 = b.x;
    let x1 = b.x + b.w;
    let y0 = page_h - (b.y + b.h); // bottom
    let y1 = page_h - b.y; // top
    let r = radius.min(b.w * 0.5).min(b.h * 0.5).max(0.0);
    if r <= 0.5 {
        let _ = writeln!(s, "{} {} {} {} re", fmt(x0), fmt(y0), fmt(b.w), fmt(b.h));
        return;
    }
    let k = r * 0.5523; // circle bezier constant
    let _ = writeln!(s, "{} {} m", fmt(x0 + r), fmt(y0));
    let _ = writeln!(s, "{} {} l", fmt(x1 - r), fmt(y0));
    let _ = writeln!(
        s,
        "{} {} {} {} {} {} c",
        fmt(x1 - r + k),
        fmt(y0),
        fmt(x1),
        fmt(y0 + r - k),
        fmt(x1),
        fmt(y0 + r)
    );
    let _ = writeln!(s, "{} {} l", fmt(x1), fmt(y1 - r));
    let _ = writeln!(
        s,
        "{} {} {} {} {} {} c",
        fmt(x1),
        fmt(y1 - r + k),
        fmt(x1 - r + k),
        fmt(y1),
        fmt(x1 - r),
        fmt(y1)
    );
    let _ = writeln!(s, "{} {} l", fmt(x0 + r), fmt(y1));
    let _ = writeln!(
        s,
        "{} {} {} {} {} {} c",
        fmt(x0 + r - k),
        fmt(y1),
        fmt(x0),
        fmt(y1 - r + k),
        fmt(x0),
        fmt(y1 - r)
    );
    let _ = writeln!(s, "{} {} l", fmt(x0), fmt(y0 + r));
    let _ = writeln!(
        s,
        "{} {} {} {} {} {} c",
        fmt(x0),
        fmt(y0 + r - k),
        fmt(x0 + r - k),
        fmt(y0),
        fmt(x0 + r),
        fmt(y0)
    );
    s.push_str("h\n");
}

/// Append an ellipse path (4 bezier arcs) inscribed in `b`, in PDF coords.
fn ellipse_path(s: &mut String, b: &PxRect, page_h: f32) {
    let rx = b.w * 0.5;
    let ry = b.h * 0.5;
    let cx = b.x + rx;
    let cy = page_h - (b.y + ry);
    let kx = rx * 0.5523;
    let ky = ry * 0.5523;
    let _ = writeln!(s, "{} {} m", fmt(cx + rx), fmt(cy));
    let _ = writeln!(
        s,
        "{} {} {} {} {} {} c",
        fmt(cx + rx),
        fmt(cy + ky),
        fmt(cx + kx),
        fmt(cy + ry),
        fmt(cx),
        fmt(cy + ry)
    );
    let _ = writeln!(
        s,
        "{} {} {} {} {} {} c",
        fmt(cx - kx),
        fmt(cy + ry),
        fmt(cx - rx),
        fmt(cy + ky),
        fmt(cx - rx),
        fmt(cy)
    );
    let _ = writeln!(
        s,
        "{} {} {} {} {} {} c",
        fmt(cx - rx),
        fmt(cy - ky),
        fmt(cx - kx),
        fmt(cy - ry),
        fmt(cx),
        fmt(cy - ry)
    );
    let _ = writeln!(
        s,
        "{} {} {} {} {} {} c",
        fmt(cx + kx),
        fmt(cy - ry),
        fmt(cx + rx),
        fmt(cy - ky),
        fmt(cx + rx),
        fmt(cy)
    );
    s.push_str("h\n");
}

/// Emit the fill/stroke painting operator for the current path.
fn paint_op(s: &mut String, fill: &Option<(u8, u8, u8)>, stroke: &Option<((u8, u8, u8), f32)>) {
    if let Some((r, g, b)) = fill {
        let _ = writeln!(s, "{} {} {} rg", c(*r), c(*g), c(*b));
    }
    if let Some(((r, g, b), w)) = stroke {
        let _ = writeln!(s, "{} {} {} RG\n{} w", c(*r), c(*g), c(*b), fmt(*w));
    }
    match (fill.is_some(), stroke.is_some()) {
        (true, true) => s.push_str("B\n"),
        (true, false) => s.push_str("f\n"),
        (false, true) => s.push_str("S\n"),
        (false, false) => s.push_str("n\n"),
    }
}

/// Draw a small filled arrowhead at `tip`, pointing away from `tail`.
fn arrowhead(
    s: &mut String,
    tail: (f32, f32),
    tip: (f32, f32),
    width: f32,
    color: (u8, u8, u8),
    page_h: f32,
) {
    let dx = tip.0 - tail.0;
    let dy = tip.1 - tail.1;
    let len = (dx * dx + dy * dy).sqrt().max(1e-3);
    let (ux, uy) = (dx / len, dy / len);
    let size = (width * 3.5).max(6.0);
    // Two barb points, rotated +/- ~25deg from the reverse direction.
    let ang = 0.45_f32;
    let (ca, sa) = (ang.cos(), ang.sin());
    let bx = -ux;
    let by = -uy;
    let p1 = (
        tip.0 + (bx * ca - by * sa) * size,
        tip.1 + (bx * sa + by * ca) * size,
    );
    let p2 = (
        tip.0 + (bx * ca + by * sa) * size,
        tip.1 + (-bx * sa + by * ca) * size,
    );
    let (r, g, b) = color;
    let _ = writeln!(
        s,
        "{} {} {} rg\n{} {} m\n{} {} l\n{} {} l\nf",
        c(r),
        c(g),
        c(b),
        fmt(tip.0),
        fmt(page_h - tip.1),
        fmt(p1.0),
        fmt(page_h - p1.1),
        fmt(p2.0),
        fmt(page_h - p2.1)
    );
}

/// Decode an embedded image to an image XObject payload: `(width, height, filter, data)`.
/// JPEG is passed through as `DCTDecode`; other formats are decoded to RGB and `FlateDecode`d.
/// Images larger than `image_dpi` over the page are downscaled to bound size.
fn embed_image<'a>(
    bytes: &'a [u8],
    image_dpi: f32,
    _pt_w: f32,
    _pt_h: f32,
) -> Option<(u32, u32, &'static str, std::borrow::Cow<'a, [u8]>)> {
    let is_jpeg = bytes.starts_with(&[0xFF, 0xD8, 0xFF]);
    let img = image::load_from_memory(bytes).ok()?;
    let (mut w, mut h) = (img.width(), img.height());
    if w == 0 || h == 0 {
        return None;
    }
    // Cap dimensions to keep files reasonable (image_dpi over a ~13in page).
    let max_dim = (image_dpi * 14.0).max(64.0) as u32;
    if is_jpeg {
        // Pass the original JPEG bytes through (no re-encode); DCTDecode handles it.
        return Some((w, h, "DCTDecode", std::borrow::Cow::Borrowed(bytes)));
    }
    let mut rgb = img.to_rgb8();
    if w > max_dim || h > max_dim {
        let scale = (max_dim as f32 / w as f32).min(max_dim as f32 / h as f32);
        let nw = ((w as f32 * scale) as u32).max(1);
        let nh = ((h as f32 * scale) as u32).max(1);
        rgb = image::imageops::resize(&rgb, nw, nh, image::imageops::FilterType::Triangle);
        w = nw;
        h = nh;
    }
    Some((
        w,
        h,
        "FlateDecode",
        std::borrow::Cow::Owned(flate(rgb.as_raw())),
    ))
}

fn solid(fill: &Option<Paint>) -> Option<(u8, u8, u8)> {
    match fill {
        Some(Paint::Solid(c)) => Some((c.r, c.g, c.b)),
        // A gradient is approximated by its start color (PDF shading is future work).
        Some(Paint::Linear { from, .. }) => Some((from.r, from.g, from.b)),
        None => None,
    }
}

fn strokepx(s: &StrokePx) -> ((u8, u8, u8), f32) {
    ((s.color.r, s.color.g, s.color.b), s.width.max(0.5))
}

/// Format a coordinate with up to 2 decimals, no trailing zeros.
fn fmt(v: f32) -> String {
    let s = format!("{v:.2}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s.is_empty() {
        "0".into()
    } else {
        s.into()
    }
}

/// Normalize an 8-bit channel to a 0..1 PDF color component.
fn c(v: u8) -> String {
    fmt(v as f32 / 255.0)
}

/// Escape a string for a PDF literal `( ... )`.
fn pdf_text_escape(s: &str) -> String {
    let mut out = String::new();
    for ch in s.chars() {
        match ch {
            '(' | ')' | '\\' => {
                out.push('\\');
                out.push(ch);
            }
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            _ => out.push(ch),
        }
    }
    out
}

/// Wrap a dictionary/stream `body` into a full PDF object `"N 0 obj\n...\nendobj\n"`.
fn obj_bytes(id: u32, body: &[u8]) -> Vec<u8> {
    let mut v = format!("{id} 0 obj\n").into_bytes();
    v.extend_from_slice(body);
    v.extend_from_slice(b"\nendobj\n");
    v
}

/// Incremental PDF object writer that tracks ids and emits a correct xref table.
struct PdfWriter {
    objects: Vec<(u32, Vec<u8>)>,
    next: u32,
}

impl PdfWriter {
    fn new() -> Self {
        PdfWriter {
            objects: Vec::new(),
            next: 1,
        }
    }
    /// Allocate one object id.
    fn alloc(&mut self) -> u32 {
        let id = self.next;
        self.next += 1;
        id
    }
    /// The next id that would be allocated (without consuming it).
    fn peek(&self) -> u32 {
        self.next
    }
    /// Reserve `n` ids (used after an external builder consumed a block starting at `peek`).
    fn reserve(&mut self, n: u32) {
        self.next += n;
    }
    /// Store a fully-serialized object (the complete `"N 0 obj ... endobj\n"` bytes).
    fn put(&mut self, id: u32, full: Vec<u8>) {
        self.objects.push((id, full));
    }
    /// Serialize the whole document.
    fn finish(mut self, catalog: u32, info: u32) -> Vec<u8> {
        self.objects.sort_by_key(|(id, _)| *id);
        let max_id = self.objects.iter().map(|(id, _)| *id).max().unwrap_or(0);
        let mut out: Vec<u8> = Vec::new();
        out.extend_from_slice(b"%PDF-1.7\n");
        out.extend_from_slice(&[b'%', 0xE2, 0xE3, 0xCF, 0xD3, b'\n']);
        let mut offsets = vec![0usize; (max_id + 1) as usize];
        for (id, full) in &self.objects {
            offsets[*id as usize] = out.len();
            out.extend_from_slice(full);
        }
        let xref_pos = out.len();
        out.extend_from_slice(format!("xref\n0 {}\n", max_id + 1).as_bytes());
        out.extend_from_slice(b"0000000000 65535 f \n");
        for id in 1..=max_id {
            out.extend_from_slice(format!("{:010} 00000 n \n", offsets[id as usize]).as_bytes());
        }
        out.extend_from_slice(
            format!(
                "trailer\n<< /Size {} /Root {catalog} 0 R /Info {info} 0 R >>\nstartxref\n{xref_pos}\n%%EOF\n",
                max_id + 1
            )
            .as_bytes(),
        );
        out
    }
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
        let pdf = export_pdf(&deck(3), &PdfOptions::default());
        assert!(pdf.starts_with(b"%PDF-"));
        assert!(count(&pdf, b"%%EOF") >= 1);
        assert_eq!(count(&pdf, b"/Type /Page /Parent"), 3);
        assert!(
            count(&pdf, b"/FlateDecode") >= 3,
            "content streams are compressed"
        );
        assert!(count(&pdf, b"/Info") >= 1, "has document metadata");
    }

    #[test]
    fn text_pdf_embeds_small_subset_font() {
        use hayate_ir::font::{FontRef, ThemeFontSlot};
        use hayate_ir::text::{Paragraph, Run, TextBody};
        use hayate_ir::units::pt;
        let mut p = Presentation::new();
        let m = p.add_master(Theme::default());
        let l = p.add_layout(m, "Blank");
        let s = p.add_slide(l);
        let t = p.add_shape(s);
        p.world.frames.insert(
            t,
            RectEmu::new(inch_f(0.5), inch_f(0.5), inch_f(9.0), inch_f(1.0)),
        );
        p.world.texts.insert(
            t,
            TextBody {
                paragraphs: vec![Paragraph::new(vec![Run {
                    text: "Hello 日本語".to_string(),
                    font: FontRef::Theme(ThemeFontSlot::Major),
                    size: pt(40),
                    color: Color::theme(ThemeColorToken::Dk1),
                    bold: true,
                    italic: false,
                    underline: false,
                }])],
                autofit: false,
                typst_source: None,
            },
        );
        let pdf = export_pdf(&p, &PdfOptions::default());
        // Text is a real embedded, selectable font — not a raster image.
        assert!(
            count(&pdf, b"/Type0") >= 1,
            "text uses an embedded Type0 font"
        );
        assert!(
            count(&pdf, b"/FontFile2") >= 1,
            "the font program is embedded"
        );
        assert!(
            count(&pdf, b"/ToUnicode") >= 1,
            "text is extractable (ToUnicode)"
        );
        // Subsetting keeps it small even though the source CJK face is ~20 MB.
        assert!(
            pdf.len() < 300_000,
            "subset keeps the file small: {} bytes",
            pdf.len()
        );
    }

    #[test]
    fn bold_is_reflected_in_output() {
        use hayate_ir::font::{FontRef, ThemeFontSlot};
        use hayate_ir::text::{Paragraph, Run, TextBody};
        use hayate_ir::units::pt;
        let deck = |bold: bool| {
            let mut p = Presentation::new();
            let m = p.add_master(Theme::default());
            let l = p.add_layout(m, "Blank");
            let s = p.add_slide(l);
            let t = p.add_shape(s);
            p.world.frames.insert(
                t,
                RectEmu::new(inch_f(0.5), inch_f(0.5), inch_f(9.0), inch_f(1.0)),
            );
            p.world.texts.insert(
                t,
                TextBody {
                    paragraphs: vec![Paragraph::new(vec![Run {
                        text: "Hello".to_string(),
                        font: FontRef::Theme(ThemeFontSlot::Major),
                        size: pt(40),
                        color: Color::theme(ThemeColorToken::Dk1),
                        bold,
                        italic: false,
                        underline: false,
                    }])],
                    autofit: false,
                    typst_source: None,
                },
            );
            p
        };
        // A bold run must produce different output than the same text in regular weight
        // (different embedded face / glyph metrics) — WYSIWYG on export.
        let regular = export_pdf(&deck(false), &PdfOptions::default());
        let bold = export_pdf(&deck(true), &PdfOptions::default());
        assert_ne!(
            regular, bold,
            "bold text should not export identically to regular"
        );
    }

    #[test]
    fn empty_deck_is_valid() {
        let pdf = export_pdf(&deck(0), &PdfOptions::default());
        assert!(pdf.starts_with(b"%PDF-"));
        assert!(count(&pdf, b"%%EOF") >= 1);
        assert!(count(&pdf, b"/Count 0") >= 1);
    }
}
