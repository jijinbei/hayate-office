//! Unit tests for the parent module.

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

    h.commit(&mut w, create_rect(e, parent, order.clone(), frame, fill));

    // Alive with every component the helper set.
    assert!(w.is_alive(e));
    assert_eq!(w.get(e, CompKind::Frame), Some(CompValue::Frame(frame)));
    assert_eq!(
        w.get(e, CompKind::Geometry),
        Some(CompValue::Geometry(Geometry::Rect))
    );
    assert_eq!(w.get(e, CompKind::Fill), Some(CompValue::Fill(fill)));
    assert_eq!(w.get(e, CompKind::Parent), Some(CompValue::Parent(parent)));
    assert_eq!(
        w.get(e, CompKind::Order),
        Some(CompValue::Order(order.clone()))
    );

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
