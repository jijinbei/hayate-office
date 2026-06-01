//! Transactions and the undo/redo history (DESIGN 6.10). One command produces one
//! transaction = one undo step.

use crate::op::Operation;
use hayate_ir::world::World;
use serde::{Deserialize, Serialize};

/// A group of operations applied and undone as a unit.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Transaction {
    pub label: String,
    pub ops: Vec<Operation>,
}

impl Transaction {
    pub fn new(label: impl Into<String>, ops: Vec<Operation>) -> Self {
        Self {
            label: label.into(),
            ops,
        }
    }

    /// Apply every operation in order and return the inverse transaction. Reverting a
    /// sequence means applying each op's inverse in reverse order.
    pub fn apply(self, w: &mut World) -> Transaction {
        let label = self.label;
        let mut groups: Vec<Vec<Operation>> = Vec::with_capacity(self.ops.len());
        for op in self.ops {
            groups.push(op.apply(w));
        }
        let ops = groups.into_iter().rev().flatten().collect();
        Transaction { label, ops }
    }
}

/// Undo/redo stacks of inverse transactions.
#[derive(Default)]
pub struct History {
    undo: Vec<Transaction>,
    redo: Vec<Transaction>,
}

impl History {
    pub fn new() -> Self {
        Self::default()
    }

    /// Apply a new transaction and record it for undo; clears the redo stack.
    pub fn commit(&mut self, w: &mut World, tx: Transaction) {
        let inverse = tx.apply(w);
        self.undo.push(inverse);
        self.redo.clear();
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    /// Revert the most recent transaction. Returns whether anything was undone.
    pub fn undo(&mut self, w: &mut World) -> bool {
        if let Some(tx) = self.undo.pop() {
            let redo = tx.apply(w);
            self.redo.push(redo);
            true
        } else {
            false
        }
    }

    /// Re-apply the most recently undone transaction. Returns whether anything was redone.
    pub fn redo(&mut self, w: &mut World) -> bool {
        if let Some(tx) = self.redo.pop() {
            let undo = tx.apply(w);
            self.undo.push(undo);
            true
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
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
            Transaction::new("set frame", vec![Operation::SetComponent { entity: e, value: frame.clone() }]),
        );
        assert_eq!(w.get(e, CompKind::Frame), Some(frame.clone()));

        assert!(h.undo(&mut w));
        assert_eq!(w.get(e, CompKind::Frame), None, "undo of first set removes it");

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
            Transaction::new("move", vec![Operation::SetComponent { entity: e, value: b.clone() }]),
        );
        assert_eq!(w.get(e, CompKind::Frame), Some(b));
        assert!(h.undo(&mut w));
        assert_eq!(w.get(e, CompKind::Frame), Some(a), "undo restores prior frame");
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
                    Operation::SetComponent { entity: e, value: frame.clone() },
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
}
