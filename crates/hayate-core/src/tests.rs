//! Unit tests for the built-in command registry.
use super::*;
use hayate_ir::geom::RectEmu;
use hayate_ir::world::{CompKind, CompValue};
use hayate_model::History;

/// Build a world with a single entity carrying a Frame; return (world, entity).
fn world_with_framed_shape() -> (World, Entity) {
    let mut w = World::new();
    let e = w.spawn();
    w.set(e, CompValue::Frame(RectEmu::new(10, 20, 100, 50)));
    (w, e)
}

#[test]
fn move_command_shifts_frame() {
    let (mut w, e) = world_with_framed_shape();
    let reg = builtins();
    let mut h = History::new();

    let tx = reg
        .build(
            "shape.move",
            &json!({ "entity": e.0, "dx": 100, "dy": 0 }),
            &w,
        )
        .expect("shape.move is registered");
    assert_eq!(tx.label, "Move Shape");
    h.commit(&mut w, tx);

    assert_eq!(
        w.get(e, CompKind::Frame),
        Some(CompValue::Frame(RectEmu::new(110, 20, 100, 50))),
        "origin shifted by dx, size unchanged"
    );

    // And it is undoable as one step.
    assert!(h.undo(&mut w));
    assert_eq!(
        w.get(e, CompKind::Frame),
        Some(CompValue::Frame(RectEmu::new(10, 20, 100, 50)))
    );
}

#[test]
fn delete_command_removes_entity() {
    let (mut w, e) = world_with_framed_shape();
    let reg = builtins();
    let mut h = History::new();

    let tx = reg
        .build("shape.delete", &json!({ "entity": e.0 }), &w)
        .expect("shape.delete is registered");
    h.commit(&mut w, tx);

    assert!(!w.is_alive(e), "entity is gone after delete");

    // Undo restores it.
    assert!(h.undo(&mut w));
    assert!(w.is_alive(e));
    assert_eq!(
        w.get(e, CompKind::Frame),
        Some(CompValue::Frame(RectEmu::new(10, 20, 100, 50)))
    );
}

#[test]
fn set_fill_command_applies_color() {
    let (mut w, e) = world_with_framed_shape();
    let reg = builtins();
    let mut h = History::new();

    // Object form.
    let tx = reg
        .build(
            "shape.set_fill",
            &json!({ "entity": e.0, "color": { "r": 255, "g": 0, "b": 0 } }),
            &w,
        )
        .unwrap();
    h.commit(&mut w, tx);
    assert_eq!(
        w.get(e, CompKind::Fill),
        Some(CompValue::Fill(Fill::Solid(Color::Literal(Rgba::rgb(
            255, 0, 0
        )))))
    );

    // Hex string form.
    let tx = reg
        .build(
            "shape.set_fill",
            &json!({ "entity": e.0, "color": "#00ff00" }),
            &w,
        )
        .unwrap();
    h.commit(&mut w, tx);
    assert_eq!(
        w.get(e, CompKind::Fill),
        Some(CompValue::Fill(Fill::Solid(Color::Literal(Rgba::rgb(
            0, 255, 0
        )))))
    );
}

#[test]
fn unknown_command_returns_none() {
    let reg = builtins();
    let w = World::new();
    assert!(reg.build("shape.nope", &json!({}), &w).is_none());
}

#[test]
fn missing_args_yield_empty_transaction() {
    let reg = builtins();
    let w = World::new();
    let tx = reg.build("shape.move", &json!({}), &w).unwrap();
    assert!(tx.ops.is_empty(), "lenient: missing fields => no ops");
}

/// Build a parent with three ordered children. Returns (world, parent, [c0, c1, c2])
/// where the children's current Order keys are strictly increasing (c0 back .. c2 front).
fn world_with_three_children() -> (World, Entity, [Entity; 3]) {
    let mut w = World::new();
    let parent = w.spawn();

    let mut children = [Entity(0); 3];
    let mut last: Option<FracIndex> = None;
    for child in &mut children {
        let e = w.spawn();
        w.set(e, CompValue::Parent(parent));
        let order = FracIndex::after(last.as_ref());
        w.set(e, CompValue::Order(order.clone()));
        last = Some(order);
        *child = e;
    }
    (w, parent, children)
}

