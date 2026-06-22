//! Typst typesetting for rich text boxes.
//!
//! A text box's canonical content is Typst markup. For on-screen preview / thumbnails / the
//! software rasterizer we compile that markup with the `typst` crate and rasterize it to an RGBA
//! image ([`render_typst_raster`]); the PDF exporter instead walks the laid-out frame to emit real
//! selectable text + vectors (Phase 2, [`layout_typst`]).
//!
//! The Typst engine (font book + standard library) is built once per thread and reused; results
//! are memoized by the exact compiled source + scale so a rebuild/zoom doesn't recompile.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

use typst::diag::{FileError, FileResult};
use typst::foundations::{Bytes, Datetime};
use typst::layout::PagedDocument;
use typst::syntax::{FileId, Source, VirtualPath};
use typst::text::{Font, FontBook};
use typst::utils::LazyHash;
use typst::{Library, LibraryExt, World};

use hayate_ir::color::Rgba;
use hayate_ir::text::HAlign;

/// A rasterized Typst box: premultiplied RGBA (row-major, top-left origin) plus pixel dimensions.
#[derive(Debug)]
pub struct RasterImage {
    pub rgba: Arc<Vec<u8>>,
    pub px_w: u32,
    pub px_h: u32,
}

/// Shared, immutable engine state (built once per thread): the standard library, the font book,
/// and the index-aligned fonts. Cheap to share into each per-render `TypstWorld` via `Arc`.
struct Engine {
    library: Arc<LazyHash<Library>>,
    book: Arc<LazyHash<FontBook>>,
    fonts: Arc<Vec<Font>>,
}

/// A minimal in-memory `World`: a single main source plus the shared engine state.
struct TypstWorld {
    library: Arc<LazyHash<Library>>,
    book: Arc<LazyHash<FontBook>>,
    fonts: Arc<Vec<Font>>,
    main: FileId,
    source: Source,
}

impl World for TypstWorld {
    fn library(&self) -> &LazyHash<Library> {
        &self.library
    }
    fn book(&self) -> &LazyHash<FontBook> {
        &self.book
    }
    fn main(&self) -> FileId {
        self.main
    }
    fn source(&self, id: FileId) -> FileResult<Source> {
        if id == self.main {
            Ok(self.source.clone())
        } else {
            Err(FileError::NotFound(
                id.vpath().as_rootless_path().to_path_buf(),
            ))
        }
    }
    fn file(&self, id: FileId) -> FileResult<Bytes> {
        Err(FileError::NotFound(
            id.vpath().as_rootless_path().to_path_buf(),
        ))
    }
    fn font(&self, index: usize) -> Option<Font> {
        self.fonts.get(index).cloned()
    }
    fn today(&self, _offset: Option<i64>) -> Option<Datetime> {
        None
    }
}

/// Load every face out of a font blob (a `.ttc` holds several), pushing `Font`s into `fonts`.
fn load_faces(data: Bytes, fonts: &mut Vec<Font>) {
    let mut index = 0u32;
    while let Some(font) = Font::new(data.clone(), index) {
        fonts.push(font);
        index += 1;
    }
}

/// Common system paths for Noto Sans CJK JP, so Japanese renders in Typst boxes. Mirrors the
/// font discovery used by the PDF CID embedder. Regular and Bold are separate files, so we load
/// one of each weight — otherwise `weight: "bold"` (e.g. a master Title) has no CJK bold face and
/// silently falls back to regular.
const CJK_REGULAR_PATHS: &[&str] = &[
    "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
    "/run/current-system/sw/share/X11/fonts/NotoSansCJK-Regular.ttc",
    "/usr/share/fonts/truetype/noto/NotoSansCJK-Regular.ttc",
];
const CJK_BOLD_PATHS: &[&str] = &[
    "/usr/share/fonts/opentype/noto/NotoSansCJK-Bold.ttc",
    "/run/current-system/sw/share/X11/fonts/NotoSansCJK-Bold.ttc",
    "/usr/share/fonts/truetype/noto/NotoSansCJK-Bold.ttc",
];

