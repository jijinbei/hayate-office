//! Ergonomic editing helpers (DESIGN 6.10). Each helper builds a labelled `Transaction`
//! out of the uniform four-kind `Operation`, so callers express intent ("create a rect",
//! "translate") without hand-assembling `SetComponent`/`Spawn` vocabulary, while undo,
//! serialization and (later) CRDT keep operating on that closed vocabulary.

use crate::history::Transaction;
use crate::op::Operation;
use hayate_ir::anim::{Anim, AnimKind, AnimStep, Easing, Effect, SlideTimeline, Trigger};
use hayate_ir::color::Color;
use hayate_ir::color::ThemeColorToken;
use hayate_ir::font::{FontRef, ThemeFontSlot};
use hayate_ir::frac::FracIndex;
use hayate_ir::geom::RectEmu;
use hayate_ir::paint::Fill;
use hayate_ir::shape::Geometry;
use hayate_ir::text::{HAlign, Paragraph, Run, TextBody};
use hayate_ir::units::pt;
use hayate_ir::world::{CompKind, CompValue, Entity, World};

/// A minimal default run carrying `text`: body theme font, 18pt, Dk1 theme color, no
/// bold/italic/underline. Used when a text helper must synthesize a run because none exists.
fn default_run(text: String) -> Run {
    Run {
        text,
        font: FontRef::Theme(ThemeFontSlot::Minor),
        size: pt(18),
        color: Color::theme(ThemeColorToken::Dk1),
        bold: false,
        italic: false,
        underline: false,
    }
}

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

/// EMU per inch (914400) times 0.2, the offset applied to a duplicate's frame so it does
/// not sit exactly on top of its source.
const DUPLICATE_OFFSET_EMU: i64 = 182880; // 0.2 inch

/// Resize `e`'s `Frame` to `(w, h)`, keeping its origin. Reads the current frame from
/// `world`; if `e` has no `Frame`, returns an empty transaction (nothing to resize).
pub fn resize(world: &World, e: Entity, w: i64, h: i64) -> Transaction {
    match world.get(e, CompKind::Frame) {
        Some(CompValue::Frame(mut frame)) => {
            frame.size.w = w;
            frame.size.h = h;
            Transaction::new(
                "resize",
                vec![Operation::SetComponent {
                    entity: e,
                    value: CompValue::Frame(frame),
                }],
            )
        }
        // No frame (or some other component kind): nothing to resize.
        _ => Transaction::new("resize", vec![]),
    }
}

/// Set (insert or replace) the `Rotation` of `e`, in degrees clockwise.
pub fn set_rotation(e: Entity, degrees: f32) -> Transaction {
    Transaction::new(
        "set rotation",
        vec![Operation::SetComponent {
            entity: e,
            value: CompValue::Rotation(degrees),
        }],
    )
}

/// Duplicate `src` onto the reserved id `new_id`: spawn the new entity, then copy every
/// component currently on `src`. If the duplicate has a `Frame`, its origin is offset by
/// (+0.2 inch, +0.2 inch) so the copy is visible rather than hidden behind the original.
/// `new_id` must come from [`World::reserve_id`] so the spawn is fully captured by this
/// (undoable) transaction.
pub fn duplicate(world: &World, src: Entity, new_id: Entity) -> Transaction {
    let mut ops = vec![Operation::Spawn { entity: new_id }];
    for comp in world.components_of(src) {
        let value = match comp {
            CompValue::Frame(mut frame) => {
                frame.origin.x += DUPLICATE_OFFSET_EMU;
                frame.origin.y += DUPLICATE_OFFSET_EMU;
                CompValue::Frame(frame)
            }
            other => other,
        };
        ops.push(Operation::SetComponent {
            entity: new_id,
            value,
        });
    }
    Transaction::new("duplicate", ops)
}

/// Set the text of the entity's first run, preserving that run's formatting. Reads the
/// current `TextBody` from `world`: if present, clones it and replaces the first paragraph's
/// first run's `text` (keeping its font/size/color/bold/...). If the body exists but has no
/// paragraphs/runs, a minimal default-run paragraph carrying `text` is inserted. If the
/// entity has no `TextBody` at all, a new one is created. Always emits `SetComponent(Text)`.
pub fn set_run_text(world: &World, e: Entity, text: String) -> Transaction {
    let new_body = match world.texts.get(&e) {
        Some(existing) => {
            let mut body = existing.clone();
            if let Some(para) = body.paragraphs.first_mut() {
                if let Some(run) = para.runs.first_mut() {
                    // Preserve formatting; replace only the text.
                    run.text = text;
                } else {
                    // Paragraph with no runs: give it a minimal default run.
                    para.runs.push(default_run(text));
                }
            } else {
                // Body with no paragraphs: create one minimal paragraph.
                body.paragraphs
                    .push(Paragraph::new(vec![default_run(text)]));
            }
            body
        }
        // No TextBody at all: create a fresh one.
        None => TextBody {
            paragraphs: vec![Paragraph::new(vec![default_run(text)])],
            autofit: false,
        },
    };
    Transaction::new(
        "set run text",
        vec![Operation::SetComponent {
            entity: e,
            value: CompValue::Text(new_body),
        }],
    )
}