/// Read an entity's Order key out of the world.
fn order_of(w: &World, e: Entity) -> FracIndex {
    match w.get(e, CompKind::Order) {
        Some(CompValue::Order(o)) => o,
        other => panic!("expected an Order on {e:?}, got {other:?}"),
    }
}

/// The three children re-sorted by their current Order key (back -> front).
fn children_by_order(w: &World, children: &[Entity; 3]) -> Vec<Entity> {
    let mut sorted = children.to_vec();
    sorted.sort_by_key(|&e| order_of(w, e));
    sorted
}

#[test]
fn bring_to_front_moves_child_last() {
    let (mut w, _parent, children) = world_with_three_children();
    let reg = builtins();
    let mut h = History::new();

    // c0 starts at the back; bring it to the front.
    let target = children[0];
    let tx = reg
        .build("shape.bring_to_front", &json!({ "entity": target.0 }), &w)
        .expect("shape.bring_to_front is registered");
    assert_eq!(tx.label, "Bring to Front");
    h.commit(&mut w, tx);

    let sorted = children_by_order(&w, &children);
    assert_eq!(
        *sorted.last().unwrap(),
        target,
        "bring_to_front puts the child last in Order; got {sorted:?}"
    );

    // One undo step restores the original ordering (c0 back).
    assert!(h.undo(&mut w));
    let restored = children_by_order(&w, &children);
    assert_eq!(restored, children.to_vec());
}

#[test]
fn send_to_back_moves_child_first() {
    let (mut w, _parent, children) = world_with_three_children();
    let reg = builtins();
    let mut h = History::new();

    // c2 starts at the front; send it to the back.
    let target = children[2];
    let tx = reg
        .build("shape.send_to_back", &json!({ "entity": target.0 }), &w)
        .expect("shape.send_to_back is registered");
    assert_eq!(tx.label, "Send to Back");
    h.commit(&mut w, tx);

    let sorted = children_by_order(&w, &children);
    assert_eq!(
        *sorted.first().unwrap(),
        target,
        "send_to_back puts the child first in Order; got {sorted:?}"
    );

    assert!(h.undo(&mut w));
    let restored = children_by_order(&w, &children);
    assert_eq!(restored, children.to_vec());
}

#[test]
fn z_order_missing_entity_is_lenient() {
    let reg = builtins();
    let w = World::new();
    let front = reg.build("shape.bring_to_front", &json!({}), &w).unwrap();
    let back = reg.build("shape.send_to_back", &json!({}), &w).unwrap();
    assert!(front.ops.is_empty());
    assert!(back.ops.is_empty());
}

#[test]
fn manifest_lists_builtin_commands() {
    let reg = builtins();
    let manifest = reg.manifest();
    let ids: Vec<&str> = manifest.iter().filter_map(|c| c["id"].as_str()).collect();
    assert!(ids.contains(&"shape.delete"));
    assert!(ids.contains(&"shape.set_fill"));
    assert!(ids.contains(&"shape.move"));

    // Each entry carries the documented shape.
    let mv = manifest.iter().find(|c| c["id"] == "shape.move").unwrap();
    assert_eq!(mv["title"], "Move Shape");
    assert_eq!(mv["category"], "Shape");
    let params = mv["params"].as_array().unwrap();
    assert_eq!(params[0]["name"], "entity");
    assert_eq!(params[0]["type"], "entity");
}

#[test]
fn set_rotation_command_sets_rotation() {
    let (mut w, e) = world_with_framed_shape();
    let reg = builtins();
    let mut h = History::new();

    let tx = reg
        .build(
            "shape.set_rotation",
            &json!({ "entity": e.0, "degrees": 45 }),
            &w,
        )
        .expect("shape.set_rotation is registered");
    assert_eq!(tx.label, "Set Rotation");
    h.commit(&mut w, tx);

    assert_eq!(
        w.get(e, CompKind::Rotation),
        Some(CompValue::Rotation(45.0))
    );

    // Undoable as one step (the rotation was previously absent).
    assert!(h.undo(&mut w));
    assert_eq!(w.get(e, CompKind::Rotation), None);
}

#[test]
fn fill_accent_command_sets_theme_fill() {
    use hayate_ir::color::{Color, ThemeColorToken};

    let (mut w, e) = world_with_framed_shape();
    let reg = builtins();
    let mut h = History::new();

    let tx = reg
        .build("shape.fill_accent3", &json!({ "entity": e.0 }), &w)
        .expect("shape.fill_accent3 is registered");
    h.commit(&mut w, tx);

    assert_eq!(
        w.get(e, CompKind::Fill),
        Some(CompValue::Fill(Fill::Solid(Color::theme(
            ThemeColorToken::Accent3
        ))))
    );
}

