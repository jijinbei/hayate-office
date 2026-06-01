//! Data-oriented document store (DESIGN 6.10).
//!
//! An entity is just a stable id; data lives in sparse component columns. The presence or
//! absence of a component represents an override/inherit decision (DESIGN 6.8), and plugin
//! data will later live in dynamic, runtime-registered columns. The UI layer (gpui) does
//! not use this model; it stays reactive.
//!
//! `define_world!` also generates a tagged `CompValue` / `CompKind` and generic
//! `set`/`remove`/`get`, which is the substrate for the uniform component operations in the
//! editing layer (DESIGN 6.10's four-kind Operation).

use crate::doc::{LayoutInfo, MasterInfo, PlaceholderRef, SlideInfo};
use crate::frac::FracIndex;
use crate::geom::RectEmu;
use crate::paint::{Fill, Stroke};
use crate::shape::Geometry;
use crate::text::TextBody;
use serde::{Deserialize, Serialize};
use slotmap::{new_key_type, SecondaryMap, SlotMap};

new_key_type! {
    /// Stable id of a document entity (shape, slide, master, ...).
    pub struct Entity;
}

/// Generates `World` (one sparse column per component), a tagged `CompValue`, a `CompKind`
/// tag, and generic `set`/`remove`/`get`. Adding a built-in component is a one-line change.
macro_rules! define_world {
    ($($(#[$m:meta])* $field:ident : $variant:ident : $ty:ty),* $(,)?) => {
        /// Entities plus sparse component columns.
        #[derive(Default, Serialize, Deserialize)]
        pub struct World {
            entities: SlotMap<Entity, ()>,
            $( $(#[$m])* pub $field: SecondaryMap<Entity, $ty>, )*
        }

        /// A component value tagged by its kind (one variant per column).
        #[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
        pub enum CompValue {
            $( $variant($ty), )*
        }

        /// The kind of a component, used to address a column without a value.
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
        pub enum CompKind {
            $( $variant, )*
        }

        impl CompValue {
            pub fn kind(&self) -> CompKind {
                match self { $( CompValue::$variant(_) => CompKind::$variant, )* }
            }
        }

        impl World {
            fn clear_components(&mut self, e: Entity) {
                $( self.$field.remove(e); )*
            }

            /// Set a component on `e`, returning the previous value if present.
            pub fn set(&mut self, e: Entity, v: CompValue) -> Option<CompValue> {
                match v {
                    $( CompValue::$variant(val) =>
                        self.$field.insert(e, val).map(CompValue::$variant), )*
                }
            }

            /// Remove a component of `kind` from `e`, returning the previous value if present.
            pub fn remove(&mut self, e: Entity, kind: CompKind) -> Option<CompValue> {
                match kind {
                    $( CompKind::$variant =>
                        self.$field.remove(e).map(CompValue::$variant), )*
                }
            }

            /// Read a component value of `kind` from `e`.
            pub fn get(&self, e: Entity, kind: CompKind) -> Option<CompValue> {
                match kind {
                    $( CompKind::$variant =>
                        self.$field.get(e).cloned().map(CompValue::$variant), )*
                }
            }
        }
    };
}

define_world! {
    /// Bounding box in slide coordinates (EMU), pre-rotation.
    frames: Frame: RectEmu,
    /// Rotation in degrees, clockwise.
    rotations: Rotation: f32,
    /// Sibling order key among children sharing a parent.
    order: Order: FracIndex,
    /// Parent entity (group / slide). Absent = root of its container.
    parent: Parent: Entity,
    /// Optional human-readable name (debugging and Morph matching aid).
    names: Name: String,
    /// Interior fill.
    fills: Fill: Fill,
    /// Outline.
    strokes: Stroke: Stroke,
    /// Opacity in 0.0..=1.0 (absent = fully opaque).
    opacity: Opacity: f32,
    /// Vector geometry; presence marks the entity as a vector shape.
    geometries: Geometry: Geometry,
    /// Rich text content; presence marks the entity as a text box.
    texts: Text: TextBody,

    // --- structural (DESIGN 6.8) ---
    /// Marks the entity as a slide.
    slide_info: Slide: SlideInfo,
    /// Marks the entity as a layout.
    layout_info: Layout: LayoutInfo,
    /// Marks the entity as a master.
    master_info: Master: MasterInfo,
    /// Background fill override; resolves slide -> layout -> master.
    backgrounds: Background: Fill,
    /// Speaker notes (on slide entities).
    speaker_notes: Notes: String,
    /// Placeholder link for inheriting geometry/style from layout/master.
    placeholders: Placeholder: PlaceholderRef,
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

    #[test]
    fn generic_set_remove_get_roundtrip() {
        let mut w = World::new();
        let e = w.spawn();
        let v = CompValue::Frame(RectEmu::new(1, 2, 3, 4));
        assert_eq!(v.kind(), CompKind::Frame);

        // set on empty returns None
        assert_eq!(w.set(e, v.clone()), None);
        assert_eq!(w.get(e, CompKind::Frame), Some(v.clone()));

        // set again returns previous
        let v2 = CompValue::Frame(RectEmu::new(5, 6, 7, 8));
        assert_eq!(w.set(e, v2.clone()), Some(v));
        assert_eq!(w.get(e, CompKind::Frame), Some(v2.clone()));

        // remove returns previous, then None
        assert_eq!(w.remove(e, CompKind::Frame), Some(v2));
        assert_eq!(w.get(e, CompKind::Frame), None);
        assert_eq!(w.remove(e, CompKind::Frame), None);
    }
}
