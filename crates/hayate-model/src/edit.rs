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
use hayate_ir::anim::{Anim, AnimKind, AnimStep, Easing, Effect, SlideTimeline, Trigger};
use hayate_ir::color::Color;
use hayate_ir::color::ThemeColorToken;
use hayate_ir::font::{FontRef, ThemeFontSlot};
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
                body.paragraphs.push(Paragraph::new(vec![default_run(text)]));
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
    body.paragraphs.push(Paragraph::new(vec![default_run(text)]));
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

    #[test]
    fn resize_keeps_origin_and_undo_restores() {
        let mut w = World::new();
        let mut h = History::new();
        let e = w.spawn();

        let start = RectEmu::new(10, 20, 100, 50);
        h.commit(&mut w, set_frame(e, start));

        // Resize to (300, 400): size changes, origin unchanged.
        let tx = resize(&w, e, 300, 400);
        h.commit(&mut w, tx);
        let resized = RectEmu::new(10, 20, 300, 400);
        assert_eq!(w.get(e, CompKind::Frame), Some(CompValue::Frame(resized)));

        // Undo restores the prior frame.
        assert!(h.undo(&mut w));
        assert_eq!(w.get(e, CompKind::Frame), Some(CompValue::Frame(start)));
    }

    #[test]
    fn resize_without_frame_is_empty() {
        let mut w = World::new();
        let e = w.spawn();
        let tx = resize(&w, e, 10, 10);
        assert!(tx.ops.is_empty());
    }

    #[test]
    fn set_rotation_and_undo() {
        let mut w = World::new();
        let mut h = History::new();
        let e = w.spawn();

        // No rotation initially.
        assert_eq!(w.get(e, CompKind::Rotation), None);

        h.commit(&mut w, set_rotation(e, 45.0));
        assert_eq!(
            w.get(e, CompKind::Rotation),
            Some(CompValue::Rotation(45.0))
        );

        // Undo removes the (previously absent) rotation.
        assert!(h.undo(&mut w));
        assert_eq!(w.get(e, CompKind::Rotation), None);

        // Setting over an existing rotation: undo restores the prior value.
        h.commit(&mut w, set_rotation(e, 10.0));
        h.commit(&mut w, set_rotation(e, 90.0));
        assert_eq!(
            w.get(e, CompKind::Rotation),
            Some(CompValue::Rotation(90.0))
        );
        assert!(h.undo(&mut w));
        assert_eq!(
            w.get(e, CompKind::Rotation),
            Some(CompValue::Rotation(10.0))
        );
    }

    #[test]
    fn duplicate_copies_components_with_offset_frame() {
        let mut w = World::new();
        let mut h = History::new();

        // Source with Frame + Fill + Geometry.
        let src = w.spawn();
        let frame = RectEmu::new(100, 200, 300, 400);
        w.set(src, CompValue::Frame(frame));
        w.set(src, CompValue::Fill(red()));
        w.set(src, CompValue::Geometry(Geometry::Rect));

        let new_id = w.reserve_id();
        let tx = duplicate(&w, src, new_id);
        h.commit(&mut w, tx);

        // New entity is alive with copied Fill / Geometry.
        assert!(w.is_alive(new_id));
        assert_eq!(w.get(new_id, CompKind::Fill), Some(CompValue::Fill(red())));
        assert_eq!(
            w.get(new_id, CompKind::Geometry),
            Some(CompValue::Geometry(Geometry::Rect))
        );

        // Frame is offset by (+0.2 inch, +0.2 inch); size unchanged.
        let expected = RectEmu::new(
            100 + DUPLICATE_OFFSET_EMU,
            200 + DUPLICATE_OFFSET_EMU,
            300,
            400,
        );
        assert_eq!(
            w.get(new_id, CompKind::Frame),
            Some(CompValue::Frame(expected))
        );

        // Source is unchanged.
        assert_eq!(w.get(src, CompKind::Frame), Some(CompValue::Frame(frame)));

        // Undo removes the duplicate entirely.
        assert!(h.undo(&mut w));
        assert!(!w.is_alive(new_id));
        assert_eq!(w.get(new_id, CompKind::Frame), None);
    }

    use hayate_ir::color::ThemeColorToken;
    use hayate_ir::font::{FontRef, ThemeFontSlot};
    use hayate_ir::text::{Paragraph, Run, TextBody};
    use hayate_ir::units::pt;

    fn styled_run(text: &str) -> Run {
        Run {
            text: text.to_string(),
            font: FontRef::Theme(ThemeFontSlot::Major),
            size: pt(44),
            color: Color::theme(ThemeColorToken::Accent1),
            bold: true,
            italic: false,
            underline: false,
        }
    }

    #[test]
    fn set_run_text_preserves_formatting() {
        let mut w = World::new();
        let mut h = History::new();
        let e = w.spawn();

        // Existing body with a distinctly-styled first run.
        let original = styled_run("hello");
        let body = TextBody {
            paragraphs: vec![Paragraph::new(vec![original.clone()])],
            autofit: false,
        };
        w.set(e, CompValue::Text(body));

        let tx = set_run_text(&w, e, "world".to_string());
        h.commit(&mut w, tx);

        let got = w.texts.get(&e).expect("text present");
        let run = &got.paragraphs[0].runs[0];
        // Text changed.
        assert_eq!(run.text, "world");
        // Formatting preserved.
        assert_eq!(run.size, original.size);
        assert_eq!(run.color, original.color);
        assert_eq!(run.font, original.font);
        assert!(run.bold);

        // Undoable: restores the prior text.
        assert!(h.undo(&mut w));
        let restored = w.texts.get(&e).expect("text present");
        assert_eq!(restored.paragraphs[0].runs[0].text, "hello");
    }

    #[test]
    fn set_run_text_creates_body_when_absent() {
        let mut w = World::new();
        let mut h = History::new();
        let e = w.spawn();

        // No TextBody initially.
        assert!(w.texts.get(&e).is_none());

        let tx = set_run_text(&w, e, "fresh".to_string());
        h.commit(&mut w, tx);

        let got = w.texts.get(&e).expect("text created");
        assert_eq!(got.paragraphs.len(), 1);
        let run = &got.paragraphs[0].runs[0];
        assert_eq!(run.text, "fresh");
        // Default run formatting.
        assert_eq!(run.font, FontRef::Theme(ThemeFontSlot::Minor));
        assert_eq!(run.size, pt(18));
        assert_eq!(run.color, Color::theme(ThemeColorToken::Dk1));

        // Undo removes the (previously absent) body.
        assert!(h.undo(&mut w));
        assert!(w.texts.get(&e).is_none());
    }

    #[test]
    fn append_paragraph_increases_count_and_undoes() {
        let mut w = World::new();
        let mut h = History::new();
        let e = w.spawn();

        // Start with a one-paragraph body.
        let body = TextBody {
            paragraphs: vec![Paragraph::new(vec![styled_run("first")])],
            autofit: false,
        };
        w.set(e, CompValue::Text(body));

        let tx = append_paragraph(&w, e, "second".to_string());
        h.commit(&mut w, tx);

        let got = w.texts.get(&e).expect("text present");
        assert_eq!(got.paragraphs.len(), 2);
        assert_eq!(got.paragraphs[1].runs[0].text, "second");

        // Undo restores the original single paragraph.
        assert!(h.undo(&mut w));
        let restored = w.texts.get(&e).expect("text present");
        assert_eq!(restored.paragraphs.len(), 1);
    }

    #[test]
    fn append_paragraph_creates_body_when_absent() {
        let mut w = World::new();
        let mut h = History::new();
        let e = w.spawn();

        let tx = append_paragraph(&w, e, "only".to_string());
        h.commit(&mut w, tx);

        let got = w.texts.get(&e).expect("text created");
        assert_eq!(got.paragraphs.len(), 1);
        assert_eq!(got.paragraphs[0].runs[0].text, "only");

        assert!(h.undo(&mut w));
        assert!(w.texts.get(&e).is_none());
    }

    /// Build a two-paragraph body (both left-aligned, no bullets) on `e`.
    fn two_paragraph_body(w: &mut World, e: Entity) {
        let body = TextBody {
            paragraphs: vec![
                Paragraph::new(vec![styled_run("first")]),
                Paragraph::new(vec![styled_run("second")]),
            ],
            autofit: false,
        };
        w.set(e, CompValue::Text(body));
    }

    #[test]
    fn set_paragraph_align_changes_all_paragraphs_and_undoes() {
        use hayate_ir::text::HAlign;
        let mut w = World::new();
        let mut h = History::new();
        let e = w.spawn();
        two_paragraph_body(&mut w, e);

        // Both paragraphs start left-aligned (Paragraph::new default).
        let before = w.texts.get(&e).expect("text present");
        assert_eq!(before.paragraphs[0].align, HAlign::Left);
        assert_eq!(before.paragraphs[1].align, HAlign::Left);

        let tx = set_paragraph_align(&w, e, HAlign::Center);
        h.commit(&mut w, tx);

        // Every paragraph is now centered.
        let got = w.texts.get(&e).expect("text present");
        assert_eq!(got.paragraphs[0].align, HAlign::Center);
        assert_eq!(got.paragraphs[1].align, HAlign::Center);

        // Undo restores the prior alignment for all paragraphs.
        assert!(h.undo(&mut w));
        let restored = w.texts.get(&e).expect("text present");
        assert_eq!(restored.paragraphs[0].align, HAlign::Left);
        assert_eq!(restored.paragraphs[1].align, HAlign::Left);
    }

    #[test]
    fn set_paragraph_align_without_body_is_empty() {
        use hayate_ir::text::HAlign;
        let mut w = World::new();
        let e = w.spawn();
        let tx = set_paragraph_align(&w, e, HAlign::Right);
        assert!(tx.ops.is_empty());
    }

    #[test]
    fn toggle_bullets_flips_on_then_off_across_paragraphs() {
        let mut w = World::new();
        let mut h = History::new();
        let e = w.spawn();
        two_paragraph_body(&mut w, e);

        // Initially no bullets on either paragraph.
        let before = w.texts.get(&e).expect("text present");
        assert_eq!(before.paragraphs[0].bullet_level, 0);
        assert_eq!(before.paragraphs[1].bullet_level, 0);

        // First toggle: bullets on (0 -> 1) for all paragraphs.
        let tx = toggle_bullets(&w, e);
        h.commit(&mut w, tx);
        let on = w.texts.get(&e).expect("text present");
        assert_eq!(on.paragraphs[0].bullet_level, 1);
        assert_eq!(on.paragraphs[1].bullet_level, 1);

        // Second toggle: bullets off (1 -> 0) for all paragraphs.
        let tx = toggle_bullets(&w, e);
        h.commit(&mut w, tx);
        let off = w.texts.get(&e).expect("text present");
        assert_eq!(off.paragraphs[0].bullet_level, 0);
        assert_eq!(off.paragraphs[1].bullet_level, 0);

        // Undo the off-toggle: bullets back on.
        assert!(h.undo(&mut w));
        let undone = w.texts.get(&e).expect("text present");
        assert_eq!(undone.paragraphs[0].bullet_level, 1);
        assert_eq!(undone.paragraphs[1].bullet_level, 1);
    }

    #[test]
    fn toggle_bullets_without_body_is_empty() {
        let mut w = World::new();
        let e = w.spawn();
        let tx = toggle_bullets(&w, e);
        assert!(tx.ops.is_empty());
    }

    /// Spawn three sibling entities a, b, c under `parent` with strictly increasing Order
    /// keys, returning them in that order.
    fn three_siblings(w: &mut World, parent: Entity) -> (Entity, Entity, Entity) {
        let a = w.spawn();
        let b = w.spawn();
        let c = w.spawn();
        let oa = FracIndex::after(None);
        let ob = FracIndex::after(Some(&oa));
        let oc = FracIndex::after(Some(&ob));
        for (e, o) in [(a, oa), (b, ob), (c, oc)] {
            w.set(e, CompValue::Parent(parent));
            w.set(e, CompValue::Order(o));
        }
        (a, b, c)
    }

    /// Read the live siblings of `parent` sorted by their Order key, returning entity ids.
    fn sorted_children(w: &World, parent: Entity) -> Vec<Entity> {
        let mut kids: Vec<(Entity, FracIndex)> = w
            .iter()
            .filter(|e| w.parent.get(e).copied() == Some(parent))
            .filter_map(|e| w.order.get(&e).map(|o| (e, o.clone())))
            .collect();
        kids.sort_by(|x, y| x.1.cmp(&y.1));
        kids.into_iter().map(|(e, _)| e).collect()
    }

    #[test]
    fn move_forward_reorders_and_undoes() {
        let mut w = World::new();
        let mut h = History::new();
        let parent = w.spawn();
        let (a, b, c) = three_siblings(&mut w, parent);

        // Initially a, b, c.
        assert_eq!(sorted_children(&w, parent), vec![a, b, c]);

        // Moving a forward places it between b and c.
        let tx = move_forward(&w, a);
        assert!(!tx.ops.is_empty());
        h.commit(&mut w, tx);
        assert_eq!(sorted_children(&w, parent), vec![b, a, c]);

        // Undo restores the original order.
        assert!(h.undo(&mut w));
        assert_eq!(sorted_children(&w, parent), vec![a, b, c]);
    }

    #[test]
    fn move_forward_on_last_is_noop() {
        let mut w = World::new();
        let parent = w.spawn();
        let (_a, _b, c) = three_siblings(&mut w, parent);

        // c is already last: no next sibling, so the transaction is empty.
        let tx = move_forward(&w, c);
        assert!(tx.ops.is_empty());
    }

    #[test]
    fn move_backward_reorders_and_undoes() {
        let mut w = World::new();
        let mut h = History::new();
        let parent = w.spawn();
        let (a, b, c) = three_siblings(&mut w, parent);

        // Moving c backward places it between a and b.
        let tx = move_backward(&w, c);
        assert!(!tx.ops.is_empty());
        h.commit(&mut w, tx);
        assert_eq!(sorted_children(&w, parent), vec![a, c, b]);

        // Undo restores the original order.
        assert!(h.undo(&mut w));
        assert_eq!(sorted_children(&w, parent), vec![a, b, c]);
    }

    #[test]
    fn move_backward_on_first_is_noop() {
        let mut w = World::new();
        let parent = w.spawn();
        let (a, _b, _c) = three_siblings(&mut w, parent);

        // a is already first: no previous sibling, so the transaction is empty.
        let tx = move_backward(&w, a);
        assert!(tx.ops.is_empty());
    }

    #[test]
    fn add_entrance_creates_step_appends_second_and_undoes() {
        let mut w = World::new();
        let mut h = History::new();

        // A slide entity and a shape entity (spawn + frame).
        let slide = w.spawn();
        let shape = w.spawn();
        h.commit(&mut w, set_frame(shape, RectEmu::new(0, 0, 100, 50)));

        // No timeline initially.
        assert!(w.timelines.get(&slide).is_none());

        // First entrance: a fresh one-step timeline targeting the shape.
        let tx = add_entrance(&w, slide, shape, Effect::Fade, 500);
        h.commit(&mut w, tx);
        let tl = w.timelines.get(&slide).expect("timeline created");
        assert_eq!(tl.steps.len(), 1);
        let anim = &tl.steps[0].anims[0];
        assert_eq!(anim.target, shape);
        assert_eq!(anim.kind, AnimKind::Entrance(Effect::Fade));
        assert_eq!(anim.duration, 500);

        // Second entrance: appends a second step (does not replace the first).
        let tx = add_entrance(&w, slide, shape, Effect::Zoom, 250);
        h.commit(&mut w, tx);
        let tl = w.timelines.get(&slide).expect("timeline present");
        assert_eq!(tl.steps.len(), 2);
        assert_eq!(tl.steps[1].anims[0].kind, AnimKind::Entrance(Effect::Zoom));
        assert_eq!(tl.steps[1].anims[0].duration, 250);

        // Undo the second add: back to a single step.
        assert!(h.undo(&mut w));
        let tl = w.timelines.get(&slide).expect("timeline present");
        assert_eq!(tl.steps.len(), 1);
        assert_eq!(tl.steps[0].anims[0].kind, AnimKind::Entrance(Effect::Fade));

        // Undo the first add: the (previously absent) timeline is removed.
        assert!(h.undo(&mut w));
        assert!(w.timelines.get(&slide).is_none());
    }
}
