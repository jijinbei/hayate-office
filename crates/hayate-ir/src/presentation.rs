//! The `Presentation` document: a `World` plus top-level metadata, with helpers for
//! ordered traversal and master/layout/slide inheritance resolution (DESIGN 6.8/6.10).

use crate::doc::{LayoutInfo, MasterInfo, SlideInfo};
use crate::frac::FracIndex;
use crate::geom::SizeEmu;
use crate::paint::Fill;
use crate::theme::Theme;
use crate::units::inch_f;
use crate::world::{Entity, World};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Default, Serialize, Deserialize)]
pub struct Presentation {
    pub world: World,
    pub slide_size: SizeEmu,
    pub default_master: Option<Entity>,
    /// In-memory store of embedded media, keyed by content hash. Persistence to
    /// the .hayate/PPTX package is handled elsewhere.
    pub media: BTreeMap<String, Vec<u8>>,
}

impl Presentation {
    /// A new empty presentation with a 16:9 slide size (13.333in x 7.5in).
    pub fn new() -> Self {
        Self {
            world: World::new(),
            slide_size: SizeEmu::new(inch_f(13.333), inch_f(7.5)),
            default_master: None,
            media: BTreeMap::new(),
        }
    }

    /// Add `bytes` to the in-memory media store, returning a stable content key.
    ///
    /// The key is the FNV-1a hash of the bytes rendered as hex. Insertion is
    /// idempotent: adding identical bytes yields the same key and a single entry.
    pub fn add_media(&mut self, bytes: Vec<u8>) -> String {
        let key = media_key(&bytes);
        self.media.entry(key.clone()).or_insert(bytes);
        key
    }

    /// Look up media bytes by content key.
    pub fn get_media(&self, key: &str) -> Option<&Vec<u8>> {
        self.media.get(key)
    }

    /// Order key to append after the current maximum among `siblings`.
    fn append_key(&self, siblings: &[Entity]) -> FracIndex {
        let last = siblings.last().and_then(|e| self.world.order.get(e));
        FracIndex::after(last)
    }

    /// Add a master carrying `theme`. Becomes the default master if none is set yet.
    pub fn add_master(&mut self, theme: Theme) -> Entity {
        let e = self.world.spawn();
        self.world.master_info.insert(e, MasterInfo { theme });
        if self.default_master.is_none() {
            self.default_master = Some(e);
        }
        e
    }

    /// Add a layout under `master`.
    pub fn add_layout(&mut self, master: Entity, name: impl Into<String>) -> Entity {
        let e = self.world.spawn();
        self.world.layout_info.insert(
            e,
            LayoutInfo {
                master,
                name: name.into(),
            },
        );
        e
    }

    /// Add a slide based on `layout`, appended after existing slides.
    pub fn add_slide(&mut self, layout: Entity) -> Entity {
        let key = self.append_key(&self.slides());
        let e = self.world.spawn();
        self.world.slide_info.insert(e, SlideInfo { layout });
        self.world.order.insert(e, key);
        e
    }

    /// Add a shape under `parent`, appended after existing children.
    pub fn add_shape(&mut self, parent: Entity) -> Entity {
        let key = self.append_key(&self.children(parent));
        let e = self.world.spawn();
        self.world.parent.insert(e, parent);
        self.world.order.insert(e, key);
        e
    }

    /// Children of `parent`, sorted by order key.
    pub fn children(&self, parent: Entity) -> Vec<Entity> {
        let mut v: Vec<Entity> = self
            .world
            .iter()
            .filter(|e| self.world.parent.get(e) == Some(&parent))
            .collect();
        v.sort_by(|a, b| self.world.order.get(a).cmp(&self.world.order.get(b)));
        v
    }

    /// All slides, sorted by order key.
    pub fn slides(&self) -> Vec<Entity> {
        let mut v: Vec<Entity> = self
            .world
            .iter()
            .filter(|e| self.world.slide_info.contains_key(e))
            .collect();
        v.sort_by(|a, b| self.world.order.get(a).cmp(&self.world.order.get(b)));
        v
    }

    /// The master that a slide ultimately inherits from.
    pub fn master_of(&self, slide: Entity) -> Option<Entity> {
        let layout = self.world.slide_info.get(&slide)?.layout;
        Some(self.world.layout_info.get(&layout)?.master)
    }

    /// The theme in effect for a slide (via its master).
    pub fn theme_of(&self, slide: Entity) -> Option<&Theme> {
        let master = self.master_of(slide)?;
        self.world.master_info.get(&master).map(|m| &m.theme)
    }

    /// Resolve a slide's background, falling back slide -> layout -> master.
    pub fn background_of(&self, slide: Entity) -> Option<Fill> {
        if let Some(f) = self.world.backgrounds.get(&slide) {
            return Some(*f);
        }
        let layout = self.world.slide_info.get(&slide)?.layout;
        if let Some(f) = self.world.backgrounds.get(&layout) {
            return Some(*f);
        }
        let master = self.world.layout_info.get(&layout)?.master;
        self.world.backgrounds.get(&master).copied()
    }
}

/// Compute a stable content key for `bytes` using the 64-bit FNV-1a hash,
/// rendered as a lowercase hex string. No external hashing dependency is used.
fn media_key(bytes: &[u8]) -> String {
    const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;
    let mut hash = FNV_OFFSET;
    for &b in bytes {
        hash ^= b as u64;
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    format!("{hash:016x}")
}

#[cfg(test)]
mod tests;