/// Append a new paragraph (a single minimal default run carrying `text`) to the entity's
/// `TextBody`, creating an empty body first if the entity has none. Emits
/// `SetComponent(Text)`.
pub fn append_paragraph(world: &World, e: Entity, text: String) -> Transaction {
    let mut body = match world.texts.get(&e) {
        Some(existing) => existing.clone(),
        None => TextBody {
            paragraphs: Vec::new(),
            autofit: false,
        },
    };
    body.paragraphs
        .push(Paragraph::new(vec![default_run(text)]));
    Transaction::new(
        "append paragraph",
        vec![Operation::SetComponent {
            entity: e,
            value: CompValue::Text(body),
        }],
    )
}

/// Set the horizontal alignment of *every* paragraph in the entity's `TextBody` to `align`.
/// Reads the current body from `world`; if the entity has no `TextBody`, returns an empty
/// transaction (nothing to align). Otherwise emits `SetComponent(Text)`.
pub fn set_paragraph_align(world: &World, e: Entity, align: HAlign) -> Transaction {
    let mut body = match world.texts.get(&e) {
        Some(existing) => existing.clone(),
        None => return Transaction::new("set paragraph align", vec![]),
    };
    for para in &mut body.paragraphs {
        para.align = align;
    }
    Transaction::new(
        "set paragraph align",
        vec![Operation::SetComponent {
            entity: e,
            value: CompValue::Text(body),
        }],
    )
}

/// Toggle bullets across all paragraphs of the entity's `TextBody`. If any paragraph
/// currently has `bullet_level == 0` (i.e. bullets are not uniformly on), turn bullets on by
/// setting every paragraph's `bullet_level` to 1; otherwise turn them all off (set to 0).
/// Reads the current body from `world`; if the entity has no `TextBody`, returns an empty
/// transaction. Otherwise emits `SetComponent(Text)`.
pub fn toggle_bullets(world: &World, e: Entity) -> Transaction {
    let mut body = match world.texts.get(&e) {
        Some(existing) => existing.clone(),
        None => return Transaction::new("toggle bullets", vec![]),
    };
    // Any paragraph without a bullet means "not fully bulleted": turn bullets on.
    let turn_on = body.paragraphs.iter().any(|p| p.bullet_level == 0);
    let new_level = if turn_on { 1 } else { 0 };
    for para in &mut body.paragraphs {
        para.bullet_level = new_level;
    }
    Transaction::new(
        "toggle bullets",
        vec![Operation::SetComponent {
            entity: e,
            value: CompValue::Text(body),
        }],
    )
}

/// Collect `e`'s siblings (entities sharing `e`'s parent; a `None` parent means root-level
/// siblings) paired with their `Order` key, sorted ascending by that key. `e` itself is
/// excluded, and siblings without an `Order` are skipped (they cannot be positioned
/// relative to `e`). The parent of an entity is read from `world.parent`.
fn ordered_siblings(world: &World, e: Entity) -> Vec<(Entity, FracIndex)> {
    let target_parent = world.parent.get(&e).copied();
    let mut siblings: Vec<(Entity, FracIndex)> = world
        .iter()
        .filter(|&other| other != e)
        .filter(|other| world.parent.get(other).copied() == target_parent)
        .filter_map(|other| world.order.get(&other).map(|o| (other, o.clone())))
        .collect();
    siblings.sort_by(|a, b| a.1.cmp(&b.1));
    siblings
}

/// Read `e`'s own `Order` key, if any.
fn order_of(world: &World, e: Entity) -> Option<FracIndex> {
    world.order.get(&e).cloned()
}

/// Build a transaction that sets `e`'s `Order` to `order`.
fn set_order_tx(label: &str, e: Entity, order: FracIndex) -> Transaction {
    Transaction::new(
        label.to_string(),
        vec![Operation::SetComponent {
            entity: e,
            value: CompValue::Order(order),
        }],
    )
}

