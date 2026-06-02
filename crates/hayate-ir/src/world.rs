//! Data-oriented document store (DESIGN 6.10).
//!
//! An entity is just a stable id; data lives in sparse component columns. The presence or
//! absence of a component represents an override/inherit decision (DESIGN 6.8), and plugin
//! data will later live in dynamic, runtime-registered columns. The UI layer (gpui) does
//! not use this model; it stays reactive.
//!
//! Entity ids are explicit `u64`s with a monotonic counter, so a despawned id can be
//! re-created with the *same* id via [`World::spawn_at`]. This makes Operation redo stable
//! (a generational slotmap key would change on re-spawn and break later references).
//!
//! `define_world!` also generates a tagged `CompValue` / `CompKind` and generic
//! `set`/`remove`/`get`, the substrate for the uniform component operations in the editing
//! layer (DESIGN 6.10's four-kind Operation).

use crate::doc::{LayoutInfo, MasterInfo, PlaceholderRef, SlideInfo};
use crate::frac::FracIndex;
use crate::geom::RectEmu;
use crate::paint::{Fill, Stroke};
use crate::shape::Geometry;
use crate::text::TextBody;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// Stable id of a document entity (shape, slide, master, ...).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct Entity(pub u64);

/// Generates `World` (one sparse column per component), a tagged `CompValue`, a `CompKind`
/// tag, and generic `set`/`remove`/`get`. Adding a built-in component is a one-line change.
macro_rules! define_world {
    ($($(#[$m:meta])* $field:ident : $variant:ident : $ty:ty),* $(,)?) => {
        /// Entities plus sparse component columns.
        #[derive(Default, Serialize, Deserialize)]
        pub struct World {
            next: u64,
            alive: BTreeSet<Entity>,
            $( $(#[$m])* pub $field: BTreeMap<Entity, $ty>, )*
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
                $( self.$field.remove(&e); )*
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
                        self.$field.remove(&e).map(CompValue::$variant), )*
                }
            }

            /// Read a component value of `kind` from `e`.
            pub fn get(&self, e: Entity, kind: CompKind) -> Option<CompValue> {
                match kind {
                    $( CompKind::$variant =>
                        self.$field.get(&e).cloned().map(CompValue::$variant), )*
                }
            }

            /// Collect every component currently attached to `e` (used to build the inverse
            /// of a despawn operation).
            pub fn components_of(&self, e: Entity) -> Vec<CompValue> {
                let mut out = Vec::new();
                $( if let Some(v) = self.$field.get(&e) {
                    out.push(CompValue::$variant(v.clone()));
                } )*
                out
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
    /// Group membership key: shapes sharing the same value are selected, moved, and deleted
    /// together. 0 is unused; a fresh nonzero key is minted per group.
    groups: Group: u64,
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

    // --- media & animation (DESIGN 6.15; data-only reserved seams) ---
    /// Picture reference into embedded media; presence marks the entity as a picture.
    pictures: Picture: crate::image::PictureRef,
    /// Per-slide animation timeline (on slide entities).
    timelines: Timeline: crate::anim::SlideTimeline,
    /// Slide screen transition (on slide entities).
    transitions: Transition: crate::anim::Transition,
    /// Morph matching key; carried across slide duplication to pair shapes for Morph.
    morph_keys: MorphKey: String,
}

impl World {
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a new live entity with a freshly allocated id.
    pub fn spawn(&mut self) -> Entity {
        let e = Entity(self.next);
        self.spawn_at(e);
        e
    }

    /// Reserve a fresh id without bringing it to life. The editing layer uses this to
    /// allocate an id for a `Spawn` operation without mutating document content, so the
    /// spawn stays fully captured by the (undoable) transaction.
    pub fn reserve_id(&mut self) -> Entity {
        let e = Entity(self.next);
        self.next += 1;
        e
    }

    /// Bring a specific id to life (used by Operation redo to recreate the same id).
    /// The id counter advances past `e` so future `spawn`s never collide.
    pub fn spawn_at(&mut self, e: Entity) {
        self.alive.insert(e);
        if e.0 >= self.next {
            self.next = e.0 + 1;
        }
    }

    pub fn is_alive(&self, e: Entity) -> bool {
        self.alive.contains(&e)
    }

    /// Remove an entity and all of its components. Returns whether it existed.
    pub fn despawn(&mut self, e: Entity) -> bool {
        if self.alive.remove(&e) {
            self.clear_components(e);
            true
        } else {
            false
        }
    }

    pub fn len(&self) -> usize {
        self.alive.len()
    }

    pub fn is_empty(&self) -> bool {
        self.alive.is_empty()
    }

    /// Iterate over all live entities in id order.
    pub fn iter(&self) -> impl Iterator<Item = Entity> + '_ {
        self.alive.iter().copied()
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
    fn spawn_at_recreates_same_id() {
        let mut w = World::new();
        let e = w.spawn();
        w.despawn(e);
        w.spawn_at(e);
        assert!(w.is_alive(e), "redo can recreate the same id");
        // A subsequent fresh spawn must not collide with e.
        let e2 = w.spawn();
        assert_ne!(e, e2);
    }

    #[test]
    fn despawn_clears_components() {
        let mut w = World::new();
        let e = w.spawn();
        w.frames.insert(e, RectEmu::new(0, 0, 100, 100));
        w.names.insert(e, "title".to_string());
        w.despawn(e);
        assert!(!w.frames.contains_key(&e));
        assert!(!w.names.contains_key(&e));
    }

    #[test]
    fn generic_set_remove_get_roundtrip() {
        let mut w = World::new();
        let e = w.spawn();
        let v = CompValue::Frame(RectEmu::new(1, 2, 3, 4));
        assert_eq!(v.kind(), CompKind::Frame);

        assert_eq!(w.set(e, v.clone()), None);
        assert_eq!(w.get(e, CompKind::Frame), Some(v.clone()));

        let v2 = CompValue::Frame(RectEmu::new(5, 6, 7, 8));
        assert_eq!(w.set(e, v2.clone()), Some(v));
        assert_eq!(w.get(e, CompKind::Frame), Some(v2.clone()));

        assert_eq!(w.remove(e, CompKind::Frame), Some(v2));
        assert_eq!(w.get(e, CompKind::Frame), None);
        assert_eq!(w.remove(e, CompKind::Frame), None);
    }

    #[test]
    fn picture_timeline_morph_set_get_roundtrip() {
        use crate::anim::{
            Anim, AnimKind, AnimStep, Easing, Effect, SlideTimeline, Transition, TransitionKind,
            Trigger,
        };
        use crate::geom::SizeEmu;
        use crate::image::PictureRef;

        let mut w = World::new();
        let e = w.spawn();

        // Picture
        let pic = CompValue::Picture(PictureRef {
            media_key: "sha256:abc".to_string(),
            natural: SizeEmu::new(640, 480),
        });
        assert_eq!(pic.kind(), CompKind::Picture);
        assert_eq!(w.set(e, pic.clone()), None);
        assert_eq!(w.get(e, CompKind::Picture), Some(pic));

        // Timeline
        let timeline = CompValue::Timeline(SlideTimeline {
            steps: vec![AnimStep {
                trigger: Trigger::AfterPrev { delay: 250 },
                anims: vec![Anim {
                    target: e,
                    kind: AnimKind::Entrance(Effect::Fade),
                    duration: 500,
                    delay: 0,
                    easing: Easing::EaseInOut,
                }],
            }],
        });
        assert_eq!(timeline.kind(), CompKind::Timeline);
        assert_eq!(w.set(e, timeline.clone()), None);
        assert_eq!(w.get(e, CompKind::Timeline), Some(timeline));

        // Transition
        let transition = CompValue::Transition(Transition {
            kind: TransitionKind::Push,
            duration: 300,
        });
        assert_eq!(transition.kind(), CompKind::Transition);
        assert_eq!(w.set(e, transition.clone()), None);
        assert_eq!(w.get(e, CompKind::Transition), Some(transition));

        // Morph key
        let morph = CompValue::MorphKey("logo".to_string());
        assert_eq!(morph.kind(), CompKind::MorphKey);
        assert_eq!(w.set(e, morph.clone()), None);
        assert_eq!(w.get(e, CompKind::MorphKey), Some(morph));
    }

    #[test]
    fn despawn_clears_new_components() {
        use crate::anim::SlideTimeline;
        use crate::geom::SizeEmu;
        use crate::image::PictureRef;

        let mut w = World::new();
        let e = w.spawn();
        w.pictures.insert(
            e,
            PictureRef {
                media_key: "k".to_string(),
                natural: SizeEmu::new(10, 20),
            },
        );
        w.timelines.insert(e, SlideTimeline::default());
        w.morph_keys.insert(e, "logo".to_string());

        w.despawn(e);
        assert!(!w.pictures.contains_key(&e));
        assert!(!w.timelines.contains_key(&e));
        assert!(!w.morph_keys.contains_key(&e));
    }
}