#[test]
fn filter_finds_accent_commands() {
    let reg = builtins();
    let hits = reg.filter("accent");
    let ids: Vec<&str> = hits.iter().map(|(id, _)| id.as_str()).collect();
    assert_eq!(ids.len(), 6, "exactly the six accent fills, got {ids:?}");
    for n in 1..=6 {
        let id = format!("shape.fill_accent{n}");
        assert!(ids.contains(&id.as_str()), "missing {id}");
    }
    // Titles come through alongside ids.
    assert!(hits.iter().any(|(_, title)| title == "Fill: Accent 1"));
}

#[test]
fn set_opacity_command_sets_and_clamps() {
    let (mut w, e) = world_with_framed_shape();
    let reg = builtins();
    let mut h = History::new();

    // A normal in-range value is stored as-is.
    let tx = reg
        .build(
            "shape.set_opacity",
            &json!({ "entity": e.0, "value": 0.5 }),
            &w,
        )
        .expect("shape.set_opacity is registered");
    assert_eq!(tx.label, "Set Opacity");
    h.commit(&mut w, tx);
    assert_eq!(w.get(e, CompKind::Opacity), Some(CompValue::Opacity(0.5)));

    // An out-of-range value is clamped to 1.0.
    let tx = reg
        .build(
            "shape.set_opacity",
            &json!({ "entity": e.0, "value": 2.5 }),
            &w,
        )
        .unwrap();
    h.commit(&mut w, tx);
    assert_eq!(w.get(e, CompKind::Opacity), Some(CompValue::Opacity(1.0)));

    // Undoable as one step (back to 0.5).
    assert!(h.undo(&mut w));
    assert_eq!(w.get(e, CompKind::Opacity), Some(CompValue::Opacity(0.5)));
}

#[test]
fn reset_rotation_command_sets_zero() {
    let (mut w, e) = world_with_framed_shape();
    w.set(e, CompValue::Rotation(33.0));
    let reg = builtins();
    let mut h = History::new();

    let tx = reg
        .build("shape.reset_rotation", &json!({ "entity": e.0 }), &w)
        .expect("shape.reset_rotation is registered");
    assert_eq!(tx.label, "Reset Rotation");
    h.commit(&mut w, tx);
    assert_eq!(w.get(e, CompKind::Rotation), Some(CompValue::Rotation(0.0)));

    // Undoable as one step (back to the prior rotation).
    assert!(h.undo(&mut w));
    assert_eq!(
        w.get(e, CompKind::Rotation),
        Some(CompValue::Rotation(33.0))
    );
}

#[test]
fn rotate_by_command_adds_to_current() {
    let (mut w, e) = world_with_framed_shape();
    w.set(e, CompValue::Rotation(10.0));
    let reg = builtins();
    let mut h = History::new();

    let tx = reg
        .build(
            "shape.rotate_by",
            &json!({ "entity": e.0, "degrees": 35 }),
            &w,
        )
        .expect("shape.rotate_by is registered");
    assert_eq!(tx.label, "Rotate By");
    h.commit(&mut w, tx);
    assert_eq!(
        w.get(e, CompKind::Rotation),
        Some(CompValue::Rotation(45.0))
    );

    // Undoable as one step (back to 10.0).
    assert!(h.undo(&mut w));
    assert_eq!(
        w.get(e, CompKind::Rotation),
        Some(CompValue::Rotation(10.0))
    );
}

#[test]
fn manifest_includes_new_style_and_shape_commands() {
    let reg = builtins();
    let manifest = reg.manifest();
    let ids: Vec<&str> = manifest.iter().filter_map(|c| c["id"].as_str()).collect();
    assert!(ids.contains(&"shape.set_opacity"));
    assert!(ids.contains(&"shape.reset_rotation"));
    assert!(ids.contains(&"shape.rotate_by"));
}

#[test]
fn manifest_includes_set_rotation() {
    let reg = builtins();
    let manifest = reg.manifest();
    let ids: Vec<&str> = manifest.iter().filter_map(|c| c["id"].as_str()).collect();
    assert!(ids.contains(&"shape.set_rotation"));
}

