//! Unit tests for the parent module.

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
    std::env::temp_dir().join(format!("hayate_pptx_{tag}_{}_{n}.pptx", std::process::id()))
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
    assert_eq!(
        imported.slide_size, p.slide_size,
        "slide size should round-trip"
    );

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

#[test]
fn linear_gradient_roundtrips() {
    // A shape with a two-stop linear gradient should survive export+import: both literal
    // stop colors and the angle (within the 60000ths-of-a-degree quantization) come back.
    let mut p = Presentation::new();
    let master = p.add_master(Theme::default());
    let layout = p.add_layout(master, "Blank");
    let slide = p.add_slide(layout);

    let frame = RectEmu::new(914_400, 457_200, 3_000_000, 2_000_000);
    let from = Color::literal(Rgba::rgb(0x1E, 0x88, 0xE5));
    let to = Color::literal(Rgba::rgb(0xE5, 0x39, 0x35));
    let rect = p.add_shape(slide);
    p.world.frames.insert(rect, frame);
    p.world.geometries.insert(rect, Geometry::Rect);
    p.world.fills.insert(
        rect,
        Fill::Linear {
            from,
            to,
            angle_deg: 45.0,
        },
    );

    let path = unique_temp_path("gradient");
    let _ = std::fs::remove_file(&path);
    export_pptx(&p, &path).expect("export should succeed");
    let imported = import_pptx(&path).expect("import should succeed");

    let first = imported.slides()[0];
    let fill = imported
        .children(first)
        .iter()
        .find_map(|c| imported.world.fills.get(c).copied())
        .expect("expected a fill after import");
    match fill {
        Fill::Linear {
            from: f,
            to: t,
            angle_deg,
        } => {
            assert_eq!(f, from, "gradient `from` color should round-trip");
            assert_eq!(t, to, "gradient `to` color should round-trip");
            assert!(
                (angle_deg - 45.0).abs() < 0.5,
                "angle should round-trip: {angle_deg}"
            );
        }
        other => panic!("expected a linear gradient, got {other:?}"),
    }
    let _ = std::fs::remove_file(&path);
}

#[test]
fn round_rect_radius_roundtrips() {
    // A round rect whose corner radius is a fraction of its smaller dimension should
    // survive export+import within the EMU<->adj quantization tolerance.
    let mut p = Presentation::new();
    let master = p.add_master(Theme::default());
    let layout = p.add_layout(master, "Blank");
    let slide = p.add_slide(layout);

    // Frame: 4,000,000 x 2,000,000 EMU. min(w,h) = 2,000,000. radius = 500,000 -> adj 25000.
    let frame = RectEmu::new(914_400, 457_200, 4_000_000, 2_000_000);
    let radius: i64 = 500_000;
    let rr = p.add_shape(slide);
    p.world.frames.insert(rr, frame);
    p.world
        .geometries
        .insert(rr, Geometry::RoundRect { radius });
    p.world
        .fills
        .insert(rr, Fill::Solid(Color::literal(Rgba::rgb(0x10, 0x20, 0x30))));

    let path = unique_temp_path("roundrect");
    let _ = std::fs::remove_file(&path);
    export_pptx(&p, &path).expect("export should succeed");
    let imported = import_pptx(&path).expect("import should succeed");

    let first = imported.slides()[0];
    let got = imported
        .children(first)
        .iter()
        .find_map(|c| match imported.world.geometries.get(c) {
            Some(Geometry::RoundRect { radius }) => Some(*radius),
            _ => None,
        })
        .expect("expected a round-rect geometry after import");

    // Tolerance: adj has 1/100000 granularity of min_dim => up to ~20 EMU here.
    let tol = 2_000_000 / 100_000 + 1;
    assert!(
        (got - radius).abs() <= tol,
        "round-rect radius should round-trip: got {got}, expected ~{radius}"
    );
    assert!(got > 0, "radius must not collapse to zero");

    let _ = std::fs::remove_file(&path);
}

#[test]
fn line_geometry_roundtrips() {
    // A line shape should export as a connector preset and import back to a Line geometry.
    let mut p = Presentation::new();
    let master = p.add_master(Theme::default());
    let layout = p.add_layout(master, "Blank");
    let slide = p.add_slide(layout);

    let line = p.add_shape(slide);
    p.world
        .frames
        .insert(line, RectEmu::new(914_400, 457_200, 3_000_000, 2_000_000));
    p.world
        .geometries
        .insert(line, Geometry::Line { arrow: false });

    let path = unique_temp_path("line");
    let _ = std::fs::remove_file(&path);
    export_pptx(&p, &path).expect("export should succeed");
    let imported = import_pptx(&path).expect("import should succeed");

    let first = imported.slides()[0];
    let got = imported.children(first).iter().any(|c| {
        matches!(
            imported.world.geometries.get(c),
            Some(Geometry::Line { .. })
        )
    });
    assert!(got, "expected a line geometry after import");

    let _ = std::fs::remove_file(&path);
}

#[test]
fn embedded_image_roundtrips() {
    // A deck with a single embedded PNG picture should round-trip: after import there is a
    // shape carrying a picture whose media bytes match the original.
    let mut p = Presentation::new();
    let master = p.add_master(Theme::default());
    let layout = p.add_layout(master, "Blank");
    let slide = p.add_slide(layout);

    // A tiny fake PNG: just needs the PNG magic so the exporter picks the png extension.
    let png_bytes: Vec<u8> = vec![
        0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 1, 2, 3, 4, 5, 6, 7, 8,
    ];
    let key = p.add_media(png_bytes.clone());

    let frame = RectEmu::new(914_400, 457_200, 2_000_000, 1_500_000);
    let pic = p.add_shape(slide);
    p.world.frames.insert(pic, frame);
    p.world.pictures.insert(
        pic,
        hayate_ir::image::PictureRef {
            media_key: key,
            natural: hayate_ir::geom::SizeEmu::new(2_000_000, 1_500_000),
        },
    );

    let path = unique_temp_path("image");
    let _ = std::fs::remove_file(&path);
    export_pptx(&p, &path).expect("export should succeed");
    let imported = import_pptx(&path).expect("import should succeed");

    let first = imported.slides()[0];
    let pic_child = imported
        .children(first)
        .into_iter()
        .find(|c| imported.world.pictures.contains_key(c))
        .expect("expected a shape with a picture after import");

    // The picture's frame round-trips.
    assert_eq!(
        imported.world.frames.get(&pic_child).copied(),
        Some(frame),
        "picture frame should round-trip"
    );

    // The media bytes resolve and match the original.
    let pref = imported.world.pictures.get(&pic_child).unwrap();
    let got = imported
        .get_media(&pref.media_key)
        .expect("imported media bytes should exist");
    assert_eq!(got, &png_bytes, "embedded image bytes should round-trip");

    let _ = std::fs::remove_file(&path);
}
