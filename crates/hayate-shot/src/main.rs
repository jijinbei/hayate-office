//! Headless visual debug harness.
//!
//! Runs a series of editing scenarios against the gpui-free document model, renders each
//! result with the software rasterizer, and writes PNG snapshots to `debug-shots/`. This lets
//! the rendering pipeline (IR -> Scene -> pixels) be inspected without opening the GPU window.
//!
//! Run with `cargo run -p hayate-shot` (or `just shots`).

use hayate_core::CommandRegistry;
use hayate_ir::anim::Effect;
use hayate_ir::color::{Color, Rgba, ThemeColorToken};
use hayate_ir::font::{FontRef, ThemeFontSlot};
use hayate_ir::geom::RectEmu;
use hayate_ir::paint::{Fill, Stroke};
use hayate_ir::presentation::Presentation;
use hayate_ir::shape::Geometry;
use hayate_ir::text::{Paragraph, Run, TextBody};
use hayate_ir::theme::Theme;
use hayate_ir::units::{inch_f, pt};
use hayate_ir::world::Entity;
use hayate_model::{edit, Align, Axis, History};
use hayate_render::{build_slide_scene, build_slide_scene_at, encode_png, rasterize, PxSize};
use serde_json::json;
use std::path::Path;

const OUT_W: u32 = 960;
const OUT_H: u32 = 540;