#[test]
fn set_position_command_sets_origin_keeping_size() {
    let (mut w, e) = world_with_framed_shape();
    let reg = builtins();
    let mut h = History::new();

    // x=100pt, y=50pt -> (1_270_000, 635_000) EMU; size (100, 50) preserved.
    let tx = reg
        .build(
            "shape.set_position",
            &json!({ "entity": e.0, "x": 100, "y": 50 }),
            &w,
        )
        .expect("shape.set_position is registered");
    assert_eq!(tx.label, "Set Position");
    h.commit(&mut w, tx);

    assert_eq!(
        w.get(e, CompKind::Frame),
        Some(CompValue::Frame(RectEmu::new(1_270_000, 635_000, 100, 50))),
        "origin set in EMU, size unchanged"
    );

    // Undoable as one step (back to the original frame).
    assert!(h.undo(&mut w));
    assert_eq!(
        w.get(e, CompKind::Frame),
        Some(CompValue::Frame(RectEmu::new(10, 20, 100, 50)))
    );
}

#[test]
fn set_size_command_sets_size_keeping_origin() {
    let (mut w, e) = world_with_framed_shape();
    let reg = builtins();
    let mut h = History::new();

    // w=200pt, h=100pt -> (2_540_000, 1_270_000) EMU; origin (10, 20) preserved.
    let tx = reg
        .build(
            "shape.set_size",
            &json!({ "entity": e.0, "w": 200, "h": 100 }),
            &w,
        )
        .expect("shape.set_size is registered");
    assert_eq!(tx.label, "Set Size");
    h.commit(&mut w, tx);

    assert_eq!(
        w.get(e, CompKind::Frame),
        Some(CompValue::Frame(RectEmu::new(10, 20, 2_540_000, 1_270_000))),
        "size set in EMU, origin unchanged"
    );

    // Undoable as one step (back to the original frame).
    assert!(h.undo(&mut w));
    assert_eq!(
        w.get(e, CompKind::Frame),
        Some(CompValue::Frame(RectEmu::new(10, 20, 100, 50)))
    );
}

#[test]
fn set_size_floors_to_one_point() {
    let (mut w, e) = world_with_framed_shape();
    let reg = builtins();
    let mut h = History::new();

    // Zero-ish dimensions are clamped to a minimum of 1pt (= EMU_PER_PT) each.
    let tx = reg
        .build(
            "shape.set_size",
            &json!({ "entity": e.0, "w": 0, "h": 0 }),
            &w,
        )
        .unwrap();
    h.commit(&mut w, tx);

    assert_eq!(
        w.get(e, CompKind::Frame),
        Some(CompValue::Frame(RectEmu::new(10, 20, 12_700, 12_700))),
        "each dimension floored at 1pt, origin unchanged"
    );
}

#[test]
fn set_position_missing_entity_is_lenient() {
    let reg = builtins();
    let w = World::new();
    let tx = reg
        .build("shape.set_position", &json!({ "x": 100, "y": 50 }), &w)
        .unwrap();
    assert!(tx.ops.is_empty(), "no entity => no ops");
}

#[test]
fn set_position_no_frame_is_lenient() {
    // An entity that exists but carries no Frame yields an empty transaction.
    let mut w = World::new();
    let e = w.spawn();
    let reg = builtins();
    let tx = reg
        .build(
            "shape.set_position",
            &json!({ "entity": e.0, "x": 100, "y": 50 }),
            &w,
        )
        .unwrap();
    assert!(tx.ops.is_empty(), "no Frame => no ops");
}

#[test]
fn set_size_no_frame_is_lenient() {
    let mut w = World::new();
    let e = w.spawn();
    let reg = builtins();
    let tx = reg
        .build(
            "shape.set_size",
            &json!({ "entity": e.0, "w": 200, "h": 100 }),
            &w,
        )
        .unwrap();
    assert!(tx.ops.is_empty(), "no Frame => no ops");
}

#[test]
fn manifest_includes_set_position_and_set_size() {
    let reg = builtins();
    let manifest = reg.manifest();
    let ids: Vec<&str> = manifest.iter().filter_map(|c| c["id"].as_str()).collect();
    assert!(ids.contains(&"shape.set_position"));
    assert!(ids.contains(&"shape.set_size"));
}

