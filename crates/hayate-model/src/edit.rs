//! Ergonomic editing helpers (DESIGN 6.10). Each helper builds a labelled `Transaction`
//! out of the uniform four-kind `Operation`, so callers express intent ("create a rect",
//! "translate") without hand-assembling `SetComponent`/`Spawn` vocabulary, while undo,
//! serialization and (later) CRDT keep operating on that closed vocabulary.

use crate::history::Transaction;
use crate::op::Operation;
use hayate_ir::frac::FracIndex;
use hayate_ir::geom::RectEmu;
use hayate_ir::paint::Fill;
use hayate_ir::shape::Geometry;
use hayate_ir::world::{CompKind, CompValue, Entity, World};

/// Set (insert or replace) the `Frame` of `e`.
pub fn set_frame(e: Entity, frame: RectEmu) -> Transaction {
    Transaction::new(
        "set frame",
        vec![Operation::SetComponent {
            entity: e,
            value: CompValue::Frame(frame),
        }],
    )
}

/// Set (insert or replace) the `Fill` of `e`.
pub fn set_fill(e: Entity, fill: Fill) -> Transaction {
    Transaction::new(
        "set fill",
        vec![Operation::SetComponent {
            entity: e,
            value: CompValue::Fill(fill),
        }],
    )
}

/// Shift `e`'s `Frame` origin by `(dx, dy)`, keeping its size. Reads the current frame from
/// `world`; if `e` has no `Frame`, returns an empty transaction (nothing to move).
pub fn translate(world: &World, e: Entity, dx: i64, dy: i64) -> Transaction {
    match world.get(e, CompKind::Frame) {
        Some(CompValue::Frame(mut frame)) => {
            frame.origin.x += dx;
            frame.origin.y += dy;
            Transaction::new(
                "translate",
                vec![Operation::SetComponent {
                    entity: e,
                    value: CompValue::Frame(frame),
                }],
            )
        }
        // No frame (or some other component kind): nothing to translate.
        _ => Transaction::new("translate", vec![]),
    }
}

/// Create a rectangle shape on a reserved id: spawn it, then attach parent, sibling order,
/// frame, rectangular geometry and fill. `reserved` should come from [`World::reserve_id`]
/// so the spawn is fully captured by this (undoable) transaction.
pub fn create_rect(
    reserved: Entity,
    parent: Entity,
    order: FracIndex,
    frame: RectEmu,
    fill: Fill,
) -> Transaction {
    Transaction::new(
        "create rect",
        vec![
            Operation::Spawn { entity: reserved },
            Operation::SetComponent {
                entity: reserved,
                value: CompValue::Parent(parent),
            },
            Operation::SetComponent {
                entity: reserved,
                value: CompValue::Order(order),
            },
            Operation::SetComponent {
                entity: reserved,
                value: CompValue::Frame(frame),
            },
            Operation::SetComponent {
                entity: reserved,
                value: CompValue::Geometry(Geometry::Rect),
            },
            Operation::SetComponent {
                entity: reserved,
                value: CompValue::Fill(fill),
            },
        ],
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::history::History;
    use hayate_ir::color::{Color, Rgba};

    fn red() -> Fill {
        Fill::Solid(Color::Literal(Rgba::rgb(255, 0, 0)))
    }

    #[test]
    fn create_rect_commit_undo_redo() {
        let mut w = World::new();
        let mut h = History::new();

        let parent = w.spawn();
        let e = w.reserve_id();
        let order = FracIndex::after(None);
        let frame = RectEmu::new(10, 20, 100, 50);
        let fill = red();

        h.commit(
            &mut w,
            create_rect(e, parent, order.clone(), frame, fill),
        );

        // Alive with every component the helper set.
        assert!(w.is_alive(e));
        assert_eq!(w.get(e, CompKind::Frame), Some(CompValue::Frame(frame)));
        assert_eq!(
            w.get(e, CompKind::Geometry),
            Some(CompValue::Geometry(Geometry::Rect))
        );
        assert_eq!(w.get(e, CompKind::Fill), Some(CompValue::Fill(fill)));
        assert_eq!(w.get(e, CompKind::Parent), Some(CompValue::Parent(parent)));
        assert_eq!(w.get(e, CompKind::Order), Some(CompValue::Order(order.clone())));

        // Undo removes the entity entirely.
        assert!(h.undo(&mut w));
        assert!(!w.is_alive(e));
        assert_eq!(w.get(e, CompKind::Frame), None);

        // Redo restores the same id and all components.
        assert!(h.redo(&mut w));
        assert!(w.is_alive(e));
        assert_eq!(w.get(e, CompKind::Frame), Some(CompValue::Frame(frame)));
        assert_eq!(
            w.get(e, CompKind::Geometry),
            Some(CompValue::Geometry(Geometry::Rect))
        );
        assert_eq!(w.get(e, CompKind::Fill), Some(CompValue::Fill(fill)));
        assert_eq!(w.get(e, CompKind::Parent), Some(CompValue::Parent(parent)));
        assert_eq!(w.get(e, CompKind::Order), Some(CompValue::Order(order)));
    }

    #[test]
    fn set_frame_then_translate() {
        let mut w = World::new();
        let mut h = History::new();
        let e = w.spawn();

        let start = RectEmu::new(0, 0, 100, 100);
        h.commit(&mut w, set_frame(e, start));
        assert_eq!(w.get(e, CompKind::Frame), Some(CompValue::Frame(start)));

        // Translate by (30, -10): origin shifts, size unchanged.
        let tx = translate(&w, e, 30, -10);
        h.commit(&mut w, tx);
        let moved = RectEmu::new(30, -10, 100, 100);
        assert_eq!(w.get(e, CompKind::Frame), Some(CompValue::Frame(moved)));

        // Undo restores the prior frame.
        assert!(h.undo(&mut w));
        assert_eq!(w.get(e, CompKind::Frame), Some(CompValue::Frame(start)));
    }

    #[test]
    fn translate_without_frame_is_empty() {
        let mut w = World::new();
        let e = w.spawn();
        let tx = translate(&w, e, 5, 5);
        assert!(tx.ops.is_empty());
    }
}
