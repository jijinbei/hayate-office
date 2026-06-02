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
            meta.insert(
                "slide_size",
                serde_json::to_vec(&pres.slide_size)?.as_slice(),
            )?;
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

// --- Human-readable JSON dump (open-format escape hatch, DESIGN) -----------
//
// `.hayate` is a binary redb database; for inspectability and debugging we also
// expose a plain, pretty-printed JSON view of the whole document. This is the
// open-format escape hatch promised in DESIGN: it lets a human (or a script)
// read the full document state without the redb engine. It is a one-way dump,
// not a load path.

/// Render `pres` as pretty-printed JSON:
/// `{ "format_version", "slide_size", "default_master", "entities": { "<id>": [<CompValue>, ...] } }`.
pub fn dump_json(pres: &Presentation) -> String {
    // Build the per-entity component map keyed by the stringified entity id, so
    // the result is a stable, human-readable JSON object.
    let mut entities = serde_json::Map::new();
    for e in pres.world.iter() {
        let comps = pres.world.components_of(e);
        entities.insert(e.0.to_string(), serde_json::json!(comps));
    }

    let doc = serde_json::json!({
        "format_version": FORMAT_VERSION,
        "slide_size": pres.slide_size,
        "default_master": pres.default_master,
        "entities": entities,
    });

    // dump_json builds a serde_json::Value, which serializes infallibly to a
    // String, so to_string_pretty cannot fail here.
    serde_json::to_string_pretty(&doc).expect("serializing a serde_json::Value is infallible")
}

/// Write [`dump_json`] of `pres` to `path` atomically (write to a temp file,
/// then rename over the target), mirroring [`save`].
pub fn write_json(pres: &Presentation, path: impl AsRef<Path>) -> Result<()> {
    let path = path.as_ref();
    let tmp = path.with_extension("json.tmp");
    let _ = std::fs::remove_file(&tmp);

    std::fs::write(&tmp, dump_json(pres))?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

// --- Autosave / crash recovery (MVP) ---------------------------------------
//
// This is the MVP recovery seam. We keep a full-snapshot autosave file next to
// the document at `<doc>.autosave`, written via the same atomic `save()` path.
// On a crash the caller can detect a leftover autosave and offer to recover it.
//
// A true operation-log WAL (DESIGN 6.9) — appending edits incrementally and
// replaying them — can replace this snapshot approach later without changing
// the calling convention here.

/// Autosave file path for `doc_path`: the document path with `.autosave`
/// appended to its existing name (e.g. "deck.hayate" -> "deck.hayate.autosave").
pub fn autosave_path(doc_path: &Path) -> std::path::PathBuf {
    let mut name = doc_path.as_os_str().to_os_string();
    name.push(".autosave");
    std::path::PathBuf::from(name)
}

/// Save a recovery snapshot of `pres` next to `doc_path`.
pub fn autosave(pres: &Presentation, doc_path: impl AsRef<Path>) -> Result<()> {
    save(pres, autosave_path(doc_path.as_ref()))
}

/// Whether an autosave snapshot exists for `doc_path`.
pub fn has_autosave(doc_path: impl AsRef<Path>) -> bool {
    autosave_path(doc_path.as_ref()).exists()
}

/// Load the autosave snapshot for `doc_path`.
pub fn load_autosave(doc_path: impl AsRef<Path>) -> Result<Presentation> {
    load(autosave_path(doc_path.as_ref()))
}

/// Remove the autosave file for `doc_path` if present; errors are ignored
/// (e.g. there is nothing to recover after a clean save).
pub fn clear_autosave(doc_path: impl AsRef<Path>) {
    let _ = std::fs::remove_file(autosave_path(doc_path.as_ref()));
}

#[cfg(test)]
mod tests;