fn main() {
    let out_dir = Path::new("debug-shots");
    std::fs::create_dir_all(out_dir).expect("create debug-shots dir");

    let reg = hayate_core::builtins();
    let mut idx = 0u32;
    // Snapshot the current slide under the next sequence number.
    macro_rules! save {
        ($name:expr, $p:expr, $slide:expr) => {
            write_shot(out_dir, &mut idx, $name, $p, $slide, None)
        };
    }

    // 00 base deck.
    let (p, slide, _) = deck();
    save!("base", &p, slide);

    // 01 move: shift the first accent rect.
    let (mut p, slide, mut h) = deck();
    let r1 = shape(&p, slide, 1);
    apply(
        &reg,
        &mut p,
        &mut h,
        "shape.set_position",
        json!({"entity": r1.0, "x": 30.0, "y": 360.0}),
    );
    save!("move", &p, slide);

    // 02 resize: enlarge the second accent rect.
    let (mut p, slide, mut h) = deck();
    let r2 = shape(&p, slide, 2);
    apply(
        &reg,
        &mut p,
        &mut h,
        "shape.set_size",
        json!({"entity": r2.0, "w": 320.0, "h": 90.0}),
    );
    save!("resize", &p, slide);

    // 03 rotate 30deg.
    let (mut p, slide, mut h) = deck();
    let r1 = shape(&p, slide, 1);
    apply(
        &reg,
        &mut p,
        &mut h,
        "shape.set_rotation",
        json!({"entity": r1.0, "degrees": 30.0}),
    );
    save!("rotate_30", &p, slide);

    // 04 rotate 90deg (third rect).
    let (mut p, slide, mut h) = deck();
    let r3 = shape(&p, slide, 3);
    apply(
        &reg,
        &mut p,
        &mut h,
        "shape.set_rotation",
        json!({"entity": r3.0, "degrees": 90.0}),
    );
    save!("rotate_90", &p, slide);

    // 05 duplicate: copy the first rect (offset is applied by the editor; here just a copy).
    let (mut p, slide, mut h) = deck();
    let r1 = shape(&p, slide, 1);
    let ne = p.world.reserve_id();
    let tx = edit::duplicate(&p.world, r1, ne);
    h.commit(&mut p.world, tx);
    // Nudge the copy so it is visually distinct.
    if let Some(tx) = reg.build(
        "shape.move",
        &json!({"entity": ne.0, "dx": 300000, "dy": 300000}),
        &p.world,
    ) {
        h.commit(&mut p.world, tx);
    }
    save!("duplicate", &p, slide);

    // 06 fill: recolor the second rect via an explicit hex color.
    let (mut p, slide, mut h) = deck();
    let r2 = shape(&p, slide, 2);
    apply(
        &reg,
        &mut p,
        &mut h,
        "shape.set_fill",
        json!({"entity": r2.0, "color": "#E91E63"}),
    );
    save!("fill", &p, slide);

    // 06b gradient: fill the second rect with a linear gradient.
    let (mut p, slide, mut h) = deck();
    let r2 = shape(&p, slide, 2);
    apply(
        &reg,
        &mut p,
        &mut h,
        "shape.fill_gradient",
        json!({"entity": r2.0, "from": "#1E88E5", "to": "#E53935", "angle": 0.0}),
    );
    save!("gradient", &p, slide);

    // 07 opacity: make the first rect semi-transparent.
    let (mut p, slide, mut h) = deck();
    let r1 = shape(&p, slide, 1);
    apply(
        &reg,
        &mut p,
        &mut h,
        "shape.set_opacity",
        json!({"entity": r1.0, "value": 0.35}),
    );
    save!("opacity", &p, slide);

    // 08/09 z-order: two overlapping rects, then bring the back one to front.
    let (mut p, slide, mut h) = overlap_deck();
    let kids = p.children(slide);
    let (back, _front) = (kids[0], kids[1]);
    save!("zorder_before", &p, slide);
    apply(
        &reg,
        &mut p,
        &mut h,
        "shape.bring_to_front",
        json!({"entity": back.0}),
    );
    save!("zorder_after", &p, slide);

    // 10 align: top-align the three accent rects (which start at the same y; nudge first).
    let (mut p, slide, mut h) = deck();
    let (a, b, c) = (
        shape(&p, slide, 1),
        shape(&p, slide, 2),
        shape(&p, slide, 3),
    );
    // Spread them vertically first so alignment is visible.
    for (e, y) in [(a, 200.0), (b, 320.0), (c, 120.0)] {
        if let Some(tx) = reg.build(
            "shape.set_position",
            &json!({"entity": e.0, "x": pt_x(e, &p), "y": y}),
            &p.world,
        ) {
            h.commit(&mut p.world, tx);
        }
    }
    save!("align_before", &p, slide);
    let tx = hayate_model::align::align(&p.world, &[a, b, c], Align::Top);
    h.commit(&mut p.world, tx);
    save!("align_top", &p, slide);

    // 11 distribute horizontally.
    let (mut p, slide, mut h) = deck();
    let (a, b, c) = (
        shape(&p, slide, 1),
        shape(&p, slide, 2),
        shape(&p, slide, 3),
    );
    let tx = hayate_model::align::distribute(&p.world, &[a, b, c], Axis::Horizontal);
    h.commit(&mut p.world, tx);
    save!("distribute_h", &p, slide);

    // 12 text: set ASCII text on the title to verify the bitmap font.
    let (mut p, slide, mut h) = deck();
    let title = shape(&p, slide, 0);
    apply(
        &reg,
        &mut p,
        &mut h,
        "shape.set_text",
        json!({"entity": title.0, "text": "HELLO WORLD 0123 (slide)"}),
    );
    save!("text_ascii", &p, slide);

    // 13 animation: add a fade entrance on the first rect, snapshot at t=0/350/700ms.
    let (mut p, slide, mut h) = deck();
    let r1 = shape(&p, slide, 1);
    let tx = edit::add_entrance(&p.world, slide, r1, Effect::Fade, 700);
    h.commit(&mut p.world, tx);
    for t in [0u32, 350, 700] {
        write_shot(
            out_dir,
            &mut idx,
            &format!("anim_fade_{t}ms"),
            &p,
            slide,
            Some(t),
        );
    }

    // 14 round rect + stroke: a rounded rectangle with a thick border, plus a stroked ellipse.
    {
        let mut p = Presentation::new();
        let master = p.add_master(Theme::default());
        let layout = p.add_layout(master, "Blank");
        let slide = p.add_slide(layout);

        let rr = p.add_shape(slide);
        p.world.frames.insert(
            rr,
            RectEmu::new(inch_f(1.0), inch_f(1.5), inch_f(4.0), inch_f(2.5)),
        );
        p.world.geometries.insert(
            rr,
            Geometry::RoundRect {
                radius: inch_f(0.5),
            },
        );
        p.world
            .fills
            .insert(rr, Fill::Solid(Color::theme(ThemeColorToken::Accent2)));
        p.world.strokes.insert(
            rr,
            Stroke::solid(Color::literal(Rgba::rgb(20, 20, 20)), pt(6).max(1)),
        );

        let el = p.add_shape(slide);
        p.world.frames.insert(
            el,
            RectEmu::new(inch_f(6.0), inch_f(1.5), inch_f(3.0), inch_f(2.5)),
        );
        p.world.geometries.insert(el, Geometry::Ellipse);
        // Stroke only (no fill) to verify outline rendering.
        p.world.strokes.insert(
            el,
            Stroke::solid(Color::literal(Rgba::rgb(30, 90, 200)), pt(8).max(1)),
        );
        write_shot(out_dir, &mut idx, "roundrect_stroke", &p, slide, None);
    }

    // 15 text formatting: bold + larger + centered title via the new core commands.
    let (mut p, slide, mut h) = deck();
    let title = shape(&p, slide, 0);
    apply(
        &reg,
        &mut p,
        &mut h,
        "shape.set_text",
        json!({"entity": title.0, "text": "BIG"}),
    );
    apply(
        &reg,
        &mut p,
        &mut h,
        "shape.toggle_bold",
        json!({"entity": title.0}),
    );
    apply(
        &reg,
        &mut p,
        &mut h,
        "shape.set_font_size",
        json!({"entity": title.0, "pt": 54.0}),
    );
    apply(
        &reg,
        &mut p,
        &mut h,
        "shape.align_text_center",
        json!({"entity": title.0}),
    );
    save!("text_format", &p, slide);

    // 16 PPTX round-trip with a round rect (radius) and an embedded image.
    {
        let mut p = Presentation::new();
        let master = p.add_master(Theme::default());
        let layout = p.add_layout(master, "Blank");
        let slide = p.add_slide(layout);

        let rr = p.add_shape(slide);
        p.world.frames.insert(
            rr,
            RectEmu::new(inch_f(0.8), inch_f(1.2), inch_f(4.0), inch_f(2.5)),
        );
        p.world.geometries.insert(
            rr,
            Geometry::RoundRect {
                radius: inch_f(0.6),
            },
        );
        p.world
            .fills
            .insert(rr, Fill::Solid(Color::theme(ThemeColorToken::Accent3)));

        // Embed a tiny real PNG (generated by our own encoder) as a picture.
        let img_w = 64u32;
        let img_h = 48u32;
        let mut rgba = vec![0u8; (img_w * img_h * 4) as usize];
        for (i, px) in rgba.chunks_exact_mut(4).enumerate() {
            let x = (i as u32) % img_w;
            px[0] = (x * 4) as u8;
            px[1] = 80;
            px[2] = 200;
            px[3] = 255;
        }
        let png = hayate_render::encode_png(&rgba, img_w, img_h);
        let key = p.add_media(png);
        let pic = p.add_shape(slide);
        p.world.frames.insert(
            pic,
            RectEmu::new(inch_f(5.5), inch_f(1.2), inch_f(3.0), inch_f(2.25)),
        );
        p.world.pictures.insert(
            pic,
            hayate_ir::image::PictureRef {
                media_key: key,
                natural: hayate_ir::geom::SizeEmu::new(inch_f(3.0), inch_f(2.25)),
            },
        );
        write_shot(out_dir, &mut idx, "roundrect_image_src", &p, slide, None);

        let tmp = out_dir.join("_roundrect_image.pptx");
        if hayate_format_pptx::export_pptx(&p, &tmp).is_ok() {
            if let Ok(imported) = hayate_format_pptx::import_pptx(&tmp) {
                let islide = imported.slides().first().copied().unwrap_or(slide);
                write_shot(
                    out_dir,
                    &mut idx,
                    "roundrect_image_roundtrip",
                    &imported,
                    islide,
                    None,
                );
            }
        }
        let _ = std::fs::remove_file(&tmp);
    }

    // 17 PPTX round-trip: export then re-import and render the imported deck.
    let (p, slide, _) = deck();
    let tmp = out_dir.join("_roundtrip.pptx");
    match hayate_format_pptx::export_pptx(&p, &tmp) {
        Ok(()) => match hayate_format_pptx::import_pptx(&tmp) {
            Ok(imported) => {
                let islide = imported.slides().first().copied().unwrap_or(slide);
                save!("pptx_roundtrip", &imported, islide);
            }
            Err(e) => eprintln!("pptx import failed: {e}"),
        },
        Err(e) => eprintln!("pptx export failed: {e}"),
    }
    let _ = std::fs::remove_file(&tmp);

    // PDF integrity snapshot: export a representative deck (Japanese title + a bulleted list +
    // a shape) to a real PDF. `just pdf-shot` renders it with poppler so the end-to-end PDF
    // (structure + embedded image) can be eyeballed; the same content also appears in the PNG
    // shots above, so the two can be compared.
    let pdf =
        hayate_render::pdf::export_pdf(&pdf_demo(), &hayate_render::pdf::PdfOptions::default());
    let pdf_path = out_dir.join("deck.pdf");
    std::fs::write(&pdf_path, pdf).expect("write pdf");
    eprintln!("wrote {}", pdf_path.display());

    println!("wrote {idx} snapshots to {}", out_dir.display());
}

