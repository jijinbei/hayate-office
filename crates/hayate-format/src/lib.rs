//! `.hayate` persistence (DESIGN 6.9).
//!
//! Storage engine is redb (pure-Rust, ACID). The document is stored per-entity (a row per
//! entity holding its component values as JSON) plus a small `meta` table. Saving is atomic:
//! we write a temporary database and rename it over the target.
//!
//! For MVP this is a full snapshot. The redb tables are the seam for later incremental
//! writes and an operation-log (WAL) for crash recovery.

use hayate_ir::presentation::Presentation;
use hayate_ir::world::{CompValue, Entity};
use redb::{Database, ReadableTable, TableDefinition};
use std::path::Path;

/// On-disk format version (semver). Stored in `meta` for forward-compat policy.
pub const FORMAT_VERSION: &str = "0.1.0";

const META: TableDefinition<&str, &[u8]> = TableDefinition::new("meta");
const ENTITIES: TableDefinition<u64, &[u8]> = TableDefinition::new("entities");

pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

/// Save `pres` to `path` atomically (write to a temp db, then rename over the target).
pub fn save(pres: &Presentation, path: impl AsRef<Path>) -> Result<()> {
    let path = path.as_ref();
    let tmp = path.with_extension("hayate.tmp");
    let _ = std::fs::remove_file(&tmp);

    {
        let db = Database::create(&tmp)?;
        let wtx = db.begin_write()?;
        {
            let mut meta = wtx.open_table(META)?;
            meta.insert("format_version", FORMAT_VERSION.as_bytes())?;
            meta.insert("slide_size", serde_json::to_vec(&pres.slide_size)?.as_slice())?;
            meta.insert(
                "default_master",
                serde_json::to_vec(&pres.default_master)?.as_slice(),
            )?;

            let mut ents = wtx.open_table(ENTITIES)?;
            for e in pres.world.iter() {
                let comps = pres.world.components_of(e);
                ents.insert(e.0, serde_json::to_vec(&comps)?.as_slice())?;
            }
        }
        wtx.commit()?;
    }

    std::fs::rename(&tmp, path)?;
    Ok(())
}

/// Load a presentation from `path`.
pub fn load(path: impl AsRef<Path>) -> Result<Presentation> {
    let db = Database::open(path.as_ref())?;
    let rtx = db.begin_read()?;

    let mut pres = Presentation::new();
    {
        let meta = rtx.open_table(META)?;
        if let Some(v) = meta.get("slide_size")? {
            pres.slide_size = serde_json::from_slice(v.value())?;
        }
        if let Some(v) = meta.get("default_master")? {
            pres.default_master = serde_json::from_slice(v.value())?;
        }
    }
    {
        let ents = rtx.open_table(ENTITIES)?;
        for row in ents.iter()? {
            let (k, v) = row?;
            let e = Entity(k.value());
            pres.world.spawn_at(e);
            let comps: Vec<CompValue> = serde_json::from_slice(v.value())?;
            for c in comps {
                pres.world.set(e, c);
            }
        }
    }
    Ok(pres)
}

#[cfg(test)]
mod tests {
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
        std::env::temp_dir().join(format!("hayate-test-{tag}-{}-{n}.hayate", std::process::id()))
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
        assert_eq!(loaded.world.get(shapes[0], CompKind::Frame), Some(CompValue::Frame(frame)));
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
}