fn build_engine() -> Engine {
    let mut fonts: Vec<Font> = Vec::new();
    // Typst's bundled fonts (incl. New Computer Modern + its math font, Libertinus, DejaVu Mono).
    for data in typst_assets::fonts() {
        load_faces(Bytes::new(data), &mut fonts);
    }
    // System CJK so Japanese text/labels render — load a regular AND a bold face so bold weights
    // (master Title etc.) actually render bold rather than falling back to regular.
    for paths in [CJK_REGULAR_PATHS, CJK_BOLD_PATHS] {
        for path in paths {
            if let Ok(bytes) = std::fs::read(path) {
                load_faces(Bytes::new(bytes), &mut fonts);
                break;
            }
        }
    }
    let book = FontBook::from_infos(fonts.iter().map(|f| f.info().clone()));
    Engine {
        library: Arc::new(LazyHash::new(Library::default())),
        book: Arc::new(LazyHash::new(book)),
        fonts: Arc::new(fonts),
    }
}

thread_local! {
    static ENGINE: Engine = build_engine();
    static RASTER_CACHE: RefCell<HashMap<String, Rc<Result<RasterImage, String>>>> =
        RefCell::new(HashMap::new());
    static LAYOUT_CACHE: RefCell<HashMap<String, Rc<Result<TypstLayout, String>>>> =
        RefCell::new(HashMap::new());
}

/// Max cached raster results per thread (bounds memory across zoom levels).
const CACHE_CAP: usize = 256;

/// Whether a line begins a Typst block construct that already breaks on its own (list item,
/// heading, ordered item), trimmed of leading whitespace. Such lines are left to Typst — we only
/// hard-break between two plain *prose* lines.
fn is_block_line(line: &str) -> bool {
    let t = line.trim_start();
    t.starts_with("- ")
        || t.starts_with("+ ")
        || t.starts_with("/ ")
        || t.starts_with('=')
        || t.starts_with('#')
        // ordered list: digits then '.'
        || {
            let digits: String = t.chars().take_while(|c| c.is_ascii_digit()).collect();
            !digits.is_empty() && t[digits.len()..].starts_with('.')
        }
}

/// Make Enter behave like a line break in a slide text box: a single newline between two plain
/// prose lines becomes a Typst forced line break (`\`). Blank lines (paragraph breaks), lines
/// adjacent to block constructs (lists/headings), and newlines inside a `$…$` math span are left
/// untouched so Typst markup keeps working. Pure transform applied only at render time.
fn prose_linebreaks(src: &str) -> String {
    let lines: Vec<&str> = src.split('\n').collect();
    let mut out = String::with_capacity(src.len() + 8);
    let mut in_math = false;
    for (i, line) in lines.iter().enumerate() {
        out.push_str(line);
        // Track `$` toggles across this line to know if the following newline sits inside math.
        in_math ^= line.matches('$').count() % 2 == 1;
        if i + 1 == lines.len() {
            break; // last line: no trailing newline to consider
        }
        let next = lines[i + 1];
        let both_prose = !line.trim().is_empty()
            && !next.trim().is_empty()
            && !is_block_line(line)
            && !is_block_line(next);
        let already_break = line.trim_end().ends_with('\\');
        if both_prose && !in_math && !already_break {
            out.push_str(" \\\n"); // Typst forced line break
        } else {
            out.push('\n');
        }
    }
    out
}

/// Build the full Typst document (preamble + user source) for a box of width `box_w_pt`.
/// `bold` sets the base font weight so a placeholder/run styled bold (e.g. a master Title) renders
/// bold even when the source carries no `*…*` markup.
fn wrap_source(
    source: &str,
    box_w_pt: f32,
    default_pt: f32,
    color: Rgba,
    align: HAlign,
    bold: bool,
    italic: bool,
) -> String {
    let al = match align {
        HAlign::Left | HAlign::Justify => "left",
        HAlign::Center => "center",
        HAlign::Right => "right",
    };
    let justify = matches!(align, HAlign::Justify);
    let weight = if bold { ", weight: \"bold\"" } else { "" };
    // Faux-oblique: Noto Sans CJK (and many fonts) have no italic face and Typst won't synthesize
    // one, so emphasis (`_…_`) would stay upright. Slant the emphasized content with `skew` so
    // italic renders for any script. Pivot at the baseline (`origin: bottom + left`) so the slanted
    // text sits on the same baseline as the surrounding upright text (a center pivot raises it).
    let emph_rule = "#show emph: it => box(skew(ax: -12deg, origin: bottom + left, it.body))\n";
    let body = prose_linebreaks(source);
    let body = if italic {
        // Run-level italic slants the whole body the same way (best for short, single-line slots).
        format!("#box(skew(ax: -12deg, origin: bottom + left, reflow: true)[\n{body}\n])")
    } else {
        body
    };
    format!(
        "#set page(width: {w}pt, height: auto, margin: 0pt, fill: none)\n\
         #set text(size: {sz}pt, fill: rgb(\"#{r:02x}{g:02x}{b:02x}\"){weight}, \
         font: (\"Noto Sans CJK JP\", \"New Computer Modern\"), \
         top-edge: \"ascender\", bottom-edge: \"descender\")\n\
         #set par(justify: {justify})\n\
         #set align({al})\n\
         {emph_rule}{body}",
        w = box_w_pt.max(1.0),
        sz = default_pt.max(1.0),
        r = color.r,
        g = color.g,
        b = color.b,
        weight = weight,
        justify = justify,
        al = al,
        emph_rule = emph_rule,
        body = body,
    )
}

