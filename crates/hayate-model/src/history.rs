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
mod tests;