/// A small deck exercising the PDF path's text: a bold Japanese+Latin title, a two-level bullet
/// list (JP and Latin), and an accent rectangle.
fn pdf_demo() -> Presentation {
    use hayate_ir::color::{Color, ThemeColorToken};
    use hayate_ir::font::{FontRef, ThemeFontSlot};
    use hayate_ir::geom::RectEmu;
    use hayate_ir::paint::Fill;
    use hayate_ir::shape::Geometry;
    use hayate_ir::text::{Paragraph, Run, TextBody};
    use hayate_ir::theme::Theme;
    use hayate_ir::units::{inch_f, pt};

    let run = |text: &str, slot, size, bold| Run {
        text: text.to_string(),
        font: FontRef::Theme(slot),
        size: pt(size),
        color: Color::theme(ThemeColorToken::Dk1),
        bold,
        italic: false,
        underline: false,
    };
    let mut p = Presentation::new();
    let master = p.add_master(Theme::default());
    let layout = p.add_layout(master, "Blank");
    let slide = p.add_slide(layout);

    let title = p.add_shape(slide);
    p.world.frames.insert(
        title,
        RectEmu::new(inch_f(0.5), inch_f(0.3), inch_f(9.0), inch_f(1.0)),
    );
    p.world.texts.insert(
        title,
        TextBody {
            paragraphs: vec![Paragraph::new(vec![run(
                "Hayate プレゼンテーション",
                ThemeFontSlot::Major,
                40,
                true,
            )])],
            autofit: false,
        },
    );

    let body = p.add_shape(slide);
    p.world.frames.insert(
        body,
        RectEmu::new(inch_f(0.5), inch_f(1.8), inch_f(9.0), inch_f(3.0)),
    );
    let bullet = |text: &str, level: u8| {
        let mut para = Paragraph::new(vec![run(text, ThemeFontSlot::Minor, 24, false)]);
        para.bullet_level = level;
        para
    };
    p.world.texts.insert(
        body,
        TextBody {
            paragraphs: vec![
                bullet("最初の項目", 1),
                bullet("子項目 child item", 2),
                bullet("Back to level 1", 1),
            ],
            autofit: false,
        },
    );

    let rect = p.add_shape(slide);
    p.world.frames.insert(
        rect,
        RectEmu::new(inch_f(0.5), inch_f(5.0), inch_f(1.6), inch_f(1.6)),
    );
    p.world.geometries.insert(rect, Geometry::Rect);
    p.world
        .fills
        .insert(rect, Fill::Solid(Color::theme(ThemeColorToken::Accent1)));
    p
}

