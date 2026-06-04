//! The `Presentation` document: a `World` plus top-level metadata, with helpers for
//! ordered traversal and master/layout/slide inheritance resolution (DESIGN 6.8/6.10).

use crate::doc::{LayoutInfo, MasterInfo, PlaceholderRef, SlideInfo};
use crate::frac::FracIndex;
use crate::geom::{RectEmu, SizeEmu};
use crate::paint::Fill;
use crate::shape::Geometry;
use crate::text::TextBody;
use crate::theme::Theme;
use crate::units::inch_f;
use crate::world::{Entity, World};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Default, Serialize, Deserialize)]
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
    pub fn get_media(&self, key: &str) -> Option<&[u8]> {
        self.media.get(key).map(Vec::as_slice)
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

    /// The layout a slide is based on.
    pub fn layout_of(&self, slide: Entity) -> Option<Entity> {
        Some(self.world.slide_info.get(&slide)?.layout)
    }

    /// The master that owns any container: a master is itself, a layout points at its master,
    /// and a slide resolves via its layout. Used to render/edit layouts and masters directly.
    pub fn owning_master(&self, container: Entity) -> Option<Entity> {
        if self.world.master_info.contains_key(&container) {
            return Some(container);
        }
        if let Some(li) = self.world.layout_info.get(&container) {
            return Some(li.master);
        }
        let si = self.world.slide_info.get(&container)?;
        self.world.layout_info.get(&si.layout).map(|l| l.master)
    }

    /// Theme in effect for any container (slide/layout/master), via its owning master.
    /// `container_theme(slide)` equals `theme_of(slide)`.
    pub fn container_theme(&self, container: Entity) -> Option<&Theme> {
        let master = self.owning_master(container)?;
        self.world.master_info.get(&master).map(|m| &m.theme)
    }

    /// Background resolved for any container, walking its inheritance chain up to the master.
    pub fn container_background(&self, container: Entity) -> Option<Fill> {
        if let Some(f) = self.world.backgrounds.get(&container) {
            return Some(*f);
        }
        if let Some(si) = self.world.slide_info.get(&container) {
            return self.background_of_layout_chain(si.layout);
        }
        if let Some(li) = self.world.layout_info.get(&container) {
            return self.world.backgrounds.get(&li.master).copied();
        }
        None // a master only has its own background (already checked above)
    }

    /// Background fallback from a layout up to its master.
    fn background_of_layout_chain(&self, layout: Entity) -> Option<Fill> {
        if let Some(f) = self.world.backgrounds.get(&layout) {
            return Some(*f);
        }
        let master = self.world.layout_info.get(&layout)?.master;
        self.world.backgrounds.get(&master).copied()
    }

    /// Direct children of `container` that carry a `Placeholder` component, in order.
    pub fn placeholder_shapes(&self, container: Entity) -> Vec<Entity> {
        self.children(container)
            .into_iter()
            .filter(|e| self.world.placeholders.contains_key(e))
            .collect()
    }

    /// The child of `container` whose `PlaceholderRef` matches `ph` (both type and idx).
    pub fn find_placeholder(&self, container: Entity, ph: PlaceholderRef) -> Option<Entity> {
        self.placeholder_shapes(container)
            .into_iter()
            .find(|e| self.world.placeholders.get(e) == Some(&ph))
    }

    /// Matching placeholder entities along the inheritance chain, MOST-DERIVED FIRST:
    /// `[slide_match?, layout_match?, master_match?]`, skipping levels with no match.
    pub fn placeholder_chain(&self, slide: Entity, ph: PlaceholderRef) -> [Option<Entity>; 3] {
        let slide_match = self.find_placeholder(slide, ph);
        let layout_match = self
            .layout_of(slide)
            .and_then(|layout| self.find_placeholder(layout, ph));
        let master_match = self
            .master_of(slide)
            .and_then(|master| self.find_placeholder(master, ph));
        [slide_match, layout_match, master_match]
    }

    /// Resolve a placeholder's frame, walking the chain and returning the first present.
    pub fn ph_frame(&self, slide: Entity, ph: PlaceholderRef) -> Option<RectEmu> {
        self.placeholder_chain(slide, ph)
            .into_iter()
            .flatten()
            .find_map(|e| self.world.frames.get(&e).copied())
    }

    /// Resolve a placeholder's text body, walking the chain and returning the first present.
    pub fn ph_text(&self, slide: Entity, ph: PlaceholderRef) -> Option<&TextBody> {
        self.placeholder_chain(slide, ph)
            .into_iter()
            .flatten()
            .find_map(|e| self.world.texts.get(&e))
    }

    /// Resolve a placeholder's fill, walking the chain and returning the first present.
    pub fn ph_fill(&self, slide: Entity, ph: PlaceholderRef) -> Option<Fill> {
        self.placeholder_chain(slide, ph)
            .into_iter()
            .flatten()
            .find_map(|e| self.world.fills.get(&e).copied())
    }

    /// Resolve a placeholder's geometry, walking the chain and returning the first present.
    pub fn ph_geometry(&self, slide: Entity, ph: PlaceholderRef) -> Option<Geometry> {
        self.placeholder_chain(slide, ph)
            .into_iter()
            .flatten()
            .find_map(|e| self.world.geometries.get(&e).cloned())
    }

    /// All placeholder refs in effect for a slide: the deduped (by type+idx) union of
    /// placeholders defined on the slide, its layout, and its master, sorted by
    /// `ph_type` then `idx` for a stable order.
    pub fn effective_placeholders(&self, slide: Entity) -> Vec<PlaceholderRef> {
        let mut containers = vec![slide];
        if let Some(layout) = self.layout_of(slide) {
            containers.push(layout);
        }
        if let Some(master) = self.master_of(slide) {
            containers.push(master);
        }
        let mut refs: Vec<PlaceholderRef> = Vec::new();
        for c in containers {
            for e in self.placeholder_shapes(c) {
                if let Some(ph) = self.world.placeholders.get(&e) {
                    if !refs
                        .iter()
                        .any(|r| r.ph_type == ph.ph_type && r.idx == ph.idx)
                    {
                        refs.push(*ph);
                    }
                }
            }
        }
        refs.sort_by(|a, b| a.ph_type.cmp(&b.ph_type).then(a.idx.cmp(&b.idx)));
        refs
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