#[test]
fn set_text_command_creates_body_with_run_text() {
    let (mut w, e) = world_with_framed_shape();
    let reg = builtins();
    let mut h = History::new();

    let tx = reg
        .build(
            "shape.set_text",
            &json!({ "entity": e.0, "text": "Hello" }),
            &w,
        )
        .expect("shape.set_text is registered");
    assert_eq!(tx.label, "Set Text");
    h.commit(&mut w, tx);

    match w.get(e, CompKind::Text) {
        Some(CompValue::Text(body)) => {
            assert_eq!(body.paragraphs[0].runs[0].text, "Hello");
            assert!(!body.autofit);
        }
        other => panic!("expected a Text component, got {other:?}"),
    }

    // Undoable as one step (the text was previously absent).
    assert!(h.undo(&mut w));
    assert_eq!(w.get(e, CompKind::Text), None);
}

#[test]
fn fill_black_command_sets_literal_black() {
    let (mut w, e) = world_with_framed_shape();
    let reg = builtins();
    let mut h = History::new();

    let tx = reg
        .build("shape.fill_black", &json!({ "entity": e.0 }), &w)
        .expect("shape.fill_black is registered");
    h.commit(&mut w, tx);

    assert_eq!(
        w.get(e, CompKind::Fill),
        Some(CompValue::Fill(Fill::Solid(Color::Literal(Rgba::BLACK))))
    );
}

#[test]
fn manifest_includes_text_and_fill_commands() {
    let reg = builtins();
    let manifest = reg.manifest();
    let ids: Vec<&str> = manifest.iter().filter_map(|c| c["id"].as_str()).collect();
    assert!(ids.contains(&"shape.set_text"));
    assert!(ids.contains(&"shape.fill_black"));
    assert!(ids.contains(&"shape.fill_white"));
}

/// Spawn three framed entities under a common parent with increasing order keys; return
/// (world, [e0, e1, e2]). Their frames have differing x positions so alignment is visible.
fn world_with_three_framed() -> (World, [Entity; 3]) {
    let mut w = World::new();
    let parent = w.spawn();
    let frames = [
        RectEmu::new(10, 0, 100, 50),
        RectEmu::new(30, 100, 40, 50),
        RectEmu::new(70, 200, 60, 50),
    ];
    let mut entities = [Entity(0); 3];
    let mut last: Option<FracIndex> = None;
    for (slot, frame) in entities.iter_mut().zip(frames) {
        let e = w.spawn();
        w.set(e, CompValue::Parent(parent));
        let order = FracIndex::after(last.as_ref());
        w.set(e, CompValue::Order(order.clone()));
        last = Some(order);
        w.set(e, CompValue::Frame(frame));
        *slot = e;
    }
    (w, entities)
}

#[test]
fn align_left_command_shares_min_x() {
    let (mut w, es) = world_with_three_framed();
    let reg = builtins();
    let mut h = History::new();

    let tx = reg
        .build(
            "shapes.align_left",
            &json!({ "entities": [es[0].0, es[1].0, es[2].0] }),
            &w,
        )
        .expect("shapes.align_left is registered");
    assert_eq!(tx.label, "Align Left");
    h.commit(&mut w, tx);

    // All three left edges now sit at the group minimum x (10).
    let x_of = |w: &World, e: Entity| match w.get(e, CompKind::Frame) {
        Some(CompValue::Frame(f)) => f.origin.x,
        other => panic!("expected a Frame, got {other:?}"),
    };
    assert_eq!(x_of(&w, es[0]), 10);
    assert_eq!(x_of(&w, es[1]), 10);
    assert_eq!(x_of(&w, es[2]), 10);
}

#[test]
fn distribute_h_command_equalizes_gaps() {
    let (mut w, es) = world_with_three_framed();
    // Lay the items out so distribution has something to do.
    w.set(es[0], CompValue::Frame(RectEmu::new(0, 0, 100, 50)));
    w.set(es[1], CompValue::Frame(RectEmu::new(120, 0, 40, 50)));
    w.set(es[2], CompValue::Frame(RectEmu::new(240, 0, 60, 50)));
    let reg = builtins();
    let mut h = History::new();

    let tx = reg
        .build(
            "shapes.distribute_h",
            &json!({ "entities": [es[0].0, es[1].0, es[2].0] }),
            &w,
        )
        .expect("shapes.distribute_h is registered");
    assert_eq!(tx.label, "Distribute Horizontally");
    h.commit(&mut w, tx);

    // The middle item is repositioned so gaps are equal (gap = 50): x = 100 + 50 = 150.
    assert_eq!(
        w.get(es[1], CompKind::Frame),
        Some(CompValue::Frame(RectEmu::new(150, 0, 40, 50)))
    );
}