/// Convert premultiplied RGBA (typst's raster output) to the premultiplied BGRA that gpui's
/// `RenderImage` wants, swapping the R and B channels. Defined here — in a dependency that the
/// dev profile optimizes (`profile.dev.package."*"`) — so the per-pixel loop stays fast even in a
/// debug build of the app crate, which is itself unoptimized. (At 4K this loop is ~100ms unopt vs
/// a few ms optimized, and it runs on every slideshow transition.)
pub fn rgba_to_bgra(rgba: &[u8]) -> Vec<u8> {
    let mut bgra = rgba.to_vec();
    for px in bgra.chunks_exact_mut(4) {
        px.swap(0, 2);
    }
    bgra
}

/// Compile `full_src` and rasterize page 0 at `ppp` pixels-per-point.
fn compile_raster(full_src: &str, ppp: f32) -> Result<RasterImage, String> {
    ENGINE.with(|engine| {
        let main = FileId::new_fake(VirtualPath::new("main.typ"));
        let world = TypstWorld {
            library: Arc::clone(&engine.library),
            book: Arc::clone(&engine.book),
            fonts: Arc::clone(&engine.fonts),
            main,
            source: Source::new(main, full_src.to_string()),
        };
        let doc = typst::compile::<PagedDocument>(&world)
            .output
            .map_err(|diags| {
                diags
                    .first()
                    .map(|d| d.message.to_string())
                    .unwrap_or_else(|| "typst compile error".to_string())
            })?;
        let page = doc
            .pages
            .first()
            .ok_or_else(|| "empty document".to_string())?;
        let pixmap = typst_render::render(page, ppp.max(0.1));
        let (px_w, px_h) = (pixmap.width(), pixmap.height());
        Ok(RasterImage {
            rgba: Arc::new(pixmap.take()),
            px_w,
            px_h,
        })
    })
}

/// Render a Typst box to a premultiplied-RGBA raster, memoized per (compiled source, scale).
///
/// `box_w_px` is the box width in device pixels and `ppp` is pixels-per-point (viewport scale ×
/// 12700/EMU-per-pt); the Typst page width is `box_w_px / ppp` points so the rasterized width
/// matches the on-screen box. Returns `Err(diagnostic)` on a Typst compile error so the caller can
/// fall back to plain text.
pub fn render_typst_raster(
    source: &str,
    box_w_px: f32,
    ppp: f32,
    default_pt: f32,
    color: Rgba,
    align: HAlign,
    bold: bool,
    italic: bool,
) -> Rc<Result<RasterImage, String>> {
    let box_w_pt = (box_w_px / ppp.max(0.1)).max(1.0);
    let full = wrap_source(source, box_w_pt, default_pt, color, align, bold, italic);
    // Key on the exact compiled source + quantized scale: any change to text/size/color/align/width
    // changes `full`, so the cache self-invalidates.
    let key = format!("{}\u{0}{}", (ppp * 100.0).round() as i64, full);
    RASTER_CACHE.with(|cache| {
        if let Some(hit) = cache.borrow().get(&key) {
            return Rc::clone(hit);
        }
        let result = Rc::new(compile_raster(&full, ppp));
        let mut c = cache.borrow_mut();
        if c.len() >= CACHE_CAP {
            c.clear();
        }
        c.insert(key, Rc::clone(&result));
        result
    })
}

