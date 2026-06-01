//! Data-oriented document store (DESIGN 6.10).
//!
//! An entity is just a stable id; data lives in sparse component columns. The presence or
//! absence of a component represents an override/inherit decision (DESIGN 6.8), and plugin
//! data will later live in dynamic, runtime-registered columns. The UI layer (gpui) does
//! not use this model; it stays reactive.

use crate::frac::FracIndex;
use crate::geom::RectEmu;
use crate::paint::{Fill, Stroke};
use crate::shape::Geometry;
use serde::{Deserialize, Serialize};
use slotmap::{new_key_type, SecondaryMap, SlotMap};

new_key_type! {
    /// Stable id of a document entity (shape, slide, master, ...).
    pub struct Entity;
}

/// Generates the `World` with one sparse column per listed component, plus a private
/// `clear_components` that drops an entity from every column. Adding a built-in component
/// is a one-line change here.
macro_rules! define_world {
    ($($(#[$m:meta])* $field:ident : $ty:ty),* $(,)?) => {
        /// Entities plus sparse component columns.
        #[derive(Default, Serialize, Deserialize)]
        pub struct World {
            entities: SlotMap<Entity, ()>,
            $( $(#[$m])* pub $field: SecondaryMap<Entity, $ty>, )*
        }

        impl World {
            fn clear_components(&mut self, e: Entity) {
                $( self.$field.remove(e); )*
            }
        }
    };
}

define_world! {
    /// Bounding box in slide coordinates (EMU), pre-rotation.
    frames: RectEmu,
    /// Rotation in degrees, clockwise.
    rotations: f32,
    /// Sibling order key among children sharing a parent.
    order: FracIndex,
    /// Parent entity (group / slide). Absent = root of its container.
    parent: Entity,
    /// Optional human-readable name (debugging and Morph matching aid).
    names: String,
    /// Interior fill.
    fills: Fill,
    /// Outline.
    strokes: Stroke,
    /// Opacity in 0.0..=1.0 (absent = fully opaque).
    opacity: f32,
    /// Vector geometry; presence marks the entity as a vector shape.
    geometries: Geometry,
}

impl World {
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a new live entity with no components.
    pub fn spawn(&mut self) -> Entity {
        self.entities.insert(())
    }

    pub fn is_alive(&self, e: Entity) -> bool {
        self.entities.contains_key(e)
    }

    /// Remove an entity and all of its components. Returns whether it existed.
    pub fn despawn(&mut self, e: Entity) -> bool {
        if self.entities.remove(e).is_some() {
            self.clear_components(e);
            true
        } else {
            false
        }
    }

    pub fn len(&self) -> usize {
        self.entities.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entities.is_empty()
    }

    /// Iterate over all live entities (unordered).
    pub fn iter(&self) -> impl Iterator<Item = Entity> + '_ {
        self.entities.keys()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spawn_and_despawn() {
        let mut w = World::new();
        let e = w.spawn();
        assert!(w.is_alive(e));
        assert_eq!(w.len(), 1);
        assert!(w.despawn(e));
        assert!(!w.is_alive(e));
        assert!(w.is_empty());
        assert!(!w.despawn(e), "double despawn is a no-op");
    }

    #[test]
    fn despawn_clears_components() {
        let mut w = World::new();
        let e = w.spawn();
        w.frames.insert(e, RectEmu::new(0, 0, 100, 100));
        w.names.insert(e, "title".to_string());
        assert!(w.frames.contains_key(e));
        w.despawn(e);
        assert!(!w.frames.contains_key(e));
        assert!(!w.names.contains_key(e));
    }

    #[test]
    fn columns_are_independent() {
        let mut w = World::new();
        let a = w.spawn();
        let b = w.spawn();
        w.frames.insert(a, RectEmu::new(0, 0, 10, 10));
        w.rotations.insert(b, 45.0);
        assert!(w.frames.contains_key(a) && !w.frames.contains_key(b));
        assert!(w.rotations.contains_key(b) && !w.rotations.contains_key(a));
        assert_eq!(w.iter().count(), 2);
    }
}
