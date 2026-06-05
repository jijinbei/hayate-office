//! Slide-level operations: navigation, add/duplicate/delete, and reordering.

use hayate_ir::frac::FracIndex;
use hayate_ir::world::{CompValue, Entity};
use hayate_model::{Operation, Transaction};

use crate::HayateApp;

impl HayateApp {
    pub(crate) fn next_slide(&mut self, delta: i64) {
        // Slide navigation is disabled while editing a layout/master in place.
        if !self.scope.is_slide() {
            return;
        }
        let slides = self.pres.slides();
        if let Some(i) = slides.iter().position(|&s| s == self.slide) {
            let n = slides.len() as i64;
            let ni = ((i as i64 + delta) % n + n) % n;
            self.slide = slides[ni as usize];
            self.selection = None;
            self.also.clear();
            self.rebuild();
        }
    }

    /// Add a new slide based on the current slide's layout and switch to it.
    pub(crate) fn add_slide(&mut self) {
        if !self.scope.is_slide() {
            return;
        }
        if let Some(layout) = self
            .pres
            .world
            .slide_info
            .get(&self.slide)
            .map(|s| s.layout)
        {
            self.add_slide_with_layout(layout);
        }
    }

    /// Add a new slide using the given layout (chosen from the Add Slide menu) and switch to it.
    pub(crate) fn add_slide_with_layout(&mut self, layout: Entity) {
        if !self.scope.is_slide() {
            return;
        }
        let s = self.pres.add_slide(layout);
        self.slide = s;
        self.selection = None;
        self.also.clear();
        self.add_slide_menu = false;
        self.rebuild();
    }

    /// Duplicate the current slide (copying its shapes) and switch to the copy.
    pub(crate) fn duplicate_slide(&mut self) {
        let Some(layout) = self
            .pres
            .world
            .slide_info
            .get(&self.slide)
            .map(|s| s.layout)
        else {
            return;
        };
        let children = self.pres.children(self.slide);
        let new_slide = self.pres.add_slide(layout);
        for c in children {
            let comps = self.pres.world.components_of(c);
            let ne = self.pres.world.reserve_id();
            self.pres.world.spawn_at(ne);
            for comp in comps {
                let comp = match comp {
                    CompValue::Parent(_) => CompValue::Parent(new_slide),
                    other => other,
                };
                self.pres.world.set(ne, comp);
            }
        }
        self.slide = new_slide;
        self.selection = None;
        self.rebuild();
        let _ = hayate_format::autosave(&self.pres, &self.doc_path);
    }

    /// Delete the current slide (keeps at least one slide).
    pub(crate) fn delete_slide(&mut self) {
        if self.pres.slides().len() <= 1 {
            return;
        }
        for c in self.pres.children(self.slide) {
            self.pres.world.despawn(c);
        }
        self.pres.world.despawn(self.slide);
        self.slide = self.pres.slides().first().copied().unwrap_or(self.slide);
        self.selection = None;
        self.rebuild();
        let _ = hayate_format::autosave(&self.pres, &self.doc_path);
    }

    /// Reorder `dragged` to sit immediately before `target` in the slide list, by assigning it
    /// a fractional order key between `target` and its predecessor. No-op for self-drops.
    pub(crate) fn reorder_slide(&mut self, dragged: Entity, target: Entity) {
        if dragged == target {
            return;
        }
        // Slide sequence with the dragged slide removed, to find the drop neighbor.
        let mut seq = self.pres.slides();
        seq.retain(|&e| e != dragged);
        let Some(tpos) = seq.iter().position(|&e| e == target) else {
            return;
        };
        let lo = if tpos == 0 {
            None
        } else {
            self.pres.world.order.get(&seq[tpos - 1])
        };
        let hi = self.pres.world.order.get(&target);
        let key = FracIndex::between(lo, hi);
        let tx = Transaction::new(
            "reorder slide",
            vec![Operation::SetComponent {
                entity: dragged,
                value: CompValue::Order(key),
            }],
        );
        self.commit_tx(tx);
    }
}