/// Apply a registry command, committing it to history. Logs a no-op if the command is unknown
/// or produced no operations.
fn apply(
    reg: &CommandRegistry,
    p: &mut Presentation,
    h: &mut History,
    id: &str,
    args: serde_json::Value,
) {
    match reg.build(id, &args, &p.world) {
        Some(tx) => h.commit(&mut p.world, tx),
        None => eprintln!("command {id} produced no transaction (args: {args})"),
    }
}

/// Render `slide` of `p` and write the next-numbered PNG. `t` selects an animation time.
fn write_shot(
    out: &Path,
    idx: &mut u32,
    name: &str,
    p: &Presentation,
    slide: Entity,
    t: Option<u32>,
) {
    let target = PxSize {
        w: OUT_W as f32,
        h: OUT_H as f32,
    };
    let scene = match t {
        Some(ms) => build_slide_scene_at(p, slide, target, ms),
        None => build_slide_scene(p, slide, target),
    };
    let rgba = rasterize(&scene, OUT_W, OUT_H);
    let png = encode_png(&rgba, OUT_W, OUT_H);
    let file = out.join(format!("{:02}_{}.png", *idx, name));
    std::fs::write(&file, png).expect("write png");
    *idx += 1;
    println!("  {}", file.display());
}

/// The nth child shape of `slide` in document order (0 = title, 1..=3 = accent rects, 4 = oval).
fn shape(p: &Presentation, slide: Entity, n: usize) -> Entity {
    p.children(slide)[n]
}