// ---------------------------------------------------------------------------
// Phase 2: lay the Typst box out and extract positioned glyphs + vector shapes
// (box-local points, y-down — matching the scene) for the PDF exporter, so the
// PDF carries real selectable text + vectors instead of a raster.
// ---------------------------------------------------------------------------

/// One positioned glyph in box-local points (`x`/`y` is the glyph's baseline origin).
#[derive(Debug, Clone)]
pub struct TGlyph {
    pub id: u16,
    pub x: f32,
    pub y: f32,
    /// The cluster's source text, for the PDF ToUnicode map (selection/copy).
    pub unicode: String,
}

/// A run of glyphs sharing a font + size + color.
#[derive(Debug, Clone)]
pub struct TGlyphRun {
    /// Stable per-font key (dedup across runs within one PDF).
    pub font_key: u64,
    pub font_data: Arc<Vec<u8>>,
    pub size_pt: f32,
    pub color: Rgba,
    pub glyphs: Vec<TGlyph>,
}

/// A vector shape from the Typst layout (fraction bars, rules, etc.), box-local points.
#[derive(Debug, Clone)]
pub enum TShape {
    Line {
        from: (f32, f32),
        to: (f32, f32),
        color: Rgba,
        width: f32,
    },
    Rect {
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        color: Rgba,
    },
}

/// The laid-out Typst box: positioned glyph runs + vector shapes, in box-local points.
#[derive(Debug, Default, Clone)]
pub struct TypstLayout {
    pub glyph_runs: Vec<TGlyphRun>,
    pub shapes: Vec<TShape>,
}

fn paint_rgba(paint: &typst::visualize::Paint) -> Rgba {
    match paint {
        typst::visualize::Paint::Solid(c) => {
            let [r, g, b, a] = c.to_vec4_u8();
            Rgba::rgba(r, g, b, a)
        }
        _ => Rgba::rgb(0, 0, 0),
    }
}

/// Recursively walk a frame, accumulating translation `(ox, oy)` in points (scale/skew of nested
/// groups is ignored — typst lays math/lists out with translation + per-run font size, which this
/// covers). Appends positioned glyph runs and vector shapes to `out`.
fn walk_frame(frame: &typst::layout::Frame, ox: f32, oy: f32, out: &mut TypstLayout) {
    use typst::layout::FrameItem;
    use typst::visualize::Geometry;
    for (point, item) in frame.items() {
        let px = ox + point.x.to_pt() as f32;
        let py = oy + point.y.to_pt() as f32;
        match item {
            FrameItem::Group(g) => {
                walk_frame(
                    &g.frame,
                    px + g.transform.tx.to_pt() as f32,
                    py + g.transform.ty.to_pt() as f32,
                    out,
                );
            }
            FrameItem::Text(t) => {
                let size = t.size;
                let size_pt = size.to_pt() as f32;
                let color = paint_rgba(&t.fill);
                let data = Arc::new(t.font.data().as_slice().to_vec());
                let font_key =
                    (t.font.data().as_slice().as_ptr() as u64) ^ ((t.font.index() as u64) << 1);
                let mut glyphs = Vec::with_capacity(t.glyphs.len());
                let mut xacc = 0.0f32;
                for g in &t.glyphs {
                    let gx = px + xacc + g.x_offset.at(size).to_pt() as f32;
                    let gy = py + g.y_offset.at(size).to_pt() as f32;
                    let unicode = t.text.get(g.range()).unwrap_or("").to_string();
                    glyphs.push(TGlyph {
                        id: g.id,
                        x: gx,
                        y: gy,
                        unicode,
                    });
                    xacc += g.x_advance.at(size).to_pt() as f32;
                }
                out.glyph_runs.push(TGlyphRun {
                    font_key,
                    font_data: data,
                    size_pt,
                    color,
                    glyphs,
                });
            }
            FrameItem::Shape(shape, _) => {
                let color = shape
                    .fill
                    .as_ref()
                    .map(paint_rgba)
                    .or_else(|| shape.stroke.as_ref().map(|s| paint_rgba(&s.paint)))
                    .unwrap_or(Rgba::rgb(0, 0, 0));
                match &shape.geometry {
                    Geometry::Line(to) => out.shapes.push(TShape::Line {
                        from: (px, py),
                        to: (px + to.x.to_pt() as f32, py + to.y.to_pt() as f32),
                        color,
                        width: shape
                            .stroke
                            .as_ref()
                            .map(|s| s.thickness.to_pt() as f32)
                            .unwrap_or(1.0),
                    }),
                    Geometry::Rect(size) => out.shapes.push(TShape::Rect {
                        x: px,
                        y: py,
                        w: size.x.to_pt() as f32,
                        h: size.y.to_pt() as f32,
                        color,
                    }),
                    // Curves (e.g. root signs) are uncommon; omit for now.
                    Geometry::Curve(_) => {}
                }
            }
            FrameItem::Image(..) | FrameItem::Link(..) | FrameItem::Tag(..) => {}
        }
    }
}

