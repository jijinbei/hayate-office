//! Unit tests for the parent module.

use super::*;
use hayate_ir::geom::RectEmu;
use hayate_ir::world::{CompKind, CompValue, World};

#[test]
fn set_undo_redo() {
    let mut w = World::new();
    let e = w.spawn();
    let mut h = History::new();

    let frame = CompValue::Frame(RectEmu::new(0, 0, 100, 100));
    h.commit(
        &mut w,
        Transaction::new(
            "set frame",
            vec![Operation::SetComponent {
                entity: e,
                value: frame.clone(),
            }],
        ),
    );
    assert_eq!(w.get(e, CompKind::Frame), Some(frame.clone()));

    assert!(h.undo(&mut w));
    assert_eq!(
        w.get(e, CompKind::Frame),
        None,
        "undo of first set removes it"
    );

    assert!(h.redo(&mut w));
    assert_eq!(w.get(e, CompKind::Frame), Some(frame));
}

#[test]
fn move_undo_restores_previous_value() {
    let mut w = World::new();
    let e = w.spawn();
    let a = CompValue::Frame(RectEmu::new(0, 0, 10, 10));
    let b = CompValue::Frame(RectEmu::new(50, 50, 10, 10));
    w.set(e, a.clone());
    let mut h = History::new();

    h.commit(
        &mut w,
        Transaction::new(
            "move",
            vec![Operation::SetComponent {
                entity: e,
                value: b.clone(),
            }],
        ),
    );
    assert_eq!(w.get(e, CompKind::Frame), Some(b));
    assert!(h.undo(&mut w));
    assert_eq!(
        w.get(e, CompKind::Frame),
        Some(a),
        "undo restores prior frame"
    );
}

#[test]
fn spawn_with_components_roundtrips() {
    let mut w = World::new();
    let mut h = History::new();

    // Reserve an id and create a shape (spawn + set frame) as one transaction.
    let e = w.reserve_id();
    let frame = CompValue::Frame(RectEmu::new(1, 2, 3, 4));
    h.commit(
        &mut w,
        Transaction::new(
            "add shape",
            vec![
                Operation::Spawn { entity: e },
                Operation::SetComponent {
                    entity: e,
                    value: frame.clone(),
                },
            ],
        ),
    );
    assert!(w.is_alive(e));
    assert_eq!(w.get(e, CompKind::Frame), Some(frame.clone()));

    // Undo removes the entity entirely.
    assert!(h.undo(&mut w));
    assert!(!w.is_alive(e));
    assert_eq!(w.get(e, CompKind::Frame), None);

    // Redo recreates the same id and its component.
    assert!(h.redo(&mut w));
    assert!(w.is_alive(e));
    assert_eq!(w.get(e, CompKind::Frame), Some(frame));
}
