//! The uniform component-mutation operation (DESIGN 6.10). Every document change is one of
//! four kinds, which keeps undo/serialization/CRDT working on a tiny, closed vocabulary
//! while extensibility happens at the component and command layers.

use hayate_ir::world::{CompKind, CompValue, Entity, World};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Operation {
    /// Insert or replace a component value on an entity.
    SetComponent { entity: Entity, value: CompValue },
    /// Remove a component of the given kind from an entity.
    RemoveComponent { entity: Entity, kind: CompKind },
    /// Bring an entity id to life.
    Spawn { entity: Entity },
    /// Remove an entity and all its components.
    Despawn { entity: Entity },
}

impl Operation {
    /// Apply to the world, returning the inverse as a sequence (applying the sequence in
    /// order reverts this operation). A sequence is needed because despawning an entity
    /// must be reversed by re-spawning it and restoring every component.
    pub fn apply(self, w: &mut World) -> Vec<Operation> {
        match self {
            Operation::SetComponent { entity, value } => {
                let kind = value.kind();
                match w.set(entity, value) {
                    Some(prev) => vec![Operation::SetComponent {
                        entity,
                        value: prev,
                    }],
                    None => vec![Operation::RemoveComponent { entity, kind }],
                }
            }
            Operation::RemoveComponent { entity, kind } => match w.remove(entity, kind) {
                Some(prev) => vec![Operation::SetComponent {
                    entity,
                    value: prev,
                }],
                None => vec![],
            },
            Operation::Spawn { entity } => {
                w.spawn_at(entity);
                vec![Operation::Despawn { entity }]
            }
            Operation::Despawn { entity } => {
                let comps = w.components_of(entity);
                if w.despawn(entity) {
                    let mut inv = Vec::with_capacity(comps.len() + 1);
                    inv.push(Operation::Spawn { entity });
                    for c in comps {
                        inv.push(Operation::SetComponent { entity, value: c });
                    }
                    inv
                } else {
                    vec![]
                }
            }
        }
    }
}
