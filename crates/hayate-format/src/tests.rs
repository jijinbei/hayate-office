//! Unit tests for the parent module.

use super::*;
use hayate_ir::color::{Color, ThemeColorToken};
use hayate_ir::geom::RectEmu;
use hayate_ir::paint::Fill;
use hayate_ir::shape::Geometry;
use hayate_ir::theme::Theme;
use hayate_ir::world::CompKind;
use std::sync::atomic::{AtomicU64, Ordering};

fn temp_path(tag: &str) -> std::path::PathBuf {
    static N: AtomicU64 = AtomicU64::new(0);
    let n = N.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "hayate-test-{tag}-{}-{n}.hayate",
        std::process::id()
    ))
}

#[test]
fn roundtrip_preserves_document() {
    let mut p = Presentation::new();
    let master = p.add_master(Theme::default());
    let layout = p.add_layout(master, "Blank");
    let slide = p.add_slide(layout);
    let rect = p.add_shape(slide);
    let frame = RectEmu::new(10, 20, 300, 400);
    p.world.frames.insert(rect, frame);
    p.world.geometries.insert(rect, Geometry::Rect);
    p.world
        .fills
        .insert(rect, Fill::Solid(Color::theme(ThemeColorToken::Accent1)));

    let path = temp_path("roundtrip");
    save(&p, &path).unwrap();
    let loaded = load(&path).unwrap();

    // Same number of slides and same first-shape frame.
    assert_eq!(loaded.slides().len(), 1);
    let s = loaded.slides()[0];
    let shapes = loaded.children(s);
    assert_eq!(shapes.len(), 1);
    assert_eq!(
        loaded.world.get(shapes[0], CompKind::Frame),
        Some(CompValue::Frame(frame))
    );
    // Inheritance still resolves (theme via master).
    assert!(loaded.theme_of(s).is_some());

    let _ = std::fs::remove_file(&path);
}

#[test]
fn save_is_atomic_rename() {
    // A successful save leaves no .tmp behind.
    let p = Presentation::new();
    let path = temp_path("atomic");
    save(&p, &path).unwrap();
    let tmp = path.with_extension("hayate.tmp");
    assert!(!tmp.exists(), "temp file should be renamed away");
    assert!(path.exists());
    let _ = std::fs::remove_file(&path);
}

#[test]
fn autosave_path_appends_extension() {
    let p = std::path::Path::new("deck.hayate");
    assert_eq!(
        autosave_path(p),
        std::path::PathBuf::from("deck.hayate.autosave")
    );
}

#[test]
fn autosave_writes_detectable_file() {
    let p = Presentation::new();
    let doc = temp_path("autosave-write");
    assert!(!has_autosave(&doc));
    autosave(&p, &doc).unwrap();
    assert!(has_autosave(&doc));
    // Autosave does not create the document itself.
    assert!(!doc.exists());
    clear_autosave(&doc);
}

#[test]
fn load_autosave_roundtrips() {
    let mut p = Presentation::new();
    let master = p.add_master(Theme::default());
    let layout = p.add_layout(master, "Blank");
    p.add_slide(layout);
    p.add_slide(layout);

    let doc = temp_path("autosave-roundtrip");
    autosave(&p, &doc).unwrap();
    let loaded = load_autosave(&doc).unwrap();
    assert_eq!(loaded.slides().len(), 2);

    clear_autosave(&doc);
}

#[test]
fn dump_json_is_readable_and_contains_components() {
    let mut p = Presentation::new();
    let master = p.add_master(Theme::default());
    let layout = p.add_layout(master, "Blank");
    let slide = p.add_slide(layout);
    let rect = p.add_shape(slide);
    p.world.frames.insert(rect, RectEmu::new(10, 20, 300, 400));
    p.world.geometries.insert(rect, Geometry::Rect);

    let json = dump_json(&p);
    assert!(json.contains("format_version"));
    assert!(json.contains("entities"));
    // The component variant name appears for the shape's Frame.
    assert!(json.contains("Frame"));
    // It is valid, parseable JSON.
    let _: serde_json::Value = serde_json::from_str(&json).unwrap();
}

#[test]
fn write_json_writes_readable_file() {
    let mut p = Presentation::new();
    let master = p.add_master(Theme::default());
    let layout = p.add_layout(master, "Blank");
    let slide = p.add_slide(layout);
    let rect = p.add_shape(slide);
    p.world.frames.insert(rect, RectEmu::new(1, 2, 3, 4));

    let path = temp_path("dump-json").with_extension("json");
    write_json(&p, &path).unwrap();
    // No temp file left behind after the atomic rename.
    assert!(!path.with_extension("json.tmp").exists());

    let contents = std::fs::read_to_string(&path).unwrap();
    assert!(contents.contains("format_version"));
    assert!(contents.contains("Frame"));

    let _ = std::fs::remove_file(&path);
}

#[test]
fn clear_autosave_removes_file() {
    let p = Presentation::new();
    let doc = temp_path("autosave-clear");
    autosave(&p, &doc).unwrap();
    assert!(has_autosave(&doc));
    clear_autosave(&doc);
    assert!(!has_autosave(&doc));
    // Clearing a non-existent autosave is a no-op (no panic).
    clear_autosave(&doc);
}