/// Move `e` one step forward (toward the front / later in sibling order) among its siblings.
/// Siblings are those sharing `e`'s parent (a `None` parent means root-level), sorted by
/// their `Order` key. If `e` has a next sibling `n` and a sibling-after-next `n2`, `e`'s new
/// `Order` is placed strictly between them; if `n` is the last sibling, `e`'s new `Order` is
/// placed after `n`. If `e` is already last (no next sibling) or has no `Order`, this is a
/// no-op (empty transaction).
pub fn move_forward(world: &World, e: Entity) -> Transaction {
    let empty = Transaction::new("move forward", vec![]);
    let my_order = match order_of(world, e) {
        Some(o) => o,
        None => return empty,
    };
    let siblings = ordered_siblings(world, e);
    // Index of the first sibling whose order is strictly after `e`'s order: that is the
    // "next" sibling in front of `e`.
    let next_pos = siblings.iter().position(|(_, o)| *o > my_order);
    let next_pos = match next_pos {
        Some(p) => p,
        None => return empty, // Already last: nothing in front.
    };
    let n_order = &siblings[next_pos].1;
    let new_order = match siblings.get(next_pos + 1) {
        Some((_, n2_order)) => FracIndex::between(Some(n_order), Some(n2_order)),
        None => FracIndex::after(Some(n_order)),
    };
    set_order_tx("move forward", e, new_order)
}

/// Move `e` one step backward (toward the back / earlier in sibling order) among its
/// siblings. Symmetric to [`move_forward`]: if `e` has a previous sibling `p` and a
/// sibling-before-that `p2`, `e`'s new `Order` is placed strictly between them; if `p` is the
/// first sibling, `e`'s new `Order` is placed before `p`. If `e` is already first (no
/// previous sibling) or has no `Order`, this is a no-op (empty transaction).
pub fn move_backward(world: &World, e: Entity) -> Transaction {
    let empty = Transaction::new("move backward", vec![]);
    let my_order = match order_of(world, e) {
        Some(o) => o,
        None => return empty,
    };
    let siblings = ordered_siblings(world, e);
    // Index of the last sibling whose order is strictly before `e`'s order: that is the
    // "previous" sibling behind `e`.
    let prev_pos = siblings.iter().rposition(|(_, o)| *o < my_order);
    let prev_pos = match prev_pos {
        Some(p) => p,
        None => return empty, // Already first: nothing behind.
    };
    let p_order = &siblings[prev_pos].1;
    let new_order = if prev_pos == 0 {
        FracIndex::before(Some(p_order))
    } else {
        let p2_order = &siblings[prev_pos - 1].1;
        FracIndex::between(Some(p2_order), Some(p_order))
    };
    set_order_tx("move backward", e, new_order)
}

/// Add an entrance animation for `target` to `slide`'s animation timeline. Reads the slide's
/// existing `SlideTimeline` from `world` (or starts an empty one) and appends a new
/// `AnimStep` that fires after the previous step (no delay) and runs a single
/// `Entrance(effect)` anim on `target` for `duration_ms` milliseconds with `EaseOut`. The new
/// timeline is written back via `SetComponent(Timeline)` on the SLIDE entity, so undo restores
/// the prior timeline (or removes it if the slide had none).
pub fn add_entrance(
    world: &World,
    slide: Entity,
    target: Entity,
    effect: Effect,
    duration_ms: u32,
) -> Transaction {
    let mut timeline = match world.timelines.get(&slide) {
        Some(existing) => existing.clone(),
        None => SlideTimeline::default(),
    };
    timeline.steps.push(AnimStep {
        trigger: Trigger::AfterPrev { delay: 0 },
        anims: vec![Anim {
            target,
            kind: AnimKind::Entrance(effect),
            duration: duration_ms,
            delay: 0,
            easing: Easing::EaseOut,
        }],
    });
    Transaction::new(
        "add entrance",
        vec![Operation::SetComponent {
            entity: slide,
            value: CompValue::Timeline(timeline),
        }],
    )
}

/// Assign group membership `key` to every entity in `members`, so they select, move, and
/// delete as a unit. Callers mint a fresh nonzero `key` per group.
pub fn group(members: &[Entity], key: u64) -> Transaction {
    let ops = members
        .iter()
        .map(|&entity| Operation::SetComponent {
            entity,
            value: CompValue::Group(key),
        })
        .collect();
    Transaction::new("group", ops)
}

/// Remove group membership `key` from every entity that currently carries it.
pub fn ungroup(world: &World, key: u64) -> Transaction {
    let ops = world
        .iter()
        .filter(|e| world.groups.get(e) == Some(&key))
        .map(|entity| Operation::RemoveComponent {
            entity,
            kind: CompKind::Group,
        })
        .collect();
    Transaction::new("ungroup", ops)
}

/// All entities sharing the same group as `e` (including `e`); just `[e]` if it is ungrouped.
pub fn group_members(world: &World, e: Entity) -> Vec<Entity> {
    match world.groups.get(&e) {
        Some(&key) => world
            .iter()
            .filter(|x| world.groups.get(x) == Some(&key))
            .collect(),
        None => vec![e],
    }
}

#[cfg(test)]
mod tests;