#[test]
fn align_distribute_missing_entities_is_lenient() {
    let reg = builtins();
    let w = World::new();
    // Missing `entities` -> no ids -> align/distribute return empty.
    assert!(reg
        .build("shapes.align_left", &json!({}), &w)
        .unwrap()
        .ops
        .is_empty());
    assert!(reg
        .build("shapes.distribute_v", &json!({ "entities": [] }), &w)
        .unwrap()
        .ops
        .is_empty());
}

#[test]
fn manifest_includes_arrange_commands() {
    let reg = builtins();
    let manifest = reg.manifest();
    let ids: Vec<&str> = manifest.iter().filter_map(|c| c["id"].as_str()).collect();
    for id in [
        "shapes.align_left",
        "shapes.align_hcenter",
        "shapes.align_right",
        "shapes.align_top",
        "shapes.align_vcenter",
        "shapes.align_bottom",
        "shapes.distribute_h",
        "shapes.distribute_v",
    ] {
        assert!(ids.contains(&id), "manifest missing {id}");
    }

    // The new commands are grouped under "Arrange".
    let al = manifest
        .iter()
        .find(|c| c["id"] == "shapes.align_left")
        .unwrap();
    assert_eq!(al["category"], "Arrange");
}

/// Build a world with a single entity carrying a Frame and a TextBody with two runs in one
/// paragraph (so "all runs" behaviour is observable). Returns (world, entity).
fn world_with_text_shape() -> (World, Entity) {
    use hayate_ir::text::Paragraph;
    let (mut w, e) = world_with_framed_shape();
    let body = TextBody {
        paragraphs: vec![Paragraph::new(vec![
            default_run("Hello"),
            default_run("World"),
        ])],
        autofit: false,
    };
    w.set(e, CompValue::Text(body));
    (w, e)
}

/// Read the entity's TextBody out of the world.
fn text_of(w: &World, e: Entity) -> TextBody {
    match w.get(e, CompKind::Text) {
        Some(CompValue::Text(body)) => body,
        other => panic!("expected a Text component, got {other:?}"),
    }
}

#[test]
fn set_font_size_command_sets_all_runs() {
    let (mut w, e) = world_with_text_shape();
    let reg = builtins();
    let mut h = History::new();

    let tx = reg
        .build(
            "shape.set_font_size",
            &json!({ "entity": e.0, "pt": 32 }),
            &w,
        )
        .expect("shape.set_font_size is registered");
    assert_eq!(tx.label, "Set Font Size");
    h.commit(&mut w, tx);

    let body = text_of(&w, e);
    for run in &body.paragraphs[0].runs {
        assert_eq!(run.size, pt(32), "every run sized to 32pt");
    }

    // Undoable as one step (back to the seeded 18pt).
    assert!(h.undo(&mut w));
    let body = text_of(&w, e);
    assert_eq!(body.paragraphs[0].runs[0].size, pt(18));
}

#[test]
fn set_font_size_command_clamps_to_one_point() {
    let (mut w, e) = world_with_text_shape();
    let reg = builtins();
    let mut h = History::new();

    let tx = reg
        .build(
            "shape.set_font_size",
            &json!({ "entity": e.0, "pt": 0 }),
            &w,
        )
        .unwrap();
    h.commit(&mut w, tx);

    let body = text_of(&w, e);
    assert_eq!(body.paragraphs[0].runs[0].size, pt(1), "floored at 1pt");
}

#[test]
fn toggle_bold_command_flips_all_runs() {
    let (mut w, e) = world_with_text_shape();
    let reg = builtins();
    let mut h = History::new();

    // First toggle: false -> true on every run.
    let tx = reg
        .build("shape.toggle_bold", &json!({ "entity": e.0 }), &w)
        .expect("shape.toggle_bold is registered");
    assert_eq!(tx.label, "Toggle Bold");
    h.commit(&mut w, tx);
    let body = text_of(&w, e);
    assert!(body.paragraphs[0].runs.iter().all(|r| r.bold));

    // Second toggle: back to false on every run (based on first run's value).
    let tx = reg
        .build("shape.toggle_bold", &json!({ "entity": e.0 }), &w)
        .unwrap();
    h.commit(&mut w, tx);
    let body = text_of(&w, e);
    assert!(body.paragraphs[0].runs.iter().all(|r| !r.bold));
}