/// Lay a Typst box out and return its positioned glyphs + vector shapes (box-local points), for
/// the PDF exporter. Memoized like [`render_typst_raster`]. `box_w_pt` is the page width in points.
pub fn layout_typst(
    source: &str,
    box_w_pt: f32,
    default_pt: f32,
    color: Rgba,
    align: HAlign,
    bold: bool,
    italic: bool,
) -> Rc<Result<TypstLayout, String>> {
    let full = wrap_source(source, box_w_pt, default_pt, color, align, bold, italic);
    LAYOUT_CACHE.with(|cache| {
        if let Some(hit) = cache.borrow().get(&full) {
            return Rc::clone(hit);
        }
        let result = Rc::new(ENGINE.with(|engine| {
            let main = FileId::new_fake(VirtualPath::new("main.typ"));
            let world = TypstWorld {
                library: Arc::clone(&engine.library),
                book: Arc::clone(&engine.book),
                fonts: Arc::clone(&engine.fonts),
                main,
                source: Source::new(main, full.clone()),
            };
            let doc = typst::compile::<PagedDocument>(&world)
                .output
                .map_err(|diags| {
                    diags
                        .first()
                        .map(|d| d.message.to_string())
                        .unwrap_or_else(|| "typst compile error".to_string())
                })?;
            let page = doc
                .pages
                .first()
                .ok_or_else(|| "empty document".to_string())?;
            let mut layout = TypstLayout::default();
            walk_frame(&page.frame, 0.0, 0.0, &mut layout);
            Ok(layout)
        }));
        let mut c = cache.borrow_mut();
        if c.len() >= CACHE_CAP {
            c.clear();
        }
        c.insert(full, Rc::clone(&result));
        result
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn black() -> Rgba {
        Rgba::rgb(0, 0, 0)
    }

    /// True if any pixel has non-zero alpha (i.e. something was actually drawn).
    fn has_ink(img: &RasterImage) -> bool {
        img.rgba.chunks_exact(4).any(|px| px[3] != 0)
    }

    #[test]
    fn renders_a_bulleted_list() {
        let r = render_typst_raster(
            "- a\n- b",
            400.0,
            1.0,
            18.0,
            black(),
            HAlign::Left,
            false,
            false,
        );
        let img = r.as_ref().as_ref().expect("list compiles");
        assert!(img.px_w > 0 && img.px_h > 0);
        assert!(has_ink(img), "the list drew some pixels");
    }

    #[test]
    fn renders_math() {
        let r = render_typst_raster(
            "$x^2 + y^2$",
            400.0,
            1.0,
            18.0,
            black(),
            HAlign::Left,
            false,
            false,
        );
        let img = r
            .as_ref()
            .as_ref()
            .expect("math compiles (math font present)");
        assert!(has_ink(img), "the equation drew some pixels");
    }

    #[test]
    fn broken_source_errors() {
        let r = render_typst_raster(
            "$ x ^",
            400.0,
            1.0,
            18.0,
            black(),
            HAlign::Left,
            false,
            false,
        );
        assert!(r.as_ref().is_err(), "unbalanced math is a compile error");
    }

    #[test]
    fn identical_inputs_hit_cache() {
        let a = render_typst_raster(
            "hello",
            300.0,
            1.0,
            18.0,
            black(),
            HAlign::Left,
            false,
            false,
        );
        let b = render_typst_raster(
            "hello",
            300.0,
            1.0,
            18.0,
            black(),
            HAlign::Left,
            false,
            false,
        );
        assert!(Rc::ptr_eq(&a, &b), "second call returns the cached Rc");
    }

    #[test]
    fn layout_extracts_positioned_glyphs() {
        let r = layout_typst("hello", 400.0, 18.0, black(), HAlign::Left, false, false);
        let layout = r.as_ref().as_ref().expect("compiles");
        let total: usize = layout.glyph_runs.iter().map(|g| g.glyphs.len()).sum();
        assert!(total >= 5, "at least one glyph per letter, got {total}");
        // Glyphs advance left-to-right.
        let run = &layout.glyph_runs[0];
        assert!(run.glyphs[1].x > run.glyphs[0].x, "glyphs advance in x");
        assert!(!run.glyphs[0].unicode.is_empty(), "ToUnicode text present");
    }

    #[test]
    fn layout_math_has_a_fraction_bar() {
        // A fraction renders a horizontal rule (Shape) between numerator and denominator.
        let r = layout_typst("$ 1/2 $", 400.0, 24.0, black(), HAlign::Left, false, false);
        let layout = r.as_ref().as_ref().expect("math compiles");
        assert!(!layout.glyph_runs.is_empty(), "digits are glyphs");
        assert!(
            !layout.shapes.is_empty(),
            "the fraction bar is a vector shape"
        );
    }

    #[test]
    fn prose_linebreaks_only_breaks_between_prose() {
        // A lone newline between two prose lines becomes a Typst forced break.
        assert_eq!(prose_linebreaks("a\nb"), "a \\\nb");
        // A blank line (paragraph break) is left alone.
        assert_eq!(prose_linebreaks("a\n\nb"), "a\n\nb");
        // Lists and headings already break in Typst — leave them.
        assert_eq!(prose_linebreaks("- a\n- b"), "- a\n- b");
        assert_eq!(prose_linebreaks("= 見出し\n本文"), "= 見出し\n本文");
        // Newlines inside a math span are preserved.
        assert_eq!(prose_linebreaks("$ sum\n= x $"), "$ sum\n= x $");
        // An explicit trailing break is not doubled.
        assert_eq!(prose_linebreaks("a \\\nb"), "a \\\nb");
    }

    #[test]
    fn single_newline_renders_as_two_lines() {
        let baselines = |src: &str| -> usize {
            let r = layout_typst(src, 400.0, 18.0, black(), HAlign::Left, false, false);
            let layout = r.as_ref().as_ref().expect("compiles");
            let mut ys: Vec<i32> = layout
                .glyph_runs
                .iter()
                .flat_map(|run| run.glyphs.iter().map(|g| g.y.round() as i32))
                .collect();
            ys.sort_unstable();
            ys.dedup();
            ys.len()
        };
        assert_eq!(baselines("a b"), 1, "a space-joined line is one row");
        assert_eq!(baselines("a\nb"), 2, "a newline now renders as two rows");
    }

    #[test]
    fn italic_faux_oblique_renders_for_cjk() {
        // Noto Sans CJK has no italic face; faux-oblique (skew) must still slant it, so the
        // emphasized render differs from the upright one (and from a compile error).
        let upright =
            render_typst_raster("あ", 200.0, 2.0, 40.0, black(), HAlign::Left, false, false);
        let emph = render_typst_raster(
            "_あ_",
            200.0,
            2.0,
            40.0,
            black(),
            HAlign::Left,
            false,
            false,
        );
        let run_italic =
            render_typst_raster("あ", 200.0, 2.0, 40.0, black(), HAlign::Left, false, true);
        let u = upright.as_ref().as_ref().expect("upright compiles");
        let e = emph.as_ref().as_ref().expect("emph compiles");
        let ri = run_italic.as_ref().as_ref().expect("run italic compiles");
        assert!(has_ink(u) && has_ink(e) && has_ink(ri));
        // The page width is fixed, so the slant shows up as different pixels, not a wider box.
        assert!(
            *e.rgba != *u.rgba,
            "emphasis slants the glyph (pixels differ)"
        );
        assert!(
            *ri.rgba != *u.rgba,
            "run-level italic slants the glyph (pixels differ)"
        );
    }
}
