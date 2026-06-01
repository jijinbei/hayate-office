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

#[derive(Default, Serialize, Deserialize)]
pub struct Presentation {
    pub world: World,
    pub slide_size: SizeEmu,
    pub default_master: Option<Entity>,
}

impl Presentation {
    /// A new empty presentation with a 16:9 slide size (13.333in x 7.5in).
    pub fn new() -> Self {
        Self {
            world: World::new(),
            slide_size: SizeEmu::new(inch_f(13.333), inch_f(7.5)),
            default_master: None,
        }
    }

    /// Order key to append after the current maximum among `siblings`.
    fn append_key(&self, siblings: &[Entity]) -> FracIndex {
        let last = siblings.last().and_then(|&e| self.world.order.get(e));
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
            .filter(|&e| self.world.parent.get(e) == Some(&parent))
            .collect();
        v.sort_by(|&a, &b| self.world.order.get(a).cmp(&self.world.order.get(b)));
        v
    }

    /// All slides, sorted by order key.
    pub fn slides(&self) -> Vec<Entity> {
        let mut v: Vec<Entity> = self
            .world
            .iter()
            .filter(|&e| self.world.slide_info.contains_key(e))
            .collect();
        v.sort_by(|&a, &b| self.world.order.get(a).cmp(&self.world.order.get(b)));
        v
    }

    /// The master that a slide ultimately inherits from.
    pub fn master_of(&self, slide: Entity) -> Option<Entity> {
        let layout = self.world.slide_info.get(slide)?.layout;
        Some(self.world.layout_info.get(layout)?.master)
    }

    /// The theme in effect for a slide (via its master).
    pub fn theme_of(&self, slide: Entity) -> Option<&Theme> {
        let master = self.master_of(slide)?;
        self.world.master_info.get(master).map(|m| &m.theme)
    }

    /// Resolve a slide's background, falling back slide -> layout -> master.
    pub fn background_of(&self, slide: Entity) -> Option<Fill> {
        if let Some(f) = self.world.backgrounds.get(slide) {
            return Some(*f);
        }
        let layout = self.world.slide_info.get(slide)?.layout;
        if let Some(f) = self.world.backgrounds.get(layout) {
            return Some(*f);
        }
        let master = self.world.layout_info.get(layout)?.master;
        self.world.backgrounds.get(master).copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::color::{Color, Rgba, ThemeColorToken};

    fn small_deck() -> (Presentation, Entity, Entity) {
        let mut p = Presentation::new();
        let master = p.add_master(Theme::default());
        let layout = p.add_layout(master, "Title and Content");
        let s1 = p.add_slide(layout);
        let s2 = p.add_slide(layout);
        (p, s1, s2)
    }

    #[test]
    fn slides_are_ordered_by_creation() {
        let (p, s1, s2) = small_deck();
        assert_eq!(p.slides(), vec![s1, s2]);
    }

    #[test]
    fn children_are_ordered() {
        let (mut p, s1, _) = small_deck();
        let a = p.add_shape(s1);
        let b = p.add_shape(s1);
        let c = p.add_shape(s1);
        assert_eq!(p.children(s1), vec![a, b, c]);
    }

    #[test]
    fn inheritance_resolves_to_master_and_theme() {
        let (mut p, s1, _) = small_deck();
        let master = p.master_of(s1).unwrap();
        // Background set only on the master is inherited by the slide.
        p.world
            .backgrounds
            .insert(master, Fill::Solid(Color::theme(ThemeColorToken::Lt1)));
        assert!(p.background_of(s1).is_some());
        // Theme resolves through master.
        let theme = p.theme_of(s1).unwrap();
        assert_eq!(
            theme.resolve_color(&Color::theme(ThemeColorToken::Lt1)),
            Rgba::WHITE
        );
    }
}