#[test]
fn toggle_italic_command_flips_all_runs() {
    let (mut w, e) = world_with_text_shape();
    let reg = builtins();
    let mut h = History::new();

    let tx = reg
        .build("shape.toggle_italic", &json!({ "entity": e.0 }), &w)
        .expect("shape.toggle_italic is registered");
    assert_eq!(tx.label, "Toggle Italic");
    h.commit(&mut w, tx);
    let body = text_of(&w, e);
    assert!(body.paragraphs[0].runs.iter().all(|r| r.italic));
}

#[test]
fn toggle_underline_command_flips_all_runs() {
    let (mut w, e) = world_with_text_shape();
    let reg = builtins();
    let mut h = History::new();

    let tx = reg
        .build("shape.toggle_underline", &json!({ "entity": e.0 }), &w)
        .expect("shape.toggle_underline is registered");
    assert_eq!(tx.label, "Toggle Underline");
    h.commit(&mut w, tx);
    let body = text_of(&w, e);
    assert!(body.paragraphs[0].runs.iter().all(|r| r.underline));
}

#[test]
fn toggle_bold_consistent_when_runs_differ() {
    // When runs disagree, the whole box follows the FIRST run: first=false -> all true.
    let (mut w, e) = world_with_text_shape();
    let mut body = text_of(&w, e);
    body.paragraphs[0].runs[1].bold = true; // second run already bold
    w.set(e, CompValue::Text(body));
    let reg = builtins();
    let mut h = History::new();

    let tx = reg
        .build("shape.toggle_bold", &json!({ "entity": e.0 }), &w)
        .unwrap();
    h.commit(&mut w, tx);
    let body = text_of(&w, e);
    assert!(
        body.paragraphs[0].runs.iter().all(|r| r.bold),
        "first run was false, so the whole box becomes bold"
    );
}

#[test]
fn align_text_commands_set_every_paragraph() {
    for (id, expected) in [
        ("shape.align_text_left", HAlign::Left),
        ("shape.align_text_center", HAlign::Center),
        ("shape.align_text_right", HAlign::Right),
    ] {
        let (mut w, e) = world_with_text_shape();
        // Seed a second paragraph so "every paragraph" is observable.
        let mut body = text_of(&w, e);
        body.paragraphs
            .push(Paragraph::new(vec![default_run("Second")]));
        w.set(e, CompValue::Text(body));
        let reg = builtins();
        let mut h = History::new();

        let tx = reg
            .build(id, &json!({ "entity": e.0 }), &w)
            .unwrap_or_else(|| panic!("{id} is registered"));
        h.commit(&mut w, tx);

        let body = text_of(&w, e);
        for para in &body.paragraphs {
            assert_eq!(para.align, expected, "{id} sets every paragraph");
        }
    }
}

#[test]
fn text_commands_no_text_is_lenient() {
    // An entity that exists but carries no TextBody yields an empty transaction.
    let (w, e) = world_with_framed_shape();
    let reg = builtins();
    for id in [
        "shape.toggle_bold",
        "shape.toggle_italic",
        "shape.toggle_underline",
        "shape.align_text_left",
        "shape.align_text_center",
        "shape.align_text_right",
    ] {
        let tx = reg.build(id, &json!({ "entity": e.0 }), &w).unwrap();
        assert!(tx.ops.is_empty(), "{id}: no text => no ops");
    }
    let tx = reg
        .build(
            "shape.set_font_size",
            &json!({ "entity": e.0, "pt": 24 }),
            &w,
        )
        .unwrap();
    assert!(tx.ops.is_empty(), "set_font_size: no text => no ops");
}

#[test]
fn manifest_includes_text_formatting_commands() {
    let reg = builtins();
    let manifest = reg.manifest();
    let ids: Vec<&str> = manifest.iter().filter_map(|c| c["id"].as_str()).collect();
    for id in [
        "shape.set_font_size",
        "shape.toggle_bold",
        "shape.toggle_italic",
        "shape.toggle_underline",
        "shape.align_text_left",
        "shape.align_text_center",
        "shape.align_text_right",
    ] {
        assert!(ids.contains(&id), "manifest missing {id}");
    }
}
