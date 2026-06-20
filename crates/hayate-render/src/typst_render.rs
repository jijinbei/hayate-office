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
/// font discovery used by the PDF CID embedder.
const CJK_FONT_PATHS: &[&str] = &[
    "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
    "/usr/share/fonts/opentype/noto/NotoSansCJK-Bold.ttc",
    "/run/current-system/sw/share/X11/fonts/NotoSansCJK-Regular.ttc",
    "/usr/share/fonts/truetype/noto/NotoSansCJK-Regular.ttc",
];

fn build_engine() -> Engine {
    let mut fonts: Vec<Font> = Vec::new();
    // Typst's bundled fonts (incl. New Computer Modern + its math font, Libertinus, DejaVu Mono).
    for data in typst_assets::fonts() {
        load_faces(Bytes::new(data), &mut fonts);
    }
    // System CJK so Japanese text/labels render.
    for path in CJK_FONT_PATHS {
        if let Ok(bytes) = std::fs::read(path) {
            load_faces(Bytes::new(bytes), &mut fonts);
            break;
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
}

/// Max cached raster results per thread (bounds memory across zoom levels).
const CACHE_CAP: usize = 256;

/// Build the full Typst document (preamble + user source) for a box of width `box_w_pt`.
fn wrap_source(source: &str, box_w_pt: f32, default_pt: f32, color: Rgba, align: HAlign) -> String {
    let al = match align {
        HAlign::Left | HAlign::Justify => "left",
        HAlign::Center => "center",
        HAlign::Right => "right",
    };
    let justify = matches!(align, HAlign::Justify);
    format!(
        "#set page(width: {w}pt, height: auto, margin: 0pt, fill: none)\n\
         #set text(size: {sz}pt, fill: rgb(\"#{r:02x}{g:02x}{b:02x}\"), \
         font: (\"New Computer Modern\", \"Noto Sans CJK JP\"))\n\
         #set par(justify: {justify})\n\
         #set align({al})\n\
         {body}",
        w = box_w_pt.max(1.0),
        sz = default_pt.max(1.0),
        r = color.r,
        g = color.g,
        b = color.b,
        justify = justify,
        al = al,
        body = source,
    )
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
) -> Rc<Result<RasterImage, String>> {
    let box_w_pt = (box_w_px / ppp.max(0.1)).max(1.0);
    let full = wrap_source(source, box_w_pt, default_pt, color, align);
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
        let r = render_typst_raster("- a\n- b", 400.0, 1.0, 18.0, black(), HAlign::Left);
        let img = r.as_ref().as_ref().expect("list compiles");
        assert!(img.px_w > 0 && img.px_h > 0);
        assert!(has_ink(img), "the list drew some pixels");
    }

    #[test]
    fn renders_math() {
        let r = render_typst_raster("$x^2 + y^2$", 400.0, 1.0, 18.0, black(), HAlign::Left);
        let img = r
            .as_ref()
            .as_ref()
            .expect("math compiles (math font present)");
        assert!(has_ink(img), "the equation drew some pixels");
    }

    #[test]
    fn broken_source_errors() {
        let r = render_typst_raster("$ x ^", 400.0, 1.0, 18.0, black(), HAlign::Left);
        assert!(r.as_ref().is_err(), "unbalanced math is a compile error");
    }

    #[test]
    fn identical_inputs_hit_cache() {
        let a = render_typst_raster("hello", 300.0, 1.0, 18.0, black(), HAlign::Left);
        let b = render_typst_raster("hello", 300.0, 1.0, 18.0, black(), HAlign::Left);
        assert!(Rc::ptr_eq(&a, &b), "second call returns the cached Rc");
    }
}