/// Current x of `e` in points (for re-issuing set_position while only changing y).
fn pt_x(e: Entity, p: &Presentation) -> f32 {
    p.world
        .frames
        .get(&e)
        .map(|f| f.origin.x as f32 / 12_700.0)
        .unwrap_or(0.0)
}

/// Build the standard sample deck: a title, three accent rectangles, and an ellipse.
/// Returns the presentation, its single slide, and a fresh history.
fn deck() -> (Presentation, Entity, History) {
    let mut p = Presentation::new();
    let master = p.add_master(Theme::default());
    let layout = p.add_layout(master, "Blank");
    let slide = p.add_slide(layout);

    let title = p.add_shape(slide);
    p.world.frames.insert(
        title,
        RectEmu::new(inch_f(0.5), inch_f(0.3), inch_f(9.0), inch_f(1.0)),
    );
    p.world.texts.insert(
        title,
        TextBody {
            paragraphs: vec![Paragraph::new(vec![Run {
                text: "Hayate Presentation".to_string(),
                font: FontRef::Theme(ThemeFontSlot::Major),
                size: pt(40),
                color: Color::theme(ThemeColorToken::Dk1),
                bold: true,
                italic: false,
                underline: false,
            }])],
            autofit: false,
        },
    );

    let accents = [
        ThemeColorToken::Accent1,
        ThemeColorToken::Accent2,
        ThemeColorToken::Accent3,
    ];
    for (i, token) in accents.into_iter().enumerate() {
        let e = p.add_shape(slide);
        let x = inch_f(0.5 + i as f64 * 2.0);
        p.world
            .frames
            .insert(e, RectEmu::new(x, inch_f(1.8), inch_f(1.6), inch_f(1.6)));
        p.world.geometries.insert(e, Geometry::Rect);
        p.world.fills.insert(e, Fill::Solid(Color::theme(token)));
    }

    let oval = p.add_shape(slide);
    p.world.frames.insert(
        oval,
        RectEmu::new(inch_f(6.8), inch_f(1.8), inch_f(2.4), inch_f(1.6)),
    );
    p.world.geometries.insert(oval, Geometry::Ellipse);
    p.world
        .fills
        .insert(oval, Fill::Solid(Color::theme(ThemeColorToken::Accent4)));

    (p, slide, History::new())
}

/// A deck with two overlapping rects (for z-order demos): a back red rect and a front blue one.
fn overlap_deck() -> (Presentation, Entity, History) {
    let mut p = Presentation::new();
    let master = p.add_master(Theme::default());
    let layout = p.add_layout(master, "Blank");
    let slide = p.add_slide(layout);

    let back = p.add_shape(slide);
    p.world.frames.insert(
        back,
        RectEmu::new(inch_f(2.0), inch_f(2.0), inch_f(4.0), inch_f(3.0)),
    );
    p.world.geometries.insert(back, Geometry::Rect);
    p.world
        .fills
        .insert(back, Fill::Solid(Color::theme(ThemeColorToken::Accent1)));

    let front = p.add_shape(slide);
    p.world.frames.insert(
        front,
        RectEmu::new(inch_f(3.5), inch_f(3.0), inch_f(4.0), inch_f(3.0)),
    );
    p.world.geometries.insert(front, Geometry::Rect);
    p.world
        .fills
        .insert(front, Fill::Solid(Color::theme(ThemeColorToken::Accent2)));

    (p, slide, History::new())
}
