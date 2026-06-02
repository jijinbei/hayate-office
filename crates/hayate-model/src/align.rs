//! Alignment and distribution helpers (DESIGN 6.10). Given a set of entities, these build a
//! labelled `Transaction` of `SetComponent(Frame)` operations that reposition each entity's
//! `Frame` relative to the group bounding box (alignment) or to even spacing across an axis
//! (distribution). Sizes are never changed; only frame origins move. Entities without a
//! `Frame` are skipped, and the helpers return an empty transaction when there are too few
//! framed entities to act on.

use crate::history::Transaction;
use crate::op::Operation;
use hayate_ir::geom::RectEmu;
use hayate_ir::world::{CompKind, CompValue, Entity, World};

/// Which edge or center to align to within the group bounding box.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Align {
    /// Align left edges to the group's minimum x.
    Left,
    /// Center horizontally on the group's horizontal center.
    HCenter,
    /// Align right edges to the group's maximum x.
    Right,
    /// Align top edges to the group's minimum y.
    Top,
    /// Center vertically on the group's vertical center.
    VCenter,
    /// Align bottom edges to the group's maximum y.
    Bottom,
}

/// The axis along which entities are distributed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Axis {
    /// Distribute along x (vary origin.x).
    Horizontal,
    /// Distribute along y (vary origin.y).
    Vertical,
}

/// Collect the `(entity, frame)` pairs for the entities that currently have a `Frame`,
/// preserving the input order. Entities without a `Frame` are skipped.
fn framed(world: &World, entities: &[Entity]) -> Vec<(Entity, RectEmu)> {
    entities
        .iter()
        .filter_map(|&e| match world.get(e, CompKind::Frame) {
            Some(CompValue::Frame(frame)) => Some((e, frame)),
            _ => None,
        })
        .collect()
}

/// Build a `SetComponent(Frame)` op for `e` with `frame`.
fn set_frame_op(e: Entity, frame: RectEmu) -> Operation {
    Operation::SetComponent {
        entity: e,
        value: CompValue::Frame(frame),
    }
}

/// Align the framed entities to the group bounding box according to `how`.
///
/// The group bounding box is computed from the entities' current frames. Each entity's frame
/// origin is then moved so the relevant edge or center matches the group box; the size is left
/// unchanged. Entities without a `Frame` are skipped. One `SetComponent(Frame)` is emitted per
/// entity that actually moves. Returns an empty transaction if fewer than two framed entities
/// are present.
pub fn align(world: &World, entities: &[Entity], how: Align) -> Transaction {
    let items = framed(world, entities);
    if items.len() < 2 {
        return Transaction::new("align", vec![]);
    }

    // Group bounding box across all framed entities.
    let min_x = items.iter().map(|(_, f)| f.origin.x).min().unwrap();
    let min_y = items.iter().map(|(_, f)| f.origin.y).min().unwrap();
    let max_right = items.iter().map(|(_, f)| f.right()).max().unwrap();
    let max_bottom = items.iter().map(|(_, f)| f.bottom()).max().unwrap();
    let center_x = (min_x + max_right) / 2;
    let center_y = (min_y + max_bottom) / 2;

    let mut ops = Vec::new();
    for (e, frame) in items {
        let mut moved = frame;
        match how {
            Align::Left => moved.origin.x = min_x,
            Align::Right => moved.origin.x = max_right - frame.size.w,
            Align::HCenter => moved.origin.x = center_x - frame.size.w / 2,
            Align::Top => moved.origin.y = min_y,
            Align::Bottom => moved.origin.y = max_bottom - frame.size.h,
            Align::VCenter => moved.origin.y = center_y - frame.size.h / 2,
        }
        // Only emit an op when the frame actually changes.
        if moved != frame {
            ops.push(set_frame_op(e, moved));
        }
    }

    Transaction::new("align", ops)
}

/// Distribute the framed entities so the gaps between adjacent items along `axis` are equal.
///
/// Entities are sorted by their position on the axis (origin coordinate). The first and last
/// items keep their positions; the items in between are repositioned so that the empty space
/// between consecutive frames is the same everywhere. Sizes are unchanged. Entities without a
/// `Frame` are skipped, and the transaction is empty if fewer than three framed entities are
/// present.
pub fn distribute(world: &World, entities: &[Entity], axis: Axis) -> Transaction {
    let mut items = framed(world, entities);
    if items.len() < 3 {
        return Transaction::new("distribute", vec![]);
    }

    // Project a frame onto the axis as (origin, extent).
    let origin = |f: &RectEmu| match axis {
        Axis::Horizontal => f.origin.x,
        Axis::Vertical => f.origin.y,
    };
    let extent = |f: &RectEmu| match axis {
        Axis::Horizontal => f.size.w,
        Axis::Vertical => f.size.h,
    };

    // Sort by leading edge along the axis.
    items.sort_by_key(|(_, f)| origin(f));

    let n = items.len();
    let first = &items[0].1;
    let last = &items[n - 1].1;

    // The span available for gaps is the distance between the first item's leading edge and the
    // last item's trailing edge, minus the total extent of all items. Distributed across the
    // (n - 1) gaps.
    let total_extent: i64 = items.iter().map(|(_, f)| extent(f)).sum();
    let span = (origin(last) + extent(last)) - origin(first);
    let gap = (span - total_extent) / (n as i64 - 1);

    // Walk left to right placing each item's leading edge after the previous trailing edge plus
    // one gap. The first stays put; the last lands on its original position by construction.
    let mut ops = Vec::new();
    // Cursor tracks where the next item's leading edge should go. It starts just past the
    // (fixed) first item.
    let mut cursor = origin(first) + extent(first) + gap;
    for (idx, (e, frame)) in items.iter().enumerate() {
        let mut moved = *frame;
        if idx == 0 {
            // First item is fixed; the cursor was already seeded past it.
            continue;
        }
        if idx == n - 1 {
            // Last item is fixed; nothing to place.
            continue;
        }
        match axis {
            Axis::Horizontal => moved.origin.x = cursor,
            Axis::Vertical => moved.origin.y = cursor,
        }
        cursor += extent(frame) + gap;
        if moved != *frame {
            ops.push(set_frame_op(*e, moved));
        }
    }

    Transaction::new("distribute", ops)
}

#[cfg(test)]
mod tests;
